//! Engine dispatch — single point of truth for routing engine-aware
//! operations to the right per-engine implementation.
//!
//! Top-level routing happens here: every public function matches
//! `engine.storage_class()` first and delegates to either
//! `dispatch::relational::*` or `dispatch::keyvalue::*`. Per-engine
//! match statements live inside those submodules. See ADR-0008 for
//! the storage-class fork and ADR-0001 for the per-feature catch-up
//! policy.
//!
//! ## Two error shapes
//!
//! - [`relational::friendly_sqlx_error`] / Redis-specific errors —
//!   surfaced verbatim from the engine implementation.
//! - [`not_applicable`] — the operation does not exist for the
//!   engine's class (DDL on Redis, pub/sub on PG). Returned at the
//!   routing layer so each cross-class case is one branch, not one
//!   arm per engine.
//!
//! ## No `_ =>` wildcards
//!
//! Within each class submodule, matches over `DatabaseEngine` are
//! exhaustive. Adding a relational engine forces a match arm in
//! `relational::*`; adding a keyvalue engine forces an arm in
//! `keyvalue::*`. The cross-class `unreachable!()` branches in
//! relational submodule (asserting Redis never reaches there) are
//! invariant assertions, not wildcards.

pub(crate) mod keyvalue;
pub(crate) mod relational;

// Re-export the helpers the engine modules (postgres, clickhouse) and
// the Tauri command layer (lib.rs) reach for via `crate::dispatch::*`.
// They live inside `relational` because they're sqlx/SQL-shaped helpers.
pub(crate) use relational::{
    build_paged_select_query, ensure_sqlx_drivers, friendly_sqlx_error, should_fetch_rows,
};

use crate::{
    CellEdit, CellEditKeyValue, CommitCellEditsResult, ConnectResult, CopyTableResult,
    DatabaseOverviewStats, DeleteRowsResult, ExecuteDdlResult, ExportDdlResult, ImportRowsResult,
    InsertRowResult, MutationStatus, PgAdminSnapshot, PgBackendActionResult, PgDumpResult,
    PgRestoreResult, QueryResult, RelationInfo, SchemaExplorer, SchemaRelationships,
    SeedColumnSpec, SeedTableResult, ServerDetails, StorageClass, StoredConnection, TableStructure,
};

/// "This operation does not exist on this engine's class." Reserved
/// for the storage-class boundary — Redis getting asked for
/// `execute_ddl`, a future relational engine getting asked for
/// `scan_keys`. The frontend can choose to hide the affordance rather
/// than render a "not supported" wall.
fn not_applicable(connection: &StoredConnection, operation: &str) -> String {
    format!(
        "{operation} does not apply to {} — the concept does not exist on \
         this engine class.",
        relational::engine_name(&connection.engine())
    )
}

// ---------------------------------------------------------------------------
// Storage-class router — one branch per cross-class case, per operation.
// ---------------------------------------------------------------------------

/// Verify the connection is live and measure latency. Both classes
/// implement this — relational uses sqlx connect, keyvalue runs the
/// Redis capabilities probe (PING + INFO + MODULE LIST + DBSIZE).
pub async fn ping_connection(connection: &StoredConnection) -> Result<ConnectResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::ping_connection(connection).await,
        StorageClass::KeyValue => keyvalue::ping_connection(connection).await,
    }
}

pub async fn run_query(connection: &StoredConnection, query: &str) -> Result<QueryResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::run_query(connection, query).await,
        StorageClass::KeyValue => Err(not_applicable(connection, "SQL query execution")),
    }
}

pub async fn load_schema_explorer(
    connection: &StoredConnection,
) -> Result<Vec<SchemaExplorer>, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::load_schema_explorer(connection).await,
        StorageClass::KeyValue => Err(not_applicable(connection, "Schema explorer")),
    }
}

pub async fn fetch_table_structure(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
) -> Result<TableStructure, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => {
            relational::fetch_table_structure(connection, schema, table).await
        }
        StorageClass::KeyValue => Err(not_applicable(connection, "Table structure")),
    }
}

