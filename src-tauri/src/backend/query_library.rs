//! Profile-local saved work. These calls never hydrate a connection or execute
//! SQL against PostgreSQL, and remain available for deleted connections.
use super::Backend;
use crate::query_library::native;

pub use native::{
    HistoryRecord, LibraryCursor, LibraryError, LibraryPage, LibraryRequest, SavedQueryRecord,
};

impl Backend {
    pub async fn load_query_history(
        &self,
        request: LibraryRequest,
    ) -> Result<LibraryPage<HistoryRecord>, LibraryError> {
        self.call(move |state| async move { Ok(native::history(&state, request).await) })
            .await
            .map_err(LibraryError::Unavailable)?
    }

    /// The host supplies a completed execution record. Cancelled executions
    /// remain excluded by the execution caller, as in the Tauri host.
    pub async fn append_query_history(&self, entry: HistoryRecord) -> Result<(), LibraryError> {
        self.call(move |state| async move { Ok(native::append(&state, entry).await) })
            .await
            .map_err(LibraryError::Unavailable)?
    }

    pub async fn delete_query_history(&self, id: String) -> Result<(), LibraryError> {
        self.call(move |state| async move { Ok(native::delete_history(&state, &id).await) })
            .await
            .map_err(LibraryError::Unavailable)?
    }

    /// Explicit profile-wide clearing, matching the existing history action.
    pub async fn clear_query_history(&self) -> Result<(), LibraryError> {
        self.call(move |state| async move {
            Ok(crate::query_library::clear_query_history(&state)
                .await
                .map_err(|_| LibraryError::Storage))
        })
        .await
        .map_err(LibraryError::Unavailable)?
    }

    pub async fn load_saved_queries(
        &self,
        request: LibraryRequest,
    ) -> Result<LibraryPage<SavedQueryRecord>, LibraryError> {
        self.call(move |state| async move { Ok(native::saved(&state, request).await) })
            .await
            .map_err(LibraryError::Unavailable)?
    }

    /// Returns only the acknowledged record, never an unbounded whole library.
    /// Existing creation time is preserved; updated_at is service generated.
    pub async fn save_saved_query(
        &self,
        query: SavedQueryRecord,
    ) -> Result<SavedQueryRecord, LibraryError> {
        self.call(move |state| async move { Ok(native::save(&state, query).await) })
            .await
            .map_err(LibraryError::Unavailable)?
    }

    /// Update editor SQL/title/binding while preserving the saved record's
    /// current favorite and owner metadata in the same SQLite transaction.
    pub async fn save_query_draft(
        &self,
        query: SavedQueryRecord,
    ) -> Result<SavedQueryRecord, LibraryError> {
        self.call(move |state| async move { Ok(native::save_draft(&state, query).await) })
            .await
            .map_err(LibraryError::Unavailable)?
    }

    pub async fn delete_saved_query(&self, id: String) -> Result<(), LibraryError> {
        self.call(move |state| async move { Ok(native::delete_saved(&state, &id).await) })
            .await
            .map_err(LibraryError::Unavailable)?
    }
}

#[cfg(test)]
#[path = "query_library_tests.rs"]
mod tests;

impl HistoryRecord {
    pub fn mark_started(&mut self) {
        self.started_at = chrono::Utc::now().to_rfc3339();
    }
    /// Capture the host's execution identity, exact submitted SQL and wall-clock
    /// start before dispatch. Runtime duration is supplied from a monotonic clock.
    pub fn started(
        id: String,
        sql: String,
        connection_id: String,
        connection_name: String,
        database: String,
    ) -> Self {
        Self {
            id,
            sql,
            connection_id,
            connection_name,
            database,
            engine: "PostgreSQL".into(),
            status: "success".into(),
            error_message: None,
            runtime_ms: 0,
            row_count: None,
            started_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// RFC3339 UTC timestamp for profile-local native activity records.
pub fn utc_timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}
