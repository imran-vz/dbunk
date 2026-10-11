//! The engine-neutral table model shared by every schema-edit planner.
//!
//! A [`SchemaBaseline`] is one complete captured table. A [`SchemaDraft`] is an
//! editable copy whose every column, key, index and constraint carries an
//! [`Origin`]: `Existing` objects keep the baseline identity through renames,
//! so a rename is never planned as drop plus add. Columns are referenced by
//! origin from keys, indexes and foreign keys, so a renamed column stays a
//! member of its keys.
use serde::{Deserialize, Serialize};
use std::fmt;

/// Columns per table. PostgreSQL allows 1600, MySQL 4096, SQLite 2000 by
/// default; the editor refuses anything larger.
pub const MAX_SCHEMA_EDIT_COLUMNS: usize = 4096;
/// Keys, foreign keys, checks and indexes, each.
pub const MAX_SCHEMA_EDIT_OBJECTS: usize = 1024;
/// Members of one key or index.
pub const MAX_SCHEMA_EDIT_MEMBERS: usize = 64;
/// Identifier bytes accepted by any engine before its own rule applies.
pub const MAX_SCHEMA_EDIT_NAME_BYTES: usize = 256;
/// Comment bytes.
pub const MAX_SCHEMA_EDIT_COMMENT_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SchemaEngine {
    Postgres,
    MySql,
    Sqlite,
}

impl SchemaEngine {
    pub fn label(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::MySql => "MySQL",
            Self::Sqlite => "SQLite",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerFlavor {
    Postgres,
    MySql,
    MariaDb,
    Sqlite,
}

/// What the captured server supports. Planners consult these, never the
/// version string.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerFeatures {
    /// `RENAME COLUMN` (SQLite 3.25+, MySQL 8.0+, MariaDB 10.5+).
    pub rename_column: bool,
    /// `ALTER TABLE … DROP COLUMN` (SQLite 3.35+).
    pub drop_column: bool,
    /// MySQL 8.0.12+ `ALGORITHM=INSTANT` for ADD COLUMN.
    pub instant_add_column: bool,
    /// MySQL 8.0.29+: instant ADD COLUMN at any position.
    pub instant_add_column_anywhere: bool,
    /// CHECK constraints are enforced, not just parsed.
    pub enforced_check: bool,
    /// Columns can be reordered (`FIRST` / `AFTER`).
    pub column_reorder: bool,
    /// DDL runs inside a transaction (PostgreSQL, SQLite).
    pub transactional_ddl: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerFacts {
    pub flavor: ServerFlavor,
    pub version: String,
    pub features: ServerFeatures,
}

/// Opaque, engine-specific evidence that the table has not changed since
/// capture: PostgreSQL relation OID and xmins, SQLite `schema_version`, a hash
/// of the full MySQL introspection. Compared for equality only.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DriftToken(pub String);

impl fmt::Debug for DriftToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DriftToken(<opaque>)")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Editability {
    Editable,
    /// The editor shows the structure but offers no edit. Every reason is
    /// user-facing text (unsupported server, unparseable definition, a
    /// catalog/definition mismatch, a read-only connection).
    ReadOnly {
        reasons: Vec<String>,
    },
}

impl Editability {
    pub fn is_editable(&self) -> bool {
        matches!(self, Self::Editable)
    }
}

/// Identity of one object within its baseline. Engines choose a key that is
/// stable across captures of an unchanged table (PostgreSQL attnum or
/// constraint OID, SQLite cid, MySQL ordinal), so a rebase can carry edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BaselineId(pub u64);

/// Identity of an object the draft created.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DraftId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "camelCase")]
pub enum Origin {
    Existing(BaselineId),
    New(DraftId),
}

impl Origin {
    pub fn is_new(self) -> bool {
        matches!(self, Self::New(_))
    }
    pub fn existing(self) -> Option<BaselineId> {
        match self {
            Self::Existing(id) => Some(id),
            Self::New(_) => None,
        }
    }
}

/// One SQL expression (a default, check, generated column or index
/// predicate). It is the server's own rendering when captured and compared
/// byte for byte, so an untouched expression is never a change. See
/// [`super::fragment`] for the per-dialect shape check.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SqlFragment(pub String);

impl SqlFragment {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SqlFragment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SqlFragment({:?})", self.0)
    }
}

impl fmt::Display for SqlFragment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum DefaultValue {
    #[default]
    None,
    /// A string literal; planners quote it for their dialect.
    Literal(String),
    /// Raw SQL, rendered as written (wrapped in parentheses where required).
    Expression(SqlFragment),
}

