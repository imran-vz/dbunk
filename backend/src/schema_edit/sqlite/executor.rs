//! Runs a reviewed plan on the session connection, following SQLite's
//! documented procedure for schema changes:
//!
//! 1. Refuse while the user has a transaction open (probe with `BEGIN`).
//! 2. Turn `foreign_keys` off (it is a no-op inside a transaction) and, for
//!    a rebuild, `legacy_alter_table` on, so renaming the new table does not
//!    trip over views that name the old one. Both are restored afterwards,
//!    whatever the outcome.
//! 3. `BEGIN IMMEDIATE`, then recheck the drift token.
//! 4. Run the statements in order.
//! 5. Verify before COMMIT: `foreign_key_check` when keys were enforced (or,
//!    when they were not, for the keys the draft adds or changes),
//!    `quick_check` on the table, and a re-capture compared with the draft.
//! 6. COMMIT. Any failure rolls back; the table is unchanged.
//!
//! Cancellation arrives through the session's progress handler: the running
//! statement fails with `SQLITE_INTERRUPT`, and the rollback reports
//! `Cancelled`.
use super::capture::{capture_table, drift_token};
use super::planner::{expected_table, is_rebuild};
use super::verify::compare_tables;
use super::{parse_version, quote};
use crate::schema_edit::{
    Editability, ForeignKeyModel, Origin, SchemaBaseline, SchemaDraft, SchemaEditFailure,
    SchemaEditOutcome, SchemaEditPreview, TableModel,
};
use futures_util::future::BoxFuture;
use sqlx::{Row, SqliteConnection};
use std::time::Instant;

/// `SQLITE_INTERRUPT`.
const INTERRUPTED: &str = "9";

fn failure(error: sqlx::Error) -> SchemaEditFailure {
    match error {
        sqlx::Error::Database(error) => {
            let code = error.code().map(|code| code.into_owned());
            match code.as_deref() {
                Some(INTERRUPTED) => SchemaEditFailure::Cancelled,
                // SQLITE_BUSY, SQLITE_LOCKED and their extended codes.
                Some(code) if matches!(primary_code(code), Some(5 | 6)) => {
                    SchemaEditFailure::LockTimeout
                }
                _ => SchemaEditFailure::Database {
                    code,
                    message: error.message().to_owned(),
                },
            }
        }
        sqlx::Error::Io(_) | sqlx::Error::WorkerCrashed => SchemaEditFailure::Connection,
        other => SchemaEditFailure::Database {
            code: None,
            message: other.to_string(),
        },
    }
}

/// The primary result code of an extended SQLite code.
fn primary_code(code: &str) -> Option<i64> {
    code.parse::<i64>().ok().map(|code| code & 0xff)
}

fn database_message(error: &sqlx::Error) -> String {
    match error {
        sqlx::Error::Database(error) => error.message().to_owned(),
        other => other.to_string(),
    }
}

/// Boxed: `raw_sql` futures are only `Send` for one concrete lifetime, which
/// the session worker's spawned future cannot express otherwise.
fn run<'a>(
    connection: &'a mut SqliteConnection,
    sql: &'a str,
) -> BoxFuture<'a, Result<(), sqlx::Error>> {
    Box::pin(async move {
        sqlx::Executor::execute(&mut *connection, sql)
            .await
            .map(|_| ())
    })
}

async fn pragma_flag(connection: &mut SqliteConnection, name: &str) -> Result<bool, sqlx::Error> {
    let value = sqlx::query_scalar::<_, i64>(&format!("PRAGMA {name}"))
        .persistent(false)
        .fetch_one(&mut *connection)
        .await?;
    Ok(value != 0)
}

/// Rolls back our transaction. A transaction SQLite already rolled back
/// (an interrupted write) counts as rolled back; any other rollback failure
/// leaves the outcome unknown.
async fn roll_back(
    connection: &mut SqliteConnection,
    reason: SchemaEditFailure,
) -> SchemaEditOutcome {
    match run(connection, "ROLLBACK").await {
        Ok(()) => SchemaEditOutcome::RolledBack { reason },
        Err(error) if database_message(&error).contains("no transaction is active") => {
            SchemaEditOutcome::RolledBack { reason }
        }
        Err(_) => SchemaEditOutcome::OutcomeUnknown {
            committed: 0,
            reason,
        },
    }
}

