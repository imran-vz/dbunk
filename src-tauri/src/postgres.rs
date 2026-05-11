//! PostgreSQL-specific implementations of every command operation.
//!
//! Per ADR-0001 PostgreSQL is the reference engine — every connection-level
//! command (run_query, table structure, DDL, edits, schema relationships,
//! overview stats) has a full PG implementation here, with other engines
//! either stubbed or routed through cross-engine sqlx-Any helpers in
//! `lib.rs`.
//!
//! ## Public surface
//!
//! Every function takes a `&StoredConnection` plus operation-specific args
//! and returns the same result types the dispatchers expose to the
//! frontend. The dispatcher in `lib.rs` is responsible for engine
//! pattern-matching, activity tracking, and error mapping for non-PG
//! engines — this module assumes its caller has already decided "yes, run
//! this against PG."
//!
//! ## Internals
//!
//! - [`connect`] is the single seam for `PgConnectOptions` + auth + future
//!   PG concerns (pooling, statement timeouts, `application_name`).
//! - [`value_to_string`] / [`row_to_strings`] coerce native PG row values
//!   into the string-only shape the frontend expects. The native driver
//!   handles richer Postgres types (UUID, timestamps, JSON, bytes) than
//!   sqlx-Any can represent.
//! - The pure SQL builders (`build_update`, `build_insert`, `build_delete`)
//!   compose parameterized statements with PG's `$1..$N` placeholder
//!   convention. They are the test-coverage opportunity flagged as
//!   deepening #7.

use std::collections::HashMap;
use std::time::Instant;

use sqlx::{
    postgres::{PgConnectOptions, PgConnection, PgRow},
    Column, Connection, Executor, Row,
};

use crate::{
    bytes_to_hex, dispatch::should_fetch_rows, quote_double, CellEdit, CellEditKeyValue,
    ColumnInfo, CommitCellEditsResult, ConstraintInfo, DatabaseOverviewStats, DeleteRowsResult,
    ExecuteDdlResult, ForeignKeyInfo, IndexInfo, InsertRowResult, QueryResult, SchemaForeignKey,
    SchemaRelationships, SchemaTableColumn, SchemaTableNode, StoredConnection,
    StructureCapabilities, TableStructure,
};

// ---------------------------------------------------------------------------
// Connection
// ---------------------------------------------------------------------------

/// Open a `PgConnection` from a stored Connection record.
///
/// Defaults the port to 5432 when the stored value is `0` (the sentinel we
/// use for "use the engine default" — set when the user leaves the port
/// field blank). This is the single seam where future PG connection
/// concerns plug in: pooling, TLS modes, statement timeouts,
/// `application_name`, etc.
async fn connect(connection: &StoredConnection) -> Result<PgConnection, String> {
    let mut options = PgConnectOptions::new()
        .host(&connection.host)
        .username(&connection.user)
        .database(&connection.database)
        .port(if connection.port == 0 {
            5432
        } else {
            connection.port
        });

    if !connection.password.is_empty() {
        options = options.password(&connection.password);
    }

    PgConnection::connect_with(&options)
        .await
        .map_err(|error| error.to_string())
}

// ---------------------------------------------------------------------------
// Row → string coercion
// ---------------------------------------------------------------------------

/// Convert a single PG cell to its string display form.
///
/// PG's native driver carries richer type information than sqlx-Any, so we
/// probe known column types via `try_get::<Option<T>, _>` and fall through
/// to "NULL" if nothing matches. Order matters: integer `try_get` succeeds
/// for numeric-looking text on some drivers, so we try strings first.
fn value_to_string(row: &PgRow, index: usize) -> String {
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return value.unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<i32>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<i16>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<f32>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<bool>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(index) {
        return value
            .map(|v| v.to_rfc3339())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<chrono::NaiveDateTime>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<chrono::NaiveDate>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<chrono::NaiveTime>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<uuid::Uuid>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<serde_json::Value>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return value
            .map(|bytes| bytes_to_hex(&bytes))
            .unwrap_or_else(|| "NULL".to_string());
    }
    "NULL".to_string()
}

fn row_to_strings(row: &PgRow) -> Vec<String> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(index, _)| value_to_string(row, index))
        .collect()
}

// ---------------------------------------------------------------------------
// Run query
// ---------------------------------------------------------------------------

