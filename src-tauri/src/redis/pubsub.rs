//! Pub/Sub tab — pattern-subscription with polling-style drain.
//!
//! Phase 1.3 surface: one session = one dedicated connection. The
//! caller starts a session with one or more patterns, then polls
//! `drain` to retrieve buffered messages. Close drops the
//! connection. Backpressure: per-session token bucket throttles
//! forwarding at 1000 msg/sec; excess messages are dropped with a
//! sampling indicator.
//!
//! Tier 2 adds `discover_channels` — a `PUBSUB CHANNELS`-backed lookup
//! that returns the currently-active channel list. Channels Redis
//! knows about only become visible once at least one subscriber is
//! listening, so the discovery output is "what subscribers see right
//! now" rather than "what publishers might use".

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::redis::connection;
use crate::redis::value::{self, SerializedValue};
use crate::RedisStoredConnection;

/// Tauri event channel name. Frontend listens for these via
/// `listen("pubsub-message", ...)` — replaces the older 750ms polling
/// drain (`drain` is kept around as a deprecated fallback / catch-up).
pub const PUBSUB_EVENT: &str = "pubsub-message";

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

/// Envelope emitted on the `pubsub-message` Tauri channel. Frontend
/// filters by `session_id` so multiple Pub/Sub tabs can share one
/// global listener.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PubsubEvent {
    session_id: String,
    message: DrainedMessage,
}

pub async fn start_session(
    app: &AppHandle,
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
    let app_for_task = app.clone();

    let handle = tokio::spawn(async move {
        let mut stream = pubsub.on_message();
        while let Some(msg) = futures_next(&mut stream).await {
            let channel = msg.get_channel_name().to_string();
            let pattern = msg
                .get_pattern::<String>()
                .ok()
                .unwrap_or_else(|| channel.clone());
            let payload_value: redis::Value = msg.get_payload().unwrap_or(redis::Value::Nil);
            let message = DrainedMessage {
                received_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0),
                channel,
                pattern_matched: pattern,
                payload: value::serialize(payload_value),
            };
            // Buffer first so a frontend that hasn't attached its
            // listener yet (a few ms window during session-start)
            // can catch up via the `drain` endpoint. Then emit so a
            // listener that's already attached sees the message
            // without polling.
            push_message(&session_id_for_task, message.clone());
            if let Err(err) = app_for_task.emit(
                PUBSUB_EVENT,
                PubsubEvent {
                    session_id: session_id_for_task.clone(),
                    message,
                },
            ) {
                log::warn!("pubsub emit failed: {}", err);
            }
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
            channels.sort_by_key(|channel| std::cmp::Reverse(channel.count));
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

// ---------------------------------------------------------------------------
// Channel discovery (Tier 2)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverChannelsPayload {
    pub connection_id: String,
    #[serde(default)]
    pub pattern: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredChannel {
    pub channel: String,
    pub subscribers: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverChannelsResult {
    pub channels: Vec<DiscoveredChannel>,
}

pub async fn discover_channels(
    connection: &RedisStoredConnection,
    payload: &DiscoverChannelsPayload,
) -> Result<DiscoverChannelsResult, String> {
    let mut conn = connection::manager_for(connection).await?;
    let pattern = payload.pattern.as_deref().unwrap_or("*");
    let channels: Vec<String> = redis::cmd("PUBSUB")
        .arg("CHANNELS")
        .arg(pattern)
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    if channels.is_empty() {
        return Ok(DiscoverChannelsResult { channels: vec![] });
    }
    // One round trip for the subscriber counts; PUBSUB NUMSUB returns
    // an alternating [channel, count, channel, count, …] array.
    let mut numsub = redis::cmd("PUBSUB");
    numsub.arg("NUMSUB");
    for channel in &channels {
        numsub.arg(channel);
    }
    let pairs: redis::Value = numsub
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    let mut subscribers: HashMap<String, u64> = HashMap::new();
    if let redis::Value::Array(values) = pairs {
        let mut iter = values.into_iter();
        while let (Some(channel), Some(count)) = (iter.next(), iter.next()) {
            let channel = match channel {
                redis::Value::BulkString(bytes) => String::from_utf8(bytes).unwrap_or_default(),
                redis::Value::SimpleString(s) => s,
                _ => continue,
            };
            let count = match count {
                redis::Value::Int(n) => u64::try_from(n).unwrap_or(0),
                _ => 0,
            };
            subscribers.insert(channel, count);
        }
    }
    let mut result: Vec<DiscoveredChannel> = channels
        .into_iter()
        .map(|channel| {
            let subscribers = subscribers.remove(&channel).unwrap_or(0);
            DiscoveredChannel {
                channel,
                subscribers,
            }
        })
        .collect();
    result.sort_by_key(|channel| std::cmp::Reverse(channel.subscribers));
    Ok(DiscoverChannelsResult { channels: result })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishPayload {
    pub connection_id: String,
    pub channel: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishResult {
    /// Number of clients that received the message (from `PUBLISH`'s
    /// integer reply). On a fresh server with no subscribers this is
    /// 0, which is informational rather than an error.
    pub receivers: u64,
}

/// Issue `PUBLISH channel message` against the connection. Uses the
/// shared multiplexed manager — Pub/Sub-subscriber sessions are
/// separate (one connection per subscriber), so publishing doesn't
/// interact with the subscribe-side ring buffer.
pub async fn publish(
    connection: &RedisStoredConnection,
    payload: &PublishPayload,
) -> Result<PublishResult, String> {
    if payload.channel.trim().is_empty() {
        return Err("Channel is required".to_string());
    }
    let mut conn = connection::manager_for(connection).await?;
    let receivers: i64 = redis::cmd("PUBLISH")
        .arg(&payload.channel)
        .arg(&payload.message)
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    Ok(PublishResult {
        receivers: u64::try_from(receivers).unwrap_or(0),
    })
}
