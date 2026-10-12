//! Apply on a dedicated short connection to the session's resolved endpoint,
//! so DDL's implicit commit never touches a transaction the user has open in
//! the query tab.
//!
//! 1. Connect (bounded), clear `sql_mode` flags that change literal or
//!    definition rendering, and set `lock_wait_timeout` so a metadata lock
//!    fails fast.
//! 2. Re-capture and compare the drift token; a mismatch dispatches nothing.
//! 3. Run the statements in order. MySQL DDL is atomic per statement on
//!    InnoDB, so a server error on the first statement leaves the table as it
//!    was (`RolledBack`), and on a later one leaves the earlier statements
//!    committed (`Stopped`). A lost connection after dispatch is
//!    `OutcomeUnknown`. A statement that outlives its budget is stopped with
//!    `KILL QUERY` from a side connection.
//! 4. Re-capture and verify against the draft. Every statement has committed
//!    by then, so a mismatch cannot be rolled back; it is reported as
//!    `OutcomeUnknown { VerificationFailed }`, which tells the host to reload
//!    and reconcile and never to replay.
use super::build::safe_sql_mode;
use super::catalog::{cell, message};
use super::sql::{canonical_type, is_textual, loose_expression, same_action};
use crate::schema_edit::{
    types_equal, ColumnModel, DefaultValue, IndexTarget, SchemaBaseline, SchemaDraft,
    SchemaEditFailure, SchemaEditOutcome, SchemaEditPreview, SchemaEngine, TableModel,
};
use sqlx::mysql::{MySqlConnectOptions, MySqlConnection, MySqlDatabaseError};
use sqlx::{ConnectOptions, Connection, Executor};
use std::time::{Duration, Instant};
use tokio::time::timeout;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Setup plus the drift re-capture, and the verification re-capture.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);
/// One `ALTER TABLE`. A copying change on a large table can run long; past
/// this it is stopped with `KILL QUERY`, which InnoDB rolls back.
const STATEMENT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const KILL_TIMEOUT: Duration = Duration::from_secs(5);
/// After `KILL QUERY`, the statement must hand the connection back within
/// this budget or its outcome is unknown.
const KILL_GRACE: Duration = Duration::from_secs(60);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// Seconds an `ALTER TABLE` waits for the table's metadata lock.
pub(super) const LOCK_WAIT_TIMEOUT_SECONDS: u32 = 10;

/// `ER_LOCK_WAIT_TIMEOUT`.
const LOCK_WAIT_TIMEOUT: u16 = 1205;
/// `ER_QUERY_INTERRUPTED`: stopped by `KILL QUERY`.
const QUERY_INTERRUPTED: u16 = 1317;

fn number(error: &sqlx::Error) -> Option<u16> {
    match error {
        sqlx::Error::Database(database) => database
            .try_downcast_ref::<MySqlDatabaseError>()
            .map(MySqlDatabaseError::number),
        _ => None,
    }
}

/// Transport and protocol failures, and server errors that end the
/// connection.
fn is_fatal(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(_) => matches!(
            number(error),
            // Shutdown in progress, connection killed, idle timeout, session
            // killed, server gone, connection lost.
            Some(1053 | 1927 | 4031 | 3169 | 2006 | 2013)
        ),
        sqlx::Error::Io(_)
        | sqlx::Error::Tls(_)
        | sqlx::Error::Protocol(_)
        | sqlx::Error::WorkerCrashed
        | sqlx::Error::PoolClosed
        | sqlx::Error::PoolTimedOut => true,
        _ => false,
    }
}

fn database_failure(error: &sqlx::Error) -> SchemaEditFailure {
    match error {
        sqlx::Error::Database(database) => {
            match database.try_downcast_ref::<MySqlDatabaseError>() {
                Some(mysql) => SchemaEditFailure::Database {
                    code: Some(mysql.number().to_string()),
                    message: mysql.message().to_owned(),
                },
                None => SchemaEditFailure::Database {
                    code: None,
                    message: database.message().to_owned(),
                },
            }
        }
        other => SchemaEditFailure::Database {
            code: None,
            message: message(other),
        },
    }
}