/// Run an ad-hoc query against PG using the native driver.
///
/// Selects (and other row-returning statements) come back with column
/// metadata + row strings; everything else returns `rows_affected`. Routing
/// SELECT vs DML lives in `should_fetch_rows`.
pub async fn run_query(connection: &StoredConnection, query: &str) -> Result<QueryResult, String> {
    let mut conn = connect(connection).await?;
    let start = Instant::now();

    if should_fetch_rows(query) {
        let rows = sqlx::query(query)
            .fetch_all(&mut conn)
            .await
            .map_err(|error| error.to_string())?;
        let columns = rows
            .first()
            .map(|row| {
                row.columns()
                    .iter()
                    .map(|column| column.name().to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let values = rows.iter().map(row_to_strings).collect::<Vec<_>>();
        let runtime_ms = start.elapsed().as_millis() as u64;
        Ok(QueryResult {
            columns,
            rows: values,
            runtime_ms,
            row_count: rows.len() as u64,
        })
    } else {
        let result = sqlx::query(query)
            .execute(&mut conn)
            .await
            .map_err(|error| error.to_string())?;
        let runtime_ms = start.elapsed().as_millis() as u64;
        Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            runtime_ms,
            row_count: result.rows_affected(),
        })
    }
}

// ---------------------------------------------------------------------------
// Pure SQL builders
// ---------------------------------------------------------------------------

fn build_update(
    schema: &str,
    table: &str,
    set: &[CellEditKeyValue],
    identity: &[CellEditKeyValue],
) -> (String, Vec<Option<String>>) {
    let qualified = format!("{}.{}", quote_double(schema), quote_double(table));
    let mut params: Vec<Option<String>> = Vec::with_capacity(set.len() + identity.len());
    let set_clause: Vec<String> = set
        .iter()
        .enumerate()
        .map(|(i, kv)| {
            params.push(kv.value.clone());
            format!("{} = ${}", quote_double(&kv.column), i + 1)
        })
        .collect();
    let where_clause: Vec<String> = identity
        .iter()
        .map(|kv| {
            params.push(kv.value.clone());
            // Identity values are never NULL in practice (NULLs cannot be
            // matched with `=`), but we still bind defensively. If a caller
            // ever sends NULL we fall back to `IS NULL` so the row at least
            // has a chance of matching.
            if kv.value.is_none() {
                // Pop the just-pushed None — it isn't used because we emit
                // `IS NULL` directly. Keep parameter numbering consistent.
                params.pop();
                format!("{} IS NULL", quote_double(&kv.column))
            } else {
                format!("{} = ${}", quote_double(&kv.column), params.len())
            }
        })
        .collect();
    let sql = format!(
        "UPDATE {} SET {} WHERE {}",
        qualified,
        set_clause.join(", "),
        where_clause.join(" AND ")
    );
    (sql, params)
}

fn build_insert(
    schema: &str,
    table: &str,
    values: &[CellEditKeyValue],
) -> (String, Vec<Option<String>>) {
    let qualified = format!("{}.{}", quote_double(schema), quote_double(table));
    let mut params: Vec<Option<String>> = Vec::with_capacity(values.len());
    let column_list: Vec<String> = values.iter().map(|kv| quote_double(&kv.column)).collect();
    let placeholders: Vec<String> = values
        .iter()
        .enumerate()
        .map(|(i, kv)| {
            params.push(kv.value.clone());
            format!("${}", i + 1)
        })
        .collect();
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        qualified,
        column_list.join(", "),
        placeholders.join(", ")
    );
    (sql, params)
}

fn build_delete(
    schema: &str,
    table: &str,
    identity: &[CellEditKeyValue],
) -> (String, Vec<Option<String>>) {
    let qualified = format!("{}.{}", quote_double(schema), quote_double(table));
    let mut params: Vec<Option<String>> = Vec::with_capacity(identity.len());
    let where_clause: Vec<String> = identity
        .iter()
        .map(|kv| {
            // NULL identity column → IS NULL (matches commit_cell_edits
            // behavior). Otherwise bind a positional parameter.
            if kv.value.is_none() {
                format!("{} IS NULL", quote_double(&kv.column))
            } else {
                params.push(kv.value.clone());
                format!("{} = ${}", quote_double(&kv.column), params.len())
            }
        })
        .collect();
    let sql = format!(
        "DELETE FROM {} WHERE {}",
        qualified,
        where_clause.join(" AND ")
    );
    (sql, params)
}

// ---------------------------------------------------------------------------
// DDL
// ---------------------------------------------------------------------------