pub async fn fetch_schema_relationships(
    connection: &StoredConnection,
    schema: &str,
) -> Result<SchemaRelationships, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => {
            relational::fetch_schema_relationships(connection, schema).await
        }
        StorageClass::KeyValue => Err(not_applicable(connection, "Schema relationships")),
    }
}

pub async fn fetch_table_schema_relationships(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
) -> Result<SchemaRelationships, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => {
            relational::fetch_table_schema_relationships(connection, schema, table).await
        }
        StorageClass::KeyValue => Err(not_applicable(connection, "Schema relationships")),
    }
}

pub async fn fetch_database_overview_stats(
    connection: &StoredConnection,
) -> Result<DatabaseOverviewStats, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::fetch_database_overview_stats(connection).await,
        // Phase 1.3 lights up the keyvalue overview (KeyValueOverviewStats)
        // behind a separate Tauri command (`fetch_keyvalue_overview`). The
        // shared `DatabaseOverviewStats` envelope stays relational-only.
        StorageClass::KeyValue => Err(not_applicable(connection, "Database overview stats")),
    }
}

pub async fn fetch_relation_stats(
    connection: &StoredConnection,
) -> Result<Vec<RelationInfo>, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::fetch_relation_stats(connection).await,
        StorageClass::KeyValue => Err(not_applicable(connection, "Relation stats")),
    }
}

pub async fn fetch_server_details(connection: &StoredConnection) -> Result<ServerDetails, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::fetch_server_details(connection).await,
        StorageClass::KeyValue => Err(not_applicable(connection, "Server details")),
    }
}

pub async fn fetch_admin_snapshot(
    connection: &StoredConnection,
) -> Result<PgAdminSnapshot, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::fetch_admin_snapshot(connection).await,
        StorageClass::KeyValue => Err(not_applicable(connection, "Admin tools")),
    }
}

pub async fn cancel_backend(
    connection: &StoredConnection,
    pid: i32,
) -> Result<PgBackendActionResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::cancel_backend(connection, pid).await,
        StorageClass::KeyValue => Err(not_applicable(connection, "Backend cancel")),
    }
}

pub async fn terminate_backend(
    connection: &StoredConnection,
    pid: i32,
) -> Result<PgBackendActionResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::terminate_backend(connection, pid).await,
        StorageClass::KeyValue => Err(not_applicable(connection, "Backend terminate")),
    }
}

pub async fn execute_ddl(
    connection: &StoredConnection,
    sql: &str,
) -> Result<ExecuteDdlResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::execute_ddl(connection, sql).await,
        StorageClass::KeyValue => Err(not_applicable(connection, "DDL execution")),
    }
}

pub async fn export_ddl(
    connection: &StoredConnection,
    scope: &str,
    schema: Option<&str>,
    table: Option<&str>,
) -> Result<ExportDdlResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::export_ddl(connection, scope, schema, table).await,
        StorageClass::KeyValue => Err(not_applicable(connection, "DDL export")),
    }
}

pub async fn run_pg_dump(
    connection: &StoredConnection,
    scope: &str,
    schema: Option<&str>,
    table: Option<&str>,
    format: &str,
) -> Result<PgDumpResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => {
            relational::run_pg_dump(connection, scope, schema, table, format).await
        }
        StorageClass::KeyValue => Err(not_applicable(connection, "PostgreSQL dump")),
    }
}

pub async fn run_pg_restore(
    connection: &StoredConnection,
    data_base64: &str,
    format: &str,
    clean: bool,
) -> Result<PgRestoreResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => {
            relational::run_pg_restore(connection, data_base64, format, clean).await
        }
        StorageClass::KeyValue => Err(not_applicable(connection, "PostgreSQL restore")),
    }
}

