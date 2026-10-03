//! Keyspace browsing — SCAN-driven enumeration of Redis keys.
//!
//! The frontend's `KeyspaceBrowser` calls into this module for both
//! tree-mode expansion (one `SCAN MATCH prefix:*` per branch) and
//! flat search-mode (`SCAN MATCH *foo*`). Cursor state is exposed to
//! the caller so pagination resumes across multiple invocations.
//!
//! ## Cancellable sessions
//!
//! Long-running SCANs (especially the per-key `TYPE` round trip on
//! servers without server-side `TYPE` push-down) can stall the
//! sidebar. To make those genuinely cancellable, the browser opens a
//! "scan session" on mount: `open_session` creates a dedicated
//! `MultiplexedConnection` and records its `CLIENT ID`. Subsequent
//! `scan_keys` calls route through the session's connection. A
//! `cancel_session` call issues `CLIENT KILL ID <client_id>` via the
//! SHARED manager — that severs the dedicated connection without
//! disrupting other key tabs that share the manager.
//!
//! Sessions are dropped via `close_session` on tab unmount. Cancel
//! also drops the entry so a re-open creates a fresh connection.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;

use crate::redis::connection;
use crate::RedisStoredConnection;

type ScanConn = AsyncMutex<redis::aio::MultiplexedConnection>;

struct ScanSession {
    /// Server-side client ID — the kill target.
    client_id: u64,
    /// The dedicated multiplexed connection. Held behind an
    /// `AsyncMutex` so consecutive `scan_keys` calls serialise.
    conn: Arc<ScanConn>,
}

