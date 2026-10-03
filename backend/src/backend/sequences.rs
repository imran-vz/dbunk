//! Native sequence inspect and explicit Advance/Set/Restart writes.
//!
//! Inspection never calls `nextval`. Every write is minted from one backend
//! observation, regenerated against stored policy at apply time, guarded by the
//! observed OIDs/names/definition, and never retried after dispatch. Outcomes
//! are not journaled across restarts: an unknown outcome is disclosed in-session
//! and the caller must inspect again before another write.
mod types;
pub(crate) use super::data_documents::WritePermit;
use super::{
    data::{DataDocument, DataError},
    objects::{CatalogError, PgObjectKind, PgObjectRef},
    Backend,
};
use crate::postgres::{
    connect_spec::ResolvedPostgresConnectSpec, dedicated::DriverJoins, native_sequences,
};
use crate::safety::{
    gate,
    policy::{assert_permitted, AuditDisposition},
};
use futures_util::future::BoxFuture;
use std::{future::Future, sync::Arc};
use tokio::sync::watch;
pub use types::*;

/// Shared guard: the effect row exists only while the database, namespace and
/// sequence OIDs, their names and the stored definition match the observation.
macro_rules! guard {
    () => {
        "FROM pg_catalog.pg_class c\n  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\n  JOIN pg_catalog.pg_sequence s ON s.seqrelid = c.oid\nWHERE c.oid = $1 AND c.relkind = 'S' AND n.oid = $2\n  AND n.nspname::text = $3 AND c.relname::text = $4\n  AND (SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname = pg_catalog.current_database()) = $5\n  AND pg_catalog.format_type(s.seqtypid, NULL) = $6\n  AND s.seqstart = $7 AND s.seqincrement = $8 AND s.seqmin = $9\n  AND s.seqmax = $10 AND s.seqcache = $11 AND s.seqcycle = $12"
    };
}
pub(crate) const LOCK_TIMEOUT_SQL: &str = "SET lock_timeout = '10s'";
pub(crate) const RESTART_BEGIN_SQL: &str = "BEGIN; SET LOCAL lock_timeout = '10s'";
pub(crate) const ADVANCE_SQL: &str = concat!(
    "SELECT pg_catalog.nextval(c.oid::pg_catalog.regclass) AS value\n",
    guard!()
);
pub(crate) const SET_SQL: &str = concat!(
    "SELECT pg_catalog.setval(c.oid::pg_catalog.regclass, $13::pg_catalog.int8, $14::pg_catalog.bool) AS value\n",
    guard!()
);
pub(crate) const GUARD_SQL: &str = concat!("SELECT 1::pg_catalog.int8 AS value\n", guard!());

pub(crate) fn restart_sql(target: &SequenceTarget, with: Option<i64>) -> String {
    match with {
        Some(value) => format!("ALTER SEQUENCE {} RESTART WITH {value}", target.qualified()),
        None => format!("ALTER SEQUENCE {} RESTART", target.qualified()),
    }
}

fn guard_parameters(observation: &SequenceObservation) -> Vec<String> {
    let t = &observation.target;
    let d = &observation.definition;
    vec![
        format!("{} (sequence OID)", t.sequence_oid),
        format!("{} (schema OID)", t.namespace_oid),
        format!("{:?} (schema name)", t.schema),
        format!("{:?} (sequence name)", t.name),
        format!("{} (database OID)", t.database_oid),
        format!("{:?} (data type)", d.data_type.sql()),
        format!("{} (start)", d.start),
        format!("{} (increment)", d.increment),
        format!("{} (minimum)", d.min_value),
        format!("{} (maximum)", d.max_value),
        format!("{} (cache)", d.cache),
        format!("{} (cycle)", d.cycle),
    ]
}

fn observed_next(observation: &SequenceObservation) -> Option<i64> {
    match observation.value {
        SequenceValue::Read {
            last_value,
            is_called,
        } => observation.definition.next_after(last_value, is_called),
        SequenceValue::NotReadable => None,
    }
}

/// Moving the next value against the increment direction can reissue values
/// that existing rows already use.
fn rewind_warning(observation: &SequenceObservation, next: i64) -> &'static str {
    let backwards = observed_next(observation).is_some_and(|current| {
        if observation.definition.increment > 0 {
            next < current
        } else {
            next > current
        }
    });
    if backwards {
        " Warning: this moves the sequence backwards relative to the observed value; future values may collide with existing rows."
    } else {
        ""
    }
}

