//! Bounded enum/composite/range/multirange and domain reconstruction.
use super::components::{self, Budget, Sql};
use super::*;
use crate::postgres::objects::PgTypeAttribute;

#[derive(Deserialize)]
struct Header {
    oid: u32,
    kind: String,
    owner: Option<String>,
    comment: Option<String>,
    version: u32,
}
#[derive(Deserialize)]
struct Value {
    value: String,
}
#[derive(Deserialize)]
struct Range {
    subtype: String,
    range_schema: String,
    range_name: String,
    subtype_schema: String,
    subtype_name: String,
    multirange_schema: Option<String>,
    multirange_name: Option<String>,
    opclass_schema: Option<String>,
    opclass_name: Option<String>,
    collation_schema: Option<String>,
    collation_name: Option<String>,
    canonical_schema: Option<String>,
    canonical_name: Option<String>,
    diff_schema: Option<String>,
    diff_name: Option<String>,
}
#[derive(Deserialize)]
struct Domain {
    oid: u32,
    owner: Option<String>,
    comment: Option<String>,
    base_type: String,
    not_null: bool,
    default_value: Option<String>,
}

pub(super) async fn load(
    client: &Client,
    reference: PgObjectRef,
) -> Result<PgObjectDescription, CatalogError> {
    let mut budget = Budget::default();
    if reference.kind == PgObjectKind::Domain {
        let domain = components::one(
            components::rows::<Domain>(
                client,
                DOMAIN,
                &[&reference.schema, &reference.name],
                &["owner", "comment", "base_type"],
                &["default_value"],
                &mut budget,
            )
            .await?,
        )?;
        let checks = components::rows::<Value>(
            client,
            DOMAIN_CHECKS,
            &[&domain.oid],
            &[],
            &["value"],
            &mut budget,
        )
        .await?
        .into_iter()
        .map(|value| value.value)
        .collect();
        return render_domain(reference, domain, checks);
    }
    let header = components::one(
        components::rows::<Header>(
            client,
            HEADER,
            &[&reference.schema, &reference.name],
            &["owner", "comment", "kind"],
            &[],
            &mut budget,
        )
        .await?,
    )?;
    match header.kind.as_str() {
        "e" => {
            let labels = components::rows::<Value>(
                client,
                ENUM_LABELS,
                &[&header.oid],
                &["value"],
                &[],
                &mut budget,
            )
            .await?
            .into_iter()
            .map(|value| value.value)
            .collect();
            render_enum(reference, header, labels)
        }
        "c" => {
            let attributes = components::rows::<PgTypeAttribute>(
                client,
                ATTRIBUTES,
                &[&header.oid],
                &["name", "dataType"],
                &[],
                &mut budget,
            )
            .await?;
            render_composite(reference, header, attributes)
        }
        "r" | "m" => {
            let sql = range_sql(header.version);
            let range = components::one(
                components::rows::<Range>(
                    client,
                    &sql,
                    &[&header.oid, &header.kind],
                    &[
                        "subtype",
                        "range_schema",
                        "range_name",
                        "subtype_schema",
                        "subtype_name",
                        "multirange_schema",
                        "multirange_name",
                        "opclass_schema",
                        "opclass_name",
                        "collation_schema",
                        "collation_name",
                        "canonical_schema",
                        "canonical_name",
                        "diff_schema",
                        "diff_name",
                    ],
                    &[],
                    &mut budget,
                )
                .await?,
            )?;
            render_range(reference, header, range)
        }
        _ => Err(CatalogError::InvalidResponse),
    }
}
fn render_enum(
    reference: PgObjectRef,
    header: Header,
    labels: Vec<String>,
) -> Result<PgObjectDescription, CatalogError> {
    let mut sql = Sql::default();
    sql.push("CREATE TYPE ")?;
    sql.target(&reference)?;
    sql.push(" AS ENUM (")?;
    for (index, label) in labels.iter().enumerate() {
        if index > 0 {
            sql.push(", ")?;
        }
        sql.literal(label)?;
    }
    sql.push(");")?;
    components::finish(
        reference,
        header.owner,
        header.comment,
        sql,
        PgObjectFacts::Type {
            class: PgTypeClass::Enum,
            enum_labels: Some(labels),
            attributes: None,
            subtype: None,
        },
    )
}
fn render_composite(
    reference: PgObjectRef,
    header: Header,
    attributes: Vec<PgTypeAttribute>,
) -> Result<PgObjectDescription, CatalogError> {
    let mut sql = Sql::default();
    sql.push("CREATE TYPE ")?;
    sql.target(&reference)?;
    sql.push(" AS (\n")?;
    for (index, attribute) in attributes.iter().enumerate() {
        if index > 0 {
            sql.push(",\n")?;
        }
        sql.push("  ")?;
        sql.ident(&attribute.name)?;
        sql.push(" ")?;
        sql.push(&attribute.data_type)?;
    }
    sql.push("\n);")?;
    components::finish(
        reference,
        header.owner,
        header.comment,
        sql,
        PgObjectFacts::Type {
            class: PgTypeClass::Composite,
            enum_labels: None,
            attributes: Some(attributes),
            subtype: None,
        },
    )
}
fn render_range(
    reference: PgObjectRef,
    header: Header,
    range: Range,
) -> Result<PgObjectDescription, CatalogError> {
    let multi = header.kind == "m";
    if multi
        && (range.multirange_schema.as_deref() != reference.schema.as_deref()
            || range.multirange_name.as_deref() != Some(reference.name.as_str()))
    {
        return Err(CatalogError::InvalidResponse);
    }
    let mut sql = Sql::default();
    sql.push("CREATE TYPE ")?;
    if multi {
        sql.qualified(&range.range_schema, &range.range_name)?;
    } else {
        sql.target(&reference)?;
    }
    sql.push(" AS RANGE (\n  SUBTYPE = ")?;
    sql.qualified(&range.subtype_schema, &range.subtype_name)?;
    for (label, schema, name) in [
        (
            "SUBTYPE_OPCLASS",
            &range.opclass_schema,
            &range.opclass_name,
        ),
        ("COLLATION", &range.collation_schema, &range.collation_name),
        ("CANONICAL", &range.canonical_schema, &range.canonical_name),
        ("SUBTYPE_DIFF", &range.diff_schema, &range.diff_name),
        (
            "MULTIRANGE_TYPE_NAME",
            &range.multirange_schema,
            &range.multirange_name,
        ),
    ] {
        if schema.is_some() != name.is_some() {
            return Err(CatalogError::InvalidResponse);
        }
        if let Some((schema, name)) = schema.as_deref().zip(name.as_deref()) {
            sql.push(",\n  ")?;
            sql.push(label)?;
            sql.push(" = ")?;
            sql.qualified(schema, name)?;
        }
    }
    sql.push("\n);")?;
    components::finish(
        reference,
        header.owner,
        header.comment,
        sql,
        PgObjectFacts::Type {
            class: if multi {
                PgTypeClass::Multirange
            } else {
                PgTypeClass::Range
            },
            enum_labels: None,
            attributes: None,
            subtype: Some(range.subtype),
        },
    )
}
fn render_domain(
    reference: PgObjectRef,
    domain: Domain,
    checks: Vec<String>,
) -> Result<PgObjectDescription, CatalogError> {
    let mut sql = Sql::default();
    sql.push("CREATE DOMAIN ")?;
    sql.target(&reference)?;
    sql.push(" AS ")?;
    sql.push(&domain.base_type)?;
    if let Some(default) = &domain.default_value {
        sql.push(" DEFAULT ")?;
        sql.push(default)?;
    }
    if domain.not_null {
        sql.push(" NOT NULL")?;
    }
    for check in &checks {
        sql.push("\n  ")?;
        sql.push(check)?;
    }
    sql.push(";")?;
    components::finish(
        reference,
        domain.owner,
        domain.comment,
        sql,
        PgObjectFacts::Domain {
            base_type: domain.base_type,
            not_null: domain.not_null,
            default_value: domain.default_value,
            checks,
        },
    )
}