static SCAN_SESSIONS: Lazy<Mutex<HashMap<String, ScanSession>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn cached_session_conn(session_id: &str) -> Option<Arc<ScanConn>> {
    SCAN_SESSIONS
        .lock()
        .ok()?
        .get(session_id)
        .map(|s| s.conn.clone())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenScanSessionPayload {
    pub connection_id: String,
    pub session_id: String,
    /// Optional override of the DB number the SCAN session targets.
    /// When `None`, the session connects to the connection record's
    /// default `db_number`. Lets the keyspace browser switch DBs
    /// without reconnecting the shared manager (which would affect
    /// every other tab on this connection).
    #[serde(default)]
    pub db_number: Option<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenScanSessionResult {
    pub client_id: u64,
}

/// Open (or reset) a dedicated SCAN connection for this session. The
/// connection is sourced from a fresh `redis::Client::open` rather
/// than the shared manager so it has its own server-side client ID
/// and can be killed independently.
pub async fn open_session(
    connection: &RedisStoredConnection,
    payload: &OpenScanSessionPayload,
) -> Result<OpenScanSessionResult, String> {
    close_session_internal(&payload.session_id);
    let mut stored = connection.clone();
    if let Some(db) = payload.db_number {
        if db > 15 {
            return Err(format!("Redis DB number must be 0–15 (got {db})"));
        }
        stored.db_number = db;
    }
    let client = redis::Client::open(crate::redis::url::build(&stored)?.url.as_str())
        .map_err(connection::redis_err)?;
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .map_err(connection::redis_err)?;
    let client_id: u64 = redis::cmd("CLIENT")
        .arg("ID")
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    if let Ok(mut map) = SCAN_SESSIONS.lock() {
        map.insert(
            payload.session_id.clone(),
            ScanSession {
                client_id,
                conn: Arc::new(AsyncMutex::new(conn)),
            },
        );
    }
    Ok(OpenScanSessionResult { client_id })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelScanSessionPayload {
    pub connection_id: String,
    pub session_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelScanSessionResult {
    /// Number of clients reported killed by `CLIENT KILL ID`. Zero
    /// when the session was never opened or already torn down.
    pub killed: u64,
}

/// Cancel a scan session: issue `CLIENT KILL ID <id>` via the shared
/// manager (which is a separate connection, so the kill propagates
/// server-side), then drop the local session entry so a re-open
/// builds a fresh connection.
pub async fn cancel_session(
    connection: &RedisStoredConnection,
    payload: &CancelScanSessionPayload,
) -> Result<CancelScanSessionResult, String> {
    let client_id = {
        let map = SCAN_SESSIONS
            .lock()
            .map_err(|_| "scan session lock poisoned".to_string())?;
        map.get(&payload.session_id).map(|s| s.client_id)
    };
    let Some(client_id) = client_id else {
        return Ok(CancelScanSessionResult { killed: 0 });
    };
    let mut shared = connection::manager_for(connection).await?;
    let killed: i64 = redis::cmd("CLIENT")
        .arg("KILL")
        .arg("ID")
        .arg(client_id)
        .query_async(&mut shared)
        .await
        .map_err(connection::redis_err)?;
    close_session_internal(&payload.session_id);
    Ok(CancelScanSessionResult {
        killed: u64::try_from(killed).unwrap_or(0),
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseScanSessionPayload {
    pub session_id: String,
}

/// Drop the dedicated connection for this session without killing
/// the server-side client. Called on unmount.
pub fn close_session(payload: &CloseScanSessionPayload) {
    close_session_internal(&payload.session_id);
}

fn close_session_internal(session_id: &str) {
    if let Ok(mut map) = SCAN_SESSIONS.lock() {
        map.remove(session_id);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanKeysPayload {
    pub connection_id: String,
    /// MATCH pattern. Use `*` for the whole keyspace.
    #[serde(default = "default_match")]
    pub pattern: String,
    /// Per-batch hint sent to Redis as `COUNT`. The server may
    /// return fewer or more; treat as advisory.
    #[serde(default = "default_count")]
    pub count: u32,
    /// `TYPE` filter (`string`/`hash`/`list`/`set`/`zset`/`stream`).
    /// `None` returns every type. Redis 6.0+ pushes this server-side;
    /// older servers fall back to a per-key TYPE round trip.
    #[serde(default)]
    pub type_filter: Option<String>,
    /// Cursor handed back from a previous page; `None` or `"0"`
    /// starts a fresh scan.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Optional scan-session ID. When present and the session has
    /// been opened via `open_session`, the SCAN routes through that
    /// session's dedicated connection so a `cancel_session` call can
    /// abort the in-flight server-side work via `CLIENT KILL`.
    #[serde(default)]
    pub session_id: Option<String>,
}

fn default_match() -> String {
    "*".to_string()
}
fn default_count() -> u32 {
    200
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanKeysResult {
    pub keys: Vec<ScannedKey>,
    /// `None` when the SCAN is complete (cursor returned to "0").
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedKey {
    pub name: String,
    pub r#type: String,
}

/// Run one SCAN batch. The frontend orchestrates the cursor loop —
/// each invocation returns up to `count` keys and the next cursor.
/// Reason: the Tauri serde boundary doesn't stream, so making each
/// page a separate command keeps the UI responsive.
pub async fn scan_keys(
    connection: &RedisStoredConnection,
    payload: &ScanKeysPayload,
) -> Result<ScanKeysResult, String> {
    let session_conn = payload.session_id.as_deref().and_then(cached_session_conn);

    let cursor: String = payload
        .cursor
        .clone()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "0".to_string());

    let mut cmd = redis::cmd("SCAN");
    cmd.arg(&cursor)
        .arg("MATCH")
        .arg(&payload.pattern)
        .arg("COUNT")
        .arg(payload.count);
    if let Some(type_filter) = payload.type_filter.as_deref() {
        cmd.arg("TYPE").arg(type_filter);
    }

    let (next_cursor, keys): (String, Vec<String>) = if let Some(session) = &session_conn {
        let mut guard = session.lock().await;
        cmd.query_async(&mut *guard)
            .await
            .map_err(connection::redis_err)?
    } else {
        let mut shared = connection::manager_for(connection).await?;
        cmd.query_async(&mut shared)
            .await
            .map_err(connection::redis_err)?
    };

    // Resolve type per key. When the SCAN already TYPE-filtered, we
    // know every match is the requested type. Otherwise we round-trip
    // per key — expensive on large pages but unavoidable without
    // server-side TYPE support. This loop is the part most worth
    // cancelling.
    let scanned = if let Some(t) = payload.type_filter.as_deref() {
        keys.into_iter()
            .map(|name| ScannedKey {
                name,
                r#type: t.to_string(),
            })
            .collect()
    } else if let Some(session) = &session_conn {
        let mut guard = session.lock().await;
        let mut out = Vec::with_capacity(keys.len());
        for name in keys {
            let key_type: String = guard
                .key_type(&name)
                .await
                .map_err(connection::redis_err)
                .unwrap_or_else(|_| "none".to_string());
            out.push(ScannedKey {
                name,
                r#type: key_type,
            });
        }
        out
    } else {
        let mut shared = connection::manager_for(connection).await?;
        let mut out = Vec::with_capacity(keys.len());
        for name in keys {
            let key_type: String = shared
                .key_type(&name)
                .await
                .map_err(connection::redis_err)
                .unwrap_or_else(|_| "none".to_string());
            out.push(ScannedKey {
                name,
                r#type: key_type,
            });
        }
        out
    };

    Ok(ScanKeysResult {
        keys: scanned,
        next_cursor: if next_cursor == "0" {
            None
        } else {
            Some(next_cursor)
        },
    })
}
