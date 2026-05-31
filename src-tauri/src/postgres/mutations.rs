//! Cell edits, row inserts, row deletes, and bulk imports.

use std::time::Instant;

use sqlx::Acquire;

use crate::{
    quote_double, CellEdit, CellEditKeyValue, CommitCellEditsResult, DeleteRowsResult,
    ImportRowsResult, InsertRowResult, StoredConnection,
};

use super::connect;

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
            if kv.value.is_none() {
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

fn build_bulk_insert(
    schema: &str,
    table: &str,
    columns: &[String],
    rows: &[Vec<Option<String>>],
) -> (String, Vec<Option<String>>) {
    let qualified = format!("{}.{}", quote_double(schema), quote_double(table));
    let column_list = columns
        .iter()
        .map(|column| quote_double(column))
        .collect::<Vec<_>>()
        .join(", ");
    let mut params = Vec::with_capacity(columns.len() * rows.len());
    let values = rows
        .iter()
        .enumerate()
        .map(|(row_index, row)| {
            let placeholders = columns
                .iter()
                .enumerate()
                .map(|(column_index, _)| {
                    params.push(row.get(column_index).cloned().unwrap_or(None));
                    format!("${}", row_index * columns.len() + column_index + 1)
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("({})", placeholders)
        })
        .collect::<Vec<_>>()
        .join(", ");
    (
        format!(
            "INSERT INTO {} ({}) VALUES {}",
            qualified, column_list, values
        ),
        params,
    )
}

fn csv_copy_field(value: &Option<String>) -> String {
    match value {
        None => "\\N".to_string(),
        Some(value) => {
            let escaped = value.replace('"', "\"\"");
            format!("\"{}\"", escaped)
        }
    }
}

fn build_copy_csv(rows: &[Vec<Option<String>>]) -> Vec<u8> {
    rows.iter()
        .map(|row| row.iter().map(csv_copy_field).collect::<Vec<_>>().join(","))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
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
// Public mutation operations
// ---------------------------------------------------------------------------

pub async fn commit_cell_edits(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    edits: &[CellEdit],
) -> Result<CommitCellEditsResult, String> {
    let mut conn = connect(connection).await?;
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

pub async fn import_rows(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    columns: &[String],
    rows: &[Vec<Option<String>>],
) -> Result<ImportRowsResult, String> {
    if columns.is_empty() {
        return Err("import has no mapped columns".to_string());
    }
    if rows.is_empty() {
        return Ok(ImportRowsResult {
            runtime_ms: 0,
            rows_affected: 0,
        });
    }
    let mut conn = connect(connection).await?;
    let mut tx = conn.begin().await.map_err(|error| error.to_string())?;
    let start = Instant::now();
    let mut rows_affected = 0;
    for chunk in rows.chunks(500) {
        let (sql, params) = build_bulk_insert(schema, table, columns, chunk);
        let mut query = sqlx::query(&sql);
        for param in &params {
            query = query.bind(param.as_deref());
        }
        rows_affected += query
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?
            .rows_affected();
    }
    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(ImportRowsResult {
        runtime_ms: start.elapsed().as_millis() as u64,
        rows_affected,
    })
}

pub async fn copy_import_rows(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    columns: &[String],
    rows: &[Vec<Option<String>>],
) -> Result<ImportRowsResult, String> {
    if columns.is_empty() {
        return Err("import has no mapped columns".to_string());
    }
    if rows.is_empty() {
        return Ok(ImportRowsResult {
            runtime_ms: 0,
            rows_affected: 0,
        });
    }
    let mut conn = connect(connection).await?;
    let start = Instant::now();
    let qualified = format!("{}.{}", quote_double(schema), quote_double(table));
    let column_list = columns
        .iter()
        .map(|column| quote_double(column))
        .collect::<Vec<_>>()
        .join(", ");
    let statement = format!(
        "COPY {} ({}) FROM STDIN WITH (FORMAT csv, NULL '\\N')",
        qualified, column_list
    );
    let payload = build_copy_csv(rows);
    let mut copy = conn
        .copy_in_raw(&statement)
        .await
        .map_err(|error| error.to_string())?;
    copy.send(payload)
        .await
        .map_err(|error| error.to_string())?;
    let rows_affected = copy.finish().await.map_err(|error| error.to_string())?;
    Ok(ImportRowsResult {
        runtime_ms: start.elapsed().as_millis() as u64,
        rows_affected,
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(column: &str, value: Option<&str>) -> CellEditKeyValue {
        CellEditKeyValue {
            column: column.to_string(),
            value: value.map(str::to_string),
        }
    }

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
        let set = vec![kv(r#"weird"col"#, Some("v"))];
        let identity = vec![kv("id", Some("1"))];
        let (sql, _) = build_update(r#"my"schema"#, r#"my"table"#, &set, &identity);
        assert_eq!(
            sql,
            r#"UPDATE "my""schema"."my""table" SET "weird""col" = $1 WHERE "id" = $2"#
        );
    }

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
