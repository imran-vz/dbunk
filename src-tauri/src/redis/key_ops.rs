//! Write operations on Redis keys — string + hash editors and the
//! key-level ops (DEL, EXPIRE/PERSIST, RENAME, create_key for all
//! seven types). All paths consult the auto-read-only state and
//! refuse on replicas.

use serde::{Deserialize, Serialize};

use crate::redis::capabilities;
use crate::redis::connection;
use crate::StoredConnection;

async fn assert_writable(connection: &StoredConnection) -> Result<(), String> {
    // Cheap-ish: the capabilities probe runs `INFO replication`. We
    // cache nothing today; the cost is one round trip per write. If
    // this shows up on a profile we'll cache per-session.
    let (_, caps) = capabilities::probe(connection).await?;
    if matches!(caps.role.as_deref(), Some("replica") | Some("slave")) {
        return Err(
            "This Redis server reports role=replica. Writes are disabled \
             (ADR-0009 auto-read-only)."
                .to_string(),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// String
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetStringPayload {
    pub connection_id: String,
    pub key: String,
    pub value: String,
    /// Optional TTL in seconds; `None` = no expiry change.
    #[serde(default)]
    pub ttl_seconds: Option<i64>,
    /// `true` → also issue PERSIST after SET, removing any TTL.
    #[serde(default)]
    pub persist: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetStringResult {
    pub ok: bool,
}

pub async fn set_string(
    connection: &StoredConnection,
    payload: &SetStringPayload,
) -> Result<SetStringResult, String> {
    assert_writable(connection).await?;
    let mut conn = connection::manager_for(connection).await?;
    let mut cmd = redis::cmd("SET");
    cmd.arg(&payload.key).arg(&payload.value);
    if let Some(ttl) = payload.ttl_seconds {
        if ttl > 0 {
            cmd.arg("EX").arg(ttl);
        } else if !payload.persist {
            cmd.arg("KEEPTTL");
        }
    } else if !payload.persist {
        cmd.arg("KEEPTTL");
    }
    let _: redis::Value = cmd
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    if payload.persist {
        let _: redis::Value = redis::cmd("PERSIST")
            .arg(&payload.key)
            .query_async(&mut conn)
            .await
            .map_err(connection::redis_err)?;
    }
    Ok(SetStringResult { ok: true })
}

// ---------------------------------------------------------------------------
// Hash
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetHashFieldsPayload {
    pub connection_id: String,
    pub key: String,
    pub entries: Vec<(String, String)>,
}

pub async fn set_hash_fields(
    connection: &StoredConnection,
    payload: &SetHashFieldsPayload,
) -> Result<(), String> {
    assert_writable(connection).await?;
    if payload.entries.is_empty() {
        return Ok(());
    }
    let mut conn = connection::manager_for(connection).await?;
    let mut cmd = redis::cmd("HSET");
    cmd.arg(&payload.key);
    for (field, value) in &payload.entries {
        cmd.arg(field).arg(value);
    }
    let _: redis::Value = cmd
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteHashFieldsPayload {
    pub connection_id: String,
    pub key: String,
    pub fields: Vec<String>,
}

pub async fn delete_hash_fields(
    connection: &StoredConnection,
    payload: &DeleteHashFieldsPayload,
) -> Result<(), String> {
    assert_writable(connection).await?;
    if payload.fields.is_empty() {
        return Ok(());
    }
    let mut conn = connection::manager_for(connection).await?;
    let mut cmd = redis::cmd("HDEL");
    cmd.arg(&payload.key);
    for field in &payload.fields {
        cmd.arg(field);
    }
    let _: redis::Value = cmd
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Key-level ops
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelKeysPayload {
    pub connection_id: String,
    pub keys: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelKeysResult {
    pub deleted: u64,
}

pub async fn del_keys(
    connection: &StoredConnection,
    payload: &DelKeysPayload,
) -> Result<DelKeysResult, String> {
    assert_writable(connection).await?;
    if payload.keys.is_empty() {
        return Ok(DelKeysResult { deleted: 0 });
    }
    let mut conn = connection::manager_for(connection).await?;
    let mut cmd = redis::cmd("DEL");
    for key in &payload.keys {
        cmd.arg(key);
    }
    let n: i64 = cmd
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    Ok(DelKeysResult {
        deleted: n.max(0) as u64,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetExpirePayload {
    pub connection_id: String,
    pub key: String,
    /// `Some(n)` → EXPIRE n; `None` → PERSIST (remove TTL).
    #[serde(default)]
    pub ttl_seconds: Option<i64>,
}

pub async fn set_expire(
    connection: &StoredConnection,
    payload: &SetExpirePayload,
) -> Result<(), String> {
    assert_writable(connection).await?;
    let mut conn = connection::manager_for(connection).await?;
    match payload.ttl_seconds {
        Some(seconds) if seconds > 0 => {
            let _: redis::Value = redis::cmd("EXPIRE")
                .arg(&payload.key)
                .arg(seconds)
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
        }
        _ => {
            let _: redis::Value = redis::cmd("PERSIST")
                .arg(&payload.key)
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameKeyPayload {
    pub connection_id: String,
    pub from: String,
    pub to: String,
}

pub async fn rename_key(
    connection: &StoredConnection,
    payload: &RenameKeyPayload,
) -> Result<(), String> {
    assert_writable(connection).await?;
    if payload.from == payload.to {
        return Ok(());
    }
    let mut conn = connection::manager_for(connection).await?;
    let _: redis::Value = redis::cmd("RENAME")
        .arg(&payload.from)
        .arg(&payload.to)
        .query_async(&mut conn)
        .await
        .map_err(connection::redis_err)?;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateKeyPayload {
    pub connection_id: String,
    pub key: String,
    /// `string`, `hash`, `list`, `set`, `zset`, `stream`, `json`.
    pub r#type: String,
    /// Type-specific payload. For string: `{ value }`. For hash:
    /// `{ entries: [[field, value], ...] }`. For list: `{ items }`.
    /// For set: `{ members }`. For zset: `{ entries: [[member, score]] }`.
    /// For stream: `{ entries: [{ id?, fields: [[k, v]] }] }`. For
    /// json: `{ value }` (the raw JSON text).
    pub payload: serde_json::Value,
    #[serde(default)]
    pub ttl_seconds: Option<i64>,
}

pub async fn create_key(
    connection: &StoredConnection,
    payload: &CreateKeyPayload,
) -> Result<(), String> {
    assert_writable(connection).await?;
    let mut conn = connection::manager_for(connection).await?;

    match payload.r#type.as_str() {
        "string" => {
            let value = payload
                .payload
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or("string create requires `payload.value`")?;
            let mut cmd = redis::cmd("SET");
            cmd.arg(&payload.key).arg(value);
            if let Some(ttl) = payload.ttl_seconds {
                if ttl > 0 {
                    cmd.arg("EX").arg(ttl);
                }
            }
            let _: redis::Value = cmd
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
        }
        "hash" => {
            let entries = payload
                .payload
                .get("entries")
                .and_then(|v| v.as_array())
                .ok_or("hash create requires `payload.entries`")?;
            let mut cmd = redis::cmd("HSET");
            cmd.arg(&payload.key);
            for pair in entries {
                let field = pair
                    .get(0)
                    .and_then(|v| v.as_str())
                    .ok_or("hash entry missing field")?;
                let value = pair
                    .get(1)
                    .and_then(|v| v.as_str())
                    .ok_or("hash entry missing value")?;
                cmd.arg(field).arg(value);
            }
            let _: redis::Value = cmd
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
        }
        "list" => {
            let items = payload
                .payload
                .get("items")
                .and_then(|v| v.as_array())
                .ok_or("list create requires `payload.items`")?;
            let mut cmd = redis::cmd("RPUSH");
            cmd.arg(&payload.key);
            for item in items {
                cmd.arg(item.as_str().unwrap_or(""));
            }
            let _: redis::Value = cmd
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
        }
        "set" => {
            let members = payload
                .payload
                .get("members")
                .and_then(|v| v.as_array())
                .ok_or("set create requires `payload.members`")?;
            let mut cmd = redis::cmd("SADD");
            cmd.arg(&payload.key);
            for m in members {
                cmd.arg(m.as_str().unwrap_or(""));
            }
            let _: redis::Value = cmd
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
        }
        "zset" => {
            let entries = payload
                .payload
                .get("entries")
                .and_then(|v| v.as_array())
                .ok_or("zset create requires `payload.entries`")?;
            let mut cmd = redis::cmd("ZADD");
            cmd.arg(&payload.key);
            for pair in entries {
                let member = pair
                    .get(0)
                    .and_then(|v| v.as_str())
                    .ok_or("zset entry missing member")?;
                let score = pair
                    .get(1)
                    .and_then(|v| v.as_f64())
                    .ok_or("zset entry missing score")?;
                cmd.arg(score).arg(member);
            }
            let _: redis::Value = cmd
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
        }
        "stream" => {
            let entries = payload
                .payload
                .get("entries")
                .and_then(|v| v.as_array())
                .ok_or("stream create requires `payload.entries`")?;
            for entry in entries {
                let id = entry
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("*");
                let fields = entry
                    .get("fields")
                    .and_then(|v| v.as_array())
                    .ok_or("stream entry missing fields")?;
                let mut cmd = redis::cmd("XADD");
                cmd.arg(&payload.key).arg(id);
                for pair in fields {
                    let k = pair
                        .get(0)
                        .and_then(|v| v.as_str())
                        .ok_or("stream field missing key")?;
                    let v = pair
                        .get(1)
                        .and_then(|v| v.as_str())
                        .ok_or("stream field missing value")?;
                    cmd.arg(k).arg(v);
                }
                let _: redis::Value = cmd
                    .query_async(&mut conn)
                    .await
                    .map_err(connection::redis_err)?;
            }
        }
        "json" => {
            let value = payload
                .payload
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or("json create requires `payload.value` (raw JSON text)")?;
            let _: redis::Value = redis::cmd("JSON.SET")
                .arg(&payload.key)
                .arg("$")
                .arg(value)
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
        }
        other => return Err(format!("unsupported type for create_key: {other}")),
    }

    if let Some(ttl) = payload.ttl_seconds {
        if ttl > 0 && payload.r#type != "string" {
            let _: redis::Value = redis::cmd("EXPIRE")
                .arg(&payload.key)
                .arg(ttl)
                .query_async(&mut conn)
                .await
                .map_err(connection::redis_err)?;
        }
    }

    Ok(())
}