pub async fn refresh_materialized_view(
    connection: &StoredConnection,
    schema: &str,
    view: &str,
    concurrently: bool,
) -> Result<ExecuteDdlResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => {
            relational::refresh_materialized_view(connection, schema, view, concurrently).await
        }
        StorageClass::KeyValue => Err(not_applicable(connection, "Materialized view refresh")),
    }
}

pub async fn run_maintenance(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    action: &str,
) -> Result<ExecuteDdlResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => {
            relational::run_maintenance(connection, schema, table, action).await
        }
        StorageClass::KeyValue => Err(not_applicable(connection, "Table maintenance")),
    }
}

pub async fn commit_cell_edits(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    edits: &[CellEdit],
) -> Result<CommitCellEditsResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => {
            relational::commit_cell_edits(connection, schema, table, edits).await
        }
        StorageClass::KeyValue => Err(not_applicable(connection, "Cell edit commit")),
    }
}

pub async fn insert_row(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    values: &[CellEditKeyValue],
) -> Result<InsertRowResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::insert_row(connection, schema, table, values).await,
        StorageClass::KeyValue => Err(not_applicable(connection, "Row insert")),
    }
}

pub async fn seed_table(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    row_count: u32,
    seed: u64,
    specs: &[SeedColumnSpec],
) -> Result<SeedTableResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => {
            relational::seed_table(connection, schema, table, row_count, seed, specs).await
        }
        StorageClass::KeyValue => Err(not_applicable(connection, "Table seeding")),
    }
}

pub async fn import_rows(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    columns: &[String],
    rows: &[Vec<Option<String>>],
    use_copy: bool,
) -> Result<ImportRowsResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => {
            relational::import_rows(connection, schema, table, columns, rows, use_copy).await
        }
        StorageClass::KeyValue => Err(not_applicable(connection, "Row import")),
    }
}

pub async fn copy_table_rows(
    source: &StoredConnection,
    destination: &StoredConnection,
    source_schema: &str,
    source_table: &str,
    destination_schema: &str,
    destination_table: &str,
    page_size: u32,
) -> Result<CopyTableResult, String> {
    match (
        source.engine().storage_class(),
        destination.engine().storage_class(),
    ) {
        (StorageClass::Relational, StorageClass::Relational) => {
            relational::copy_table_rows(
                source,
                destination,
                source_schema,
                source_table,
                destination_schema,
                destination_table,
                page_size,
            )
            .await
        }
        _ => Err("Table copy requires two relational connections".to_string()),
    }
}

pub async fn delete_rows(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    rows: &[Vec<CellEditKeyValue>],
) -> Result<DeleteRowsResult, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => relational::delete_rows(connection, schema, table, rows).await,
        StorageClass::KeyValue => Err(not_applicable(connection, "Row delete")),
    }
}

pub async fn poll_mutation_status(
    connection: &StoredConnection,
    database: &str,
    table: &str,
    mutation_ids: &[String],
) -> Result<Vec<MutationStatus>, String> {
    match connection.engine().storage_class() {
        StorageClass::Relational => {
            relational::poll_mutation_status(connection, database, table, mutation_ids).await
        }
        StorageClass::KeyValue => Err(not_applicable(connection, "Mutation polling")),
    }
}

#[cfg(test)]
mod tests {
    use crate::{DatabaseEngine, StorageClass};

    /// Snapshot: the storage class of every `DatabaseEngine` variant.
    /// The mirror test on the TS side asserts the same classification;
    /// drift between the two breaks CI.
    #[test]
    fn storage_class_is_stable_per_engine() {
        let cases: &[(DatabaseEngine, StorageClass)] = &[
            (DatabaseEngine::PostgreSQL, StorageClass::Relational),
            (DatabaseEngine::MySQL, StorageClass::Relational),
            (DatabaseEngine::SQLite, StorageClass::Relational),
            (DatabaseEngine::ClickHouse, StorageClass::Relational),
            (DatabaseEngine::Redis, StorageClass::KeyValue),
        ];
        for (engine, expected) in cases {
            assert_eq!(engine.storage_class(), *expected, "engine={:?}", engine);
        }
    }
}
