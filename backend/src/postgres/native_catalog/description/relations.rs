//! Relation-oriented reconstruction, not a canonical dump: storage, ordinary
//! INHERITS, RLS, triggers and grants remain outside baseline description scope.
//! Foreign relations are inspected through pg_catalog only, never through FDWs.
use super::components::{self, Budget, Sql};
use super::*;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Header {
    oid: u32,
    owner: Option<String>,
    comment: Option<String>,
    version: u32,
    partition_key: Option<String>,
    partition_bound: Option<String>,
    parent_schema: Option<String>,
    parent_name: Option<String>,
    server: Option<String>,
}
#[derive(Deserialize)]
struct Column {
    number: i32,
    name: String,
    data_type: String,
    not_null: bool,
    default_expr: Option<String>,
    identity: String,
    generated: String,
    collation_schema: Option<String>,
    collation_name: Option<String>,
    seq_start: Option<String>,
    seq_increment: Option<String>,
    seq_min: Option<String>,
    seq_max: Option<String>,
    seq_cache: Option<String>,
    seq_cycle: Option<bool>,
}
#[derive(Deserialize)]
struct Constraint {
    name: String,
    definition: String,
}
#[derive(Deserialize)]
struct Index {
    definition: String,
}
#[derive(Deserialize)]
struct FdwOption {
    number: i32,
    option: String,
}

pub(super) async fn load(
    client: &Client,
    reference: PgObjectRef,
) -> Result<PgObjectDescription, CatalogError> {
    let foreign = reference.kind == PgObjectKind::ForeignTable;
    let kinds: Vec<&str> = if foreign { vec!["f"] } else { vec!["r", "p"] };
    let mut budget = Budget::default();
    let header: Header = components::one(
        components::rows(
            client,
            HEADER,
            &[&reference.schema, &reference.name, &kinds],
            &["owner", "comment", "parent_schema", "parent_name", "server"],
            &["partition_key", "partition_bound"],
            &mut budget,
        )
        .await?,
    )?;
    let columns = if foreign || header.parent_name.is_none() {
        components::rows(
            client,
            &columns_sql(header.version),
            &[&header.oid],
            &["name", "data_type", "collation_schema", "collation_name"],
            &["default_expr"],
            &mut budget,
        )
        .await?
    } else {
        Vec::new()
    };
    let constraints = components::rows(
        client,
        CONSTRAINTS,
        &[&header.oid, &foreign],
        &["name"],
        &["definition"],
        &mut budget,
    )
    .await?;
    let mut options = BTreeMap::<i32, Vec<String>>::new();
    let indexes = if foreign {
        for option in components::rows::<FdwOption>(
            client,
            FDW_OPTIONS,
            &[&header.oid],
            &["option"],
            &[],
            &mut budget,
        )
        .await?
        {
            options
                .entry(option.number)
                .or_default()
                .push(option.option);
        }
        Vec::new()
    } else {
        components::rows(
            client,
            INDEXES,
            &[&header.oid],
            &[],
            &["definition"],
            &mut budget,
        )
        .await?
    };
    render(reference, header, columns, constraints, indexes, options)
}

