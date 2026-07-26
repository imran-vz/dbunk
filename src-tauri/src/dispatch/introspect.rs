//! Table-structure introspection for the sqlx-Any engines (MySQL,
//! SQLite).
//!
//! PostgreSQL and ClickHouse each own their catalog queries in their
//! engine modules; MySQL and SQLite share one because they share one
//! driver path. Both report columns, primary key, foreign keys and
//! indexes — enough for constraint-aware Table Seeding (ADR-0020) to
//! know what it must satisfy.
//!
//! Mutation capabilities stay `false`: reading a table's shape is not
//! the same as being able to edit its rows, and the per-engine catch-up
//! for cell edits / inserts / deletes hasn't happened yet (ADR-0001).

use sqlx::{Any, AnyConnection};

use crate::{
    ColumnInfo, ConstraintInfo, DatabaseEngine, ForeignKeyInfo, IndexInfo, StoredConnection,
    StructureCapabilities, TableStructure,
};

use super::relational::{row_to_strings, sqlx_connect};

/// Value `sqlx`-Any renders for SQL NULL through `row_to_strings`.
const NULL_TEXT: &str = "NULL";

fn cell(row: &[String], index: usize) -> Option<&str> {
    row.get(index)
        .map(String::as_str)
        .filter(|value| *value != NULL_TEXT)
}

fn cell_owned(row: &[String], index: usize) -> String {
    cell(row, index).unwrap_or_default().to_string()
}

async fn fetch(
    conn: &mut AnyConnection,
    sql: &str,
    binds: &[&str],
) -> Result<Vec<Vec<String>>, String> {
    let mut query = sqlx::query::<Any>(sql);
    for bind in binds {
        query = query.bind(*bind);
    }
    let rows = query
        .fetch_all(conn)
        .await
        .map_err(|error| error.to_string())?;
    Ok(rows.iter().map(row_to_strings).collect())
}

pub(super) async fn fetch_table_structure(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
) -> Result<TableStructure, String> {
    let mut conn = sqlx_connect(connection).await?;
    match connection.engine() {
        DatabaseEngine::MySQL => {
            let schema = if schema.is_empty() {
                connection.database()
            } else {
                schema
            };
            mysql_structure(&mut conn, schema, table).await
        }
        DatabaseEngine::SQLite => sqlite_structure(&mut conn, table).await,
        other => Err(format!(
            "BUG: sqlx-Any introspection reached for {}",
            super::relational::engine_name(&other)
        )),
    }
}

// ---------------------------------------------------------------------------
// MySQL
// ---------------------------------------------------------------------------

