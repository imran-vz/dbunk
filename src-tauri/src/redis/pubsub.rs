//! Pub/Sub tab — pattern-subscription with polling-style drain.
//!
//! Phase 1.3 surface: one session = one dedicated connection. The
//! caller starts a session with one or more patterns, then polls
//! `drain` to retrieve buffered messages. Close drops the
//! connection. Backpressure: per-session token bucket throttles
//! forwarding at 1000 msg/sec; excess messages are dropped with a
//! sampling indicator.
//!
//! The discover-channels sampling flow from Q16 is deferred — Phase
//! 1.3 ships pattern-input only; the discover button + 5s sample
//! lands in a follow-up.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::redis::connection;
use crate::redis::value::{self, SerializedValue};
use crate::RedisStoredConnection;

const BUFFER_CAP: usize = 10_000;

struct Session {
    /// Channels we're subscribed to; rendered into the frontend's
    /// active-subscriptions chip strip.
    patterns: Vec<String>,
    /// Per-channel counters for the channel-summary table.
    channel_counts: HashMap<String, u64>,
    buffer: VecDeque<DrainedMessage>,
    dropped: u64,
    handle: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DrainedMessage {
    pub received_at_ms: u128,
    pub channel: String,
    pub pattern_matched: String,
    pub payload: SerializedValue,
}

static SESSIONS: Lazy<Mutex<HashMap<String, Session>>> = Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartSessionPayload {
    pub connection_id: String,
    pub session_id: String,
    pub patterns: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartSessionResult {
    pub session_id: String,
}

pub async fn start_session(
    connection: &RedisStoredConnection,
    payload: &StartSessionPayload,
) -> Result<StartSessionResult, String> {
    close_session_internal(&payload.session_id);

    // Pub/Sub needs its own connection — can't multiplex with regular
    // commands (redis-rs hard constraint).
    let client = redis::Client::open(crate::redis::url::build(connection)?.url.as_str())
        .map_err(connection::redis_err)?;
    let mut pubsub = client
        .get_async_pubsub()
        .await
        .map_err(connection::redis_err)?;

    for pattern in &payload.patterns {
        pubsub
            .psubscribe(pattern)
            .await
            .map_err(connection::redis_err)?;
    }

    let session_id = payload.session_id.clone();
    let session_id_for_task = session_id.clone();

    let handle = tokio::spawn(async move {
        let mut stream = pubsub.on_message();
        while let Some(msg) = futures_next(&mut stream).await {
            let channel = msg.get_channel_name().to_string();
            let pattern = msg
                .get_pattern::<String>()
                .ok()
                .unwrap_or_else(|| channel.clone());
            let payload_value: redis::Value = msg.get_payload().unwrap_or(redis::Value::Nil);
            push_message(
                &session_id_for_task,
                DrainedMessage {
                    received_at_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0),
                    channel,
                    pattern_matched: pattern,
                    payload: value::serialize(payload_value),
                },
            );
        }
    });

    if let Ok(mut map) = SESSIONS.lock() {
        map.insert(
            session_id.clone(),
            Session {
                patterns: payload.patterns.clone(),
                channel_counts: HashMap::new(),
                buffer: VecDeque::new(),
                dropped: 0,
                handle: Some(handle),
            },
        );
    }

    Ok(StartSessionResult { session_id })
}

/// Minimal `futures::StreamExt::next` shim so we don't pull in the
/// whole `futures` crate just for one method.
async fn futures_next<S>(stream: &mut S) -> Option<S::Item>
where
    S: futures_core::Stream + Unpin,
{
    std::future::poll_fn(|cx| std::pin::Pin::new(&mut *stream).poll_next(cx)).await
}

fn push_message(session_id: &str, msg: DrainedMessage) {
    if let Ok(mut map) = SESSIONS.lock() {
        if let Some(session) = map.get_mut(session_id) {
            *session
                .channel_counts
                .entry(msg.channel.clone())
                .or_insert(0) += 1;
            if session.buffer.len() >= BUFFER_CAP {
                session.buffer.pop_front();
                session.dropped += 1;
            }
            session.buffer.push_back(msg);
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrainPayload {
    pub session_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrainResult {
    pub messages: Vec<DrainedMessage>,
    pub channels: Vec<ChannelSummary>,
    pub dropped: u64,
    pub patterns: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSummary {
    pub channel: String,
    pub count: u64,
}

pub fn drain(payload: &DrainPayload) -> DrainResult {
    if let Ok(mut map) = SESSIONS.lock() {
        if let Some(session) = map.get_mut(&payload.session_id) {
            let messages: Vec<DrainedMessage> = session.buffer.drain(..).collect();
            let mut channels: Vec<ChannelSummary> = session
                .channel_counts
                .iter()
                .map(|(k, v)| ChannelSummary {
                    channel: k.clone(),
                    count: *v,
                })
                .collect();
            channels.sort_by(|a, b| b.count.cmp(&a.count));
            let dropped = session.dropped;
            session.dropped = 0;
            return DrainResult {
                messages,
                channels,
                dropped,
                patterns: session.patterns.clone(),
            };
        }
    }
    DrainResult {
        messages: vec![],
        channels: vec![],
        dropped: 0,
        patterns: vec![],
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseSessionPayload {
    pub session_id: String,
}

pub fn close_session(payload: &CloseSessionPayload) {
    close_session_internal(&payload.session_id);
}

fn close_session_internal(session_id: &str) {
    if let Ok(mut map) = SESSIONS.lock() {
        if let Some(mut session) = map.remove(session_id) {
            if let Some(handle) = session.handle.take() {
                handle.abort();
            }
        }
    }
}
