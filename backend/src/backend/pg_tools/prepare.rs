use super::registry::map_error;
use super::*;
use crate::{
    backend::{admit_connection, Inner},
    postgres::backup::{native::source, protocol as legacy, runner},
    safety::{
        gate,
        policy::{assert_permitted, AuditDisposition, WriteIntent},
    },
    StoredConnection,
};

fn policy(
    connection: &StoredConnection,
    kind: PgToolKind,
    confirmed: bool,
) -> Result<(bool, AuditDisposition), PgToolError> {
    if kind == PgToolKind::Backup {
        return Ok((false, AuditDisposition::NotRequired));
    }
    match assert_permitted(
        &gate::resolved_policy(connection),
        &WriteIntent::Restore,
        confirmed,
    ) {
        Ok(authorization) => Ok((false, authorization.audit_disposition())),
        Err(refusal) => refusal.fold(
            |_, _| Err(PgToolError::PolicyBlocked),
            |_| Ok((true, AuditDisposition::NotRequired)),
        ),
    }
}
fn target(connection: &StoredConnection) -> Result<PgToolTarget, PgToolError> {
    let StoredConnection::PostgreSQL(pg) = connection else {
        return Err(PgToolError::InvalidRequest);
    };
    let target = PgToolTarget {
        connection_name: pg.name.clone(),
        host: pg.host.clone(),
        port: pg.effective_port(),
        database: pg.database.clone(),
        user: pg.user.clone(),
        environment: format!("{:?}", pg.environment),
        safe_mode: format!("{:?}", pg.safe_mode),
        read_only: pg.read_only,
    };
    target
        .checked_heap_bytes()
        .ok_or(PgToolError::InvalidRequest)?;
    Ok(target)
}
pub(super) async fn prepare(inner: Arc<Inner>, registry: Registry, id: PgToolAttemptId) {
    let result = prepare_inner(&inner, &registry, id).await;
    if let Err(error) = result {
        registry.finish(id, error).await;
    }
}
async fn prepare_inner(
    inner: &Arc<Inner>,
    registry: &Registry,
    id: PgToolAttemptId,
) -> Result<(), PgToolError> {
    let (connection, intent, cancel, work) = registry.update(id, |e| {
        (
            e.observation.connection_id.clone(),
            e.intent.take(),
            e.copy_cancel.clone(),
            e.work.clone(),
        )
    })?;
    let intent = intent.ok_or(PgToolError::StaleReview)?;
    let mut cancel_wait = registry.update(id, |e| e.cancel.subscribe())?;
    let (target, requires_confirmation) = {
        let _gate = tokio::select! {biased;_=cancel_wait.wait_for(|cancelled|*cancelled)=>return Err(PgToolError::Cancelled),gate=inner.development_gate.lock()=>gate};
        if cancel.cancelled() {
            return Err(PgToolError::Cancelled);
        }
        admit_connection(&inner.state, inner.development.as_deref(), &connection)
            .await
            .map_err(|_| PgToolError::StaleReview)?;
        if !registry.update(id, |e| e.admission.as_ref().is_some_and(|a| a.current()))? {
            return Err(PgToolError::StaleReview);
        }
        let stored = crate::storage::read_connection_by_id(&inner.state.pool, &connection)
            .await
            .map_err(|_| PgToolError::StaleReview)?
            .ok_or(PgToolError::StaleReview)?;
        (target(&stored)?, policy(&stored, intent.kind, false)?.0)
    };
    let path = intent.path.clone();
    let copy_cancel = cancel.clone();
    let source = if intent.kind == PgToolKind::Restore {
        let value = work
            .spawn_blocking(move || source::copy(&path, &copy_cancel))
            .map_err(|_| PgToolError::Busy)?
            .await
            .map_err(|_| PgToolError::Cleanup)?
            .map_err(|_| PgToolError::Cleanup)?
            .map_err(|failure| {
                if let Some(partial) = failure.partial {
                    let _ = registry.update(id, |entry| entry.source = Some(Arc::new(partial)));
                }
                match failure.error {
                    source::Error::Io => PgToolError::FileIo,
                    source::Error::Changed => PgToolError::SourceChanged,
                    source::Error::Limit => PgToolError::InvalidRequest,
                    source::Error::Cancelled => PgToolError::Cancelled,
                    source::Error::Cleanup => PgToolError::Cleanup,
                }
            })?;
        Some(Arc::new(value))
    } else {
        work.spawn_blocking(move || {
            match std::fs::symlink_metadata(&path) {
                Ok(_) => return Err(PgToolError::DestinationExists),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(PgToolError::FileIo),
            }
            if !std::fs::metadata(path.parent().ok_or(PgToolError::InvalidPath)?)
                .map_err(|_| PgToolError::FileIo)?
                .is_dir()
            {
                return Err(PgToolError::InvalidPath);
            }
            Ok(())
        })
        .map_err(|_| PgToolError::Busy)?
        .await
        .map_err(|_| PgToolError::Cleanup)?
        .map_err(|_| PgToolError::Cleanup)??;
        None
    };
    registry.update(id, |e| {
        e.source = source;
        e.intent = Some(intent);
        e.target = Some(target);
        e.requires_confirmation = requires_confirmation;
        e.observation.source_bytes = e.source.as_ref().map(|s| s.bytes);
        if cancel.cancelled() {
            return Err(PgToolError::Cancelled);
        }
        if !e.admission.as_ref().is_some_and(|a| a.current()) {
            return Err(PgToolError::StaleReview);
        }
        e.observation.phase = PgToolPhase::ReadyReview;
        Ok(())
    })?
}
pub(super) async fn dispatch(
    inner: Arc<Inner>,
    registry: Registry,
    id: PgToolAttemptId,
    confirmed: bool,
) {
    let result = dispatch_inner(&inner, &registry, id, confirmed).await;
    if let Err(error) = result {
        registry.finish(id, error).await;
    }
}
async fn dispatch_inner(
    inner: &Arc<Inner>,
    registry: &Registry,
    id: PgToolAttemptId,
    confirmed: bool,
) -> Result<(), PgToolError> {
    let (connection, kind, cancel) = registry.update(id, |e| {
        (
            e.observation.connection_id.clone(),
            e.observation.kind,
            e.cancel.subscribe(),
        )
    })?;
    let preparation = async {
        let gate = inner.development_gate.lock().await;
        if *cancel.borrow() {
            return Err(PgToolError::Cancelled);
        }
        admit_connection(&inner.state, inner.development.as_deref(), &connection)
            .await
            .map_err(|_| PgToolError::StaleReview)?;
        if !registry.update(id, |e| e.admission.as_ref().is_some_and(|a| a.current()))? {
            return Err(PgToolError::StaleReview);
        }
        let stored = crate::storage::read_connection_by_id(&inner.state.pool, &connection)
            .await
            .map_err(|_| PgToolError::StaleReview)?
            .ok_or(PgToolError::StaleReview)?;
        let (needs_confirmation, audit) = policy(&stored, kind, confirmed)?;
        if needs_confirmation {
            return Err(PgToolError::StaleReview);
        }
        if kind == PgToolKind::Restore
            && (inner.csv_transfers.import_in_progress(&connection)
                || crate::backend::table_copy::destination_write_in_progress(inner, &connection)
                || crate::backend::table_seed::destination_write_in_progress(inner, &connection))
        {
            return Err(PgToolError::Busy);
        }
        let hydrated = crate::app::find_connection(&inner.state, &connection)
            .await
            .map_err(|_| PgToolError::Credentials)?;
        Ok((gate, hydrated, audit))
    };
    tokio::pin!(preparation);
    let cancellation = cancel.clone();
    let (gate, hydrated, audit) = tokio::select! {biased;_=async{let mut cancel=cancellation;let _=cancel.wait_for(|cancelled|*cancelled).await;}=>return Err(PgToolError::Cancelled),value=&mut preparation=>value?};
    if kind == PgToolKind::Restore {
        // Once this retirement begins it must settle; cancellation is checked
        // again at manager admission, never by dropping this teardown future.
        crate::backend::data::retire_data(inner, &inner.state, Some(&connection))
            .await
            .map_err(|_| PgToolError::Cleanup)?;
    }
    let state = inner.state.clone();
    let completed_connection = connection.clone();
    let completion = Box::pin(async move {
        if audit == AuditDisposition::RequiredAfterSuccess {
            gate::record_override(
                &state.pool,
                &completed_connection,
                "start_pg_restore",
                &WriteIntent::Restore,
            )
            .await;
        }
        let _ = crate::storage::touch_connection_activity(&state.pool, &completed_connection).await;
    });
    #[cfg(test)]
    let test_runner = registry.test_runner.lock().unwrap().clone();
    // Registry cancellation and manager start have one short mutex fence. No
    // filesystem, credential or network operation holds this registry lock.
    registry.update(id, |e| {
        if *e.cancel.borrow() {
            return Err(PgToolError::Cancelled);
        }
        let admission = e.admission.take().ok_or(PgToolError::StaleReview)?;
        let intent = e.intent.as_ref().ok_or(PgToolError::StaleReview)?;
        let request = request(intent, &connection, e.source.as_deref(), confirmed)?;
        let mut snapshot = request.snapshot();
        snapshot.file_name = e.observation.file_name.clone();
        let source = e.source.clone();
        let job = inner
            .state
            .pg_tool_jobs
            .start(
                admission,
                snapshot,
                move |context| async move {
                    let _source = source;
                    #[cfg(test)]
                    {
                        let run = test_runner.ok_or_else(|| {
                            legacy::PgToolJobError::invalid(
                                "test",
                                "Native process execution requires an explicit injected runner",
                            )
                        })?;
                        run(context, hydrated, request).await
                    }
                    #[cfg(not(test))]
                    runner::run(context, hydrated, request).await
                },
                completion,
            )
            .map_err(map_error)?;
        e.job_id = Some(job.job_id);
        Ok(())
    })??;
    drop(gate);
    Ok(())
}
fn request(
    intent: &PgToolIntent,
    connection: &str,
    source: Option<&source::Snapshot>,
    confirmed: bool,
) -> Result<runner::Request, PgToolError> {
    let format = match intent.format {
        PgToolFormat::Plain => legacy::PgBackupFormat::Plain,
        PgToolFormat::Custom => legacy::PgBackupFormat::Custom,
    };
    Ok(match intent.kind {
        PgToolKind::Backup => runner::Request::Backup(legacy::StartPgBackupPayload {
            connection_id: connection.into(),
            destination_path: intent.path.to_str().ok_or(PgToolError::InvalidPath)?.into(),
            format,
            scope: match &intent.scope {
                PgToolScope::Database => legacy::PgBackupScope::Database,
                PgToolScope::Schema { schema } => legacy::PgBackupScope::Schema {
                    schema: schema.clone(),
                },
                PgToolScope::Table { schema, table } => legacy::PgBackupScope::Table {
                    schema: schema.clone(),
                    table: table.clone(),
                },
            },
            clean: intent.clean,
        }),
        PgToolKind::Restore => runner::Request::Restore(legacy::StartPgRestorePayload {
            connection_id: connection.into(),
            source_path: source
                .ok_or(PgToolError::StaleReview)?
                .file
                .path()
                .to_str()
                .ok_or(PgToolError::InvalidPath)?
                .into(),
            format,
            clean: intent.clean,
            confirmed,
        }),
    })
}
