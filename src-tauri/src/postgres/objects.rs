use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row};

use super::object_ddl::PgObjectError;
use crate::{DatabaseEngine, StoredConnection};

pub(crate) const CATALOG_KIND_CAP: usize = 2_000;

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgObjectKind {
    Schema,
    Table,
    View,
    MaterializedView,
    ForeignTable,
    Sequence,
    Function,
    Procedure,
    Aggregate,
    Type,
    Domain,
    Extension,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgTypeClass {
    Enum,
    Composite,
    Range,
    Multirange,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgObjectRef {
    pub kind: PgObjectKind,
    pub schema: Option<String>,
    pub name: String,
    pub identity_args: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgCatalogEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_args: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_class: Option<PgTypeClass>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgSchemaObjects {
    pub name: String,
    pub tables: Vec<PgCatalogEntry>,
    pub views: Vec<PgCatalogEntry>,
    pub materialized_views: Vec<PgCatalogEntry>,
    pub foreign_tables: Vec<PgCatalogEntry>,
    pub sequences: Vec<PgCatalogEntry>,
    pub functions: Vec<PgCatalogEntry>,
    pub procedures: Vec<PgCatalogEntry>,
    pub aggregates: Vec<PgCatalogEntry>,
    pub types: Vec<PgCatalogEntry>,
    pub domains: Vec<PgCatalogEntry>,
    pub extensions: Vec<PgCatalogEntry>,
}

impl PgSchemaObjects {
    pub(crate) fn empty(name: String) -> Self {
        Self {
            name,
            tables: Vec::new(),
            views: Vec::new(),
            materialized_views: Vec::new(),
            foreign_tables: Vec::new(),
            sequences: Vec::new(),
            functions: Vec::new(),
            procedures: Vec::new(),
            aggregates: Vec::new(),
            types: Vec::new(),
            domains: Vec::new(),
            extensions: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgCatalogTruncation {
    pub schema: Option<String>,
    pub kind: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgObjectCatalog {
    pub schemas: Vec<PgSchemaObjects>,
    pub event_triggers: Vec<PgCatalogEntry>,
    pub roles: Vec<PgCatalogEntry>,
    pub tablespaces: Vec<PgCatalogEntry>,
    pub truncated: Vec<PgCatalogTruncation>,
}

pub(crate) async fn load_pg_object_catalog(
    connection: &StoredConnection,
) -> Result<PgObjectCatalog, PgObjectError> {
    ensure_postgres(connection)?;
    let mut conn = super::connect(connection)
        .await
        .map_err(|message| PgObjectError::Connection { message })?;
    let row_limit = (CATALOG_KIND_CAP + 1) as i64;
    let schema_rows = sqlx::query(
        r#"
SELECT n.nspname::text AS name,
       obj_description(n.oid, 'pg_namespace') AS comment
FROM pg_namespace n
WHERE n.nspname <> 'information_schema'
  AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
  AND NOT EXISTS (
    SELECT 1 FROM pg_depend d
    WHERE d.classid = 'pg_namespace'::regclass
      AND d.objid = n.oid
      AND d.deptype = 'e'
  )
ORDER BY n.nspname
LIMIT $1
"#,
    )
    .bind(row_limit)
    .fetch_all(&mut *conn)
    .await
    .map_err(read_database_error)?;

    let mut truncated = Vec::new();
    let mut schemas = BTreeMap::new();
    for row in schema_rows {
        let name: String = row.try_get("name").map_err(read_database_error)?;
        if schemas.len() == CATALOG_KIND_CAP {
            record_truncation(&mut truncated, None, "schema");
            break;
        }
        schemas.insert(name.clone(), PgSchemaObjects::empty(name));
    }

    let retained_schema_names = schemas.keys().cloned().collect::<Vec<_>>();

    load_relation_entries(
        &mut conn,
        row_limit,
        &retained_schema_names,
        &mut schemas,
        &mut truncated,
    )
    .await?;
    load_sequence_entries(
        &mut conn,
        row_limit,
        &retained_schema_names,
        &mut schemas,
        &mut truncated,
    )
    .await?;
    load_routine_entries(
        &mut conn,
        row_limit,
        &retained_schema_names,
        &mut schemas,
        &mut truncated,
    )
    .await?;
    load_type_entries(
        &mut conn,
        row_limit,
        &retained_schema_names,
        &mut schemas,
        &mut truncated,
    )
    .await?;
    load_domain_entries(
        &mut conn,
        row_limit,
        &retained_schema_names,
        &mut schemas,
        &mut truncated,
    )
    .await?;
    load_extension_entries(
        &mut conn,
        row_limit,
        &retained_schema_names,
        &mut schemas,
        &mut truncated,
    )
    .await?;

    let event_triggers = load_database_entries(
        &mut conn,
        row_limit,
        r#"
SELECT evtname::text AS name,
       obj_description(oid, 'pg_event_trigger') AS comment
FROM pg_event_trigger
ORDER BY evtname
LIMIT $1
"#,
        "event-trigger",
        &mut truncated,
    )
    .await?;
    let roles = load_database_entries(
        &mut conn,
        row_limit,
        r#"
SELECT rolname::text AS name,
       shobj_description(oid, 'pg_authid') AS comment
FROM pg_roles
ORDER BY rolname
LIMIT $1
"#,
        "role",
        &mut truncated,
    )
    .await?;
    let tablespaces = load_database_entries(
        &mut conn,
        row_limit,
        r#"
SELECT spcname::text AS name,
       shobj_description(oid, 'pg_tablespace') AS comment
FROM pg_tablespace
ORDER BY spcname
LIMIT $1
"#,
        "tablespace",
        &mut truncated,
    )
    .await?;

    Ok(PgObjectCatalog {
        schemas: schemas.into_values().collect(),
        event_triggers,
        roles,
        tablespaces,
        truncated,
    })
}

fn ensure_postgres(connection: &StoredConnection) -> Result<(), PgObjectError> {
    if connection.engine() == DatabaseEngine::PostgreSQL {
        return Ok(());
    }
    Err(PgObjectError::UnsupportedEngine {
        engine: format!("{:?}", connection.engine()),
    })
}

async fn load_relation_entries(
    conn: &mut sqlx::PgConnection,
    row_limit: i64,
    schema_names: &[String],
    schemas: &mut BTreeMap<String, PgSchemaObjects>,
    truncated: &mut Vec<PgCatalogTruncation>,
) -> Result<(), PgObjectError> {
    let rows = sqlx::query(
        r#"
WITH ranked AS (
  SELECT n.nspname::text AS schema_name,
         c.relname::text AS name,
         CASE c.relkind
           WHEN 'r' THEN 'table'
           WHEN 'p' THEN 'table'
           WHEN 'v' THEN 'view'
           WHEN 'm' THEN 'materialized-view'
           WHEN 'f' THEN 'foreign-table'
         END AS kind,
         obj_description(c.oid, 'pg_class') AS comment,
         row_number() OVER (
           PARTITION BY n.nspname,
             CASE c.relkind
               WHEN 'r' THEN 'table'
               WHEN 'p' THEN 'table'
               WHEN 'v' THEN 'view'
               WHEN 'm' THEN 'materialized-view'
               WHEN 'f' THEN 'foreign-table'
             END
           ORDER BY c.relname
         ) AS row_number
  FROM pg_class c
  JOIN pg_namespace n ON n.oid = c.relnamespace
  WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
    AND n.nspname = ANY($2::text[])
    AND n.nspname <> 'information_schema'
    AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
    AND NOT EXISTS (
      SELECT 1 FROM pg_depend d
      WHERE d.classid = 'pg_class'::regclass
        AND d.objid = c.oid
        AND d.deptype = 'e'
    )
)
SELECT schema_name, name, kind, comment
FROM ranked
WHERE row_number <= $1
ORDER BY schema_name, kind, name
"#,
    )
    .bind(row_limit)
    .bind(schema_names)
    .fetch_all(&mut *conn)
    .await
    .map_err(read_database_error)?;

    for row in rows {
        let schema: String = row.try_get("schema_name").map_err(read_database_error)?;
        let kind: String = row.try_get("kind").map_err(read_database_error)?;
        let entry = entry_from_row(&row, None, None)?;
        match kind.as_str() {
            "table" => push_schema_entry(schemas, truncated, &schema, &kind, entry, |objects| {
                &mut objects.tables
            }),
            "view" => push_schema_entry(schemas, truncated, &schema, &kind, entry, |objects| {
                &mut objects.views
            }),
            "materialized-view" => {
                push_schema_entry(schemas, truncated, &schema, &kind, entry, |objects| {
                    &mut objects.materialized_views
                })
            }
            "foreign-table" => {
                push_schema_entry(schemas, truncated, &schema, &kind, entry, |objects| {
                    &mut objects.foreign_tables
                })
            }
            _ => unreachable!("catalog relation query returned an unknown kind"),
        }
    }
    Ok(())
}

async fn load_sequence_entries(
    conn: &mut sqlx::PgConnection,
    row_limit: i64,
    schema_names: &[String],
    schemas: &mut BTreeMap<String, PgSchemaObjects>,
    truncated: &mut Vec<PgCatalogTruncation>,
) -> Result<(), PgObjectError> {
    let rows = sqlx::query(
        r#"
WITH ranked AS (
  SELECT n.nspname::text AS schema_name,
         c.relname::text AS name,
         obj_description(c.oid, 'pg_class') AS comment,
         row_number() OVER (PARTITION BY n.nspname ORDER BY c.relname) AS row_number
  FROM pg_class c
  JOIN pg_namespace n ON n.oid = c.relnamespace
  WHERE c.relkind = 'S'
    AND n.nspname = ANY($2::text[])
    AND n.nspname <> 'information_schema'
    AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
    AND NOT EXISTS (
      SELECT 1 FROM pg_depend d
      WHERE d.classid = 'pg_class'::regclass
        AND d.objid = c.oid
        AND d.deptype = 'e'
    )
)
SELECT schema_name, name, comment
FROM ranked
WHERE row_number <= $1
ORDER BY schema_name, name
"#,
    )
    .bind(row_limit)
    .bind(schema_names)
    .fetch_all(&mut *conn)
    .await
    .map_err(read_database_error)?;
    push_uniform_entries(rows, schemas, truncated, "sequence", |objects| {
        &mut objects.sequences
    })
}

async fn load_routine_entries(
    conn: &mut sqlx::PgConnection,
    row_limit: i64,
    schema_names: &[String],
    schemas: &mut BTreeMap<String, PgSchemaObjects>,
    truncated: &mut Vec<PgCatalogTruncation>,
) -> Result<(), PgObjectError> {
    let rows = sqlx::query(
        r#"
WITH ranked AS (
  SELECT n.nspname::text AS schema_name,
         p.proname::text AS name,
         pg_get_function_identity_arguments(p.oid)::text AS identity_args,
         CASE p.prokind WHEN 'f' THEN 'function' WHEN 'p' THEN 'procedure' ELSE 'aggregate' END AS kind,
         obj_description(p.oid, 'pg_proc') AS comment,
         row_number() OVER (
           PARTITION BY n.nspname, p.prokind
           ORDER BY p.proname, pg_get_function_identity_arguments(p.oid)
         ) AS row_number
  FROM pg_proc p
  JOIN pg_namespace n ON n.oid = p.pronamespace
  WHERE p.prokind IN ('f', 'p', 'a')
    AND n.nspname = ANY($2::text[])
    AND n.nspname <> 'information_schema'
    AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
    AND NOT EXISTS (
      SELECT 1 FROM pg_depend d
      WHERE d.classid = 'pg_proc'::regclass
        AND d.objid = p.oid
        AND d.deptype = 'e'
    )
)
SELECT schema_name, name, identity_args, kind, comment
FROM ranked
WHERE row_number <= $1
ORDER BY schema_name, kind, name, identity_args
"#,
    )
    .bind(row_limit)
    .bind(schema_names)
    .fetch_all(&mut *conn)
    .await
    .map_err(read_database_error)?;

    for row in rows {
        let schema: String = row.try_get("schema_name").map_err(read_database_error)?;
        let kind: String = row.try_get("kind").map_err(read_database_error)?;
        let identity_args: String = row.try_get("identity_args").map_err(read_database_error)?;
        let entry = entry_from_row(&row, Some(identity_args), None)?;
        match kind.as_str() {
            "function" => push_schema_entry(schemas, truncated, &schema, &kind, entry, |objects| {
                &mut objects.functions
            }),
            "procedure" => {
                push_schema_entry(schemas, truncated, &schema, &kind, entry, |objects| {
                    &mut objects.procedures
                })
            }
            "aggregate" => {
                push_schema_entry(schemas, truncated, &schema, &kind, entry, |objects| {
                    &mut objects.aggregates
                })
            }
            _ => unreachable!("catalog routine query returned an unknown kind"),
        }
    }
    Ok(())
}

async fn load_type_entries(
    conn: &mut sqlx::PgConnection,
    row_limit: i64,
    schema_names: &[String],
    schemas: &mut BTreeMap<String, PgSchemaObjects>,
    truncated: &mut Vec<PgCatalogTruncation>,
) -> Result<(), PgObjectError> {
    let rows = sqlx::query(
        r#"
WITH ranked AS (
  SELECT n.nspname::text AS schema_name,
         t.typname::text AS name,
         CASE t.typtype WHEN 'e' THEN 'enum' WHEN 'c' THEN 'composite'
                        WHEN 'r' THEN 'range' ELSE 'multirange' END AS type_class,
         obj_description(t.oid, 'pg_type') AS comment,
         row_number() OVER (PARTITION BY n.nspname ORDER BY t.typname) AS row_number
  FROM pg_type t
  JOIN pg_namespace n ON n.oid = t.typnamespace
  LEFT JOIN pg_class type_relation ON type_relation.oid = t.typrelid
  WHERE t.typtype IN ('c', 'e', 'r', 'm')
    AND n.nspname = ANY($2::text[])
    AND (t.typtype <> 'c' OR type_relation.relkind = 'c')
    AND n.nspname <> 'information_schema'
    AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
    AND NOT EXISTS (
      SELECT 1 FROM pg_depend d
      WHERE d.classid = 'pg_type'::regclass
        AND d.objid = t.oid
        AND d.deptype = 'e'
    )
)
SELECT schema_name, name, type_class, comment
FROM ranked
WHERE row_number <= $1
ORDER BY schema_name, name
"#,
    )
    .bind(row_limit)
    .bind(schema_names)
    .fetch_all(&mut *conn)
    .await
    .map_err(read_database_error)?;

    for row in rows {
        let schema: String = row.try_get("schema_name").map_err(read_database_error)?;
        let type_class: String = row.try_get("type_class").map_err(read_database_error)?;
        let type_class = match type_class.as_str() {
            "enum" => PgTypeClass::Enum,
            "composite" => PgTypeClass::Composite,
            "range" => PgTypeClass::Range,
            "multirange" => PgTypeClass::Multirange,
            _ => unreachable!("catalog type query returned an unknown type class"),
        };
        let entry = entry_from_row(&row, None, Some(type_class))?;
        push_schema_entry(schemas, truncated, &schema, "type", entry, |objects| {
            &mut objects.types
        });
    }
    Ok(())
}

async fn load_domain_entries(
    conn: &mut sqlx::PgConnection,
    row_limit: i64,
    schema_names: &[String],
    schemas: &mut BTreeMap<String, PgSchemaObjects>,
    truncated: &mut Vec<PgCatalogTruncation>,
) -> Result<(), PgObjectError> {
    let rows = sqlx::query(
        r#"
WITH ranked AS (
  SELECT n.nspname::text AS schema_name,
         t.typname::text AS name,
         obj_description(t.oid, 'pg_type') AS comment,
         row_number() OVER (PARTITION BY n.nspname ORDER BY t.typname) AS row_number
  FROM pg_type t
  JOIN pg_namespace n ON n.oid = t.typnamespace
  WHERE t.typtype = 'd'
    AND n.nspname = ANY($2::text[])
    AND n.nspname <> 'information_schema'
    AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
    AND NOT EXISTS (
      SELECT 1 FROM pg_depend d
      WHERE d.classid = 'pg_type'::regclass
        AND d.objid = t.oid
        AND d.deptype = 'e'
    )
)
SELECT schema_name, name, comment
FROM ranked
WHERE row_number <= $1
ORDER BY schema_name, name
"#,
    )
    .bind(row_limit)
    .bind(schema_names)
    .fetch_all(&mut *conn)
    .await
    .map_err(read_database_error)?;
    push_uniform_entries(rows, schemas, truncated, "domain", |objects| {
        &mut objects.domains
    })
}

async fn load_extension_entries(
    conn: &mut sqlx::PgConnection,
    row_limit: i64,
    schema_names: &[String],
    schemas: &mut BTreeMap<String, PgSchemaObjects>,
    truncated: &mut Vec<PgCatalogTruncation>,
) -> Result<(), PgObjectError> {
    let rows = sqlx::query(
        r#"
WITH ranked AS (
  SELECT n.nspname::text AS schema_name,
         e.extname::text AS name,
         obj_description(e.oid, 'pg_extension') AS comment,
         row_number() OVER (PARTITION BY n.nspname ORDER BY e.extname) AS row_number
  FROM pg_extension e
  JOIN pg_namespace n ON n.oid = e.extnamespace
  WHERE n.nspname <> 'information_schema'
    AND n.nspname = ANY($2::text[])
    AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
)
SELECT schema_name, name, comment
FROM ranked
WHERE row_number <= $1
ORDER BY schema_name, name
"#,
    )
    .bind(row_limit)
    .bind(schema_names)
    .fetch_all(&mut *conn)
    .await
    .map_err(read_database_error)?;
    push_uniform_entries(rows, schemas, truncated, "extension", |objects| {
        &mut objects.extensions
    })
}

fn push_uniform_entries(
    rows: Vec<sqlx::postgres::PgRow>,
    schemas: &mut BTreeMap<String, PgSchemaObjects>,
    truncated: &mut Vec<PgCatalogTruncation>,
    kind: &str,
    select: fn(&mut PgSchemaObjects) -> &mut Vec<PgCatalogEntry>,
) -> Result<(), PgObjectError> {
    for row in rows {
        let schema: String = row.try_get("schema_name").map_err(read_database_error)?;
        let entry = entry_from_row(&row, None, None)?;
        push_schema_entry(schemas, truncated, &schema, kind, entry, select);
    }
    Ok(())
}

fn push_schema_entry(
    schemas: &mut BTreeMap<String, PgSchemaObjects>,
    truncated: &mut Vec<PgCatalogTruncation>,
    schema: &str,
    kind: &str,
    entry: PgCatalogEntry,
    select: impl FnOnce(&mut PgSchemaObjects) -> &mut Vec<PgCatalogEntry>,
) {
    let Some(objects) = schemas.get_mut(schema) else {
        return;
    };
    let entries = select(objects);
    if entries.len() < CATALOG_KIND_CAP {
        entries.push(entry);
    } else {
        record_truncation(truncated, Some(schema.to_string()), kind);
    }
}

async fn load_database_entries(
    conn: &mut sqlx::PgConnection,
    row_limit: i64,
    sql: &str,
    kind: &str,
    truncated: &mut Vec<PgCatalogTruncation>,
) -> Result<Vec<PgCatalogEntry>, PgObjectError> {
    let rows = sqlx::query(sql)
        .bind(row_limit)
        .fetch_all(&mut *conn)
        .await
        .map_err(read_database_error)?;
    let mut entries = Vec::with_capacity(rows.len().min(CATALOG_KIND_CAP));
    for row in rows {
        if entries.len() == CATALOG_KIND_CAP {
            record_truncation(truncated, None, kind);
            break;
        }
        entries.push(entry_from_row(&row, None, None)?);
    }
    Ok(entries)
}

fn entry_from_row(
    row: &sqlx::postgres::PgRow,
    identity_args: Option<String>,
    type_class: Option<PgTypeClass>,
) -> Result<PgCatalogEntry, PgObjectError> {
    Ok(PgCatalogEntry {
        name: row.try_get("name").map_err(read_database_error)?,
        identity_args,
        comment: row.try_get("comment").map_err(read_database_error)?,
        type_class,
    })
}

fn record_truncation(truncated: &mut Vec<PgCatalogTruncation>, schema: Option<String>, kind: &str) {
    if truncated
        .iter()
        .any(|item| item.schema == schema && item.kind == kind)
    {
        return;
    }
    truncated.push(PgCatalogTruncation {
        schema,
        kind: kind.to_string(),
    });
}

pub(crate) fn read_database_error(error: sqlx::Error) -> PgObjectError {
    let (code, message) = match error.as_database_error() {
        Some(database) => (
            database.code().map(|code| code.into_owned()),
            database.message().to_string(),
        ),
        None => (None, error.to_string()),
    };
    PgObjectError::Database {
        statement_index: None,
        code,
        message,
        // PostgreSQL reports character positions relative to the submitted
        // query. Read helpers do not retain that text, so exposing the raw
        // one-based character position would violate the byte-offset wire
        // contract used by apply errors.
        position: None,
        applied_statements: 0,
        residue: None,
    }
}

pub(crate) async fn describe_pg_object(
    connection: &StoredConnection,
    reference: PgObjectRef,
) -> Result<PgObjectDescription, PgObjectError> {
    ensure_postgres(connection)?;
    validate_read_reference(&reference)?;
    let mut conn = super::connect(connection)
        .await
        .map_err(|message| PgObjectError::Connection { message })?;
    let mut transaction = conn.begin().await.map_err(read_database_error)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(read_database_error)?;

    let result = match reference.kind {
        PgObjectKind::Schema => describe_schema(&mut transaction, reference).await,
        PgObjectKind::Table
        | PgObjectKind::View
        | PgObjectKind::MaterializedView
        | PgObjectKind::ForeignTable => describe_relation(&mut transaction, reference).await,
        PgObjectKind::Sequence => describe_sequence(&mut transaction, reference).await,
        PgObjectKind::Function | PgObjectKind::Procedure | PgObjectKind::Aggregate => {
            describe_routine(&mut transaction, reference).await
        }
        PgObjectKind::Type => describe_type(&mut transaction, reference).await,
        PgObjectKind::Domain => describe_domain(&mut transaction, reference).await,
        PgObjectKind::Extension => describe_extension(&mut transaction, reference).await,
    }?;
    transaction.commit().await.map_err(read_database_error)?;
    Ok(result)
}

fn validate_read_reference(reference: &PgObjectRef) -> Result<(), PgObjectError> {
    let valid_schema = match reference.kind {
        PgObjectKind::Schema => reference.schema.is_none(),
        _ => reference
            .schema
            .as_ref()
            .is_some_and(|schema| !schema.trim().is_empty()),
    };
    let routine_identity = match reference.kind {
        PgObjectKind::Function | PgObjectKind::Procedure | PgObjectKind::Aggregate => {
            reference.identity_args.is_some()
        }
        _ => reference.identity_args.is_none(),
    };
    if reference.name.trim().is_empty() || !valid_schema || !routine_identity {
        return Err(PgObjectError::ObjectNotFound {
            reference: reference.clone(),
        });
    }
    Ok(())
}

async fn describe_schema(
    conn: &mut sqlx::PgConnection,
    reference: PgObjectRef,
) -> Result<PgObjectDescription, PgObjectError> {
    let row = sqlx::query(
        r#"
SELECT pg_get_userbyid(n.nspowner)::text AS owner,
       obj_description(n.oid, 'pg_namespace') AS comment
FROM pg_namespace n
WHERE n.nspname = $1
"#,
    )
    .bind(&reference.name)
    .fetch_optional(&mut *conn)
    .await
    .map_err(read_database_error)?
    .ok_or_else(|| object_not_found(&reference))?;
    let owner = row.try_get("owner").map_err(read_database_error)?;
    let comment = row.try_get("comment").map_err(read_database_error)?;
    Ok(PgObjectDescription {
        definition_sql: Some(format!(
            "CREATE SCHEMA {};",
            crate::quote_double(&reference.name)
        )),
        reference,
        owner,
        comment,
        facts: PgObjectFacts::Schema,
    })
}

async fn describe_relation(
    conn: &mut sqlx::PgConnection,
    reference: PgObjectRef,
) -> Result<PgObjectDescription, PgObjectError> {
    let schema = reference_schema(&reference)?;
    let expected_kinds: &[&str] = match reference.kind {
        PgObjectKind::Table => &["r", "p"],
        PgObjectKind::View => &["v"],
        PgObjectKind::MaterializedView => &["m"],
        PgObjectKind::ForeignTable => &["f"],
        _ => unreachable!("describe_relation called for a non-relation"),
    };
    let row = sqlx::query(
        r#"
SELECT c.oid::bigint AS object_id,
       c.relkind::text AS relation_kind,
       pg_get_userbyid(c.relowner)::text AS owner,
       obj_description(c.oid, 'pg_class') AS comment,
       c.relispopulated,
       CASE WHEN c.relkind IN ('v', 'm') THEN pg_get_viewdef(c.oid, true) END AS view_definition
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind::text = ANY($3)
"#,
    )
    .bind(schema)
    .bind(&reference.name)
    .bind(expected_kinds)
    .fetch_optional(&mut *conn)
    .await
    .map_err(read_database_error)?
    .ok_or_else(|| object_not_found(&reference))?;
    let owner = row.try_get("owner").map_err(read_database_error)?;
    let comment = row.try_get("comment").map_err(read_database_error)?;

    let (definition_sql, facts) = match reference.kind {
        PgObjectKind::Table => (
            Some(
                super::ddl::export_relation_ddl(conn, schema, &reference.name)
                    .await
                    .map_err(read_string_error)?,
            ),
            PgObjectFacts::Table,
        ),
        PgObjectKind::View => {
            let raw_definition: String = row
                .try_get("view_definition")
                .map_err(read_database_error)?;
            let definition = trim_view_definition(&raw_definition);
            (
                Some(format!(
                    "CREATE VIEW {} AS\n{};",
                    qualified(schema, &reference.name),
                    definition
                )),
                PgObjectFacts::View { definition },
            )
        }
        PgObjectKind::MaterializedView => {
            let raw_definition: String = row
                .try_get("view_definition")
                .map_err(read_database_error)?;
            let definition = trim_view_definition(&raw_definition);
            let populated = row.try_get("relispopulated").map_err(read_database_error)?;
            let data_clause = if populated {
                "WITH DATA"
            } else {
                "WITH NO DATA"
            };
            (
                Some(format!(
                    "CREATE MATERIALIZED VIEW {} AS\n{} {data_clause};",
                    qualified(schema, &reference.name),
                    definition,
                )),
                PgObjectFacts::MaterializedView {
                    definition,
                    populated,
                },
            )
        }
        PgObjectKind::ForeignTable => {
            let (definition, server) =
                describe_foreign_table(conn, schema, &reference.name).await?;
            (Some(definition), PgObjectFacts::ForeignTable { server })
        }
        _ => unreachable!("describe_relation called for a non-relation"),
    };

    Ok(PgObjectDescription {
        reference,
        owner,
        comment,
        definition_sql,
        facts,
    })
}

fn trim_view_definition(definition: &str) -> String {
    definition
        .trim()
        .strip_suffix(';')
        .unwrap_or(definition.trim())
        .trim_end()
        .to_string()
}

async fn describe_foreign_table(
    conn: &mut sqlx::PgConnection,
    schema: &str,
    table: &str,
) -> Result<(String, String), PgObjectError> {
    let foreign_table = sqlx::query(
        r#"
SELECT c.oid::bigint AS object_id,
       s.srvname::text AS server,
       f.ftoptions
FROM pg_foreign_table f
JOIN pg_class c ON c.oid = f.ftrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_foreign_server s ON s.oid = f.ftserver
WHERE n.nspname = $1 AND c.relname = $2
"#,
    )
    .bind(schema)
    .bind(table)
    .fetch_one(&mut *conn)
    .await
    .map_err(read_database_error)?;
    let server: String = foreign_table
        .try_get("server")
        .map_err(read_database_error)?;
    let object_id: i64 = foreign_table
        .try_get("object_id")
        .map_err(read_database_error)?;
    let table_options: Option<Vec<String>> = foreign_table
        .try_get("ftoptions")
        .map_err(read_database_error)?;
    let columns = sqlx::query(
        r#"
SELECT a.attname::text AS name,
       format_type(a.atttypid, a.atttypmod)::text AS data_type,
       a.attnotnull,
       a.attfdwoptions,
       pg_get_expr(default_value.adbin, default_value.adrelid, true)::text AS default_value,
       collation_namespace.nspname::text AS collation_schema,
       column_collation.collname::text AS collation_name
FROM pg_attribute a
JOIN pg_class c ON c.oid = a.attrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_type attribute_type ON attribute_type.oid = a.atttypid
LEFT JOIN pg_attrdef default_value
  ON default_value.adrelid = a.attrelid AND default_value.adnum = a.attnum
LEFT JOIN pg_collation column_collation
  ON column_collation.oid = a.attcollation
 AND a.attcollation <> attribute_type.typcollation
LEFT JOIN pg_namespace collation_namespace
  ON collation_namespace.oid = column_collation.collnamespace
WHERE n.nspname = $1 AND c.relname = $2
  AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum
"#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut *conn)
    .await
    .map_err(read_database_error)?;
    let mut definitions = Vec::with_capacity(columns.len());
    for column in columns {
        let name: String = column.try_get("name").map_err(read_database_error)?;
        let data_type: String = column.try_get("data_type").map_err(read_database_error)?;
        let not_null: bool = column.try_get("attnotnull").map_err(read_database_error)?;
        let options: Option<Vec<String>> = column
            .try_get("attfdwoptions")
            .map_err(read_database_error)?;
        let default_value: Option<String> = column
            .try_get("default_value")
            .map_err(read_database_error)?;
        let collation_schema: Option<String> = column
            .try_get("collation_schema")
            .map_err(read_database_error)?;
        let collation_name: Option<String> = column
            .try_get("collation_name")
            .map_err(read_database_error)?;
        let mut definition = format!(
            "  {} {}{}",
            crate::quote_double(&name),
            data_type,
            render_fdw_options(options.as_deref())?,
        );
        if let Some((collation_schema, collation_name)) = collation_schema.zip(collation_name) {
            definition.push_str(&format!(
                " COLLATE {}",
                qualified(&collation_schema, &collation_name)
            ));
        }
        if let Some(default_value) = default_value {
            definition.push_str(&format!(" DEFAULT {default_value}"));
        }
        if not_null {
            definition.push_str(" NOT NULL");
        }
        definitions.push(definition);
    }
    let checks = sqlx::query(
        r#"
SELECT constraint_row.conname::text AS name,
       pg_get_constraintdef(constraint_row.oid, true)::text AS definition
FROM pg_constraint constraint_row
WHERE constraint_row.conrelid = $1::oid
  AND constraint_row.contype = 'c'
ORDER BY constraint_row.conname
"#,
    )
    .bind(object_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(read_database_error)?;
    for check in checks {
        let name: String = check.try_get("name").map_err(read_database_error)?;
        let definition: String = check.try_get("definition").map_err(read_database_error)?;
        definitions.push(format!(
            "  CONSTRAINT {} {definition}",
            crate::quote_double(&name)
        ));
    }
    let table_options = render_fdw_options(table_options.as_deref())?;
    Ok((
        format!(
            "CREATE FOREIGN TABLE {} (\n{}\n) SERVER {}{};",
            qualified(schema, table),
            definitions.join(",\n"),
            crate::quote_double(&server),
            table_options
        ),
        server,
    ))
}

fn render_fdw_options(options: Option<&[String]>) -> Result<String, PgObjectError> {
    let Some(options) = options.filter(|options| !options.is_empty()) else {
        return Ok(String::new());
    };
    let rendered = options
        .iter()
        .map(|option| {
            let (name, value) = option
                .split_once('=')
                .filter(|(name, _)| !name.is_empty())
                .ok_or_else(|| {
                    read_string_error(format!("invalid foreign-data option: {option}"))
                })?;
            Ok(format!(
                "{} {}",
                crate::quote_double(name),
                crate::quote_literal(value)
            ))
        })
        .collect::<Result<Vec<_>, PgObjectError>>()?;
    Ok(format!(" OPTIONS ({})", rendered.join(", ")))
}

fn object_not_found(reference: &PgObjectRef) -> PgObjectError {
    PgObjectError::ObjectNotFound {
        reference: reference.clone(),
    }
}

fn reference_schema(reference: &PgObjectRef) -> Result<&str, PgObjectError> {
    reference
        .schema
        .as_deref()
        .ok_or_else(|| object_not_found(reference))
}

fn qualified(schema: &str, name: &str) -> String {
    format!(
        "{}.{}",
        crate::quote_double(schema),
        crate::quote_double(name)
    )
}

fn read_string_error(message: String) -> PgObjectError {
    PgObjectError::Database {
        statement_index: None,
        code: None,
        message,
        position: None,
        applied_statements: 0,
        residue: None,
    }
}

async fn describe_sequence(
    conn: &mut sqlx::PgConnection,
    reference: PgObjectRef,
) -> Result<PgObjectDescription, PgObjectError> {
    let schema = reference_schema(&reference)?;
    let row = sqlx::query(
        r#"
SELECT pg_get_userbyid(c.relowner)::text AS owner,
       obj_description(c.oid, 'pg_class') AS comment,
       s.data_type::text,
       s.start_value::text AS start_value,
       s.increment_by::text AS increment_by,
       s.min_value::text AS min_value,
       s.max_value::text AS max_value,
       s.cycle,
       s.cache_size::text AS cache_size,
       s.last_value::text AS last_value,
       owned.owned_schema,
       owned.owned_table,
       owned.owned_column
FROM pg_sequences s
JOIN pg_namespace n ON n.nspname = s.schemaname
JOIN pg_class c ON c.relnamespace = n.oid AND c.relname = s.sequencename AND c.relkind = 'S'
LEFT JOIN LATERAL (
  SELECT target_ns.nspname::text AS owned_schema,
         target.relname::text AS owned_table,
         attribute.attname::text AS owned_column
  FROM pg_depend dependency
  JOIN pg_class target ON target.oid = dependency.refobjid
  JOIN pg_namespace target_ns ON target_ns.oid = target.relnamespace
  JOIN pg_attribute attribute
    ON attribute.attrelid = target.oid AND attribute.attnum = dependency.refobjsubid
  WHERE dependency.classid = 'pg_class'::regclass
    AND dependency.objid = c.oid
    AND dependency.deptype IN ('a', 'i')
  LIMIT 1
) owned ON true
WHERE s.schemaname = $1 AND s.sequencename = $2
"#,
    )
    .bind(schema)
    .bind(&reference.name)
    .fetch_optional(&mut *conn)
    .await
    .map_err(read_database_error)?
    .ok_or_else(|| object_not_found(&reference))?;

    let owner = row.try_get("owner").map_err(read_database_error)?;
    let comment = row.try_get("comment").map_err(read_database_error)?;
    let data_type = row.try_get("data_type").map_err(read_database_error)?;
    let start = row.try_get("start_value").map_err(read_database_error)?;
    let increment = row.try_get("increment_by").map_err(read_database_error)?;
    let min_value = row.try_get("min_value").map_err(read_database_error)?;
    let max_value = row.try_get("max_value").map_err(read_database_error)?;
    let cycle = row.try_get("cycle").map_err(read_database_error)?;
    let cache = row.try_get("cache_size").map_err(read_database_error)?;
    let last_value = row.try_get("last_value").map_err(read_database_error)?;
    let owned_schema: Option<String> = row.try_get("owned_schema").map_err(read_database_error)?;
    let owned_table: Option<String> = row.try_get("owned_table").map_err(read_database_error)?;
    let owned_column: Option<String> = row.try_get("owned_column").map_err(read_database_error)?;
    let owned_parts = owned_schema
        .as_deref()
        .zip(owned_table.as_deref())
        .zip(owned_column.as_deref())
        .map(|((schema, table), column)| (schema, table, column));
    let owned_by = owned_parts.map(|(schema, table, column)| format!("{schema}.{table}.{column}"));
    let mut definition = format!(
        "CREATE SEQUENCE {} AS {} INCREMENT BY {} MINVALUE {} MAXVALUE {} START WITH {} CACHE {} {};",
        qualified(schema, &reference.name),
        data_type,
        increment,
        min_value,
        max_value,
        start,
        cache,
        if cycle { "CYCLE" } else { "NO CYCLE" }
    );
    if let Some((owned_schema, owned_table, owned_column)) = owned_parts {
        definition.push_str(&format!(
            "\nALTER SEQUENCE {} OWNED BY {};",
            qualified(schema, &reference.name),
            render_sequence_owner(owned_schema, owned_table, owned_column)
        ));
    }
    Ok(PgObjectDescription {
        reference,
        owner,
        comment,
        definition_sql: Some(definition),
        facts: PgObjectFacts::Sequence {
            data_type,
            start,
            increment,
            min_value,
            max_value,
            cycle,
            cache,
            last_value,
            owned_by,
        },
    })
}

fn render_sequence_owner(schema: &str, table: &str, column: &str) -> String {
    format!(
        "{}.{}.{}",
        crate::quote_double(schema),
        crate::quote_double(table),
        crate::quote_double(column)
    )
}

async fn describe_routine(
    conn: &mut sqlx::PgConnection,
    reference: PgObjectRef,
) -> Result<PgObjectDescription, PgObjectError> {
    let schema = reference_schema(&reference)?;
    let prokind = match reference.kind {
        PgObjectKind::Function => "f",
        PgObjectKind::Procedure => "p",
        PgObjectKind::Aggregate => "a",
        _ => unreachable!("describe_routine called for a non-routine"),
    };
    let row = sqlx::query(
        r#"
SELECT p.oid::bigint AS object_id,
       pg_get_userbyid(p.proowner)::text AS owner,
       obj_description(p.oid, 'pg_proc') AS comment,
       language.lanname::text AS language,
       CASE WHEN p.prokind = 'p' THEN NULL ELSE pg_get_function_result(p.oid) END AS returns,
       CASE p.provolatile WHEN 'i' THEN 'immutable' WHEN 's' THEN 'stable' ELSE 'volatile' END AS volatility,
       pg_get_function_arguments(p.oid)::text AS arguments,
       CASE WHEN p.prokind = 'a' THEN NULL ELSE pg_get_functiondef(p.oid) END AS definition
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
JOIN pg_language language ON language.oid = p.prolang
WHERE n.nspname = $1
  AND p.proname = $2
  AND p.prokind::text = $3
  AND pg_get_function_identity_arguments(p.oid) = $4
"#,
    )
    .bind(schema)
    .bind(&reference.name)
    .bind(prokind)
    .bind(reference.identity_args.as_deref().unwrap_or_default())
    .fetch_optional(&mut *conn)
    .await
    .map_err(read_database_error)?
    .ok_or_else(|| object_not_found(&reference))?;
    let definition_sql = row.try_get("definition").map_err(read_database_error)?;
    Ok(PgObjectDescription {
        reference,
        owner: row.try_get("owner").map_err(read_database_error)?,
        comment: row.try_get("comment").map_err(read_database_error)?,
        definition_sql,
        facts: PgObjectFacts::Routine {
            language: row.try_get("language").map_err(read_database_error)?,
            returns: row.try_get("returns").map_err(read_database_error)?,
            volatility: row.try_get("volatility").map_err(read_database_error)?,
            arguments: row.try_get("arguments").map_err(read_database_error)?,
        },
    })
}

async fn describe_type(
    conn: &mut sqlx::PgConnection,
    reference: PgObjectRef,
) -> Result<PgObjectDescription, PgObjectError> {
    let schema = reference_schema(&reference)?;
    let row = sqlx::query(
        r#"
SELECT t.oid::bigint AS object_id,
       t.typtype::text AS type_kind,
       pg_get_userbyid(t.typowner)::text AS owner,
       obj_description(t.oid, 'pg_type') AS comment
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace
LEFT JOIN pg_class type_relation ON type_relation.oid = t.typrelid
WHERE n.nspname = $1 AND t.typname = $2
  AND t.typtype IN ('c', 'e', 'r', 'm')
  AND (t.typtype <> 'c' OR type_relation.relkind = 'c')
"#,
    )
    .bind(schema)
    .bind(&reference.name)
    .fetch_optional(&mut *conn)
    .await
    .map_err(read_database_error)?
    .ok_or_else(|| object_not_found(&reference))?;
    let object_id: i64 = row.try_get("object_id").map_err(read_database_error)?;
    let type_kind: String = row.try_get("type_kind").map_err(read_database_error)?;
    let owner = row.try_get("owner").map_err(read_database_error)?;
    let comment = row.try_get("comment").map_err(read_database_error)?;

    let (class, enum_labels, attributes, subtype, definition_sql) = match type_kind.as_str() {
        "e" => {
            let labels = sqlx::query_scalar::<_, String>(
                "SELECT enumlabel::text FROM pg_enum WHERE enumtypid = $1::oid ORDER BY enumsortorder",
            )
            .bind(object_id)
            .fetch_all(&mut *conn)
            .await
            .map_err(read_database_error)?;
            let definition = format!(
                "CREATE TYPE {} AS ENUM ({});",
                qualified(schema, &reference.name),
                labels
                    .iter()
                    .map(|label| crate::quote_literal(label))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            (PgTypeClass::Enum, Some(labels), None, None, definition)
        }
        "c" => {
            let rows = sqlx::query(
                r#"
SELECT a.attname::text AS name,
       format_type(a.atttypid, a.atttypmod)::text AS data_type,
       NOT a.attnotnull AS nullable
FROM pg_type t
JOIN pg_attribute a ON a.attrelid = t.typrelid
WHERE t.oid = $1::oid AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum
"#,
            )
            .bind(object_id)
            .fetch_all(&mut *conn)
            .await
            .map_err(read_database_error)?;
            let mut values = Vec::with_capacity(rows.len());
            for row in rows {
                values.push(PgTypeAttribute {
                    name: row.try_get("name").map_err(read_database_error)?,
                    data_type: row.try_get("data_type").map_err(read_database_error)?,
                    nullable: row.try_get("nullable").map_err(read_database_error)?,
                });
            }
            let definition = format!(
                "CREATE TYPE {} AS (\n{}\n);",
                qualified(schema, &reference.name),
                values
                    .iter()
                    .map(|attribute| format!(
                        "  {} {}",
                        crate::quote_double(&attribute.name),
                        attribute.data_type
                    ))
                    .collect::<Vec<_>>()
                    .join(",\n")
            );
            (PgTypeClass::Composite, None, Some(values), None, definition)
        }
        "r" | "m" => {
            // pg_range.rngmultitypid only exists on PostgreSQL 14+; older
            // servers still describe range types, just without the
            // MULTIRANGE_TYPE_NAME option.
            let multirange_supported = server_version_num(conn).await? >= 140_000;
            let (multirange_join, multirange_filter) = if multirange_supported {
                (
                    "LEFT JOIN pg_type multirange_name ON multirange_name.oid = range_type.rngmultitypid",
                    "($2 = 'm' AND range_type.rngmultitypid = $1::oid)",
                )
            } else {
                ("LEFT JOIN pg_type multirange_name ON false", "false")
            };
            let range_sql = format!(
                r#"
SELECT format_type(range_type.rngsubtype, NULL)::text AS subtype,
       range_name.typname::text AS range_name,
       range_namespace.nspname::text AS range_schema,
       subtype_name.typname::text AS subtype_name,
       subtype_namespace.nspname::text AS subtype_schema,
       multirange_name.typname::text AS multirange_name,
       multirange_namespace.nspname::text AS multirange_schema,
       operator_class.opcname::text AS operator_class_name,
       operator_namespace.nspname::text AS operator_class_schema,
       range_collation.collname::text AS collation_name,
       collation_namespace.nspname::text AS collation_schema,
       canonical_proc.proname::text AS canonical_name,
       canonical_namespace.nspname::text AS canonical_schema,
       subtype_diff.proname::text AS subtype_diff_name,
       subtype_diff_namespace.nspname::text AS subtype_diff_schema
FROM pg_range range_type
JOIN pg_type range_name ON range_name.oid = range_type.rngtypid
JOIN pg_namespace range_namespace ON range_namespace.oid = range_name.typnamespace
JOIN pg_type subtype_name ON subtype_name.oid = range_type.rngsubtype
JOIN pg_namespace subtype_namespace ON subtype_namespace.oid = subtype_name.typnamespace
{multirange_join}
LEFT JOIN pg_namespace multirange_namespace ON multirange_namespace.oid = multirange_name.typnamespace
JOIN pg_opclass operator_class ON operator_class.oid = range_type.rngsubopc
JOIN pg_namespace operator_namespace ON operator_namespace.oid = operator_class.opcnamespace
LEFT JOIN pg_collation range_collation ON range_collation.oid = NULLIF(range_type.rngcollation, 0)
LEFT JOIN pg_namespace collation_namespace ON collation_namespace.oid = range_collation.collnamespace
LEFT JOIN pg_proc canonical_proc ON canonical_proc.oid = NULLIF(range_type.rngcanonical, 0)
LEFT JOIN pg_namespace canonical_namespace ON canonical_namespace.oid = canonical_proc.pronamespace
LEFT JOIN pg_proc subtype_diff ON subtype_diff.oid = NULLIF(range_type.rngsubdiff, 0)
LEFT JOIN pg_namespace subtype_diff_namespace ON subtype_diff_namespace.oid = subtype_diff.pronamespace
WHERE ($2 = 'r' AND range_type.rngtypid = $1::oid)
   OR {multirange_filter}
"#
            );
            let range = sqlx::query(&range_sql)
                .bind(object_id)
                .bind(&type_kind)
                .fetch_one(&mut *conn)
                .await
                .map_err(read_database_error)?;
            let subtype: String = range.try_get("subtype").map_err(read_database_error)?;
            let range_name: String = range.try_get("range_name").map_err(read_database_error)?;
            let range_schema: String =
                range.try_get("range_schema").map_err(read_database_error)?;
            let subtype_name: String =
                range.try_get("subtype_name").map_err(read_database_error)?;
            let subtype_schema: String = range
                .try_get("subtype_schema")
                .map_err(read_database_error)?;
            let multirange_name: Option<String> = range
                .try_get("multirange_name")
                .map_err(read_database_error)?;
            let multirange_schema: Option<String> = range
                .try_get("multirange_schema")
                .map_err(read_database_error)?;
            let class = if type_kind == "r" {
                PgTypeClass::Range
            } else {
                PgTypeClass::Multirange
            };
            let mut options = vec![format!(
                "SUBTYPE = {}",
                qualified(&subtype_schema, &subtype_name)
            )];
            push_qualified_range_option(
                &mut options,
                "SUBTYPE_OPCLASS",
                range
                    .try_get::<Option<String>, _>("operator_class_schema")
                    .map_err(read_database_error)?,
                range
                    .try_get::<Option<String>, _>("operator_class_name")
                    .map_err(read_database_error)?,
            );
            push_qualified_range_option(
                &mut options,
                "COLLATION",
                range
                    .try_get::<Option<String>, _>("collation_schema")
                    .map_err(read_database_error)?,
                range
                    .try_get::<Option<String>, _>("collation_name")
                    .map_err(read_database_error)?,
            );
            push_qualified_range_option(
                &mut options,
                "CANONICAL",
                range
                    .try_get::<Option<String>, _>("canonical_schema")
                    .map_err(read_database_error)?,
                range
                    .try_get::<Option<String>, _>("canonical_name")
                    .map_err(read_database_error)?,
            );
            push_qualified_range_option(
                &mut options,
                "SUBTYPE_DIFF",
                range
                    .try_get::<Option<String>, _>("subtype_diff_schema")
                    .map_err(read_database_error)?,
                range
                    .try_get::<Option<String>, _>("subtype_diff_name")
                    .map_err(read_database_error)?,
            );
            let multirange_identity = multirange_schema
                .as_deref()
                .zip(multirange_name.as_deref())
                .map(|(schema, name)| qualified(schema, name));
            push_qualified_range_option(
                &mut options,
                "MULTIRANGE_TYPE_NAME",
                multirange_schema,
                multirange_name,
            );
            let definition_name = if type_kind == "r" {
                qualified(schema, &reference.name)
            } else {
                qualified(&range_schema, &range_name)
            };
            let definition = format!(
                "CREATE TYPE {definition_name} AS RANGE (\n  {}\n);",
                options.join(",\n  ")
            );
            if type_kind == "m" {
                debug_assert_eq!(
                    Some(qualified(schema, &reference.name)),
                    multirange_identity
                );
            }
            (class, None, None, Some(subtype), definition)
        }
        _ => unreachable!("type lookup returned an unknown class"),
    };
    Ok(PgObjectDescription {
        reference,
        owner,
        comment,
        definition_sql: Some(definition_sql),
        facts: PgObjectFacts::Type {
            class,
            enum_labels,
            attributes,
            subtype,
        },
    })
}

async fn server_version_num(conn: &mut sqlx::PgConnection) -> Result<i64, PgObjectError> {
    sqlx::query_scalar::<_, String>("SELECT current_setting('server_version_num')")
        .fetch_one(&mut *conn)
        .await
        .map_err(read_database_error)?
        .parse::<i64>()
        .map_err(|error| PgObjectError::Connection {
            message: format!("unreadable server_version_num: {error}"),
        })
}

fn push_qualified_range_option(
    options: &mut Vec<String>,
    label: &str,
    schema: Option<String>,
    name: Option<String>,
) {
    if let Some((schema, name)) = schema.zip(name) {
        options.push(format!("{label} = {}", qualified(&schema, &name)));
    }
}

async fn describe_domain(
    conn: &mut sqlx::PgConnection,
    reference: PgObjectRef,
) -> Result<PgObjectDescription, PgObjectError> {
    let schema = reference_schema(&reference)?;
    let row = sqlx::query(
        r#"
SELECT t.oid::bigint AS object_id,
       pg_get_userbyid(t.typowner)::text AS owner,
       obj_description(t.oid, 'pg_type') AS comment,
       format_type(t.typbasetype, t.typtypmod)::text AS base_type,
       t.typnotnull AS not_null,
       t.typdefault::text AS default_value
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace
WHERE n.nspname = $1 AND t.typname = $2 AND t.typtype = 'd'
"#,
    )
    .bind(schema)
    .bind(&reference.name)
    .fetch_optional(&mut *conn)
    .await
    .map_err(read_database_error)?
    .ok_or_else(|| object_not_found(&reference))?;
    let object_id: i64 = row.try_get("object_id").map_err(read_database_error)?;
    let base_type: String = row.try_get("base_type").map_err(read_database_error)?;
    let not_null: bool = row.try_get("not_null").map_err(read_database_error)?;
    let default_value: Option<String> =
        row.try_get("default_value").map_err(read_database_error)?;
    let checks = sqlx::query_scalar::<_, String>(
        r#"
SELECT pg_get_constraintdef(c.oid, true)::text
FROM pg_constraint c
WHERE c.contypid = $1::oid
ORDER BY c.conname
"#,
    )
    .bind(object_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(read_database_error)?;
    let mut definition = format!(
        "CREATE DOMAIN {} AS {}",
        qualified(schema, &reference.name),
        base_type
    );
    if let Some(default_value) = &default_value {
        definition.push_str(&format!(" DEFAULT {default_value}"));
    }
    if not_null {
        definition.push_str(" NOT NULL");
    }
    for check in &checks {
        definition.push_str(&format!("\n  {check}"));
    }
    definition.push(';');
    Ok(PgObjectDescription {
        reference,
        owner: row.try_get("owner").map_err(read_database_error)?,
        comment: row.try_get("comment").map_err(read_database_error)?,
        definition_sql: Some(definition),
        facts: PgObjectFacts::Domain {
            base_type,
            not_null,
            default_value,
            checks,
        },
    })
}

async fn describe_extension(
    conn: &mut sqlx::PgConnection,
    reference: PgObjectRef,
) -> Result<PgObjectDescription, PgObjectError> {
    let row = sqlx::query(
        r#"
SELECT pg_get_userbyid(e.extowner)::text AS owner,
       obj_description(e.oid, 'pg_extension') AS comment,
       e.extversion::text AS version,
       n.nspname::text AS schema_name
FROM pg_extension e
JOIN pg_namespace n ON n.oid = e.extnamespace
WHERE e.extname = $1
"#,
    )
    .bind(&reference.name)
    .fetch_optional(&mut *conn)
    .await
    .map_err(read_database_error)?
    .ok_or_else(|| object_not_found(&reference))?;
    let version: String = row.try_get("version").map_err(read_database_error)?;
    let schema: String = row.try_get("schema_name").map_err(read_database_error)?;
    Ok(PgObjectDescription {
        definition_sql: Some(format!(
            "CREATE EXTENSION {} WITH SCHEMA {} VERSION {};",
            crate::quote_double(&reference.name),
            crate::quote_double(&schema),
            crate::quote_literal(&version)
        )),
        reference,
        owner: row.try_get("owner").map_err(read_database_error)?,
        comment: row.try_get("comment").map_err(read_database_error)?,
        facts: PgObjectFacts::Extension { version, schema },
    })
}

const DROP_IMPACT_DEPTH_CAP: u32 = 8;
const DROP_IMPACT_RESULT_CAP: usize = 200;
const DROP_IMPACT_ADDRESS_CAP_PER_DEPTH: usize = 201;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct DropAddress {
    class_id: i64,
    object_id: i64,
    sub_id: i64,
}

/// A dependent found by one breadth of the walk. `reported` mirrors what
/// PostgreSQL itself discloses for `DROP ... CASCADE`: normal (`n`)
/// dependents are listed, while auto (`a`) and internal (`i`) dependents are
/// dropped silently because they are parts of their owner (column defaults,
/// constraints, indexes, row types). Owned and identity sequences are the one
/// exception, since they are separate relations whose loss operators expect
/// to see. Silent dependents are still walked so that anything depending on
/// them is discovered.
#[derive(Debug, Clone, Copy)]
struct DropCandidate {
    address: DropAddress,
    reported: bool,
}

const DROP_CANDIDATES_SQL: &str = r#"
WITH frontier(class_id, object_id, sub_id) AS MATERIALIZED (
  SELECT * FROM UNNEST($1::bigint[], $2::bigint[], $3::bigint[])
),
visited(class_id, object_id, sub_id) AS MATERIALIZED (
  SELECT * FROM UNNEST($4::bigint[], $5::bigint[], $6::bigint[])
),
candidate_rows AS MATERIALIZED (
  SELECT CASE WHEN rewrite.rulename = '_RETURN'
              THEN 'pg_class'::regclass::oid::bigint
              ELSE dependency.classid::bigint END AS class_id,
         CASE WHEN rewrite.rulename = '_RETURN'
              THEN rewrite.ev_class::bigint
              ELSE dependency.objid::bigint END AS object_id,
         CASE WHEN rewrite.rulename = '_RETURN'
              THEN 0::bigint
              ELSE dependency.objsubid::bigint END AS sub_id,
         (dependency.deptype = 'n'
          OR (dependency.classid = 'pg_class'::regclass
              AND EXISTS (
                SELECT 1 FROM pg_class owned
                WHERE owned.oid = dependency.objid AND owned.relkind = 'S'
              ))) AS reported
  FROM frontier walk
  JOIN pg_depend dependency
    ON dependency.refclassid = walk.class_id::oid
   AND dependency.refobjid = walk.object_id::oid
   AND (walk.sub_id = 0 OR dependency.refobjsubid = walk.sub_id)
   AND dependency.deptype IN ('n', 'a', 'i')
  LEFT JOIN pg_rewrite rewrite
    ON dependency.classid = 'pg_rewrite'::regclass
   AND rewrite.oid = dependency.objid
),
normalized AS MATERIALIZED (
  SELECT class_id, object_id, sub_id, bool_or(reported) AS reported
  FROM candidate_rows
  WHERE object_id IS NOT NULL
  GROUP BY class_id, object_id, sub_id
),
unvisited AS MATERIALIZED (
  SELECT candidate.class_id, candidate.object_id, candidate.sub_id, candidate.reported
  FROM normalized candidate
  WHERE NOT EXISTS (
    SELECT 1
    FROM visited
    WHERE visited.class_id = candidate.class_id
      AND visited.object_id = candidate.object_id
      AND visited.sub_id = candidate.sub_id
  )
)
SELECT class_id, object_id, sub_id, reported
FROM unvisited
ORDER BY class_id, object_id, sub_id
LIMIT $7
"#;

async fn load_drop_candidates(
    conn: &mut sqlx::PgConnection,
    frontier: &[DropAddress],
    visited: &BTreeSet<DropAddress>,
    limit: usize,
) -> Result<Vec<DropCandidate>, PgObjectError> {
    let frontier_class = frontier
        .iter()
        .map(|item| item.class_id)
        .collect::<Vec<_>>();
    let frontier_object = frontier
        .iter()
        .map(|item| item.object_id)
        .collect::<Vec<_>>();
    let frontier_sub = frontier.iter().map(|item| item.sub_id).collect::<Vec<_>>();
    let visited_class = visited.iter().map(|item| item.class_id).collect::<Vec<_>>();
    let visited_object = visited
        .iter()
        .map(|item| item.object_id)
        .collect::<Vec<_>>();
    let visited_sub = visited.iter().map(|item| item.sub_id).collect::<Vec<_>>();
    let rows = sqlx::query_as::<_, (i64, i64, i64, bool)>(DROP_CANDIDATES_SQL)
        .bind(frontier_class)
        .bind(frontier_object)
        .bind(frontier_sub)
        .bind(visited_class)
        .bind(visited_object)
        .bind(visited_sub)
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(&mut *conn)
        .await
        .map_err(read_database_error)?;
    Ok(rows
        .into_iter()
        .map(|(class_id, object_id, sub_id, reported)| DropCandidate {
            address: DropAddress {
                class_id,
                object_id,
                sub_id,
            },
            reported,
        })
        .collect())
}

pub(crate) async fn load_pg_drop_impact(
    connection: &StoredConnection,
    reference: PgObjectRef,
) -> Result<PgDropImpact, PgObjectError> {
    ensure_postgres(connection)?;
    validate_read_reference(&reference)?;
    let mut conn = super::connect(connection)
        .await
        .map_err(|message| PgObjectError::Connection { message })?;
    let mut transaction = conn.begin().await.map_err(read_database_error)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(read_database_error)?;
    let result = load_pg_drop_impact_snapshot(&mut transaction, &reference).await?;
    transaction.commit().await.map_err(read_database_error)?;
    Ok(result)
}

async fn load_pg_drop_impact_snapshot(
    conn: &mut sqlx::PgConnection,
    reference: &PgObjectRef,
) -> Result<PgDropImpact, PgObjectError> {
    let (class_id, object_id) = resolve_object_address(conn, reference).await?;
    let root = DropAddress {
        class_id,
        object_id,
        sub_id: 0,
    };
    let mut visited = BTreeSet::from([root]);
    let mut frontier = vec![root];
    let mut discovered = Vec::<(DropAddress, u32)>::new();
    let mut truncated = false;

    // At most 201 normalized addresses enter each depth. Including the root,
    // the walk therefore executes with no more than 1,609 retained addresses.
    for depth in 1..=DROP_IMPACT_DEPTH_CAP {
        let mut candidates = load_drop_candidates(
            conn,
            &frontier,
            &visited,
            DROP_IMPACT_ADDRESS_CAP_PER_DEPTH + 1,
        )
        .await?;
        if candidates.len() > DROP_IMPACT_ADDRESS_CAP_PER_DEPTH {
            truncated = true;
            candidates.truncate(DROP_IMPACT_ADDRESS_CAP_PER_DEPTH);
        }
        frontier.clear();
        for candidate in candidates {
            if visited.insert(candidate.address) {
                frontier.push(candidate.address);
                if candidate.reported {
                    discovered.push((candidate.address, depth));
                }
            }
        }
        if frontier.is_empty() {
            break;
        }
    }
    if !frontier.is_empty()
        && !load_drop_candidates(conn, &frontier, &visited, 1)
            .await?
            .is_empty()
    {
        truncated = true;
    }

    if discovered.is_empty() {
        return Ok(PgDropImpact {
            dependents: Vec::new(),
            truncated,
        });
    }
    let classes = discovered
        .iter()
        .map(|(address, _)| address.class_id)
        .collect::<Vec<_>>();
    let objects = discovered
        .iter()
        .map(|(address, _)| address.object_id)
        .collect::<Vec<_>>();
    let sub_ids = discovered
        .iter()
        .map(|(address, _)| address.sub_id)
        .collect::<Vec<_>>();
    let depths = discovered
        .iter()
        .map(|(_, depth)| i64::from(*depth))
        .collect::<Vec<_>>();
    let rows = sqlx::query(
        r#"
WITH addresses(class_id, object_id, sub_id, depth) AS MATERIALIZED (
  SELECT * FROM UNNEST($1::bigint[], $2::bigint[], $3::bigint[], $4::bigint[])
),
identified_rows AS MATERIALIZED (
  SELECT identified.type::text AS object_type,
         identified.identity::text AS identity,
         addresses.depth::integer AS depth
  FROM addresses
  CROSS JOIN LATERAL pg_identify_object(
    addresses.class_id::oid,
    addresses.object_id::oid,
    addresses.sub_id::integer
  ) identified
),
identified AS MATERIALIZED (
  SELECT DISTINCT ON (identity COLLATE "C") object_type, identity, depth
  FROM identified_rows
  ORDER BY identity COLLATE "C", depth, object_type COLLATE "C"
)
SELECT object_type, identity, depth
FROM identified
ORDER BY depth, identity COLLATE "C"
LIMIT 201
"#,
    )
    .bind(classes)
    .bind(objects)
    .bind(sub_ids)
    .bind(depths)
    .fetch_all(&mut *conn)
    .await
    .map_err(read_database_error)?;

    truncated |= rows.len() > DROP_IMPACT_RESULT_CAP;
    let dependents = rows
        .into_iter()
        .take(DROP_IMPACT_RESULT_CAP)
        .map(|row| {
            let depth: i32 = row.try_get("depth").map_err(read_database_error)?;
            Ok(PgDropDependent {
                object_type: row.try_get("object_type").map_err(read_database_error)?,
                identity: row.try_get("identity").map_err(read_database_error)?,
                depth: u32::try_from(depth).unwrap_or_default(),
            })
        })
        .collect::<Result<Vec<_>, PgObjectError>>()?;
    Ok(PgDropImpact {
        dependents,
        truncated,
    })
}

async fn resolve_object_address(
    conn: &mut sqlx::PgConnection,
    reference: &PgObjectRef,
) -> Result<(i64, i64), PgObjectError> {
    let schema = reference.schema.as_deref();
    let query = match reference.kind {
        PgObjectKind::Schema => {
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT 'pg_namespace'::regclass::oid::bigint, oid::bigint FROM pg_namespace WHERE nspname = $1",
            )
            .bind(&reference.name)
        }
        PgObjectKind::Table
        | PgObjectKind::View
        | PgObjectKind::MaterializedView
        | PgObjectKind::ForeignTable
        | PgObjectKind::Sequence => {
            let relkinds: Vec<&str> = match reference.kind {
                PgObjectKind::Table => vec!["r", "p"],
                PgObjectKind::View => vec!["v"],
                PgObjectKind::MaterializedView => vec!["m"],
                PgObjectKind::ForeignTable => vec!["f"],
                PgObjectKind::Sequence => vec!["S"],
                _ => unreachable!(),
            };
            sqlx::query_as::<_, (i64, i64)>(
                r#"
SELECT 'pg_class'::regclass::oid::bigint, c.oid::bigint
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind::text = ANY($3)
"#,
            )
            .bind(schema.unwrap_or_default())
            .bind(&reference.name)
            .bind(relkinds)
        }
        PgObjectKind::Function | PgObjectKind::Procedure | PgObjectKind::Aggregate => {
            let prokind = match reference.kind {
                PgObjectKind::Function => "f",
                PgObjectKind::Procedure => "p",
                PgObjectKind::Aggregate => "a",
                _ => unreachable!(),
            };
            sqlx::query_as::<_, (i64, i64)>(
                r#"
SELECT 'pg_proc'::regclass::oid::bigint, p.oid::bigint
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
WHERE n.nspname = $1 AND p.proname = $2 AND p.prokind::text = $3
  AND pg_get_function_identity_arguments(p.oid) = $4
"#,
            )
            .bind(schema.unwrap_or_default())
            .bind(&reference.name)
            .bind(prokind)
            .bind(reference.identity_args.as_deref().unwrap_or_default())
        }
        PgObjectKind::Type | PgObjectKind::Domain => {
            let kinds: Vec<&str> = if reference.kind == PgObjectKind::Domain {
                vec!["d"]
            } else {
                vec!["c", "e", "r", "m"]
            };
            sqlx::query_as::<_, (i64, i64)>(
                r#"
SELECT 'pg_type'::regclass::oid::bigint, t.oid::bigint
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace
LEFT JOIN pg_class type_relation ON type_relation.oid = t.typrelid
WHERE n.nspname = $1 AND t.typname = $2 AND t.typtype::text = ANY($3)
  AND (t.typtype <> 'c' OR type_relation.relkind = 'c')
"#,
            )
            .bind(schema.unwrap_or_default())
            .bind(&reference.name)
            .bind(kinds)
        }
        PgObjectKind::Extension => sqlx::query_as::<_, (i64, i64)>(
            "SELECT 'pg_extension'::regclass::oid::bigint, oid::bigint FROM pg_extension WHERE extname = $1",
        )
        .bind(&reference.name),
    };
    query
        .fetch_optional(&mut *conn)
        .await
        .map_err(read_database_error)?
        .ok_or_else(|| object_not_found(reference))
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgTypeAttribute {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PgObjectFacts {
    Schema,
    Table,
    View {
        definition: String,
    },
    MaterializedView {
        definition: String,
        populated: bool,
    },
    ForeignTable {
        server: String,
    },
    Sequence {
        data_type: String,
        start: String,
        increment: String,
        min_value: String,
        max_value: String,
        cycle: bool,
        cache: String,
        last_value: Option<String>,
        owned_by: Option<String>,
    },
    Routine {
        language: String,
        returns: Option<String>,
        volatility: Option<String>,
        arguments: String,
    },
    Type {
        class: PgTypeClass,
        enum_labels: Option<Vec<String>>,
        attributes: Option<Vec<PgTypeAttribute>>,
        subtype: Option<String>,
    },
    Domain {
        base_type: String,
        not_null: bool,
        default_value: Option<String>,
        checks: Vec<String>,
    },
    Extension {
        version: String,
        schema: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgObjectDescription {
    pub reference: PgObjectRef,
    pub owner: Option<String>,
    pub comment: Option<String>,
    pub definition_sql: Option<String>,
    pub facts: PgObjectFacts,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgDropDependent {
    pub object_type: String,
    pub identity: String,
    pub depth: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgDropImpact {
    pub dependents: Vec<PgDropDependent>,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ConnectionOrganization, Environment, PgStoredConnection, SafeMode, SqliteStoredConnection,
        SshTunnelConfig,
    };

    fn test_connection(id: &str) -> StoredConnection {
        let port = std::env::var("DBUNK_OBJECT_TEST_PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .unwrap_or(15432);
        StoredConnection::PostgreSQL(PgStoredConnection {
            organization: ConnectionOrganization::default(),
            id: id.into(),
            name: "PostgreSQL object live test".into(),
            database: "dbunk_demo".into(),
            host: "127.0.0.1".into(),
            port,
            user: "dbunk".into(),
            password: "dbunk".into(),
            role: "read/write".into(),
            environment: Environment::Development,
            safe_mode: SafeMode::Disabled,
            read_only: false,
            last_activity_at: None,
            ssl: port == 15433,
            tls_options: None,
            driver_options: None,
            ssh_tunnel: SshTunnelConfig::default(),
        })
    }

    #[test]
    fn foreign_data_options_are_reconstructed_with_quoting() {
        assert_eq!(
            render_fdw_options(Some(&["column_name=remote'id".into(), "token=a=b".into(),]))
                .expect("valid FDW options"),
            " OPTIONS (\"column_name\" E'remote''id', \"token\" E'a=b')"
        );
        assert!(render_fdw_options(Some(&["missing_separator".into()])).is_err());
    }

    #[test]
    fn view_definition_terminator_is_removed_before_reconstruction() {
        assert_eq!(trim_view_definition("  SELECT 1;\n"), "SELECT 1");
        assert_eq!(trim_view_definition("SELECT ';'::text"), "SELECT ';'::text");
    }

    #[test]
    fn sequence_owner_components_are_quoted_without_parsing_a_display_string() {
        assert_eq!(
            render_sequence_owner("tenant.with.dot", "Order\"Log", "value.part"),
            "\"tenant.with.dot\".\"Order\"\"Log\".\"value.part\""
        );
    }

    #[test]
    fn sequence_fact_quantities_serialize_as_decimal_strings() {
        let facts = PgObjectFacts::Sequence {
            data_type: "bigint".into(),
            start: i64::MIN.to_string(),
            increment: "1".into(),
            min_value: i64::MIN.to_string(),
            max_value: i64::MAX.to_string(),
            cycle: false,
            cache: "1".into(),
            last_value: Some(i64::MAX.to_string()),
            owned_by: None,
        };
        let value = serde_json::to_value(facts).expect("serialize sequence facts");
        assert!(value["start"].is_string());
        assert_eq!(value["maxValue"], i64::MAX.to_string());
        assert!(value["lastValue"].is_string());
    }

    #[test]
    fn drop_candidate_query_normalizes_and_bounds_before_expansion() {
        assert!(DROP_CANDIDATES_SQL
            .contains("(walk.sub_id = 0 OR dependency.refobjsubid = walk.sub_id)"));
        let normalized = DROP_CANDIDATES_SQL
            .find("normalized AS MATERIALIZED")
            .expect("normalized CTE");
        let limit = DROP_CANDIDATES_SQL
            .find("LIMIT $7")
            .expect("candidate limit");
        assert!(normalized < limit);
        assert_eq!(
            1 + DROP_IMPACT_DEPTH_CAP as usize * DROP_IMPACT_ADDRESS_CAP_PER_DEPTH,
            1_609
        );
    }

    #[tokio::test]
    async fn object_catalog_rejects_non_postgresql_connections() {
        let connection = StoredConnection::SQLite(SqliteStoredConnection {
            id: "sqlite".into(),
            name: "SQLite".into(),
            database: ":memory:".into(),
            host: String::new(),
            port: 0,
            user: String::new(),
            password: String::new(),
            role: "read/write".into(),
            environment: Environment::Development,
            safe_mode: SafeMode::Disabled,
            read_only: false,
            last_activity_at: None,
            organization: ConnectionOrganization::default(),
        });
        assert!(matches!(
            load_pg_object_catalog(&connection).await,
            Err(PgObjectError::UnsupportedEngine { engine }) if engine == "SQLite"
        ));
    }

    #[tokio::test]
    #[serial_test::serial]
    #[ignore = "requires pnpm db:postgres"]
    async fn object_catalog_live_lists_typed_capped_fixture_objects() {
        let connection = test_connection("pg-object-catalog-live-test");
        let mut conn = super::super::connect(&connection).await.expect("connect");
        sqlx::raw_sql(
            r#"
DROP SCHEMA IF EXISTS object_catalog_cap CASCADE;
CREATE SCHEMA object_catalog_cap;
DO $$
BEGIN
  FOR index IN 1..2001 LOOP
    EXECUTE format('CREATE SEQUENCE object_catalog_cap.sequence_%s', lpad(index::text, 4, '0'));
  END LOOP;
END
$$;
"#,
        )
        .execute(&mut *conn)
        .await
        .expect("create cap fixture");
        drop(conn);

        let catalog = load_pg_object_catalog(&connection)
            .await
            .expect("load object catalog");
        let lifecycle = catalog
            .schemas
            .iter()
            .find(|schema| schema.name == "lifecycle")
            .expect("lifecycle schema");
        assert!(lifecycle.tables.iter().any(|entry| entry.name == "orders"
            && entry.comment.as_deref()
                == Some("Orders used to exercise object lifecycle operations.")));
        assert!(lifecycle
            .views
            .iter()
            .any(|entry| entry.name == "orders_view"));
        assert!(lifecycle
            .materialized_views
            .iter()
            .any(|entry| entry.name == "orders_mat"));
        assert!(lifecycle
            .foreign_tables
            .iter()
            .any(|entry| entry.name == "remote_orders"));
        assert!(lifecycle
            .sequences
            .iter()
            .any(|entry| entry.name == "order_seq"));
        let overloads = lifecycle
            .functions
            .iter()
            .filter(|entry| entry.name == "add_nums")
            .collect::<Vec<_>>();
        assert_eq!(overloads.len(), 2);
        assert_ne!(overloads[0].identity_args, overloads[1].identity_args);
        assert!(lifecycle
            .procedures
            .iter()
            .any(|entry| entry.name == "bump_orders"));
        assert!(lifecycle
            .aggregates
            .iter()
            .any(|entry| entry.name == "sum_squares"));
        assert!(lifecycle.types.iter().any(|entry| {
            entry.name == "order_status" && entry.type_class == Some(PgTypeClass::Enum)
        }));
        assert!(lifecycle.types.iter().any(|entry| {
            entry.name == "_internal_status" && entry.type_class == Some(PgTypeClass::Enum)
        }));
        assert!(lifecycle.types.iter().any(|entry| {
            entry.name == "money_pair" && entry.type_class == Some(PgTypeClass::Composite)
        }));
        assert!(lifecycle.types.iter().any(|entry| {
            entry.name == "order_id_range" && entry.type_class == Some(PgTypeClass::Range)
        }));
        assert!(lifecycle.types.iter().any(|entry| {
            entry.name == "order_id_multirange" && entry.type_class == Some(PgTypeClass::Multirange)
        }));
        assert!(lifecycle
            .domains
            .iter()
            .any(|entry| entry.name == "positive_int"));
        assert!(lifecycle
            .extensions
            .iter()
            .any(|entry| entry.name == "hstore"));
        assert!(!catalog.schemas.iter().any(
            |schema| schema.name.starts_with("pg_temp") || schema.name.starts_with("pg_toast")
        ));
        let public = catalog
            .schemas
            .iter()
            .find(|schema| schema.name == "public")
            .expect("public schema");
        assert!(!public
            .functions
            .iter()
            .any(|entry| matches!(entry.name.as_str(), "crypt" | "digest" | "gen_salt")));
        assert!(!public.types.iter().any(|entry| entry.name == "citext"));
        assert!(!catalog.roles.is_empty());
        assert!(!catalog.tablespaces.is_empty());

        let capped = catalog
            .schemas
            .iter()
            .find(|schema| schema.name == "object_catalog_cap")
            .expect("cap schema");
        assert_eq!(capped.sequences.len(), CATALOG_KIND_CAP);
        assert!(catalog.truncated.iter().any(|item| {
            item.schema.as_deref() == Some("object_catalog_cap") && item.kind == "sequence"
        }));

        let mut conn = super::super::connect(&connection).await.expect("reconnect");
        sqlx::query("DROP SCHEMA object_catalog_cap CASCADE")
            .execute(&mut *conn)
            .await
            .expect("drop cap fixture");
    }

    fn object_ref(kind: PgObjectKind, name: &str) -> PgObjectRef {
        PgObjectRef {
            kind,
            schema: if kind == PgObjectKind::Schema {
                None
            } else {
                Some("lifecycle".into())
            },
            name: name.into(),
            identity_args: None,
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    #[ignore = "requires pnpm db:postgres"]
    async fn object_description_and_drop_impact_live_cover_every_kind() {
        let connection = test_connection("pg-object-description-live-test");

        let schema = describe_pg_object(
            &connection,
            PgObjectRef {
                kind: PgObjectKind::Schema,
                schema: None,
                name: "lifecycle".into(),
                identity_args: None,
            },
        )
        .await
        .expect("describe schema");
        assert_eq!(
            schema.comment.as_deref(),
            Some("Fixture objects for PostgreSQL lifecycle workflows.")
        );
        assert!(matches!(schema.facts, PgObjectFacts::Schema));

        let table = describe_pg_object(&connection, object_ref(PgObjectKind::Table, "orders"))
            .await
            .expect("describe table");
        assert!(table
            .definition_sql
            .as_deref()
            .is_some_and(|sql| sql.starts_with("CREATE TABLE")));
        assert!(matches!(table.facts, PgObjectFacts::Table));

        let view = describe_pg_object(&connection, object_ref(PgObjectKind::View, "orders_view"))
            .await
            .expect("describe view");
        let PgObjectFacts::View { definition } = &view.facts else {
            panic!("view facts");
        };
        assert!(!definition.ends_with(';'));
        assert_eq!(
            view.definition_sql.as_deref(),
            Some(format!("CREATE VIEW \"lifecycle\".\"orders_view\" AS\n{definition};").as_str())
        );

        let materialized = describe_pg_object(
            &connection,
            object_ref(PgObjectKind::MaterializedView, "orders_mat"),
        )
        .await
        .expect("describe materialized view");
        let PgObjectFacts::MaterializedView {
            definition,
            populated,
        } = &materialized.facts
        else {
            panic!("materialized-view facts");
        };
        assert!(*populated);
        assert!(!definition.ends_with(';'));
        assert_eq!(
            materialized.definition_sql.as_deref(),
            Some(
                format!(
                    "CREATE MATERIALIZED VIEW \"lifecycle\".\"orders_mat\" AS\n{definition} WITH DATA;"
                )
                .as_str()
            )
        );

        let foreign = describe_pg_object(
            &connection,
            object_ref(PgObjectKind::ForeignTable, "remote_orders"),
        )
        .await
        .expect("describe foreign table");
        assert!(foreign
            .definition_sql
            .as_deref()
            .is_some_and(|sql| sql.starts_with("CREATE FOREIGN TABLE")));
        let foreign_sql = foreign
            .definition_sql
            .as_deref()
            .expect("foreign table DDL");
        assert!(foreign_sql.contains("\"id\" integer OPTIONS (\"column_name\" E'id')"));
        assert!(foreign_sql.contains("\"status\" text OPTIONS (\"column_name\" E'status')"));
        assert!(foreign_sql.contains("COLLATE \"pg_catalog\".\"C\""));
        assert!(foreign_sql.contains(" DEFAULT "));
        assert!(foreign_sql.contains("CONSTRAINT \"remote_orders_status_present\" CHECK"));
        assert!(foreign_sql.contains("status <> ''::text"));
        assert!(foreign_sql.contains("\"schema_name\" E'lifecycle'"));
        assert!(foreign_sql.contains("\"table_name\" E'orders'"));
        assert!(foreign_sql.contains("\"fetch_size\" E'100'"));
        assert!(matches!(
            foreign.facts,
            PgObjectFacts::ForeignTable { ref server }
                if server == "lifecycle_fixture_server"
        ));

        let sequence =
            describe_pg_object(&connection, object_ref(PgObjectKind::Sequence, "order_seq"))
                .await
                .expect("describe sequence");
        assert!(matches!(
            sequence.facts,
            PgObjectFacts::Sequence {
                ref start,
                ref increment,
                last_value: Some(_),
                ref owned_by,
                ..
            } if start == "100"
                && increment == "5"
                && owned_by.as_deref() == Some("lifecycle.orders.id")
        ));

        for identity_args in ["integer, integer", "text, text"] {
            let routine = describe_pg_object(
                &connection,
                PgObjectRef {
                    kind: PgObjectKind::Function,
                    schema: Some("lifecycle".into()),
                    name: "add_nums".into(),
                    identity_args: Some(identity_args.into()),
                },
            )
            .await
            .expect("describe overloaded function");
            assert!(routine
                .definition_sql
                .as_deref()
                .is_some_and(|sql| sql.starts_with("CREATE OR REPLACE FUNCTION")));
        }
        let procedure = describe_pg_object(
            &connection,
            PgObjectRef {
                kind: PgObjectKind::Procedure,
                schema: Some("lifecycle".into()),
                name: "bump_orders".into(),
                identity_args: Some(String::new()),
            },
        )
        .await
        .expect("describe procedure");
        assert!(procedure
            .definition_sql
            .as_deref()
            .is_some_and(|sql| sql.starts_with("CREATE OR REPLACE PROCEDURE")));
        let aggregate = describe_pg_object(
            &connection,
            PgObjectRef {
                kind: PgObjectKind::Aggregate,
                schema: Some("lifecycle".into()),
                name: "sum_squares".into(),
                identity_args: Some("numeric".into()),
            },
        )
        .await
        .expect("describe aggregate");
        assert!(aggregate.definition_sql.is_none());
        assert!(matches!(aggregate.facts, PgObjectFacts::Routine { .. }));

        let enum_type =
            describe_pg_object(&connection, object_ref(PgObjectKind::Type, "order_status"))
                .await
                .expect("describe enum");
        assert!(matches!(
            enum_type.facts,
            PgObjectFacts::Type {
                class: PgTypeClass::Enum,
                enum_labels: Some(ref labels),
                ..
            } if labels == &["new", "processing", "done"]
        ));
        let composite =
            describe_pg_object(&connection, object_ref(PgObjectKind::Type, "money_pair"))
                .await
                .expect("describe composite");
        assert!(matches!(
            composite.facts,
            PgObjectFacts::Type {
                class: PgTypeClass::Composite,
                attributes: Some(ref attributes),
                ..
            } if attributes.len() == 2
        ));
        let range = describe_pg_object(
            &connection,
            object_ref(PgObjectKind::Type, "order_id_range"),
        )
        .await
        .expect("describe range");
        assert!(range.definition_sql.as_deref().is_some_and(|sql| {
            sql.contains("MULTIRANGE_TYPE_NAME = \"lifecycle\".\"order_id_multirange\"")
                && sql.contains("SUBTYPE_OPCLASS = \"pg_catalog\".")
        }));
        assert!(matches!(
            range.facts,
            PgObjectFacts::Type {
                class: PgTypeClass::Range,
                subtype: Some(ref subtype),
                ..
            } if subtype == "integer"
        ));
        let multirange = describe_pg_object(
            &connection,
            object_ref(PgObjectKind::Type, "order_id_multirange"),
        )
        .await
        .expect("describe multirange");
        assert!(multirange.definition_sql.as_deref().is_some_and(|sql| {
            sql.contains("MULTIRANGE_TYPE_NAME = \"lifecycle\".\"order_id_multirange\"")
        }));
        assert!(matches!(
            multirange.facts,
            PgObjectFacts::Type {
                class: PgTypeClass::Multirange,
                subtype: Some(ref subtype),
                ..
            } if subtype == "integer"
        ));
        let domain = describe_pg_object(
            &connection,
            object_ref(PgObjectKind::Domain, "positive_int"),
        )
        .await
        .expect("describe domain");
        assert!(matches!(
            domain.facts,
            PgObjectFacts::Domain { ref checks, .. } if !checks.is_empty()
        ));
        let extension =
            describe_pg_object(&connection, object_ref(PgObjectKind::Extension, "hstore"))
                .await
                .expect("describe extension");
        assert!(matches!(
            extension.facts,
            PgObjectFacts::Extension { ref schema, .. } if schema == "lifecycle"
        ));

        let impact = load_pg_drop_impact(&connection, object_ref(PgObjectKind::Table, "orders"))
            .await
            .expect("orders drop impact");
        assert!(impact.dependents.windows(2).all(|pair| {
            (pair[0].depth, pair[0].identity.as_str()) <= (pair[1].depth, pair[1].identity.as_str())
        }));
        assert_eq!(
            impact
                .dependents
                .iter()
                .map(|dependent| dependent.identity.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            impact.dependents.len()
        );
        assert!(impact.dependents.iter().any(|dependent| {
            dependent.identity.contains("lifecycle.orders_view") && dependent.depth == 1
        }));
        assert!(impact.dependents.iter().any(|dependent| {
            dependent.identity.contains("lifecycle.orders_mat") && dependent.depth == 2
        }));
        // OWNED BY is an auto dependency, so sequence loss is disclosed.
        assert!(impact.dependents.iter().any(|dependent| {
            dependent.identity.contains("lifecycle.order_seq") && dependent.depth == 1
        }));
        // Parts of the table itself are dropped silently, as CASCADE reports.
        assert!(!impact.dependents.iter().any(|dependent| {
            dependent.identity.contains("orders_pkey")
                || dependent.object_type == "default value"
                || dependent.object_type == "type"
        }));

        let mut conn = super::super::connect(&connection).await.expect("connect");
        sqlx::raw_sql(
            r#"
DROP SCHEMA IF EXISTS object_impact CASCADE;
CREATE SCHEMA object_impact;
CREATE TABLE object_impact.base (id integer);
CREATE TABLE object_impact.chain_base (id integer);
CREATE DOMAIN object_impact.marker AS integer;
CREATE TABLE object_impact.column_base (
  marked object_impact.marker,
  unrelated integer
);
CREATE TABLE object_impact.identity_owner (
  id bigint GENERATED ALWAYS AS IDENTITY
);
CREATE VIEW object_impact.marked_view AS
  SELECT marked FROM object_impact.column_base;
CREATE VIEW object_impact.unrelated_view AS
  SELECT unrelated FROM object_impact.column_base;
CREATE TABLE object_impact.stats (id integer);
CREATE TABLE object_impact.audit (id integer);
CREATE RULE log_ins AS ON INSERT TO object_impact.audit
  DO ALSO INSERT INTO object_impact.stats SELECT NEW.id;
CREATE TABLE object_impact.sales (id integer NOT NULL, sold_at date NOT NULL)
  PARTITION BY RANGE (sold_at);
CREATE TABLE object_impact.sales_2024 PARTITION OF object_impact.sales
  FOR VALUES FROM ('2024-01-01') TO ('2025-01-01');
CREATE INDEX sales_sold_at_idx ON object_impact.sales (sold_at);
DO $$
BEGIN
  FOR index IN 1..205 LOOP
    EXECUTE format('CREATE VIEW object_impact.dependent_%s AS SELECT * FROM object_impact.base', lpad(index::text, 3, '0'));
  END LOOP;
END
$$;
CREATE VIEW object_impact.chain_01 AS SELECT * FROM object_impact.chain_base;
CREATE VIEW object_impact.chain_02 AS SELECT * FROM object_impact.chain_01;
CREATE VIEW object_impact.chain_03 AS SELECT * FROM object_impact.chain_02;
CREATE VIEW object_impact.chain_04 AS SELECT * FROM object_impact.chain_03;
CREATE VIEW object_impact.chain_05 AS SELECT * FROM object_impact.chain_04;
CREATE VIEW object_impact.chain_06 AS SELECT * FROM object_impact.chain_05;
CREATE VIEW object_impact.chain_07 AS SELECT * FROM object_impact.chain_06;
CREATE VIEW object_impact.chain_08 AS SELECT * FROM object_impact.chain_07;
CREATE VIEW object_impact.chain_09 AS SELECT * FROM object_impact.chain_08;
CREATE VIEW object_impact.chain_10 AS SELECT * FROM object_impact.chain_09;
"#,
        )
        .execute(&mut *conn)
        .await
        .expect("create impact fixtures");
        drop(conn);
        let synthetic = PgObjectRef {
            kind: PgObjectKind::Table,
            schema: Some("object_impact".into()),
            name: "base".into(),
            identity_args: None,
        };
        let impact = load_pg_drop_impact(&connection, synthetic.clone())
            .await
            .expect("capped impact");
        assert!(impact.truncated);
        assert_eq!(impact.dependents.len(), 200);
        assert!(impact
            .dependents
            .iter()
            .all(|dependent| dependent.depth <= 8));

        let chain_impact = load_pg_drop_impact(
            &connection,
            PgObjectRef {
                kind: PgObjectKind::Table,
                schema: Some("object_impact".into()),
                name: "chain_base".into(),
                identity_args: None,
            },
        )
        .await
        .expect("depth-limited impact");
        assert!(chain_impact.dependents.iter().any(|dependent| {
            dependent.identity.contains("object_impact.chain_08") && dependent.depth == 8
        }));
        assert!(!chain_impact.dependents.iter().any(|dependent| {
            dependent.identity.contains("object_impact.chain_09")
                || dependent.identity.contains("object_impact.chain_10")
        }));

        let column_impact = load_pg_drop_impact(
            &connection,
            PgObjectRef {
                kind: PgObjectKind::Domain,
                schema: Some("object_impact".into()),
                name: "marker".into(),
                identity_args: None,
            },
        )
        .await
        .expect("column-specific impact");
        assert!(column_impact
            .dependents
            .iter()
            .any(|dependent| dependent.identity.contains("object_impact.marked_view")));
        assert!(!column_impact
            .dependents
            .iter()
            .any(|dependent| dependent.identity.contains("object_impact.unrelated_view")));

        let rule_impact = load_pg_drop_impact(
            &connection,
            PgObjectRef {
                kind: PgObjectKind::Table,
                schema: Some("object_impact".into()),
                name: "stats".into(),
                identity_args: None,
            },
        )
        .await
        .expect("rule-referenced impact");
        // DROP TABLE stats CASCADE drops the rule, not the table it is on.
        assert!(rule_impact
            .dependents
            .iter()
            .any(|dependent| dependent.object_type == "rule"));
        assert!(!rule_impact.dependents.iter().any(|dependent| {
            dependent.object_type == "table" && dependent.identity.contains("object_impact.audit")
        }));

        let partitioned = describe_pg_object(
            &connection,
            PgObjectRef {
                kind: PgObjectKind::Table,
                schema: Some("object_impact".into()),
                name: "sales".into(),
                identity_args: None,
            },
        )
        .await
        .expect("describe partitioned parent");
        let parent_sql = partitioned.definition_sql.expect("parent DDL");
        assert!(parent_sql.contains(") PARTITION BY RANGE (sold_at);"));
        assert!(parent_sql.contains("CREATE INDEX sales_sold_at_idx"));
        let partition = describe_pg_object(
            &connection,
            PgObjectRef {
                kind: PgObjectKind::Table,
                schema: Some("object_impact".into()),
                name: "sales_2024".into(),
                identity_args: None,
            },
        )
        .await
        .expect("describe partition");
        let partition_sql = partition.definition_sql.expect("partition DDL");
        assert_eq!(
            partition_sql,
            "CREATE TABLE \"object_impact\".\"sales_2024\" PARTITION OF \"object_impact\".\"sales\" \
             FOR VALUES FROM ('2024-01-01') TO ('2025-01-01');"
        );

        let identity_sequence = describe_pg_object(
            &connection,
            PgObjectRef {
                kind: PgObjectKind::Sequence,
                schema: Some("object_impact".into()),
                name: "identity_owner_id_seq".into(),
                identity_args: None,
            },
        )
        .await
        .expect("describe identity-owned sequence");
        assert!(matches!(
            identity_sequence.facts,
            PgObjectFacts::Sequence { ref owned_by, .. }
                if owned_by.as_deref() == Some("object_impact.identity_owner.id")
        ));
        let identity_impact = load_pg_drop_impact(
            &connection,
            PgObjectRef {
                kind: PgObjectKind::Table,
                schema: Some("object_impact".into()),
                name: "identity_owner".into(),
                identity_args: None,
            },
        )
        .await
        .expect("identity table drop impact");
        assert!(identity_impact.dependents.iter().any(|dependent| {
            dependent
                .identity
                .contains("object_impact.identity_owner_id_seq")
        }));

        let mut conn = super::super::connect(&connection).await.expect("reconnect");
        sqlx::query("DROP SCHEMA object_impact CASCADE")
            .execute(&mut *conn)
            .await
            .expect("drop impact fixtures");
        sqlx::raw_sql(
            "CREATE TABLE lifecycle.disappearing (id integer); DROP TABLE lifecycle.disappearing;",
        )
        .execute(&mut *conn)
        .await
        .expect("create and drop race fixture");
        drop(conn);
        assert!(matches!(
            describe_pg_object(&connection, object_ref(PgObjectKind::Table, "disappearing")).await,
            Err(PgObjectError::ObjectNotFound { .. })
        ));
    }
}
