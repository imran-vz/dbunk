//! Native read-only PG16 comparison jobs and document-owned response leases.
mod bounds;
mod readers;
mod types;
use super::{Backend, Inner};
use crate::postgres::schema_compare::protocol::StartRequest;
pub use crate::postgres::schema_compare::{
    capture::ExcludedCount,
    normalize::DifferenceKind,
    pages::{ObservedSides, SummaryDifference},
    protocol::{
        CaptureMetadata, ColumnField, CompareError, ComparisonMetadata, ConstraintField, Coverage,
        Eligibility, Endpoint, ExcludedCategory, Exclusion, FieldPath, IncomparableReason,
        IndexField, IndexKeyField, Limit, ReadRequest, RelationIdentity, RelationKind,
        ResultIdentity, ResultRequest, Side, SnapshotConsistency, Status, StatusState, TableField,
    },
    values::{ValueKind, ValueRef},
};
pub use readers::{SchemaComparisonReader, SchemaComparisonResponse};
use std::sync::{Arc, Weak};
pub use types::*;

#[derive(Default)]
pub(super) struct Registry {
    pub owner: crate::postgres::backup::native::Ownership,
    readers: readers::Readers,
    #[cfg(test)]
    test_capture: std::sync::Mutex<Option<TestCapture>>,
}
#[cfg(test)]
type TestCapture = Arc<
    dyn Fn(
            crate::postgres::schema_compare::manager::JobContext,
        ) -> futures_util::future::BoxFuture<
            'static,
            Result<crate::postgres::schema_compare::diff::Comparison, CompareError>,
        > + Send
        + Sync,
>;
impl Registry {
    pub fn close(&self) {
        self.readers.close_all();
    }
    pub async fn drain_until(&self, deadline: tokio::time::Instant) -> Result<(), String> {
        self.owner.drain_until(deadline).await.map_err(|_| {
            "Comparison workers did not join before the shutdown deadline".to_string()
        })?;
        self.readers.drain_until(deadline).await.map_err(|_| {
            "Comparison response leases remain owned after the shutdown deadline".to_string()
        })
    }
}
impl Backend {
    /// Admission is registered synchronously. Reusing the exact request reconciles
    /// its existing record; an expired request can never silently launch again.
    pub fn begin_schema_comparison(
        &self,
        start: SchemaComparisonStart,
    ) -> Result<Status, CompareError> {
        start
            .checked_heap_bytes()
            .ok_or(CompareError::InvalidRequest)?;
        let _submission = self.0.submission.lock().unwrap();
        if self.0.closing.load(std::sync::atomic::Ordering::Acquire) {
            return Err(CompareError::Unavailable);
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(CompareError::Unavailable);
        }
        let inner = self.0.clone();
        self.0
            .state
            .pg_schema_compare
            .start(start.0, move |context| async move {
                // Validate both authorities before hydrating either endpoint.
                let (source, target) = {
                    let _gate = context.control.wait(inner.development_gate.lock()).await?;
                    context.check_current()?;
                    for endpoint in [&context.request.source, &context.request.target] {
                        context
                            .control
                            .wait(super::admit_connection(
                                &inner.state,
                                inner.development.as_deref(),
                                &endpoint.connection_id,
                            ))
                            .await?
                            .map_err(|_| CompareError::Unavailable)?;
                    }
                    context.check_current()?;
                    let source = context
                        .control
                        .wait(crate::app::find_connection(
                            &inner.state,
                            &context.request.source.connection_id,
                        ))
                        .await?
                        .map_err(|_| CompareError::Unavailable)?;
                    let target = if context.request.source.connection_id
                        == context.request.target.connection_id
                    {
                        None
                    } else {
                        Some(
                            context
                                .control
                                .wait(crate::app::find_connection(
                                    &inner.state,
                                    &context.request.target.connection_id,
                                ))
                                .await?
                                .map_err(|_| CompareError::Unavailable)?,
                        )
                    };
                    context.check_current()?;
                    (source, target)
                };
                #[cfg(test)]
                {
                    let capture = inner
                        .schema_comparisons
                        .test_capture
                        .lock()
                        .unwrap()
                        .clone();
                    if let Some(capture) = capture {
                        return capture(context).await;
                    }
                }
                {
                    use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
                    let source = ResolvedPostgresConnectSpec::from_connection(&source)
                        .map_err(|_| CompareError::UnsupportedEngine { side: Side::Source })?;
                    let target = target
                        .as_ref()
                        .map(ResolvedPostgresConnectSpec::from_connection)
                        .transpose()
                        .map_err(|_| CompareError::UnsupportedEngine { side: Side::Target })?;
                    crate::postgres::schema_compare::manager::runner::run_resolved(
                        context,
                        &source,
                        target.as_ref(),
                        inner.state.pool.clone(),
                    )
                    .await
                }
            })
    }
    pub fn get_schema_comparison(&self, job_id: &str) -> Result<Status, CompareError> {
        self.0.state.pg_schema_compare.get(job_id)
    }
    pub fn list_schema_comparisons(&self) -> Result<SchemaComparisonList, CompareError> {
        let list = SchemaComparisonList {
            jobs: self.0.state.pg_schema_compare.list(),
        };
        list.checked_heap_bytes()
            .ok_or(CompareError::LimitExceeded {
                limit: Limit::Allocation,
            })?;
        Ok(list)
    }
    pub fn cancel_schema_comparison(&self, job_id: &str) -> Result<Status, CompareError> {
        self.0.state.pg_schema_compare.cancel(job_id)
    }
    pub fn release_schema_comparison(&self, job_id: &str) -> Result<(), CompareError> {
        self.0.state.pg_schema_compare.release(job_id)
    }
}

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;

/// Allocation-free admission for a queued exact result/read descriptor.
pub fn checked_schema_comparison_read_bytes(
    request: &ResultRequest,
    read: &ReadRequest,
) -> Option<usize> {
    bounds::request_bytes(request)?
        .checked_add(bounds::read_bytes(read)?)
        .filter(|bytes| *bytes <= 4096)
}
