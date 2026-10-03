//! DDL execution, DDL export, and materialized view refresh.

use std::collections::BTreeSet;
use std::time::Instant;

use sqlx::{Executor, Row};

use crate::{quote_double, ExecuteDdlResult, ExportDdlResult, StoredConnection};

use super::connect;

pub(crate) async fn export_relation_ddl(
    conn: &mut sqlx::postgres::PgConnection,
    schema: &str,
    table: &str,
) -> Result<String, String> {
    let relation = sqlx::query(
        r#"
        SELECT c.relkind::text AS relkind,
               CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END AS partition_key,
               CASE WHEN c.relispartition THEN pg_get_expr(c.relpartbound, c.oid) END AS partition_bound,
               parent_namespace.nspname AS parent_schema,
               parent.relname AS parent_name
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        LEFT JOIN pg_inherits inherits ON c.relispartition AND inherits.inhrelid = c.oid
        LEFT JOIN pg_class parent ON parent.oid = inherits.inhparent
        LEFT JOIN pg_namespace parent_namespace ON parent_namespace.oid = parent.relnamespace
        WHERE n.nspname = $1 AND c.relname = $2
          AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;
    let Some(relation) = relation else {
        return Err(format!("relation {schema}.{table} was not found"));
    };
    let relkind: String = relation
        .try_get("relkind")
        .map_err(|error| error.to_string())?;
    let partition_key: Option<String> = relation
        .try_get("partition_key")
        .map_err(|error| error.to_string())?;
    let partition_bound: Option<String> = relation
        .try_get("partition_bound")
        .map_err(|error| error.to_string())?;
    let parent_schema: Option<String> = relation
        .try_get("parent_schema")
        .map_err(|error| error.to_string())?;
    let parent_name: Option<String> = relation
        .try_get("parent_name")
        .map_err(|error| error.to_string())?;
    let partition_parent = parent_schema.zip(parent_name);
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
    // A partition inherits its columns from the parent; only constraints
    // declared on the partition itself can appear in PARTITION OF.
    let is_partition = partition_parent.is_some();
    for column in columns {
        if is_partition {
            break;
        }
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
        WHERE n.nspname = $1 AND c.relname = $2 AND con.conislocal
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

    let mut ddl = match partition_parent {
        Some((parent_schema, parent_name)) => {
            let parent = format!(
                "{}.{}",
                quote_double(&parent_schema),
                quote_double(&parent_name)
            );
            let mut ddl = format!("CREATE TABLE {qualified} PARTITION OF {parent}");
            if !definitions.is_empty() {
                ddl.push_str(&format!(" (\n{}\n)", definitions.join(",\n")));
            }
            ddl.push(' ');
            ddl.push_str(partition_bound.as_deref().unwrap_or("DEFAULT"));
            ddl
        }
        None => format!("CREATE TABLE {qualified} (\n{}\n)", definitions.join(",\n")),
    };
    if let Some(partition_key) = partition_key {
        ddl.push_str(" PARTITION BY ");
        ddl.push_str(&partition_key);
    }
    ddl.push(';');
    // Indexes attached from a partitioned parent are recorded in pg_inherits
    // and are recreated by the parent's index, not by the partition.
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
          AND NOT EXISTS (
            SELECT 1 FROM pg_inherits inherited WHERE inherited.inhrelid = idx.oid
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
