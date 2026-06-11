//! Table structure and schema relationship introspection.

use std::collections::HashMap;

use sqlx::Row;

use crate::{
    ColumnInfo, ConstraintInfo, ForeignKeyInfo, IndexInfo, SchemaForeignKey, SchemaRelationships,
    SchemaTableColumn, SchemaTableNode, SchemaTableTrigger, StoredConnection,
    StructureCapabilities, TableStructure,
};

use super::connect;
use super::relationship_metadata::{
    classify_cardinality, detect_junction_tables, fk_columns_nullable, trigger_enabled,
    trigger_events, trigger_orientation, trigger_timing, unique_set_covers, OutgoingFk,
    RELATIONSHIP_TYPE_FOREIGN_KEY,
};

pub(super) fn fk_action_label(code: &str) -> Option<String> {
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
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    let primary_key_cols: Vec<String> = pk_rows
        .iter()
        .map(|row| row.try_get::<String, _>("column_name").unwrap_or_default())
        .collect();

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
    .fetch_all(&mut *conn)
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
            derivation_kind: None,
        });
    }

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
    .fetch_all(&mut *conn)
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
    .fetch_all(&mut *conn)
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
    .fetch_all(&mut *conn)
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
            can_update_rows: has_primary_key,
            can_delete_rows: has_primary_key,
            can_alter_schema: true,
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

