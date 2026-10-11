//! Capture: one table read from `sqlite_master.sql` (parsed twice, by our
//! slicing parser and by `sqlparser`) and cross-checked against the PRAGMAs.
//! Any disagreement makes the baseline read-only with a reason; nothing is
//! guessed, because a rebuild restates the whole table.
use super::definition::{
    cross_check, parse_create_index, parse_create_table, MemberTarget, ParsedConstraint,
    ParsedIndex, ParsedTable,
};
use super::lexer::{all_names, dequote, tokenize, Kind};
use super::{
    column_origin, features, object_origin, parse_version, qualified, quote, quote_literal,
    TAG_CHECK, TAG_FOREIGN_KEY, TAG_INDEX, TAG_PRIMARY_KEY, TAG_UNIQUE,
};
use crate::schema_edit::{
    CheckModel, ColumnExtra, ColumnModel, DefaultValue, Dependent, DriftToken, Editability,
    ForeignKeyModel, Identity, InboundReference, IndexMember, IndexModel, IndexTarget, KeyModel,
    Origin, ReferentialAction, SchemaBaseline, SchemaEngine, ServerFacts, ServerFlavor, TableModel,
    TableOptions, MAX_SCHEMA_EDIT_COLUMNS, MAX_SCHEMA_EDIT_OBJECTS,
};
use sqlx::{Row, SqliteConnection};
use std::collections::BTreeSet;

/// Rows counted for the large-table note; the count stops here.
pub(super) const ROW_ESTIMATE_CAP: u64 = 100_001;
/// Catalog rows read per query (columns, keys, indexes, dependents).
const MAX_CATALOG_ROWS: i64 = 5_000;

pub(super) fn db_error(error: sqlx::Error) -> String {
    match error {
        sqlx::Error::Database(error) => error.message().to_owned(),
        other => other.to_string(),
    }
}

/// One `pragma_table_xinfo` row.
#[derive(Clone, Debug)]
struct CatalogColumn {
    cid: i64,
    name: String,
    data_type: String,
    not_null: bool,
    default: Option<String>,
    pk: i64,
    hidden: i64,
}

/// One `pragma_foreign_key_list` key, its rows grouped by id.
#[derive(Clone, Debug)]
struct CatalogForeignKey {
    table: String,
    from: Vec<String>,
    to: Vec<Option<String>>,
    on_update: String,
    on_delete: String,
    match_kind: String,
}

/// One `pragma_index_xinfo` key column.
#[derive(Clone, Debug)]
struct CatalogIndexColumn {
    cid: i64,
    name: Option<String>,
    descending: bool,
    collation: Option<String>,
}

#[derive(Clone, Debug)]
struct CatalogIndex {
    name: String,
    unique: bool,
    /// `c` (CREATE INDEX), `u` (UNIQUE) or `pk`.
    origin: String,
    partial: bool,
    columns: Vec<CatalogIndexColumn>,
}

/// `PRAGMA table_list` for this table (SQLite 3.37+).
#[derive(Clone, Debug)]
struct Listing {
    kind: String,
    without_rowid: bool,
    strict: bool,
}

struct Catalog {
    columns: Vec<CatalogColumn>,
    foreign_keys: Vec<CatalogForeignKey>,
    indexes: Vec<CatalogIndex>,
    /// Explicit index names that have a definition in `sqlite_master`.
    defined_indexes: Vec<(String, String)>,
}

/// The drift token: `PRAGMA schema_version` of `schema`.
pub(super) async fn drift_token(
    connection: &mut SqliteConnection,
    schema: &str,
) -> Result<DriftToken, sqlx::Error> {
    let version = sqlx::query_scalar::<_, i64>(&format!("PRAGMA {}.schema_version", quote(schema)))
        .persistent(false)
        .fetch_one(&mut *connection)
        .await?;
    Ok(DriftToken(format!("schema_version:{version}")))
}

