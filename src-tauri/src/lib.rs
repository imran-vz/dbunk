//! dbunk's backend.
//!
//! Everything outside `commands` and `tauri_host` is host-neutral: it builds
//! and tests with `--no-default-features`, with no Tauri in the dependency
//! graph. The `tauri-host` feature (default) adds the Tauri command adapters
//! and the application entry point. See `host` for what a host supplies and
//! ADR-0032 for the boundary.

// Until every family's service is extracted from `commands`, much of the
// backend has its only callers there, so a build without the Tauri host sees
// those functions as unused. The default build keeps the lints on.
#![cfg_attr(not(feature = "tauri-host"), allow(dead_code, unused_imports))]

mod app;
#[cfg(feature = "isolated-profile")]
pub mod backend;
mod clickhouse;
#[cfg(feature = "tauri-host")]
mod commands;
mod connections;
mod credentials;
mod diagnosis;
mod dispatch;
mod docker;
mod host;
mod keychain;
mod managed;
mod postgres;
mod query_library;
mod query_session;
mod redis;
mod result_mutation;
mod safety;
mod seed;
mod settings;
mod socket_lifecycle;
mod storage;
mod table_browse;
#[cfg(feature = "tauri-host")]
mod tauri_host;
mod tunnel;
mod types;
mod xlsx;
#[cfg(feature = "isolated-profile")]
mod xlsx_native;

#[cfg(all(test, feature = "tauri-host"))]
#[path = "commands/pg_backup/lifecycle_tests.rs"]
mod pg_backup_lifecycle_tests;

// Re-export DTOs at the crate root so existing `crate::Foo` paths in child
// modules and the `#[tauri::command]` macros keep working unchanged.
pub(crate) use types::*;

#[cfg(feature = "tauri-host")]
pub use tauri_host::run;

pub(crate) use app::{close_socket_managers_for_exit, AppState};
#[cfg(test)]
pub(crate) use app::{configure_test_keyring, test_app_state};

pub(crate) const MAX_QUERY_HISTORY: usize = storage::QUERY_HISTORY_CAP as usize;

pub(crate) const DEFAULT_TABLE_PAGE_SIZE: u32 = 100;
pub(crate) const MAX_TABLE_PAGE_SIZE: u32 = 1000;

// ---------------------------------------------------------------------------
// Utility functions used by engine modules
// ---------------------------------------------------------------------------

pub(crate) fn quote_double(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub(crate) fn quote_literal(value: &str) -> String {
    format!("E'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}

pub(crate) fn quote_backtick(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

pub(crate) fn qualified_table_name(engine: &DatabaseEngine, schema: &str, table: &str) -> String {
    match engine {
        DatabaseEngine::PostgreSQL | DatabaseEngine::SQLite => {
            if schema.is_empty() {
                quote_double(table)
            } else {
                format!("{}.{}", quote_double(schema), quote_double(table))
            }
        }
        DatabaseEngine::MySQL | DatabaseEngine::ClickHouse => {
            if schema.is_empty() {
                quote_backtick(table)
            } else {
                format!("{}.{}", quote_backtick(schema), quote_backtick(table))
            }
        }
        DatabaseEngine::Redis => {
            unreachable!("BUG: qualified_table_name called on Redis connection")
        }
    }
}

pub(crate) fn parse_total_rows(result: &QueryResult) -> Option<u64> {
    result
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(|cell| cell.parse::<u64>().ok())
}

/// Bytes → `0x` hex string. Used by every engine's value coercion path
/// for binary column display (Postgres `bytea`, sqlx-Any `Vec<u8>`),
/// kept at the crate root so each engine module can reach it without
/// re-implementing.
pub(crate) fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2 + 2);
    output.push_str("0x");
    for byte in bytes {
        output.push_str(&format!("{:02x}", byte));
    }
    output
}

/// tokio-postgres logs bound parameter values at Debug. Query Session
/// parameter values must never reach a log (ADR-0031), so this stays at Warn
/// or stricter whatever the crate's own level is.
pub(crate) const TOKIO_POSTGRES_LOG_LEVEL: log::LevelFilter = log::LevelFilter::Warn;
