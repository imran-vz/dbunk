//! The Schema Change Set: what a draft changes relative to its baseline, at
//! object and field level, plus the engine-neutral operation order.
use super::model::{BaselineId, Origin};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SchemaObject {
    Table,
    Column,
    PrimaryKey,
    Unique,
    ForeignKey,
    Check,
    Index,
}

impl SchemaObject {
    pub fn label(self) -> &'static str {
        match self {
            Self::Table => "Table",
            Self::Column => "Column",
            Self::PrimaryKey => "Primary key",
            Self::Unique => "Unique",
            Self::ForeignKey => "Foreign key",
            Self::Check => "Check",
            Self::Index => "Index",
        }
    }

    /// The review dialog's grouping: Columns, Keys, Foreign keys, Checks,
    /// Indexes, Table.
    pub fn group(self) -> ChangeGroup {
        match self {
            Self::Column => ChangeGroup::Columns,
            Self::PrimaryKey | Self::Unique => ChangeGroup::Keys,
            Self::ForeignKey => ChangeGroup::ForeignKeys,
            Self::Check => ChangeGroup::Checks,
            Self::Index => ChangeGroup::Indexes,
            Self::Table => ChangeGroup::Table,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeGroup {
    Columns,
    Keys,
    ForeignKeys,
    Checks,
    Indexes,
    Table,
}

impl ChangeGroup {
    pub const ALL: [ChangeGroup; 6] = [
        Self::Columns,
        Self::Keys,
        Self::ForeignKeys,
        Self::Checks,
        Self::Indexes,
        Self::Table,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Columns => "Columns",
            Self::Keys => "Keys",
            Self::ForeignKeys => "Foreign keys",
            Self::Checks => "Checks",
            Self::Indexes => "Indexes",
            Self::Table => "Table",
        }
    }
}

/// One attribute that can change. Renames are [`ChangeKind::Renamed`], not a
/// field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Field {
    // Table.
    Comment,
    Strict,
    WithoutRowid,
    Engine,
    Charset,
    Collation,
    RowFormat,
    // Column (`Comment`, `Charset` and `Collation` are shared).
    DataType,
    Nullable,
    Default,
    Generated,
    Identity,
    OnUpdate,
    Invisible,
    Position,
    // Keys, foreign keys, checks and indexes.
    Columns,
    Members,
    Include,
    Unique,
    Method,
    Predicate,
    Reference,
    OnUpdateAction,
    OnDeleteAction,
    Deferrable,
    Expression,
    Enforced,
}

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Self::Comment => "comment",
            Self::Strict => "STRICT",
            Self::WithoutRowid => "WITHOUT ROWID",
            Self::Engine => "engine",
            Self::Charset => "character set",
            Self::Collation => "collation",
            Self::RowFormat => "row format",
            Self::DataType => "type",
            Self::Nullable => "nullable",
            Self::Default => "default",
            Self::Generated => "generated",
            Self::Identity => "identity",
            Self::OnUpdate => "on update",
            Self::Invisible => "invisible",
            Self::Position => "position",
            Self::Columns => "columns",
            Self::Members => "members",
            Self::Include => "include",
            Self::Unique => "unique",
            Self::Method => "method",
            Self::Predicate => "predicate",
            Self::Reference => "references",
            Self::OnUpdateAction => "on update",
            Self::OnDeleteAction => "on delete",
            Self::Deferrable => "deferrable",
            Self::Expression => "expression",
            Self::Enforced => "enforced",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ChangeKind {
    Added,
    Removed,
    Renamed {
        from: String,
        to: String,
    },
    /// `before` and `after` are display text, not SQL.
    Changed {
        field: Field,
        before: String,
        after: String,
    },
}

/// Why a change can lose data or availability. Drives review emphasis only;
/// gating stays `WriteIntent::Ddl` (ADR-0004).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Destructive {
    DropColumn,
    DropConstraint,
    DropIndex,
    /// A key, check or index is dropped and re-created.
    Recreate,
    /// A type change that is not provably widening.
    NarrowingType,
    SetNotNull,
    /// A new NOT NULL column without a default fails on a non-empty table.
    AddNotNullWithoutDefault,
    /// A generated expression change recomputes every row.
    Regenerate,
}

