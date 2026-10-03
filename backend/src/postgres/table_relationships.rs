use std::collections::{HashMap, HashSet};

use sqlx::Row;

use crate::{
    SchemaForeignKey, SchemaRelationships, SchemaTableColumn, SchemaTableNode, SchemaTableTrigger,
    StoredConnection,
};

use super::connect;
use super::relationship_metadata::{
    classify_cardinality, detect_junction_tables, fk_columns_nullable, trigger_enabled,
    trigger_events, trigger_orientation, trigger_timing, unique_set_covers, OutgoingFk,
    RELATIONSHIP_TYPE_FOREIGN_KEY,
};
use super::schema::fk_action_label;

pub async fn fetch_table_schema_relationships(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
) -> Result<SchemaRelationships, String> {
    let mut conn = connect(connection).await?;

    let fk_rows = sqlx::query(
        r#"
        WITH focus AS (
          SELECT c.oid
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = $1
            AND c.relname = $2
            AND c.relkind IN ('r', 'p')
        )
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
          AND (
            con.conrelid IN (SELECT oid FROM focus)
            OR con.confrelid IN (SELECT oid FROM focus)
          )
        ORDER BY nsp.nspname, cls.relname, con.conname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

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

    let mut table_keys =
        HashSet::<(String, String)>::from([(schema.to_string(), table.to_string())]);
    for fk in &raw_fks {
        table_keys.insert((fk.from_schema.clone(), fk.from_table.clone()));
        table_keys.insert((fk.to_schema.clone(), fk.to_table.clone()));
    }

    let table_rows = sqlx::query(
        r#"
        WITH focus AS (
          SELECT c.oid
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = $1
            AND c.relname = $2
            AND c.relkind IN ('r', 'p')
        ),
        involved AS (
          SELECT oid FROM focus
          UNION
          SELECT con.conrelid
          FROM pg_constraint con
          WHERE con.contype = 'f'
            AND con.conparentid = 0
            AND con.confrelid IN (SELECT oid FROM focus)
          UNION
          SELECT con.confrelid
          FROM pg_constraint con
          WHERE con.contype = 'f'
            AND con.conparentid = 0
            AND con.conrelid IN (SELECT oid FROM focus)
        )
        SELECT n.nspname::text AS schema_name,
               c.relname::text AS name
        FROM involved i
        JOIN pg_class c ON c.oid = i.oid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        ORDER BY n.nspname, c.relname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    let column_rows = sqlx::query(
        r#"
        WITH focus AS (
          SELECT c.oid
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = $1
            AND c.relname = $2
            AND c.relkind IN ('r', 'p')
        ),
        involved AS (
          SELECT oid FROM focus
          UNION
          SELECT con.conrelid
          FROM pg_constraint con
          WHERE con.contype = 'f'
            AND con.conparentid = 0
            AND con.confrelid IN (SELECT oid FROM focus)
          UNION
          SELECT con.confrelid
          FROM pg_constraint con
          WHERE con.contype = 'f'
            AND con.conparentid IN (0)
            AND con.conrelid IN (SELECT oid FROM focus)
        )
        SELECT n.nspname::text AS schema_name,
               cls.relname::text AS table_name,
               a.attname::text AS column_name,
               format_type(a.atttypid, a.atttypmod)::text AS data_type,
               (NOT a.attnotnull)::bool AS nullable,
               a.attnum::int AS ordinal_position,
               EXISTS (
                 SELECT 1
                 FROM pg_index ix
                 WHERE ix.indrelid = cls.oid
                   AND ix.indisprimary
                   AND a.attnum = ANY(ix.indkey)
               ) AS is_primary_key,
               d.description::text AS comment
        FROM involved i
        JOIN pg_class cls ON cls.oid = i.oid
        JOIN pg_namespace n ON n.oid = cls.relnamespace
        JOIN pg_attribute a ON a.attrelid = cls.oid
        LEFT JOIN pg_description d
          ON d.objoid = cls.oid
         AND d.objsubid = a.attnum
        WHERE a.attnum > 0
          AND NOT a.attisdropped
        ORDER BY n.nspname, cls.relname, a.attnum
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    let mut columns_by_table: HashMap<(String, String), Vec<SchemaTableColumn>> = HashMap::new();
    for row in column_rows {
        let schema_name: String = row.try_get("schema_name").unwrap_or_default();
        let table_name: String = row.try_get("table_name").unwrap_or_default();
        columns_by_table
            .entry((schema_name, table_name))
            .or_default()
            .push(SchemaTableColumn {
                name: row.try_get("column_name").unwrap_or_default(),
                data_type: row.try_get("data_type").unwrap_or_default(),
                nullable: row.try_get("nullable").unwrap_or(false),
                is_primary_key: row.try_get("is_primary_key").unwrap_or(false),
                ordinal_position: row.try_get("ordinal_position").unwrap_or(0),
                comment: row.try_get::<Option<String>, _>("comment").unwrap_or(None),
            });
    }

    let unique_rows = sqlx::query(
        r#"
        WITH focus AS (
          SELECT c.oid
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = $1
            AND c.relname = $2
            AND c.relkind IN ('r', 'p')
        ),
        involved AS (
          SELECT oid FROM focus
          UNION
          SELECT con.conrelid
          FROM pg_constraint con
          WHERE con.contype = 'f'
            AND con.conparentid = 0
            AND con.confrelid IN (SELECT oid FROM focus)
          UNION
          SELECT con.confrelid
          FROM pg_constraint con
          WHERE con.contype = 'f'
            AND con.conparentid = 0
            AND con.conrelid IN (SELECT oid FROM focus)
        )
        SELECT n.nspname::text AS schema_name,
               cls.relname::text AS table_name,
               (
                   SELECT array_agg(att.attname::text ORDER BY u.ord)
                   FROM unnest(ix.indkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = ix.indrelid AND att.attnum = u.attnum
                   WHERE u.ord <= ix.indnkeyatts
               ) AS columns
        FROM pg_index ix
        JOIN involved i ON i.oid = ix.indrelid
        JOIN pg_class cls ON cls.oid = ix.indrelid
        JOIN pg_namespace n ON n.oid = cls.relnamespace
        WHERE ix.indisunique
          AND ix.indisvalid
          AND ix.indpred IS NULL
          AND ix.indexprs IS NULL
        ORDER BY n.nspname, cls.relname, ix.indexrelid
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    let mut unique_sets_by_table: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for row in unique_rows {
        let schema_name: String = row.try_get("schema_name").unwrap_or_default();
        let table_name: String = row.try_get("table_name").unwrap_or_default();
        let columns: Vec<String> = row.try_get("columns").unwrap_or_default();
        if !columns.is_empty() {
            unique_sets_by_table
                .entry(format!("{schema_name}.{table_name}"))
                .or_default()
                .push(columns);
        }
    }

    let trigger_rows = sqlx::query(
        r#"
        WITH focus AS (
          SELECT c.oid
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = $1
            AND c.relname = $2
            AND c.relkind IN ('r', 'p')
        ),
        involved AS (
          SELECT oid FROM focus
          UNION
          SELECT con.conrelid
          FROM pg_constraint con
          WHERE con.contype = 'f'
            AND con.conparentid = 0
            AND con.confrelid IN (SELECT oid FROM focus)
          UNION
          SELECT con.confrelid
          FROM pg_constraint con
          WHERE con.contype = 'f'
            AND con.conparentid = 0
            AND con.conrelid IN (SELECT oid FROM focus)
        )
        SELECT n.nspname::text AS schema_name,
               cls.relname::text AS table_name,
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
        JOIN involved i ON i.oid = tg.tgrelid
        JOIN pg_class cls ON cls.oid = tg.tgrelid
        JOIN pg_namespace n ON n.oid = cls.relnamespace
        JOIN pg_proc proc ON proc.oid = tg.tgfoid
        WHERE NOT tg.tgisinternal
          AND tg.tgparentid = 0
        ORDER BY n.nspname, cls.relname, tg.tgname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    let mut triggers_by_table: HashMap<(String, String), Vec<SchemaTableTrigger>> = HashMap::new();
    for row in trigger_rows {
        let schema_name: String = row.try_get("schema_name").unwrap_or_default();
        let table_name: String = row.try_get("table_name").unwrap_or_default();
        let tgtype: i16 = row.try_get("tgtype").unwrap_or(0);
        let enabled_state: String = row.try_get("enabled_state").unwrap_or_default();
        triggers_by_table
            .entry((schema_name, table_name.clone()))
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

    let outgoing: Vec<OutgoingFk<'_>> = raw_fks
        .iter()
        .map(|fk| OutgoingFk {
            constraint: &fk.constraint_name,
            table: &fk.from_table,
            columns: &fk.from_columns,
        })
        .collect();
    let unqualified_unique_sets: HashMap<String, Vec<Vec<String>>> = unique_sets_by_table
        .iter()
        .map(|(key, value)| {
            (
                key.rsplit_once('.')
                    .map(|(_, table)| table.to_string())
                    .unwrap_or_else(|| key.clone()),
                value.clone(),
            )
        })
        .collect();
    let junctions = detect_junction_tables(&outgoing, &unqualified_unique_sets);

    for row in &table_rows {
        let schema_name: String = row.try_get("schema_name").unwrap_or_default();
        let table_name: String = row.try_get("name").unwrap_or_default();
        table_keys.insert((schema_name, table_name));
    }
    let mut table_keys = table_keys.into_iter().collect::<Vec<_>>();
    table_keys.sort();

    let tables = table_keys
        .into_iter()
        .map(|(schema_name, table_name)| {
            let key = (schema_name.clone(), table_name.clone());
            let columns = columns_by_table.remove(&key).unwrap_or_default();
            let triggers = triggers_by_table.remove(&key).unwrap_or_default();
            SchemaTableNode {
                schema: schema_name,
                name: table_name.clone(),
                column_count: columns.len() as u32,
                columns,
                is_junction_table: Some(junctions.tables.contains(&table_name)),
                triggers,
            }
        })
        .collect();

    let foreign_keys = raw_fks
        .into_iter()
        .map(|fk| {
            let unique_key = format!("{}.{}", fk.from_schema, fk.from_table);
            let fk_columns_unique = unique_set_covers(
                &fk.from_columns,
                unique_sets_by_table
                    .get(&unique_key)
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
