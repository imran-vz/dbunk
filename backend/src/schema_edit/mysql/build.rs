//! Catalog rows plus the parsed definition into a [`SchemaBaseline`]. Pure.
//!
//! The catalog is the source for names, types, nullability, charsets,
//! collations, comments, keys, foreign keys and table options; the
//! definition (`SHOW CREATE TABLE`) is the source for expression text
//! (defaults, `ON UPDATE`, generated columns, checks, functional key parts),
//! because that is the rendering the server accepts back unchanged. Every
//! field both describe is compared; any disagreement, and anything the model
//! cannot represent, makes the baseline read-only with a reason.
//!
//! Identity: a column's `BaselineId` is its `ORDINAL_POSITION`, stable across
//! captures of an unchanged table. Other objects use `kind << 48 | n`, with
//! `n` the 1-based position in name order within the kind (primary key 1,
//! unique 2, foreign key 3, check 4, index 5), so no two kinds collide and a
//! column (at most 4096) never collides with a constraint.
use super::catalog::{Catalog, ColumnRow, IndexRow};
use super::definition::{self, DefColumn, DefDefault, DefKey, Definition, KeyKind};
use super::sql::{loose_expression, referential_action, same_action};
use super::version::{self, Version};
use crate::schema_edit::{
    normalize_type, BaselineId, CheckModel, ColumnExtra, ColumnModel, DefaultValue, Deferrable,
    Dependent, DriftToken, Editability, ForeignKeyModel, Generated, Identity, InboundReference,
    IndexMember, IndexModel, IndexTarget, KeyModel, Origin, ReferentialAction, SchemaBaseline,
    SchemaEngine,
    SqlFragment, TableModel, TableOptions, MAX_SCHEMA_EDIT_COLUMNS, MAX_SCHEMA_EDIT_MEMBERS,
    MAX_SCHEMA_EDIT_OBJECTS,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub(super) const PRIMARY_TAG: u64 = 1;
pub(super) const UNIQUE_TAG: u64 = 2;
pub(super) const FOREIGN_KEY_TAG: u64 = 3;
pub(super) const CHECK_TAG: u64 = 4;
pub(super) const INDEX_TAG: u64 = 5;

pub(super) fn object_id(tag: u64, n: usize) -> Origin {
    Origin::Existing(BaselineId((tag << 48) | n as u64))
}

/// `sql_mode` flags that change how the server renders definitions or reads
/// literals; a capture under them is not trusted.
const UNSAFE_SQL_MODES: [&str; 13] = [
    "ANSI",
    "ANSI_QUOTES",
    "NO_BACKSLASH_ESCAPES",
    "NO_FIELD_OPTIONS",
    "NO_KEY_OPTIONS",
    "NO_TABLE_OPTIONS",
    "ORACLE",
    "POSTGRESQL",
    "MSSQL",
    "DB2",
    "MAXDB",
    "MYSQL323",
    "MYSQL40",
];

/// `mode` without the unsafe flags, for the apply connection.
pub(super) fn safe_sql_mode(mode: &str) -> String {
    mode.split(',')
        .map(str::trim)
        .filter(|flag| {
            !flag.is_empty() && !UNSAFE_SQL_MODES.contains(&flag.to_ascii_uppercase().as_str())
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn unsafe_sql_modes(mode: &str) -> Vec<String> {
    mode.split(',')
        .map(|flag| flag.trim().to_ascii_uppercase())
        .filter(|flag| UNSAFE_SQL_MODES.contains(&flag.as_str()))
        .collect()
}

/// The drift token: a hash of the server version, the captured model and
/// the read-only reasons. Row counts and the `AUTO_INCREMENT` counter are not
/// part of the model, so ordinary writes never drift a baseline.
pub(super) fn drift(server_version: &str, table: &TableModel, reasons: &[String]) -> DriftToken {
    let encoded = serde_json::to_vec(&(server_version, table, reasons)).unwrap_or_default();
    let digest = Sha256::digest(&encoded);
    DriftToken(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

struct Problems(Vec<String>);

impl Problems {
    fn push(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if !self.0.contains(&reason) {
            self.0.push(reason);
        }
    }
    fn disagree(&mut self, what: &str) {
        self.push(format!(
            "The catalog and SHOW CREATE TABLE disagree on {what}"
        ));
    }
}

/// A catalog default, classified per flavor.
#[derive(Debug, PartialEq, Eq)]
enum CatalogDefault {
    None,
    Literal(String),
    Expression(String),
}

fn catalog_default(row: &ColumnRow, version: Option<&Version>) -> CatalogDefault {
    let Some(value) = &row.default else {
        return CatalogDefault::None;
    };
    let mariadb = version.is_some_and(Version::is_mariadb);
    if mariadb {
        if value == "NULL" {
            return CatalogDefault::None;
        }
        if let Some(text) = definition::unquote(value) {
            return CatalogDefault::Literal(text);
        }
        let numeric = value.trim_start_matches('-');
        if !numeric.is_empty()
            && numeric
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
            && numeric.starts_with(|c: char| c.is_ascii_digit())
        {
            return CatalogDefault::Literal(value.clone());
        }
        return CatalogDefault::Expression(value.clone());
    }
    if extra_has(&row.extra, "DEFAULT_GENERATED") {
        CatalogDefault::Expression(value.clone())
    } else {
        CatalogDefault::Literal(value.clone())
    }
}

fn extra_has(extra: &str, word: &str) -> bool {
    extra
        .split_whitespace()
        .any(|part| part.eq_ignore_ascii_case(word))
}

fn extra_lower(extra: &str) -> String {
    extra.to_ascii_lowercase()
}

pub(super) fn default_from_definition(default: &Option<DefDefault>) -> DefaultValue {
    match default {
        None | Some(DefDefault::Null) => DefaultValue::None,
        Some(DefDefault::String(value)) => DefaultValue::Literal(value.clone()),
        Some(DefDefault::Number(value)) => DefaultValue::Literal(value.clone()),
        Some(DefDefault::Bits(value)) | Some(DefDefault::Expression(value)) => {
            DefaultValue::Expression(SqlFragment::new(value.clone()))
        }
    }
}

fn defaults_agree(definition: &Option<DefDefault>, catalog: &CatalogDefault) -> bool {
    match (definition, catalog) {
        (None | Some(DefDefault::Null), CatalogDefault::None) => true,
        (Some(DefDefault::String(a)), CatalogDefault::Literal(b)) => a == b,
        (Some(DefDefault::Number(a)), CatalogDefault::Literal(b)) => a == b,
        (Some(DefDefault::Bits(_)), CatalogDefault::Literal(_) | CatalogDefault::Expression(_)) => {
            true
        }
        (Some(DefDefault::Expression(_)), CatalogDefault::Expression(_)) => true,
        // MySQL before 8.0.13 reports CURRENT_TIMESTAMP without
        // DEFAULT_GENERATED.
        (Some(DefDefault::Expression(a)), CatalogDefault::Literal(b)) => {
            loose_expression(a) == loose_expression(b)
        }
        _ => false,
    }
}

/// Builds the baseline. `catalog.create_table` empty means the object is not
/// a base table.
pub(super) fn baseline(
    server_version: &str,
    database: &str,
    table: &str,
    catalog: &Catalog,
) -> SchemaBaseline {
    let (server, version, gate) = version::facts(server_version);
    let mut problems = Problems(Vec::new());
    if let Some(reason) = gate {
        problems.push(reason);
    }
    for flag in unsafe_sql_modes(&catalog.sql_mode) {
        problems.push(format!(
            "The session sql_mode includes {flag}, which changes how table definitions are rendered"
        ));
    }
    if !catalog.table.table_type.eq_ignore_ascii_case("BASE TABLE") {
        problems.push(format!(
            "Only base tables can be edited; this is a {}",
            catalog.table.table_type.to_ascii_lowercase()
        ));
    }
    if catalog.truncated || catalog.columns.len() > MAX_SCHEMA_EDIT_COLUMNS {
        problems.push("The table has more columns, keys or indexes than the editor supports");
    }
    if catalog
        .table
        .create_options
        .to_ascii_lowercase()
        .contains("partitioned")
    {
        problems.push("Partitioned tables are not supported");
    }

    let parsed = if catalog.create_table.is_empty() {
        None
    } else {
        match definition::parse(&catalog.create_table) {
            Ok(parsed) => {
                for reason in &parsed.unsupported {
                    problems.push(reason.clone());
                }
                for reason in definition::cross_check(&catalog.create_table, &parsed) {
                    problems.push(reason);
                }
                if parsed.name != table {
                    problems.disagree("the table name");
                }
                Some(parsed)
            }
            Err(error) => {
                problems.push(format!("The table definition could not be read: {error}"));
                None
            }
        }
    };
    let parsed = parsed.as_ref();
    let version = version.as_ref();

    // Columns.
    if let Some(parsed) = parsed {
        let names: Vec<&str> = parsed.columns.iter().map(|c| c.name.as_str()).collect();
        let catalog_names: Vec<&str> = catalog.columns.iter().map(|c| c.name.as_str()).collect();
        if names != catalog_names {
            problems.disagree("the column list");
        }
    }
    let mut columns = Vec::with_capacity(catalog.columns.len());
    let mut origins: HashMap<String, Origin> = HashMap::new();
    for (index, row) in catalog.columns.iter().enumerate() {
        let ordinal = if row.ordinal == 0 {
            index as u64 + 1
        } else {
            u64::from(row.ordinal)
        };
        let origin = Origin::Existing(BaselineId(ordinal));
        origins.insert(row.name.to_lowercase(), origin);
        let defined = parsed.and_then(|p| p.columns.iter().find(|c| c.name == row.name));
        if let Some(defined) = defined {
            column_problems(&mut problems, row, defined, version);
        }
        columns.push(column_model(origin, row, defined, version));
    }
    let origin_of = |name: &str| origins.get(&name.to_lowercase()).copied();

    // Indexes, grouped by name in catalog order (name, sequence).
    let mut groups: Vec<(&str, Vec<&IndexRow>)> = Vec::new();
    for row in &catalog.indexes {
        match groups.last_mut() {
            Some((name, rows)) if *name == row.index => rows.push(row),
            _ => groups.push((row.index.as_str(), vec![row])),
        }
    }
    if groups.len() > MAX_SCHEMA_EDIT_OBJECTS
        || groups.iter().any(|(_, rows)| rows.len() > MAX_SCHEMA_EDIT_MEMBERS)
    {
        problems.push("The table has more indexes than the editor supports");
    }
    let mut primary_key = None;
    let mut uniques = Vec::new();
    let mut indexes = Vec::new();
    let mut matched_keys = 0usize;
    for (name, rows) in &groups {
        let defined: Option<&DefKey> = parsed.and_then(|p| {
            if *name == "PRIMARY" {
                p.primary_key.as_ref()
            } else {
                p.keys.iter().find(|k| k.name.as_deref() == Some(*name))
            }
        });
        if parsed.is_some() {
            match defined {
                Some(defined) => {
                    matched_keys += usize::from(*name != "PRIMARY");
                    key_problems(&mut problems, name, rows, defined);
                }
                None => problems.disagree(&format!("index {name}")),
            }
        }
        if rows.iter().any(|row| !row.comment.is_empty()) {
            problems.push(format!("Index {name} has a comment, which is not supported"));
        }
        if rows.iter().any(|row| !row.visible) {
            problems.push(format!("Index {name} is invisible, which is not supported"));
        }
        let mut members = Vec::with_capacity(rows.len());
        for (position, row) in rows.iter().enumerate() {
            let target = match &row.column {
                Some(column) => match origin_of(column) {
                    Some(origin) => IndexTarget::Column(origin),
                    None => {
                        problems.disagree(&format!("the columns of index {name}"));
                        continue;
                    }
                },
                None => {
                    let text = defined
                        .and_then(|k| k.parts.get(position))
                        .and_then(|part| part.expression.clone())
                        .or_else(|| row.expression.clone())
                        .unwrap_or_default();
                    IndexTarget::Expression(SqlFragment::new(text))
                }
            };
            members.push(IndexMember {
                target,
                descending: row.collation.as_deref() == Some("D"),
                prefix_length: row.sub_part,
                collation: None,
            });
        }
        if *name == "PRIMARY" {
            let plain: Option<Vec<Origin>> = members
                .iter()
                .map(|member| match member.target {
                    IndexTarget::Column(origin)
                        if member.prefix_length.is_none() && !member.descending =>
                    {
                        Some(origin)
                    }
                    _ => None,
                })
                .collect();
            match plain {
                Some(columns) => {
                    primary_key = Some(KeyModel {
                        origin: object_id(PRIMARY_TAG, 1),
                        name: None,
                        columns,
                    })
                }
                None => problems
                    .push("A primary key with prefix or descending parts is not supported"),
            }
            continue;
        }
        let unique = !rows[0].non_unique;
        let method = match rows[0].index_type.to_ascii_uppercase().as_str() {
            "BTREE" | "" => None,
            other => Some(other.to_owned()),
        };
        let plain: Option<Vec<Origin>> = members
            .iter()
            .map(|member| match member.target {
                IndexTarget::Column(origin)
                    if member.prefix_length.is_none() && !member.descending =>
                {
                    Some(origin)
                }
                _ => None,
            })
            .collect();
        match plain {
            Some(columns) if unique && method.is_none() => uniques.push(KeyModel {
                origin: object_id(UNIQUE_TAG, uniques.len() + 1),
                name: Some((*name).to_owned()),
                columns,
            }),
            _ => indexes.push(IndexModel {
                origin: object_id(INDEX_TAG, indexes.len() + 1),
                name: (*name).to_owned(),
                unique,
                method,
                members,
                include: Vec::new(),
                predicate: None,
            }),
        }
    }
    if let Some(parsed) = parsed {
        if parsed.keys.len() != matched_keys
            || (parsed.primary_key.is_some() != primary_key.is_some()
                && !problems.0.iter().any(|p| p.contains("primary key")))
        {
            problems.disagree("the keys and indexes");
        }
    }

    // Foreign keys, grouped by name.
    let mut foreign_groups: Vec<(&str, Vec<&super::catalog::KeyColumnRow>)> = Vec::new();
    for row in &catalog.key_columns {
        match foreign_groups.last_mut() {
            Some((name, rows)) if *name == row.constraint => rows.push(row),
            _ => foreign_groups.push((row.constraint.as_str(), vec![row])),
        }
    }
    let mut foreign_keys = Vec::new();
    for (name, rows) in &foreign_groups {
        let rules = catalog.referential.iter().find(|r| r.constraint == *name);
        let action = |rule: Option<&str>| rule.and_then(referential_action);
        let on_update = action(rules.map(|r| r.update_rule.as_str()));
        let on_delete = action(rules.map(|r| r.delete_rule.as_str()));
        if on_update.is_none() || on_delete.is_none() {
            problems.push(format!(
                "Foreign key {name} has referential actions that are not supported"
            ));
        }
        let mut columns = Vec::new();
        for row in rows {
            match origin_of(&row.column) {
                Some(origin) => columns.push(origin),
                None => problems.disagree(&format!("the columns of foreign key {name}")),
            }
        }
        let referenced_schema = rows[0]
            .referenced_schema
            .clone()
            .filter(|schema| schema != database);
        let key = ForeignKeyModel {
            origin: object_id(FOREIGN_KEY_TAG, foreign_keys.len() + 1),
            name: Some((*name).to_owned()),
            columns,
            referenced_schema,
            referenced_table: rows[0].referenced_table.clone().unwrap_or_default(),
            referenced_columns: rows
                .iter()
                .map(|row| row.referenced_column.clone().unwrap_or_default())
                .collect(),
            on_update: on_update.unwrap_or_default(),
            on_delete: on_delete.unwrap_or_default(),
            deferrable: Deferrable::Never,
        };
        if let Some(parsed) = parsed {
            foreign_key_problems(&mut problems, parsed, rows, &key, database);
        }
        foreign_keys.push(key);
    }
    if let Some(parsed) = parsed {
        if parsed.foreign_keys.len() != foreign_keys.len() {
            problems.disagree("the foreign keys");
        }
    }

    // Checks.
    let mut checks = Vec::new();
    for row in &catalog.checks {
        if row.level.eq_ignore_ascii_case("Column") {
            problems.push(format!(
                "Column-level CHECK constraint {} is not supported",
                row.name
            ));
            continue;
        }
        let defined = parsed.and_then(|p| {
            p.checks
                .iter()
                .find(|check| check.name.as_deref() == Some(row.name.as_str()))
        });
        let enforced_in_catalog = catalog
            .constraints
            .iter()
            .find(|c| c.name == row.name && c.kind.eq_ignore_ascii_case("CHECK"))
            .is_none_or(|c| c.enforced);
        match defined {
            Some(defined) => {
                if loose_expression(&defined.expression) != loose_expression(&row.clause) {
                    problems.disagree(&format!("check {}", row.name));
                }
                if defined.enforced != enforced_in_catalog {
                    problems.disagree(&format!("whether check {} is enforced", row.name));
                }
            }
            None if parsed.is_some() => problems.disagree(&format!("check {}", row.name)),
            None => {}
        }
        checks.push(CheckModel {
            origin: object_id(CHECK_TAG, checks.len() + 1),
            name: Some(row.name.clone()),
            expression: SqlFragment::new(
                defined
                    .map(|d| d.expression.clone())
                    .unwrap_or_else(|| row.clause.clone()),
            ),
            enforced: defined.map_or(enforced_in_catalog, |d| d.enforced),
        });
    }
    if let Some(parsed) = parsed {
        if parsed.checks.len() != checks.len() {
            problems.disagree("the check constraints");
        }
    }

    // Table options.
    let options = TableOptions {
        strict: false,
        without_rowid: false,
        engine: catalog.table.engine.clone(),
        charset: catalog.table.charset.clone(),
        collation: catalog.table.collation.clone(),
        row_format: parsed
            .and_then(|p| p.option("ROW_FORMAT"))
            .map(str::to_ascii_uppercase),
    };
    if let Some(parsed) = parsed {
        option_problems(&mut problems, parsed, catalog);
    }

    let table_model = TableModel {
        schema: database.to_owned(),
        name: table.to_owned(),
        comment: Some(catalog.table.comment.clone()).filter(|c| !c.is_empty()),
        columns,
        primary_key,
        uniques,
        foreign_keys,
        checks,
        indexes,
        options,
    };
    let reasons = problems.0;
    let drift = drift(server_version, &table_model, &reasons);
    let editable = if reasons.is_empty() {
        Editability::Editable
    } else {
        Editability::ReadOnly { reasons }
    };
    SchemaBaseline {
        engine: SchemaEngine::MySql,
        server,
        table: table_model,
        drift,
        editable,
        inbound: inbound(catalog),
        dependents: dependents(catalog, database),
        estimated_rows: catalog.table.rows,
    }
}

fn column_model(
    origin: Origin,
    row: &ColumnRow,
    defined: Option<&DefColumn>,
    version: Option<&Version>,
) -> ColumnModel {
    let extra = extra_lower(&row.extra);
    let generated = match defined {
        Some(defined) => defined.generated.as_ref().map(|(expression, stored)| Generated {
            expression: SqlFragment::new(expression.clone()),
            stored: *stored,
        }),
        None => (extra.contains("virtual generated") || extra.contains("stored generated")).then(
            || Generated {
                expression: SqlFragment::new(row.generation.clone().unwrap_or_default()),
                stored: extra.contains("stored generated"),
            },
        ),
    };
    let default = match defined {
        Some(defined) => default_from_definition(&defined.default),
        None => match catalog_default(row, version) {
            CatalogDefault::None => DefaultValue::None,
            CatalogDefault::Literal(value) => DefaultValue::Literal(value),
            CatalogDefault::Expression(value) => DefaultValue::Expression(SqlFragment::new(value)),
        },
    };
    let on_update = match defined {
        Some(defined) => defined.on_update.clone(),
        None => extra
            .find("on update ")
            .map(|at| row.extra[at + "on update ".len()..].trim().to_owned()),
    };
    ColumnModel {
        origin,
        name: row.name.clone(),
        data_type: row.column_type.clone(),
        nullable: row.nullable,
        default,
        generated,
        identity: if extra_has(&row.extra, "auto_increment") {
            Identity::AutoIncrement
        } else {
            Identity::None
        },
        collation: row.collation.clone(),
        comment: Some(row.comment.clone()).filter(|c| !c.is_empty()),
        extra: ColumnExtra {
            charset: row.charset.clone(),
            on_update: on_update.map(SqlFragment::new),
            invisible: extra_has(&row.extra, "INVISIBLE"),
        },
    }
}

fn column_problems(
    problems: &mut Problems,
    row: &ColumnRow,
    defined: &DefColumn,
    version: Option<&Version>,
) {
    let name = &row.name;
    let mut disagree = |field: &str| problems.disagree(&format!("the {field} of column {name}"));
    if normalize_type(&defined.data_type) != normalize_type(&row.column_type) {
        disagree("type");
    }
    if row.nullable == (defined.not_null == Some(true)) {
        disagree("nullability");
    }
    if let Some(charset) = &defined.charset {
        if !row
            .charset
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case(charset))
        {
            disagree("character set");
        }
    }
    if let Some(collation) = &defined.collation {
        if !row
            .collation
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case(collation))
        {
            disagree("collation");
        }
    }
    if !defaults_agree(&defined.default, &catalog_default(row, version)) {
        disagree("default");
    }
    let extra = extra_lower(&row.extra);
    match (&defined.on_update, extra.find("on update ")) {
        (None, None) => {}
        (Some(expression), Some(at)) => {
            if !loose_expression(&extra[at..]).contains(&loose_expression(expression)) {
                disagree("ON UPDATE");
            }
        }
        _ => disagree("ON UPDATE"),
    }
    if extra_has(&row.extra, "auto_increment") != defined.auto_increment {
        disagree("AUTO_INCREMENT");
    }
    if extra_has(&row.extra, "INVISIBLE") != defined.invisible {
        disagree("visibility");
    }
    let catalog_generated = if extra.contains("stored generated") {
        Some(true)
    } else if extra.contains("virtual generated") {
        Some(false)
    } else {
        None
    };
    match (&defined.generated, catalog_generated) {
        (None, None) => {}
        (Some((expression, stored)), Some(catalog_stored)) => {
            if *stored != catalog_stored {
                disagree("generation");
            }
            if let Some(catalog_expression) = row.generation.as_deref().filter(|e| !e.is_empty()) {
                if loose_expression(expression) != loose_expression(catalog_expression) {
                    disagree("generated expression");
                }
            }
        }
        _ => disagree("generation"),
    }
    if defined.comment.as_deref().unwrap_or_default() != row.comment {
        disagree("comment");
    }
}

fn key_problems(problems: &mut Problems, name: &str, rows: &[&IndexRow], defined: &DefKey) {
    let index_type = rows[0].index_type.to_ascii_uppercase();
    let kind = if name == "PRIMARY" {
        KeyKind::Primary
    } else if index_type == "FULLTEXT" {
        KeyKind::Fulltext
    } else if index_type == "SPATIAL" {
        KeyKind::Spatial
    } else if !rows[0].non_unique {
        KeyKind::Unique
    } else {
        KeyKind::Index
    };
    let mut disagree = |what: &str| problems.disagree(&format!("the {what} of index {name}"));
    if kind != defined.kind {
        disagree("kind");
    }
    if let Some(using) = &defined.using {
        if !using.eq_ignore_ascii_case(&index_type) {
            disagree("method");
        }
    }
    if rows.len() != defined.parts.len() {
        disagree("parts");
        return;
    }
    for (row, part) in rows.iter().zip(&defined.parts) {
        let same_column = match (&row.column, &part.column) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            (None, None) => part.expression.is_some(),
            _ => false,
        };
        // Full-text and spatial parts report no prefix in either source.
        let same_prefix = row.sub_part == part.prefix
            || matches!(kind, KeyKind::Fulltext | KeyKind::Spatial);
        if !same_column || !same_prefix || (row.collation.as_deref() == Some("D")) != part.descending
        {
            disagree("parts");
            return;
        }
    }
}

fn foreign_key_problems(
    problems: &mut Problems,
    parsed: &Definition,
    rows: &[&super::catalog::KeyColumnRow],
    key: &ForeignKeyModel,
    database: &str,
) {
    let name = key.name.clone().unwrap_or_default();
    let Some(defined) = parsed
        .foreign_keys
        .iter()
        .find(|k| k.name.as_deref() == Some(name.as_str()))
    else {
        problems.disagree(&format!("foreign key {name}"));
        return;
    };
    let catalog_columns: Vec<&str> = rows.iter().map(|row| row.column.as_str()).collect();
    let defined_columns: Vec<&str> = defined.columns.iter().map(String::as_str).collect();
    let catalog_schema = rows[0].referenced_schema.as_deref().unwrap_or(database);
    let same_schema = match &defined.referenced_schema {
        Some(schema) => schema == catalog_schema,
        None => catalog_schema == database,
    };
    let action_agrees = |defined: &Option<String>, catalog: ReferentialAction| match defined {
        None => matches!(
            catalog,
            ReferentialAction::NoAction | ReferentialAction::Restrict
        ),
        Some(text) => referential_action(text).is_some_and(|action| same_action(action, catalog)),
    };
    if catalog_columns != defined_columns
        || !same_schema
        || defined.referenced_table != key.referenced_table
        || defined.referenced_columns != key.referenced_columns
        || !action_agrees(&defined.on_delete, key.on_delete)
        || !action_agrees(&defined.on_update, key.on_update)
    {
        problems.disagree(&format!("foreign key {name}"));
    }
}

fn option_problems(problems: &mut Problems, parsed: &Definition, catalog: &Catalog) {
    let same = |defined: Option<&str>, catalog: Option<&str>| match (defined, catalog) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, _) => true,
        (Some(_), None) => false,
    };
    if !same(parsed.option("ENGINE"), catalog.table.engine.as_deref()) {
        problems.disagree("the storage engine");
    }
    if !same(parsed.option("CHARSET"), catalog.table.charset.as_deref()) {
        problems.disagree("the table character set");
    }
    if !same(parsed.option("COLLATE"), catalog.table.collation.as_deref()) {
        problems.disagree("the table collation");
    }
    if parsed.option("COMMENT").unwrap_or_default() != catalog.table.comment {
        problems.disagree("the table comment");
    }
}

fn inbound(catalog: &Catalog) -> Vec<InboundReference> {
    let mut references: Vec<InboundReference> = Vec::new();
    for row in &catalog.inbound {
        match references.last_mut() {
            Some(last)
                if last.schema == row.schema
                    && last.table == row.table
                    && last.name == row.constraint =>
            {
                last.columns.push(row.column.clone());
            }
            _ => references.push(InboundReference {
                schema: row.schema.clone(),
                table: row.table.clone(),
                name: row.constraint.clone(),
                columns: vec![row.column.clone()],
            }),
        }
    }
    references
}

fn dependents(catalog: &Catalog, database: &str) -> Vec<Dependent> {
    catalog
        .triggers
        .iter()
        .map(|name| Dependent {
            kind: "trigger".into(),
            schema: database.to_owned(),
            name: name.clone(),
            definition: None,
        })
        .chain(catalog.views.iter().map(|(schema, name)| Dependent {
            kind: "view".into(),
            schema: schema.clone(),
            name: name.clone(),
            definition: None,
        }))
        .collect()
}