/// Connection settings changed for the apply, restored afterwards.
struct Settings {
    restore_foreign_keys: bool,
    restore_legacy_alter_table: bool,
}

impl Settings {
    async fn restore(&self, connection: &mut SqliteConnection) {
        // Outside a transaction these always take effect.
        if self.restore_legacy_alter_table {
            let _ = run(connection, "PRAGMA legacy_alter_table = OFF").await;
        }
        if self.restore_foreign_keys {
            let _ = run(connection, "PRAGMA foreign_keys = ON").await;
        }
    }
}

pub(super) async fn execute(
    connection: &mut SqliteConnection,
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    preview: &SchemaEditPreview,
) -> SchemaEditOutcome {
    let started = Instant::now();
    if let Editability::ReadOnly { reasons } = &baseline.editable {
        return SchemaEditOutcome::NotDispatched {
            reason: SchemaEditFailure::Unsupported {
                reason: reasons
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "This table is read-only".into()),
            },
        };
    }
    if preview.statements.is_empty() {
        return SchemaEditOutcome::NotDispatched {
            reason: SchemaEditFailure::Unsupported {
                reason: "Nothing to change".into(),
            },
        };
    }

    // 1. A user transaction makes BEGIN fail; leave it untouched.
    match run(connection, "BEGIN").await {
        Ok(()) => {}
        Err(error) if database_message(&error).contains("within a transaction") => {
            return SchemaEditOutcome::NotDispatched {
                reason: SchemaEditFailure::OpenTransaction,
            }
        }
        Err(error) => {
            return SchemaEditOutcome::NotDispatched {
                reason: failure(error),
            }
        }
    }
    if let Err(error) = run(connection, "ROLLBACK").await {
        return SchemaEditOutcome::OutcomeUnknown {
            committed: 0,
            reason: failure(error),
        };
    }

    // 2. Connection settings.
    let mut settings = Settings {
        restore_foreign_keys: false,
        restore_legacy_alter_table: false,
    };
    let foreign_keys = match pragma_flag(connection, "foreign_keys").await {
        Ok(on) => on,
        Err(error) => {
            return SchemaEditOutcome::NotDispatched {
                reason: failure(error),
            }
        }
    };
    if foreign_keys {
        if let Err(error) = run(connection, "PRAGMA foreign_keys = OFF").await {
            return SchemaEditOutcome::NotDispatched {
                reason: failure(error),
            };
        }
        settings.restore_foreign_keys = true;
    }
    if is_rebuild(&preview.notes) {
        match pragma_flag(connection, "legacy_alter_table").await {
            Ok(true) => {}
            Ok(false) => match run(connection, "PRAGMA legacy_alter_table = ON").await {
                Ok(()) => settings.restore_legacy_alter_table = true,
                Err(error) => {
                    settings.restore(connection).await;
                    return SchemaEditOutcome::NotDispatched {
                        reason: failure(error),
                    };
                }
            },
            Err(error) => {
                settings.restore(connection).await;
                return SchemaEditOutcome::NotDispatched {
                    reason: failure(error),
                };
            }
        }
    }

    let outcome = transaction(connection, baseline, draft, preview, foreign_keys, started).await;
    settings.restore(connection).await;
    outcome
}

