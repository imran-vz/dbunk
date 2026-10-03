//! Durable changes never carry analysis or review authority across launches.
use super::*;

impl MutationDraft {
    /// Measures the durable representation without cloning staged values. Save
    /// admission must happen before materializing any workspace snapshot.
    pub fn snapshot_bytes(&self) -> usize {
        use serde::{Serialize, Serializer, ser::SerializeSeq};
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct SavedChange<'a> {
            id: Uuid,
            included: bool,
            identity_kind: Option<MutationIdentityKind>,
            originals: &'a [MutationValue],
            operation: &'a MutationOp,
        }
        struct Changes<'a>(&'a MutationDraft);
        impl Serialize for Changes<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut sequence = serializer.serialize_seq(Some(self.0.changes.len()))?;
                for change in &self.0.changes {
                    sequence.serialize_element(&SavedChange {
                        id: change.id,
                        included: change.included,
                        identity_kind: change.row.as_ref().map(|row| row.kind),
                        originals: change
                            .row
                            .as_ref()
                            .map_or(&[], |row| row.originals.as_slice()),
                        operation: &change.operation,
                    })?;
                }
                sequence.end()
            }
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Snapshot<'a> {
            changes: Changes<'a>,
            apply_state: WorkspaceApplyState,
        }
        crate::results::encoded_size(&Snapshot {
            changes: Changes(self),
            apply_state: if self.outcome_unknown() {
                WorkspaceApplyState::OutcomeUnknown
            } else {
                WorkspaceApplyState::Staged
            },
        })
    }

    pub fn snapshot(&self) -> WorkspaceMutationDraft {
        WorkspaceMutationDraft {
            changes: self
                .changes
                .iter()
                .map(|change| WorkspaceStagedChange {
                    id: change.id.to_string(),
                    included: change.included,
                    identity_kind: change.row.as_ref().map(|row| row.kind),
                    originals: change
                        .row
                        .as_ref()
                        .map_or_else(Vec::new, |row| row.originals.clone()),
                    operation: change.operation.clone(),
                })
                .collect(),
            apply_state: if self.applying.is_some() || self.outcome_unknown {
                WorkspaceApplyState::OutcomeUnknown
            } else {
                WorkspaceApplyState::Staged
            },
        }
    }

    /// Restored changes can be inspected/exported immediately, but review and
    /// apply remain unavailable until refresh_analysis succeeds.
    pub fn restore(saved: &WorkspaceTableState) -> Result<Self, ModelError> {
        saved.validate().map_err(|_| ModelError::InvalidInput)?;
        let draft = saved.draft.as_ref().ok_or(ModelError::Unavailable)?;
        Self::restore_validated(draft)
    }

    pub fn restore_query(
        saved: &dbunk_lib::backend::WorkspaceQueryChanges,
    ) -> Result<Self, ModelError> {
        saved.validate().map_err(|_| ModelError::InvalidInput)?;
        Self::restore_validated(&saved.draft)
    }

    fn restore_validated(draft: &WorkspaceMutationDraft) -> Result<Self, ModelError> {
        let changes = draft
            .changes
            .iter()
            .map(|change| {
                let row = match &change.operation {
                    MutationOp::Insert { .. } => None,
                    MutationOp::Update {
                        table, identity, ..
                    }
                    | MutationOp::Delete {
                        table, identity, ..
                    } => Some(CapturedRow {
                        table: table.clone(),
                        kind: change.identity_kind.ok_or(ModelError::InvalidInput)?,
                        identity: identity.clone(),
                        originals: change.originals.clone(),
                    }),
                };
                Ok(Change {
                    id: Uuid::parse_str(&change.id).map_err(|_| ModelError::InvalidInput)?,
                    included: change.included,
                    row,
                    operation: change.operation.clone(),
                })
            })
            .collect::<Result<Vec<_>, ModelError>>()?;
        if crate::results::encoded_size(&changes) > DRAFT_BYTES {
            return Err(ModelError::Budget);
        }
        Ok(Self {
            owner: Uuid::new_v4(),
            analysis: None,
            revision: 0,
            changes,
            applying: None,
            invalidated: true,
            outcome_unknown: draft.apply_state == WorkspaceApplyState::OutcomeUnknown,
        })
    }

    pub fn outcome_unknown(&self) -> bool {
        self.outcome_unknown || self.applying.is_some()
    }

    /// Host action only after explicit reconciliation of the previous outcome.
    /// Clearing this warning still requires fresh analysis and a new review.
    pub fn mark_outcome_reconciled(&mut self) -> Result<(), ModelError> {
        if self.applying.is_some() {
            return Err(ModelError::Applying);
        }
        if !self.outcome_unknown {
            return Err(ModelError::InvalidInput);
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(ModelError::Unavailable)?;
        self.outcome_unknown = false;
        self.invalidated = true;
        self.analysis = None;
        Ok(())
    }

    /// Adopts a fresh backend analysis only when it still describes every saved
    /// operation. Original values never rebase to newly fetched database rows.
    pub fn refresh_analysis(&mut self, analysis: AnalyzeResultSetResult) -> Result<(), ModelError> {
        if self.applying.is_some() {
            return Err(ModelError::Applying);
        }
        if self.outcome_unknown {
            return Err(ModelError::OutcomeUnknown);
        }
        let candidate = Self::new(analysis)?;
        for change in &self.changes {
            let (target, writes, capability) = match &change.operation {
                MutationOp::Insert { table, values } => (table, values.as_slice(), 0),
                MutationOp::Update { table, set, .. } => (table, set.as_slice(), 1),
                MutationOp::Delete { table, .. } => (table, &[][..], 2),
            };
            let analysis = candidate.analysis.as_ref().unwrap();
            let table = analysis
                .tables
                .iter()
                .find(|table| table.schema == target.schema && table.table == target.table)
                .ok_or(ModelError::Stale)?;
            let allowed = match capability {
                0 => table.insertable.allowed,
                1 => table.updatable.allowed,
                _ => table.deletable.allowed,
            };
            if !allowed {
                return Err(ModelError::Unavailable);
            }
            candidate.writes(table, writes)?;
            if let Some(row) = &change.row {
                if row.kind != table.identity.kind
                    || !row
                        .identity
                        .iter()
                        .map(|value| &value.column)
                        .eq(table.identity.columns.iter())
                {
                    return Err(ModelError::Stale);
                }
                let projected = analysis
                    .columns
                    .iter()
                    .filter_map(|column| match &column.origin {
                        ColumnOrigin::Table {
                            schema,
                            table,
                            column,
                            ..
                        } if schema == &target.schema && table == &target.table => Some(column),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if projected
                    .iter()
                    .any(|column| !row.originals.iter().any(|value| &value.column == *column))
                    || row.originals.iter().any(|value| {
                        !projected.contains(&&value.column)
                            && !table.identity.columns.contains(&value.column)
                    })
                {
                    return Err(ModelError::Stale);
                }
            }
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(ModelError::Unavailable)?;
        self.analysis = candidate.analysis;
        self.invalidated = false;
        Ok(())
    }
}