fn render(
    reference: PgObjectRef,
    header: Header,
    columns: Vec<Column>,
    constraints: Vec<Constraint>,
    indexes: Vec<Index>,
    options: BTreeMap<i32, Vec<String>>,
) -> Result<PgObjectDescription, CatalogError> {
    let foreign = reference.kind == PgObjectKind::ForeignTable;
    let mut sql = Sql::default();
    sql.push(if foreign {
        "CREATE FOREIGN TABLE "
    } else {
        "CREATE TABLE "
    })?;
    sql.target(&reference)?;
    let parent = header
        .parent_schema
        .as_deref()
        .zip(header.parent_name.as_deref());
    if !foreign {
        if header.parent_schema.is_some() != header.parent_name.is_some() {
            return Err(CatalogError::InvalidResponse);
        }
        if let Some((schema, name)) = parent {
            sql.push(" PARTITION OF ")?;
            sql.qualified(schema, name)?;
        }
    }
    let partition = !foreign && parent.is_some();
    let definitions = !columns.is_empty() || !constraints.is_empty();
    if !partition || definitions {
        sql.push(" (\n")?;
        let mut first = true;
        for column in &columns {
            if !first {
                sql.push(",\n")?;
            }
            first = false;
            render_column(
                &mut sql,
                column,
                options.get(&column.number).map(Vec::as_slice),
                foreign,
            )?;
        }
        for constraint in &constraints {
            if !first {
                sql.push(",\n")?;
            }
            first = false;
            sql.push("  CONSTRAINT ")?;
            sql.ident(&constraint.name)?;
            sql.push(" ")?;
            sql.push(&constraint.definition)?;
        }
        sql.push("\n)")?;
    }
    if foreign {
        let server = header
            .server
            .as_ref()
            .ok_or(CatalogError::InvalidResponse)?;
        sql.push(" SERVER ")?;
        sql.ident(server)?;
        render_options(&mut sql, options.get(&0).map(Vec::as_slice))?;
    } else {
        if partition {
            sql.push(" ")?;
            sql.push(
                header
                    .partition_bound
                    .as_deref()
                    .ok_or(CatalogError::InvalidResponse)?,
            )?;
        }
        if let Some(key) = &header.partition_key {
            sql.push(" PARTITION BY ")?;
            sql.push(key)?;
        }
    }
    sql.push(";")?;
    for index in indexes {
        sql.push("\n")?;
        sql.push(index.definition.trim_end_matches(';'))?;
        sql.push(";")?;
    }
    let facts = if foreign {
        PgObjectFacts::ForeignTable {
            server: header.server.unwrap(),
        }
    } else {
        PgObjectFacts::Table
    };
    components::finish(reference, header.owner, header.comment, sql, facts)
}
fn render_column(
    sql: &mut Sql,
    column: &Column,
    options: Option<&[String]>,
    foreign: bool,
) -> Result<(), CatalogError> {
    sql.push("  ")?;
    sql.ident(&column.name)?;
    sql.push(" ")?;
    sql.push(&column.data_type)?;
    if foreign {
        render_options(sql, options)?;
    }
    if column.collation_schema.is_some() != column.collation_name.is_some() {
        return Err(CatalogError::InvalidResponse);
    }
    if let Some((schema, name)) = column
        .collation_schema
        .as_deref()
        .zip(column.collation_name.as_deref())
    {
        sql.push(" COLLATE ")?;
        sql.qualified(schema, name)?;
    }
    if !column.identity.is_empty() {
        if !column.generated.is_empty() || column.default_expr.is_some() {
            return Err(CatalogError::InvalidResponse);
        }
        sql.push(match column.identity.as_str() {
            "a" => " GENERATED ALWAYS AS IDENTITY",
            "d" => " GENERATED BY DEFAULT AS IDENTITY",
            _ => return Err(CatalogError::InvalidResponse),
        })?;
        sql.push(" (")?;
        for (index, (label, value)) in [
            ("START WITH ", &column.seq_start),
            ("INCREMENT BY ", &column.seq_increment),
            ("MINVALUE ", &column.seq_min),
            ("MAXVALUE ", &column.seq_max),
            ("CACHE ", &column.seq_cache),
        ]
        .into_iter()
        .enumerate()
        {
            let value = value.as_deref().ok_or(CatalogError::InvalidResponse)?;
            value
                .parse::<i64>()
                .map_err(|_| CatalogError::InvalidResponse)?;
            if index > 0 {
                sql.push(" ")?;
            }
            sql.push(label)?;
            sql.push(value)?;
        }
        sql.push(if column.seq_cycle.ok_or(CatalogError::InvalidResponse)? {
            " CYCLE)"
        } else {
            " NO CYCLE)"
        })?;
    } else if !column.generated.is_empty() {
        sql.push(" GENERATED ALWAYS AS (")?;
        sql.push(
            column
                .default_expr
                .as_deref()
                .ok_or(CatalogError::InvalidResponse)?,
        )?;
        sql.push(match column.generated.as_str() {
            "s" => ") STORED",
            "v" => ") VIRTUAL",
            _ => return Err(CatalogError::InvalidResponse),
        })?;
    } else if let Some(default) = &column.default_expr {
        sql.push(" DEFAULT ")?;
        sql.push(default)?;
    }
    if column.not_null {
        sql.push(" NOT NULL")?;
    }
    Ok(())
}
fn render_options(sql: &mut Sql, options: Option<&[String]>) -> Result<(), CatalogError> {
    let Some(options) = options.filter(|options| !options.is_empty()) else {
        return Ok(());
    };
    sql.push(" OPTIONS (")?;
    for (index, option) in options.iter().enumerate() {
        let (name, value) = option
            .split_once('=')
            .filter(|(name, _)| !name.is_empty())
            .ok_or(CatalogError::InvalidResponse)?;
        if index > 0 {
            sql.push(", ")?;
        }
        sql.ident(name)?;
        sql.push(" ")?;
        sql.literal(value)?;
    }
    sql.push(")")
}