impl Destructive {
    pub fn label(self) -> &'static str {
        match self {
            Self::DropColumn => "drops the column and its data",
            Self::DropConstraint => "drops a constraint",
            Self::DropIndex => "drops an index",
            Self::Recreate => "drops and re-creates the object",
            Self::NarrowingType => "type change may truncate or fail on existing values",
            Self::SetNotNull => "fails if any row holds NULL",
            Self::AddNotNullWithoutDefault => "fails on a table that has rows",
            Self::Regenerate => "recomputes the column for every row",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaChange {
    pub object: SchemaObject,
    /// `None` for table-level changes.
    pub origin: Option<Origin>,
    /// The draft name, or the baseline name of a removed object.
    pub name: String,
    pub kind: ChangeKind,
    pub destructive: Option<Destructive>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaChangeSet {
    pub changes: Vec<SchemaChange>,
}

impl SchemaChangeSet {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
    pub fn len(&self) -> usize {
        self.changes.len()
    }
    pub fn destructive(&self) -> impl Iterator<Item = &SchemaChange> {
        self.changes
            .iter()
            .filter(|change| change.destructive.is_some())
    }
    pub fn in_group(&self, group: ChangeGroup) -> impl Iterator<Item = &SchemaChange> {
        self.changes
            .iter()
            .filter(move |change| change.object.group() == group)
    }
    /// Changes that touch one object (a column, key, index …).
    pub fn for_origin(&self, origin: Origin) -> impl Iterator<Item = &SchemaChange> {
        self.changes
            .iter()
            .filter(move |change| change.origin == Some(origin))
    }
    /// Distinct objects changed; a column with three field changes counts
    /// once. This is the review button's count.
    pub fn object_count(&self) -> usize {
        let mut seen: Vec<(SchemaObject, Option<Origin>)> = Vec::new();
        for change in &self.changes {
            let key = (change.object, change.origin);
            if !seen.contains(&key) {
                seen.push(key);
            }
        }
        seen.len()
    }
}

/// A column attribute an `ALTER COLUMN` changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColumnField {
    DataType,
    Collation,
    Nullable,
    Default,
    Generated,
    Identity,
    Charset,
    OnUpdate,
    Invisible,
}

/// One engine-neutral operation, in plan order. Engines map steps onto their
/// own statements (or, for a SQLite rebuild, use the whole draft instead).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "step",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlanStep {
    // 1. Drops of removed objects and of objects re-created below.
    DropForeignKey {
        id: BaselineId,
    },
    DropCheck {
        id: BaselineId,
    },
    DropUnique {
        id: BaselineId,
    },
    DropPrimaryKey {
        id: BaselineId,
    },
    DropIndex {
        id: BaselineId,
    },
    // 2. Renames.
    RenameColumn {
        id: BaselineId,
    },
    RenameConstraint {
        object: SchemaObject,
        id: BaselineId,
    },
    RenameTable,
    // 3. Column alterations.
    AlterColumn {
        id: BaselineId,
        fields: Vec<ColumnField>,
    },
    // 4.
    AddColumn {
        origin: Origin,
    },
    // 5.
    DropColumn {
        id: BaselineId,
    },
    /// Column order changed (only engines with `column_reorder` plan it).
    ReorderColumns,
    // 6. Adds; `origin` is the draft object (new, or existing and re-created).
    AddPrimaryKey {
        origin: Origin,
    },
    AddUnique {
        origin: Origin,
    },
    AddCheck {
        origin: Origin,
    },
    CreateIndex {
        origin: Origin,
    },
    // 7.
    AddForeignKey {
        origin: Origin,
    },
    // 8. Comments and options.
    SetTableComment,
    SetColumnComment {
        origin: Origin,
    },
    SetTableOptions {
        fields: Vec<Field>,
    },
}

impl PlanStep {
    /// The plan phase, 1 to 8, as listed in Plan 033.
    pub fn phase(&self) -> u8 {
        match self {
            Self::DropForeignKey { .. }
            | Self::DropCheck { .. }
            | Self::DropUnique { .. }
            | Self::DropPrimaryKey { .. }
            | Self::DropIndex { .. } => 1,
            Self::RenameColumn { .. } | Self::RenameConstraint { .. } | Self::RenameTable => 2,
            Self::AlterColumn { .. } => 3,
            Self::AddColumn { .. } => 4,
            Self::DropColumn { .. } | Self::ReorderColumns => 5,
            Self::AddPrimaryKey { .. }
            | Self::AddUnique { .. }
            | Self::AddCheck { .. }
            | Self::CreateIndex { .. } => 6,
            Self::AddForeignKey { .. } => 7,
            Self::SetTableComment
            | Self::SetColumnComment { .. }
            | Self::SetTableOptions { .. } => 8,
        }
    }
}

/// One problem that keeps a draft from being reviewed. `origin` locates it in
/// the editor; `None` is table-level.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftIssue {
    pub object: SchemaObject,
    pub origin: Option<Origin>,
    pub message: String,
}

impl DraftIssue {
    pub fn table(message: impl Into<String>) -> Self {
        Self {
            object: SchemaObject::Table,
            origin: None,
            message: message.into(),
        }
    }
    pub fn at(object: SchemaObject, origin: Origin, message: impl Into<String>) -> Self {
        Self {
            object,
            origin: Some(origin),
            message: message.into(),
        }
    }
}
