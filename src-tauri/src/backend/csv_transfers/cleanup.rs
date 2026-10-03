//! Async unlink ownership for terminal/pre-dispatch paths that have no driver.
use super::*;
use registry::{InspectionEntry, JobEntry, Registry};
impl Registry {
    pub(super) fn cleanup_inspection(&self, entry: &mut InspectionEntry, id: CsvInspectionId) {
        let Some(source) = entry.source.clone() else {
            entry.permit.take();
            entry.admission.take();
            entry.observation.cleanup = CsvCleanup::Complete;
            return;
        };
        source.cancel();
        entry.observation.cleanup = CsvCleanup::Pending;
        let state = self.state.clone();
        let revision = entry.revision;
        if self
            .owner
            .spawn(async move {
                let cleaned = source
                    .cleanup(tokio::time::Instant::now() + std::time::Duration::from_secs(5))
                    .await
                    .is_ok();
                let mut state = state.lock().unwrap();
                if let Some(e) = state
                    .inspections
                    .get_mut(&id)
                    .filter(|e| e.revision == revision)
                {
                    e.observation.cleanup = if cleaned {
                        CsvCleanup::Complete
                    } else {
                        CsvCleanup::Failed
                    };
                    if cleaned {
                        e.source.take();
                        e.permit.take();
                        e.admission.take();
                        e.workbook_permit.take();
                    }
                }
            })
            .is_err()
        {
            entry.observation.cleanup = CsvCleanup::Failed;
        }
    }
    pub(super) fn cleanup_job(&self, entry: &mut JobEntry, id: CsvTransferAttemptId) {
        let Some(source) = entry.source.clone() else {
            return;
        };
        source.cancel();
        entry.observation.cleanup = CsvCleanup::Pending;
        let state = self.state.clone();
        if self
            .owner
            .spawn(async move {
                let cleaned = source
                    .cleanup(tokio::time::Instant::now() + std::time::Duration::from_secs(5))
                    .await
                    .is_ok();
                let mut state = state.lock().unwrap();
                if let Some(e) = state
                    .jobs
                    .get_mut(&id)
                    .filter(|e| e.source.as_ref().is_some_and(|s| Arc::ptr_eq(s, &source)))
                {
                    e.observation.cleanup = if cleaned {
                        CsvCleanup::Complete
                    } else {
                        CsvCleanup::Failed
                    };
                    if cleaned {
                        e.source.take();
                        e.execution.take();
                        e.admission.take();
                    }
                }
            })
            .is_err()
        {
            entry.observation.cleanup = CsvCleanup::Failed;
        }
    }
}