/// Captures `schema.table`. `estimate_rows` bounds a row count for the
/// large-table note (skipped by the executor's in-transaction re-capture).
pub(super) async fn capture_table(
    connection: &mut SqliteConnection,
    schema: &str,
    table: &str,
    sqlite_version: &str,
    estimate_rows: bool,
) -> Result<SchemaBaseline, String> {
    let master = format!("{}.sqlite_master", quote(schema));
    let (kind, sql) = sqlx::query_as::<_, (String, Option<String>)>(&format!(
        "SELECT type, sql FROM {master} WHERE name = ?1 AND type IN ('table', 'view')"
    ))
    .persistent(false)
    .bind(table)
    .fetch_optional(&mut *connection)
    .await
    .map_err(db_error)?
    .ok_or_else(|| format!("{schema}.{table} no longer exists"))?;
    let drift = drift_token(connection, schema).await.map_err(db_error)?;
    let version = parse_version(sqlite_version);

    let columns = read_columns(connection, schema, table).await?;
    let foreign_keys = read_foreign_keys(connection, schema, table).await?;
    let indexes = read_indexes(connection, schema, table).await?;
    let defined_indexes = sqlx::query_as::<_, (String, String)>(&format!(
        "SELECT name, sql FROM {master} WHERE type = 'index' AND tbl_name = ?1 COLLATE NOCASE \
         AND sql IS NOT NULL ORDER BY name LIMIT ?2"
    ))
    .persistent(false)
    .bind(table)
    .bind(MAX_CATALOG_ROWS)
    .fetch_all(&mut *connection)
    .await
    .map_err(db_error)?;
    let dependents = read_dependents(connection, schema, table).await?;
    let inbound = read_inbound(connection, schema, table).await?;
    let listing = if version >= (3, 37, 0) {
        read_listing(connection, schema, table).await?
    } else {
        None
    };

    let mut reasons = Vec::new();
    if table
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("sqlite_"))
    {
        reasons.push("SQLite's internal tables are not editable".to_owned());
    }
    if kind == "view" {
        reasons.push("Views are not editable in the structure editor".to_owned());
    }
    match listing.as_ref().map(|listing| listing.kind.as_str()) {
        Some("virtual") => reasons.push("Virtual tables are not editable".to_owned()),
        Some("shadow") => {
            reasons.push("This table belongs to a virtual table and is not editable".to_owned())
        }
        _ => {}
    }
    if listing.is_none() && kind == "table" {
        if let Some(owner) = shadow_owner(connection, schema, table).await? {
            reasons.push(format!(
                "This table looks like part of virtual table {owner} and is not editable"
            ));
        }
    }
    if version < (3, 26, 0) {
        reasons.push("Editing structure needs SQLite 3.26 or later".to_owned());
    }
    if schema != "temp" {
        let temp_triggers = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM temp.sqlite_master WHERE type = 'trigger' \
             AND tbl_name = ?1 COLLATE NOCASE",
        )
        .persistent(false)
        .bind(table)
        .fetch_one(&mut *connection)
        .await
        .map_err(db_error)?;
        if temp_triggers > 0 {
            reasons.push(
                "A temporary trigger depends on this table; a rebuild would drop it".to_owned(),
            );
        }
    }
    if columns.len() > MAX_SCHEMA_EDIT_COLUMNS
        || foreign_keys.len() > MAX_SCHEMA_EDIT_OBJECTS
        || indexes.len() > MAX_SCHEMA_EDIT_OBJECTS
    {
        reasons.push("The table is too large for the structure editor".to_owned());
    }

    let catalog = Catalog {
        columns,
        foreign_keys,
        indexes,
        defined_indexes,
    };
    let built = match (&sql, kind.as_str()) {
        (Some(sql), "table") if reasons.is_empty() => {
            build_model(schema, table, sql, &catalog, listing.as_ref())
        }
        (None, "table") => Err("SQLite stores no definition for this table".to_owned()),
        _ => Err(String::new()),
    };
    let model = match built {
        Ok(model) => model,
        Err(reason) => {
            if !reason.is_empty() {
                reasons.push(reason);
            }
            if reasons.is_empty() {
                reasons.push("This table is not editable".to_owned());
            }
            catalog_model(schema, table, &catalog, listing.as_ref())
        }
    };
    let estimated_rows = if estimate_rows && kind == "table" && reasons.is_empty() {
        Some(count_rows(connection, schema, table).await?)
    } else {
        None
    };
    Ok(SchemaBaseline {
        engine: SchemaEngine::Sqlite,
        server: ServerFacts {
            flavor: ServerFlavor::Sqlite,
            version: sqlite_version.to_owned(),
            features: features(version),
        },
        table: model,
        drift,
        editable: if reasons.is_empty() {
            Editability::Editable
        } else {
            Editability::ReadOnly { reasons }
        },
        inbound,
        dependents,
        estimated_rows,
    })
}

async fn read_columns(
    connection: &mut SqliteConnection,
    schema: &str,
    table: &str,
) -> Result<Vec<CatalogColumn>, String> {
    let rows = sqlx::query_as::<_, (i64, String, Option<String>, i64, Option<String>, i64, i64)>(
        "SELECT cid, name, type, \"notnull\", dflt_value, pk, hidden \
         FROM pragma_table_xinfo(?1, ?2) ORDER BY cid LIMIT ?3",
    )
    .persistent(false)
    .bind(table)
    .bind(schema)
    .bind(MAX_CATALOG_ROWS)
    .fetch_all(&mut *connection)
    .await
    .map_err(db_error)?;
    Ok(rows
        .into_iter()
        .map(
            |(cid, name, data_type, not_null, default, pk, hidden)| CatalogColumn {
                cid,
                name,
                data_type: data_type.unwrap_or_default(),
                not_null: not_null != 0,
                default,
                pk,
                hidden,
            },
        )
        .collect())
}

