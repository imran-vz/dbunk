//! CLI tab — REPL command execution.
//!
//! `run_command` issues an arbitrary Redis command (caller
//! pre-tokenised) and returns the serialised result.
//! Destructive-command guard from ADR-0009 is enforced here: any
//! command on the hard or soft list is rejected unless
//! `confirmed: true` is set on the payload. Pub/Sub commands are
//! redirected to the Pub/Sub tab.
//!
//! ## MULTI/EXEC isolation
//!
//! `MULTI ... EXEC` only works on a single physical connection.
//! When the caller supplies a `session_id`, commands route to that
//! session's dedicated `redis::aio::Connection` (not the shared
//! multiplexed manager). The dedicated connection is created lazily
//! on first use and held until `close_session` (called from
//! `CliTab` on unmount) or until the connection record is dropped.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;

use crate::redis::connection;
use crate::redis::destructive_commands::{DESTRUCTIVE_HARD, DESTRUCTIVE_SOFT};
use crate::redis::value::{self, SerializedValue};
use crate::RedisStoredConnection;

/// Per-session dedicated connection cache. Keyed by `session_id`
/// (typically the CLI tab's ID). `redis::aio::MultiplexedConnection`
/// is fine for non-transactional commands but MULTI/EXEC needs the
/// SAME physical request stream — we hand each session its own
/// `MultiplexedConnection` clone, which internally maps to a single
/// TCP connection per `redis::Client` instance.
///
/// A real `redis::aio::Connection` would be stricter but isn't
/// `Clone`; the `MultiplexedConnection` clone-per-session approach
/// keeps the lock scope tiny without paying the
/// build-a-new-connection cost on every command. Session ↔ TCP
/// connection mapping is 1:1.
type CliConn = AsyncMutex<redis::aio::MultiplexedConnection>;

static CLI_SESSIONS: Lazy<Mutex<HashMap<String, Arc<CliConn>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn cached_session(session_id: &str) -> Option<Arc<CliConn>> {
    CLI_SESSIONS.lock().ok()?.get(session_id).cloned()
}

fn insert_session(session_id: String, conn: Arc<CliConn>) {
    if let Ok(mut cache) = CLI_SESSIONS.lock() {
        cache.entry(session_id).or_insert(conn);
    }
}

async fn session_connection_for(
    connection: &RedisStoredConnection,
    session_id: &str,
) -> Result<Arc<CliConn>, String> {
    if let Some(existing) = cached_session(session_id) {
        return Ok(existing);
    }
    let client = redis::Client::open(crate::redis::url::build(connection)?.url.as_str())
        .map_err(connection::redis_err)?;
    let conn = client
        .get_multiplexed_async_connection()
        .await
        .map_err(connection::redis_err)?;
    let arc = Arc::new(AsyncMutex::new(conn));
    insert_session(session_id.to_string(), arc.clone());
    Ok(arc)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseSessionPayload {
    pub session_id: String,
}

/// Drop the dedicated connection for a CLI session. Called when the
/// CLI tab unmounts. Best-effort.
pub fn close_session(payload: &CloseSessionPayload) {
    if let Ok(mut cache) = CLI_SESSIONS.lock() {
        cache.remove(&payload.session_id);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCommandPayload {
    pub connection_id: String,
    /// Pre-tokenised command. First token is the command name; rest
    /// are arguments. Empty tokens treated as empty args.
    pub tokens: Vec<String>,
    /// User confirmed the destructive-command modal. Without it,
    /// destructive commands are rejected with `needs-confirmation`.
    #[serde(default)]
    pub confirmed: bool,
    /// Stable per-CLI-tab ID. When present, all commands for this
    /// session route through one dedicated `MultiplexedConnection`
    /// — necessary for `MULTI ... EXEC` correctness because the
    /// shared manager can interleave other commands between MULTI
    /// and EXEC. Omitting it falls back to the shared manager.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RunCommandResult {
    Ok {
        value: SerializedValue,
        runtime_ms: u64,
    },
    NeedsConfirmation {
        command: String,
        severity: String,
    },
    Rejected {
        reason: String,
    },
}

pub async fn run_command(
    connection: &RedisStoredConnection,
    payload: &RunCommandPayload,
) -> Result<RunCommandResult, String> {
    if payload.tokens.is_empty() {
        return Ok(RunCommandResult::Rejected {
            reason: "Empty command".into(),
        });
    }

    let head = payload.tokens[0].to_uppercase();
    let two = if payload.tokens.len() >= 2 {
        format!("{} {}", head, payload.tokens[1].to_uppercase())
    } else {
        head.clone()
    };

    // Pub/Sub commands must run on a dedicated connection (see
    // `pubsub.rs`); rejecting them here keeps the multiplexed
    // connection healthy.
    if matches!(
        head.as_str(),
        "SUBSCRIBE" | "PSUBSCRIBE" | "UNSUBSCRIBE" | "PUNSUBSCRIBE" | "MONITOR"
    ) {
        return Ok(RunCommandResult::Rejected {
            reason: format!(
                "{head} is not available in the CLI. Use the Pub/Sub tab for subscriptions.",
            ),
        });
    }

    let destructive = DESTRUCTIVE_HARD
        .iter()
        .find(|entry| head == **entry || two == **entry)
        .map(|entry| (*entry, "hard"))
        .or_else(|| {
            DESTRUCTIVE_SOFT
                .iter()
                .find(|entry| head == **entry || two == **entry)
                .map(|entry| (*entry, "soft"))
        });

    if let Some((matched, severity)) = destructive {
        if !payload.confirmed {
            return Ok(RunCommandResult::NeedsConfirmation {
                command: matched.to_string(),
                severity: severity.to_string(),
            });
        }
    }

    let mut cmd = redis::cmd(&payload.tokens[0]);
    for arg in payload.tokens.iter().skip(1) {
        cmd.arg(arg);
    }

    let start = std::time::Instant::now();
    let result = match &payload.session_id {
        Some(session_id) => {
            let session = session_connection_for(connection, session_id).await?;
            let mut guard = session.lock().await;
            cmd.query_async::<redis::Value>(&mut *guard)
                .await
                .map_err(connection::redis_err)?
        }
        None => {
            let mut conn = connection::manager_for(connection).await?;
            cmd.query_async::<redis::Value>(&mut conn)
                .await
                .map_err(connection::redis_err)?
        }
    };
    let runtime_ms = start.elapsed().as_millis() as u64;

    Ok(RunCommandResult::Ok {
        value: value::serialize(result),
        runtime_ms,
    })
}
