//! Server-wide ClickHouse object catalog (Plan 031 step 4).
//!
//! ClickHouse has no schemas: the server holds databases (`system.databases`,
//! already filtered to what the user may see) and each holds tables (with
//! their engine), views, materialized views (with their target table) and
//! dictionaries (`system.dictionaries`). Every list is bounded; a catalog that
//! hit a bound says so instead of silently looking complete.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;

use super::bounded::{self, ClickHouseError, ClickHouseRows, Limits};
use crate::StoredConnection;

/// Most objects (tables, views, dictionaries) read in one catalog.
pub(crate) const MAX_OBJECTS: usize = 20_000;
pub(crate) const MAX_DATABASES: usize = 5_000;
const CATALOG_BYTES: usize = 32 * 1024 * 1024;
const CATALOG_TIMEOUT: Duration = Duration::from_secs(30);

/// Database engines whose table list is read from a remote server. Listing
/// them can stall or fail, so their contents are not read with the catalog.
const EXTERNAL_ENGINES: &[&str] = &[
    "MySQL",
    "MaterializedMySQL",
    "PostgreSQL",
    "MaterializedPostgreSQL",
    "SQLite",
];
const SYSTEM_DATABASES: &[&str] = &["system", "information_schema", "INFORMATION_SCHEMA"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickHouseCatalog {
    /// User databases first, then system databases; each group by name.
    pub databases: Vec<ClickHouseDatabase>,
    /// Dictionaries defined in server configuration rather than a database.
    pub config_dictionaries: Vec<ClickHouseDictionary>,
    /// A list stopped at its bound; some objects are missing.
    pub truncated: bool,
    /// `system.dictionaries` could not be read (usually a privilege); DDL
    /// dictionaries still come from `system.tables`, without status.
    pub dictionaries_error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickHouseDatabase {
    pub name: String,
    pub engine: String,
    pub system: bool,
    /// False for external-engine databases, whose objects are not listed.
    pub listed: bool,
    pub tables: Vec<ClickHouseTable>,
    pub views: Vec<ClickHouseTable>,
    pub materialized_views: Vec<ClickHouseMaterializedView>,
    pub dictionaries: Vec<ClickHouseDictionary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickHouseTable {
    pub name: String,
    pub engine: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickHouseMaterializedView {
    pub name: String,
    /// `database.table` that stores the view's rows, when known.
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickHouseDictionary {
    pub name: String,
    /// `LOADED`, `NOT_LOADED`, `FAILED`, … from `system.dictionaries`.
    pub status: Option<String>,
}

fn limits(max_rows: usize) -> Limits {
    Limits {
        max_rows,
        max_bytes: CATALOG_BYTES,
        timeout: CATALOG_TIMEOUT,
    }
}

fn text(row: &[Option<String>], index: usize) -> String {
    row.get(index).cloned().flatten().unwrap_or_default()
}

pub(crate) async fn fetch(
    connection: &StoredConnection,
) -> Result<ClickHouseCatalog, ClickHouseError> {
    let databases = bounded::run(
        connection,
        "SELECT name, engine FROM system.databases ORDER BY name",
        None,
        limits(MAX_DATABASES),
    )
    .await?;
    let external = EXTERNAL_ENGINES
        .iter()
        .map(|engine| format!("'{engine}'"))
        .collect::<Vec<_>>()
        .join(", ");
    // Only materialized views need their DDL (for the target table).
    let tables = bounded::run(
        connection,
        &format!(
            "SELECT database, name, engine, \
             if(engine = 'MaterializedView', create_table_query, '') AS ddl, \
             toString(uuid) AS uuid \
             FROM system.tables \
             WHERE NOT is_temporary AND database IN \
             (SELECT name FROM system.databases WHERE engine NOT IN ({external})) \
             ORDER BY database, name"
        ),
        None,
        limits(MAX_OBJECTS),
    )
    .await?;
    let dictionaries = bounded::run(
        connection,
        "SELECT database, name, toString(status) AS status FROM system.dictionaries \
         ORDER BY database, name",
        None,
        limits(MAX_OBJECTS),
    )
    .await;
    // A lost connection fails the whole catalog; a refused privilege does not.
    let dictionaries = match dictionaries {
        Err(error) if error.kind != bounded::ClickHouseErrorKind::Server => return Err(error),
        other => other,
    };
    Ok(assemble(databases, tables, dictionaries))
}

pub(crate) fn assemble(
    databases: ClickHouseRows,
    tables: ClickHouseRows,
    dictionaries: Result<ClickHouseRows, ClickHouseError>,
) -> ClickHouseCatalog {
    let mut truncated = databases.truncated.is_some() || tables.truncated.is_some();
    let mut by_name = BTreeMap::new();
    for row in &databases.rows {
        let name = text(row, 0);
        let engine = text(row, 1);
        by_name.insert(
            name.clone(),
            ClickHouseDatabase {
                system: SYSTEM_DATABASES.contains(&name.as_str()),
                listed: !EXTERNAL_ENGINES.contains(&engine.as_str()),
                name,
                engine,
                ..Default::default()
            },
        );
    }
    for row in &tables.rows {
        let (database, name, engine) = (text(row, 0), text(row, 1), text(row, 2));
        let Some(entry) = by_name.get_mut(&database) else {
            continue;
        };
        match engine.as_str() {
            "View" | "LiveView" | "WindowView" => {
                entry.views.push(ClickHouseTable { name, engine });
            }
            "MaterializedView" => {
                let target =
                    materialized_view_target(&text(row, 3), &database, &name, &text(row, 4));
                entry
                    .materialized_views
                    .push(ClickHouseMaterializedView { name, target });
            }
            "Dictionary" => entry
                .dictionaries
                .push(ClickHouseDictionary { name, status: None }),
            _ => entry.tables.push(ClickHouseTable { name, engine }),
        }
    }
    let mut config_dictionaries = Vec::new();
    let dictionaries_error = match dictionaries {
        Ok(rows) => {
            truncated |= rows.truncated.is_some();
            for row in &rows.rows {
                let (database, name) = (text(row, 0), text(row, 1));
                let status = row.get(2).cloned().flatten().filter(|s| !s.is_empty());
                let list = match by_name.get_mut(&database) {
                    Some(entry) => &mut entry.dictionaries,
                    None if database.is_empty() => &mut config_dictionaries,
                    None => continue,
                };
                match list.iter_mut().find(|dictionary| dictionary.name == name) {
                    Some(existing) => existing.status = status,
                    None => list.push(ClickHouseDictionary { name, status }),
                }
            }
            None
        }
        Err(error) => Some(error.message),
    };
    let mut databases = by_name.into_values().collect::<Vec<_>>();
    for database in &mut databases {
        database.dictionaries.sort_by(|a, b| a.name.cmp(&b.name));
    }
    databases.sort_by(|a, b| a.system.cmp(&b.system).then_with(|| a.name.cmp(&b.name)));
    ClickHouseCatalog {
        databases,
        config_dictionaries,
        truncated,
        dictionaries_error,
    }
}

/// Target of a materialized view: the `TO` table from its DDL, else the
/// implicit inner table (`.inner_id.<uuid>` on Atomic databases, `.inner.<name>`
/// on Ordinary ones).
pub(crate) fn materialized_view_target(
    ddl: &str,
    database: &str,
    name: &str,
    uuid: &str,
) -> Option<String> {
    if let Some(target) = to_clause(ddl) {
        return Some(if target.contains('.') {
            target
        } else {
            format!("{database}.{target}")
        });
    }
    if ddl.is_empty() {
        return None;
    }
    let nil = uuid.is_empty() || uuid.bytes().all(|b| b == b'0' || b == b'-');
    Some(if nil {
        format!("{database}..inner.{name}")
    } else {
        format!("{database}..inner_id.{uuid}")
    })
}

/// Reads `CREATE MATERIALIZED VIEW [IF NOT EXISTS] name [ON CLUSTER c] TO
/// target …` and returns `target` with identifier quotes removed.
fn to_clause(ddl: &str) -> Option<String> {
    let tokens = tokens(ddl);
    let mut index = 0;
    for keyword in ["CREATE", "MATERIALIZED", "VIEW"] {
        if !tokens.get(index)?.eq_ignore_ascii_case(keyword) {
            return None;
        }
        index += 1;
    }
    if tokens.get(index)?.eq_ignore_ascii_case("IF") {
        index += 3;
    }
    index = skip_name(&tokens, index)?;
    if tokens.get(index)?.eq_ignore_ascii_case("ON") {
        index = skip_name(&tokens, index + 2)?;
    }
    if !tokens.get(index)?.eq_ignore_ascii_case("TO") {
        return None;
    }
    let mut target = unquote(tokens.get(index + 1)?);
    if tokens.get(index + 2).map(String::as_str) == Some(".") {
        target = format!("{target}.{}", unquote(tokens.get(index + 3)?));
    }
    Some(target)
}

fn skip_name(tokens: &[String], index: usize) -> Option<usize> {
    tokens.get(index)?;
    Some(if tokens.get(index + 1).map(String::as_str) == Some(".") {
        index + 3
    } else {
        index + 1
    })
}

fn unquote(token: &str) -> String {
    let bytes = token.as_bytes();
    match (bytes.first(), bytes.last()) {
        (Some(b'`'), Some(b'`')) | (Some(b'"'), Some(b'"')) if token.len() >= 2 => {
            token[1..token.len() - 1].to_string()
        }
        _ => token.to_string(),
    }
}

/// Words, quoted identifiers and single punctuation; enough for the DDL head.
fn tokens(sql: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = sql.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        if ch == '`' || ch == '"' {
            let mut end = sql.len();
            let mut escaped = false;
            for (index, next) in chars.by_ref() {
                if escaped {
                    escaped = false;
                } else if next == '\\' {
                    escaped = true;
                } else if next == ch {
                    end = index + next.len_utf8();
                    break;
                }
            }
            tokens.push(sql[start..end].to_string());
        } else if ch.is_alphanumeric() || ch == '_' {
            let mut end = start + ch.len_utf8();
            while let Some(&(index, next)) = chars.peek() {
                if next.is_alphanumeric() || next == '_' {
                    end = index + next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            tokens.push(sql[start..end].to_string());
        } else {
            tokens.push(ch.to_string());
        }
        // The head never needs more than a dozen tokens.
        if tokens.len() > 16 {
            break;
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clickhouse::bounded::{ClickHouseErrorKind, ClickHouseTruncation};

    fn rows(values: &[&[Option<&str>]]) -> ClickHouseRows {
        ClickHouseRows {
            rows: values
                .iter()
                .map(|row| row.iter().map(|cell| cell.map(str::to_string)).collect())
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn materialized_view_targets_follow_to_clause_or_inner_table() {
        assert_eq!(
            materialized_view_target(
                "CREATE MATERIALIZED VIEW analytics.mv TO analytics.daily (`d` Date) AS SELECT 1",
                "analytics",
                "mv",
                "6a1f",
            )
            .as_deref(),
            Some("analytics.daily")
        );
        assert_eq!(
            materialized_view_target(
                "CREATE MATERIALIZED VIEW IF NOT EXISTS `a b`.`m v` ON CLUSTER main TO `other db`.`t` AS SELECT 1",
                "a b",
                "m v",
                "",
            )
            .as_deref(),
            Some("other db.t")
        );
        assert_eq!(
            materialized_view_target(
                "CREATE MATERIALIZED VIEW db.mv TO target AS SELECT 1",
                "db",
                "mv",
                ""
            )
            .as_deref(),
            Some("db.target")
        );
        assert_eq!(
            materialized_view_target(
                "CREATE MATERIALIZED VIEW db.mv ENGINE = MergeTree ORDER BY x AS SELECT 1",
                "db",
                "mv",
                "11111111-2222-3333-4444-555555555555",
            )
            .as_deref(),
            Some("db..inner_id.11111111-2222-3333-4444-555555555555")
        );
        assert_eq!(
            materialized_view_target(
                "CREATE MATERIALIZED VIEW db.mv ENGINE = Memory AS SELECT 1",
                "db",
                "mv",
                "00000000-0000-0000-0000-000000000000",
            )
            .as_deref(),
            Some("db..inner.mv")
        );
        assert_eq!(materialized_view_target("", "db", "mv", ""), None);
    }

    #[test]
    fn catalog_lists_every_database_and_keeps_kinds_separate() {
        let catalog = assemble(
            rows(&[
                &[Some("analytics"), Some("Atomic")],
                &[Some("default"), Some("Atomic")],
                &[Some("legacy_pg"), Some("PostgreSQL")],
                &[Some("system"), Some("Atomic")],
            ]),
            rows(&[
                &[
                    Some("analytics"),
                    Some("daily"),
                    Some("SummingMergeTree"),
                    Some(""),
                    None,
                ],
                &[
                    Some("analytics"),
                    Some("events"),
                    Some("Distributed"),
                    Some(""),
                    None,
                ],
                &[
                    Some("analytics"),
                    Some("mv"),
                    Some("MaterializedView"),
                    Some("CREATE MATERIALIZED VIEW analytics.mv TO analytics.daily AS SELECT 1"),
                    Some("x"),
                ],
                &[
                    Some("analytics"),
                    Some("recent"),
                    Some("View"),
                    Some(""),
                    None,
                ],
                &[
                    Some("analytics"),
                    Some("geo"),
                    Some("Dictionary"),
                    Some(""),
                    None,
                ],
                &[
                    Some("default"),
                    Some("t"),
                    Some("MergeTree"),
                    Some(""),
                    None,
                ],
                &[
                    Some("system"),
                    Some("parts"),
                    Some("SystemParts"),
                    Some(""),
                    None,
                ],
                &[Some("gone"), Some("x"), Some("MergeTree"), Some(""), None],
            ]),
            Ok(rows(&[
                &[Some(""), Some("from_config"), Some("LOADED")],
                &[Some("analytics"), Some("geo"), Some("FAILED")],
            ])),
        );
        let names = catalog
            .databases
            .iter()
            .map(|database| database.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["analytics", "default", "legacy_pg", "system"]);
        let analytics = &catalog.databases[0];
        assert_eq!(
            analytics.tables,
            vec![
                ClickHouseTable {
                    name: "daily".into(),
                    engine: "SummingMergeTree".into()
                },
                ClickHouseTable {
                    name: "events".into(),
                    engine: "Distributed".into()
                },
            ]
        );
        assert_eq!(analytics.views.len(), 1);
        assert_eq!(
            analytics.materialized_views,
            vec![ClickHouseMaterializedView {
                name: "mv".into(),
                target: Some("analytics.daily".into())
            }]
        );
        assert_eq!(
            analytics.dictionaries,
            vec![ClickHouseDictionary {
                name: "geo".into(),
                status: Some("FAILED".into())
            }]
        );
        assert!(!catalog.databases[2].listed);
        assert!(catalog.databases[3].system);
        assert_eq!(catalog.config_dictionaries[0].name, "from_config");
        assert!(!catalog.truncated);
        assert_eq!(catalog.dictionaries_error, None);
    }

    #[test]
    fn bounds_and_unreadable_dictionaries_are_reported() {
        let mut tables = rows(&[&[Some("db"), Some("d"), Some("Dictionary"), Some(""), None]]);
        tables.truncated = Some(ClickHouseTruncation::Rows);
        let catalog = assemble(
            rows(&[&[Some("db"), Some("Atomic")]]),
            tables,
            Err(ClickHouseError::new(
                ClickHouseErrorKind::Server,
                "Code: 497. Not enough privileges",
            )),
        );
        assert!(catalog.truncated);
        assert!(catalog.dictionaries_error.unwrap().contains("privileges"));
        assert_eq!(catalog.databases[0].dictionaries[0].status, None);
    }
}
