//! Profile-local history and saved-query services, shared by desktop hosts.
//!
//! These functions persist supplied records; they never execute SQL. Execution
//! callers decide when to append history. In particular, the existing Query
//! Session caller skips cancelled executions and computes row counts itself.
use crate::{
    storage, AppState, DeleteSavedQueryPayload, QueryHistoryEntry, SavedQuery, MAX_QUERY_HISTORY,
};

pub(crate) async fn load_query_history(
    state: &AppState,
    limit: Option<u32>,
) -> Result<Vec<QueryHistoryEntry>, String> {
    storage::read_query_history(&state.pool, limit).await
}

pub(crate) async fn append_query_history(
    state: &AppState,
    entry: QueryHistoryEntry,
) -> Result<Vec<QueryHistoryEntry>, String> {
    storage::insert_query_history(&state.pool, &entry).await?;
    storage::read_query_history(&state.pool, Some(MAX_QUERY_HISTORY as u32)).await
}

pub(crate) async fn clear_query_history(state: &AppState) -> Result<(), String> {
    storage::clear_query_history(&state.pool).await
}

pub(crate) async fn load_saved_queries(state: &AppState) -> Result<Vec<SavedQuery>, String> {
    storage::read_saved_queries(&state.pool).await
}

/// Upserts by ID and refreshes updated_at. Storage preserves the original
/// created_at on updates, even if the caller supplies a replacement timestamp.
pub(crate) async fn save_saved_query(
    state: &AppState,
    query: SavedQuery,
) -> Result<Vec<SavedQuery>, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut next = query;
    next.updated_at = now.clone();
    if next.created_at.is_empty() {
        next.created_at = now;
    }
    storage::upsert_saved_query(&state.pool, &next).await?;
    storage::read_saved_queries(&state.pool).await
}

pub(crate) async fn delete_saved_query(
    state: &AppState,
    payload: DeleteSavedQueryPayload,
) -> Result<Vec<SavedQuery>, String> {
    storage::delete_saved_query(&state.pool, &payload.id).await?;
    storage::read_saved_queries(&state.pool).await
}

/// The native host pages through legacy records without changing the Tauri
/// commands' whole-list contract. Limits are checked before retaining text.
#[cfg(feature = "isolated-profile")]
pub(crate) mod native {
    use super::*;
    use crate::{query_session::protocol::QuerySessionError, DatabaseEngine};
    use futures_util::TryStreamExt;
    use serde::{Deserialize, Serialize};
    use sqlx::{sqlite::SqliteRow, Row};
    use std::str::FromStr;

    const PAGE_ROWS: u32 = 200;
    const SCAN_ROWS: usize = 2000;
    const ROW_BYTES: usize = 2 * 1024 * 1024;
    const PAGE_BYTES: usize = 8 * 1024 * 1024;
    const SQL_BYTES: usize = 1024 * 1024;
    const TEXT_BYTES: usize = 8192;

