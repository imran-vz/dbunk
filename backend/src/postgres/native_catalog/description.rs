//! Exact-or-refused descriptions in one repeatable-read, read-only transaction.
//! Collection-bearing kinds stream guarded components under one shared budget.
use super::*;
use crate::postgres::objects::{PgObjectDescription, PgObjectFacts, PgObjectKind, PgObjectRef};
use serde::Deserialize;

mod components;
mod relations;
mod type_description;

pub const MAX_DESCRIPTION_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DESCRIPTION_TEXT_BYTES: usize = 1024 * 1024;
/// Shared count of header and component rows for collection-bearing descriptions.
pub const MAX_DESCRIPTION_COMPONENTS: usize = 4096;

pub(crate) fn validate(reference: &PgObjectRef) -> Result<(), CatalogError> {
    use PgObjectKind::*;
    for value in [
        Some(reference.name.as_str()),
        reference.schema.as_deref(),
        reference.identity_args.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if value.len() > MAX_TEXT_BYTES || value.contains('\0') {
            return Err(CatalogError::InvalidReference);
        }
    }
    let schema_valid = if reference.kind == Schema {
        reference.schema.is_none()
    } else {
        reference
            .schema
            .as_ref()
            .is_some_and(|schema| !schema.is_empty())
    };
    let routine = matches!(reference.kind, Function | Procedure | Aggregate);
    if reference.name.is_empty() || !schema_valid || routine != reference.identity_args.is_some() {
        return Err(CatalogError::InvalidReference);
    }
    Ok(())
}

pub(crate) async fn read(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    reference: PgObjectRef,
) -> Result<PgObjectDescription, CatalogError> {
    validate(&reference)?;
    owned_read(
        spec,
        drivers,
        cancellation,
        Duration::from_secs(30),
        move |client, timeout| {
            Box::pin(async move {
                begin_snapshot(client, timeout).await?;
                let description = load(client, reference).await?;
                client
                    .batch_execute("COMMIT")
                    .await
                    .map_err(|_| CatalogError::Database)?;
                Ok(description)
            })
        },
    )
    .await
}

pub(crate) async fn load(
    client: &Client,
    reference: PgObjectRef,
) -> Result<PgObjectDescription, CatalogError> {
    use PgObjectKind::*;
    if matches!(reference.kind, Table | ForeignTable) {
        return relations::load(client, reference).await;
    }
    if matches!(reference.kind, Type | Domain) {
        return type_description::load(client, reference).await;
    }
    let prokind = match reference.kind {
        Function => "f",
        Procedure => "p",
        Aggregate => "a",
        _ => "",
    };
    let relation_kind = if reference.kind == MaterializedView {
        "m"
    } else {
        "v"
    };
    let (source, params): (&str, Vec<&(dyn ToSql + Sync)>) = match reference.kind {
        Schema => (SCHEMA, vec![&reference.name]),
        View | MaterializedView => {
            // Relation kind is part of identity; a recreated table cannot be
            // described as the old view merely because its name matches.
            (
                VIEW,
                vec![&reference.schema, &reference.name, &relation_kind],
            )
        }
        Sequence => (SEQUENCE, vec![&reference.schema, &reference.name]),
        Function | Procedure | Aggregate => (
            ROUTINE,
            vec![
                &reference.schema,
                &reference.name,
                &reference.identity_args,
                &prokind,
            ],
        ),
        Extension => (EXTENSION, vec![&reference.schema, &reference.name]),
        _ => return Err(CatalogError::UnsupportedObjectKind),
    };
    let query = bounded_query(source);
    let rows = client
        .query_raw(&query, params)
        .await
        .map_err(|_| CatalogError::Database)?;
    tokio::pin!(rows);
    let row = rows
        .try_next()
        .await
        .map_err(|_| CatalogError::Database)?
        .ok_or(CatalogError::ObjectNotFound)?;
    if row
        .try_get::<_, bool>("oversized")
        .map_err(|_| CatalogError::InvalidResponse)?
    {
        return Err(CatalogError::DescriptionLimit);
    }
    let payload: &str = row
        .try_get("payload")
        .map_err(|_| CatalogError::InvalidResponse)?;
    let description = decode(reference, payload)?;
    if rows
        .try_next()
        .await
        .map_err(|_| CatalogError::Database)?
        .is_some()
    {
        return Err(CatalogError::InvalidResponse);
    }
    Ok(description)
}