pub(crate) fn preview(
    observation: &SequenceObservation,
    intent: SequenceIntent,
    statement_timeout_ms: Option<u32>,
) -> Result<SequencePreview, SequenceError> {
    if !observation.valid() {
        return Err(SequenceError::InvalidTarget);
    }
    let definition = &observation.definition;
    let target = &observation.target;
    let regclass = format!(
        "{}::pg_catalog.regclass",
        crate::quote_literal(&target.qualified())
    );
    let mut parameters = guard_parameters(observation);
    let (summary, sql, effect) = match intent {
        SequenceIntent::Advance => (
            format!("SELECT pg_catalog.nextval({regclass})"),
            format!("{LOCK_TIMEOUT_SQL}\n{ADVANCE_SQL}"),
            match observation.value {
                SequenceValue::Read {
                    last_value,
                    is_called,
                } => format!(
                    "Consumes one value. From the observation, {}. Other sessions may consume values first; the returned value is reported.",
                    definition.describe_next(last_value, is_called)
                ),
                SequenceValue::NotReadable => {
                    "Consumes one value. The current value was not readable; the returned value is reported.".into()
                }
            },
        ),
        SequenceIntent::Set { value, is_called } => {
            if !definition.contains(value) {
                return Err(SequenceError::OutOfRange);
            }
            parameters.push(format!("{value} (new last_value)"));
            parameters.push(format!("{is_called} (is_called)"));
            let next = definition.next_after(value, is_called);
            (
                format!("SELECT pg_catalog.setval({regclass}, {value}, {is_called})"),
                format!("{LOCK_TIMEOUT_SQL}\n{SET_SQL}"),
                format!(
                    "Sets last_value = {value} and is_called = {is_called}; {}.{}",
                    definition.describe_next(value, is_called),
                    next.map_or("", |next| rewind_warning(observation, next))
                ),
            )
        }
        SequenceIntent::Restart { with } => {
            if with.is_some_and(|value| !definition.contains(value)) {
                return Err(SequenceError::OutOfRange);
            }
            let next = with.unwrap_or(definition.start);
            let alter = restart_sql(target, with);
            (
                alter.clone(),
                format!("{RESTART_BEGIN_SQL}\n{GUARD_SQL}\n{alter}\n{GUARD_SQL}\nCOMMIT"),
                format!(
                    "Resets the sequence so the next nextval returns {next}{}. Both guard checks must return one row or the transaction rolls back.{}",
                    if with.is_none() {
                        " (the observed START value)"
                    } else {
                        ""
                    },
                    rewind_warning(observation, next)
                ),
            )
        }
    };
    Ok(SequencePreview {
        summary,
        sql,
        parameters,
        effect,
        transactional: intent.transactional(),
        operation_timeout_ms: SEQUENCE_OPERATION_TIMEOUT_MS,
        statement_timeout_ms,
    })
}

impl Backend {
    /// Dedicated, bounded, read-only observation. It never calls nextval. The
    /// returned observation, not a caller-supplied OID, mints reviews.
    pub async fn observe_sequence(
        &self,
        document: &DataDocument,
        reference: PgObjectRef,
    ) -> Result<ObservedSequence, SequenceError> {
        if reference.kind != PgObjectKind::Sequence
            || !reference
                .schema
                .as_ref()
                .is_some_and(|s| types::valid_name(s))
            || !types::valid_name(&reference.name)
            || reference.identity_args.is_some()
        {
            return Err(SequenceError::InvalidTarget);
        }
        let (observation, statement_timeout_ms) = self
            .object_read(document, move |spec, drivers, cancellation| async move {
                let timeout = spec.driver_options.statement_timeout_ms;
                native_sequences::observe(&spec, &drivers, cancellation, &reference)
                    .await
                    .map(|observation| (observation, timeout))
            })
            .await
            .map_err(|error| match error {
                DataError::Catalog(CatalogError::ObjectNotFound) => SequenceError::NotFound,
                _ => SequenceError::Unavailable,
            })?;
        if !observation.valid() {
            return Err(SequenceError::InvalidTarget);
        }
        Ok(ObservedSequence {
            document: document.clone(),
            observation,
            statement_timeout_ms,
        })
    }
    pub async fn apply_sequence(
        &self,
        review: SequenceReview,
    ) -> Result<SequenceSubmission, SequenceError> {
        self.submit_sequence(review, false, native_sequences::execute)
            .await
    }
    pub async fn confirm_sequence(
        &self,
        confirmation: SequenceConfirmation,
    ) -> Result<SequenceSubmission, SequenceError> {
        self.submit_sequence(confirmation.review, true, native_sequences::execute)
            .await
    }
    async fn submit_sequence<F, Fut>(
        &self,
        review: SequenceReview,
        confirmed: bool,
        execute: F,
    ) -> Result<SequenceSubmission, SequenceError>
    where
        F: FnOnce(
                ResolvedPostgresConnectSpec,
                DriverJoins,
                WritePermit,
                watch::Receiver<u64>,
                SequenceIntent,
                SequenceObservation,
                SequencePreview,
            ) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = native_sequences::Execution> + Send + 'static,
    {
        self.submit_sequence_with_loader(review, confirmed, execute, |state, connection| {
            Box::pin(async move {
                crate::app::find_connection(&state, &connection)
                    .await
                    .ok()
                    .and_then(|connection| {
                        ResolvedPostgresConnectSpec::from_connection(&connection).ok()
                    })
            })
        })
        .await
    }