async fn read_foreign_keys(
    connection: &mut SqliteConnection,
    schema: &str,
    table: &str,
) -> Result<Vec<CatalogForeignKey>, String> {
    type KeyRow = (i64, String, String, Option<String>, String, String, String);
    let rows = sqlx::query_as::<_, KeyRow>(
        "SELECT id, \"table\", \"from\", \"to\", on_update, on_delete, \"match\" \
         FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq LIMIT ?3",
    )
    .persistent(false)
    .bind(table)
    .bind(schema)
    .bind(MAX_CATALOG_ROWS)
    .fetch_all(&mut *connection)
    .await
    .map_err(db_error)?;
    let mut keys: Vec<(i64, CatalogForeignKey)> = Vec::new();
    for (id, referenced, from, to, on_update, on_delete, match_kind) in rows {
        match keys.last_mut() {
            Some((last, key)) if *last == id => {
                key.from.push(from);
                key.to.push(to);
            }
            _ => keys.push((
                id,
                CatalogForeignKey {
                    table: referenced,
                    from: vec![from],
                    to: vec![to],
                    on_update,
                    on_delete,
                    match_kind,
                },
            )),
        }
    }
    Ok(keys.into_iter().map(|(_, key)| key).collect())
}

async fn read_indexes(
    connection: &mut SqliteConnection,
    schema: &str,
    table: &str,
) -> Result<Vec<CatalogIndex>, String> {
    let list = sqlx::query_as::<_, (String, i64, String, i64)>(
        "SELECT name, \"unique\", origin, partial FROM pragma_index_list(?1, ?2) \
         ORDER BY name LIMIT ?3",
    )
    .persistent(false)
    .bind(table)
    .bind(schema)
    .bind(MAX_CATALOG_ROWS)
    .fetch_all(&mut *connection)
    .await
    .map_err(db_error)?;
    let mut indexes = Vec::with_capacity(list.len());
    for (name, unique, origin, partial) in list {
        let columns = sqlx::query_as::<_, (i64, Option<String>, i64, Option<String>)>(
            "SELECT cid, name, \"desc\", coll FROM pragma_index_xinfo(?1, ?2) \
             WHERE key = 1 ORDER BY seqno LIMIT ?3",
        )
        .persistent(false)
        .bind(&name)
        .bind(schema)
        .bind(MAX_CATALOG_ROWS)
        .fetch_all(&mut *connection)
        .await
        .map_err(db_error)?
        .into_iter()
        .map(|(cid, name, descending, collation)| CatalogIndexColumn {
            cid,
            name,
            descending: descending != 0,
            collation,
        })
        .collect();
        indexes.push(CatalogIndex {
            name,
            unique: unique != 0,
            origin,
            partial: partial != 0,
            columns,
        });
    }
    Ok(indexes)
}

async fn read_listing(
    connection: &mut SqliteConnection,
    schema: &str,
    table: &str,
) -> Result<Option<Listing>, String> {
    let rows = sqlx::query(&format!(
        "PRAGMA {}.table_list({})",
        quote(schema),
        quote_literal(table)
    ))
    .persistent(false)
    .fetch_all(&mut *connection)
    .await
    .map_err(db_error)?;
    for row in rows {
        let name: String = row.try_get(1).map_err(db_error)?;
        if name != table {
            continue;
        }
        let kind: String = row.try_get(2).map_err(db_error)?;
        let without_rowid: i64 = row.try_get(4).map_err(db_error)?;
        let strict: i64 = row.try_get(5).map_err(db_error)?;
        return Ok(Some(Listing {
            kind,
            without_rowid: without_rowid != 0,
            strict: strict != 0,
        }));
    }
    Ok(None)
}

/// Without `table_list`: a virtual table whose name prefixes this one with
/// an underscore (FTS and R*Tree shadow tables are named that way).
async fn shadow_owner(
    connection: &mut SqliteConnection,
    schema: &str,
    table: &str,
) -> Result<Option<String>, String> {
    let virtuals = sqlx::query_scalar::<_, String>(&format!(
        "SELECT name FROM {}.sqlite_master WHERE type = 'table' \
         AND sql LIKE 'CREATE VIRTUAL TABLE%' ORDER BY name LIMIT ?1",
        quote(schema)
    ))
    .persistent(false)
    .bind(MAX_CATALOG_ROWS)
    .fetch_all(&mut *connection)
    .await
    .map_err(db_error)?;
    let lower = table.to_ascii_lowercase();
    Ok(virtuals.into_iter().find(|owner| {
        let prefix = format!("{}_", owner.to_ascii_lowercase());
        lower.starts_with(&prefix)
    }))
}

/// Triggers on the table (re-created after a rebuild), and views and other
/// triggers whose SQL names it.
async fn read_dependents(
    connection: &mut SqliteConnection,
    schema: &str,
    table: &str,
) -> Result<Vec<Dependent>, String> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>)>(&format!(
        "SELECT type, name, tbl_name, sql FROM {}.sqlite_master \
         WHERE type IN ('trigger', 'view') ORDER BY type, name LIMIT ?1",
        quote(schema)
    ))
    .persistent(false)
    .bind(MAX_CATALOG_ROWS)
    .fetch_all(&mut *connection)
    .await
    .map_err(db_error)?;
    let lower = table.to_ascii_lowercase();
    let mut own = Vec::new();
    let mut others = Vec::new();
    for (kind, name, owner, sql) in rows {
        let mentions = sql
            .as_deref()
            .is_some_and(|sql| all_names(sql).contains(&lower));
        let dependent = |kind: &str| Dependent {
            kind: kind.to_owned(),
            schema: schema.to_owned(),
            name: name.clone(),
            definition: sql.clone(),
        };
        if kind == "trigger" && owner.eq_ignore_ascii_case(table) {
            own.push(dependent(TRIGGER));
        } else if kind == "trigger" && mentions {
            others.push(dependent(REFERENCING_TRIGGER));
        } else if kind == "view" && mentions {
            others.push(dependent(VIEW));
        }
    }
    own.extend(others);
    Ok(own)
}