/// Steps 3 to 6, with `foreign_keys` already off.
async fn transaction(
    connection: &mut SqliteConnection,
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    preview: &SchemaEditPreview,
    foreign_keys: bool,
    started: Instant,
) -> SchemaEditOutcome {
    let schema = baseline.table.schema.as_str();
    if let Err(error) = run(connection, "BEGIN IMMEDIATE").await {
        return SchemaEditOutcome::NotDispatched {
            reason: failure(error),
        };
    }
    match drift_token(connection, schema).await {
        Ok(token) if token == baseline.drift => {}
        Ok(_) => {
            // Nothing of the plan ran; the probe transaction is empty.
            return match roll_back(connection, SchemaEditFailure::BaselineChanged).await {
                SchemaEditOutcome::RolledBack { reason } => {
                    SchemaEditOutcome::NotDispatched { reason }
                }
                other => other,
            };
        }
        Err(error) => {
            return match roll_back(connection, failure(error)).await {
                SchemaEditOutcome::RolledBack { reason } => {
                    SchemaEditOutcome::NotDispatched { reason }
                }
                other => other,
            };
        }
    }

    for statement in &preview.statements {
        if let Err(error) = run(connection, &statement.sql).await {
            return roll_back(connection, failure(error)).await;
        }
    }

    if let Err(reason) = verify(connection, baseline, draft, foreign_keys).await {
        return roll_back(connection, reason).await;
    }

    if let Err(error) = run(connection, "COMMIT").await {
        return roll_back(connection, failure(error)).await;
    }
    SchemaEditOutcome::Applied {
        runtime_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    }
}

fn verification(detail: impl Into<String>) -> SchemaEditFailure {
    SchemaEditFailure::VerificationFailed {
        detail: detail.into(),
    }
}

/// A verification read that failed: an interrupt is a cancellation.
fn read_failure(error: sqlx::Error) -> SchemaEditFailure {
    match failure(error) {
        SchemaEditFailure::Cancelled => SchemaEditFailure::Cancelled,
        SchemaEditFailure::Database { message, .. } => verification(message),
        other => other,
    }
}

/// A foreign key as `foreign_key_list` names it: referenced table, local
/// columns, referenced columns (empty for the primary key), case-folded.
type KeySignature = (String, Vec<String>, Vec<String>);

fn signature(table: &str, from: &[String], to: &[String]) -> KeySignature {
    let fold = |names: &[String]| names.iter().map(|n| n.to_ascii_lowercase()).collect();
    (table.to_ascii_lowercase(), fold(from), fold(to))
}

/// Signatures of the keys the draft carries over unchanged, or `None` when
/// it keeps every key that way. A key is unchanged when its columns and
/// target are as captured and none of its columns changed type (a new
/// affinity can convert the copied values).
fn kept_keys(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    expected: &TableModel,
) -> Option<Vec<KeySignature>> {
    let old = &baseline.table;
    let unchanged = |key: &ForeignKeyModel| {
        let drafted = draft.table.foreign_key(key.origin);
        let before = old.foreign_key(key.origin);
        match (drafted, before) {
            (Some(drafted), Some(before)) => {
                drafted.columns == before.columns
                    && drafted.referenced_table == before.referenced_table
                    && drafted.referenced_columns == before.referenced_columns
                    && drafted.columns.iter().all(|origin| {
                        match (draft.table.column(*origin), old.column(*origin)) {
                            (Some(now), Some(then)) => now.data_type == then.data_type,
                            _ => false,
                        }
                    })
            }
            _ => false,
        }
    };
    if expected.foreign_keys.iter().all(unchanged) {
        return None;
    }
    let names = |origins: &[Origin]| -> Vec<String> {
        origins
            .iter()
            .filter_map(|origin| expected.column(*origin).map(|c| c.name.clone()))
            .collect()
    };
    Some(
        expected
            .foreign_keys
            .iter()
            .filter(|key| unchanged(key))
            .map(|key| {
                signature(
                    &key.referenced_table,
                    &names(&key.columns),
                    &key.referenced_columns,
                )
            })
            .collect(),
    )
}

