//! DDL execution, DDL export, pg_dump, pg_restore, materialized view refresh.

use std::collections::BTreeSet;
use std::process::{Command, Stdio};
use std::time::Instant;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use sqlx::{Executor, Row};

use crate::{
    quote_double, ExecuteDdlResult, ExportDdlResult, PgDumpResult, PgRestoreResult,
    StoredConnection,
};

use super::{connect, pg_connection};

fn pg_tool_command(connection: &StoredConnection, binary: &str) -> Result<Command, String> {
    let connection = pg_connection(connection)?;
    let mut command = Command::new(binary);
    let port = if connection.port == 0 {
        5432
    } else {
        connection.port
    };
    command
        .arg("--host")
        .arg(&connection.host)
        .arg("--port")
        .arg(port.to_string())
        .arg("--username")
        .arg(&connection.user)
        .arg("--dbname")
        .arg(&connection.database)
        .env("PGPASSWORD", &connection.password)
        .env(
            "PGSSLMODE",
            if connection.ssl { "prefer" } else { "disable" },
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

fn command_error(binary: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("{binary} exited with status {}", output.status)
    } else {
        stderr
    }
}

async fn export_relation_ddl(
    conn: &mut sqlx::postgres::PgConnection,
    schema: &str,
    table: &str,
) -> Result<String, String> {
    let relkind: Option<String> = sqlx::query_scalar(
        r#"
        SELECT c.relkind::text
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = $1 AND c.relname = $2
          AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;
    let Some(relkind) = relkind else {
        return Err(format!("relation {schema}.{table} was not found"));
    };
    let qualified = format!("{}.{}", quote_double(schema), quote_double(table));
    if relkind == "v" || relkind == "m" {
        let view_sql: String = sqlx::query_scalar(
            r#"
            SELECT pg_get_viewdef(c.oid, true)
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = $1 AND c.relname = $2
            "#,
        )
        .bind(schema)
        .bind(table)
        .fetch_one(&mut *conn)
        .await
        .map_err(|error| error.to_string())?;
        let prefix = if relkind == "m" {
            "CREATE MATERIALIZED VIEW"
        } else {
            "CREATE VIEW"
        };
        return Ok(format!("{prefix} {qualified} AS\n{view_sql};"));
    }

    let columns = sqlx::query(
        r#"
        SELECT a.attname,
               format_type(a.atttypid, a.atttypmod) AS data_type,
               a.attnotnull,
               pg_get_expr(d.adbin, d.adrelid) AS default_expr
        FROM pg_attribute a
        JOIN pg_class c ON c.oid = a.attrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
        WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped
        ORDER BY a.attnum
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    let mut definitions = Vec::new();
    for column in columns {
        let name: String = column
            .try_get("attname")
            .map_err(|error| error.to_string())?;
        let data_type: String = column
            .try_get("data_type")
            .map_err(|error| error.to_string())?;
        let not_null: bool = column
            .try_get("attnotnull")
            .map_err(|error| error.to_string())?;
        let default_expr: Option<String> = column
            .try_get("default_expr")
            .map_err(|error| error.to_string())?;
        let mut line = format!("  {} {}", quote_double(&name), data_type);
        if let Some(default_expr) = default_expr {
            line.push_str(" DEFAULT ");
            line.push_str(&default_expr);
        }
        if not_null {
            line.push_str(" NOT NULL");
        }
        definitions.push(line);
    }

    let constraints = sqlx::query(
        r#"
        SELECT conname, pg_get_constraintdef(con.oid, true) AS definition
        FROM pg_constraint con
        JOIN pg_class c ON c.oid = con.conrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = $1 AND c.relname = $2
        ORDER BY con.contype, con.conname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;
    for constraint in constraints {
        let name: String = constraint
            .try_get("conname")
            .map_err(|error| error.to_string())?;
        let definition: String = constraint
            .try_get("definition")
            .map_err(|error| error.to_string())?;
        definitions.push(format!(
            "  CONSTRAINT {} {}",
            quote_double(&name),
            definition
        ));
    }

    let mut ddl = format!(
        "CREATE TABLE {qualified} (\n{}\n);",
        definitions.join(",\n")
    );
    let indexes = sqlx::query_scalar::<_, String>(
        r#"
        SELECT i.indexdef
        FROM pg_indexes i
        JOIN pg_class idx ON idx.relname = i.indexname
        JOIN pg_namespace ns ON ns.nspname = i.schemaname AND ns.oid = idx.relnamespace
        WHERE i.schemaname = $1 AND i.tablename = $2
          AND NOT EXISTS (
            SELECT 1 FROM pg_constraint con WHERE con.conindid = idx.oid
          )
        ORDER BY i.indexname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;
    for index in indexes {
        ddl.push('\n');
        ddl.push_str(index.trim_end_matches(';'));
        ddl.push(';');
    }
    Ok(ddl)
}

async fn relation_names(
    conn: &mut sqlx::postgres::PgConnection,
    schema: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    sqlx::query_as::<_, (String, String)>(
        r#"
        SELECT n.nspname, c.relname
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
          AND ($1::text IS NULL OR n.nspname = $1)
          AND n.nspname NOT IN ('pg_catalog', 'information_schema')
          AND n.nspname NOT LIKE 'pg_toast%'
        ORDER BY n.nspname, c.relname
        "#,
    )
    .bind(schema)
    .fetch_all(conn)
    .await
    .map_err(|error| error.to_string())
}

pub async fn export_ddl(
    connection: &StoredConnection,
    scope: &str,
    schema: Option<&str>,
    table: Option<&str>,
) -> Result<ExportDdlResult, String> {
    let mut conn = connect(connection).await?;
    let start = Instant::now();
    let sql = match scope {
        "table" => {
            let schema = schema.ok_or_else(|| "DDL export requires a schema".to_string())?;
            let table = table.ok_or_else(|| "table DDL export requires a table".to_string())?;
            export_relation_ddl(&mut conn, schema, table).await?
        }
        "schema" => {
            let schema = schema.ok_or_else(|| "schema DDL export requires a schema".to_string())?;
            let names = relation_names(&mut conn, Some(schema)).await?;
            let mut parts = Vec::with_capacity(names.len() + 1);
            parts.push(format!(
                "CREATE SCHEMA IF NOT EXISTS {};",
                quote_double(schema)
            ));
            for (schema, table) in names {
                parts.push(export_relation_ddl(&mut conn, &schema, &table).await?);
            }
            parts.join("\n\n")
        }
        "database" => {
            let names = relation_names(&mut conn, None).await?;
            let mut schemas = BTreeSet::new();
            for (schema, _) in &names {
                if schema != "public" {
                    schemas.insert(schema.clone());
                }
            }
            let mut parts = Vec::with_capacity(names.len() + schemas.len());
            for schema in schemas {
                parts.push(format!(
                    "CREATE SCHEMA IF NOT EXISTS {};",
                    quote_double(&schema)
                ));
            }
            for (schema, table) in names {
                parts.push(export_relation_ddl(&mut conn, &schema, &table).await?);
            }
            parts.join("\n\n")
        }
        _ => return Err("unsupported DDL export scope".to_string()),
    };
    Ok(ExportDdlResult {
        sql,
        runtime_ms: start.elapsed().as_millis() as u64,
    })
}

pub async fn run_pg_dump(
    connection: &StoredConnection,
    scope: &str,
    schema: Option<&str>,
    table: Option<&str>,
    format: &str,
) -> Result<PgDumpResult, String> {
    let start = Instant::now();
    let mut command = pg_tool_command(connection, "pg_dump")?;
    let extension = match format {
        "plain" => {
            command.arg("--format").arg("plain");
            "sql"
        }
        "custom" => {
            command.arg("--format").arg("custom");
            "dump"
        }
        _ => return Err("pg_dump format must be plain or custom".to_string()),
    };
    match scope {
        "database" => {}
        "schema" => {
            let schema = schema.ok_or_else(|| "schema dump requires a schema".to_string())?;
            command.arg("--schema").arg(schema);
        }
        "table" => {
            let schema = schema.ok_or_else(|| "table dump requires a schema".to_string())?;
            let table = table.ok_or_else(|| "table dump requires a table".to_string())?;
            command
                .arg("--table")
                .arg(format!("{}.{}", quote_double(schema), quote_double(table)));
        }
        _ => return Err("unsupported pg_dump scope".to_string()),
    }
    let output = command.output().map_err(|error| {
        format!("failed to run pg_dump; make sure PostgreSQL client tools are installed: {error}")
    })?;
    if !output.status.success() {
        return Err(command_error("pg_dump", &output));
    }
    Ok(PgDumpResult {
        data_base64: B64.encode(output.stdout),
        extension: extension.to_string(),
        runtime_ms: start.elapsed().as_millis() as u64,
    })
}

pub async fn run_pg_restore(
    connection: &StoredConnection,
    data_base64: &str,
    format: &str,
    clean: bool,
) -> Result<PgRestoreResult, String> {
    let start = Instant::now();
    let data = B64
        .decode(data_base64)
        .map_err(|error| format!("restore payload is not valid base64: {error}"))?;
    let output = if format == "plain" {
        let mut command = pg_tool_command(connection, "psql")?;
        if clean {
            command.arg("--single-transaction");
        }
        command.stdin(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to run psql: {error}"))?;
        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write;
            stdin.write_all(&data).map_err(|error| error.to_string())?;
        }
        child
            .wait_with_output()
            .map_err(|error| format!("failed to wait for psql: {error}"))?
    } else if format == "custom" {
        let input = tempfile::NamedTempFile::new().map_err(|error| error.to_string())?;
        std::fs::write(input.path(), data).map_err(|error| error.to_string())?;
        let mut command = pg_tool_command(connection, "pg_restore")?;
        if clean {
            command.arg("--clean").arg("--if-exists");
        }
        command.arg(input.path());
        command.output().map_err(|error| {
            format!("failed to run pg_restore; make sure PostgreSQL client tools are installed: {error}")
        })?
    } else {
        return Err("restore format must be plain or custom".to_string());
    };
    if !output.status.success() {
        return Err(command_error(
            if format == "plain" {
                "psql"
            } else {
                "pg_restore"
            },
            &output,
        ));
    }
    Ok(PgRestoreResult {
        runtime_ms: start.elapsed().as_millis() as u64,
    })
}

pub async fn execute_ddl(
    connection: &StoredConnection,
    sql: &str,
) -> Result<ExecuteDdlResult, String> {
    let mut conn = connect(connection).await?;
    let start = Instant::now();

    if let Err(error) = conn.execute("BEGIN").await {
        return Err(error.to_string());
    }
    match conn.execute(sql).await {
        Ok(_) => {
            if let Err(error) = conn.execute("COMMIT").await {
                let _ = conn.execute("ROLLBACK").await;
                return Err(error.to_string());
            }
            Ok(ExecuteDdlResult {
                runtime_ms: start.elapsed().as_millis() as u64,
            })
        }
        Err(error) => {
            let _ = conn.execute("ROLLBACK").await;
            Err(error.to_string())
        }
    }
}

pub async fn refresh_materialized_view(
    connection: &StoredConnection,
    schema: &str,
    view: &str,
    concurrently: bool,
) -> Result<ExecuteDdlResult, String> {
    let mut conn = connect(connection).await?;
    let start = Instant::now();
    let qualified = format!("{}.{}", quote_double(schema), quote_double(view));
    let sql = if concurrently {
        format!("REFRESH MATERIALIZED VIEW CONCURRENTLY {qualified}")
    } else {
        format!("REFRESH MATERIALIZED VIEW {qualified}")
    };
    conn.execute(sql.as_str())
        .await
        .map_err(|error| error.to_string())?;
    Ok(ExecuteDdlResult {
        runtime_ms: start.elapsed().as_millis() as u64,
    })
}