/// A failure before any DDL was sent.
fn setup_failure(error: &sqlx::Error) -> SchemaEditFailure {
    if is_fatal(error) {
        SchemaEditFailure::Connection
    } else {
        database_failure(error)
    }
}

enum Ran {
    Done,
    /// The server refused the statement; it did not apply.
    Failed(SchemaEditFailure),
    /// The statement's outcome is unknown.
    Lost(SchemaEditFailure),
}

fn classify(error: &sqlx::Error) -> Ran {
    if is_fatal(error) {
        return Ran::Lost(SchemaEditFailure::Connection);
    }
    match number(error) {
        Some(LOCK_WAIT_TIMEOUT) => Ran::Failed(SchemaEditFailure::LockTimeout),
        Some(QUERY_INTERRUPTED) => Ran::Failed(SchemaEditFailure::Cancelled),
        Some(_) => Ran::Failed(database_failure(error)),
        // Not a server reply: nothing says whether the statement ran.
        None => Ran::Lost(database_failure(error)),
    }
}

async fn close(connection: MySqlConnection) {
    let _ = timeout(CLOSE_TIMEOUT, connection.close()).await;
}

/// `KILL QUERY` from a short side connection.
async fn kill_query(options: &MySqlConnectOptions, thread: u64) {
    if thread == 0 {
        return;
    }
    let Ok(Ok(mut side)) = timeout(CONNECT_TIMEOUT, options.connect()).await else {
        return;
    };
    let _ = timeout(
        KILL_TIMEOUT,
        (&mut side).execute(format!("KILL QUERY {thread}").as_str()),
    )
    .await;
    close(side).await;
}

/// Session setup and the drift check. Returns the connection's thread id.
async fn prepare(
    connection: &mut MySqlConnection,
    baseline: &SchemaBaseline,
) -> Result<u64, SchemaEditFailure> {
    let row = (&mut *connection)
        .fetch_one(
            "SELECT CAST(CONNECTION_ID() AS CHAR), CAST(VERSION() AS CHAR), \
             CAST(@@SESSION.sql_mode AS CHAR)",
        )
        .await
        .map_err(|error| setup_failure(&error))?;
    let thread = cell(&row, 0)
        .and_then(|id| id.trim().parse().ok())
        .unwrap_or_default();
    let version = cell(&row, 1).unwrap_or_default();
    // Strict mode makes a narrowing ALTER fail instead of truncating data
    // with only a warning on a non-strict server.
    let mut mode = safe_sql_mode(&cell(&row, 2).unwrap_or_default());
    if !mode
        .split(',')
        .any(|flag| flag.eq_ignore_ascii_case("STRICT_ALL_TABLES"))
    {
        if !mode.is_empty() {
            mode.push(',');
        }
        mode.push_str("STRICT_ALL_TABLES");
    }
    sqlx::query("SET SESSION sql_mode = ?")
        .bind(mode)
        .execute(&mut *connection)
        .await
        .map_err(|error| setup_failure(&error))?;
    (&mut *connection)
        .execute(format!("SET SESSION lock_wait_timeout = {LOCK_WAIT_TIMEOUT_SECONDS}").as_str())
        .await
        .map_err(|error| setup_failure(&error))?;
    let current = super::capture(
        connection,
        &version,
        &baseline.table.schema,
        &baseline.table.name,
    )
    .await
    // A missing table has changed since review; other failures (a lost
    // connection, a permission error) are reported as they are.
    .map_err(|message| {
        if message.ends_with("was not found") {
            SchemaEditFailure::BaselineChanged
        } else {
            SchemaEditFailure::Database {
                code: None,
                message,
            }
        }
    })?;
    if current.drift != baseline.drift {
        return Err(SchemaEditFailure::BaselineChanged);
    }
    Ok(thread)
}

