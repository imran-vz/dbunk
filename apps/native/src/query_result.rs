//! Exact execution provenance and borrowed retained rows for query mutations.
//! Row payloads remain owned by the grid lease. The host must drop this capture
//! before replacing/clearing that grid, and must never substitute editor SQL.
use crate::results::{ResultModel, Row, TerminalStatus, encoded_size};
use dbunk_lib::backend::{
    QueryExecutionContext, QueryMutationSource, QueryMutationSourceError,
    data::{AnalyzeResultSetResult, AnalyzeSource},
};
use std::{
    cell::{Cell, OnceCell},
    rc::Rc,
};

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
    fn grow(&mut self, bytes: usize) -> bool {
        if bytes > WORKSPACE_BYTES.saturating_sub(self.budget.get()) {
            return false;
        }
        self.budget.set(self.budget.get() + bytes);
        self.bytes += bytes;
        true
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
/// The execution context is set once, only from the exactly matched completed
/// execution; it is never persisted, so restored sources keep the
/// session-independent subset (qualified targets, ASCII guards, no dates).
pub struct Provenance {
    pub source: QueryMutationSource,
    context: OnceCell<Box<QueryExecutionContext>>,
    lease: std::cell::RefCell<Reservation>,
}
impl Provenance {
    pub fn context(&self) -> Option<&QueryExecutionContext> {
        self.context.get().map(AsRef::as_ref)
    }
    /// Captured cells crossed the wire as UTF-8, so non-ASCII identity and
    /// guard text is the exact stored value.
    pub fn utf8(&self) -> bool {
        self.context().is_some_and(QueryExecutionContext::utf8)
    }
    pub fn analysis_source(&self) -> Result<AnalyzeSource, QueryMutationSourceError> {
        self.source.analysis_source(self.context())
    }
    /// Retains the context under the shared allowance; refusal leaves the
    /// source context-free, which only narrows what can be edited.
    fn attach(&self, context: Box<QueryExecutionContext>) -> bool {
        if self.context.get().is_some() {
            return false;
        }
        let bytes = context_bytes(&context);
        if !self.lease.borrow_mut().grow(bytes) {
            return false;
        }
        self.context.set(context).is_ok()
    }
    /// Loaded workspaces are bounded before decoding and precede result retention.
    pub fn restore(source: QueryMutationSource, budget: Rc<Cell<usize>>) -> Rc<Self> {
        let bytes = encoded_size(&source)
            .saturating_add(source.statement_sql().len())
            .saturating_add(16 * 1024);
        budget.set(budget.get().saturating_add(bytes));
        Rc::new(Self {
            source,
            context: OnceCell::new(),
            lease: std::cell::RefCell::new(Reservation { budget, bytes }),
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
                context: OnceCell::new(),
                lease: std::cell::RefCell::new(lease),
            }),
            connection: connection.into(),
            session: session.into(),
            execution: execution.into(),
        }))
    }
    pub fn matches(&self, connection: &str, session: &str, execution: &str) -> bool {
        self.connection == connection && self.session == session && self.execution == execution
    }
    /// Called only for this exact execution's successful completion.
    pub fn complete(&self, context: Option<Box<QueryExecutionContext>>) -> bool {
        context.is_some_and(|context| self.provenance.attach(context))
    }
}

fn context_bytes(context: &QueryExecutionContext) -> usize {
    [
        &context.client_encoding,
        &context.date_style,
        &context.interval_style,
        &context.search_path,
    ]
    .iter()
    .map(|value| value.as_ref().map_or(0, String::len))
    .sum::<usize>()
    .saturating_add(std::mem::size_of::<QueryExecutionContext>())
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
    utf8: bool,
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
    if !utf8
        && guarded.into_iter().any(|(position, _)| {
            row[position]
                .as_ref()
                .is_some_and(|value| !value.is_ascii())
        })
    {
        return Err(
            "Non-ASCII query guards need the execution's reported UTF8 client encoding; edit this row in a table tab",
        );
    }
    Ok((index, column.clone()))
}

#[cfg(test)]
mod tests;

/// Recovered query guards retain their bytes but cannot be reviewed without the
/// same encoding proof required for newly captured query rows. A restored
/// provenance has no execution context, so `utf8` is false for it.
pub fn guards_supported(operation: &dbunk_lib::backend::data::MutationOp, utf8: bool) -> bool {
    use dbunk_lib::backend::data::MutationOp;
    match operation {
        MutationOp::Update {
            identity, guards, ..
        } => {
            utf8 || identity
                .iter()
                .chain(guards)
                .all(|value| value.value.as_ref().is_none_or(|text| text.is_ascii()))
        }
        _ => false,
    }
}
