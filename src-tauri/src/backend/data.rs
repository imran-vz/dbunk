//! Native table and change-review services. Opaque documents bind operations to
//! one connection and private manager tab; caller-supplied routing IDs are ignored.
pub use super::data_documents::DataDocument;
use super::*;
pub use crate::result_mutation::protocol::{
    AnalysisStatement, AnalyzeResultSetPayload, AnalyzeResultSetResult, AnalyzeSource,
    AnalyzedColumn, AnalyzedTable, AppliedOperation, ApplyResult, ApplyResultMutationsPayload,
    CancelResultMutationPayload, CancelResultMutationResult, CapabilityReason, CapabilityVerdict,
    ClearVirtualKeyPayload, CloseResultMutationPayload, ColumnOrigin, ColumnWritability, DmlParam,
    InvalidPlanReason, LoadVirtualKeyPayload, MutationIdentity, MutationIdentityKind, MutationOp,
    MutationPlan, MutationTable, MutationValue, NativeAnalysisContext, NotAnalyzableReason,
    PreviewResult, PreviewResultMutationsPayload, PreviewStatement, ResultMutationError,
    SaveVirtualKeyPayload, VirtualKey,
};
use crate::result_mutation::service as mutation_service;
pub use crate::table_browse::protocol::{
    BrowseColumn, BrowseCount, BrowseCountKind, BrowseCountPolicy, BrowseCursor,
    BrowseExactCountResult, BrowseFilter, BrowseIdentity, BrowseIdentityKind, BrowseInspection,
    BrowseNulls, BrowsePageInfo, BrowsePageMode, BrowsePageRequest, BrowseSortDirection,
    BrowseSortKey, BrowseTableDataPayload, BrowseTableResult, CancelTableBrowseResult,
    ComparisonOperator, CountTableBrowseRowsPayload, InspectionParam, LoadTableGridPrefsPayload,
    SaveTableGridPrefsPayload, TableBrowseError, TableBrowseTabPayload, TableGridPrefs,
    TextMatchOperator,
};
use crate::table_browse::service as browse_service;
use tokio::sync::OwnedMutexGuard;

