//! Exact execution provenance and borrowed retained rows for query mutations.
//! Row payloads remain owned by the grid lease. The host must drop this capture
//! before replacing/clearing that grid, and must never substitute editor SQL.
use crate::results::{ResultModel, Row, TerminalStatus, encoded_size};
use dbunk_lib::backend::{QueryMutationSource, data::AnalyzeResultSetResult};
use std::{cell::Cell, rc::Rc};

const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
const SOURCE_WORK_BYTES: usize = 16 * 1024 * 1024;
const ROW_METADATA_BYTES: usize = 1024 * 1024;

struct Reservation {
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Reservation {
    fn admit(bytes: usize, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        if bytes > WORKSPACE_BYTES.saturating_sub(budget.get()) {
            return Err("Result editing needs more shared allowance; clear another result or tool");
        }
        budget.set(budget.get() + bytes);
        Ok(Self { budget, bytes })
    }
    fn shrink(&mut self, bytes: usize) {
        assert!(
            bytes <= self.bytes,
            "provenance exceeded admitted working allowance"
        );
        self.budget.set(self.budget.get() - (self.bytes - bytes));
        self.bytes = bytes;
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}

/// Runtime identity is never persisted as analysis authority.
/// The lease also reserves one statement copy while analysis is queued or running.
/// The private validated source can be described without parsing it again.
pub struct Provenance {
    pub source: QueryMutationSource,
    _lease: Reservation,
}
impl Provenance {
    /// Loaded workspaces are bounded before decoding and precede result retention.
    pub fn restore(source: QueryMutationSource, budget: Rc<Cell<usize>>) -> Rc<Self> {
        let bytes = encoded_size(&source)
            .saturating_add(source.statement_sql().len())
            .saturating_add(16 * 1024);
        budget.set(budget.get().saturating_add(bytes));
        Rc::new(Self {
            source,
            _lease: Reservation { budget, bytes },
        })
    }
}

pub struct ExecutedSource {
    pub provenance: Rc<Provenance>,
    pub connection: String,
    pub session: String,
    pub execution: String,
}
impl ExecutedSource {
    pub fn prepare(
        sql: &str,
        parameter_mode: bool,
        connection: &str,
        session: &str,
        execution: &str,
        budget: Rc<Cell<usize>>,
    ) -> Result<Rc<Self>, String> {
        if sql.len() > 1024 * 1024
            || [connection, session, execution]
                .iter()
                .any(|id| id.is_empty() || id.len() > 128)
        {
            return Err("Result editing source exceeds its identity or 1 MiB SQL limit".into());
        }
        let mut lease = Reservation::admit(SOURCE_WORK_BYTES, budget).map_err(str::to_owned)?;
        let source = QueryMutationSource::new(sql.to_owned(), parameter_mode)
            .map_err(|error| format!("Result editing unavailable: {error}"))?;
        let retained = encoded_size(&source)
            .saturating_add(source.statement_sql().len())
            .saturating_add(connection.len() + session.len() + execution.len())
            .saturating_add(16 * 1024);
        lease.shrink(retained);
        Ok(Rc::new(Self {
            provenance: Rc::new(Provenance {
                source,
                _lease: lease,
            }),
            connection: connection.into(),
            session: session.into(),
            execution: execution.into(),
        }))
    }
    pub fn matches(&self, connection: &str, session: &str, execution: &str) -> bool {
        self.connection == connection && self.session == session && self.execution == execution
    }
}

/// Shares original strings with the first result set; never stages displayed or
/// clipped text. Whole-row omission can leave retained rows exact, but shortened
/// cells or incomplete truncation disclosure invalidate the entire execution.
pub struct QueryRows {
    pub origin: Rc<ExecutedSource>,
    pub rows: Vec<Row>,
    columns: Vec<String>,
    _lease: Reservation,
}
impl QueryRows {
    pub fn capture(
        origin: Rc<ExecutedSource>,
        model: &ResultModel,
        budget: Rc<Cell<usize>>,
    ) -> Result<Rc<Self>, &'static str> {
        let completion = model
            .completion
            .as_ref()
            .ok_or("Wait for query completion before editing")?;
        if completion.status != TerminalStatus::Completed {
            return Err("Only successfully completed query results can be edited");
        }
        if model.native_omitted_metadata != 0
            || completion.omitted_metadata_bytes != 0
            || completion.truncation_reasons.iter().any(|reason| {
                !matches!(
                    reason.as_str(),
                    "rowCount" | "rowLimit" | "resultSets" | "notices"
                )
            })
        {
            return Err(
                "Result values or their truncation metadata were shortened; rerun a smaller query before editing",
            );
        }
        let set = model
            .sets
            .first()
            .filter(|set| set.index == 0)
            .ok_or("The first result set is unavailable")?;
        if set.row_count.is_none()
            || set.columns.iter().any(Option::is_none)
            || set.columns.len() > 4096
            || encoded_size(&set.columns) > 256 * 1024
            || set.rows.len() > 10_000
            || set.rows.iter().any(|row| row.len() != set.columns.len())
        {
            return Err("Result metadata is incomplete or exceeds editing limits");
        }
        let lease = Reservation::admit(ROW_METADATA_BYTES, budget)?;
        Ok(Rc::new(Self {
            origin,
            rows: set.rows.clone(),
            columns: set
                .columns
                .iter()
                .map(|name| name.as_ref().unwrap().clone())
                .collect(),
            _lease: lease,
        }))
    }
    pub fn compatible(&self, analysis: &AnalyzeResultSetResult) -> bool {
        self.columns.len() == analysis.columns.len()
            && self
                .columns
                .iter()
                .zip(&analysis.columns)
                .all(|(name, column)| name == &column.name)
    }
}

/// Resolves the selected origin table, including joins. Query identities must be
/// visibly projected and non-NULL; hidden browse identities are a separate path.
pub fn editable_target(
    analysis: &AnalyzeResultSetResult,
    row: &[Option<String>],
    column: usize,
) -> Result<(usize, String), &'static str> {
    use dbunk_lib::backend::data::{ColumnOrigin, ColumnWritability, MutationIdentityKind};
    let column_index = column;
    let selected = analysis
        .columns
        .get(column)
        .ok_or("Selected column is not in this analysis")?;
    if selected.writability != ColumnWritability::Writable {
        return Err("Generated, identity-always and system columns are read-only");
    }
    let ColumnOrigin::Table {
        schema,
        table,
        column,
        ..
    } = &selected.origin
    else {
        return Err("Computed columns are read-only");
    };
    let (index, target) = analysis
        .tables
        .iter()
        .enumerate()
        .find(|(_, origin)| origin.schema == *schema && origin.table == *table)
        .ok_or("The source table could not be resolved")?;
    if !target.updatable.allowed
        || target.identity.kind == MutationIdentityKind::None
        || !target.identity_projected
        || target.identity.columns.len() != target.identity_projection_indexes.len()
        || target.identity.columns.is_empty()
    {
        return Err("Project the complete row identity to edit this table");
    }
    if row.len() != analysis.columns.len()
        || target
            .identity_projection_indexes
            .iter()
            .any(|index| row.get(*index).is_none_or(Option::is_none))
    {
        return Err("This row has a missing or NULL identity value");
    }
    let full_guard = matches!(
        target.identity.kind,
        MutationIdentityKind::VirtualKey | MutationIdentityKind::CtidFallback
    );
    let guarded = analysis.columns.iter().enumerate().filter(|(position, candidate)| {
        target.identity_projection_indexes.contains(position)
            || *position == column_index
            || full_guard && matches!(&candidate.origin, ColumnOrigin::Table { schema: origin_schema, table: origin_table, .. } if origin_schema == schema && origin_table == table)
    });
    if guarded.into_iter().any(|(position, _)| {
        row[position]
            .as_ref()
            .is_some_and(|value| !value.is_ascii())
    }) {
        return Err(
            "Non-ASCII query guards need execution encoding metadata; edit this row in a table tab",
        );
    }
    Ok((index, column.clone()))
}

#[cfg(test)]
mod tests;

/// Recovered query guards retain their bytes but cannot be reviewed without the
/// same conservative encoding proof required for newly captured query rows.
pub fn guards_supported(operation: &dbunk_lib::backend::data::MutationOp) -> bool {
    use dbunk_lib::backend::data::MutationOp;
    match operation {
        MutationOp::Update {
            identity, guards, ..
        } => identity
            .iter()
            .chain(guards)
            .all(|value| value.value.as_ref().is_none_or(|text| text.is_ascii())),
        _ => false,
    }
}