const HEADER: &str = r#"
SELECT c.oid::bigint AS oid, pg_get_userbyid(c.relowner)::text AS owner, obj_description(c.oid, 'pg_class') AS comment,
 current_setting('server_version_num')::int AS version,
 CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END AS partition_key,
 CASE WHEN c.relispartition THEN pg_get_expr(c.relpartbound, c.oid) END AS partition_bound,
 parent_ns.nspname::text AS parent_schema, parent.relname::text AS parent_name, server.srvname::text AS server
FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_inherits inherited ON c.relispartition AND inherited.inhrelid = c.oid
LEFT JOIN pg_class parent ON parent.oid = inherited.inhparent
LEFT JOIN pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace
LEFT JOIN pg_foreign_table ft ON ft.ftrelid = c.oid LEFT JOIN pg_foreign_server server ON server.oid = ft.ftserver
WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind::text = ANY($3)
"#;
fn columns_sql(version: u32) -> String {
    // attgenerated was added in PG12. The unused branch must not reference an
    // absent catalog column, even inside a CASE expression.
    let generated = if version >= 120_000 {
        "a.attgenerated::text"
    } else {
        "''::text"
    };
    format!(
        r#"
SELECT a.attnum::int AS number, a.attname::text AS name, format_type(a.atttypid, a.atttypmod)::text AS data_type,
 a.attnotnull AS not_null, pg_get_expr(d.adbin, d.adrelid, true)::text AS default_expr,
 a.attidentity::text AS identity, {generated} AS generated,
 cn.nspname::text AS collation_schema, col.collname::text AS collation_name,
 seq.seqstart::text AS seq_start, seq.seqincrement::text AS seq_increment, seq.seqmin::text AS seq_min,
 seq.seqmax::text AS seq_max, seq.seqcache::text AS seq_cache, seq.seqcycle AS seq_cycle
FROM pg_attribute a JOIN pg_type typ ON typ.oid = a.atttypid
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
LEFT JOIN pg_collation col ON col.oid = a.attcollation AND a.attcollation <> typ.typcollation
LEFT JOIN pg_namespace cn ON cn.oid = col.collnamespace
LEFT JOIN pg_depend dep ON a.attidentity <> '' AND dep.refclassid = 'pg_class'::regclass AND dep.refobjid = a.attrelid AND dep.refobjsubid = a.attnum AND dep.classid = 'pg_class'::regclass AND dep.deptype = 'i'
LEFT JOIN pg_sequence seq ON seq.seqrelid = dep.objid
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum
"#
    )
}
// Column nullability is emitted from attnotnull; do not duplicate catalog
// NOT NULL entries as table constraints. Triggers are outside this reconstruction.
const CONSTRAINTS: &str = r#"
SELECT con.conname::text AS name, pg_get_constraintdef(con.oid, true)::text AS definition
FROM pg_constraint con WHERE con.conrelid = $1 AND (($2::bool AND con.contype = 'c') OR (NOT $2::bool AND con.conislocal AND con.contype IN ('c','p','u','f','x')))
ORDER BY con.contype, con.conname
"#;
const INDEXES: &str = r#"
SELECT pg_get_indexdef(idx.oid)::text AS definition
FROM pg_index i JOIN pg_class idx ON idx.oid = i.indexrelid
WHERE i.indrelid = $1 AND NOT EXISTS (SELECT 1 FROM pg_constraint con WHERE con.conindid = idx.oid AND con.conrelid = i.indrelid AND con.contype IN ('p','u','x'))
 AND NOT EXISTS (SELECT 1 FROM pg_inherits inherited WHERE inherited.inhrelid = idx.oid)
ORDER BY idx.relname
"#;
const FDW_OPTIONS: &str = r#"
SELECT number, option FROM (
 SELECT 0::int AS number, o.value AS option, o.position
 FROM pg_foreign_table ft CROSS JOIN LATERAL unnest(ft.ftoptions) WITH ORDINALITY AS o(value, position) WHERE ft.ftrelid = $1
 UNION ALL
 SELECT a.attnum::int AS number, o.value AS option, o.position
 FROM pg_attribute a CROSS JOIN LATERAL unnest(a.attfdwoptions) WITH ORDINALITY AS o(value, position)
 WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
) options ORDER BY number, position
"#;

#[cfg(test)]
mod tests;