/// Dependent kinds. Only `trigger` (a trigger on the table itself) is
/// re-created after a rebuild.
pub(super) const TRIGGER: &str = "trigger";
pub(super) const REFERENCING_TRIGGER: &str = "referencing trigger";
pub(super) const VIEW: &str = "view";

async fn read_inbound(
    connection: &mut SqliteConnection,
    schema: &str,
    table: &str,
) -> Result<Vec<InboundReference>, String> {
    let rows = sqlx::query_as::<_, (String, i64, Option<String>, String)>(&format!(
        "SELECT m.name, f.id, f.\"to\", f.\"from\" FROM {}.sqlite_master AS m, \
         pragma_foreign_key_list(m.name, ?2) AS f \
         WHERE m.type = 'table' AND f.\"table\" = ?1 COLLATE NOCASE \
         AND m.name <> ?1 COLLATE NOCASE ORDER BY m.name, f.id, f.seq LIMIT ?3",
        quote(schema)
    ))
    .persistent(false)
    .bind(table)
    .bind(schema)
    .bind(MAX_CATALOG_ROWS)
    .fetch_all(&mut *connection)
    .await
    .map_err(db_error)?;
    let mut inbound: Vec<(String, i64, InboundReference)> = Vec::new();
    for (child, id, to, from) in rows {
        let column = to.unwrap_or_else(|| format!("(primary key, from {from})"));
        match inbound.last_mut() {
            Some((last_child, last_id, reference)) if *last_child == child && *last_id == id => {
                reference.columns.push(column)
            }
            _ => inbound.push((
                child.clone(),
                id,
                InboundReference {
                    schema: schema.to_owned(),
                    table: child,
                    name: format!("fk{id}"),
                    columns: vec![column],
                },
            )),
        }
    }
    Ok(inbound
        .into_iter()
        .map(|(_, _, reference)| reference)
        .collect())
}

async fn count_rows(
    connection: &mut SqliteConnection,
    schema: &str,
    table: &str,
) -> Result<u64, String> {
    let count = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT count(*) FROM (SELECT 1 FROM {} LIMIT {ROW_ESTIMATE_CAP})",
        qualified(schema, table)
    ))
    .persistent(false)
    .fetch_one(&mut *connection)
    .await
    .map_err(db_error)?;
    Ok(u64::try_from(count).unwrap_or(0))
}

/// A default's model form: one string token is a literal, anything else is
/// kept as written.
pub(super) fn default_from_text(text: &str) -> DefaultValue {
    if let Ok(tokens) = tokenize(text) {
        if let [token] = tokens.as_slice() {
            if token.kind == Kind::String {
                return DefaultValue::Literal(dequote(token.text(text)));
            }
        }
    }
    DefaultValue::Expression(crate::schema_edit::SqlFragment::new(text))
}

fn action(text: &str) -> Option<ReferentialAction> {
    Some(match text.to_ascii_uppercase().as_str() {
        "NO ACTION" | "NONE" | "" => ReferentialAction::NoAction,
        "RESTRICT" => ReferentialAction::Restrict,
        "CASCADE" => ReferentialAction::Cascade,
        "SET NULL" => ReferentialAction::SetNull,
        "SET DEFAULT" => ReferentialAction::SetDefault,
        _ => return None,
    })
}

fn mismatch(detail: impl std::fmt::Display) -> String {
    format!("The table definition disagrees with SQLite's catalog: {detail}")
}

/// SQLite's `sqlite3Dequote` on a declared type: only a type that starts
/// with a quote is dequoted.
fn declared_type(text: &str) -> String {
    match text.as_bytes().first() {
        Some(b'"' | b'\'' | b'`' | b'[') => dequote(text),
        _ => text.to_owned(),
    }
}