/// `foreign_key_check` on the rebuilt table, failing on a violation of any
/// key not in `kept`.
async fn new_key_violations(
    connection: &mut SqliteConnection,
    schema: &str,
    table: &str,
    kept: &[KeySignature],
) -> Result<(), SchemaEditFailure> {
    type KeyRow = (i64, String, String, Option<String>);
    let rows = sqlx::query_as::<_, KeyRow>(
        "SELECT id, \"table\", \"from\", \"to\" \
         FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq",
    )
    .persistent(false)
    .bind(table)
    .bind(schema)
    .fetch_all(&mut *connection)
    .await
    .map_err(read_failure)?;
    let mut keys: Vec<(i64, String, Vec<String>, Vec<String>)> = Vec::new();
    for (id, referenced, from, to) in rows {
        match keys.last_mut() {
            Some((last, _, froms, tos)) if *last == id => {
                froms.push(from);
                tos.extend(to);
            }
            _ => keys.push((id, referenced, vec![from], to.into_iter().collect())),
        }
    }
    let violated = sqlx::query(&format!(
        "PRAGMA {}.foreign_key_check({})",
        quote(schema),
        quote(table)
    ))
    .persistent(false)
    .fetch_all(&mut *connection)
    .await
    .map_err(read_failure)?;
    for row in violated {
        let fkid: i64 = row.try_get(3).map_err(read_failure)?;
        let known =
            keys.iter()
                .find(|(id, ..)| *id == fkid)
                .is_some_and(|(_, referenced, from, to)| {
                    kept.contains(&signature(referenced, from, to))
                });
        if !known {
            return Err(SchemaEditFailure::ForeignKeyViolation);
        }
    }
    Ok(())
}

/// Step 5, inside the transaction.
async fn verify(
    connection: &mut SqliteConnection,
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    foreign_keys: bool,
) -> Result<(), SchemaEditFailure> {
    let schema = baseline.table.schema.as_str();
    let table = draft.table.name.as_str();
    let expected = expected_table(baseline, draft);
    if foreign_keys {
        // This table's own keys, and every table whose keys reference it.
        let mut checked: Vec<&str> = vec![table];
        for reference in &baseline.inbound {
            if !checked
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&reference.table))
            {
                checked.push(&reference.table);
            }
        }
        for name in checked {
            let violation = sqlx::query(&format!(
                "PRAGMA {}.foreign_key_check({})",
                quote(schema),
                quote(name)
            ))
            .persistent(false)
            .fetch_optional(&mut *connection)
            .await
            .map_err(read_failure)?;
            if violation.is_some() {
                return Err(SchemaEditFailure::ForeignKeyViolation);
            }
        }
    } else if let Some(kept) = kept_keys(baseline, draft, &expected) {
        // Unenforced keys are the user's choice, but a key this edit adds or
        // changes must hold for the rows it copied. Keys carried over as they
        // were keep whatever rows they already had.
        new_key_violations(connection, schema, table, &kept).await?;
    }
    // `quick_check(table)` exists from SQLite 3.33.
    if parse_version(&baseline.server.version) >= (3, 33, 0) {
        let rows = sqlx::query(&format!(
            "PRAGMA {}.quick_check({})",
            quote(schema),
            quote(table)
        ))
        .persistent(false)
        .fetch_all(&mut *connection)
        .await
        .map_err(read_failure)?;
        let first: Option<String> = match rows.first() {
            Some(row) => Some(row.try_get(0).map_err(read_failure)?),
            None => None,
        };
        if first.as_deref() != Some("ok") {
            return Err(verification(format!(
                "quick_check reported {}",
                first.unwrap_or_else(|| "nothing".into())
            )));
        }
    }
    let recaptured = capture_table(connection, schema, table, &baseline.server.version, false)
        .await
        .map_err(|message| {
            if message.contains("interrupted") {
                SchemaEditFailure::Cancelled
            } else {
                verification(message)
            }
        })?;
    if let Editability::ReadOnly { reasons } = &recaptured.editable {
        return Err(verification(format!(
            "the new definition cannot be read back ({})",
            reasons.join("; ")
        )));
    }
    compare_tables(&expected, &recaptured.table).map_err(verification)?;
    // A rebuild must leave no trace of its temporary table.
    let leftover = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT count(*) FROM {}.sqlite_master WHERE name = ?1",
        quote(schema)
    ))
    .persistent(false)
    .bind(format!(
        "{}{}",
        super::planner::REBUILD_PREFIX,
        baseline.table.name
    ))
    .fetch_one(&mut *connection)
    .await
    .map_err(read_failure)?;
    if leftover != 0 {
        return Err(verification(
            "the rebuild's temporary table is still present",
        ));
    }
    Ok(())
}
