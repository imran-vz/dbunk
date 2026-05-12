//! CLI tab — REPL command execution.
//!
//! Phase 1.3 surface: `run_command` issues an arbitrary Redis command
//! (caller pre-tokenised) and returns the serialised result.
//! Destructive-command guard from ADR-0009 is enforced here: any
//! command on the hard or soft list is rejected unless
//! `confirmed: true` is set on the payload. Pub/Sub commands are
//! redirected to the Pub/Sub tab.
//!
//! MULTI/EXEC tracking is deferred to a follow-up — pipelines fan
//! out by connection_id, not session_id, today; tracking transaction
//! state per CLI tab needs a session abstraction we don't yet have.

use serde::{Deserialize, Serialize};

use crate::redis::connection;
use crate::redis::destructive_commands::{DESTRUCTIVE_HARD, DESTRUCTIVE_SOFT};
use crate::redis::value::{self, SerializedValue};
use crate::RedisStoredConnection;

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

    let mut conn = connection::manager_for(connection).await?;
    let mut cmd = redis::cmd(&payload.tokens[0]);
    for arg in payload.tokens.iter().skip(1) {
        cmd.arg(arg);
    }

    let start = std::time::Instant::now();
    let result = cmd
        .query_async::<redis::Value>(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    let runtime_ms = start.elapsed().as_millis() as u64;

    Ok(RunCommandResult::Ok {
        value: value::serialize(result),
        runtime_ms,
    })
}
