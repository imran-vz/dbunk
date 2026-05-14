//! Per-key inspection — metadata + per-type value fetchers.
//!
//! The frontend's `KeyInspectorTab` pipelines `fetch_key_metadata`
//! (TTL + encoding + size + element count) with a type-specific
//! `fetch_*` call (`fetch_string`, `fetch_hash`, etc.). Each fetcher
//! returns paginated data where applicable; the inspector's
//! preliminary count drives full-fetch vs SCAN-mode selection (Q15).

use redis::AsyncCommands;
use serde::{Deserialize, Serialize};

use crate::redis::connection;
use crate::redis::value::{self, SerializedValue};
use crate::RedisStoredConnection;

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyPayload {
    pub connection_id: String,
    pub key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyMetadata {
    /// `string`/`hash`/`list`/`set`/`zset`/`stream`/`none` (when the
    /// key doesn't exist).
    pub r#type: String,
    /// Seconds until expiry. `-1` = no expiry; `-2` = key missing.
    pub ttl_seconds: i64,
    /// Internal Redis encoding (`embstr`/`raw`/`listpack`/etc.).
    /// `None` when `OBJECT ENCODING` errors (managed Redis sometimes
    /// blocks `OBJECT`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// Element count for collections / string length in bytes for
    /// strings. `None` when not applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_count: Option<u64>,
    /// `MEMORY USAGE` result in bytes. Gated for large keys by the
    /// frontend — `None` when not measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
}

pub async fn fetch_key_metadata(
    connection: &RedisStoredConnection,
    payload: &KeyPayload,
) -> Result<KeyMetadata, String> {
    let mut conn = connection::manager_for(connection).await?;

    let key_type: String = conn
        .key_type(&payload.key)
        .await
        .map_err(connection::redis_err)?;
    let ttl: i64 = redis::cmd("TTL")
        .arg(&payload.key)
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    let encoding: Option<String> = redis::cmd("OBJECT")
        .arg("ENCODING")
        .arg(&payload.key)
        .query_async(&mut conn)
        .await
        .ok();

    let element_count = match key_type.as_str() {
        "string" => redis::cmd("STRLEN")
            .arg(&payload.key)
            .query_async::<i64>(&mut conn)
            .await
            .ok()
            .map(|n| n.max(0) as u64),
        "hash" => conn.hlen::<_, i64>(&payload.key).await.ok().map(|n| n as u64),
        "list" => conn.llen::<_, i64>(&payload.key).await.ok().map(|n| n as u64),
        "set" => conn.scard::<_, i64>(&payload.key).await.ok().map(|n| n as u64),
        "zset" => conn.zcard::<_, i64>(&payload.key).await.ok().map(|n| n as u64),
        "stream" => redis::cmd("XLEN")
            .arg(&payload.key)
            .query_async::<i64>(&mut conn)
            .await
            .ok()
            .map(|n| n.max(0) as u64),
        _ => None,
    };

    Ok(KeyMetadata {
        r#type: key_type,
        ttl_seconds: ttl,
        encoding,
        element_count,
        memory_bytes: None,
    })
}

// ---------------------------------------------------------------------------
// String
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchStringPayload {
    pub connection_id: String,
    pub key: String,
    #[serde(default = "default_max_bytes")]
    pub max_bytes: u32,
}

fn default_max_bytes() -> u32 {
    1024 * 1024
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StringValuePayload {
    pub value: SerializedValue,
    pub total_bytes: u64,
    pub truncated: bool,
}

pub async fn fetch_string(
    connection: &RedisStoredConnection,
    payload: &FetchStringPayload,
) -> Result<StringValuePayload, String> {
    let mut conn = connection::manager_for(connection).await?;

    let total: i64 = conn
        .strlen(&payload.key)
        .await
        .map_err(connection::redis_err)?;
    let total_bytes = total.max(0) as u64;

    if total_bytes > payload.max_bytes as u64 {
        // GETRANGE 0 max-1 — bounded prefix only.
        let bytes: Vec<u8> = redis::cmd("GETRANGE")
            .arg(&payload.key)
            .arg(0)
            .arg(i64::from(payload.max_bytes) - 1)
            .query_async(&mut conn)
            .await
            .map_err(connection::redis_err)?;
        return Ok(StringValuePayload {
            value: value::encode_string(bytes),
            total_bytes,
            truncated: true,
        });
    }

    let bytes: Option<Vec<u8>> = conn.get(&payload.key).await.map_err(connection::redis_err)?;
    let bytes = bytes.unwrap_or_default();
    Ok(StringValuePayload {
        value: value::encode_string(bytes),
        total_bytes,
        truncated: false,
    })
}

// ---------------------------------------------------------------------------
// Hash
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchHashPayload {
    pub connection_id: String,
    pub key: String,
    /// `full` or `scan`. The inspector picks `full` when HLEN is
    /// below the threshold (default 500), else `scan`.
    #[serde(default)]
    pub mode: HashMode,
    #[serde(default = "default_scan_count")]
    pub count: u32,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HashMode {
    #[default]
    Full,
    Scan,
}

fn default_scan_count() -> u32 {
    200
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HashValuePayload {
    pub entries: Vec<(SerializedValue, SerializedValue)>,
    pub next_cursor: Option<String>,
}

pub async fn fetch_hash(
    connection: &RedisStoredConnection,
    payload: &FetchHashPayload,
) -> Result<HashValuePayload, String> {
    let mut conn = connection::manager_for(connection).await?;

    match payload.mode {
        HashMode::Full => {
            let raw: Vec<redis::Value> = redis::cmd("HGETALL")
                .arg(&payload.key)
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
            Ok(HashValuePayload {
                entries: pair_up(raw),
                next_cursor: None,
            })
        }
        HashMode::Scan => {
            let cursor = payload.cursor.clone().unwrap_or_else(|| "0".to_string());
            let mut cmd = redis::cmd("HSCAN");
            cmd.arg(&payload.key)
                .arg(&cursor)
                .arg("COUNT")
                .arg(payload.count);
            if let Some(pattern) = payload.pattern.as_deref() {
                cmd.arg("MATCH").arg(pattern);
            }
            let (next, raw): (String, Vec<redis::Value>) = cmd
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
            Ok(HashValuePayload {
                entries: pair_up(raw),
                next_cursor: if next == "0" { None } else { Some(next) },
            })
        }
    }
}

fn pair_up(values: Vec<redis::Value>) -> Vec<(SerializedValue, SerializedValue)> {
    let mut iter = values.into_iter();
    let mut out = Vec::new();
    while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
        out.push((value::serialize(k), value::serialize(v)));
    }
    out
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchListPayload {
    pub connection_id: String,
    pub key: String,
    #[serde(default)]
    pub start: i64,
    #[serde(default = "default_list_stop")]
    pub stop: i64,
    #[serde(default)]
    pub reverse: bool,
}

fn default_list_stop() -> i64 {
    199
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListValuePayload {
    pub items: Vec<SerializedValue>,
}

pub async fn fetch_list(
    connection: &RedisStoredConnection,
    payload: &FetchListPayload,
) -> Result<ListValuePayload, String> {
    let mut conn = connection::manager_for(connection).await?;
    let raw: Vec<redis::Value> = redis::cmd("LRANGE")
        .arg(&payload.key)
        .arg(payload.start)
        .arg(payload.stop)
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    let mut items: Vec<SerializedValue> = raw.into_iter().map(value::serialize).collect();
    if payload.reverse {
        items.reverse();
    }
    Ok(ListValuePayload { items })
}

// ---------------------------------------------------------------------------
// Set
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchSetPayload {
    pub connection_id: String,
    pub key: String,
    #[serde(default)]
    pub mode: SetMode,
    #[serde(default = "default_scan_count")]
    pub count: u32,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SetMode {
    #[default]
    Full,
    Scan,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetValuePayload {
    pub members: Vec<SerializedValue>,
    pub next_cursor: Option<String>,
}

pub async fn fetch_set(
    connection: &RedisStoredConnection,
    payload: &FetchSetPayload,
) -> Result<SetValuePayload, String> {
    let mut conn = connection::manager_for(connection).await?;

    match payload.mode {
        SetMode::Full => {
            let raw: Vec<redis::Value> = redis::cmd("SMEMBERS")
                .arg(&payload.key)
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
            Ok(SetValuePayload {
                members: raw.into_iter().map(value::serialize).collect(),
                next_cursor: None,
            })
        }
        SetMode::Scan => {
            let cursor = payload.cursor.clone().unwrap_or_else(|| "0".to_string());
            let mut cmd = redis::cmd("SSCAN");
            cmd.arg(&payload.key)
                .arg(&cursor)
                .arg("COUNT")
                .arg(payload.count);
            if let Some(pattern) = payload.pattern.as_deref() {
                cmd.arg("MATCH").arg(pattern);
            }
            let (next, raw): (String, Vec<redis::Value>) = cmd
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
            Ok(SetValuePayload {
                members: raw.into_iter().map(value::serialize).collect(),
                next_cursor: if next == "0" { None } else { Some(next) },
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Sorted set
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchSortedSetPayload {
    pub connection_id: String,
    pub key: String,
    #[serde(default)]
    pub mode: ZsetMode,
    #[serde(default)]
    pub start: i64,
    #[serde(default = "default_list_stop")]
    pub stop: i64,
    #[serde(default)]
    pub reverse: bool,
    /// For `byscore` mode — `-inf` and `+inf` accepted.
    #[serde(default)]
    pub score_min: Option<String>,
    #[serde(default)]
    pub score_max: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ZsetMode {
    #[default]
    Rank,
    Byscore,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SortedSetValuePayload {
    pub entries: Vec<(SerializedValue, f64)>,
}

pub async fn fetch_sorted_set(
    connection: &RedisStoredConnection,
    payload: &FetchSortedSetPayload,
) -> Result<SortedSetValuePayload, String> {
    let mut conn = connection::manager_for(connection).await?;

    match payload.mode {
        ZsetMode::Rank => {
            let cmd_name = if payload.reverse {
                "ZREVRANGE"
            } else {
                "ZRANGE"
            };
            let raw: Vec<redis::Value> = redis::cmd(cmd_name)
                .arg(&payload.key)
                .arg(payload.start)
                .arg(payload.stop)
                .arg("WITHSCORES")
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
            Ok(SortedSetValuePayload {
                entries: parse_zset_entries(raw),
            })
        }
        ZsetMode::Byscore => {
            let min = payload.score_min.clone().unwrap_or_else(|| "-inf".to_string());
            let max = payload.score_max.clone().unwrap_or_else(|| "+inf".to_string());
            let cmd_name = if payload.reverse {
                "ZREVRANGEBYSCORE"
            } else {
                "ZRANGEBYSCORE"
            };
            let raw: Vec<redis::Value> = redis::cmd(cmd_name)
                .arg(&payload.key)
                .arg(if payload.reverse { &max } else { &min })
                .arg(if payload.reverse { &min } else { &max })
                .arg("WITHSCORES")
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
            Ok(SortedSetValuePayload {
                entries: parse_zset_entries(raw),
            })
        }
    }
}

fn parse_zset_entries(values: Vec<redis::Value>) -> Vec<(SerializedValue, f64)> {
    let mut iter = values.into_iter();
    let mut out = Vec::new();
    while let (Some(member), Some(score)) = (iter.next(), iter.next()) {
        let score_str = match value::serialize(score) {
            SerializedValue::String { value, .. } => value,
            SerializedValue::Int { value } => value.to_string(),
            _ => "0".to_string(),
        };
        let score_f64 = score_str.parse::<f64>().unwrap_or(0.0);
        out.push((value::serialize(member), score_f64));
    }
    out
}

// ---------------------------------------------------------------------------
// Stream
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchStreamPayload {
    pub connection_id: String,
    pub key: String,
    #[serde(default = "default_stream_start")]
    pub start: String,
    #[serde(default = "default_stream_end")]
    pub end: String,
    #[serde(default = "default_scan_count")]
    pub count: u32,
    #[serde(default)]
    pub reverse: bool,
}

fn default_stream_start() -> String {
    "-".to_string()
}
fn default_stream_end() -> String {
    "+".to_string()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamValuePayload {
    pub entries: Vec<StreamEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamEntry {
    pub id: String,
    pub fields: Vec<(SerializedValue, SerializedValue)>,
}

pub async fn fetch_stream(
    connection: &RedisStoredConnection,
    payload: &FetchStreamPayload,
) -> Result<StreamValuePayload, String> {
    let mut conn = connection::manager_for(connection).await?;
    let cmd_name = if payload.reverse {
        "XREVRANGE"
    } else {
        "XRANGE"
    };
    let raw: Vec<redis::Value> = redis::cmd(cmd_name)
        .arg(&payload.key)
        .arg(if payload.reverse {
            &payload.end
        } else {
            &payload.start
        })
        .arg(if payload.reverse {
            &payload.start
        } else {
            &payload.end
        })
        .arg("COUNT")
        .arg(payload.count)
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;

    let mut entries = Vec::with_capacity(raw.len());
    for entry in raw {
        let parts = match entry {
            redis::Value::Array(parts) => parts,
            _ => continue,
        };
        let mut iter = parts.into_iter();
        let id_value = iter.next().unwrap_or(redis::Value::Nil);
        let id = match value::serialize(id_value) {
            SerializedValue::String { value, .. } => value,
            SerializedValue::Status { value } => value,
            _ => "?".to_string(),
        };
        let fields_value = iter.next().unwrap_or(redis::Value::Array(vec![]));
        let field_pairs = match fields_value {
            redis::Value::Array(values) => pair_up(values),
            _ => vec![],
        };
        entries.push(StreamEntry {
            id,
            fields: field_pairs,
        });
    }

    Ok(StreamValuePayload { entries })
}

// ---------------------------------------------------------------------------
// JSON (RedisJSON / ReJSON)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchJsonPayload {
    pub connection_id: String,
    pub key: String,
    /// JSONPath (`$` for the full document).
    #[serde(default = "default_json_path")]
    pub path: String,
}

fn default_json_path() -> String {
    "$".to_string()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonValuePayload {
    /// The raw JSON text returned by `JSON.GET`. The frontend parses
    /// it for tree rendering.
    pub value: String,
}

pub async fn fetch_json(
    connection: &RedisStoredConnection,
    payload: &FetchJsonPayload,
) -> Result<JsonValuePayload, String> {
    let mut conn = connection::manager_for(connection).await?;
    let raw: redis::Value = redis::cmd("JSON.GET")
        .arg(&payload.key)
        .arg(&payload.path)
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    let text = match value::serialize(raw) {
        SerializedValue::String { value, .. } => value,
        SerializedValue::Status { value } => value,
        SerializedValue::Nil => "null".to_string(),
        other => serde_json::to_string(&other).unwrap_or_else(|_| "null".to_string()),
    };
    Ok(JsonValuePayload { value: text })
}