pub async fn execute_ddl(
    connection: &StoredConnection,
    sql: &str,
) -> Result<ExecuteDdlResult, String> {
    let mut conn = connect(connection).await?;
    let start = Instant::now();

    // Wrap the entire DDL batch in an explicit transaction so a partial
    // failure leaves the schema unchanged. sqlx's PgConnection::execute
    // accepts multi-statement strings, which lets us send the generated
    // DDL as a single round-trip.
    if let Err(error) = conn.execute("BEGIN").await {
        return Err(error.to_string());
    }
    match conn.execute(sql).await {
        Ok(_) => {
            if let Err(error) = conn.execute("COMMIT").await {
                // Best-effort rollback if COMMIT fails (rare).
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

// ---------------------------------------------------------------------------
// Cell edits / inserts / deletes
// ---------------------------------------------------------------------------

pub async fn commit_cell_edits(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    edits: &[CellEdit],
) -> Result<CommitCellEditsResult, String> {
    let mut conn = connect(connection).await?;
    // sqlx's `Transaction` rolls back automatically on drop, so any `?`
    // return below acts as ROLLBACK without the manual clean-up that used to
    // litter every error path.
    let mut tx = conn.begin().await.map_err(|error| error.to_string())?;
    let start = Instant::now();
    let mut total_rows_affected: u64 = 0;

    for edit in edits {
        if edit.set.is_empty() {
            return Err("edit has no SET columns".to_string());
        }
        if edit.identity.is_empty() {
            return Err("edit has no identity columns".to_string());
        }
        let (sql, params) = build_update(schema, table, &edit.set, &edit.identity);
        let mut query = sqlx::query(&sql);
        for param in &params {
            query = query.bind(param.as_deref());
        }
        let result = query
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
        let affected = result.rows_affected();
        if affected == 0 {
            let identity_desc = edit
                .identity
                .iter()
                .map(|kv| format!("{}={}", kv.column, kv.value.as_deref().unwrap_or("NULL")))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!("row not found: {}", identity_desc));
        }
        total_rows_affected += affected;
    }

    tx.commit().await.map_err(|error| error.to_string())?;

    Ok(CommitCellEditsResult {
        rows_affected: total_rows_affected,
        runtime_ms: start.elapsed().as_millis() as u64,
        state: "committed".to_string(),
        database: String::new(),
        table: String::new(),
        mutation_ids: Vec::new(),
    })
}

pub async fn insert_row(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    values: &[CellEditKeyValue],
) -> Result<InsertRowResult, String> {
    let mut conn = connect(connection).await?;
    let mut tx = conn.begin().await.map_err(|error| error.to_string())?;
    let start = Instant::now();

    let (sql, params) = build_insert(schema, table, values);
    let mut query = sqlx::query(&sql);
    for param in &params {
        query = query.bind(param.as_deref());
    }
    let rows_affected = query
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?
        .rows_affected();

    tx.commit().await.map_err(|error| error.to_string())?;

    Ok(InsertRowResult {
        rows_affected,
        runtime_ms: start.elapsed().as_millis() as u64,
    })
}

pub async fn delete_rows(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    rows: &[Vec<CellEditKeyValue>],
) -> Result<DeleteRowsResult, String> {
    let mut conn = connect(connection).await?;
    let mut tx = conn.begin().await.map_err(|error| error.to_string())?;
    let start = Instant::now();
    let mut total_rows_affected: u64 = 0;

    for identity in rows {
        if identity.is_empty() {
            return Err("missing identity".to_string());
        }
        let (sql, params) = build_delete(schema, table, identity);
        let mut query = sqlx::query(&sql);
        for param in &params {
            query = query.bind(param.as_deref());
        }
        let result = query
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
        let affected = result.rows_affected();
        if affected == 0 {
            let identity_desc = identity
                .iter()
                .map(|kv| format!("{}={}", kv.column, kv.value.as_deref().unwrap_or("NULL")))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!("row not found: {}", identity_desc));
        }
        total_rows_affected += affected;
    }

    tx.commit().await.map_err(|error| error.to_string())?;

    Ok(DeleteRowsResult {
        rows_affected: total_rows_affected,
        runtime_ms: start.elapsed().as_millis() as u64,
        state: "committed".to_string(),
        database: String::new(),
        table: String::new(),
        mutation_ids: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Table structure
// ---------------------------------------------------------------------------

fn fk_action_label(code: &str) -> Option<String> {
    match code {
        "a" => Some("NO ACTION".to_string()),
        "r" => Some("RESTRICT".to_string()),
        "c" => Some("CASCADE".to_string()),
        "n" => Some("SET NULL".to_string()),
        "d" => Some("SET DEFAULT".to_string()),
        _ => None,
    }
}

fn constraint_kind(code: &str) -> &'static str {
    match code {
        "c" => "check",
        "u" => "unique",
        "x" => "exclusion",
        "p" => "primary key",
        "f" => "foreign key",
        _ => "constraint",
    }
}

pub async fn fetch_table_structure(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
) -> Result<TableStructure, String> {
    let mut conn = connect(connection).await?;

    // Primary key columns first so we can mark them on the columns list.
    let pk_rows = sqlx::query(
        r#"
        SELECT kcu.column_name::text AS column_name
        FROM information_schema.table_constraints tc
        JOIN information_schema.key_column_usage kcu
          ON tc.constraint_name = kcu.constraint_name
         AND tc.table_schema = kcu.table_schema
         AND tc.table_name = kcu.table_name
        WHERE tc.constraint_type = 'PRIMARY KEY'
          AND tc.table_schema = $1
          AND tc.table_name = $2
        ORDER BY kcu.ordinal_position
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let primary_key_cols: Vec<String> = pk_rows
        .iter()
        .map(|row| row.try_get::<String, _>("column_name").unwrap_or_default())
        .collect();

    // Columns
    let column_rows = sqlx::query(
        r#"
        SELECT column_name::text AS column_name,
               data_type::text AS data_type,
               udt_name::text AS udt_name,
               is_nullable::text AS is_nullable,
               column_default::text AS column_default,
               ordinal_position::int AS ordinal_position,
               character_maximum_length::int AS character_maximum_length,
               numeric_precision::int AS numeric_precision,
               numeric_scale::int AS numeric_scale
        FROM information_schema.columns
        WHERE table_schema = $1 AND table_name = $2
        ORDER BY ordinal_position
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let mut columns = Vec::with_capacity(column_rows.len());
    for row in column_rows {
        let name: String = row.try_get("column_name").unwrap_or_default();
        let data_type: String = row.try_get("data_type").unwrap_or_default();
        let udt_name: String = row.try_get("udt_name").unwrap_or_default();
        let is_nullable: String = row.try_get("is_nullable").unwrap_or_default();
        let default_value: Option<String> = row.try_get("column_default").ok();
        let ordinal_position: i32 = row.try_get("ordinal_position").unwrap_or(0);
        let char_len: Option<i32> = row.try_get("character_maximum_length").ok();
        let numeric_precision: Option<i32> = row.try_get("numeric_precision").ok();
        let numeric_scale: Option<i32> = row.try_get("numeric_scale").ok();

        // Build a richer rendered type. Trust `data_type` from information_schema
        // for the high-level family and fall back to `udt_name` for specifics.
        let rendered_type = match data_type.as_str() {
            "character varying" => match char_len {
                Some(len) => format!("varchar({})", len),
                None => "varchar".to_string(),
            },
            "character" => match char_len {
                Some(len) => format!("char({})", len),
                None => "char".to_string(),
            },
            "numeric" => match (numeric_precision, numeric_scale) {
                (Some(p), Some(s)) if s > 0 => format!("numeric({},{})", p, s),
                (Some(p), _) => format!("numeric({})", p),
                _ => "numeric".to_string(),
            },
            "USER-DEFINED" | "ARRAY" => udt_name.clone(),
            other => other.to_string(),
        };

        columns.push(ColumnInfo {
            name: name.clone(),
            data_type: rendered_type,
            nullable: is_nullable.eq_ignore_ascii_case("YES"),
            default_value,
            is_primary_key: primary_key_cols.iter().any(|pk| pk == &name),
            ordinal_position,
        });
    }

    // Foreign keys
    let fk_rows = sqlx::query(
        r#"
        SELECT con.conname::text AS name,
               nsp_ref.nspname::text AS referenced_schema,
               cls_ref.relname::text AS referenced_table,
               con.confupdtype::text AS on_update,
               con.confdeltype::text AS on_delete,
               (
                   SELECT array_agg(att.attname::text ORDER BY u.ord)
                   FROM unnest(con.conkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = con.conrelid AND att.attnum = u.attnum
               ) AS columns,
               (
                   SELECT array_agg(att.attname::text ORDER BY u.ord)
                   FROM unnest(con.confkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = con.confrelid AND att.attnum = u.attnum
               ) AS referenced_columns
        FROM pg_constraint con
        JOIN pg_class cls ON cls.oid = con.conrelid
        JOIN pg_namespace nsp ON nsp.oid = cls.relnamespace
        JOIN pg_class cls_ref ON cls_ref.oid = con.confrelid
        JOIN pg_namespace nsp_ref ON nsp_ref.oid = cls_ref.relnamespace
        WHERE con.contype = 'f'
          AND nsp.nspname = $1
          AND cls.relname = $2
        ORDER BY con.conname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let foreign_keys = fk_rows
        .into_iter()
        .map(|row| {
            let name: String = row.try_get("name").unwrap_or_default();
            let referenced_schema: String = row.try_get("referenced_schema").unwrap_or_default();
            let referenced_table: String = row.try_get("referenced_table").unwrap_or_default();
            let on_update_code: String = row.try_get("on_update").unwrap_or_default();
            let on_delete_code: String = row.try_get("on_delete").unwrap_or_default();
            let columns: Vec<String> = row.try_get("columns").unwrap_or_default();
            let referenced_columns: Vec<String> =
                row.try_get("referenced_columns").unwrap_or_default();
            ForeignKeyInfo {
                name,
                columns,
                referenced_schema,
                referenced_table,
                referenced_columns,
                on_update: fk_action_label(&on_update_code),
                on_delete: fk_action_label(&on_delete_code),
            }
        })
        .collect::<Vec<_>>();

    // Indexes
    let index_rows = sqlx::query(
        r#"
        SELECT i.relname::text AS index_name,
               ix.indisunique AS is_unique,
               ix.indisprimary AS is_primary,
               am.amname::text AS method,
               (
                   SELECT array_agg(att.attname::text ORDER BY u.ord)
                   FROM unnest(ix.indkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = ix.indrelid AND att.attnum = u.attnum
               ) AS columns
        FROM pg_class t
        JOIN pg_index ix ON t.oid = ix.indrelid
        JOIN pg_class i ON i.oid = ix.indexrelid
        JOIN pg_am am ON am.oid = i.relam
        JOIN pg_namespace n ON n.oid = t.relnamespace
        WHERE n.nspname = $1 AND t.relname = $2
        ORDER BY i.relname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let indexes = index_rows
        .into_iter()
        .map(|row| IndexInfo {
            name: row.try_get("index_name").unwrap_or_default(),
            columns: row.try_get("columns").unwrap_or_default(),
            is_unique: row.try_get("is_unique").unwrap_or(false),
            is_primary: row.try_get("is_primary").unwrap_or(false),
            method: row.try_get::<String, _>("method").ok(),
        })
        .collect::<Vec<_>>();

    // Other constraints (CHECK, UNIQUE, EXCLUDE) — skip primary key and foreign key here.
    let constraint_rows = sqlx::query(
        r#"
        SELECT con.conname::text AS name,
               con.contype::text AS contype,
               pg_get_constraintdef(con.oid) AS definition
        FROM pg_constraint con
        JOIN pg_class cls ON cls.oid = con.conrelid
        JOIN pg_namespace nsp ON nsp.oid = cls.relnamespace
        WHERE nsp.nspname = $1
          AND cls.relname = $2
          AND con.contype IN ('c', 'u', 'x')
        ORDER BY con.conname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let constraints = constraint_rows
        .into_iter()
        .map(|row| {
            let kind_code: String = row.try_get("contype").unwrap_or_default();
            ConstraintInfo {
                name: row.try_get("name").unwrap_or_default(),
                kind: constraint_kind(&kind_code).to_string(),
                definition: row.try_get("definition").unwrap_or_default(),
            }
        })
        .collect::<Vec<_>>();

    let primary_key = if primary_key_cols.is_empty() {
        None
    } else {
        Some(primary_key_cols)
    };

    let has_primary_key = primary_key.is_some();

    Ok(TableStructure {
        columns,
        primary_key,
        foreign_keys,
        indexes,
        constraints,
        capabilities: StructureCapabilities {
            columns: true,
            primary_key: true,
            foreign_keys: true,
            indexes: true,
            constraints: true,
            can_insert_rows: true,
            // Row-level UPDATE/DELETE need a row identity; without a PK
            // we'd be matching on full-row values which is dangerous.
            // The frontend already gates on row-identity availability;
            // this flag matches that policy.
            can_update_rows: has_primary_key,
            can_delete_rows: has_primary_key,
            can_alter_schema: true,
            update_semantics: "synchronous".to_string(),
            uniqueness_guarantee: if has_primary_key {
                "exact".to_string()
            } else {
                "best-effort".to_string()
            },
        },
        table_engine: None,
        partition_by: None,
        sample_by: None,
    })
}

// ---------------------------------------------------------------------------
// Schema relationships
// ---------------------------------------------------------------------------

pub async fn fetch_schema_relationships(
    connection: &StoredConnection,
    schema: &str,
) -> Result<SchemaRelationships, String> {
    let mut conn = connect(connection).await?;

    // Tables in the requested schema. Views are intentionally excluded.
    let table_rows = sqlx::query(
        r#"
        SELECT t.table_name::text AS name
        FROM information_schema.tables t
        WHERE t.table_schema = $1
          AND t.table_type = 'BASE TABLE'
        ORDER BY t.table_name
        "#,
    )
    .bind(schema)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let column_rows = sqlx::query(
        r#"
        SELECT c.table_name::text AS table_name,
               c.column_name::text AS column_name,
               c.data_type::text AS data_type,
               c.udt_name::text AS udt_name,
               c.is_nullable::text AS is_nullable,
               c.ordinal_position::int AS ordinal_position,
               (kcu.column_name IS NOT NULL) AS is_primary_key
        FROM information_schema.columns c
        LEFT JOIN information_schema.table_constraints tc
          ON tc.constraint_type = 'PRIMARY KEY'
         AND tc.table_schema = c.table_schema
         AND tc.table_name = c.table_name
        LEFT JOIN information_schema.key_column_usage kcu
          ON kcu.constraint_name = tc.constraint_name
         AND kcu.table_schema = tc.table_schema
         AND kcu.table_name = tc.table_name
         AND kcu.column_name = c.column_name
        WHERE c.table_schema = $1
        ORDER BY c.table_name, c.ordinal_position
        "#,
    )
    .bind(schema)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let mut columns_by_table: HashMap<String, Vec<SchemaTableColumn>> = HashMap::new();
    for row in column_rows {
        let table_name: String = row.try_get("table_name").unwrap_or_default();
        let data_type: String = row.try_get("data_type").unwrap_or_default();
        let udt_name: String = row.try_get("udt_name").unwrap_or_default();
        let rendered_type = match data_type.as_str() {
            "USER-DEFINED" | "ARRAY" => udt_name,
            other => other.to_string(),
        };
        columns_by_table
            .entry(table_name)
            .or_default()
            .push(SchemaTableColumn {
                name: row.try_get("column_name").unwrap_or_default(),
                data_type: rendered_type,
                nullable: row
                    .try_get::<String, _>("is_nullable")
                    .unwrap_or_default()
                    .eq_ignore_ascii_case("YES"),
                is_primary_key: row.try_get("is_primary_key").unwrap_or(false),
                ordinal_position: row.try_get("ordinal_position").unwrap_or(0),
            });
    }

    let tables: Vec<SchemaTableNode> = table_rows
        .into_iter()
        .map(|row| {
            let name: String = row.try_get("name").unwrap_or_default();
            let columns = columns_by_table.remove(&name).unwrap_or_default();
            SchemaTableNode {
                schema: schema.to_string(),
                name,
                column_count: columns.len() as u32,
                columns,
            }
        })
        .collect();

    // Foreign keys originating in the requested schema. We aggregate the
    // participating columns into arrays directly in SQL using the constraint
    // catalog so multi-column FKs collapse to a single row. The referenced
    // table may live in a different schema; we surface that on the result so
    // the UI can render cross-schema edges.
    let fk_rows = sqlx::query(
        r#"
        SELECT con.conname::text AS name,
               nsp.nspname::text AS from_schema,
               cls.relname::text AS from_table,
               nsp_ref.nspname::text AS to_schema,
               cls_ref.relname::text AS to_table,
               (
                   SELECT array_agg(att.attname::text ORDER BY u.ord)
                   FROM unnest(con.conkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = con.conrelid AND att.attnum = u.attnum
               ) AS from_columns,
               (
                   SELECT array_agg(att.attname::text ORDER BY u.ord)
                   FROM unnest(con.confkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = con.confrelid AND att.attnum = u.attnum
               ) AS to_columns
        FROM pg_constraint con
        JOIN pg_class cls ON cls.oid = con.conrelid
        JOIN pg_namespace nsp ON nsp.oid = cls.relnamespace
        JOIN pg_class cls_ref ON cls_ref.oid = con.confrelid
        JOIN pg_namespace nsp_ref ON nsp_ref.oid = cls_ref.relnamespace
        WHERE con.contype = 'f'
          AND nsp.nspname = $1
        ORDER BY con.conname
        "#,
    )
    .bind(schema)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let foreign_keys: Vec<SchemaForeignKey> = fk_rows
        .into_iter()
        .map(|row| SchemaForeignKey {
            constraint_name: row.try_get("name").unwrap_or_default(),
            from_schema: row.try_get("from_schema").unwrap_or_default(),
            from_table: row.try_get("from_table").unwrap_or_default(),
            from_columns: row.try_get("from_columns").unwrap_or_default(),
            to_schema: row.try_get("to_schema").unwrap_or_default(),
            to_table: row.try_get("to_table").unwrap_or_default(),
            to_columns: row.try_get("to_columns").unwrap_or_default(),
        })
        .collect();

    Ok(SchemaRelationships {
        tables,
        foreign_keys,
    })
}

// ---------------------------------------------------------------------------
// Database overview stats
// ---------------------------------------------------------------------------

pub async fn load_database_overview_stats(
    connection: &StoredConnection,
) -> Result<DatabaseOverviewStats, String> {
    let mut conn = connect(connection).await?;

    // Single round trip: aggregate everything we display on the overview.
    // `row_count_estimate` is the planner's reltuples estimate (cheap and
    // good enough for the dashboard); precise counts would require per-table
    // SELECT count(*) which can be expensive on large databases.
    let row = sqlx::query(
        r#"
        WITH user_relations AS (
            SELECT c.oid, c.relkind, c.reltuples, n.nspname
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
              AND n.nspname NOT LIKE 'pg_toast%'
        )
        SELECT
            pg_database_size(current_database())::bigint AS database_size_bytes,
            COALESCE(SUM(pg_table_size(oid)) FILTER (WHERE relkind IN ('r', 'p')), 0)::bigint AS table_size_bytes,
            COALESCE(SUM(pg_indexes_size(oid)) FILTER (WHERE relkind IN ('r', 'p')), 0)::bigint AS index_size_bytes,
            COUNT(*) FILTER (WHERE relkind IN ('r', 'p'))::bigint AS table_count,
            COUNT(DISTINCT nspname)::bigint AS schema_count,
            COALESCE(SUM(GREATEST(reltuples, 0)::bigint) FILTER (WHERE relkind IN ('r', 'p')), 0)::bigint AS row_count_estimate,
            COUNT(*) FILTER (WHERE relkind = 'i')::bigint AS index_count,
            (SELECT COUNT(*) FROM pg_stat_activity WHERE datname = current_database())::bigint AS connection_count
        FROM user_relations
        "#,
    )
    .fetch_one(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    Ok(DatabaseOverviewStats {
        database_size_bytes: row.try_get("database_size_bytes").unwrap_or(0),
        table_size_bytes: row.try_get("table_size_bytes").unwrap_or(0),
        index_size_bytes: row.try_get("index_size_bytes").unwrap_or(0),
        table_count: row.try_get("table_count").unwrap_or(0),
        schema_count: row.try_get("schema_count").unwrap_or(0),
        row_count_estimate: row.try_get("row_count_estimate").unwrap_or(0),
        index_count: row.try_get("index_count").unwrap_or(0),
        connection_count: row.try_get("connection_count").unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    //! Tests cover the pure SQL builders only — every other public function
    //! in this module needs a live PostgreSQL to exercise. SQL composition
    //! (identifier escaping, NULL handling, `$N` parameter alignment) is
    //! exactly the kind of thing where bugs hide silently in production
    //! until a value with an embedded quote shows up; the integration path
    //! is too expensive to be a primary test surface.

    use super::*;

    /// Construct a `CellEditKeyValue` succinctly. `None` value = SQL `NULL`.
    fn kv(column: &str, value: Option<&str>) -> CellEditKeyValue {
        CellEditKeyValue {
            column: column.to_string(),
            value: value.map(str::to_string),
        }
    }

    // ----- build_update ------------------------------------------------

    #[test]
    fn update_emits_set_then_where_with_aligned_params() {
        let set = vec![kv("name", Some("Alice")), kv("email", Some("a@b.c"))];
        let identity = vec![kv("id", Some("42"))];
        let (sql, params) = build_update("public", "users", &set, &identity);
        assert_eq!(
            sql,
            r#"UPDATE "public"."users" SET "name" = $1, "email" = $2 WHERE "id" = $3"#
        );
        assert_eq!(
            params,
            vec![
                Some("Alice".into()),
                Some("a@b.c".into()),
                Some("42".into())
            ]
        );
    }

    #[test]
    fn update_binds_none_for_set_null_value() {
        // SET col = NULL must still bind a positional parameter so $N stays
        // aligned; it just binds None which sqlx maps to NULL.
        let set = vec![kv("name", None)];
        let identity = vec![kv("id", Some("42"))];
        let (sql, params) = build_update("public", "users", &set, &identity);
        assert_eq!(
            sql,
            r#"UPDATE "public"."users" SET "name" = $1 WHERE "id" = $2"#
        );
        assert_eq!(params, vec![None, Some("42".into())]);
    }

    #[test]
    fn update_renders_null_identity_as_is_null_and_skips_param() {
        // NULLs cannot be matched with `=`, so we emit `IS NULL` directly
        // and don't bind a parameter for that identity column.
        let set = vec![kv("name", Some("Alice"))];
        let identity = vec![kv("deleted_at", None)];
        let (sql, params) = build_update("public", "users", &set, &identity);
        assert_eq!(
            sql,
            r#"UPDATE "public"."users" SET "name" = $1 WHERE "deleted_at" IS NULL"#
        );
        assert_eq!(params, vec![Some("Alice".into())]);
    }

    #[test]
    fn update_keeps_param_indices_when_identity_mixes_null_and_value() {
        // Two SET cols ($1, $2), then identity with one NULL (no param) and
        // one real value. The real value must bind to $3 (params.len() at
        // the time of emission), not $4.
        let set = vec![kv("a", Some("1")), kv("b", Some("2"))];
        let identity = vec![kv("nulled", None), kv("real", Some("x"))];
        let (sql, params) = build_update("public", "t", &set, &identity);
        assert_eq!(
            sql,
            r#"UPDATE "public"."t" SET "a" = $1, "b" = $2 WHERE "nulled" IS NULL AND "real" = $3"#
        );
        assert_eq!(
            params,
            vec![Some("1".into()), Some("2".into()), Some("x".into())]
        );
    }

    #[test]
    fn update_quotes_identifiers_with_embedded_quotes() {
        // `quote_double` doubles internal `"` characters per SQL identifier
        // rules. A column literally named `weird"col` becomes `"weird""col"`.
        let set = vec![kv(r#"weird"col"#, Some("v"))];
        let identity = vec![kv("id", Some("1"))];
        let (sql, _) = build_update(r#"my"schema"#, r#"my"table"#, &set, &identity);
        assert_eq!(
            sql,
            r#"UPDATE "my""schema"."my""table" SET "weird""col" = $1 WHERE "id" = $2"#
        );
    }

    // ----- build_insert ------------------------------------------------

    #[test]
    fn insert_emits_columns_in_order_with_aligned_placeholders() {
        let values = vec![kv("name", Some("Alice")), kv("email", Some("a@b.c"))];
        let (sql, params) = build_insert("public", "users", &values);
        assert_eq!(
            sql,
            r#"INSERT INTO "public"."users" ("name", "email") VALUES ($1, $2)"#
        );
        assert_eq!(params, vec![Some("Alice".into()), Some("a@b.c".into())]);
    }

    #[test]
    fn insert_binds_none_for_null_columns() {
        let values = vec![kv("name", Some("Alice")), kv("middle_name", None)];
        let (sql, params) = build_insert("public", "users", &values);
        assert_eq!(
            sql,
            r#"INSERT INTO "public"."users" ("name", "middle_name") VALUES ($1, $2)"#
        );
        assert_eq!(params, vec![Some("Alice".into()), None]);
    }

    #[test]
    fn insert_quotes_identifiers_with_embedded_quotes() {
        let values = vec![kv(r#"col"name"#, Some("v"))];
        let (sql, _) = build_insert(r#"my"schema"#, r#"my"table"#, &values);
        assert_eq!(
            sql,
            r#"INSERT INTO "my""schema"."my""table" ("col""name") VALUES ($1)"#
        );
    }

    // ----- build_delete ------------------------------------------------

    #[test]
    fn delete_emits_where_clause_with_aligned_params() {
        let identity = vec![kv("id", Some("42"))];
        let (sql, params) = build_delete("public", "users", &identity);
        assert_eq!(sql, r#"DELETE FROM "public"."users" WHERE "id" = $1"#);
        assert_eq!(params, vec![Some("42".into())]);
    }

    #[test]
    fn delete_supports_composite_identity() {
        let identity = vec![kv("a", Some("1")), kv("b", Some("2"))];
        let (sql, params) = build_delete("public", "t", &identity);
        assert_eq!(
            sql,
            r#"DELETE FROM "public"."t" WHERE "a" = $1 AND "b" = $2"#
        );
        assert_eq!(params, vec![Some("1".into()), Some("2".into())]);
    }

    #[test]
    fn delete_renders_null_identity_as_is_null_and_skips_param() {
        // Same behaviour as build_update: NULL identity → `IS NULL`, no
        // parameter binding, downstream non-NULL identities still bind to
        // params.len() at emission time.
        let identity = vec![kv("deleted_at", None), kv("id", Some("42"))];
        let (sql, params) = build_delete("public", "t", &identity);
        assert_eq!(
            sql,
            r#"DELETE FROM "public"."t" WHERE "deleted_at" IS NULL AND "id" = $1"#
        );
        assert_eq!(params, vec![Some("42".into())]);
    }

    #[test]
    fn delete_quotes_identifiers_with_embedded_quotes() {
        let identity = vec![kv(r#"odd"col"#, Some("1"))];
        let (sql, _) = build_delete(r#"my"schema"#, r#"my"table"#, &identity);
        assert_eq!(
            sql,
            r#"DELETE FROM "my""schema"."my""table" WHERE "odd""col" = $1"#
        );
    }
}
