//! Server-tab fetch — pipelined INFO sections + MODULE LIST +
//! SLOWLOG. Returns [`KeyValueOverviewStats`] for the
//! `fetch_keyvalue_overview` Tauri command. Per-section degradation
//! (missing/restricted sections render as `None`) so a single ACL
//! restriction doesn't blank the whole tab.

use serde::Serialize;

use crate::redis::connection;
use crate::{RedisModuleInfo, StoredConnection};

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KeyValueOverviewStats {
    pub identity: ServerIdentity,
    pub keyspace: Vec<KeyspaceInfo>,
    pub memory: Option<MemoryInfo>,
    pub clients: Option<ClientsInfo>,
    pub replication: Option<ReplicationInfo>,
    pub modules: Option<Vec<RedisModuleInfo>>,
    pub slow_log: Option<Vec<SlowLogEntry>>,
    pub persistence: Option<PersistenceInfo>,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServerIdentity {
    pub version: Option<String>,
    pub mode: Option<String>,
    pub uptime_seconds: Option<u64>,
    pub os: Option<String>,
    pub arch: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyspaceInfo {
    pub db_number: u8,
    pub keys: u64,
    pub expires: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInfo {
    pub used_memory: Option<u64>,
    pub used_memory_rss: Option<u64>,
    pub used_memory_peak: Option<u64>,
    pub fragmentation_ratio: Option<f64>,
    pub maxmemory: Option<u64>,
    pub maxmemory_policy: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientsInfo {
    pub connected_clients: u64,
    pub maxclients: Option<u64>,
    pub blocked_clients: u64,
    pub tracking_clients: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplicationInfo {
    pub role: String,
    pub connected_slaves: Option<u32>,
    pub master_link_status: Option<String>,
    pub master_host: Option<String>,
    pub master_port: Option<u16>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlowLogEntry {
    pub id: i64,
    pub timestamp: i64,
    pub duration_us: i64,
    pub command: String,
    pub client_address: Option<String>,
    pub client_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistenceInfo {
    pub rdb_last_save_time: Option<i64>,
    pub rdb_changes_since_last_save: Option<u64>,
    pub aof_enabled: Option<bool>,
    pub aof_last_rewrite_time: Option<i64>,
}

pub async fn fetch_overview(
    connection: &StoredConnection,
) -> Result<KeyValueOverviewStats, String> {
    let mut conn = connection::manager_for(connection).await?;

    let mut out = KeyValueOverviewStats::default();

    if let Ok(info) = redis::cmd("INFO")
        .arg("server")
        .query_async::<String>(&mut conn)
        .await
    {
        out.identity = ServerIdentity {
            version: info_field(&info, "redis_version"),
            mode: info_field(&info, "redis_mode"),
            uptime_seconds: info_field(&info, "uptime_in_seconds")
                .and_then(|v| v.parse().ok()),
            os: info_field(&info, "os"),
            arch: info_field(&info, "arch_bits"),
        };
    }

    if let Ok(info) = redis::cmd("INFO")
        .arg("keyspace")
        .query_async::<String>(&mut conn)
        .await
    {
        out.keyspace = parse_keyspace(&info);
    }

    if let Ok(info) = redis::cmd("INFO")
        .arg("memory")
        .query_async::<String>(&mut conn)
        .await
    {
        out.memory = Some(MemoryInfo {
            used_memory: info_field(&info, "used_memory").and_then(|v| v.parse().ok()),
            used_memory_rss: info_field(&info, "used_memory_rss")
                .and_then(|v| v.parse().ok()),
            used_memory_peak: info_field(&info, "used_memory_peak")
                .and_then(|v| v.parse().ok()),
            fragmentation_ratio: info_field(&info, "mem_fragmentation_ratio")
                .and_then(|v| v.parse().ok()),
            maxmemory: info_field(&info, "maxmemory").and_then(|v| v.parse().ok()),
            maxmemory_policy: info_field(&info, "maxmemory_policy"),
        });
    }

    if let Ok(info) = redis::cmd("INFO")
        .arg("clients")
        .query_async::<String>(&mut conn)
        .await
    {
        out.clients = Some(ClientsInfo {
            connected_clients: info_field(&info, "connected_clients")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            maxclients: info_field(&info, "maxclients").and_then(|v| v.parse().ok()),
            blocked_clients: info_field(&info, "blocked_clients")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            tracking_clients: info_field(&info, "tracking_clients")
                .and_then(|v| v.parse().ok()),
        });
    }

    if let Ok(info) = redis::cmd("INFO")
        .arg("replication")
        .query_async::<String>(&mut conn)
        .await
    {
        let role = info_field(&info, "role").unwrap_or_else(|| "unknown".into());
        out.replication = Some(ReplicationInfo {
            role,
            connected_slaves: info_field(&info, "connected_slaves")
                .and_then(|v| v.parse().ok()),
            master_link_status: info_field(&info, "master_link_status"),
            master_host: info_field(&info, "master_host"),
            master_port: info_field(&info, "master_port").and_then(|v| v.parse().ok()),
        });
    }

    if let Ok(info) = redis::cmd("INFO")
        .arg("persistence")
        .query_async::<String>(&mut conn)
        .await
    {
        out.persistence = Some(PersistenceInfo {
            rdb_last_save_time: info_field(&info, "rdb_last_save_time")
                .and_then(|v| v.parse().ok()),
            rdb_changes_since_last_save: info_field(&info, "rdb_changes_since_last_save")
                .and_then(|v| v.parse().ok()),
            aof_enabled: info_field(&info, "aof_enabled").map(|v| v != "0"),
            aof_last_rewrite_time: info_field(&info, "aof_last_rewrite_time_sec")
                .and_then(|v| v.parse().ok()),
        });
    }

    if let Ok(modules) = redis::cmd("MODULE")
        .arg("LIST")
        .query_async::<redis::Value>(&mut conn)
        .await
    {
        out.modules = parse_module_list(modules);
    }

    if let Ok(entries) = redis::cmd("SLOWLOG")
        .arg("GET")
        .arg(25)
        .query_async::<redis::Value>(&mut conn)
        .await
    {
        out.slow_log = parse_slow_log(entries);
    }

    Ok(out)
}

fn info_field(info: &str, field: &str) -> Option<String> {
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

fn parse_keyspace(info: &str) -> Vec<KeyspaceInfo> {
    let mut out = Vec::new();
    for line in info.lines() {
        let line = line.trim();
        if !line.starts_with("db") {
            continue;
        }
        // `db0:keys=12,expires=3,avg_ttl=0`
        let (db_part, body) = match line.split_once(':') {
            Some(parts) => parts,
            None => continue,
        };
        let db_number: u8 = db_part.trim_start_matches("db").parse().unwrap_or(0);
        let mut keys = 0u64;
        let mut expires = 0u64;
        for kv in body.split(',') {
            if let Some((k, v)) = kv.split_once('=') {
                match k {
                    "keys" => keys = v.parse().unwrap_or(0),
                    "expires" => expires = v.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        out.push(KeyspaceInfo {
            db_number,
            keys,
            expires,
        });
    }
    out
}

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
            let key_str = scalar(key);
            let value_str = scalar(value);
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

fn parse_slow_log(value: redis::Value) -> Option<Vec<SlowLogEntry>> {
    let entries = match value {
        redis::Value::Array(entries) => entries,
        _ => return None,
    };
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let parts = match entry {
            redis::Value::Array(p) => p,
            _ => continue,
        };
        let mut iter = parts.into_iter();
        let id = match iter.next() {
            Some(redis::Value::Int(n)) => n,
            _ => continue,
        };
        let timestamp = match iter.next() {
            Some(redis::Value::Int(n)) => n,
            _ => continue,
        };
        let duration = match iter.next() {
            Some(redis::Value::Int(n)) => n,
            _ => continue,
        };
        let command = match iter.next() {
            Some(redis::Value::Array(tokens)) => tokens
                .into_iter()
                .filter_map(scalar)
                .collect::<Vec<_>>()
                .join(" "),
            _ => String::new(),
        };
        let client_address = iter.next().and_then(scalar);
        let client_name = iter.next().and_then(scalar);
        out.push(SlowLogEntry {
            id,
            timestamp,
            duration_us: duration,
            command,
            client_address,
            client_name,
        });
    }
    Some(out)
}

fn scalar(value: redis::Value) -> Option<String> {
    match value {
        redis::Value::BulkString(bytes) => String::from_utf8(bytes).ok(),
        redis::Value::SimpleString(s) => Some(s),
        redis::Value::Int(n) => Some(n.to_string()),
        _ => None,
    }
}
