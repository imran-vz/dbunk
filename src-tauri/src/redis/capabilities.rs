//! Connect-time capabilities pipeline.
//!
//! Fired right after a successful `PING`, this builds the
//! [`RedisCapabilities`] payload that:
//!  - drives the post-test-connection banner in the new-connection form
//!  - feeds the server-signal auto-read-only gate (ADR-0009) via the
//!    `role` field
//!  - tells the Key Inspector whether `OBJECT FREQ` is meaningful
//!    (LFU eviction policies only)
//!
//! Every section is tolerant of `NOPERM`/unknown-command errors —
//! managed Redis frequently blocks `INFO replication`, `MODULE LIST`,
//! or `CONFIG GET`, and we'd rather show a partial banner than fail
//! the whole connect-test.

use std::time::Instant;

use crate::redis::connection;
use crate::{RedisCapabilities, RedisModuleInfo, RedisStoredConnection};

/// Connect once via a fresh multiplexed connection and run the
/// capabilities probe. Latency is measured around the initial PING
/// (the rest of the pipeline runs against the same already-open
/// socket, so its cost is amortised in subsequent server-tab fetches).
pub async fn probe(connection: &RedisStoredConnection) -> Result<(u64, RedisCapabilities), String> {
    log::info!("probe: starting for connection_id={}", connection.id);
    let start = Instant::now();
    let mut conn = connection::open_oneshot(connection).await?;

    // PING first — the latency we report is the round-trip to PING,
    // not the whole capabilities pipeline.
    match redis::cmd("PING").query_async::<String>(&mut conn).await {
        Ok(reply) => log::debug!("probe: PING → {}", reply),
        Err(err) => {
            log::error!("probe: PING failed → {}", err);
            return Err(connection::redis_err(err));
        }
    }
    let latency_ms = start.elapsed().as_millis() as u64;

    let mut caps = RedisCapabilities::default();

    match redis::cmd("INFO")
        .arg("server")
        .query_async::<String>(&mut conn)
        .await
    {
        Ok(info) => {
            caps.server_version = parse_info_field(&info, "redis_version");
            log::debug!("probe: INFO server → version={:?}", caps.server_version);
        }
        Err(err) => log::warn!("probe: INFO server failed (continuing) → {}", err),
    }

    match redis::cmd("INFO")
        .arg("replication")
        .query_async::<String>(&mut conn)
        .await
    {
        Ok(info) => {
            caps.role = parse_info_field(&info, "role");
            caps.connected_slaves = parse_info_field(&info, "connected_slaves")
                .and_then(|value| value.parse::<u32>().ok());
            log::debug!(
                "probe: INFO replication → role={:?} slaves={:?}",
                caps.role,
                caps.connected_slaves
            );
        }
        Err(err) => log::warn!("probe: INFO replication failed (continuing) → {}", err),
    }

    match redis::cmd("MODULE")
        .arg("LIST")
        .query_async::<redis::Value>(&mut conn)
        .await
    {
        Ok(modules) => {
            caps.modules = parse_module_list(modules);
            log::debug!(
                "probe: MODULE LIST → {} modules detected",
                caps.modules.as_ref().map(|m| m.len()).unwrap_or(0)
            );
        }
        Err(err) => log::warn!("probe: MODULE LIST failed (continuing) → {}", err),
    }

    match redis::cmd("DBSIZE").query_async::<u64>(&mut conn).await {
        Ok(size) => {
            caps.db_size = Some(size);
            log::debug!("probe: DBSIZE → {}", size);
        }
        Err(err) => log::warn!("probe: DBSIZE failed (continuing) → {}", err),
    }

    match redis::cmd("CONFIG")
        .arg("GET")
        .arg("maxmemory-policy")
        .query_async::<redis::Value>(&mut conn)
        .await
    {
        Ok(value) => {
            caps.maxmemory_policy = parse_config_get_value(value);
            log::debug!(
                "probe: CONFIG GET maxmemory-policy → {:?}",
                caps.maxmemory_policy
            );
        }
        Err(err) => log::warn!("probe: CONFIG GET failed (continuing) → {}", err),
    }

    log::info!("probe: complete in {}ms", latency_ms);
    Ok((latency_ms, caps))
}