const HEADER: &str = r#"
SELECT t.oid::bigint AS oid, t.typtype::text AS kind, pg_get_userbyid(t.typowner)::text AS owner,
 obj_description(t.oid, 'pg_type') AS comment, current_setting('server_version_num')::int AS version
FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace LEFT JOIN pg_class rel ON rel.oid = t.typrelid
WHERE n.nspname = $1 AND t.typname = $2 AND t.typtype IN ('c', 'e', 'r', 'm') AND (t.typtype <> 'c' OR rel.relkind = 'c')
"#;
const ENUM_LABELS: &str =
    "SELECT enumlabel::text AS value FROM pg_enum WHERE enumtypid = $1 ORDER BY enumsortorder";
const ATTRIBUTES: &str = r#"
SELECT a.attname::text AS name, format_type(a.atttypid, a.atttypmod)::text AS "dataType", NOT a.attnotnull AS nullable
FROM pg_type t JOIN pg_attribute a ON a.attrelid = t.typrelid WHERE t.oid = $1 AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum
"#;
fn range_sql(version: u32) -> String {
    let (join, filter) = if version >= 140_000 {
        (
            "LEFT JOIN pg_type multi ON multi.oid = r.rngmultitypid",
            "($2 = 'm' AND r.rngmultitypid = $1)",
        )
    } else {
        ("LEFT JOIN pg_type multi ON false", "false")
    };
    format!(
        r#"
SELECT format_type(r.rngsubtype, NULL)::text AS subtype, rn.nspname::text AS range_schema, rt.typname::text AS range_name,
 sn.nspname::text AS subtype_schema, st.typname::text AS subtype_name, mn.nspname::text AS multirange_schema, multi.typname::text AS multirange_name,
 onsp.nspname::text AS opclass_schema, op.opcname::text AS opclass_name,
 cn.nspname::text AS collation_schema, col.collname::text AS collation_name,
 fnsp.nspname::text AS canonical_schema, canonical.proname::text AS canonical_name,
 dn.nspname::text AS diff_schema, diff.proname::text AS diff_name
FROM pg_range r JOIN pg_type rt ON rt.oid = r.rngtypid JOIN pg_namespace rn ON rn.oid = rt.typnamespace
JOIN pg_type st ON st.oid = r.rngsubtype JOIN pg_namespace sn ON sn.oid = st.typnamespace
{join} LEFT JOIN pg_namespace mn ON mn.oid = multi.typnamespace
JOIN pg_opclass op ON op.oid = r.rngsubopc JOIN pg_namespace onsp ON onsp.oid = op.opcnamespace
LEFT JOIN pg_collation col ON col.oid = NULLIF(r.rngcollation, 0) LEFT JOIN pg_namespace cn ON cn.oid = col.collnamespace
LEFT JOIN pg_proc canonical ON canonical.oid = NULLIF(r.rngcanonical, 0) LEFT JOIN pg_namespace fnsp ON fnsp.oid = canonical.pronamespace
LEFT JOIN pg_proc diff ON diff.oid = NULLIF(r.rngsubdiff, 0) LEFT JOIN pg_namespace dn ON dn.oid = diff.pronamespace
WHERE ($2 = 'r' AND r.rngtypid = $1) OR {filter}
"#
    )
}
const DOMAIN: &str = r#"
SELECT t.oid::bigint AS oid, pg_get_userbyid(t.typowner)::text AS owner, obj_description(t.oid, 'pg_type') AS comment,
 format_type(t.typbasetype, t.typtypmod)::text AS base_type, t.typnotnull AS not_null, t.typdefault::text AS default_value
FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE n.nspname = $1 AND t.typname = $2 AND t.typtype = 'd'
"#;
// NOT NULL is represented by typnotnull, not by the CHECK list.
const DOMAIN_CHECKS: &str = "SELECT pg_get_constraintdef(c.oid, true)::text AS value FROM pg_constraint c WHERE c.contypid = $1 AND c.contype = 'c' ORDER BY c.conname";

#[cfg(test)]
mod tests;