    #[derive(Debug)]
    pub enum LibraryError {
        InvalidInput(&'static str),
        TooLarge,
        Storage,
        Unavailable(QuerySessionError),
    }
    impl std::fmt::Display for LibraryError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::InvalidInput(message) => f.write_str(message),
                Self::TooLarge => {
                    f.write_str("A library record exceeds the native read or write budget")
                }
                Self::Storage => f.write_str("Query library storage operation failed"),
                Self::Unavailable(_) => f.write_str("Query library is closing or busy"),
            }
        }
    }
    impl std::error::Error for LibraryError {}

    /// Ordering tuple, not authorization. Reuse only with the same filters.
    /// New edits can move rows before a cursor; restarting refreshes the list.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct LibraryCursor {
        pub timestamp: String,
        pub id: String,
        pub favorite: bool,
    }

    #[derive(Clone)]
    pub struct LibraryRequest {
        pub limit: u32,
        pub cursor: Option<LibraryCursor>,
        pub connection_id: Option<String>,
        pub search: String,
        /// History accepts success/error; saved queries require None.
        pub status: Option<String>,
    }
    impl Default for LibraryRequest {
        fn default() -> Self {
            Self {
                limit: 50,
                cursor: None,
                connection_id: None,
                search: String::new(),
                status: None,
            }
        }
    }
    #[derive(Serialize)]
    pub struct LibraryPage<T> {
        pub entries: Vec<T>,
        /// Some also on an empty page when the finite scan budget was used.
        pub next: Option<LibraryCursor>,
    }

    // No derived Debug: SQL can contain literals and error messages can repeat
    // them. Explicit display/export is the only consumer of these values.
    #[derive(Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct HistoryRecord {
        pub id: String,
        pub sql: String,
        pub connection_id: String,
        pub connection_name: String,
        pub database: String,
        pub engine: String,
        pub status: String,
        pub error_message: Option<String>,
        pub runtime_ms: u64,
        pub row_count: Option<u64>,
        pub started_at: String,
    }
    #[derive(Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SavedQueryRecord {
        pub id: String,
        pub name: String,
        pub body: String,
        pub connection_id: Option<String>,
        pub is_favorite: bool,
        pub owner_id: Option<String>,
        pub created_at: String,
        pub updated_at: String,
    }

    fn identity(value: &str) -> Result<(), LibraryError> {
        if value.is_empty() || value.len() > 256 {
            Err(LibraryError::InvalidInput(
                "Identity must contain 1 to 256 bytes",
            ))
        } else {
            Ok(())
        }
    }
    fn request_valid(request: &LibraryRequest, saved: bool) -> Result<(), LibraryError> {
        if request.limit == 0 || request.limit > PAGE_ROWS {
            return Err(LibraryError::InvalidInput("Page size must be 1 to 200"));
        }
        if request.search.len() > TEXT_BYTES {
            return Err(LibraryError::TooLarge);
        }
        if let Some(id) = &request.connection_id {
            identity(id)?;
        }
        if let Some(cursor) = &request.cursor {
            identity(&cursor.id)?;
            if cursor.timestamp.len() > TEXT_BYTES {
                return Err(LibraryError::TooLarge);
            }
        }
        if let Some(status) = &request.status {
            if saved || !matches!(status.as_str(), "success" | "error") {
                return Err(LibraryError::InvalidInput(
                    "Status filter must be success or error, and only applies to history",
                ));
            }
        }
        Ok(())
    }
    fn text_valid<'a>(texts: impl IntoIterator<Item = &'a str>) -> Result<(), LibraryError> {
        if texts.into_iter().any(|text| text.len() > TEXT_BYTES) {
            Err(LibraryError::TooLarge)
        } else {
            Ok(())
        }
    }

    trait LibraryRow: Sized + Serialize {
        const TABLE: &'static str;
        const COLUMNS: &'static [&'static str];
        const TIME: &'static str;
        const FAVORITE: &'static str;
        fn decode(row: SqliteRow) -> Result<Self, sqlx::Error>;
        fn cursor(&self) -> LibraryCursor;
        fn matches(&self, needle: &str) -> bool;
    }
    impl LibraryRow for HistoryRecord {
        const TABLE: &'static str = "query_history";
        const COLUMNS: &'static [&'static str] = &[
            "id",
            "sql",
            "connection_id",
            "connection_name",
            "database_name",
            "engine",
            "status",
            "error_message",
            "runtime_ms",
            "row_count",
            "started_at",
        ];
        const TIME: &'static str = "started_at";
        const FAVORITE: &'static str = "0";
        fn decode(row: SqliteRow) -> Result<Self, sqlx::Error> {
            Ok(Self {
                id: row.try_get("id")?,
                sql: row.try_get("sql")?,
                connection_id: row.try_get("connection_id")?,
                connection_name: row.try_get("connection_name")?,
                database: row.try_get("database_name")?,
                engine: row.try_get("engine")?,
                status: row.try_get("status")?,
                error_message: row.try_get("error_message")?,
                runtime_ms: row.try_get::<i64, _>("runtime_ms")?.max(0) as u64,
                row_count: row
                    .try_get::<Option<i64>, _>("row_count")?
                    .map(|v| v.max(0) as u64),
                started_at: row.try_get("started_at")?,
            })
        }
        fn cursor(&self) -> LibraryCursor {
            LibraryCursor {
                timestamp: self.started_at.clone(),
                id: self.id.clone(),
                favorite: false,
            }
        }
        fn matches(&self, needle: &str) -> bool {
            self.sql.to_lowercase().contains(needle)
        }
    }
    impl LibraryRow for SavedQueryRecord {
        const TABLE: &'static str = "saved_queries";
        const COLUMNS: &'static [&'static str] = &[
            "id",
            "name",
            "body",
            "connection_id",
            "is_favorite",
            "owner_id",
            "created_at",
            "updated_at",
        ];
        const TIME: &'static str = "updated_at";
        const FAVORITE: &'static str = "(is_favorite != 0)";
        fn decode(row: SqliteRow) -> Result<Self, sqlx::Error> {
            Ok(Self {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
                body: row.try_get("body")?,
                connection_id: row.try_get("connection_id")?,
                is_favorite: row.try_get::<i64, _>("is_favorite")? != 0,
                owner_id: row.try_get("owner_id")?,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
            })
        }
        fn cursor(&self) -> LibraryCursor {
            LibraryCursor {
                timestamp: self.updated_at.clone(),
                id: self.id.clone(),
                favorite: self.is_favorite,
            }
        }
        fn matches(&self, needle: &str) -> bool {
            self.name.to_lowercase().contains(needle) || self.body.to_lowercase().contains(needle)
        }
    }

    /// CASE suppresses oversized text before SQLx allocates a row. Streaming
    /// bounds both retained results and scan work, including Unicode filtering.
    async fn list<T: LibraryRow>(
        state: &AppState,
        request: LibraryRequest,
        saved: bool,
    ) -> Result<LibraryPage<T>, LibraryError> {
        request_valid(&request, saved)?;
        let lengths = T::COLUMNS
            .iter()
            .map(|column| format!("coalesce(length(CAST({column} AS BLOB)),0)"))
            .collect::<Vec<_>>()
            .join(" + ");
        let columns = T::COLUMNS
            .iter()
            .map(|column| {
                format!("CASE WHEN row_bytes <= {ROW_BYTES} THEN {column} END AS {column}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT row_bytes, {columns} FROM (SELECT *, {lengths} AS row_bytes, {} AS favorite_sort FROM {}) WHERE (? IS NULL OR connection_id = ?) {} AND (? = 0 OR favorite_sort < ? OR (favorite_sort = ? AND ({} < ? OR ({} = ? AND id < ?)))) ORDER BY favorite_sort DESC, {} DESC, id DESC LIMIT {}", T::FAVORITE, T::TABLE, if saved { "AND (? IS NULL OR ? IS NULL)" } else { "AND (? IS NULL OR status = ?)" }, T::TIME, T::TIME, T::TIME, SCAN_ROWS + 1);
        let cursor = request.cursor.as_ref();
        let needle = request.search.trim().to_lowercase();
        let mut rows = sqlx::query(&sql)
            .bind(&request.connection_id)
            .bind(&request.connection_id)
            .bind(&request.status)
            .bind(&request.status)
            .bind(cursor.is_some())
            .bind(cursor.is_some_and(|v| v.favorite))
            .bind(cursor.is_some_and(|v| v.favorite))
            .bind(cursor.map(|v| v.timestamp.as_str()).unwrap_or_default())
            .bind(cursor.map(|v| v.timestamp.as_str()).unwrap_or_default())
            .bind(cursor.map(|v| v.id.as_str()).unwrap_or_default())
            .fetch(&state.pool);
        let mut entries = Vec::new();
        let mut previous = request.cursor.clone();
        // The continuation duplicates the raw timestamp and ID. Every byte
        // may become a six-byte JSON control escape; also reserve punctuation
        // for the page/cursor objects and a comma for each retained entry.
        let mut bytes = 6 * (TEXT_BYTES + 256) + 128 + PAGE_ROWS as usize;
        let mut scanned = 0;
        while let Some(row) = rows.try_next().await.map_err(|_| LibraryError::Storage)? {
            if entries.len() == request.limit as usize || scanned == SCAN_ROWS {
                return Ok(LibraryPage {
                    entries,
                    next: previous,
                });
            }
            let size: i64 = row
                .try_get("row_bytes")
                .map_err(|_| LibraryError::Storage)?;
            if size < 0 || size as usize > ROW_BYTES {
                return Err(LibraryError::TooLarge);
            }
            let entry = T::decode(row).map_err(|_| LibraryError::Storage)?;
            let next = entry.cursor();
            if identity(&next.id).is_err() || next.timestamp.len() > TEXT_BYTES {
                return Err(LibraryError::TooLarge);
            }
            if needle.is_empty() || entry.matches(&needle) {
                let size = encoded_bytes(&entry)?;
                if bytes + size > PAGE_BYTES {
                    if entries.is_empty() {
                        return Err(LibraryError::TooLarge);
                    }
                    return Ok(LibraryPage {
                        entries,
                        next: previous,
                    });
                }
                bytes += size;
                previous = Some(next);
                entries.push(entry);
            } else {
                previous = Some(next);
            }
            scanned += 1;
        }
        Ok(LibraryPage {
            entries,
            next: None,
        })
    }
    fn encoded_bytes(value: &impl Serialize) -> Result<usize, LibraryError> {
        struct Counter(usize);
        impl std::io::Write for Counter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut counter = Counter(0);
        serde_json::to_writer(&mut counter, value).map_err(|_| LibraryError::Storage)?;
        Ok(counter.0)
    }
    pub(crate) async fn history(
        state: &AppState,
        request: LibraryRequest,
    ) -> Result<LibraryPage<HistoryRecord>, LibraryError> {
        list(state, request, false).await
    }
    pub(crate) async fn saved(
        state: &AppState,
        request: LibraryRequest,
    ) -> Result<LibraryPage<SavedQueryRecord>, LibraryError> {
        list(state, request, true).await
    }

    pub(crate) async fn append(state: &AppState, entry: HistoryRecord) -> Result<(), LibraryError> {
        identity(&entry.id)?;
        identity(&entry.connection_id)?;
        text_valid([
            entry.connection_name.as_str(),
            &entry.database,
            &entry.started_at,
        ])?;
        if entry.sql.len() > SQL_BYTES
            || entry
                .error_message
                .as_ref()
                .is_some_and(|v| v.len() > SQL_BYTES)
        {
            return Err(LibraryError::TooLarge);
        }
        if !matches!(entry.status.as_str(), "success" | "error") {
            return Err(LibraryError::InvalidInput(
                "History status must be success or error",
            ));
        }
        let engine = DatabaseEngine::from_str(&entry.engine)
            .map_err(|_| LibraryError::InvalidInput("Unknown database engine"))?;
        if encoded_bytes(&entry)? > ROW_BYTES {
            return Err(LibraryError::TooLarge);
        }
        let entry = QueryHistoryEntry {
            id: entry.id,
            sql: entry.sql,
            connection_id: entry.connection_id,
            connection_name: entry.connection_name,
            database: entry.database,
            engine,
            status: entry.status,
            error_message: entry.error_message,
            runtime_ms: entry.runtime_ms,
            row_count: entry.row_count,
            started_at: entry.started_at,
        };
        // Native acknowledgements include retention. A failed trim must not
        // leave an appended row behind while reporting an unsuccessful write.
        let mut transaction = state
            .pool
            .begin()
            .await
            .map_err(|_| LibraryError::Storage)?;
        sqlx::query("INSERT OR REPLACE INTO query_history (id,sql,connection_id,connection_name,database_name,engine,status,error_message,runtime_ms,row_count,started_at) VALUES (?,?,?,?,?,?,?,?,?,?,?)")
            .bind(&entry.id).bind(&entry.sql).bind(&entry.connection_id).bind(&entry.connection_name).bind(&entry.database).bind(entry.engine.as_str()).bind(&entry.status).bind(&entry.error_message)
            .bind(i64::try_from(entry.runtime_ms).unwrap_or(i64::MAX)).bind(entry.row_count.map(|value| i64::try_from(value).unwrap_or(i64::MAX))).bind(&entry.started_at)
            .execute(&mut *transaction).await.map_err(|_| LibraryError::Storage)?;
        sqlx::query("DELETE FROM query_history WHERE id NOT IN (SELECT id FROM query_history ORDER BY started_at DESC, id DESC LIMIT ?)")
            .bind(MAX_QUERY_HISTORY as i64).execute(&mut *transaction).await.map_err(|_| LibraryError::Storage)?;
        transaction
            .commit()
            .await
            .map_err(|_| LibraryError::Storage)
    }
    pub(crate) async fn delete_history(state: &AppState, id: &str) -> Result<(), LibraryError> {
        identity(id)?;
        sqlx::query("DELETE FROM query_history WHERE id = ?")
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(|_| LibraryError::Storage)?;
        Ok(())
    }
    pub(crate) async fn delete_saved(state: &AppState, id: &str) -> Result<(), LibraryError> {
        identity(id)?;
        storage::delete_saved_query(&state.pool, id)
            .await
            .map_err(|_| LibraryError::Storage)
    }
    pub(crate) async fn save(
        state: &AppState,
        record: SavedQueryRecord,
    ) -> Result<SavedQueryRecord, LibraryError> {
        save_inner(state, record, false).await
    }
    pub(crate) async fn save_draft(
        state: &AppState,
        record: SavedQueryRecord,
    ) -> Result<SavedQueryRecord, LibraryError> {
        save_inner(state, record, true).await
    }
    async fn save_inner(
        state: &AppState,
        mut record: SavedQueryRecord,
        preserve_organization: bool,
    ) -> Result<SavedQueryRecord, LibraryError> {
        identity(&record.id)?;
        if let Some(id) = &record.connection_id {
            identity(id)?;
        }
        text_valid([
            record.name.as_str(),
            &record.created_at,
            record.owner_id.as_deref().unwrap_or_default(),
        ])?;
        if record.body.len() > SQL_BYTES {
            return Err(LibraryError::TooLarge);
        }
        record.updated_at = chrono::Utc::now().to_rfc3339();
        if record.created_at.is_empty() {
            record.created_at = record.updated_at.clone();
        }
        let mut transaction = state
            .pool
            .begin()
            .await
            .map_err(|_| LibraryError::Storage)?;
        let created_bytes: Option<i64> = sqlx::query_scalar(
            "SELECT length(CAST(created_at AS BLOB)) FROM saved_queries WHERE id = ?",
        )
        .bind(&record.id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| LibraryError::Storage)?;
        if created_bytes.is_some_and(|bytes| bytes < 0 || bytes as usize > TEXT_BYTES) {
            return Err(LibraryError::TooLarge);
        }
        if preserve_organization {
            let metadata: Option<(bool, Option<String>, i64)> = sqlx::query_as("SELECT is_favorite, CASE WHEN length(CAST(owner_id AS BLOB)) <= 8192 THEN owner_id END, COALESCE(length(CAST(owner_id AS BLOB)), 0) FROM saved_queries WHERE id = ?")
                .bind(&record.id).fetch_optional(&mut *transaction).await.map_err(|_| LibraryError::Storage)?;
            if let Some((favorite, owner, bytes)) = metadata {
                if bytes > TEXT_BYTES as i64 {
                    return Err(LibraryError::TooLarge);
                }
                record.is_favorite = favorite;
                record.owner_id = owner;
            }
        }
        // RETURNING acknowledges the exact committed row (including immutable
        // created_at) without a racy, unbounded follow-up whole-library read.
        let row = sqlx::query("INSERT INTO saved_queries (id,name,body,connection_id,is_favorite,owner_id,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,body=excluded.body,connection_id=excluded.connection_id,is_favorite=excluded.is_favorite,owner_id=excluded.owner_id,updated_at=excluded.updated_at RETURNING id,name,body,connection_id,is_favorite,owner_id,created_at,updated_at")
            .bind(&record.id).bind(&record.name).bind(&record.body).bind(&record.connection_id).bind(record.is_favorite).bind(&record.owner_id).bind(&record.created_at).bind(&record.updated_at)
            .fetch_one(&mut *transaction).await.map_err(|_| LibraryError::Storage)?;
        let saved = SavedQueryRecord::decode(row).map_err(|_| LibraryError::Storage)?;
        transaction
            .commit()
            .await
            .map_err(|_| LibraryError::Storage)?;
        Ok(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postgres::schema_compare::manager::CompareManager;
    use crate::{credentials, DatabaseEngine};

    async fn state() -> (tempfile::TempDir, AppState) {
        let directory = tempfile::tempdir().unwrap();
        let paths = storage::Paths::from_dir(directory.path().to_path_buf());
        let pool = storage::open_pool(&paths).await.unwrap();
        let state = AppState::with_credentials(
            paths,
            CompareManager::new(),
            credentials::Context::fixture(pool),
        );
        (directory, state)
    }

    fn history(id: &str, started_at: &str) -> QueryHistoryEntry {
        QueryHistoryEntry {
            id: id.into(),
            sql: "SELECT '漢字';\n-- exact draft\n".into(),
            connection_id: "removed-connection".into(),
            connection_name: "Archived database".into(),
            database: "example".into(),
            engine: DatabaseEngine::PostgreSQL,
            status: "error".into(),
            error_message: Some("statement timed out".into()),
            runtime_ms: 17,
            row_count: None,
            started_at: started_at.into(),
        }
    }

    #[tokio::test]
    async fn history_replaces_by_id_preserves_outcomes_and_orders_by_start_time() {
        let (_directory, state) = state().await;
        let newer = history("newer", "2026-10-02T00:00:02Z");
        let older = history("older", "2026-10-02T00:00:01Z");
        append_query_history(&state, newer.clone()).await.unwrap();
        let entries = append_query_history(&state, older.clone()).await.unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["newer", "older"]
        );
        assert_eq!(
            serde_json::to_value(&entries[1]).unwrap(),
            serde_json::to_value(&older).unwrap()
        );
        let mut replacement = newer;
        replacement.status = "success".into();
        replacement.error_message = None;
        replacement.runtime_ms = u64::MAX;
        replacement.row_count = Some(u64::MAX);
        let entries = append_query_history(&state, replacement).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].status, "success");
        assert_eq!(entries[0].runtime_ms, i64::MAX as u64);
        assert_eq!(entries[0].row_count, Some(i64::MAX as u64));
        assert_eq!(entries[0].error_message, None);
        assert!(load_query_history(&state, Some(0))
            .await
            .unwrap()
            .is_empty());
        assert_eq!(load_query_history(&state, Some(1)).await.unwrap().len(), 1);
        clear_query_history(&state).await.unwrap();
        assert!(load_query_history(&state, None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn append_trims_history_globally_to_newest_two_thousand() {
        let (_directory, state) = state().await;
        sqlx::query("WITH RECURSIVE rows(n) AS (SELECT 0 UNION ALL SELECT n + 1 FROM rows WHERE n < ?) INSERT INTO query_history (id, sql, connection_id, connection_name, database_name, engine, status, runtime_ms, started_at) SELECT 'old-' || n, 'SELECT 1', CASE WHEN n % 2 = 0 THEN 'first' ELSE 'second' END, 'Fixture', 'fixture', 'PostgreSQL', 'success', 0, datetime('2026-01-01', '+' || n || ' seconds') FROM rows")
            .bind(MAX_QUERY_HISTORY as i64 - 1)
            .execute(&state.pool).await.unwrap();
        let entries = append_query_history(&state, history("latest", "2026-10-02T00:00:00Z"))
            .await
            .unwrap();
        assert_eq!(entries.len(), MAX_QUERY_HISTORY);
        assert_eq!(entries[0].id, "latest");
        assert_eq!(entries.last().unwrap().id, "old-1");
        assert!(entries.iter().any(|entry| entry.connection_id == "first"));
        assert!(entries.iter().any(|entry| entry.connection_id == "second"));
        assert_eq!(
            load_query_history(&state, Some(u32::MAX))
                .await
                .unwrap()
                .len(),
            MAX_QUERY_HISTORY
        );
    }

    #[tokio::test]
    async fn saved_queries_preserve_creation_exact_sql_and_failed_write_state() {
        let (_directory, state) = state().await;
        let initial = SavedQuery {
            id: "saved".into(),
            name: "Unicode draft".into(),
            body: "SELECT 'é漢字';\n\n".into(),
            connection_id: Some("missing-connection".into()),
            is_favorite: true,
            owner_id: None,
            created_at: String::new(),
            updated_at: "ignored".into(),
        };
        let rows = save_saved_query(&state, initial.clone()).await.unwrap();
        let created = rows[0].created_at.clone();
        assert_eq!(created, rows[0].updated_at);
        assert!(chrono::DateTime::parse_from_rfc3339(&created).is_ok());
        assert_eq!(rows[0].body, initial.body);
        assert_eq!(rows[0].connection_id, initial.connection_id);
        // A distinct older record makes recency ordering independent of clock resolution.
        let mut older = initial.clone();
        older.id = "older".into();
        older.created_at = "2000-01-01T00:00:00Z".into();
        older.updated_at = older.created_at.clone();
        storage::upsert_saved_query(&state.pool, &older)
            .await
            .unwrap();
        let mut edit = initial;
        edit.created_at = "2099-01-01T00:00:00Z".into();
        edit.name = "Renamed".into();
        edit.connection_id = None;
        edit.is_favorite = false;
        edit.owner_id = Some("future-owner".into());
        let rows = save_saved_query(&state, edit.clone()).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "saved");
        assert_eq!(rows[0].created_at, created);
        assert_eq!(rows[0].name, "Renamed");
        assert_eq!(rows[0].connection_id, None);
        assert!(!rows[0].is_favorite);
        assert_eq!(rows[0].owner_id.as_deref(), Some("future-owner"));
        assert_ne!(rows[0].updated_at, "ignored");
        let before = serde_json::to_value(&rows).unwrap();
        sqlx::query("CREATE TRIGGER reject_saved_write BEFORE INSERT ON saved_queries BEGIN SELECT RAISE(ABORT, 'injected write failure'); END").execute(&state.pool).await.unwrap();
        edit.body = "changed but not durable".into();
        assert!(save_saved_query(&state, edit)
            .await
            .unwrap_err()
            .contains("injected write failure"));
        assert_eq!(
            serde_json::to_value(load_saved_queries(&state).await.unwrap()).unwrap(),
            before
        );
        let rows = delete_saved_query(&state, DeleteSavedQueryPayload { id: "saved".into() })
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "older");
        assert_eq!(
            delete_saved_query(&state, DeleteSavedQueryPayload { id: "saved".into() })
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