#[derive(Debug)]
pub enum DataError {
    Unavailable(QuerySessionError),
    Document(&'static str),
    Browse(TableBrowseError),
    Mutation(ResultMutationError),
    Storage(String),
    Catalog(super::objects::CatalogError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataCloseOutcome {
    Closed,
    /// A shared actor failed to settle; all data documents on this connection
    /// were retired and its browse/mutation sockets were aborted and joined.
    ConnectionDataClosed,
}

impl DataDocument {
    pub fn connection_id(&self) -> &str {
        &self.0.connection
    }
}

impl Backend {
    pub async fn open_data_document(
        &self,
        window: &str,
        tab: &str,
        connection_id: &str,
    ) -> Result<DataDocument, DataError> {
        let inner = self.0.clone();
        let window = window.to_owned();
        let tab = tab.to_owned();
        let connection = connection_id.to_owned();
        self.development_call(move |state| async move {
            if inner.tool_jobs.restore_in_progress(&connection)
                || inner.csv_transfers.import_in_progress(&connection)
                || super::table_copy::destination_write_in_progress(&inner, &connection)
                || super::table_seed::destination_write_in_progress(&inner, &connection)
            {
                return Ok(Err(DataError::Document(
                    "An owned transfer reserves this connection; retry after cleanup",
                )));
            }
            admit_connection(&state, inner.development.as_deref(), &connection).await?;
            Ok(inner
                .documents
                .register(window, tab, connection)
                .map_err(DataError::Document))
        })
        .await
        .map_err(DataError::Unavailable)?
    }

    pub(super) async fn data_call<T, F, Fut>(
        &self,
        document: &DataDocument,
        operation: F,
    ) -> Result<T, DataError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<AppState>, DataDocument, OwnedMutexGuard<()>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, DataError>> + Send + 'static,
    {
        let inner = self.0.clone();
        let document = document.clone();
        self.call_with_admission(self.0.data_admission.clone(), move |state| async move {
            let admission = inner.development_gate.clone().lock_owned().await;
            if inner.closing.load(Ordering::SeqCst) {
                return Err(QuerySessionError::ConnectionClosing);
            }
            // Acquire the document permit after startup admission. Lifecycle
            // operations can then join admitted work without waiting on a task
            // that itself needs the lifecycle gate to discover it was retired.
            let permit = match inner.documents.enter(&document).await {
                Ok(permit) => permit,
                Err(error) => return Ok(Err(DataError::Document(error))),
            };
            admit_connection(&state, inner.development.as_deref(), &document.0.connection).await?;
            let result = operation(state, document, admission).await;
            drop(permit);
            Ok(result)
        })
        .await
        .map_err(DataError::Unavailable)?
    }

    pub async fn browse_table(
        &self,
        document: &DataDocument,
        mut payload: BrowseTableDataPayload,
    ) -> Result<BrowseTableResult, DataError> {
        self.data_call(document, move |state, document, admission| async move {
            payload.connection_id = document.0.connection.clone();
            payload.tab_id = document.0.manager_tab.clone();
            let pending = browse_service::start_browse(&state, payload)
                .await
                .map_err(DataError::Browse)?;
            drop(admission);
            pending.await.map_err(DataError::Browse)
        })
        .await
    }

    pub async fn count_table(
        &self,
        document: &DataDocument,
        mut payload: CountTableBrowseRowsPayload,
    ) -> Result<BrowseExactCountResult, DataError> {
        self.data_call(document, move |state, document, admission| async move {
            payload.connection_id = document.0.connection.clone();
            payload.tab_id = document.0.manager_tab.clone();
            let pending = browse_service::start_count(&state, payload)
                .await
                .map_err(DataError::Browse)?;
            drop(admission);
            pending.await.map_err(DataError::Browse)
        })
        .await
    }

    pub async fn analyze_result(
        &self,
        document: &DataDocument,
        mut payload: AnalyzeResultSetPayload,
    ) -> Result<AnalyzeResultSetResult, DataError> {
        self.data_call(document, move |state, document, admission| async move {
            payload.connection_id = document.0.connection.clone();
            payload.tab_id = document.0.manager_tab.clone();
            let pending = mutation_service::start_analyze(&state, payload)
                .await
                .map_err(DataError::Mutation)?;
            drop(admission);
            pending.await.map_err(DataError::Mutation)
        })
        .await
    }

    /// Uses the control budget, even when every data request slot is occupied.
    pub async fn cancel_data(&self, document: &DataDocument) -> Result<(), DataError> {
        let inner = self.0.clone();
        let document = document.clone();
        self.development_call(move |state| async move {
            let _permit = match inner.documents.enter(&document).await {
                Ok(permit) => permit,
                Err(error) => return Ok(Err(DataError::Document(error))),
            };
            document.0.cancel_reads();
            tokio::join!(
                state
                    .table_browse
                    .cancel_tab(&document.0.connection, &document.0.manager_tab),
                state
                    .result_mutations
                    .cancel_tab(&document.0.connection, &document.0.manager_tab),
            );
            Ok(Ok(()))
        })
        .await
        .map_err(DataError::Unavailable)?
    }

    /// Retires admission before joining work. Dropping the caller's future
    /// cannot abandon cleanup. Forced close is explicitly visible to the host.
    pub async fn close_data_document(
        &self,
        document: &DataDocument,
    ) -> Result<DataCloseOutcome, DataError> {
        let inner = self.0.clone();
        let document = document.clone();
        self.development_call(move |state| async move {
            // Retire only after an owned cleanup task has been admitted. A
            // saturated control budget must leave the document usable/retryable.
            if let Err(error) = inner.documents.retire(&document) {
                return Ok(Err(DataError::Document(error)));
            }
            let started = tokio::time::Instant::now();
            let graceful = async {
                tokio::join!(
                    state
                        .table_browse
                        .close_tab(&document.0.connection, &document.0.manager_tab),
                    state
                        .result_mutations
                        .close_native_tab(&document.0.connection, &document.0.manager_tab),
                );
                inner.documents.finish(&document).await
            };
            if let Ok(result) =
                tokio::time::timeout_at(started + Duration::from_secs(3), graceful).await
            {
                return Ok(result
                    .map(|()| DataCloseOutcome::Closed)
                    .map_err(DataError::Document));
            }
            inner
                .documents
                .retire_matching(Some(&document.0.connection));
            let forced = async {
                tokio::join!(
                    state
                        .table_browse
                        .force_native_teardown(Some(&document.0.connection)),
                    state
                        .result_mutations
                        .force_native_teardown(Some(&document.0.connection)),
                );
                inner
                    .documents
                    .finish_retired(Some(&document.0.connection))
                    .await;
                state
                    .table_browse
                    .end_connection_teardown(&document.0.connection)
                    .await;
                state
                    .result_mutations
                    .end_connection_teardown(&document.0.connection)
                    .await;
            };
            tokio::time::timeout_at(started + Duration::from_secs(5), forced)
                .await
                .map_err(|_| QuerySessionError::Timeout {
                    operation: "closeDataDocument".into(),
                })?;
            Ok(Ok(DataCloseOutcome::ConnectionDataClosed))
        })
        .await
        .map_err(DataError::Unavailable)?
    }
}

mod mutations;
mod preferences;
pub use mutations::{MutationConfirmation, MutationReview, MutationSubmission};
#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;

/// The caller holds startup admission. Retire handles before a connection or
/// credential mutation so an old request/review can never reopen the new state.
pub(super) async fn retire_data(
    inner: &Inner,
    state: &AppState,
    connection: Option<&str>,
) -> Result<(), String> {
    inner.documents.retire_matching(connection);
    let started = tokio::time::Instant::now();
    let graceful = async {
        match connection {
            Some(id) => {
                tokio::join!(
                    state.table_browse.begin_connection_teardown(id),
                    state.result_mutations.begin_connection_teardown(id),
                );
            }
            None => {
                tokio::join!(
                    state.table_browse.begin_global_teardown(),
                    state.result_mutations.begin_global_teardown(),
                );
            }
        }
    };
    if tokio::time::timeout_at(started + Duration::from_secs(3), graceful)
        .await
        .is_err()
    {
        tokio::time::timeout_at(started + Duration::from_secs(5), async {
            tokio::join!(
                state.table_browse.force_native_teardown(connection),
                state.result_mutations.force_native_teardown(connection),
            );
        })
        .await
        .map_err(|_| {
            "Data cleanup could not join all owned tasks within five seconds".to_string()
        })?;
    }
    // Each request task owns its lease until its service result settles.
    tokio::time::timeout_at(
        started + Duration::from_secs(5),
        inner.documents.finish_retired(connection),
    )
    .await
    .map_err(|_| "Data requests did not settle after socket cleanup".to_string())?;
    match connection {
        Some(id) => {
            state.table_browse.end_connection_teardown(id).await;
            state.result_mutations.end_connection_teardown(id).await;
        }
        None => {
            state.table_browse.end_global_teardown().await;
            state.result_mutations.end_global_teardown().await;
        }
    }
    Ok(())
}