/// `information_schema` columns are typed as `varchar` over utf8mb3 in
/// some server builds, which the Any driver refuses to decode — every
/// text projection is cast to `CHAR` for the same reason the schema
/// explorer does it.
async fn mysql_structure(
    conn: &mut AnyConnection,
    schema: &str,
    table: &str,
) -> Result<TableStructure, String> {
    let column_rows = fetch(
        conn,
        "SELECT CAST(column_name AS CHAR), CAST(column_type AS CHAR), \
                CAST(is_nullable AS CHAR), CAST(column_default AS CHAR), \
                CAST(extra AS CHAR), ordinal_position \
         FROM information_schema.columns \
         WHERE table_schema = ? AND table_name = ? \
         ORDER BY ordinal_position",
        &[schema, table],
    )
    .await?;
    if column_rows.is_empty() {
        return Err(format!(
            "table \"{schema}\".\"{table}\" was not found (or is not visible to this user)"
        ));
    }

    let index_rows = fetch(
        conn,
        "SELECT CAST(index_name AS CHAR), non_unique, CAST(column_name AS CHAR), \
                CAST(index_type AS CHAR) \
         FROM information_schema.statistics \
         WHERE table_schema = ? AND table_name = ? \
         ORDER BY index_name, seq_in_index",
        &[schema, table],
    )
    .await?;

    let fk_rows = fetch(
        conn,
        "SELECT CAST(k.constraint_name AS CHAR), CAST(k.column_name AS CHAR), \
                CAST(k.referenced_table_schema AS CHAR), CAST(k.referenced_table_name AS CHAR), \
                CAST(k.referenced_column_name AS CHAR), CAST(r.update_rule AS CHAR), \
                CAST(r.delete_rule AS CHAR) \
         FROM information_schema.key_column_usage k \
         JOIN information_schema.referential_constraints r \
           ON r.constraint_schema = k.constraint_schema \
          AND r.constraint_name = k.constraint_name \
          AND r.table_name = k.table_name \
         WHERE k.table_schema = ? AND k.table_name = ? \
           AND k.referenced_table_name IS NOT NULL \
         ORDER BY k.constraint_name, k.ordinal_position",
        &[schema, table],
    )
    .await?;

    // CHECK constraints only exist from MySQL 8.0.16 / MariaDB 10.2 —
    // best-effort, an older server just reports none.
    let check_rows = fetch(
        conn,
        "SELECT CAST(constraint_name AS CHAR), CAST(check_clause AS CHAR) \
         FROM information_schema.check_constraints \
         WHERE constraint_schema = ? AND table_name = ?",
        &[schema, table],
    )
    .await
    .unwrap_or_default();

    // Primary key membership comes from the PRIMARY index, in key order.
    let primary_key_columns: Vec<String> = index_rows
        .iter()
        .filter(|row| cell(row, 0) == Some("PRIMARY"))
        .map(|row| cell_owned(row, 2))
        .collect();

    let columns: Vec<ColumnInfo> = column_rows
        .iter()
        .map(|row| {
            let name = cell_owned(row, 0);
            let extra = cell_owned(row, 4).to_ascii_lowercase();
            let declared_default = cell(row, 3).map(str::to_string);
            // MySQL keeps "this column fills itself in" in `extra`, not
            // in the default — surface it the way PostgreSQL surfaces
            // `nextval(...)` so both read the same downstream.
            let default_value = match declared_default {
                Some(value) => Some(value),
                None if extra.contains("auto_increment") => Some("AUTO_INCREMENT".to_string()),
                None => None,
            };
            let derivation_kind = if extra.contains("virtual generated") {
                Some("VIRTUAL".to_string())
            } else if extra.contains("stored generated") {
                Some("STORED".to_string())
            } else {
                None
            };
            ColumnInfo {
                is_primary_key: primary_key_columns.contains(&name),
                name,
                data_type: cell_owned(row, 1),
                nullable: cell(row, 2).unwrap_or("NO").eq_ignore_ascii_case("YES"),
                default_value,
                ordinal_position: cell(row, 5).and_then(|v| v.parse().ok()).unwrap_or(0),
                derivation_kind,
            }
        })
        .collect();

    let mut indexes: Vec<IndexInfo> = Vec::new();
    for row in &index_rows {
        let name = cell_owned(row, 0);
        let column = cell_owned(row, 2);
        match indexes.last_mut() {
            Some(index) if index.name == name => index.columns.push(column),
            _ => indexes.push(IndexInfo {
                is_unique: cell(row, 1) == Some("0"),
                is_primary: name == "PRIMARY",
                name,
                columns: vec![column],
                method: cell(row, 3).map(str::to_string),
            }),
        }
    }

    let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
    for row in &fk_rows {
        let name = cell_owned(row, 0);
        match foreign_keys.last_mut() {
            Some(fk) if fk.name == name => {
                fk.columns.push(cell_owned(row, 1));
                fk.referenced_columns.push(cell_owned(row, 4));
            }
            _ => foreign_keys.push(ForeignKeyInfo {
                name,
                columns: vec![cell_owned(row, 1)],
                referenced_schema: cell_owned(row, 2),
                referenced_table: cell_owned(row, 3),
                referenced_columns: vec![cell_owned(row, 4)],
                on_update: cell(row, 5).map(str::to_string),
                on_delete: cell(row, 6).map(str::to_string),
            }),
        }
    }

    let constraints: Vec<ConstraintInfo> = check_rows
        .iter()
        .map(|row| ConstraintInfo {
            name: cell_owned(row, 0),
            kind: "CHECK".to_string(),
            definition: cell_owned(row, 1),
        })
        .collect();

    Ok(assemble(
        columns,
        primary_key_columns,
        foreign_keys,
        indexes,
        constraints,
    ))
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

async fn sqlite_structure(conn: &mut AnyConnection, table: &str) -> Result<TableStructure, String> {
    let column_rows = fetch(
        conn,
        "SELECT name, type, \"notnull\", dflt_value, pk, cid FROM pragma_table_info(?)",
        &[table],
    )
    .await?;
    if column_rows.is_empty() {
        return Err(format!("table \"{table}\" was not found"));
    }

    // `pk` is the 1-based position within the primary key, 0 when the
    // column isn't part of it.
    let mut key_members: Vec<(i64, String)> = column_rows
        .iter()
        .filter_map(|row| {
            let position: i64 = cell(row, 4).and_then(|v| v.parse().ok()).unwrap_or(0);
            (position > 0).then(|| (position, cell_owned(row, 0)))
        })
        .collect();
    key_members.sort_by_key(|(position, _)| *position);
    let primary_key_columns: Vec<String> = key_members.into_iter().map(|(_, name)| name).collect();

    let columns: Vec<ColumnInfo> = column_rows
        .iter()
        .map(|row| {
            let name = cell_owned(row, 0);
            ColumnInfo {
                is_primary_key: primary_key_columns.contains(&name),
                name,
                data_type: cell_owned(row, 1),
                nullable: cell(row, 2) != Some("1"),
                default_value: cell(row, 3).map(str::to_string),
                ordinal_position: cell(row, 5)
                    .and_then(|v| v.parse::<i32>().ok())
                    .map(|cid| cid + 1)
                    .unwrap_or(0),
                derivation_kind: None,
            }
        })
        .collect();

    let fk_rows = fetch(
        conn,
        "SELECT id, seq, \"table\", \"from\", \"to\", on_update, on_delete \
         FROM pragma_foreign_key_list(?) ORDER BY id, seq",
        &[table],
    )
    .await?;
    let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
    for row in &fk_rows {
        let id = cell_owned(row, 0);
        let child = cell_owned(row, 3);
        // A NULL `to` means "the parent's primary key" — resolve it so
        // the seeder always has a concrete column to sample.
        let parent_table = cell_owned(row, 2);
        let parent_column = match cell(row, 4) {
            Some(column) => column.to_string(),
            None => sqlite_primary_key_column(conn, &parent_table, cell(row, 1).unwrap_or("0"))
                .await
                .unwrap_or_default(),
        };
        let name = format!("fk_{table}_{id}");
        match foreign_keys.last_mut() {
            Some(fk) if fk.name == name => {
                fk.columns.push(child);
                fk.referenced_columns.push(parent_column);
            }
            _ => foreign_keys.push(ForeignKeyInfo {
                name,
                columns: vec![child],
                referenced_schema: String::new(),
                referenced_table: parent_table,
                referenced_columns: vec![parent_column],
                on_update: cell(row, 5).map(str::to_string),
                on_delete: cell(row, 6).map(str::to_string),
            }),
        }
    }

    let index_rows = fetch(
        conn,
        "SELECT name, \"unique\", origin FROM pragma_index_list(?)",
        &[table],
    )
    .await?;
    let mut indexes: Vec<IndexInfo> = Vec::new();
    for row in &index_rows {
        let name = cell_owned(row, 0);
        let member_rows = fetch(
            conn,
            "SELECT name FROM pragma_index_info(?) ORDER BY seqno",
            &[name.as_str()],
        )
        .await?;
        indexes.push(IndexInfo {
            columns: member_rows.iter().map(|row| cell_owned(row, 0)).collect(),
            is_unique: cell(row, 1) == Some("1"),
            // origin "pk" marks the implicit index behind a non-rowid
            // primary key.
            is_primary: cell(row, 2) == Some("pk"),
            name,
            method: None,
        });
    }

    Ok(assemble(
        columns,
        primary_key_columns,
        foreign_keys,
        indexes,
        Vec::new(),
    ))
}

/// The `seq`-th column of `table`'s primary key, for foreign keys
/// declared without an explicit parent column.
async fn sqlite_primary_key_column(
    conn: &mut AnyConnection,
    table: &str,
    seq: &str,
) -> Option<String> {
    let rows = fetch(
        conn,
        "SELECT name, pk FROM pragma_table_info(?) WHERE pk > 0 ORDER BY pk",
        &[table],
    )
    .await
    .ok()?;
    let index: usize = seq.parse().unwrap_or(0);
    rows.get(index).map(|row| cell_owned(row, 0))
}

// ---------------------------------------------------------------------------

fn assemble(
    columns: Vec<ColumnInfo>,
    primary_key_columns: Vec<String>,
    foreign_keys: Vec<ForeignKeyInfo>,
    indexes: Vec<IndexInfo>,
    constraints: Vec<ConstraintInfo>,
) -> TableStructure {
    let has_primary_key = !primary_key_columns.is_empty();
    TableStructure {
        columns,
        primary_key: has_primary_key.then_some(primary_key_columns),
        foreign_keys,
        indexes,
        constraints,
        capabilities: StructureCapabilities {
            columns: true,
            primary_key: true,
            foreign_keys: true,
            indexes: true,
            constraints: true,
            // Row mutation on these engines is still unimplemented —
            // reading the shape doesn't imply we can write through it.
            can_insert_rows: false,
            can_update_rows: false,
            can_delete_rows: false,
            can_alter_schema: false,
            uniqueness_guarantee: if has_primary_key {
                "exact".to_string()
            } else {
                "best-effort".to_string()
            },
        },
        table_engine: None,
        partition_by: None,
        sample_by: None,
    }
}