pub async fn fetch_schema_relationships(
    connection: &StoredConnection,
    schema: &str,
) -> Result<SchemaRelationships, String> {
    let mut conn = connect(connection).await?;

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
    .fetch_all(&mut *conn)
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
               (kcu.column_name IS NOT NULL) AS is_primary_key,
               d.description::text AS comment
        FROM information_schema.columns c
        LEFT JOIN pg_namespace pg_nsp
          ON pg_nsp.nspname = c.table_schema
        LEFT JOIN pg_class pg_cls
          ON pg_cls.relnamespace = pg_nsp.oid
         AND pg_cls.relname = c.table_name
        LEFT JOIN pg_attribute a
          ON a.attrelid = pg_cls.oid
         AND a.attname = c.column_name
         AND a.attnum > 0
        LEFT JOIN pg_description d
          ON d.objoid = pg_cls.oid
         AND d.objsubid = a.attnum
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
    .fetch_all(&mut *conn)
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
                comment: row.try_get::<Option<String>, _>("comment").unwrap_or(None),
            });
    }

    let fk_rows = sqlx::query(
        r#"
        SELECT con.conname::text AS name,
               nsp.nspname::text AS from_schema,
               cls.relname::text AS from_table,
               nsp_ref.nspname::text AS to_schema,
               cls_ref.relname::text AS to_table,
               con.confupdtype::text AS on_update,
               con.confdeltype::text AS on_delete,
               (
                   SELECT array_agg(att.attname::text ORDER BY u.ord)
                   FROM unnest(con.conkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = con.conrelid AND att.attnum = u.attnum
               ) AS from_columns,
               (
                   SELECT array_agg(att.attnotnull ORDER BY u.ord)
                   FROM unnest(con.conkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = con.conrelid AND att.attnum = u.attnum
               ) AS from_columns_not_null,
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
          AND con.conparentid = 0
          AND nsp.nspname = $1
        ORDER BY cls.relname, con.conname
        "#,
    )
    .bind(schema)
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    // Unique (non-partial, non-expression) index key sets per table —
    // the authority for Relationship Cardinality and junction
    // detection. Partial/expression indexes don't guarantee row-level
    // uniqueness of the column values, so they are excluded.
    let unique_rows = sqlx::query(
        r#"
        SELECT cls.relname::text AS table_name,
               (
                   SELECT array_agg(att.attname::text ORDER BY u.ord)
                   FROM unnest(ix.indkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = ix.indrelid AND att.attnum = u.attnum
                   WHERE u.ord <= ix.indnkeyatts
               ) AS columns
        FROM pg_index ix
        JOIN pg_class cls ON cls.oid = ix.indrelid
        JOIN pg_namespace nsp ON nsp.oid = cls.relnamespace
        WHERE nsp.nspname = $1
          AND cls.relkind IN ('r', 'p')
          AND ix.indisunique
          AND ix.indisvalid
          AND ix.indpred IS NULL
          AND ix.indexprs IS NULL
        ORDER BY cls.relname, ix.indexrelid
        "#,
    )
    .bind(schema)
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    let mut unique_sets_by_table: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for row in unique_rows {
        let table_name: String = row.try_get("table_name").unwrap_or_default();
        let columns: Vec<String> = row.try_get("columns").unwrap_or_default();
        if !columns.is_empty() {
            unique_sets_by_table
                .entry(table_name)
                .or_default()
                .push(columns);
        }
    }

    // Compact Trigger Indicator metadata. User triggers only —
    // FK-enforcement triggers and the like stay hidden, as are
    // partition-cloned copies of a partitioned parent's trigger
    // (`tgparentid`, PostgreSQL 13+; every older release is EOL).
    let trigger_rows = sqlx::query(
        r#"
        SELECT cls.relname::text AS table_name,
               tg.tgname::text AS name,
               tg.tgtype::int2 AS tgtype,
               tg.tgenabled::text AS enabled_state,
               proc.proname::text AS function_name,
               (
                   SELECT array_agg(att.attname::text ORDER BY u.ord)
                   FROM unnest(tg.tgattr) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = tg.tgrelid AND att.attnum = u.attnum
               ) AS columns
        FROM pg_trigger tg
        JOIN pg_class cls ON cls.oid = tg.tgrelid
        JOIN pg_namespace nsp ON nsp.oid = cls.relnamespace
        JOIN pg_proc proc ON proc.oid = tg.tgfoid
        WHERE NOT tg.tgisinternal
          AND tg.tgparentid = 0
          AND cls.relkind IN ('r', 'p')
          AND nsp.nspname = $1
        ORDER BY cls.relname, tg.tgname
        "#,
    )
    .bind(schema)
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    let mut triggers_by_table: HashMap<String, Vec<SchemaTableTrigger>> = HashMap::new();
    for row in trigger_rows {
        let table_name: String = row.try_get("table_name").unwrap_or_default();
        let tgtype: i16 = row.try_get("tgtype").unwrap_or(0);
        let enabled_state: String = row.try_get("enabled_state").unwrap_or_default();
        triggers_by_table
            .entry(table_name.clone())
            .or_default()
            .push(SchemaTableTrigger {
                name: row.try_get("name").unwrap_or_default(),
                table: table_name,
                columns: row.try_get("columns").unwrap_or_default(),
                timing: trigger_timing(tgtype).to_string(),
                events: trigger_events(tgtype),
                orientation: trigger_orientation(tgtype).to_string(),
                enabled: trigger_enabled(&enabled_state),
                function_name: row.try_get("function_name").unwrap_or_default(),
            });
    }

    struct FkRow {
        constraint_name: String,
        from_schema: String,
        from_table: String,
        from_columns: Vec<String>,
        from_columns_not_null: Vec<bool>,
        to_schema: String,
        to_table: String,
        to_columns: Vec<String>,
        on_update: Option<String>,
        on_delete: Option<String>,
    }

    let raw_fks: Vec<FkRow> = fk_rows
        .into_iter()
        .map(|row| {
            let on_update_code: String = row.try_get("on_update").unwrap_or_default();
            let on_delete_code: String = row.try_get("on_delete").unwrap_or_default();
            FkRow {
                constraint_name: row.try_get("name").unwrap_or_default(),
                from_schema: row.try_get("from_schema").unwrap_or_default(),
                from_table: row.try_get("from_table").unwrap_or_default(),
                from_columns: row.try_get("from_columns").unwrap_or_default(),
                from_columns_not_null: row.try_get("from_columns_not_null").unwrap_or_default(),
                to_schema: row.try_get("to_schema").unwrap_or_default(),
                to_table: row.try_get("to_table").unwrap_or_default(),
                to_columns: row.try_get("to_columns").unwrap_or_default(),
                on_update: fk_action_label(&on_update_code),
                on_delete: fk_action_label(&on_delete_code),
            }
        })
        .collect();

    let outgoing: Vec<OutgoingFk<'_>> = raw_fks
        .iter()
        .map(|fk| OutgoingFk {
            constraint: &fk.constraint_name,
            table: &fk.from_table,
            columns: &fk.from_columns,
        })
        .collect();
    let junctions = detect_junction_tables(&outgoing, &unique_sets_by_table);

    let tables: Vec<SchemaTableNode> = table_rows
        .into_iter()
        .map(|row| {
            let name: String = row.try_get("name").unwrap_or_default();
            let columns = columns_by_table.remove(&name).unwrap_or_default();
            let triggers = triggers_by_table.remove(&name).unwrap_or_default();
            let is_junction_table = junctions.tables.contains(&name);
            SchemaTableNode {
                schema: schema.to_string(),
                name,
                column_count: columns.len() as u32,
                columns,
                is_junction_table: Some(is_junction_table),
                triggers,
            }
        })
        .collect();

    let foreign_keys: Vec<SchemaForeignKey> = raw_fks
        .into_iter()
        .map(|fk| {
            let fk_columns_unique = unique_set_covers(
                &fk.from_columns,
                unique_sets_by_table
                    .get(&fk.from_table)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
            );
            let (cardinality, cardinality_reason) = classify_cardinality(fk_columns_unique);
            let nullable = fk_columns_nullable(&fk.from_columns, &fk.from_columns_not_null);
            let is_junction_participant =
                junctions.is_participant(&fk.from_table, &fk.constraint_name);
            SchemaForeignKey {
                constraint_name: fk.constraint_name,
                from_schema: fk.from_schema,
                from_table: fk.from_table,
                from_columns: fk.from_columns,
                to_schema: fk.to_schema,
                to_table: fk.to_table,
                to_columns: fk.to_columns,
                relationship_type: Some(RELATIONSHIP_TYPE_FOREIGN_KEY.to_string()),
                cardinality: Some(cardinality.to_string()),
                cardinality_reason: Some(cardinality_reason.to_string()),
                on_update: fk.on_update,
                on_delete: fk.on_delete,
                fk_columns_nullable: Some(nullable),
                fk_columns_unique: Some(fk_columns_unique),
                is_junction_participant: Some(is_junction_participant),
            }
        })
        .collect();

    Ok(SchemaRelationships {
        tables,
        foreign_keys,
    })
}

#[cfg(test)]
mod tests {
    use super::fk_action_label;

    #[test]
    fn fk_action_label_maps_every_pg_action_code() {
        assert_eq!(fk_action_label("a").as_deref(), Some("NO ACTION"));
        assert_eq!(fk_action_label("r").as_deref(), Some("RESTRICT"));
        assert_eq!(fk_action_label("c").as_deref(), Some("CASCADE"));
        assert_eq!(fk_action_label("n").as_deref(), Some("SET NULL"));
        assert_eq!(fk_action_label("d").as_deref(), Some("SET DEFAULT"));
    }

    #[test]
    fn fk_action_label_omits_unknown_codes_rather_than_guessing() {
        assert_eq!(fk_action_label(""), None);
        assert_eq!(fk_action_label("x"), None);
    }
}