impl DefaultValue {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Generated {
    pub expression: SqlFragment,
    /// `STORED`; otherwise `VIRTUAL`.
    pub stored: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Identity {
    #[default]
    None,
    /// PostgreSQL `GENERATED ALWAYS AS IDENTITY`.
    Always,
    /// PostgreSQL `GENERATED BY DEFAULT AS IDENTITY`.
    ByDefault,
    /// MySQL `AUTO_INCREMENT`, SQLite `AUTOINCREMENT`.
    AutoIncrement,
}

/// MySQL column attributes that a `MODIFY COLUMN` must restate. Empty
/// elsewhere.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ColumnExtra {
    pub charset: Option<String>,
    /// `ON UPDATE CURRENT_TIMESTAMP[(n)]`.
    pub on_update: Option<SqlFragment>,
    pub invisible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnModel {
    pub origin: Origin,
    pub name: String,
    /// Validated per dialect, compared as normalized text.
    pub data_type: String,
    pub nullable: bool,
    pub default: DefaultValue,
    pub generated: Option<Generated>,
    pub identity: Identity,
    pub collation: Option<String>,
    pub comment: Option<String>,
    pub extra: ColumnExtra,
}

impl ColumnModel {
    /// A nullable column with no default; the caller sets the rest.
    pub fn new(origin: Origin, name: impl Into<String>, data_type: impl Into<String>) -> Self {
        Self {
            origin,
            name: name.into(),
            data_type: data_type.into(),
            nullable: true,
            default: DefaultValue::None,
            generated: None,
            identity: Identity::None,
            collation: None,
            comment: None,
            extra: ColumnExtra::default(),
        }
    }
}

/// A primary key or unique constraint. `columns` are column origins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyModel {
    pub origin: Origin,
    /// `None` lets the server name it (SQLite and MySQL primary keys).
    pub name: Option<String>,
    pub columns: Vec<Origin>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReferentialAction {
    #[default]
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ReferentialAction {
    pub fn sql(self) -> &'static str {
        match self {
            Self::NoAction => "NO ACTION",
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Deferrable {
    #[default]
    /// `NOT DEFERRABLE`.
    Never,
    InitiallyImmediate,
    InitiallyDeferred,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyModel {
    pub origin: Origin,
    pub name: Option<String>,
    /// Local column origins, paired by position with `referenced_columns`.
    pub columns: Vec<Origin>,
    /// `None` means the table's own schema (and always for SQLite).
    pub referenced_schema: Option<String>,
    pub referenced_table: String,
    /// Names in the referenced table. Empty means its primary key (SQLite).
    pub referenced_columns: Vec<String>,
    pub on_update: ReferentialAction,
    pub on_delete: ReferentialAction,
    pub deferrable: Deferrable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckModel {
    pub origin: Origin,
    pub name: Option<String>,
    pub expression: SqlFragment,
    /// MySQL `NOT ENFORCED` is `false`; always `true` elsewhere.
    pub enforced: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum IndexTarget {
    Column(Origin),
    Expression(SqlFragment),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexMember {
    pub target: IndexTarget,
    pub descending: bool,
    /// MySQL key prefix length.
    pub prefix_length: Option<u32>,
    pub collation: Option<String>,
}

impl IndexMember {
    pub fn column(origin: Origin) -> Self {
        Self {
            target: IndexTarget::Column(origin),
            descending: false,
            prefix_length: None,
            collation: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexModel {
    pub origin: Origin,
    pub name: String,
    pub unique: bool,
    /// Access method (`btree`, `gin`, `HASH`, `FULLTEXT` …); `None` is the
    /// engine default.
    pub method: Option<String>,
    pub members: Vec<IndexMember>,
    /// PostgreSQL `INCLUDE` columns.
    pub include: Vec<Origin>,
    /// Partial index predicate (PostgreSQL, SQLite).
    pub predicate: Option<SqlFragment>,
}

/// Engine table options. Fields that do not apply to an engine stay at their
/// defaults and never differ.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TableOptions {
    /// SQLite `STRICT`.
    pub strict: bool,
    /// SQLite `WITHOUT ROWID`.
    pub without_rowid: bool,
    /// MySQL `ENGINE`.
    pub engine: Option<String>,
    /// MySQL default character set.
    pub charset: Option<String>,
    /// MySQL default collation.
    pub collation: Option<String>,
    /// MySQL `ROW_FORMAT`.
    pub row_format: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableModel {
    /// PostgreSQL schema, MySQL database, SQLite schema (`main`, `temp` or an
    /// attachment).
    pub schema: String,
    pub name: String,
    pub comment: Option<String>,
    pub columns: Vec<ColumnModel>,
    pub primary_key: Option<KeyModel>,
    pub uniques: Vec<KeyModel>,
    pub foreign_keys: Vec<ForeignKeyModel>,
    pub checks: Vec<CheckModel>,
    pub indexes: Vec<IndexModel>,
    pub options: TableOptions,
}

impl TableModel {
    pub fn column(&self, origin: Origin) -> Option<&ColumnModel> {
        self.columns.iter().find(|column| column.origin == origin)
    }
    pub fn column_mut(&mut self, origin: Origin) -> Option<&mut ColumnModel> {
        self.columns
            .iter_mut()
            .find(|column| column.origin == origin)
    }
    pub fn column_name(&self, origin: Origin) -> Option<&str> {
        self.column(origin).map(|column| column.name.as_str())
    }
    /// Member names in order; a dangling origin renders as `?`.
    pub fn column_names(&self, origins: &[Origin]) -> Vec<String> {
        origins
            .iter()
            .map(|origin| self.column_name(*origin).unwrap_or("?").to_owned())
            .collect()
    }
    pub fn key(&self, origin: Origin) -> Option<&KeyModel> {
        self.primary_key
            .iter()
            .chain(self.uniques.iter())
            .find(|key| key.origin == origin)
    }
    pub fn foreign_key(&self, origin: Origin) -> Option<&ForeignKeyModel> {
        self.foreign_keys.iter().find(|key| key.origin == origin)
    }
    pub fn check(&self, origin: Origin) -> Option<&CheckModel> {
        self.checks.iter().find(|check| check.origin == origin)
    }
    pub fn index(&self, origin: Origin) -> Option<&IndexModel> {
        self.indexes.iter().find(|index| index.origin == origin)
    }
    /// True when a key, index or foreign key uses the column as a member.
    pub fn column_is_referenced(&self, origin: Origin) -> bool {
        self.primary_key
            .iter()
            .chain(self.uniques.iter())
            .any(|key| key.columns.contains(&origin))
            || self
                .foreign_keys
                .iter()
                .any(|key| key.columns.contains(&origin))
            || self.indexes.iter().any(|index| index_uses(index, origin))
    }
}

pub(crate) fn index_uses(index: &IndexModel, origin: Origin) -> bool {
    index.include.contains(&origin)
        || index
            .members
            .iter()
            .any(|member| member.target == IndexTarget::Column(origin))
}

/// A foreign key elsewhere that references this table. Review evidence only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboundReference {
    pub schema: String,
    pub table: String,
    pub name: String,
    /// Referenced column names in this table.
    pub columns: Vec<String>,
}

/// A view, trigger or other object that depends on this table and that a
/// rebuild or column drop can break. Review evidence only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dependent {
    /// `view`, `trigger`, `materialized view`, …
    pub kind: String,
    pub schema: String,
    pub name: String,
    /// The captured definition, when the engine re-creates it (SQLite
    /// triggers after a rebuild).
    pub definition: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaBaseline {
    pub engine: SchemaEngine,
    pub server: ServerFacts,
    pub table: TableModel,
    pub drift: DriftToken,
    pub editable: Editability,
    pub inbound: Vec<InboundReference>,
    pub dependents: Vec<Dependent>,
    /// Approximate row count, for large-table notes; `None` when unknown.
    pub estimated_rows: Option<u64>,
}

/// An editable copy of a baseline's table. Every object the user adds gets a
/// fresh [`DraftId`]; editing never changes an origin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDraft {
    pub table: TableModel,
    /// The next `DraftId`. Monotonic, so an id is never reused within a draft.
    pub next_id: u64,
}

impl SchemaDraft {
    pub fn from_baseline(baseline: &SchemaBaseline) -> Self {
        Self {
            table: baseline.table.clone(),
            next_id: 1,
        }
    }

    pub fn new_origin(&mut self) -> Origin {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        Origin::New(DraftId(id))
    }

    /// Appends a new nullable column and returns its origin.
    pub fn add_column(&mut self, name: impl Into<String>, data_type: impl Into<String>) -> Origin {
        let origin = self.new_origin();
        self.table
            .columns
            .push(ColumnModel::new(origin, name, data_type));
        origin
    }

    /// Removes a column and, as PostgreSQL's `DROP COLUMN` does, every key,
    /// index and foreign key that uses it. Checks are expressions and are
    /// left for the planner and validation to report.
    pub fn remove_column(&mut self, origin: Origin) -> bool {
        let before = self.table.columns.len();
        self.table.columns.retain(|column| column.origin != origin);
        if self.table.columns.len() == before {
            return false;
        }
        self.drop_references(origin);
        true
    }

    /// Removes every key, index and foreign key that uses `origin` as a
    /// member, as a column drop does.
    pub fn drop_references(&mut self, origin: Origin) {
        if self
            .table
            .primary_key
            .as_ref()
            .is_some_and(|key| key.columns.contains(&origin))
        {
            self.table.primary_key = None;
        }
        self.table
            .uniques
            .retain(|key| !key.columns.contains(&origin));
        self.table
            .foreign_keys
            .retain(|key| !key.columns.contains(&origin));
        self.table
            .indexes
            .retain(|index| !index_uses(index, origin));
    }

    /// Moves a column to `position` (clamped). Only engines with
    /// `column_reorder` (or a SQLite rebuild) plan a reorder; elsewhere the
    /// planner refuses it as unsupported.
    pub fn move_column(&mut self, origin: Origin, position: usize) -> bool {
        let Some(from) = self
            .table
            .columns
            .iter()
            .position(|column| column.origin == origin)
        else {
            return false;
        };
        let column = self.table.columns.remove(from);
        let to = position.min(self.table.columns.len());
        self.table.columns.insert(to, column);
        true
    }

    /// Restores one column's attributes from the baseline, keeping its
    /// position. A new column is removed instead.
    pub fn revert_column(&mut self, baseline: &SchemaBaseline, origin: Origin) -> bool {
        match baseline.table.column(origin) {
            Some(original) => match self.table.column_mut(origin) {
                Some(column) => {
                    *column = original.clone();
                    true
                }
                None => false,
            },
            None => self.remove_column(origin),
        }
    }
}
