//! Identity-safe staged changes and revision-bound apply settlement.
use super::*;
use std::rc::Rc;
mod batch;
mod overlay;
mod retention;
pub use batch::{BulkOutcome, BulkRow};
pub use overlay::*;

#[derive(Clone, serde::Serialize)]
struct CapturedRow {
    table: MutationTable,
    kind: MutationIdentityKind,
    identity: Vec<MutationValue>,
    originals: Vec<MutationValue>,
}
#[derive(Clone, serde::Serialize)]
struct Change {
    #[serde(skip)]
    id: Uuid,
    included: bool,
    row: Option<CapturedRow>,
    operation: MutationOp,
}

/// Immutable inputs to backend preview. This is a revision fence, not policy
/// authorization; the runtime must still obtain/consume the backend review.
pub struct ReviewPlan {
    owner: Uuid,
    revision: u64,
    analysis_id: u64,
    plan: Rc<MutationPlan>,
    changes: Vec<Uuid>,
}
impl ReviewPlan {
    pub fn plan(&self) -> &MutationPlan {
        &self.plan
    }
    pub fn analysis_id(&self) -> u64 {
        self.analysis_id
    }
}

pub struct ApplyTicket {
    owner: Uuid,
    revision: u64,
    changes: Vec<Uuid>,
}
pub enum ApplyResolution {
    Applied,
    Failed {
        change: Option<Uuid>,
        error: ResultMutationError,
    },
}

