use std::str::FromStr;

use sqlx::{Row, SqlitePool};

use crate::{BastionAuthMethod, BastionServer};

use super::{i64_to_u16, now};

fn row_to_bastion(row: sqlx::sqlite::SqliteRow) -> Result<BastionServer, String> {
    Ok(BastionServer {
        id: row.get("id"),
        name: row.get("name"),
        host: row.get("host"),
        port: i64_to_u16(row.get("port")),
        user: row.get("user_name"),
        auth_method: BastionAuthMethod::from_str(row.get::<String, _>("auth_method").as_str())?,
        private_key_path: row.get("private_key_path"),
        host_key_fingerprint: row.get("host_key_fingerprint"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub async fn read_bastion_servers(pool: &SqlitePool) -> Result<Vec<BastionServer>, String> {
    let rows = sqlx::query(
        "SELECT id, name, host, port, user_name, auth_method,
                private_key_path, host_key_fingerprint, created_at, updated_at
         FROM bastion_servers
         ORDER BY name COLLATE NOCASE ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;

    rows.into_iter().map(row_to_bastion).collect()
}

pub async fn read_bastion_server_by_id(
    pool: &SqlitePool,
    bastion_id: &str,
) -> Result<Option<BastionServer>, String> {
    let row = sqlx::query(
        "SELECT id, name, host, port, user_name, auth_method,
                private_key_path, host_key_fingerprint, created_at, updated_at
         FROM bastion_servers
         WHERE id = ?",
    )
    .bind(bastion_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;

    row.map(row_to_bastion).transpose()
}

pub async fn upsert_bastion_server(
    pool: &SqlitePool,
    bastion: &BastionServer,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO bastion_servers (
            id, name, host, port, user_name, auth_method, private_key_path,
            host_key_fingerprint, created_at, updated_at
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            host = excluded.host,
            port = excluded.port,
            user_name = excluded.user_name,
            auth_method = excluded.auth_method,
            private_key_path = excluded.private_key_path,
            host_key_fingerprint = excluded.host_key_fingerprint,
            updated_at = excluded.updated_at",
    )
    .bind(&bastion.id)
    .bind(&bastion.name)
    .bind(&bastion.host)
    .bind(i64::from(bastion.port))
    .bind(&bastion.user)
    .bind(bastion.auth_method.as_str())
    .bind(&bastion.private_key_path)
    .bind(&bastion.host_key_fingerprint)
    .bind(&bastion.created_at)
    .bind(&bastion.updated_at)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn delete_bastion_server(pool: &SqlitePool, bastion_id: &str) -> Result<bool, String> {
    let result = sqlx::query("DELETE FROM bastion_servers WHERE id = ?")
        .bind(bastion_id)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(result.rows_affected() > 0)
}

pub async fn count_connections_referencing_bastion(
    pool: &SqlitePool,
    bastion_id: &str,
) -> Result<i64, String> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM connections
         WHERE ssh_tunnel_enabled = 1 AND ssh_tunnel_bastion_server_id = ?",
    )
    .bind(bastion_id)
    .fetch_one(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(count)
}

pub async fn connection_ids_referencing_bastion(
    pool: &SqlitePool,
    bastion_id: &str,
) -> Result<Vec<String>, String> {
    let rows = sqlx::query(
        "SELECT id
         FROM connections
         WHERE ssh_tunnel_enabled = 1 AND ssh_tunnel_bastion_server_id = ?
         ORDER BY id",
    )
    .bind(bastion_id)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(rows.into_iter().map(|row| row.get("id")).collect())
}

pub async fn update_bastion_host_key_fingerprint(
    pool: &SqlitePool,
    bastion_id: &str,
    fingerprint: Option<&str>,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE bastion_servers
         SET host_key_fingerprint = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(fingerprint)
    .bind(now())
    .bind(bastion_id)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}