async fn run(
    connection: &mut MySqlConnection,
    sql: &str,
    options: &MySqlConnectOptions,
    thread: u64,
) -> Ran {
    let mut work = (&mut *connection).execute(sql);
    match timeout(STATEMENT_TIMEOUT, &mut work).await {
        Ok(Ok(_)) => Ran::Done,
        Ok(Err(error)) => classify(&error),
        Err(_) => {
            kill_query(options, thread).await;
            match timeout(KILL_GRACE, &mut work).await {
                // It finished just as the kill was sent.
                Ok(Ok(_)) => Ran::Done,
                Ok(Err(error)) if number(&error) == Some(QUERY_INTERRUPTED) => {
                    Ran::Failed(SchemaEditFailure::Timeout)
                }
                Ok(Err(error)) => classify(&error),
                Err(_) => Ran::Lost(SchemaEditFailure::Timeout),
            }
        }
    }
}

pub(super) async fn execute(
    options: &MySqlConnectOptions,
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    preview: &SchemaEditPreview,
) -> SchemaEditOutcome {
    if preview.engine != SchemaEngine::MySql || baseline.engine != SchemaEngine::MySql {
        return SchemaEditOutcome::NotDispatched {
            reason: SchemaEditFailure::Unsupported {
                reason: "The plan is not a MySQL plan".into(),
            },
        };
    }
    let mut connection = match timeout(CONNECT_TIMEOUT, options.connect()).await {
        Ok(Ok(connection)) => connection,
        Ok(Err(_)) | Err(_) => {
            return SchemaEditOutcome::NotDispatched {
                reason: SchemaEditFailure::Connection,
            }
        }
    };
    let thread = match timeout(CAPTURE_TIMEOUT, prepare(&mut connection, baseline)).await {
        Ok(Ok(thread)) => thread,
        Ok(Err(reason)) => {
            close(connection).await;
            return SchemaEditOutcome::NotDispatched { reason };
        }
        // Only catalog reads were in flight; dropping the connection ends
        // them.
        Err(_) => {
            return SchemaEditOutcome::NotDispatched {
                reason: SchemaEditFailure::Timeout,
            }
        }
    };
    let started = Instant::now();
    for (index, statement) in preview.statements.iter().enumerate() {
        match run(&mut connection, &statement.sql, options, thread).await {
            Ran::Done => {}
            Ran::Failed(reason) => {
                close(connection).await;
                return if index == 0 {
                    SchemaEditOutcome::RolledBack { reason }
                } else {
                    SchemaEditOutcome::Stopped {
                        committed: index,
                        failed_statement: index,
                        reason,
                    }
                };
            }
            // Protocol state unknown: drop without COM_QUIT.
            Ran::Lost(reason) => return SchemaEditOutcome::OutcomeUnknown { reason },
        }
    }
    let runtime_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let verified = timeout(
        CAPTURE_TIMEOUT,
        verify_applied(&mut connection, baseline, draft),
    )
    .await;
    let outcome = match verified {
        Ok(Ok(())) => SchemaEditOutcome::Applied { runtime_ms },
        Ok(Err(detail)) => SchemaEditOutcome::OutcomeUnknown {
            reason: SchemaEditFailure::VerificationFailed { detail },
        },
        Err(_) => SchemaEditOutcome::OutcomeUnknown {
            reason: SchemaEditFailure::VerificationFailed {
                detail: "the table could not be re-read in time".into(),
            },
        },
    };
    close(connection).await;
    outcome
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

async fn verify_applied(
    connection: &mut MySqlConnection,
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
) -> Result<(), String> {
    let captured = super::capture(
        connection,
        &baseline.server.version,
        &draft.table.schema,
        &draft.table.name,
    )
    .await
    .map_err(|error| format!("the table could not be re-read: {error}"))?;
    let problems = verify(baseline, draft, &captured);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Problems reported per verification, at most.
const MAX_PROBLEMS: usize = 8;

fn same_text(a: Option<&str>, b: Option<&str>) -> bool {
    a.unwrap_or_default() == b.unwrap_or_default()
}

fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// An index or unique key as comparable text.
#[derive(Debug)]
struct IndexShape {
    name: Option<String>,
    unique: bool,
    method: String,
    parts: Vec<String>,
}

fn shapes(table: &TableModel) -> Vec<IndexShape> {
    let column = |origin| {
        table
            .column_name(origin)
            .unwrap_or("?")
            .to_ascii_lowercase()
    };
    let mut shapes: Vec<IndexShape> = table
        .uniques
        .iter()
        .map(|key| IndexShape {
            name: key.name.clone(),
            unique: true,
            method: "BTREE".into(),
            parts: key.columns.iter().map(|origin| column(*origin)).collect(),
        })
        .collect();
    shapes.extend(table.indexes.iter().map(|index| {
        IndexShape {
            name: Some(index.name.clone()),
            unique: index.unique,
            method: index
                .method
                .clone()
                .unwrap_or_else(|| "BTREE".into())
                .to_ascii_uppercase(),
            parts: index
                .members
                .iter()
                .map(|member| {
                    let mut part = match &member.target {
                        IndexTarget::Column(origin) => column(*origin),
                        IndexTarget::Expression(expression) => {
                            format!("({})", loose_expression(expression.as_str()))
                        }
                    };
                    if let Some(length) = member.prefix_length {
                        part.push_str(&format!("({length})"));
                    }
                    if member.descending {
                        part.push_str(" desc");
                    }
                    part
                })
                .collect(),
        }
    }));
    shapes
}

/// Compares the re-captured table with the draft. Structure (names, order,
/// types, nullability, keys, members, references) is compared exactly, with
/// types canonicalized. Defaults are compared byte for byte when the draft
/// left them as captured, and by kind (or loosely) when edited, since the
/// server normalizes what it stores. Check and generated expressions are
/// always compared loosely: a table copy re-renders even untouched ones
/// (a literal's charset introducer follows the column's charset). A
/// draft charset or collation of `None` (inherit) accepts whatever the table
/// default gave.
pub(super) fn verify(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    captured: &SchemaBaseline,
) -> Vec<String> {
    let old = &baseline.table;
    let want = &draft.table;
    let got = &captured.table;
    let mut problems = Vec::new();

    if got.name != want.name {
        problems.push(format!("table name is {}", got.name));
    }
    if !same_text(want.comment.as_deref(), got.comment.as_deref()) {
        problems.push("table comment differs".into());
    }
    for (what, before, wanted, found) in [
        (
            "engine",
            &old.options.engine,
            &want.options.engine,
            &got.options.engine,
        ),
        (
            "character set",
            &old.options.charset,
            &want.options.charset,
            &got.options.charset,
        ),
        (
            "collation",
            &old.options.collation,
            &want.options.collation,
            &got.options.collation,
        ),
        (
            "row format",
            &old.options.row_format,
            &want.options.row_format,
            &got.options.row_format,
        ),
    ] {
        if wanted != before
            && !wanted
                .as_deref()
                .unwrap_or("DEFAULT")
                .eq_ignore_ascii_case(found.as_deref().unwrap_or("DEFAULT"))
        {
            problems.push(format!("table {what} differs"));
        }
    }

    let want_names: Vec<&str> = want.columns.iter().map(|c| c.name.as_str()).collect();
    let got_names: Vec<&str> = got.columns.iter().map(|c| c.name.as_str()).collect();
    if want_names.len() != got_names.len()
        || want_names
            .iter()
            .zip(&got_names)
            .any(|(a, b)| !same_name(a, b))
    {
        problems.push(format!(
            "columns are [{}], expected [{}]",
            got_names.join(", "),
            want_names.join(", ")
        ));
    } else {
        for (wanted, found) in want.columns.iter().zip(&got.columns) {
            let before = old
                .column(wanted.origin)
                .filter(|_| !wanted.origin.is_new());
            column_problems(&mut problems, before, wanted, found);
        }
    }

    let primary = |table: &TableModel| {
        table.primary_key.as_ref().map(|key| {
            table
                .column_names(&key.columns)
                .iter()
                .map(|name| name.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
    };
    if primary(want) != primary(got) {
        problems.push("primary key differs".into());
    }

    // Indexes and unique keys.
    let wanted_shapes = shapes(want);
    let mut found_shapes: Vec<Option<IndexShape>> = shapes(got).into_iter().map(Some).collect();
    for shape in &wanted_shapes {
        let position = found_shapes.iter().position(|found| {
            found.as_ref().is_some_and(|found| match &shape.name {
                Some(name) => found.name.as_deref().is_some_and(|n| same_name(n, name)),
                None => {
                    found.unique == shape.unique
                        && found.method == shape.method
                        && found.parts == shape.parts
                }
            })
        });
        match position.and_then(|position| found_shapes[position].take()) {
            Some(found) => {
                if found.unique != shape.unique
                    || found.method != shape.method
                    || found.parts != shape.parts
                {
                    problems.push(format!(
                        "index {} differs",
                        shape.name.as_deref().unwrap_or_default()
                    ));
                }
            }
            None => problems.push(format!(
                "index {} is missing",
                shape.name.as_deref().unwrap_or("(unnamed)")
            )),
        }
    }
    // MySQL adds an index for a foreign key that has none, named after the
    // key; that one is expected.
    for found in found_shapes.into_iter().flatten() {
        let implicit = !found.unique
            && want.foreign_keys.iter().any(|key| {
                let columns: Vec<String> = want
                    .column_names(&key.columns)
                    .iter()
                    .map(|name| name.to_ascii_lowercase())
                    .collect();
                found.parts.len() <= columns.len()
                    && found.parts[..] == columns[..found.parts.len()]
                    && (key.name.is_none()
                        || found
                            .name
                            .as_deref()
                            .zip(key.name.as_deref())
                            .is_some_and(|(a, b)| same_name(a, b)))
            });
        if !implicit {
            problems.push(format!(
                "unexpected index {}",
                found.name.as_deref().unwrap_or_default()
            ));
        }
    }

    // Foreign keys.
    let mut found_keys: Vec<_> = got.foreign_keys.iter().map(Some).collect();
    for key in &want.foreign_keys {
        let columns = want.column_names(&key.columns);
        let position = found_keys.iter().position(|found| {
            found.is_some_and(|found| match &key.name {
                Some(name) => found.name.as_deref().is_some_and(|n| same_name(n, name)),
                None => {
                    got.column_names(&found.columns)
                        .iter()
                        .zip(&columns)
                        .all(|(a, b)| same_name(a, b))
                        && found.columns.len() == columns.len()
                }
            })
        });
        let Some(found) = position.and_then(|position| found_keys[position].take()) else {
            problems.push(format!(
                "foreign key {} is missing",
                key.name.as_deref().unwrap_or("(unnamed)")
            ));
            continue;
        };
        let found_columns = got.column_names(&found.columns);
        let schema = |value: &Option<String>, table: &TableModel| {
            value.clone().unwrap_or_else(|| table.schema.clone())
        };
        let same_columns = |a: &[String], b: &[String]| {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_name(x, y))
        };
        if !same_columns(&columns, &found_columns)
            || !same_name(&key.referenced_table, &found.referenced_table)
            || schema(&key.referenced_schema, want) != schema(&found.referenced_schema, got)
            || !same_columns(&key.referenced_columns, &found.referenced_columns)
            || !same_action(key.on_delete, found.on_delete)
            || !same_action(key.on_update, found.on_update)
        {
            problems.push(format!(
                "foreign key {} differs",
                key.name.as_deref().unwrap_or("(unnamed)")
            ));
        }
    }
    if found_keys.iter().any(Option::is_some) {
        problems.push("an unexpected foreign key exists".into());
    }

    // Checks.
    let mut found_checks: Vec<_> = got.checks.iter().map(Some).collect();
    for check in &want.checks {
        let position = found_checks.iter().position(|found| {
            found.is_some_and(|found| match &check.name {
                Some(name) => found.name.as_deref().is_some_and(|n| same_name(n, name)),
                None => {
                    loose_expression(found.expression.as_str())
                        == loose_expression(check.expression.as_str())
                }
            })
        });
        let Some(found) = position.and_then(|position| found_checks[position].take()) else {
            problems.push(format!(
                "check {} is missing",
                check.name.as_deref().unwrap_or("(unnamed)")
            ));
            continue;
        };
        let same_expression = loose_expression(found.expression.as_str())
            == loose_expression(check.expression.as_str());
        if !same_expression || found.enforced != check.enforced {
            problems.push(format!(
                "check {} differs",
                check.name.as_deref().unwrap_or("(unnamed)")
            ));
        }
    }
    if found_checks.iter().any(Option::is_some) {
        problems.push("an unexpected check exists".into());
    }

    problems.truncate(MAX_PROBLEMS);
    problems
}

fn default_kind(value: &DefaultValue) -> u8 {
    match value {
        DefaultValue::None => 0,
        DefaultValue::Literal(_) => 1,
        DefaultValue::Expression(_) => 2,
    }
}

fn column_problems(
    problems: &mut Vec<String>,
    before: Option<&ColumnModel>,
    wanted: &ColumnModel,
    found: &ColumnModel,
) {
    let name = &wanted.name;
    let mut differ = |what: &str| problems.push(format!("column {name}: {what} differs"));
    if canonical_type(&wanted.data_type) != canonical_type(&found.data_type) {
        differ("type");
    }
    if wanted.nullable != found.nullable {
        differ("nullability");
    }
    if wanted.identity != found.identity {
        differ("auto-increment");
    }
    if wanted.extra.invisible != found.extra.invisible {
        differ("visibility");
    }
    if !same_text(wanted.comment.as_deref(), found.comment.as_deref()) {
        differ("comment");
    }
    // After a type change the server re-renders an untouched default in the
    // new type's format (`0.00` → `0.0000`), so it is compared loosely.
    let untouched_default = before.is_some_and(|before| {
        before.default == wanted.default && types_equal(&before.data_type, &wanted.data_type)
    });
    let same_default = if untouched_default {
        found.default == wanted.default
    } else {
        match (&wanted.default, &found.default) {
            (DefaultValue::Expression(a), DefaultValue::Expression(b)) => {
                // `uuid()` is stored as `(uuid())`; `now()` may come back as
                // `CURRENT_TIMESTAMP`.
                loose_expression(a.as_str()) == loose_expression(b.as_str())
                    || default_kind(&wanted.default) == default_kind(&found.default)
            }
            (a, b) => default_kind(a) == default_kind(b),
        }
    };
    if !same_default {
        differ("default");
    }
    match (&wanted.generated, &found.generated) {
        (None, None) => {}
        (Some(a), Some(b)) => {
            let same_expression =
                loose_expression(a.expression.as_str()) == loose_expression(b.expression.as_str());
            if a.stored != b.stored || !same_expression {
                differ("generation");
            }
        }
        _ => differ("generation"),
    }
    match (&wanted.extra.on_update, &found.extra.on_update) {
        (None, None) => {}
        (Some(a), Some(b)) => {
            let untouched =
                before.is_some_and(|before| before.extra.on_update == wanted.extra.on_update);
            if untouched && a != b {
                differ("on update");
            }
        }
        _ => differ("on update"),
    }
    if is_textual(&wanted.data_type) {
        if let Some(charset) = &wanted.extra.charset {
            if !found
                .extra
                .charset
                .as_deref()
                .is_some_and(|c| same_name(c, charset))
            {
                differ("character set");
            }
        }
        if let Some(collation) = &wanted.collation {
            if !found
                .collation
                .as_deref()
                .is_some_and(|c| same_name(c, collation))
            {
                differ("collation");
            }
        }
    }
}