pub struct MutationDraft {
    owner: Uuid,
    analysis: Option<AnalyzeResultSetResult>,
    revision: u64,
    changes: Vec<Change>,
    applying: Option<u64>,
    invalidated: bool,
    outcome_unknown: bool,
}
impl MutationDraft {
    pub fn new(analysis: AnalyzeResultSetResult) -> Result<Self, ModelError> {
        if !matches!(analysis.statement, AnalysisStatement::Analyzed) {
            return Err(ModelError::Unavailable);
        }
        if crate::results::encoded_size(&analysis) > ANALYSIS_BYTES {
            return Err(ModelError::Budget);
        }
        Ok(Self {
            owner: Uuid::new_v4(),
            analysis: Some(analysis),
            revision: 0,
            changes: Vec::new(),
            applying: None,
            invalidated: false,
            outcome_unknown: false,
        })
    }
    pub fn len(&self) -> usize {
        self.changes.len()
    }
    /// Identifies this draft instance; with `revision` it keys derived caches.
    pub fn owner(&self) -> Uuid {
        self.owner
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
    pub fn changes(&self) -> impl Iterator<Item = (Uuid, bool, &MutationOp)> {
        self.changes
            .iter()
            .map(|change| (change.id, change.included, &change.operation))
    }
    pub fn remove(&mut self, id: Uuid) -> Result<(), ModelError> {
        self.selection_editable()?;
        let mut changes = self.changes.clone();
        let index = changes
            .iter()
            .position(|change| change.id == id)
            .ok_or(ModelError::InvalidInput)?;
        changes.remove(index);
        self.publish(changes)
    }
    pub fn invalidate(&mut self) {
        self.invalidated = true;
        self.analysis = None;
    }
    fn editable(&self) -> Result<(), ModelError> {
        self.selection_editable()?;
        if self.invalidated {
            Err(ModelError::Stale)
        } else {
            Ok(())
        }
    }
    /// Selection and removal need no catalog authority, but must preserve an
    /// in-flight or uncertain apply's exact changes until its outcome is known.
    fn selection_editable(&self) -> Result<(), ModelError> {
        if self.applying.is_some() {
            Err(ModelError::Applying)
        } else if self.outcome_unknown {
            Err(ModelError::OutcomeUnknown)
        } else {
            Ok(())
        }
    }
    fn publish(&mut self, changes: Vec<Change>) -> Result<(), ModelError> {
        if changes.len() > CHANGE_LIMIT || crate::results::encoded_size(&changes) > DRAFT_BYTES {
            return Err(ModelError::Budget);
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(ModelError::Unavailable)?;
        self.changes = changes;
        Ok(())
    }
    fn table(&self, index: usize) -> Result<&AnalyzedTable, ModelError> {
        self.analysis
            .as_ref()
            .ok_or(ModelError::Stale)?
            .tables
            .get(index)
            .ok_or(ModelError::InvalidInput)
    }
    fn writes(&self, table: &AnalyzedTable, values: &[MutationValue]) -> Result<(), ModelError> {
        for (index, value) in values.iter().enumerate() {
            if values[..index].iter().any(|old| old.column == value.column) {
                return Err(ModelError::InvalidInput);
            }
            if !self.analysis.as_ref().ok_or(ModelError::Stale)?.columns.iter().any(|column| matches!(&column.origin, ColumnOrigin::Table { schema, table: name, column: original, .. }
                if schema == &table.schema && name == &table.table && original == &value.column)
                && column.writability == ColumnWritability::Writable) { return Err(ModelError::Unavailable); }
        }
        Ok(())
    }
    /// Captures original projected values and backend-selected identity. Hidden
    /// system identity is accepted only for ctid fallback, with exact arity.
    fn capture(
        &self,
        table: &AnalyzedTable,
        row: &[Option<String>],
        hidden_identity: Option<&[String]>,
        truncated: bool,
    ) -> Result<CapturedRow, ModelError> {
        if truncated
            || row.len()
                != self
                    .analysis
                    .as_ref()
                    .ok_or(ModelError::Stale)?
                    .columns
                    .len()
            || table.identity.kind == MutationIdentityKind::None
            || table.identity.columns.is_empty()
        {
            return Err(ModelError::Unavailable);
        }
        if crate::results::encoded_size(&row) > DRAFT_BYTES
            || hidden_identity
                .is_some_and(|values| crate::results::encoded_size(&values) > DRAFT_BYTES)
        {
            return Err(ModelError::Budget);
        }
        let mut originals: Vec<MutationValue> = Vec::new();
        for (column, value) in self
            .analysis
            .as_ref()
            .ok_or(ModelError::Stale)?
            .columns
            .iter()
            .zip(row)
        {
            if let ColumnOrigin::Table {
                schema,
                table: name,
                column: original,
                ..
            } = &column.origin
            {
                if schema != &table.schema || name != &table.table {
                    continue;
                }
                if let Some(previous) = originals
                    .iter()
                    .find(|previous| &previous.column == original)
                {
                    if &previous.value != value {
                        return Err(ModelError::InvalidInput);
                    }
                } else {
                    originals.push(MutationValue {
                        column: original.clone(),
                        value: value.clone(),
                    });
                }
            }
        }
        let identity = if table.identity.kind == MutationIdentityKind::CtidFallback
            && !(hidden_identity.is_none() && table.identity_projected)
        {
            let hidden = hidden_identity.ok_or(ModelError::Unavailable)?;
            if hidden.len() != table.identity.columns.len() {
                return Err(ModelError::InvalidInput);
            }
            table
                .identity
                .columns
                .iter()
                .zip(hidden)
                .map(|(column, value)| MutationValue {
                    column: column.clone(),
                    value: Some(value.clone()),
                })
                .collect::<Vec<_>>()
        } else {
            table
                .identity
                .columns
                .iter()
                .map(|column| {
                    originals
                        .iter()
                        .find(|value| &value.column == column)
                        .cloned()
                        .ok_or(ModelError::Unavailable)
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        if matches!(
            table.identity.kind,
            MutationIdentityKind::PrimaryKey | MutationIdentityKind::UniqueIndex
        ) && identity.iter().any(|value| value.value.is_none())
        {
            return Err(ModelError::Unavailable);
        }
        for value in &identity {
            if !originals.iter().any(|old| old.column == value.column) {
                originals.push(value.clone());
            }
        }
        Ok(CapturedRow {
            table: MutationTable {
                schema: table.schema.clone(),
                table: table.table.clone(),
            },
            kind: table.identity.kind,
            identity,
            originals,
        })
    }
    fn existing(&self, row: &CapturedRow) -> Result<Option<usize>, ModelError> {
        Self::existing_in(&self.changes, row)
    }
    fn existing_in(changes: &[Change], row: &CapturedRow) -> Result<Option<usize>, ModelError> {
        let index = changes.iter().position(|change| {
            change
                .row
                .as_ref()
                .is_some_and(|old| old.table == row.table && old.identity == row.identity)
        });
        if let Some(index) = index {
            let old = changes[index].row.as_ref().unwrap();
            // An unproven key can identify distinct rows; ctid can be reused.
            // Never merge a new full-row capture into a different staged row.
            if matches!(
                row.kind,
                MutationIdentityKind::VirtualKey | MutationIdentityKind::CtidFallback
            ) && old.originals != row.originals
            {
                return Err(ModelError::AmbiguousIdentity);
            }
        }
        Ok(index)
    }
    /// Reopening an editor starts from its staged value. Identity and guards
    /// still come from the original page, including when the key itself changed.
    pub fn edit_value<'a>(
        &'a self,
        table_index: usize,
        row: &'a [Option<String>],
        hidden_identity: Option<&[String]>,
        truncated: bool,
        column_index: usize,
    ) -> Result<&'a Option<String>, ModelError> {
        self.editable()?;
        let table = self.table(table_index)?;
        if !table.updatable.allowed {
            return Err(ModelError::Unavailable);
        }
        let column = self
            .analysis
            .as_ref()
            .ok_or(ModelError::Stale)?
            .columns
            .get(column_index)
            .ok_or(ModelError::InvalidInput)?;
        let ColumnOrigin::Table {
            schema,
            table: name,
            column: original,
            ..
        } = &column.origin
        else {
            return Err(ModelError::Unavailable);
        };
        if schema != &table.schema
            || name != &table.table
            || column.writability != ColumnWritability::Writable
        {
            return Err(ModelError::Unavailable);
        }
        let captured = self.capture(table, row, hidden_identity, truncated)?;
        if let Some(index) = self.existing(&captured)? {
            match &self.changes[index].operation {
                MutationOp::Delete { .. } => return Err(ModelError::Unavailable),
                MutationOp::Update { set, .. } => {
                    if let Some(value) = set.iter().find(|value| &value.column == original) {
                        return Ok(&value.value);
                    }
                }
                MutationOp::Insert { .. } => {}
            }
        }
        row.get(column_index).ok_or(ModelError::InvalidInput)
    }

    pub fn stage_update(
        &mut self,
        table_index: usize,
        row: &[Option<String>],
        hidden_identity: Option<&[String]>,
        truncated: bool,
        edits: Vec<MutationValue>,
    ) -> Result<(), ModelError> {
        self.editable()?;
        if truncated {
            return Err(ModelError::Unavailable);
        }
        let mut changes = self.changes.clone();
        self.update_changes(&mut changes, table_index, row, hidden_identity, edits)?;
        self.publish(changes)
    }
    /// Shared merge for one candidate draft. The caller publishes atomically.
    fn update_changes(
        &self,
        changes: &mut Vec<Change>,
        table_index: usize,
        row: &[Option<String>],
        hidden_identity: Option<&[String]>,
        edits: Vec<MutationValue>,
    ) -> Result<bool, ModelError> {
        let table = self.table(table_index)?;
        if !table.updatable.allowed {
            return Err(ModelError::Unavailable);
        }
        self.writes(table, &edits)?;
        let captured = self.capture(table, row, hidden_identity, false)?;
        let existing = Self::existing_in(changes, &captured)?;
        let mut set = match existing.map(|index| &changes[index].operation) {
            Some(MutationOp::Delete { .. }) => return Err(ModelError::Unavailable),
            Some(MutationOp::Update { set, .. }) => set.clone(),
            _ => Vec::new(),
        };
        let captured = existing
            .and_then(|index| changes[index].row.clone())
            .unwrap_or(captured);
        for edit in edits {
            let original = captured
                .originals
                .iter()
                .find(|old| old.column == edit.column)
                .ok_or(ModelError::InvalidInput)?;
            if edit.value == original.value {
                set.retain(|value| value.column != edit.column);
            } else if let Some(value) = set.iter_mut().find(|value| value.column == edit.column) {
                *value = edit;
            } else {
                set.push(edit);
            }
        }
        if set.is_empty() {
            if let Some(index) = existing {
                changes.remove(index);
            }
            return Ok(existing.is_some());
        }
        let guards = if matches!(
            captured.kind,
            MutationIdentityKind::VirtualKey | MutationIdentityKind::CtidFallback
        ) {
            captured.originals.clone()
        } else {
            set.iter()
                .map(|value| {
                    captured
                        .originals
                        .iter()
                        .find(|old| old.column == value.column)
                        .unwrap()
                        .clone()
                })
                .collect()
        };
        let operation = MutationOp::Update {
            table: captured.table.clone(),
            identity: captured.identity.clone(),
            guards,
            set,
        };
        let changed = existing.is_none_or(|index| changes[index].operation != operation);
        if changed {
            replace_change(changes, existing, Some(captured), operation);
        }
        Ok(changed)
    }
    pub fn stage_delete(
        &mut self,
        table_index: usize,
        row: &[Option<String>],
        hidden_identity: Option<&[String]>,
        truncated: bool,
    ) -> Result<(), ModelError> {
        self.editable()?;
        let table = self.table(table_index)?;
        if !table.deletable.allowed {
            return Err(ModelError::Unavailable);
        }
        let row = self.capture(table, row, hidden_identity, truncated)?;
        let existing = self.existing(&row)?;
        let row = existing
            .and_then(|index| self.changes[index].row.clone())
            .unwrap_or(row);
        let operation = MutationOp::Delete {
            table: row.table.clone(),
            identity: row.identity.clone(),
            guards: row.originals.clone(),
        };
        let mut changes = self.changes.clone();
        replace_change(&mut changes, existing, Some(row), operation);
        self.publish(changes)
    }
    /// Stages every row's delete or none of them. Each row is captured from
    /// its original page values; an existing update on the same row becomes
    /// the delete, keeping its original capture. Returns the rows staged.
    pub fn stage_deletes(
        &mut self,
        table_index: usize,
        rows: &[(&[Option<String>], Option<&[String]>)],
        truncated: bool,
    ) -> Result<usize, ModelError> {
        self.editable()?;
        if rows.is_empty() {
            return Err(ModelError::InvalidInput);
        }
        if rows.len() > CHANGE_LIMIT {
            return Err(ModelError::Budget);
        }
        let table = self.table(table_index)?;
        if !table.deletable.allowed {
            return Err(ModelError::Unavailable);
        }
        let mut changes = self.changes.clone();
        for (row, hidden) in rows {
            let captured = self.capture(table, row, *hidden, truncated)?;
            let existing = Self::existing_in(&changes, &captured)?;
            let captured = existing
                .and_then(|index| changes[index].row.clone())
                .unwrap_or(captured);
            let operation = MutationOp::Delete {
                table: captured.table.clone(),
                identity: captured.identity.clone(),
                guards: captured.originals.clone(),
            };
            replace_change(&mut changes, existing, Some(captured), operation);
            if changes.len() > CHANGE_LIMIT {
                return Err(ModelError::Budget);
            }
        }
        self.publish(changes)?;
        Ok(rows.len())
    }
    /// The staged insert, its analysed target table and its position.
    fn insert_change(&self, id: Uuid) -> Result<(usize, &AnalyzedTable), ModelError> {
        let index = self
            .changes
            .iter()
            .position(|change| change.id == id)
            .ok_or(ModelError::InvalidInput)?;
        let MutationOp::Insert { table: target, .. } = &self.changes[index].operation else {
            return Err(ModelError::InvalidInput);
        };
        let table = self
            .analysis
            .as_ref()
            .ok_or(ModelError::Stale)?
            .tables
            .iter()
            .find(|table| table.schema == target.schema && table.table == target.table)
            .ok_or(ModelError::Stale)?;
        if !table.insertable.allowed {
            return Err(ModelError::Unavailable);
        }
        Ok((index, table))
    }
    /// An insert cell for editing: `None` is omitted (DEFAULT), `Some(None)`
    /// an explicit NULL. Refuses columns the insert cannot write.
    pub fn insert_value(
        &self,
        id: Uuid,
        column: &str,
    ) -> Result<Option<Option<&str>>, ModelError> {
        self.editable()?;
        let (index, table) = self.insert_change(id)?;
        self.writes(
            table,
            &[MutationValue {
                column: column.to_owned(),
                value: None,
            }],
        )?;
        let MutationOp::Insert { values, .. } = &self.changes[index].operation else {
            return Err(ModelError::InvalidInput);
        };
        Ok(values
            .iter()
            .find(|value| value.column == column)
            .map(|value| value.value.as_deref()))
    }
    /// Sets one insert cell: `None` drops the value so the column default
    /// applies, `Some(None)` writes NULL.
    pub fn set_insert_value(
        &mut self,
        id: Uuid,
        column: &str,
        value: Option<Option<String>>,
    ) -> Result<(), ModelError> {
        self.editable()?;
        let (index, table) = self.insert_change(id)?;
        self.writes(
            table,
            &[MutationValue {
                column: column.to_owned(),
                value: None,
            }],
        )?;
        let mut changes = self.changes.clone();
        let MutationOp::Insert { values, .. } = &mut changes[index].operation else {
            return Err(ModelError::InvalidInput);
        };
        let position = values.iter().position(|old| old.column == column);
        match (position, value) {
            (None, None) => return Ok(()),
            (Some(position), None) => {
                values.remove(position);
            }
            (Some(position), Some(value)) => {
                if values[position].value == value {
                    return Ok(());
                }
                values[position].value = value;
            }
            (None, Some(value)) => values.push(MutationValue {
                column: column.to_owned(),
                value,
            }),
        }
        self.publish(changes)
    }
    /// Missing columns remain omitted (defaults); None is an explicit SQL NULL.
    pub fn stage_insert(
        &mut self,
        table_index: usize,
        values: Vec<MutationValue>,
    ) -> Result<Uuid, ModelError> {
        self.editable()?;
        let table = self.table(table_index)?;
        if !table.insertable.allowed {
            return Err(ModelError::Unavailable);
        }
        self.writes(table, &values)?;
        let mut changes = self.changes.clone();
        let id = Uuid::new_v4();
        changes.push(Change {
            id,
            included: true,
            row: None,
            operation: MutationOp::Insert {
                table: MutationTable {
                    schema: table.schema.clone(),
                    table: table.table.clone(),
                },
                values,
            },
        });
        self.publish(changes)?;
        Ok(id)
    }
    pub fn include(&mut self, id: Uuid, included: bool) -> Result<(), ModelError> {
        self.selection_editable()?;
        let mut changes = self.changes.clone();
        changes
            .iter_mut()
            .find(|change| change.id == id)
            .ok_or(ModelError::InvalidInput)?
            .included = included;
        self.publish(changes)
    }
    pub fn review(&self) -> Result<ReviewPlan, ModelError> {
        self.editable()?;
        let selected = self
            .changes
            .iter()
            .filter(|change| change.included)
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Err(ModelError::Unavailable);
        }
        Ok(ReviewPlan {
            owner: self.owner,
            revision: self.revision,
            analysis_id: self.analysis.as_ref().ok_or(ModelError::Stale)?.analysis_id,
            plan: Rc::new(MutationPlan {
                operations: selected
                    .iter()
                    .map(|change| change.operation.clone())
                    .collect(),
            }),
            changes: selected.iter().map(|change| change.id).collect(),
        })
    }
    pub fn begin_apply(&mut self, review: &ReviewPlan) -> Result<ApplyTicket, ModelError> {
        self.editable()?;
        if review.owner != self.owner || review.revision != self.revision {
            return Err(ModelError::Stale);
        }
        self.applying = Some(review.revision);
        Ok(ApplyTicket {
            owner: self.owner,
            revision: review.revision,
            changes: review.changes.clone(),
        })
    }
    /// Only an accepted complete response removes included changes. Failure or
    /// a missing reply retains them; recovery must not automatically retry DML.
    pub fn finish_apply(
        &mut self,
        ticket: ApplyTicket,
        result: Result<ApplyResult, ResultMutationError>,
    ) -> Result<ApplyResolution, ModelError> {
        if ticket.owner != self.owner || self.applying != Some(ticket.revision) {
            return Err(ModelError::Stale);
        }
        self.applying = None;
        match result {
            Err(error) => {
                self.outcome_unknown = matches!(
                    error,
                    ResultMutationError::ConnectionLost
                        | ResultMutationError::ConnectionClosing
                        | ResultMutationError::Timeout { .. }
                );
                let index = match &error {
                    ResultMutationError::Conflict { op_index }
                    | ResultMutationError::IdentityNotUnique { op_index }
                    | ResultMutationError::LockTimeout { op_index } => Some(*op_index),
                    ResultMutationError::Database { op_index, .. } => *op_index,
                    _ => None,
                };
                Ok(ApplyResolution::Failed {
                    change: index.and_then(|index| ticket.changes.get(index).copied()),
                    error,
                })
            }
            Ok(result) => {
                if result.operations.len() != ticket.changes.len()
                    || result
                        .operations
                        .iter()
                        .enumerate()
                        .any(|(index, op)| op.op_index != index || op.rows_affected != 1)
                {
                    self.invalidated = true;
                    self.outcome_unknown = true;
                    return Err(ModelError::InvalidReply);
                }
                self.changes
                    .retain(|change| !ticket.changes.contains(&change.id));
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(ModelError::Unavailable)?;
                Ok(ApplyResolution::Applied)
            }
        }
    }
}

fn replace_change(
    changes: &mut Vec<Change>,
    existing: Option<usize>,
    row: Option<CapturedRow>,
    operation: MutationOp,
) {
    if let Some(index) = existing {
        changes[index].row = row;
        changes[index].operation = operation;
    } else {
        changes.push(Change {
            id: Uuid::new_v4(),
            included: true,
            row,
            operation,
        });
    }
}

#[path = "mutation_recovery.rs"]
mod recovery;