/// Parses, cross-checks and converts the definition. Every error is a
/// user-facing read-only reason.
fn build_model(
    schema: &str,
    table: &str,
    sql: &str,
    catalog: &Catalog,
    listing: Option<&Listing>,
) -> Result<TableModel, String> {
    let parsed = parse_create_table(sql)
        .map_err(|detail| format!("The table definition could not be read: {detail}"))?;
    cross_check(&parsed, sql).map_err(|detail| {
        format!("The table definition uses syntax the editor cannot verify ({detail})")
    })?;
    if !parsed.name.eq_ignore_ascii_case(table) {
        return Err(mismatch("the definition names another table"));
    }
    if let Some(listing) = listing {
        if listing.without_rowid != parsed.without_rowid || listing.strict != parsed.strict {
            return Err(mismatch("STRICT or WITHOUT ROWID differs"));
        }
    }

    // Columns.
    if parsed.columns.len() != catalog.columns.len() {
        return Err(mismatch(format!(
            "the definition has {} columns, SQLite reports {}",
            parsed.columns.len(),
            catalog.columns.len()
        )));
    }
    let find = |name: &str| -> Result<usize, String> {
        parsed
            .columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| mismatch(format!("a constraint names unknown column {name}")))
    };
    let mut primary: Option<(Option<String>, Vec<usize>)> = None;
    for (index, column) in parsed.columns.iter().enumerate() {
        if let Some(key) = &column.primary_key {
            if primary.is_some() {
                return Err(mismatch("more than one primary key"));
            }
            primary = Some((key.name.clone(), vec![index]));
        }
    }
    for constraint in &parsed.constraints {
        if let ParsedConstraint::PrimaryKey { name, columns } = constraint {
            if primary.is_some() {
                return Err(mismatch("more than one primary key"));
            }
            let members = columns
                .iter()
                .map(|column| find(column.as_str()))
                .collect::<Result<Vec<_>, _>>()?;
            primary = Some((name.clone(), members));
        }
    }
    for (index, (column, row)) in parsed.columns.iter().zip(&catalog.columns).enumerate() {
        let label = &row.name;
        if column.name != row.name {
            return Err(mismatch(format!(
                "column {} is {} in the catalog",
                column.name, row.name
            )));
        }
        if !declared_type(&column.data_type).eq_ignore_ascii_case(&row.data_type) {
            return Err(mismatch(format!("the type of {label} differs")));
        }
        let implied_not_null = parsed.without_rowid && row.pk > 0;
        if !implied_not_null && column.not_null != row.not_null {
            return Err(mismatch(format!("NOT NULL on {label} differs")));
        }
        if column.generated.is_none() && column.default != row.default {
            return Err(mismatch(format!("the default of {label} differs")));
        }
        let expected_hidden = match &column.generated {
            None => 0,
            Some(generated) if generated.stored => 3,
            Some(_) => 2,
        };
        if row.hidden != expected_hidden {
            return Err(mismatch(format!(
                "the generated expression of {label} differs"
            )));
        }
        let expected_pk = primary
            .as_ref()
            .and_then(|(_, members)| members.iter().position(|member| *member == index))
            .map_or(0, |position| position as i64 + 1);
        if row.pk != expected_pk {
            return Err(mismatch(format!(
                "primary key membership of {label} differs"
            )));
        }
    }
    let origin_of = |index: usize| column_origin(catalog.columns[index].cid);
    let rowid_alias = !parsed.without_rowid
        && primary.as_ref().is_some_and(|(_, members)| {
            members.len() == 1
                && declared_type(&parsed.columns[members[0]].data_type)
                    .eq_ignore_ascii_case("INTEGER")
        });
    if parsed.columns.iter().any(|column| {
        column
            .primary_key
            .as_ref()
            .is_some_and(|key| key.autoincrement)
    }) && !rowid_alias
    {
        return Err(mismatch("AUTOINCREMENT is not on an INTEGER PRIMARY KEY"));
    }

    // Keys: the autoindexes SQLite built must be exactly the declared keys.
    let lower = |index: usize| catalog.columns[index].name.to_ascii_lowercase();
    let mut unique_members: Vec<(Option<String>, Vec<usize>)> = Vec::new();
    for (index, column) in parsed.columns.iter().enumerate() {
        for name in &column.uniques {
            unique_members.push((name.clone(), vec![index]));
        }
    }
    for constraint in &parsed.constraints {
        if let ParsedConstraint::Unique { name, columns } = constraint {
            let members = columns
                .iter()
                .map(|column| find(column.as_str()))
                .collect::<Result<Vec<_>, _>>()?;
            unique_members.push((name.clone(), members));
        }
    }
    let mut expected_keys: BTreeSet<Vec<String>> = unique_members
        .iter()
        .map(|(_, members)| members.iter().map(|m| lower(*m)).collect())
        .collect();
    if let Some((_, members)) = primary.as_ref().filter(|_| !rowid_alias) {
        expected_keys.insert(members.iter().map(|m| lower(*m)).collect());
    }
    let mut actual_keys = BTreeSet::new();
    for index in &catalog.indexes {
        if index.origin == "c" {
            continue;
        }
        let names: Vec<String> = index
            .columns
            .iter()
            .map(|column| column.name.as_deref().unwrap_or("").to_ascii_lowercase())
            .collect();
        if index.origin == "pk" {
            let declared: Option<Vec<String>> = primary
                .as_ref()
                .filter(|_| !rowid_alias)
                .map(|(_, members)| members.iter().map(|m| lower(*m)).collect());
            if declared.as_ref() != Some(&names) {
                return Err(mismatch("the primary key index differs"));
            }
        }
        actual_keys.insert(names);
    }
    if expected_keys != actual_keys {
        return Err(mismatch("the unique and primary key indexes differ"));
    }

    // Foreign keys, compared as multisets (SQLite lists them newest first).
    let mut parsed_keys = Vec::new();
    for (index, column) in parsed.columns.iter().enumerate() {
        for reference in &column.references {
            parsed_keys.push((vec![index], reference.clone()));
        }
    }
    for constraint in &parsed.constraints {
        if let ParsedConstraint::ForeignKey { columns, reference } = constraint {
            let members = columns
                .iter()
                .map(|column| find(column.as_str()))
                .collect::<Result<Vec<_>, _>>()?;
            parsed_keys.push((members, reference.clone()));
        }
    }
    let mut expected_fks: Vec<String> = parsed_keys
        .iter()
        .map(|(members, reference)| {
            fk_signature(
                &reference.table,
                &members.iter().map(|m| lower(*m)).collect::<Vec<_>>(),
                &reference
                    .columns
                    .iter()
                    .map(|c| Some(c.to_ascii_lowercase()))
                    .collect::<Vec<_>>(),
                reference.on_update,
                reference.on_delete,
                members.len(),
            )
        })
        .collect();
    let mut actual_fks = Vec::new();
    for key in &catalog.foreign_keys {
        let (Some(on_update), Some(on_delete)) = (action(&key.on_update), action(&key.on_delete))
        else {
            return Err(mismatch("a foreign key has an unknown action"));
        };
        if !key.match_kind.eq_ignore_ascii_case("NONE") {
            return Err(mismatch("a foreign key has a MATCH clause"));
        }
        actual_fks.push(fk_signature(
            &key.table,
            &key.from
                .iter()
                .map(|c| c.to_ascii_lowercase())
                .collect::<Vec<_>>(),
            &key.to
                .iter()
                .map(|c| c.as_ref().map(|c| c.to_ascii_lowercase()))
                .collect::<Vec<_>>(),
            on_update,
            on_delete,
            key.from.len(),
        ));
    }
    expected_fks.sort();
    actual_fks.sort();
    if expected_fks != actual_fks {
        return Err(mismatch("the foreign keys differ"));
    }

    // Explicit indexes.
    let mut explicit: Vec<&CatalogIndex> = catalog
        .indexes
        .iter()
        .filter(|index| index.origin == "c")
        .collect();
    explicit.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    if explicit.len() != catalog.defined_indexes.len()
        || explicit.iter().any(|index| {
            !catalog
                .defined_indexes
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case(&index.name))
        })
    {
        return Err(mismatch("the indexes differ"));
    }
    let mut indexes = Vec::with_capacity(explicit.len());
    for (n, index) in explicit.iter().enumerate() {
        let sql = catalog
            .defined_indexes
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(&index.name))
            .map(|(_, sql)| sql.as_str());
        let Some(sql) = sql else {
            return Err(mismatch(format!("index {} has no definition", index.name)));
        };
        let parsed_index = parse_create_index(sql).map_err(|detail| {
            format!(
                "The definition of index {} could not be read: {detail}",
                index.name
            )
        })?;
        indexes.push(index_model(
            n,
            index,
            &parsed_index,
            table,
            &parsed,
            catalog,
        )?);
    }

    // The model.
    let columns: Vec<ColumnModel> = parsed
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| ColumnModel {
            origin: origin_of(index),
            name: catalog.columns[index].name.clone(),
            data_type: column.data_type.clone(),
            nullable: !column.not_null,
            default: column
                .default
                .as_deref()
                .map_or(DefaultValue::None, default_from_text),
            generated: column.generated.clone(),
            identity: if column
                .primary_key
                .as_ref()
                .is_some_and(|key| key.autoincrement)
            {
                Identity::AutoIncrement
            } else {
                Identity::None
            },
            collation: column.collation.clone(),
            comment: None,
            extra: ColumnExtra::default(),
        })
        .collect();
    let primary_key = primary.map(|(name, members)| KeyModel {
        origin: object_origin(TAG_PRIMARY_KEY, 0),
        name,
        columns: members.into_iter().map(origin_of).collect(),
    });
    let uniques = unique_members
        .into_iter()
        .enumerate()
        .map(|(n, (name, members))| KeyModel {
            origin: object_origin(TAG_UNIQUE, n),
            name,
            columns: members.into_iter().map(origin_of).collect(),
        })
        .collect();
    let foreign_keys = parsed_keys
        .into_iter()
        .enumerate()
        .map(|(n, (members, reference))| ForeignKeyModel {
            origin: object_origin(TAG_FOREIGN_KEY, n),
            name: reference.name,
            columns: members.into_iter().map(origin_of).collect(),
            referenced_schema: None,
            referenced_table: reference.table,
            referenced_columns: reference.columns,
            on_update: reference.on_update,
            on_delete: reference.on_delete,
            deferrable: reference.deferrable,
        })
        .collect();
    let mut checks = Vec::new();
    for column in &parsed.columns {
        for (name, expression) in &column.checks {
            checks.push((name.clone(), expression.clone()));
        }
    }
    for constraint in &parsed.constraints {
        if let ParsedConstraint::Check { name, expression } = constraint {
            checks.push((name.clone(), expression.clone()));
        }
    }
    let checks = checks
        .into_iter()
        .enumerate()
        .map(|(n, (name, expression))| CheckModel {
            origin: object_origin(TAG_CHECK, n),
            name,
            expression: crate::schema_edit::SqlFragment::new(expression),
            enforced: true,
        })
        .collect();
    Ok(TableModel {
        schema: schema.to_owned(),
        name: table.to_owned(),
        comment: None,
        columns,
        primary_key,
        uniques,
        foreign_keys,
        checks,
        indexes,
        options: TableOptions {
            strict: parsed.strict,
            without_rowid: parsed.without_rowid,
            ..TableOptions::default()
        },
    })
}

