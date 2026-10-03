//! Durable user intent only. A saved operation is not a backend review or an
//! authorization: restored changes need fresh analysis and explicit review.
use super::WorkspaceError;
use crate::backend::query_mutation::QueryMutationSource;
use crate::result_mutation::protocol::{MutationIdentityKind, MutationOp, MutationValue};
use crate::table_browse::protocol::{BrowseFilter, BrowseSortKey};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceTableState {
    pub schema: String,
    pub table: String,
    pub filters: Vec<BrowseFilter>,
    pub sort: Vec<BrowseSortKey>,
    pub page_size: u32,
    pub draft: Option<WorkspaceMutationDraft>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceMutationDraft {
    pub changes: Vec<WorkspaceStagedChange>,
    pub apply_state: WorkspaceApplyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceApplyState {
    Staged,
    /// An apply was dispatched or its outcome could not be established. Never
    /// interpret recovery as rollback or automatically submit these operations.
    OutcomeUnknown,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceStagedChange {
    pub id: String,
    pub included: bool,
    /// None only for inserts. This captures the old identity claim, not proof
    /// that it remains valid against a new catalog snapshot.
    pub identity_kind: Option<MutationIdentityKind>,
    pub originals: Vec<MutationValue>,
    pub operation: MutationOp,
}

impl fmt::Debug for WorkspaceTableState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceTableState")
            .field(
                "staged_changes",
                &self.draft.as_ref().map_or(0, |draft| draft.changes.len()),
            )
            .field(
                "apply_state",
                &self.draft.as_ref().map(|draft| draft.apply_state),
            )
            .finish_non_exhaustive()
    }
}

impl WorkspaceTableState {
    /// Checks shape and guard consistency only. Backend analysis, stored policy
    /// and review are still mandatory; this method cannot authorize execution.
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        if !identifier(&self.schema)
            || !identifier(&self.table)
            || !(1..=1000).contains(&self.page_size)
        {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        if self.filters.len() > 256 || self.sort.len() > 256 {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        if let Some(draft) = &self.draft {
            validate_draft(draft, Some((&self.schema, &self.table)))?;
        }
        Ok(())
    }
}

pub const WORKSPACE_MUTATION_MAX_CHANGES: usize = 128;
pub const WORKSPACE_MUTATION_MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceQueryChanges {
    pub source: QueryMutationSource,
    pub draft: WorkspaceMutationDraft,
}
impl fmt::Debug for WorkspaceQueryChanges {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceQueryChanges")
            .field("source", &self.source)
            .field("staged_changes", &self.draft.changes.len())
            .field("apply_state", &self.draft.apply_state)
            .finish_non_exhaustive()
    }
}
impl WorkspaceQueryChanges {
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        self.source
            .validate()
            .map_err(|_| WorkspaceError::InvalidSnapshot)?;
        validate_draft(&self.draft, None)
    }
}

/// Query drafts allow UPDATEs to each described origin; table drafts bind every
/// operation to one relation. Both use the same original-identity/guard rules.
fn validate_draft(
    draft: &WorkspaceMutationDraft,
    relation: Option<(&str, &str)>,
) -> Result<(), WorkspaceError> {
    if draft.changes.len() > WORKSPACE_MUTATION_MAX_CHANGES
        || (draft.changes.is_empty() && draft.apply_state == WorkspaceApplyState::OutcomeUnknown)
    {
        return Err(WorkspaceError::InvalidSnapshot);
    }
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > WORKSPACE_MUTATION_MAX_BYTES.saturating_sub(self.0) {
                return Err(std::io::Error::other("mutation draft budget"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(&mut Count(0), draft).map_err(|_| WorkspaceError::TooLarge)?;
    let mut ids = HashSet::new();
    for change in &draft.changes {
        let id = uuid::Uuid::parse_str(&change.id).map_err(|_| WorkspaceError::InvalidSnapshot)?;
        if !ids.insert(id) || !values_valid(&change.originals) {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        let (table, identity, guards, set) = match &change.operation {
            MutationOp::Insert { table, values } => {
                if relation.is_none()
                    || change.identity_kind.is_some()
                    || !change.originals.is_empty()
                    || !values_valid(values)
                {
                    return Err(WorkspaceError::InvalidSnapshot);
                }
                (table, None, None, None)
            }
            MutationOp::Update {
                table,
                identity,
                guards,
                set,
            } => {
                if set.is_empty()
                    || !values_valid(set)
                    || set.iter().any(|value| {
                        !change
                            .originals
                            .iter()
                            .any(|old| old.column == value.column)
                    })
                {
                    return Err(WorkspaceError::InvalidSnapshot);
                }
                (table, Some(identity), Some(guards), Some(set))
            }
            MutationOp::Delete {
                table,
                identity,
                guards,
            } => {
                if relation.is_none() {
                    return Err(WorkspaceError::InvalidSnapshot);
                }
                (table, Some(identity), Some(guards), None)
            }
        };
        if !identifier(&table.schema)
            || !identifier(&table.table)
            || relation.is_some_and(|(schema, name)| table.schema != schema || table.table != name)
        {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        let Some(identity) = identity else {
            continue;
        };
        let kind = change
            .identity_kind
            .ok_or(WorkspaceError::InvalidSnapshot)?;
        if kind == MutationIdentityKind::None
            || identity.is_empty()
            || !values_valid(identity)
            || identity
                .iter()
                .any(|value| !change.originals.contains(value))
            || ((relation.is_none()
                || matches!(
                    kind,
                    MutationIdentityKind::PrimaryKey | MutationIdentityKind::UniqueIndex
                ))
                && identity.iter().any(|value| value.value.is_none()))
        {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        let guards = guards.ok_or(WorkspaceError::InvalidSnapshot)?;
        if let Some(set) = set.filter(|_| {
            matches!(
                kind,
                MutationIdentityKind::PrimaryKey | MutationIdentityKind::UniqueIndex
            )
        }) {
            if guards.len() != set.len()
                || guards.iter().zip(set).any(|(guard, value)| {
                    change
                        .originals
                        .iter()
                        .find(|old| old.column == value.column)
                        != Some(guard)
                })
            {
                return Err(WorkspaceError::InvalidSnapshot);
            }
        } else if guards != &change.originals {
            return Err(WorkspaceError::InvalidSnapshot);
        }
    }
    Ok(())
}

fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.contains('\0')
}
fn values_valid(values: &[MutationValue]) -> bool {
    let mut columns = HashSet::new();
    values
        .iter()
        .all(|value| identifier(&value.column) && columns.insert(&value.column))
}
