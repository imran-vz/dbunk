use super::*;
use crate::{
    backend::{admit_connection, Inner},
    postgres::transfer::{native::IoContext, protocol as legacy, runner},
    safety::{
        gate,
        policy::{assert_permitted, AuditDisposition, WriteIntent},
    },
    StoredConnection,
};
use std::time::Duration;
use tokio::sync::watch;
fn policy(
    connection: &StoredConnection,
    direction: CsvDirection,
    confirmed: bool,
) -> Result<(bool, AuditDisposition), CsvError> {
    if direction == CsvDirection::Export {
        return Ok((false, AuditDisposition::NotRequired));
    }
    match assert_permitted(
        &gate::resolved_policy(connection),
        &WriteIntent::Import,
        confirmed,
    ) {
        Ok(value) => Ok((false, value.audit_disposition())),
        Err(refusal) => refusal.fold(
            |_, _| Err(CsvError::PolicyBlocked),
            |_| Ok((true, AuditDisposition::NotRequired)),
        ),
    }
}
fn target(connection: &StoredConnection) -> Result<CsvConnectionTarget, CsvError> {
    let StoredConnection::PostgreSQL(pg) = connection else {
        return Err(CsvError::UnsupportedTarget);
    };
    let target = CsvConnectionTarget {
        connection_name: pg.name.clone(),
        host: pg.host.clone(),
        port: pg.effective_port(),
        database: pg.database.clone(),
        user: pg.user.clone(),
        environment: format!("{:?}", pg.environment),
        safe_mode: format!("{:?}", pg.safe_mode),
        read_only: pg.read_only,
    };
    target.checked_heap_bytes().ok_or(CsvError::Limit)?;
    Ok(target)
}
enum Prepared {
    Workbook(Arc<CsvWorkbookData>),
    Csv(Box<PreparedCsv>),
}
struct PreparedCsv {
    core: runner::Review,
    target: CsvConnectionTarget,
    requires_confirmation: bool,
    workbook: Option<CsvWorkbookSource>,
}
pub(super) async fn inspect(inner: Arc<Inner>, id: CsvInspectionId) {
    let setup = {
        let mut state = inner.csv_transfers.state.lock().unwrap();
        let Some(entry) = state.inspections.get_mut(&id) else {
            return;
        };
        let intent = entry.intent.take().unwrap();
        if intent.xlsx {
            entry.intent = Some(intent.clone());
        }
        (
            intent,
            entry.observation.connection_id.clone(),
            entry.cancel.subscribe(),
            entry.admission.as_ref().unwrap().cancellation(),
            entry.io.clone(),
            entry.source.clone(),
            entry.workbook.clone(),
            entry.selected_sheet,
        )
    };
    let (intent, connection, mut cancel, mut lifecycle, io, source, workbook, selected) = setup;
    let mut failure_detail = None;
    let operation = async {
        let (hydrated, target, requires_confirmation) = {
            let _gate = inner.development_gate.lock().await;
            admit_connection(&inner.state, inner.development.as_deref(), &connection)
                .await
                .map_err(|_| CsvError::StaleReview)?;
            if !inner
                .csv_transfers
                .state
                .lock()
                .unwrap()
                .inspections
                .get(&id)
                .is_some_and(|e| e.admission.as_ref().is_some_and(|a| a.current()))
            {
                return Err(CsvError::StaleReview);
            }
            let stored = crate::storage::read_connection_by_id(&inner.state.pool, &connection)
                .await
                .map_err(|_| CsvError::StaleReview)?
                .ok_or(CsvError::StaleReview)?;
            let requires = policy(&stored, intent.direction, false)?.0;
            let target = target(&stored)?;
            // Workbook listing itself does not need credentials or a socket.
            let hydrated = if intent.xlsx && selected.is_none() {
                None
            } else {
                Some(
                    crate::app::find_connection(&inner.state, &connection)
                        .await
                        .map_err(|_| CsvError::Credentials)?,
                )
            };
            (hydrated, target, requires)
        };
        let mut path = intent.source;
        let mut provenance = None;
        if intent.xlsx {
            let source = source.as_ref().ok_or(CsvError::StaleReview)?;
            let permit = inner
                .csv_transfers
                .parsers
                .clone()
                .try_acquire_owned()
                .map_err(|_| CsvError::WorkBudget)?;
            if workbook.is_none() {
                let metadata = source
                    .snapshot(id, path.take().ok_or(CsvError::InvalidRequest)?, permit)
                    .await?;
                return Ok(Prepared::Workbook(metadata));
            }
            let (canonical, metadata) = source
                .materialize(
                    selected.ok_or(CsvError::InvalidRequest)?,
                    intent.options.null_token.clone(),
                    permit,
                )
                .await?;
            path = Some(canonical);
            provenance = Some(metadata);
        }
        let payload = legacy::InspectPayload {
            connection_id: connection.clone(),
            schema: intent.target.schema,
            table: intent.target.table,
            direction: direction(intent.direction),
            source_path: path.map(|path| {
                path.into_os_string()
                    .into_string()
                    .expect("validated Unicode path")
            }),
            options: intent.options.legacy(),
        };
        let hydrated = hydrated.ok_or(CsvError::StaleReview)?;
        #[cfg(test)]
        let core = {
            let inspect = inner
                .csv_transfers
                .test_inspector
                .lock()
                .unwrap()
                .clone()
                .ok_or(CsvError::InvalidRequest)?;
            inspect(hydrated, payload, io.clone())
                .await
                .map_err(|cause| {
                    failure_detail = diagnostic(&cause);
                    error(cause)
                })?
        };
        #[cfg(not(test))]
        let core = runner::inspect_in(hydrated, payload, io.clone())
            .await
            .map_err(|cause| {
                failure_detail = diagnostic(&cause);
                error(cause)
            })?;
        Ok::<_, CsvError>(Prepared::Csv(Box::new(PreparedCsv {
            core,
            target,
            requires_confirmation,
            workbook: provenance,
        })))
    };
    let result = tokio::select! {biased;_=cancel.wait_for(|v|*v)=>Err(CsvError::Cancelled),_=lifecycle.wait_for(|v|*v)=>Err(CsvError::Closing),result=tokio::time::timeout(Duration::from_secs(30),operation)=>result.map_err(|_|CsvError::Timeout).and_then(|v|v)};
    let io_cleaned = io
        .cleanup(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .is_ok();
    let result = if !io_cleaned {
        Err(CsvError::Cleanup)
    } else if *cancel.borrow() {
        Err(CsvError::Cancelled)
    } else {
        result
    };
    let source_cleaned = if !io_cleaned {
        false
    } else if result.is_err() {
        if let Some(source) = &source {
            source
                .cleanup(tokio::time::Instant::now() + Duration::from_secs(5))
                .await
                .is_ok()
        } else {
            true
        }
    } else {
        true
    };
    let mut state = inner.csv_transfers.state.lock().unwrap();
    let Some(entry) = state.inspections.get_mut(&id) else {
        return;
    };
    let result = if *entry.cancel.borrow() {
        Err(CsvError::Cancelled)
    } else {
        result
    };
    match result {
        Ok(Prepared::Workbook(data)) => {
            entry.workbook = Some(data);
            entry.workbook_permit = entry.permit.take().map(Arc::new);
            entry.observation.phase = CsvInspectionPhase::WorkbookReady;
            entry.observation.expires_at = Some(
                (chrono::Utc::now() + chrono::Duration::seconds(CSV_INSPECTION_TTL_SECONDS as i64))
                    .to_rfc3339(),
            );
            entry.created = std::time::Instant::now();
        }
        Ok(Prepared::Csv(prepared)) => {
            let PreparedCsv {
                core,
                target,
                requires_confirmation: requires,
                workbook: provenance,
            } = *prepared;
            let ready = make_ready(
                id,
                core,
                target,
                requires,
                entry.permit.take().unwrap(),
                provenance,
                entry.workbook.as_ref().map(|w| w.file_name.as_str()),
            );
            match ready.and_then(|ready| {
                let token = inner
                    .state
                    .pg_transfers
                    .insert_shared_review(
                        entry.admission.as_ref().ok_or(CsvError::StaleReview)?,
                        ready.core.clone(),
                    )
                    .map_err(error)?;
                Ok((ready, token))
            }) {
                Ok((ready, token)) => {
                    entry.observation.phase = CsvInspectionPhase::Ready;
                    entry.observation.expires_at = Some(ready.data.expires_at.clone());
                    entry.created = std::time::Instant::now();
                    entry.ready = Some(Arc::new(ready));
                    entry.token = Some(token);
                }
                Err(error) => {
                    entry.observation.phase = CsvInspectionPhase::Failed;
                    entry.observation.failure = Some(error);
                }
            }
        }
        Err(error) => {
            entry.observation.phase = if error == CsvError::Cancelled {
                CsvInspectionPhase::Cancelled
            } else {
                CsvInspectionPhase::Failed
            };
            entry.observation.failure = Some(error);
            entry.observation.diagnostic = failure_detail;
        }
    }
    entry.observation.cleanup = if io_cleaned && source_cleaned {
        CsvCleanup::Complete
    } else {
        CsvCleanup::Failed
    };
    if io_cleaned && source_cleaned {
        entry.admission.take();
        entry.permit.take();
        if matches!(
            entry.observation.phase,
            CsvInspectionPhase::Failed | CsvInspectionPhase::Cancelled
        ) {
            // Also handles cancellation that won immediately after successful preparation.
            inner.csv_transfers.cleanup_inspection(entry, id);
        }
    }
}

fn make_ready(
    id: CsvInspectionId,
    mut core: runner::Review,
    target: CsvConnectionTarget,
    requires_confirmation: bool,
    permit: tokio::sync::OwnedSemaphorePermit,
    workbook: Option<CsvWorkbookSource>,
    original_file_name: Option<&str>,
) -> Result<InspectionReady, CsvError> {
    let view = &mut core.inspection;
    let source_columns = std::mem::take(&mut view.source_columns)
        .into_iter()
        .map(|c| CsvSourceColumn {
            index: c.index,
            name: c.name,
        })
        .collect::<Vec<_>>();
    view.source_columns = (0..source_columns.len())
        .map(|index| legacy::SourceColumn {
            index,
            name: String::new(),
        })
        .collect();
    let data = CsvInspectionData {
        workbook,
        inspection_id: id,
        connection_id: view.connection_id.clone(),
        target: CsvTarget {
            schema: view.schema.clone(),
            table: view.table.clone(),
        },
        direction: if view.direction == legacy::Direction::Import {
            CsvDirection::Import
        } else {
            CsvDirection::Export
        },
        file_name: original_file_name
            .map(str::to_owned)
            .or_else(|| view.file_name.take()),
        total_bytes: view.total_bytes,
        source_columns,
        target_columns: std::mem::take(&mut view.target_columns)
            .into_iter()
            .map(|c| CsvTargetColumn {
                name: c.name,
                data_type: c.data_type,
                nullable: c.nullable,
                has_default: c.has_default,
                generated: c.generated,
                identity: c.identity,
            })
            .collect(),
        sample_rows: std::mem::take(&mut view.sample_rows),
        sample_truncated: view.sample_truncated,
        options: CsvOptions {
            delimiter: view.options.delimiter.clone(),
            quote: view.options.quote.clone(),
            escape: view.options.escape.clone(),
            null_token: view.options.null_token.clone(),
            header: view.options.header,
        },
        connection: target,
        expires_at: (chrono::Utc::now()
            + chrono::Duration::seconds(CSV_INSPECTION_TTL_SECONDS as i64))
        .to_rfc3339(),
    };
    let retained = data
        .checked_heap_bytes()
        .and_then(|n| n.checked_add(core.private_heap_bytes()))
        .and_then(|n| {
            n.checked_add(
                std::mem::size_of::<InspectionReady>() + std::mem::size_of::<CsvInspection>() + 64,
            )
        })
        .filter(|n| *n <= MAX_CSV_INSPECTION_BYTES)
        .ok_or(CsvError::Limit)?;
    // Only small metadata remains in the core view; the sample vectors moved to
    // the public immutable capture. The runner shares this core through Arc.
    let moved = core;
    Ok(InspectionReady {
        data,
        core: Arc::new(moved),
        requires_confirmation,
        retained,
        _permit: permit,
    })
}
pub(super) async fn dispatch(inner: Arc<Inner>, id: CsvTransferAttemptId, confirmed: bool) {
    let prepared = dispatch_inner(&inner, id, confirmed).await;
    if let Err(failure) = prepared {
        finish_pre_dispatch(&inner, id, failure);
        return;
    }
    monitor(inner, id).await;
}
async fn dispatch_inner(
    inner: &Arc<Inner>,
    id: CsvTransferAttemptId,
    confirmed: bool,
) -> Result<(), CsvError> {
    let (review, mut cancel, mut lifecycle) = {
        let state = inner.csv_transfers.state.lock().unwrap();
        let entry = state.jobs.get(&id).ok_or(CsvError::Missing)?;
        (
            entry.review.clone().ok_or(CsvError::StaleReview)?,
            entry.cancel.subscribe(),
            entry
                .admission
                .as_ref()
                .ok_or(CsvError::StaleReview)?
                .cancellation(),
        )
    };
    let connection = review.inspection().data().connection_id.clone();
    let direction = review.direction();
    let preparation = async {
        let gate = inner.development_gate.lock().await;
        admit_connection(&inner.state, inner.development.as_deref(), &connection)
            .await
            .map_err(|_| CsvError::StaleReview)?;
        if !inner
            .csv_transfers
            .state
            .lock()
            .unwrap()
            .jobs
            .get(&id)
            .is_some_and(|e| e.admission.as_ref().is_some_and(|a| a.current()))
        {
            return Err(CsvError::StaleReview);
        }
        if direction == CsvDirection::Import {
            if inner.tool_jobs.restore_in_progress(&connection)
                || crate::backend::table_copy::destination_write_in_progress(inner, &connection)
                || crate::backend::table_seed::destination_write_in_progress(inner, &connection)
            {
                return Err(CsvError::Busy);
            }
            inner
                .csv_transfers
                .state
                .lock()
                .unwrap()
                .jobs
                .get_mut(&id)
                .ok_or(CsvError::Missing)?
                .import_reserved = true;
        }
        let stored = crate::storage::read_connection_by_id(&inner.state.pool, &connection)
            .await
            .map_err(|_| CsvError::StaleReview)?
            .ok_or(CsvError::StaleReview)?;
        let (needs, audit) = policy(&stored, direction, confirmed)?;
        if needs {
            return Err(CsvError::StaleReview);
        }
        let hydrated = crate::app::find_connection(&inner.state, &connection)
            .await
            .map_err(|_| CsvError::Credentials)?;
        Ok::<_, CsvError>((gate, hydrated, audit))
    };
    let (gate, hydrated, audit) = tokio::select! {biased;_=cancel.wait_for(|v|*v)=>return Err(CsvError::Cancelled),_=lifecycle.wait_for(|v|*v)=>return Err(CsvError::Closing),value=tokio::time::timeout(Duration::from_secs(30),preparation)=>value.map_err(|_|CsvError::Timeout)??};
    if direction == CsvDirection::Import {
        crate::backend::data::retire_data(inner, &inner.state, Some(&connection))
            .await
            .map_err(|_| CsvError::Cleanup)?;
    }
    let request = match direction {
        CsvDirection::Import => runner::RunRequest::Import {
            mapping: review
                .mapping()
                .iter()
                .map(|m| legacy::ColumnMapping {
                    source_index: m.source_index,
                    target_column: m.target_column.clone(),
                })
                .collect(),
        },
        CsvDirection::Export => runner::RunRequest::Export {
            destination_path: review
                .data
                .destination
                .as_ref()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned(),
        },
    };
    let core = review.inspection().ready.core.clone();
    let state = inner.state.clone();
    let completed_connection = connection.clone();
    let completion = Box::pin(async move {
        if audit == AuditDisposition::RequiredAfterSuccess {
            gate::record_override(
                &state.pool,
                &completed_connection,
                "start_pg_csv_import",
                &WriteIntent::Import,
            )
            .await;
        }
        let _ = crate::storage::touch_connection_activity(&state.pool, &completed_connection).await;
    });
    let mut registry = inner.csv_transfers.state.lock().unwrap();
    let entry = registry.jobs.get_mut(&id).ok_or(CsvError::Missing)?;
    if *entry.cancel.borrow() {
        return Err(CsvError::Cancelled);
    }
    let snapshot = legacy::Snapshot {
        job_id: String::new(),
        connection_id: connection,
        schema: entry.observation.target.schema.clone(),
        table: entry.observation.target.table.clone(),
        direction: direction.into(),
        file_name: entry.observation.file_name.clone(),
        phase: legacy::Phase::Preparing,
        started_at: entry.observation.started_at.clone(),
        finished_at: None,
        total_bytes: entry.observation.total_bytes,
        bytes_processed: 0,
        rows_processed: None,
        rows_committed: None,
        failure: None,
    };
    let admission = entry.admission.take().ok_or(CsvError::StaleReview)?;
    let source_owner = entry.source.clone();
    #[cfg(test)]
    let injected = inner.csv_transfers.test_runner.lock().unwrap().clone();
    let accepted = inner
        .state
        .pg_transfers
        .start(
            admission,
            &entry.token,
            snapshot,
            move |context| async move {
                let _source_owner = source_owner;
                #[cfg(test)]
                {
                    match injected {
                        Some(run) => run(context, hydrated, core, request).await,
                        None => Err(legacy::TransferError::invalid(
                            "test",
                            "No injected CSV runner",
                        )),
                    }
                }
                #[cfg(not(test))]
                {
                    runner::run(context, hydrated, core, request).await
                }
            },
            completion,
        )
        .map_err(error)?;
    entry.job_id = Some(accepted.job_id);
    entry.review.take();
    entry.observation.effect = CsvEffect::Pending;
    drop(registry);
    drop(gate);
    Ok(())
}
fn finish_pre_dispatch(inner: &Inner, id: CsvTransferAttemptId, error: CsvError) {
    let mut state = inner.csv_transfers.state.lock().unwrap();
    if let Some(entry) = state.jobs.get_mut(&id) {
        entry.observation.phase = if error == CsvError::Cancelled {
            CsvTransferPhase::Cancelled
        } else {
            CsvTransferPhase::Failed
        };
        entry.observation.failure = Some(error);
        entry.observation.finished_at = Some(chrono::Utc::now().to_rfc3339());
        entry.finished = Some(std::time::Instant::now());
        entry.review.take();
        inner.state.pg_transfers.release_review(&entry.token);
        if error == CsvError::Cleanup {
            entry.observation.cleanup = CsvCleanup::Failed;
        } else {
            if entry.source.is_none() {
                entry.admission.take();
                entry.execution.take();
            }
            entry.observation.cleanup = CsvCleanup::Complete;
            inner.csv_transfers.cleanup_job(entry, id);
        }
    }
}
async fn monitor(inner: Arc<Inner>, id: CsvTransferAttemptId) {
    loop {
        let job = {
            let state = inner.csv_transfers.state.lock().unwrap();
            let Some(entry) = state.jobs.get(&id) else {
                return;
            };
            entry.job_id.clone().unwrap()
        };
        let Some((snapshot, _io)) = inner.state.pg_transfers.observation(&job) else {
            return;
        };
        let terminal = matches!(
            snapshot.phase,
            legacy::Phase::Completed
                | legacy::Phase::Cancelled
                | legacy::Phase::Failed
                | legacy::Phase::OutcomeUnknown
        );
        let retired = if terminal {
            let imported = {
                let state = inner.csv_transfers.state.lock().unwrap();
                state.jobs.get(&id).is_some_and(|e| e.import_reserved)
            };
            let retired = if imported {
                let _gate = inner.development_gate.lock().await;
                crate::backend::data::retire_data(
                    &inner,
                    &inner.state,
                    Some(&snapshot.connection_id),
                )
                .await
                .is_ok()
            } else {
                true
            };
            retired
        } else {
            true
        };
        {
            let mut state = inner.csv_transfers.state.lock().unwrap();
            let Some(entry) = state.jobs.get_mut(&id) else {
                return;
            };
            entry.observation.phase = phase(snapshot.phase);
            entry.observation.bytes_processed = snapshot.bytes_processed;
            entry.observation.rows_processed = snapshot.rows_processed;
            entry.observation.rows_committed = snapshot.rows_committed;
            entry.observation.failure = snapshot.failure.clone().map(error);
            entry.observation.diagnostic = snapshot.failure.as_ref().and_then(diagnostic);
            entry.observation.effect = if snapshot.phase == legacy::Phase::Completed {
                CsvEffect::Succeeded
            } else if snapshot.phase == legacy::Phase::OutcomeUnknown {
                CsvEffect::Unknown
            } else if terminal {
                CsvEffect::NotApplied
            } else {
                CsvEffect::Pending
            };
            entry.observation.finished_at = snapshot.finished_at.clone();
            if terminal {
                entry.finished = Some(std::time::Instant::now());
            }
            let changed = terminal
                && entry.observation.direction == CsvDirection::Import
                && matches!(
                    entry.observation.effect,
                    CsvEffect::Succeeded | CsvEffect::Unknown
                )
                && entry.observation.import_change_revision.is_none();
            if changed {
                state.revision = state
                    .revision
                    .checked_add(1)
                    .expect("bounded process lifetime CSV revision");
                let revision = state.revision;
                state
                    .jobs
                    .get_mut(&id)
                    .unwrap()
                    .observation
                    .import_change_revision = Some(revision);
            }
        }
        if terminal {
            let mut cleaned = inner
                .state
                .pg_transfers
                .settle_native(&job, tokio::time::Instant::now() + Duration::from_secs(5))
                .await
                .is_ok()
                && retired;
            let source = inner
                .csv_transfers
                .state
                .lock()
                .unwrap()
                .jobs
                .get(&id)
                .and_then(|e| e.source.clone());
            if cleaned {
                if let Some(source) = &source {
                    cleaned = source
                        .cleanup(tokio::time::Instant::now() + Duration::from_secs(5))
                        .await
                        .is_ok();
                }
            }
            let mut state = inner.csv_transfers.state.lock().unwrap();
            if let Some(entry) = state.jobs.get_mut(&id) {
                entry.observation.cleanup = if cleaned {
                    CsvCleanup::Complete
                } else {
                    CsvCleanup::Failed
                };
                if cleaned {
                    entry.execution.take();
                    entry.admission.take();
                    entry.source.take();
                } else if entry.observation.failure.is_none() {
                    entry.observation.failure = Some(CsvError::Cleanup);
                }
            }
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
fn direction(value: CsvDirection) -> legacy::Direction {
    match value {
        CsvDirection::Import => legacy::Direction::Import,
        CsvDirection::Export => legacy::Direction::Export,
    }
}
impl From<CsvDirection> for legacy::Direction {
    fn from(value: CsvDirection) -> Self {
        direction(value)
    }
}
fn phase(value: legacy::Phase) -> CsvTransferPhase {
    match value {
        legacy::Phase::Preparing => CsvTransferPhase::Preparing,
        legacy::Phase::Running => CsvTransferPhase::Running,
        legacy::Phase::Cancelling => CsvTransferPhase::Cancelling,
        legacy::Phase::Finalizing => CsvTransferPhase::Finalizing,
        legacy::Phase::Completed => CsvTransferPhase::Completed,
        legacy::Phase::Cancelled => CsvTransferPhase::Cancelled,
        legacy::Phase::Failed => CsvTransferPhase::Failed,
        legacy::Phase::OutcomeUnknown => CsvTransferPhase::OutcomeUnknown,
    }
}
pub(super) fn error(value: legacy::TransferError) -> CsvError {
    use legacy::TransferError as E;
    match value {
        E::UnsupportedEngine | E::UnsupportedTarget { .. } => CsvError::UnsupportedTarget,
        E::InvalidRequest { field, .. } => {
            if field == "metadata" {
                CsvError::Limit
            } else {
                CsvError::InvalidRequest
            }
        }
        E::ConnectionClosing => CsvError::Closing,
        E::JobLimitReached => CsvError::Busy,
        E::JobNotFound => CsvError::Missing,
        E::JobActive => CsvError::Active,
        E::InspectionExpired => CsvError::InspectionExpired,
        E::SourceChanged => CsvError::SourceChanged,
        E::TargetChanged => CsvError::TargetChanged,
        E::DestinationExists => CsvError::DestinationExists,
        E::Csv { .. } => CsvError::Csv,
        E::Database { .. } => CsvError::Database,
        E::Io { .. } => CsvError::FileIo,
        E::Timeout { .. } => CsvError::Timeout,
        E::PolicyBlocked { .. } | E::PolicyNeedsConfirmation { .. } => CsvError::PolicyBlocked,
        E::Cancelled => CsvError::Cancelled,
        E::OutcomeUnknown => CsvError::Database,
        E::ExportLimitExceeded { .. } => CsvError::Limit,
    }
}
fn diagnostic(value: &legacy::TransferError) -> Option<CsvDiagnostic> {
    use legacy::TransferError as E;
    match value {
        E::Csv { reason, .. } | E::Database { reason, .. } | E::Io { reason, .. }
            if reason.len() > 512 =>
        {
            return None
        }
        E::Timeout { operation } | E::Io { operation, .. } if operation.len() > 64 => return None,
        E::Database {
            code: Some(code), ..
        } if code.len() != 5 => return None,
        _ => {}
    }
    let mut d = CsvDiagnostic {
        record: None,
        column: None,
        sqlstate: None,
        operation: None,
        reason: String::new(),
        export_limit: None,
    };
    match value {
        E::Csv {
            record,
            column,
            reason,
        } => {
            d.record = Some(*record);
            d.column = *column;
            d.reason = reason.clone();
        }
        E::Database { code, reason } => {
            d.sqlstate = code.clone();
            d.reason = reason.clone();
        }
        E::Io { operation, reason } => {
            d.operation = Some(operation.clone());
            d.reason = reason.clone();
        }
        E::Timeout { operation } => {
            d.operation = Some(operation.clone());
            d.reason = "Operation timed out".into();
        }
        E::ExportLimitExceeded { limit } => {
            d.export_limit = Some(match limit {
                legacy::ExportLimit::Field => CsvExportLimit::Field,
                legacy::ExportLimit::Record => CsvExportLimit::Record,
            });
            d.reason = "Server output exceeds the bounded CSV field or record limit".into();
        }
        _ => return None,
    }
    d.checked_heap_bytes()?;
    Some(d)
}