/// `INFO` returns CRLF-delimited `field:value` pairs (with `#`-prefixed
/// section headers we ignore). Helper picks one field's value.
fn parse_info_field(info: &str, field: &str) -> Option<String> {
    for line in info.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.trim() == field {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// `MODULE LIST` returns an array of arrays — each inner array is a
/// flat `name <name> ver <version> ...` map. We only need `name` and
/// `ver`; everything else is dropped.
fn parse_module_list(value: redis::Value) -> Option<Vec<RedisModuleInfo>> {
    let modules = match value {
        redis::Value::Array(modules) => modules,
        _ => return None,
    };
    let mut out = Vec::with_capacity(modules.len());
    for module in modules {
        let fields = match module {
            redis::Value::Array(fields) => fields,
            _ => continue,
        };
        let mut name = None;
        let mut version = None;
        let mut iter = fields.into_iter();
        while let (Some(key), Some(value)) = (iter.next(), iter.next()) {
            let key_str = value_to_string(key);
            let value_str = value_to_string(value);
            match (key_str.as_deref(), value_str) {
                (Some("name"), Some(v)) => name = Some(v),
                (Some("ver"), Some(v)) => version = Some(v),
                _ => {}
            }
        }
        if let (Some(name), Some(version)) = (name, version) {
            out.push(RedisModuleInfo { name, version });
        }
    }
    Some(out)
}

/// `CONFIG GET key` returns a 2-element array: `[key, value]`. We only
/// want the value.
fn parse_config_get_value(value: redis::Value) -> Option<String> {
    let parts = match value {
        redis::Value::Array(parts) => parts,
        _ => return None,
    };
    let mut iter = parts.into_iter();
    let _key = iter.next()?;
    value_to_string(iter.next()?)
}

fn value_to_string(value: redis::Value) -> Option<String> {
    match value {
        redis::Value::BulkString(bytes) => String::from_utf8(bytes).ok(),
        redis::Value::SimpleString(s) => Some(s),
        redis::Value::Int(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_info_field_with_crlf() {
        let info = "# Server\r\nredis_version:7.2.4\r\nos:Darwin\r\n";
        assert_eq!(
            parse_info_field(info, "redis_version"),
            Some("7.2.4".to_string())
        );
        assert_eq!(parse_info_field(info, "os"), Some("Darwin".to_string()));
        assert_eq!(parse_info_field(info, "nonexistent"), None);
    }

    #[test]
    fn parses_replication_role_and_slaves() {
        let info = "# Replication\nrole:master\nconnected_slaves:2\n";
        assert_eq!(parse_info_field(info, "role"), Some("master".to_string()));
        assert_eq!(
            parse_info_field(info, "connected_slaves"),
            Some("2".to_string())
        );
    }

    #[test]
    fn parses_module_list_into_name_ver_pairs() {
        let value = redis::Value::Array(vec![redis::Value::Array(vec![
            redis::Value::BulkString(b"name".to_vec()),
            redis::Value::BulkString(b"ReJSON".to_vec()),
            redis::Value::BulkString(b"ver".to_vec()),
            redis::Value::Int(20610),
        ])]);
        let modules = parse_module_list(value).unwrap();
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "ReJSON");
        assert_eq!(modules[0].version, "20610");
    }

    #[test]
    fn parses_config_get_returns_value_only() {
        let value = redis::Value::Array(vec![
            redis::Value::BulkString(b"maxmemory-policy".to_vec()),
            redis::Value::BulkString(b"allkeys-lfu".to_vec()),
        ]);
        assert_eq!(parse_config_get_value(value), Some("allkeys-lfu".into()));
    }
}
