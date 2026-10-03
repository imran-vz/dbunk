//! Profile mutations retire CSV work before changing credentials or endpoints.
use super::*;
use crate::backend::Inner;
use std::sync::Mutex;
use tokio::time::{Duration, Instant};

pub(crate) struct RetirementGuard(Arc<Mutex<registry::State>>);
impl Drop for RetirementGuard {
    fn drop(&mut self) {
        self.0.lock().unwrap().retiring.take();
    }
}

/// Caller holds development_gate and retains this guard through the following
/// metadata/credential mutation and ordinary manager fence. Synchronous CSV
/// registration is refused for the whole interval. Failure leaves actual work
/// and its reservations owned, and prevents the caller's mutation.
pub(in crate::backend) async fn retire_connection(
    inner: &Arc<Inner>,
    connection: Option<&str>,
    deadline: Instant,
) -> Result<RetirementGuard, String> {
    let registry = &inner.csv_transfers;
    let (guard, inspections, jobs) = {
        let mut state = registry.state.lock().unwrap();
        if state.retiring.is_some() {
            return Err("CSV profile retirement is already active".into());
        }
        state.retiring = Some(connection.map(str::to_owned));
        let inspections = state
            .inspections
            .iter()
            .filter(|(_, e)| connection.is_none_or(|id| id == e.observation.connection_id))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let jobs = state
            .jobs
            .iter()
            .filter(|(_, e)| connection.is_none_or(|id| id == e.observation.connection_id))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        (RetirementGuard(registry.state.clone()), inspections, jobs)
    };
    for id in &inspections {
        let _ = registry.cancel_inspection(&inner.state.pg_transfers, *id);
    }
    for id in &jobs {
        let _ = registry.cancel(&inner.state.pg_transfers, *id);
    }
    let settle = async {
        loop {
            let mut pending = false;
            let core = {
                let state = registry.state.lock().unwrap();
                for id in &inspections {
                    if let Some(entry) = state.inspections.get(id) {
                        match entry.observation.cleanup {
                            CsvCleanup::Failed => return Err(()),
                            CsvCleanup::Pending => pending = true,
                            CsvCleanup::Complete => {}
                        }
                    }
                }
                let mut core = Vec::with_capacity(jobs.len());
                for id in &jobs {
                    if let Some(entry) = state.jobs.get(id) {
                        if entry.observation.cleanup == CsvCleanup::Failed {
                            return Err(());
                        }
                        if entry.observation.cleanup == CsvCleanup::Complete {
                            continue;
                        }
                        if let Some(id) = &entry.job_id {
                            core.push((id.clone(), entry.source.clone()));
                        } else {
                            pending = true;
                        }
                    }
                }
                core
            };
            // The core worker does not take development_gate. The facade monitor
            // may be waiting for it and is deliberately excluded from this join.
            for (id, source) in core {
                inner
                    .state
                    .pg_transfers
                    .settle_native(&id, deadline)
                    .await?;
                if let Some(source) = source {
                    source.cleanup(deadline).await.map_err(|_| ())?;
                }
            }
            if !pending {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout_at(deadline, settle)
        .await
        .map_err(|_| {
            "CSV cleanup did not finish; connection and credentials were not changed".to_string()
        })?
        .map_err(|_| {
            "CSV cleanup failed; connection and credentials were not changed".to_string()
        })?;
    Ok(guard)
}