fn bounded_query(source: &str) -> String {
    format!("WITH source AS ({source}), encoded AS (SELECT json_build_object('owner', owner, 'comment', comment, 'definitionSql', definition_sql, 'facts', facts, 'sequenceOwner', sequence_owner)::text AS payload, oversized FROM source) SELECT CASE WHEN NOT oversized AND octet_length(convert_to(payload, 'UTF8')) <= {MAX_DESCRIPTION_BYTES} THEN payload END AS payload, (oversized OR octet_length(convert_to(payload, 'UTF8')) > {MAX_DESCRIPTION_BYTES}) AS oversized FROM encoded LIMIT 2")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDescription {
    owner: Option<String>,
    comment: Option<String>,
    definition_sql: Option<String>,
    facts: PgObjectFacts,
    sequence_owner: Option<[String; 3]>,
}

fn decode(reference: PgObjectRef, payload: &str) -> Result<PgObjectDescription, CatalogError> {
    validate(&reference)?;
    if payload.len() > MAX_DESCRIPTION_BYTES {
        return Err(CatalogError::DescriptionLimit);
    }
    let raw: RawDescription =
        serde_json::from_str(payload).map_err(|_| CatalogError::InvalidResponse)?;
    check_metadata(raw.owner.as_deref())?;
    check_metadata(raw.comment.as_deref())?;
    check_body(raw.definition_sql.as_deref())?;
    let mut description = PgObjectDescription {
        reference,
        owner: raw.owner,
        comment: raw.comment,
        definition_sql: raw.definition_sql,
        facts: raw.facts,
    };
    use PgObjectFacts as Facts;
    use PgObjectKind as Kind;
    let reference = &description.reference;
    let qualified = || {
        format!(
            "{}.{}",
            crate::quote_double(reference.schema.as_deref().unwrap()),
            crate::quote_double(&reference.name)
        )
    };
    description.definition_sql = match (&reference.kind, &mut description.facts) {
        (Kind::Schema, Facts::Schema) => Some(format!(
            "CREATE SCHEMA {};",
            crate::quote_double(&reference.name)
        )),
        (Kind::View, Facts::View { definition }) => {
            trim_definition(definition)?;
            Some(format!("CREATE VIEW {} AS\n{definition};", qualified()))
        }
        (
            Kind::MaterializedView,
            Facts::MaterializedView {
                definition,
                populated,
            },
        ) => {
            trim_definition(definition)?;
            Some(format!(
                "CREATE MATERIALIZED VIEW {} AS\n{definition} {};",
                qualified(),
                if *populated {
                    "WITH DATA"
                } else {
                    "WITH NO DATA"
                }
            ))
        }
        (
            Kind::Function | Kind::Procedure | Kind::Aggregate,
            Facts::Routine {
                language,
                returns,
                volatility,
                arguments,
                body,
                parallel,
                ..
            },
        ) => {
            for value in [
                Some(language.as_str()),
                returns.as_deref(),
                volatility.as_deref(),
                parallel.as_deref(),
            ] {
                check_metadata(value)?;
            }
            check_body(Some(arguments))?;
            check_body(body.as_deref())?;
            if reference.kind == Kind::Aggregate {
                if description.definition_sql.is_some() || body.is_some() {
                    return Err(CatalogError::InvalidResponse);
                }
            } else if description.definition_sql.is_none() || body.is_none() {
                return Err(CatalogError::InvalidResponse);
            }
            description.definition_sql
        }
        (
            Kind::Sequence,
            Facts::Sequence {
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
        ) => {
            if !matches!(data_type.as_str(), "smallint" | "integer" | "bigint") {
                return Err(CatalogError::InvalidResponse);
            }
            for value in [
                Some(start.as_str()),
                Some(increment.as_str()),
                Some(min_value.as_str()),
                Some(max_value.as_str()),
                Some(cache.as_str()),
                last_value.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                // PostgreSQL sequence quantities are signed decimal text. Do not
                // parse through floating point or rewrite a returned lexeme.
                if value.parse::<i64>().is_err() {
                    return Err(CatalogError::InvalidResponse);
                }
            }
            let mut definition = format!("CREATE SEQUENCE {} AS {data_type} INCREMENT BY {increment} MINVALUE {min_value} MAXVALUE {max_value} START WITH {start} CACHE {cache} {};", qualified(), if *cycle { "CYCLE" } else { "NO CYCLE" });
            *owned_by = if let Some(parts) = raw.sequence_owner {
                for part in &parts {
                    check_metadata(Some(part))?;
                }
                definition.push_str(&format!(
                    "\nALTER SEQUENCE {} OWNED BY {}.{}.{};",
                    qualified(),
                    crate::quote_double(&parts[0]),
                    crate::quote_double(&parts[1]),
                    crate::quote_double(&parts[2])
                ));
                Some(parts.join("."))
            } else {
                None
            };
            Some(definition)
        }
        (Kind::Extension, Facts::Extension { version, schema }) => {
            check_metadata(Some(version))?;
            check_metadata(Some(schema))?;
            if reference.schema.as_deref() != Some(schema.as_str()) {
                return Err(CatalogError::ObjectNotFound);
            }
            Some(format!(
                "CREATE EXTENSION {} WITH SCHEMA {} VERSION {};",
                crate::quote_double(&reference.name),
                crate::quote_double(schema),
                crate::quote_literal(version)
            ))
        }
        _ => return Err(CatalogError::InvalidResponse),
    };
    if json_size(&description)? > MAX_DESCRIPTION_BYTES {
        return Err(CatalogError::DescriptionLimit);
    }
    Ok(description)
}
fn check_metadata(value: Option<&str>) -> Result<(), CatalogError> {
    if value.is_some_and(|value| value.len() > MAX_TEXT_BYTES) {
        Err(CatalogError::DescriptionLimit)
    } else {
        Ok(())
    }
}
fn check_body(value: Option<&str>) -> Result<(), CatalogError> {
    if value.is_some_and(|value| value.len() > MAX_DESCRIPTION_TEXT_BYTES) {
        Err(CatalogError::DescriptionLimit)
    } else {
        Ok(())
    }
}
fn trim_definition(value: &mut String) -> Result<(), CatalogError> {
    check_body(Some(value))?;
    *value = value
        .trim()
        .strip_suffix(';')
        .unwrap_or(value.trim())
        .trim_end()
        .to_owned();
    Ok(())
}

const SCHEMA: &str = r#"
SELECT pg_get_userbyid(n.nspowner)::text AS owner, obj_description(n.oid, 'pg_namespace') AS comment,
 NULL::text AS definition_sql, json_build_object('kind', 'schema') AS facts, NULL::json AS sequence_owner,
 coalesce(octet_length(obj_description(n.oid, 'pg_namespace')), 0) > 8192 AS oversized
FROM pg_namespace n WHERE n.nspname = $1
"#;
const VIEW: &str = r#"
SELECT pg_get_userbyid(c.relowner)::text AS owner, obj_description(c.oid, 'pg_class') AS comment,
 NULL::text AS definition_sql,
 CASE WHEN c.relkind = 'm' THEN json_build_object('kind', 'materializedView', 'definition', pg_get_viewdef(c.oid, true), 'populated', c.relispopulated)
 ELSE json_build_object('kind', 'view', 'definition', pg_get_viewdef(c.oid, true)) END AS facts,
 NULL::json AS sequence_owner,
 (coalesce(octet_length(obj_description(c.oid, 'pg_class')), 0) > 8192 OR octet_length(pg_get_viewdef(c.oid, true)) > 1048576) AS oversized
FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind::text = $3
"#;
const ROUTINE: &str = r#"
SELECT pg_get_userbyid(p.proowner)::text AS owner, obj_description(p.oid, 'pg_proc') AS comment,
 CASE WHEN p.prokind = 'a' THEN NULL ELSE pg_get_functiondef(p.oid) END AS definition_sql,
 json_build_object('kind', 'routine', 'language', language.lanname::text,
 'returns', CASE WHEN p.prokind = 'p' THEN NULL ELSE pg_get_function_result(p.oid) END,
 'volatility', CASE p.provolatile WHEN 'i' THEN 'immutable' WHEN 's' THEN 'stable' ELSE 'volatile' END,
 'arguments', pg_get_function_arguments(p.oid)::text,
 'body', CASE WHEN p.prokind = 'a' THEN NULL ELSE p.prosrc END,
 'strict', p.proisstrict, 'securityDefiner', p.prosecdef,
 'parallel', CASE WHEN p.prokind = 'a' THEN NULL WHEN p.proparallel = 's' THEN 'safe' WHEN p.proparallel = 'r' THEN 'restricted' ELSE 'unsafe' END) AS facts,
 NULL::json AS sequence_owner,
 (coalesce(octet_length(obj_description(p.oid, 'pg_proc')), 0) > 8192
  OR CASE WHEN p.prokind = 'p' THEN false ELSE coalesce(octet_length(pg_get_function_result(p.oid)), 0) > 8192 END
  OR octet_length(pg_get_function_arguments(p.oid)) > 1048576
  OR CASE WHEN p.prokind = 'a' THEN false ELSE octet_length(pg_get_functiondef(p.oid)) > 1048576 OR octet_length(p.prosrc) > 1048576 END) AS oversized
FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace JOIN pg_language language ON language.oid = p.prolang
WHERE n.nspname = $1 AND p.proname = $2 AND pg_get_function_identity_arguments(p.oid) = $3 AND p.prokind::text = $4
"#;
const EXTENSION: &str = r#"
SELECT pg_get_userbyid(e.extowner)::text AS owner, obj_description(e.oid, 'pg_extension') AS comment,
 NULL::text AS definition_sql,
 json_build_object('kind', 'extension', 'version', e.extversion::text, 'schema', n.nspname::text) AS facts,
 NULL::json AS sequence_owner,
 (coalesce(octet_length(obj_description(e.oid, 'pg_extension')), 0) > 8192 OR octet_length(e.extversion::text) > 8192) AS oversized
FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace
WHERE n.nspname = $1 AND e.extname = $2
"#;
const SEQUENCE: &str = r#"
SELECT pg_get_userbyid(c.relowner)::text AS owner, obj_description(c.oid, 'pg_class') AS comment,
 NULL::text AS definition_sql,
 json_build_object('kind', 'sequence', 'dataType', s.data_type::text,
 'start', s.start_value::text, 'increment', s.increment_by::text,
 'minValue', s.min_value::text, 'maxValue', s.max_value::text,
 'cycle', s.cycle, 'cache', s.cache_size::text, 'lastValue', s.last_value::text, 'ownedBy', NULL) AS facts,
 CASE WHEN owned.owned_schema IS NULL THEN NULL ELSE json_build_array(owned.owned_schema, owned.owned_table, owned.owned_column) END AS sequence_owner,
 coalesce(octet_length(obj_description(c.oid, 'pg_class')), 0) > 8192 AS oversized
FROM pg_sequences s JOIN pg_namespace n ON n.nspname = s.schemaname
JOIN pg_class c ON c.relnamespace = n.oid AND c.relname = s.sequencename AND c.relkind = 'S'
LEFT JOIN LATERAL (
 SELECT target_ns.nspname::text AS owned_schema, target.relname::text AS owned_table, attribute.attname::text AS owned_column
 FROM pg_depend dependency JOIN pg_class target ON target.oid = dependency.refobjid
 JOIN pg_namespace target_ns ON target_ns.oid = target.relnamespace
 JOIN pg_attribute attribute ON attribute.attrelid = target.oid AND attribute.attnum = dependency.refobjsubid
 WHERE dependency.classid = 'pg_class'::regclass AND dependency.objid = c.oid AND dependency.deptype IN ('a', 'i') LIMIT 1
) owned ON true
WHERE s.schemaname = $1 AND s.sequencename = $2
"#;

#[cfg(test)]
mod tests;
