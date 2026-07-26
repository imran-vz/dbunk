//! Persistence for Managed Server records (ADR-0019).

use std::str::FromStr;

use sqlx::{Row, SqlitePool};

use crate::{DatabaseEngine, ManagedServer};

use super::i64_to_u16;

fn row_to_managed(row: sqlx::sqlite::SqliteRow) -> Result<ManagedServer, String> {
    let engine: String = row.get("engine");
    Ok(ManagedServer {
        id: row.get("id"),
        name: row.get("name"),
        engine: DatabaseEngine::from_str(&engine)?,
        version: row.get("version"),
        port: i64_to_u16(row.get("port")),
        container_name: row.get("container_name"),
        volume_name: row.get("volume_name"),
        database: row.get("database_name"),
        user: row.get("user_name"),
        connection_id: row.get("connection_id"),
        created_at: row.get("created_at"),
    })
}

const SELECT_COLUMNS: &str = "id, name, engine, version, port, container_name, volume_name,
     database_name, user_name, connection_id, created_at";

pub async fn read_managed_servers(pool: &SqlitePool) -> Result<Vec<ManagedServer>, String> {
    let rows = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM managed_servers ORDER BY name COLLATE NOCASE ASC"
    ))
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    rows.into_iter().map(row_to_managed).collect()
}

pub async fn read_managed_server_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<ManagedServer>, String> {
    let row = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM managed_servers WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    row.map(row_to_managed).transpose()
}

pub async fn read_managed_server_by_connection_id(
    pool: &SqlitePool,
    connection_id: &str,
) -> Result<Option<ManagedServer>, String> {
    let row = sqlx::query(&format!(
        "SELECT {SELECT_COLUMNS} FROM managed_servers WHERE connection_id = ?"
    ))
    .bind(connection_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    row.map(row_to_managed).transpose()
}

pub async fn upsert_managed_server(
    pool: &SqlitePool,
    server: &ManagedServer,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO managed_servers (
            id, name, engine, version, port, container_name, volume_name,
            database_name, user_name, connection_id, created_at
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            engine = excluded.engine,
            version = excluded.version,
            port = excluded.port,
            container_name = excluded.container_name,
            volume_name = excluded.volume_name,
            database_name = excluded.database_name,
            user_name = excluded.user_name,
            connection_id = excluded.connection_id",
    )
    .bind(&server.id)
    .bind(&server.name)
    .bind(server.engine.as_str())
    .bind(&server.version)
    .bind(i64::from(server.port))
    .bind(&server.container_name)
    .bind(&server.volume_name)
    .bind(&server.database)
    .bind(&server.user)
    .bind(&server.connection_id)
    .bind(&server.created_at)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn delete_managed_server(pool: &SqlitePool, id: &str) -> Result<bool, String> {
    let result = sqlx::query("DELETE FROM managed_servers WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(result.rows_affected() > 0)
}

/// Ports already claimed by managed servers, so two provisions can't
/// race onto the same host port even while one container is stopped.
pub async fn claimed_ports(pool: &SqlitePool) -> Result<Vec<u16>, String> {
    let rows = sqlx::query("SELECT port FROM managed_servers")
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| i64_to_u16(row.get("port")))
        .collect())
}