    // Injectable credential boundary so tests prove refused reviews never hydrate.
    async fn submit_sequence_with_loader<F, Fut>(
        &self,
        review: SequenceReview,
        confirmed: bool,
        execute: F,
        load: impl FnOnce(
                Arc<crate::app::AppState>,
                String,
            ) -> BoxFuture<'static, Option<ResolvedPostgresConnectSpec>>
            + Send
            + 'static,
    ) -> Result<SequenceSubmission, SequenceError>
    where
        F: FnOnce(
                ResolvedPostgresConnectSpec,
                DriverJoins,
                WritePermit,
                watch::Receiver<u64>,
                SequenceIntent,
                SequenceObservation,
                SequencePreview,
            ) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = native_sequences::Execution> + Send + 'static,
    {
        let document = review.document.clone();
        if !self.0.documents.owns(&document) {
            return Err(SequenceError::ForeignDocument);
        }
        let inner = self.0.clone();
        self.data_call(&document, move |state, document, admission| async move {
            let result = async {
                let stored =
                    crate::storage::read_connection_by_id(&state.pool, &document.0.connection)
                        .await
                        .map_err(|_| SequenceError::Storage)?
                        .ok_or(SequenceError::Unavailable)?;
                let crate::StoredConnection::PostgreSQL(postgres) = &stored else {
                    return Err(SequenceError::Unavailable);
                };
                // Regenerate from current stored options before policy can mint
                // a confirmation, acquire a write slot or hydrate credentials.
                let generated = preview(
                    &review.observation,
                    review.intent,
                    postgres
                        .driver_options
                        .as_ref()
                        .and_then(|options| options.statement_timeout_ms),
                )?;
                if generated != review.preview {
                    return Err(SequenceError::InvalidTarget);
                }
                let intent = review.intent.policy();
                let authorization =
                    match assert_permitted(&gate::resolved_policy(&stored), &intent, confirmed) {
                        Ok(value) => value,
                        Err(refusal) => {
                            return refusal.fold(
                                |_, _| Err(SequenceError::PolicyBlocked),
                                |_| {
                                    Ok(SequenceSubmission::NeedsConfirmation(Box::new(
                                        SequenceConfirmation { review },
                                    )))
                                },
                            )
                        }
                    };
                let permit = inner
                    .documents
                    .begin_write(&document)
                    .map_err(|_| SequenceError::Busy)?;
                let cancellation = document.0.read_cancellation();
                let spec = load(state.clone(), document.0.connection.clone()).await;
                let drivers = inner.tasks.child();
                drop(admission);
                let execution = match spec {
                    Some(spec)
                        if spec.driver_options.statement_timeout_ms
                            == generated.statement_timeout_ms =>
                    {
                        execute(
                            spec,
                            drivers,
                            permit,
                            cancellation,
                            review.intent,
                            review.observation.clone(),
                            generated,
                        )
                        .await
                    }
                    _ => native_sequences::Execution::not_dispatched(SequenceFailure::Connection),
                };
                if matches!(execution.outcome, SequenceOutcome::Completed { .. }) {
                    if authorization.audit_disposition() == AuditDisposition::RequiredAfterSuccess {
                        gate::record_override(
                            &state.pool,
                            &document.0.connection,
                            review.intent.command(),
                            &intent,
                        )
                        .await;
                    }
                    crate::app::touch_connection_activity(&state, &document.0.connection).await;
                }
                Ok(SequenceSubmission::Finished(Box::new(SequenceReceipt {
                    attempt_id: review.attempt_id,
                    intent: review.intent,
                    target: review.observation.target,
                    preview: review.preview,
                    outcome: execution.outcome,
                    runtime_ms: execution.runtime_ms,
                })))
            }
            .await;
            Ok(result)
        })
        .await
        .map_err(|error| match error {
            DataError::Document(_) => SequenceError::Unavailable,
            _ => SequenceError::OutcomeUnavailable,
        })?
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;