fn fk_signature(
    table: &str,
    from: &[String],
    to: &[Option<String>],
    on_update: ReferentialAction,
    on_delete: ReferentialAction,
    width: usize,
) -> String {
    // An omitted referenced column list reads as NULL per column.
    let to: Vec<Option<String>> = if to.is_empty() {
        vec![None; width]
    } else {
        to.to_vec()
    };
    format!(
        "{}|{from:?}|{to:?}|{}|{}",
        table.to_ascii_lowercase(),
        on_update.sql(),
        on_delete.sql()
    )
}

fn index_model(
    n: usize,
    index: &CatalogIndex,
    parsed_index: &ParsedIndex,
    table: &str,
    parsed: &ParsedTable,
    catalog: &Catalog,
) -> Result<IndexModel, String> {
    let label = &index.name;
    if !parsed_index.table.eq_ignore_ascii_case(table)
        || !parsed_index.name.eq_ignore_ascii_case(&index.name)
        || parsed_index.unique != index.unique
        || parsed_index.predicate.is_some() != index.partial
        || parsed_index.members.len() != index.columns.len()
    {
        return Err(mismatch(format!("index {label} differs")));
    }
    let mut members = Vec::with_capacity(parsed_index.members.len());
    for (member, row) in parsed_index.members.iter().zip(&index.columns) {
        let target = match &member.target {
            MemberTarget::Name(name) => {
                let Some(position) = parsed
                    .columns
                    .iter()
                    .position(|column| column.name.eq_ignore_ascii_case(name))
                else {
                    return Err(mismatch(format!(
                        "index {label} names unknown column {name}"
                    )));
                };
                if catalog.columns[position].cid != row.cid {
                    return Err(mismatch(format!("index {label} differs")));
                }
                IndexTarget::Column(column_origin(catalog.columns[position].cid))
            }
            MemberTarget::Expression(expression) => {
                if row.cid != -2 {
                    return Err(mismatch(format!("index {label} differs")));
                }
                IndexTarget::Expression(crate::schema_edit::SqlFragment::new(expression.clone()))
            }
        };
        if member.descending != row.descending {
            return Err(mismatch(format!("index {label} ordering differs")));
        }
        if let Some(collation) = &member.collation {
            if !row
                .collation
                .as_deref()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(collation))
            {
                return Err(mismatch(format!("index {label} collation differs")));
            }
        }
        members.push(IndexMember {
            target,
            descending: member.descending,
            prefix_length: None,
            collation: member.collation.clone(),
        });
    }
    Ok(IndexModel {
        origin: object_origin(TAG_INDEX, n),
        name: index.name.clone(),
        unique: index.unique,
        method: None,
        members,
        include: Vec::new(),
        predicate: parsed_index
            .predicate
            .clone()
            .map(crate::schema_edit::SqlFragment::new),
    })
}

