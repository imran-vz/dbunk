//! PostgreSQL engine implementation.
//!
//! Split into focused sub-modules by concern:
//! - `pool` — connection pool cache, connect/disconnect
//! - `query` — ad-hoc query execution
//! - `mutations` — cell edits, inserts, deletes, imports
//! - `schema` — table structure and schema relationship introspection
//! - `relationship_metadata` — pure cardinality/junction/trigger classification
//! - `ddl` — DDL execution, export, pg_dump/pg_restore, materialized views
//! - `admin` — server details, overview stats, session/lock snapshots, maintenance

mod admin;
pub(crate) mod connect_error;
pub(crate) mod connect_spec;
mod ddl;
pub(crate) mod dedicated;
pub(crate) mod identity;
mod mutations;
pub(crate) mod options;
mod pool;
mod query;
mod relationship_metadata;
pub(crate) mod row_budget;
mod schema;
mod seed;
pub(crate) mod sql_class;
pub(crate) mod sql_lex;
mod table_relationships;
pub(crate) mod tls;

use sqlx::postgres::PgRow;
use sqlx::Row;

use crate::{bytes_to_hex, StoredConnection};

// ---------------------------------------------------------------------------
// Re-exports — the external interface stays flat (`crate::postgres::*`)
// ---------------------------------------------------------------------------

pub use admin::{
    cancel_backend, load_admin_snapshot, load_database_overview_stats, load_relation_stats,
    load_server_details, run_maintenance, terminate_backend,
};
pub use ddl::{execute_ddl, export_ddl, refresh_materialized_view, run_pg_dump, run_pg_restore};
pub use mutations::{commit_cell_edits, copy_import_rows, delete_rows, import_rows, insert_row};
pub use pool::drop_pool;
pub use query::run_query;
pub use schema::{fetch_schema_relationships, fetch_table_structure};
pub use seed::seed_table;
pub use table_relationships::fetch_table_schema_relationships;

// ---------------------------------------------------------------------------
// Shared helpers used by multiple sub-modules
// ---------------------------------------------------------------------------

/// Acquire a connection from the cached pool.
pub(crate) use pool::connect;

/// Narrow a `StoredConnection` to its PostgreSQL variant.
fn pg_connection(connection: &StoredConnection) -> Result<&crate::PgStoredConnection, String> {
    let StoredConnection::PostgreSQL(connection) = connection else {
        return Err("PostgreSQL native tooling requires a PostgreSQL connection".to_string());
    };
    Ok(connection)
}

// ---------------------------------------------------------------------------
// Row → string coercion
// ---------------------------------------------------------------------------

/// Convert a single PG cell to its string display form.
///
/// PG's native driver carries richer type information than sqlx-Any, so we
/// probe known column types via `try_get::<Option<T>, _>` and fall through
/// to "NULL" if nothing matches. Order matters: integer `try_get` succeeds
/// for numeric-looking text on some drivers, so we try strings first.
pub(crate) fn value_to_string(row: &PgRow, index: usize) -> String {
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return value.unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<i32>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<i16>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<f32>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<bool>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(index) {
        return value
            .map(|v| v.to_rfc3339())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<chrono::NaiveDateTime>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<chrono::NaiveDate>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<chrono::NaiveTime>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<uuid::Uuid>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<serde_json::Value>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return value
            .map(|bytes| bytes_to_hex(&bytes))
            .unwrap_or_else(|| "NULL".to_string());
    }
    "NULL".to_string()
}

pub(crate) fn row_to_strings(row: &PgRow) -> Vec<String> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(index, _)| value_to_string(row, index))
        .collect()
}
