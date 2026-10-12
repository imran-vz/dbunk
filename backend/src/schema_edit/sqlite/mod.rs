//! SQLite schema edits (Plan 033 lane B): capture by parsing
//! `sqlite_master.sql` and cross-checking PRAGMAs, a fast path for ADD /
//! RENAME / DROP COLUMN, table renames and index changes, and the
//! documented 12-step table rebuild for everything else. Runs on the
//! session worker's connection.
//!
//! - `lexer`: byte-exact tokens, so captured expressions are slices of
//!   the stored definition and an untouched expression is never a change.
//! - `definition`: the `CREATE TABLE` / `CREATE INDEX` parser and the
//!   `sqlparser` cross-check.
//! - `capture`: catalog reads and the PRAGMA cross-check; any
//!   disagreement is a read-only baseline with a reason.
//! - `planner`: fast path or rebuild, one atomic group.
//! - `executor` and `verify`: the transaction, verification and
//!   restoration of connection settings.
mod capture;
mod definition;
mod executor;
mod lexer;
mod planner;
mod verify;

use super::{
    BaselineId, EnginePlan, Origin, PlanError, PlanOptions, SchemaBaseline, SchemaChangeSet,
    SchemaDraft, SchemaEditOutcome, SchemaEditPreview, ServerFeatures,
};
use futures_util::future::BoxFuture;
use sqlx::SqliteConnection;

/// Baseline id tags for non-column objects: `tag << 48 | n`, so ids never
/// collide with column ids (`cid + 1`) or across kinds.
const TAG_PRIMARY_KEY: u64 = 1;
const TAG_UNIQUE: u64 = 2;
const TAG_FOREIGN_KEY: u64 = 3;
const TAG_CHECK: u64 = 4;
const TAG_INDEX: u64 = 5;

fn column_origin(cid: i64) -> Origin {
    Origin::Existing(BaselineId(u64::try_from(cid).unwrap_or(0) + 1))
}

fn object_origin(tag: u64, n: usize) -> Origin {
    Origin::Existing(BaselineId((tag << 48) | n as u64))
}

/// `3.45.1` as `(3, 45, 1)`; missing or unreadable parts are zero.
fn parse_version(text: &str) -> (u32, u32, u32) {
    let mut parts = text.trim().split('.').map(|part| {
        part.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse::<u32>()
            .unwrap_or(0)
    });
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// What the SQLite version supports. Column reorder is not an ALTER on
/// SQLite; the planner offers it through the rebuild.
fn features(version: (u32, u32, u32)) -> ServerFeatures {
    ServerFeatures {
        rename_column: version >= (3, 25, 0),
        drop_column: version >= (3, 35, 0),
        enforced_check: true,
        transactional_ddl: true,
        column_reorder: false,
        ..ServerFeatures::default()
    }
}

fn quote(name: &str) -> String {
    crate::quote_double(name)
}

/// `'text'`, with quotes doubled.
fn quote_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn qualified(schema: &str, name: &str) -> String {
    format!("{}.{}", quote(schema), quote(name))
}

/// Plans `draft` as ordered SQLite statements. Pure and deterministic.
pub fn plan(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    _changes: &SchemaChangeSet,
    options: &PlanOptions,
) -> Result<EnginePlan, PlanError> {
    // Apply re-plans with the reviewed options; none of them means anything
    // here, so a preview never claims one.
    if *options != PlanOptions::default() {
        return Err(PlanError::Unsupported {
            reason: "Online-only, concurrent indexes, separate validation and USING are not \
                     SQLite options"
                .into(),
        });
    }
    planner::plan(baseline, draft)
}

/// Captures `schema.table`. Errors are user-facing text; a definition that
/// cannot be parsed or disagrees with the PRAGMAs yields a `ReadOnly`
/// baseline, not an error.
///
/// Boxed so the session worker's future is provably `Send` (the inner
/// futures borrow across awaits in ways the compiler cannot generalize).
pub(crate) fn capture<'a>(
    connection: &'a mut SqliteConnection,
    schema: &'a str,
    table: &'a str,
    sqlite_version: &'a str,
) -> BoxFuture<'a, Result<SchemaBaseline, String>> {
    Box::pin(capture::capture_table(
        connection,
        schema,
        table,
        sqlite_version,
        true,
    ))
}

/// Runs a reviewed plan on the session connection. The caller has already
/// re-planned and compared `preview` and checked the policy.
pub(crate) fn execute<'a>(
    connection: &'a mut SqliteConnection,
    baseline: &'a SchemaBaseline,
    draft: &'a SchemaDraft,
    preview: &'a SchemaEditPreview,
) -> BoxFuture<'a, SchemaEditOutcome> {
    Box::pin(executor::execute(connection, baseline, draft, preview))
}

#[cfg(test)]
mod live_tests;