/// What a read-only baseline shows: columns, primary key and foreign keys
/// straight from the PRAGMAs.
fn catalog_model(
    schema: &str,
    table: &str,
    catalog: &Catalog,
    listing: Option<&Listing>,
) -> TableModel {
    let columns: Vec<ColumnModel> = catalog
        .columns
        .iter()
        .filter(|row| row.hidden != 1)
        .map(|row| {
            let mut column = ColumnModel::new(
                column_origin(row.cid),
                row.name.clone(),
                row.data_type.clone(),
            );
            column.nullable = !row.not_null;
            column.default = row
                .default
                .as_deref()
                .map_or(DefaultValue::None, default_from_text);
            column
        })
        .collect();
    let mut key: Vec<&CatalogColumn> = catalog.columns.iter().filter(|row| row.pk > 0).collect();
    key.sort_by_key(|row| row.pk);
    let primary_key = (!key.is_empty()).then(|| KeyModel {
        origin: object_origin(TAG_PRIMARY_KEY, 0),
        name: None,
        columns: key.iter().map(|row| column_origin(row.cid)).collect(),
    });
    let by_name = |name: &str| -> Option<Origin> {
        catalog
            .columns
            .iter()
            .find(|row| row.name.eq_ignore_ascii_case(name))
            .map(|row| column_origin(row.cid))
    };
    let foreign_keys = catalog
        .foreign_keys
        .iter()
        .enumerate()
        .map(|(n, key)| ForeignKeyModel {
            origin: object_origin(TAG_FOREIGN_KEY, n),
            name: None,
            columns: key.from.iter().filter_map(|name| by_name(name)).collect(),
            referenced_schema: None,
            referenced_table: key.table.clone(),
            referenced_columns: key.to.iter().flatten().cloned().collect(),
            on_update: action(&key.on_update).unwrap_or_default(),
            on_delete: action(&key.on_delete).unwrap_or_default(),
            deferrable: Default::default(),
        })
        .collect();
    TableModel {
        schema: schema.to_owned(),
        name: table.to_owned(),
        comment: None,
        columns,
        primary_key,
        uniques: Vec::new(),
        foreign_keys,
        checks: Vec::new(),
        indexes: Vec::new(),
        options: TableOptions {
            strict: listing.is_some_and(|listing| listing.strict),
            without_rowid: listing.is_some_and(|listing| listing.without_rowid),
            ..TableOptions::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        cid: i64,
        name: &str,
        data_type: &str,
        not_null: bool,
        default: Option<&str>,
        pk: i64,
    ) -> CatalogColumn {
        CatalogColumn {
            cid,
            name: name.into(),
            data_type: data_type.into(),
            not_null,
            default: default.map(str::to_owned),
            pk,
            hidden: 0,
        }
    }

    fn catalog(columns: Vec<CatalogColumn>) -> Catalog {
        Catalog {
            columns,
            foreign_keys: Vec::new(),
            indexes: Vec::new(),
            defined_indexes: Vec::new(),
        }
    }

    const SQL: &str = "CREATE TABLE t (a INTEGER PRIMARY KEY, b TEXT NOT NULL DEFAULT 'x')";

    fn agreeing() -> Vec<CatalogColumn> {
        vec![
            row(0, "a", "INTEGER", false, None, 1),
            row(1, "b", "TEXT", true, Some("'x'"), 0),
        ]
    }

    #[test]
    fn an_agreeing_catalog_builds_the_model() {
        let model = build_model("main", "t", SQL, &catalog(agreeing()), None).unwrap();
        assert_eq!(model.columns[0].origin, column_origin(0));
        assert_eq!(model.columns[1].default, DefaultValue::Literal("x".into()));
        assert_eq!(
            model.primary_key.unwrap().origin,
            object_origin(TAG_PRIMARY_KEY, 0)
        );
    }

    #[test]
    fn any_disagreement_is_a_read_only_reason() {
        type Mutation = Box<dyn Fn(&mut Vec<CatalogColumn>)>;
        fn mutation(change: impl Fn(&mut Vec<CatalogColumn>) + 'static) -> Mutation {
            Box::new(change)
        }
        let mutations: Vec<Mutation> = vec![
            mutation(|c: &mut Vec<CatalogColumn>| c[1].data_type = "BLOB".into()),
            mutation(|c: &mut Vec<CatalogColumn>| c[1].not_null = false),
            mutation(|c: &mut Vec<CatalogColumn>| c[1].default = Some("'y'".into())),
            mutation(|c: &mut Vec<CatalogColumn>| c[0].pk = 0),
            mutation(|c: &mut Vec<CatalogColumn>| c[1].hidden = 2),
            mutation(|c: &mut Vec<CatalogColumn>| c[1].name = "B".into()),
            mutation(|c: &mut Vec<CatalogColumn>| c.push(row(2, "c", "", false, None, 0))),
        ];
        for (index, mutate) in mutations.iter().enumerate() {
            let mut columns = agreeing();
            mutate(&mut columns);
            let error = build_model("main", "t", SQL, &catalog(columns), None).unwrap_err();
            assert!(error.contains("disagrees"), "case {index}: {error}");
        }
        let strict = Listing {
            kind: "table".into(),
            without_rowid: false,
            strict: true,
        };
        assert!(build_model("main", "t", SQL, &catalog(agreeing()), Some(&strict)).is_err());
    }

    #[test]
    fn a_unique_constraint_needs_its_autoindex() {
        let sql = "CREATE TABLE t (a INTEGER PRIMARY KEY, b TEXT UNIQUE)";
        let columns = vec![
            row(0, "a", "INTEGER", false, None, 1),
            row(1, "b", "TEXT", false, None, 0),
        ];
        assert!(build_model("main", "t", sql, &catalog(columns.clone()), None).is_err());
        let mut with_index = catalog(columns);
        with_index.indexes.push(CatalogIndex {
            name: "sqlite_autoindex_t_1".into(),
            unique: true,
            origin: "u".into(),
            partial: false,
            columns: vec![CatalogIndexColumn {
                cid: 1,
                name: Some("b".into()),
                descending: false,
                collation: Some("BINARY".into()),
            }],
        });
        let model = build_model("main", "t", sql, &with_index, None).unwrap();
        assert_eq!(model.uniques.len(), 1);
        assert_eq!(model.uniques[0].columns, vec![column_origin(1)]);
    }

    #[test]
    fn unparseable_definitions_are_read_only_reasons() {
        let error = build_model(
            "main",
            "t",
            "CREATE TABLE t (a TEXT UNIQUE ON CONFLICT REPLACE)",
            &catalog(vec![row(0, "a", "TEXT", false, None, 0)]),
            None,
        )
        .unwrap_err();
        assert!(error.contains("ON CONFLICT"), "{error}");
    }
}
