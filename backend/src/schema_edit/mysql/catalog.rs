//! Raw `information_schema` rows and `SHOW CREATE TABLE` for one table.
//! Every value is read as text (`CAST(… AS CHAR)`, decoded lossily), so the
//! pure [`super::build`] step owns all interpretation and is testable with
//! fixture rows.
use super::sql::qualified;
use super::version::Version;
use crate::schema_edit::{
    MAX_SCHEMA_EDIT_COLUMNS, MAX_SCHEMA_EDIT_MEMBERS, MAX_SCHEMA_EDIT_OBJECTS,
};
use sqlx::mysql::{MySqlConnection, MySqlRow};
use sqlx::{Executor, Row};

/// Bytes of `SHOW CREATE TABLE` accepted.
pub(super) const MAX_DEFINITION_BYTES: usize = 1024 * 1024;
/// Index rows: every member of every index.
const MAX_INDEX_ROWS: usize = MAX_SCHEMA_EDIT_OBJECTS * MAX_SCHEMA_EDIT_MEMBERS;
/// Inbound references and dependents listed for review.
const MAX_EVIDENCE_ROWS: usize = 1000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TableRow {
    pub table_type: String,
    pub engine: Option<String>,
    pub collation: Option<String>,
    pub charset: Option<String>,
    pub comment: String,
    pub rows: Option<u64>,
    /// `CREATE_OPTIONS`: the declared options, such as `row_format=DYNAMIC`.
    /// `TABLES.ROW_FORMAT` is the format in effect, which can differ from
    /// the declared one, so it is not read.
    pub create_options: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ColumnRow {
    pub name: String,
    pub ordinal: u32,
    pub default: Option<String>,
    pub nullable: bool,
    pub column_type: String,
    pub charset: Option<String>,
    pub collation: Option<String>,
    pub extra: String,
    pub comment: String,
    pub generation: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct IndexRow {
    pub index: String,
    pub non_unique: bool,
    pub column: Option<String>,
    /// `A`, `D` or NULL.
    pub collation: Option<String>,
    pub sub_part: Option<u32>,
    pub index_type: String,
    pub expression: Option<String>,
    pub comment: String,
    pub visible: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ConstraintRow {
    pub name: String,
    /// `PRIMARY KEY`, `UNIQUE`, `FOREIGN KEY`, `CHECK`.
    pub kind: String,
    pub enforced: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct KeyColumnRow {
    pub constraint: String,
    pub column: String,
    pub referenced_schema: Option<String>,
    pub referenced_table: Option<String>,
    pub referenced_column: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ReferentialRow {
    pub constraint: String,
    pub update_rule: String,
    pub delete_rule: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct CheckRow {
    pub name: String,
    pub clause: String,
    /// MariaDB `Column` or `Table`; MySQL checks are table level.
    pub level: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct InboundRow {
    pub schema: String,
    pub table: String,
    pub constraint: String,
    pub column: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Catalog {
    pub sql_mode: String,
    pub table: TableRow,
    pub columns: Vec<ColumnRow>,
    pub indexes: Vec<IndexRow>,
    pub constraints: Vec<ConstraintRow>,
    pub key_columns: Vec<KeyColumnRow>,
    pub referential: Vec<ReferentialRow>,
    pub checks: Vec<CheckRow>,
    pub inbound: Vec<InboundRow>,
    pub triggers: Vec<String>,
    /// `(schema, name)` of views over the table (MySQL 8.0.13+).
    pub views: Vec<(String, String)>,
    pub create_table: String,
    /// A query hit its row budget.
    pub truncated: bool,
}

/// One cell as text; `None` for NULL. Bytes that are not UTF-8 are replaced
/// rather than failing the capture.
pub(super) fn cell(row: &MySqlRow, index: usize) -> Option<String> {
    let raw = row.try_get_raw(index).ok()?;
    if sqlx::ValueRef::is_null(&raw) {
        return None;
    }
    let bytes = <&[u8] as sqlx::Decode<sqlx::MySql>>::decode(raw).ok()?;
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn text(row: &[Option<String>], index: usize) -> String {
    row.get(index).cloned().flatten().unwrap_or_default()
}

fn optional(row: &[Option<String>], index: usize) -> Option<String> {
    row.get(index).cloned().flatten()
}

fn number<T: std::str::FromStr>(row: &[Option<String>], index: usize) -> Option<T> {
    optional(row, index).and_then(|value| value.trim().parse().ok())
}

fn yes(row: &[Option<String>], index: usize) -> bool {
    text(row, index).eq_ignore_ascii_case("YES")
}

pub(super) fn message(error: &sqlx::Error) -> String {
    match error {
        sqlx::Error::Database(database) => {
            match database.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>() {
                Some(mysql) => format!("{} ({})", mysql.message(), mysql.number()),
                None => database.message().to_owned(),
            }
        }
        sqlx::Error::Io(error) => format!("Connection lost: {error}"),
        error => error.to_string(),
    }
}

/// Runs a bound catalog query whose last placeholder is the row limit, and
/// reports whether the limit was hit.
async fn rows(
    connection: &mut MySqlConnection,
    sql: &str,
    binds: &[&str],
    limit: usize,
) -> Result<(Vec<Vec<Option<String>>>, bool), String> {
    let mut query = sqlx::query(sql);
    for bind in binds {
        query = query.bind(*bind);
    }
    query = query.bind((limit + 1) as u64);
    let rows = query
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| message(&error))?;
    let truncated = rows.len() > limit;
    let rows = rows
        .iter()
        .take(limit)
        .map(|row| (0..row.columns().len()).map(|i| cell(row, i)).collect())
        .collect();
    Ok((rows, truncated))
}

/// Reads everything [`super::build::baseline`] needs. `Err` is a server or
/// connection failure, or a table that does not exist; anything the model
/// cannot represent is left for the build step to report as read-only.
pub(super) async fn read(
    connection: &mut MySqlConnection,
    version: Option<Version>,
    database: &str,
    table: &str,
) -> Result<Catalog, String> {
    let mut catalog = Catalog::default();
    let mysql = |at_least: fn(&Version) -> bool| version.as_ref().is_some_and(at_least);
    let keys = [database, table];

    let (mode, _) = rows(
        connection,
        "SELECT CAST(@@SESSION.sql_mode AS CHAR) FROM DUAL WHERE 1 = 1 LIMIT ?",
        &[],
        1,
    )
    .await?;
    catalog.sql_mode = mode.first().map(|row| text(row, 0)).unwrap_or_default();

    let (tables, _) = rows(
        connection,
        "SELECT CAST(t.TABLE_TYPE AS CHAR), CAST(t.ENGINE AS CHAR), \
         CAST(t.TABLE_COLLATION AS CHAR), CAST(c.CHARACTER_SET_NAME AS CHAR), \
         CAST(t.TABLE_COMMENT AS CHAR), CAST(t.TABLE_ROWS AS CHAR), \
         CAST(t.CREATE_OPTIONS AS CHAR) \
         FROM information_schema.TABLES t \
         LEFT JOIN information_schema.COLLATIONS c ON c.COLLATION_NAME = t.TABLE_COLLATION \
         WHERE t.TABLE_SCHEMA = ? AND t.TABLE_NAME = ? LIMIT ?",
        &keys,
        1,
    )
    .await?;
    let Some(row) = tables.first() else {
        return Err(format!("Table {database}.{table} was not found"));
    };
    catalog.table = TableRow {
        table_type: text(row, 0),
        engine: optional(row, 1),
        collation: optional(row, 2),
        charset: optional(row, 3),
        comment: text(row, 4),
        rows: number(row, 5),
        create_options: text(row, 6),
    };

    let generation = if mysql(Version::has_generation_expression) {
        "CAST(GENERATION_EXPRESSION AS CHAR)"
    } else {
        "NULL"
    };
    let (columns, cut) = rows(
        connection,
        &format!(
            "SELECT CAST(COLUMN_NAME AS CHAR), CAST(ORDINAL_POSITION AS CHAR), \
             CAST(COLUMN_DEFAULT AS CHAR), CAST(IS_NULLABLE AS CHAR), CAST(COLUMN_TYPE AS CHAR), \
             CAST(CHARACTER_SET_NAME AS CHAR), CAST(COLLATION_NAME AS CHAR), CAST(EXTRA AS CHAR), \
             CAST(COLUMN_COMMENT AS CHAR), {generation} \
             FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
             ORDER BY ORDINAL_POSITION LIMIT ?"
        ),
        &keys,
        MAX_SCHEMA_EDIT_COLUMNS,
    )
    .await?;
    catalog.truncated |= cut;
    catalog.columns = columns
        .iter()
        .map(|row| ColumnRow {
            name: text(row, 0),
            ordinal: number(row, 1).unwrap_or_default(),
            default: optional(row, 2),
            nullable: yes(row, 3),
            column_type: text(row, 4),
            charset: optional(row, 5),
            collation: optional(row, 6),
            extra: text(row, 7),
            comment: text(row, 8),
            generation: optional(row, 9),
        })
        .collect();

    let expression = if mysql(Version::has_index_expression) {
        "CAST(`EXPRESSION` AS CHAR)"
    } else {
        "NULL"
    };
    let visible = if mysql(Version::has_index_visibility) {
        "CAST(IS_VISIBLE AS CHAR)"
    } else if mysql(Version::has_ignored_index) {
        "CASE WHEN IGNORED = 'YES' THEN 'NO' ELSE 'YES' END"
    } else {
        "'YES'"
    };
    let (indexes, cut) = rows(
        connection,
        &format!(
            "SELECT CAST(INDEX_NAME AS CHAR), CAST(NON_UNIQUE AS CHAR), CAST(SEQ_IN_INDEX AS CHAR), \
             CAST(COLUMN_NAME AS CHAR), CAST(`COLLATION` AS CHAR), CAST(SUB_PART AS CHAR), \
             CAST(INDEX_TYPE AS CHAR), {expression}, CAST(INDEX_COMMENT AS CHAR), {visible} \
             FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
             ORDER BY INDEX_NAME, SEQ_IN_INDEX LIMIT ?"
        ),
        &keys,
        MAX_INDEX_ROWS,
    )
    .await?;
    catalog.truncated |= cut;
    catalog.indexes = indexes
        .iter()
        .map(|row| IndexRow {
            index: text(row, 0),
            non_unique: text(row, 1).trim() != "0",
            column: optional(row, 3),
            collation: optional(row, 4),
            sub_part: number(row, 5),
            index_type: text(row, 6),
            expression: optional(row, 7),
            comment: text(row, 8),
            visible: !text(row, 9).eq_ignore_ascii_case("NO"),
        })
        .collect();

    let enforced = if mysql(Version::has_enforced_column) {
        "CAST(ENFORCED AS CHAR)"
    } else {
        "'YES'"
    };
    let (constraints, cut) = rows(
        connection,
        &format!(
            "SELECT CAST(CONSTRAINT_NAME AS CHAR), CAST(CONSTRAINT_TYPE AS CHAR), {enforced} \
             FROM information_schema.TABLE_CONSTRAINTS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
             ORDER BY CONSTRAINT_TYPE, CONSTRAINT_NAME LIMIT ?"
        ),
        &keys,
        MAX_SCHEMA_EDIT_OBJECTS * 4,
    )
    .await?;
    catalog.truncated |= cut;
    catalog.constraints = constraints
        .iter()
        .map(|row| ConstraintRow {
            name: text(row, 0),
            kind: text(row, 1),
            enforced: !text(row, 2).eq_ignore_ascii_case("NO"),
        })
        .collect();

    let (key_columns, cut) = rows(
        connection,
        "SELECT CAST(CONSTRAINT_NAME AS CHAR), CAST(COLUMN_NAME AS CHAR), \
         CAST(ORDINAL_POSITION AS CHAR), CAST(REFERENCED_TABLE_SCHEMA AS CHAR), \
         CAST(REFERENCED_TABLE_NAME AS CHAR), CAST(REFERENCED_COLUMN_NAME AS CHAR) \
         FROM information_schema.KEY_COLUMN_USAGE \
         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND REFERENCED_TABLE_NAME IS NOT NULL \
         ORDER BY CONSTRAINT_NAME, ORDINAL_POSITION LIMIT ?",
        &keys,
        MAX_INDEX_ROWS,
    )
    .await?;
    catalog.truncated |= cut;
    catalog.key_columns = key_columns
        .iter()
        .map(|row| KeyColumnRow {
            constraint: text(row, 0),
            column: text(row, 1),
            referenced_schema: optional(row, 3),
            referenced_table: optional(row, 4),
            referenced_column: optional(row, 5),
        })
        .collect();

    let (referential, cut) = rows(
        connection,
        "SELECT CAST(CONSTRAINT_NAME AS CHAR), CAST(UPDATE_RULE AS CHAR), \
         CAST(DELETE_RULE AS CHAR) FROM information_schema.REFERENTIAL_CONSTRAINTS \
         WHERE CONSTRAINT_SCHEMA = ? AND TABLE_NAME = ? ORDER BY CONSTRAINT_NAME LIMIT ?",
        &keys,
        MAX_SCHEMA_EDIT_OBJECTS,
    )
    .await?;
    catalog.truncated |= cut;
    catalog.referential = referential
        .iter()
        .map(|row| ReferentialRow {
            constraint: text(row, 0),
            update_rule: text(row, 1),
            delete_rule: text(row, 2),
        })
        .collect();

    if mysql(Version::has_check_catalog) {
        let sql = match version {
            Some(version) if version.is_mariadb() => {
                let level = if version.has_check_level() {
                    "CAST(LEVEL AS CHAR)"
                } else {
                    "'Table'"
                };
                format!(
                    "SELECT CAST(CONSTRAINT_NAME AS CHAR), CAST(CHECK_CLAUSE AS CHAR), {level} \
                     FROM information_schema.CHECK_CONSTRAINTS \
                     WHERE CONSTRAINT_SCHEMA = ? AND TABLE_NAME = ? \
                     ORDER BY CONSTRAINT_NAME LIMIT ?"
                )
            }
            _ => "SELECT CAST(cc.CONSTRAINT_NAME AS CHAR), CAST(cc.CHECK_CLAUSE AS CHAR), 'Table' \
                  FROM information_schema.CHECK_CONSTRAINTS cc \
                  JOIN information_schema.TABLE_CONSTRAINTS tc \
                  ON tc.CONSTRAINT_SCHEMA = cc.CONSTRAINT_SCHEMA \
                  AND tc.CONSTRAINT_NAME = cc.CONSTRAINT_NAME AND tc.CONSTRAINT_TYPE = 'CHECK' \
                  WHERE tc.TABLE_SCHEMA = ? AND tc.TABLE_NAME = ? \
                  ORDER BY cc.CONSTRAINT_NAME LIMIT ?"
                .to_owned(),
        };
        let (checks, cut) = rows(connection, &sql, &keys, MAX_SCHEMA_EDIT_OBJECTS).await?;
        catalog.truncated |= cut;
        catalog.checks = checks
            .iter()
            .map(|row| CheckRow {
                name: text(row, 0),
                clause: text(row, 1),
                level: text(row, 2),
            })
            .collect();
    }

    // Review evidence; never part of the drift token.
    let (inbound, _) = rows(
        connection,
        "SELECT CAST(TABLE_SCHEMA AS CHAR), CAST(TABLE_NAME AS CHAR), \
         CAST(CONSTRAINT_NAME AS CHAR), CAST(REFERENCED_COLUMN_NAME AS CHAR) \
         FROM information_schema.KEY_COLUMN_USAGE \
         WHERE REFERENCED_TABLE_SCHEMA = ? AND REFERENCED_TABLE_NAME = ? \
         AND NOT (TABLE_SCHEMA = ? AND TABLE_NAME = ?) \
         ORDER BY TABLE_SCHEMA, TABLE_NAME, CONSTRAINT_NAME, ORDINAL_POSITION LIMIT ?",
        &[database, table, database, table],
        MAX_EVIDENCE_ROWS,
    )
    .await?;
    catalog.inbound = inbound
        .iter()
        .map(|row| InboundRow {
            schema: text(row, 0),
            table: text(row, 1),
            constraint: text(row, 2),
            column: text(row, 3),
        })
        .collect();
    let (triggers, _) = rows(
        connection,
        "SELECT CAST(TRIGGER_NAME AS CHAR) FROM information_schema.TRIGGERS \
         WHERE EVENT_OBJECT_SCHEMA = ? AND EVENT_OBJECT_TABLE = ? ORDER BY TRIGGER_NAME LIMIT ?",
        &keys,
        MAX_EVIDENCE_ROWS,
    )
    .await?;
    catalog.triggers = triggers.iter().map(|row| text(row, 0)).collect();
    if mysql(Version::has_view_table_usage) {
        let (views, _) = rows(
            connection,
            "SELECT CAST(VIEW_SCHEMA AS CHAR), CAST(VIEW_NAME AS CHAR) \
             FROM information_schema.VIEW_TABLE_USAGE \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY VIEW_SCHEMA, VIEW_NAME LIMIT ?",
            &keys,
            MAX_EVIDENCE_ROWS,
        )
        .await?;
        catalog.views = views
            .iter()
            .map(|row| (text(row, 0), text(row, 1)))
            .collect();
    }

    if catalog.table.table_type.eq_ignore_ascii_case("BASE TABLE") {
        catalog.create_table = show_create(connection, database, table).await?;
    }
    Ok(catalog)
}

async fn show_create(
    connection: &mut MySqlConnection,
    database: &str,
    table: &str,
) -> Result<String, String> {
    let sql = format!("SHOW CREATE TABLE {}", qualified(database, table));
    let row = (&mut *connection)
        .fetch_optional(sql.as_str())
        .await
        .map_err(|error| message(&error))?
        .ok_or_else(|| format!("Table {database}.{table} was not found"))?;
    if row.columns().len() < 2 {
        return Err("The server returned no table definition".into());
    }
    let definition = cell(&row, 1).ok_or("The table definition is not visible to this account")?;
    if definition.len() > MAX_DEFINITION_BYTES {
        return Err("The table definition is too large to edit".into());
    }
    Ok(definition)
}
