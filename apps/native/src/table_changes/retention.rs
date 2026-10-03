use super::*;

const WORK_BYTES: usize = 32 * 1024 * 1024;
impl TableChanges {
    /// Covers candidate changes, bounded row captures, encoded insert text,
    /// literal/committed buffers and finite editor history. Arrays admit separately.
    pub(super) fn admit_work(&mut self) -> bool {
        if self.work_bytes != 0 {
            return true;
        }
        if WORK_BYTES > (128usize * 1024 * 1024).saturating_sub(self.budget.get()) {
            self.message = "Editing needs 32 MiB of shared workspace allowance; clear a result or close another tool".into();
            return false;
        }
        self.budget.set(self.budget.get() + WORK_BYTES);
        self.work_bytes = WORK_BYTES;
        true
    }
    pub(super) fn finish_work(&mut self) {
        if self.edit.is_some() {
            return;
        }
        let retained = self.unrestored.as_ref().map_or_else(
            || self.draft.as_ref().map_or(0, MutationDraft::retained_bytes),
            MutationDraft::recovery_bytes,
        );
        self.budget.set(
            self.budget
                .get()
                .saturating_sub(self.work_bytes)
                .saturating_sub(self.draft_bytes)
                .saturating_add(retained),
        );
        self.work_bytes = 0;
        self.draft_bytes = retained;
    }
    pub(super) fn restore_intent(&mut self) -> bool {
        if self.unrestored.is_none() {
            return true;
        }
        if !self.admit_work() {
            return false;
        }
        let saved = self.unrestored.take().unwrap();
        let (result, saved) = match &self.source {
            ChangeSource::Table(relation) => {
                let state = WorkspaceTableState {
                    schema: relation.schema.clone(),
                    table: relation.table.clone(),
                    filters: vec![],
                    sort: vec![],
                    page_size: 100,
                    draft: Some(saved),
                };
                (MutationDraft::restore(&state), state.draft.unwrap())
            }
            ChangeSource::Query { provenance, .. } => {
                let state = dbunk_lib::backend::WorkspaceQueryChanges {
                    source: provenance.source.clone(),
                    draft: saved,
                };
                (MutationDraft::restore_query(&state), state.draft)
            }
        };
        let restored = match result {
            Ok(draft) => {
                self.draft = Some(draft);
                self.message = "Recovered changes require fresh analysis and review".into();
                drop(saved);
                true
            }
            Err(error) => {
                self.message = format!("Saved changes retained; recovery refused: {error:?}");
                self.unrestored = Some(saved);
                false
            }
        };
        self.finish_work();
        restored
    }
}

#[cfg(test)]
mod tests;
