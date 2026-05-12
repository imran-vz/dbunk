//! Keyspace browsing — SCAN-driven enumeration of Redis keys.
//!
//! The frontend's `KeyspaceBrowser` calls into this module for both
//! tree-mode expansion (one `SCAN MATCH prefix:*` per branch) and
//! flat search-mode (`SCAN MATCH *foo*`). Cursor state is exposed to
//! the caller so pagination resumes across multiple invocations.

use redis::AsyncCommands;
use serde::{Deserialize, Serialize};

use crate::redis::connection;
use crate::StoredConnection;

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
    connection: &StoredConnection,
    payload: &ScanKeysPayload,
) -> Result<ScanKeysResult, String> {
    let mut conn = connection::manager_for(connection).await?;

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

    let (next_cursor, keys): (String, Vec<String>) = cmd
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;

    // Resolve type per key. When the SCAN already TYPE-filtered, we
    // know every match is the requested type. Otherwise we round-trip
    // per key — expensive on large pages but unavoidable without
    // server-side TYPE support.
    let scanned = if let Some(t) = payload.type_filter.as_deref() {
        keys.into_iter()
            .map(|name| ScannedKey {
                name,
                r#type: t.to_string(),
            })
            .collect()
    } else {
        let mut out = Vec::with_capacity(keys.len());
        for name in keys {
            let key_type: String = conn
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
