//! Plan 033 draft editing state, pure and engine neutral: one captured
//! baseline, the user's draft, and the change set and validation issues
//! recomputed on every edit. The editor renders rows and inputs from here and
//! never edits the draft itself; backend calls and authority stay with the
//! host (ADR-0016). Text inputs are parsed here so every edit is testable
//! without a window.
use dbunk_lib::backend::schema_edit::{
    ChangeGroup, ChangeKind, CheckModel, ColumnModel, DefaultValue, Deferrable, Destructive,
    DraftIssue, Editability, Field, ForeignKeyModel, Generated, Identity, IndexModel, KeyModel,
    Origin, RebaseConflict, ReferentialAction, SchemaBaseline, SchemaChange, SchemaChangeSet,
    SchemaDraft, SchemaEditRequest, SchemaEngine, SchemaObject, SqlFragment, TableModel, diff,
    rebase as rebase_draft, same_name, validate,
};
use std::collections::{HashMap, HashSet};

mod gate;
mod journal;
mod members;
pub use gate::*;
pub use journal::*;
use members::{members_text, names_to_origins, origins_text, parse_members, split_list, unquote};

#[cfg(test)]
mod tests;

/// Bytes accepted by one text input before the model refuses it. Comments
/// are the longest legitimate values (16 KiB in the backend model).
pub const INPUT_BYTES: usize = 16 * 1024;

/// One editable object of the draft, or of the baseline when removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Table,
    Column(Origin),
    PrimaryKey(Origin),
    Unique(Origin),
    ForeignKey(Origin),
    Check(Origin),
    Index(Origin),
}

impl Target {
    pub fn object(self) -> SchemaObject {
        match self {
            Self::Table => SchemaObject::Table,
            Self::Column(_) => SchemaObject::Column,
            Self::PrimaryKey(_) => SchemaObject::PrimaryKey,
            Self::Unique(_) => SchemaObject::Unique,
            Self::ForeignKey(_) => SchemaObject::ForeignKey,
            Self::Check(_) => SchemaObject::Check,
            Self::Index(_) => SchemaObject::Index,
        }
    }
    pub fn origin(self) -> Option<Origin> {
        match self {
            Self::Table => None,
            Self::Column(origin)
            | Self::PrimaryKey(origin)
            | Self::Unique(origin)
            | Self::ForeignKey(origin)
            | Self::Check(origin)
            | Self::Index(origin) => Some(origin),
        }
    }
}

/// One attribute a user can undo on its own. Renames are not a [`Field`] in
/// the change set, so they get their own variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Attr {
    Name,
    Field(Field),
}

/// How a row is tinted: added `ok_fill`, changed `edited_fill`, removed
/// `deleted_fill` with strikethrough. Removed rows stay visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    Unchanged,
    Added,
    Changed,
    Removed,
}

impl RowState {
    /// The state as the row's accessible text says it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Added => "added",
            Self::Changed => "changed",
            Self::Removed => "removed",
        }
    }
}

/// One changed attribute of a row, as display text (never SQL).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delta {
    pub attr: Attr,
    pub before: String,
    pub after: String,
    pub destructive: Option<Destructive>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub target: Target,
    /// The draft label, or the baseline label of a removed object.
    pub label: String,
    pub state: RowState,
    /// Field changes and renames, in change-set order.
    pub deltas: Vec<Delta>,
    /// The first destructive reason among this row's changes.
    pub destructive: Option<Destructive>,
    pub issues: usize,
    /// The row's accessible text: label, state and every old → new.
    pub description: String,
}

impl Row {
    pub fn delta(&self, attr: Attr) -> Option<&Delta> {
        self.deltas.iter().find(|delta| delta.attr == attr)
    }
}

/// One line of the draft table in display order: a section title, its
/// column heads (only when the section has rows), or one of its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Title(ChangeGroup),
    Head(ChangeGroup),
    Row(ChangeGroup, usize),
}

/// Per-row view data, rebuilt once per edit in `recompute` so rendering a
/// table of thousands of columns never searches the draft or change set.
#[derive(Default)]
struct View {
    sections: Vec<(ChangeGroup, Vec<Row>)>,
    /// Section index, row index and line of each row's target.
    at: HashMap<Target, (usize, usize, usize)>,
    /// 1-based draft position of each column.
    positions: HashMap<Origin, usize>,
    lines: Vec<Line>,
    review_count: usize,
}

/// The row a change or issue belongs to.
fn target_of(object: SchemaObject, origin: Option<Origin>) -> Option<Target> {
    Some(match (object, origin) {
        (SchemaObject::Table, None) => Target::Table,
        (SchemaObject::Table, Some(_)) | (_, None) => return None,
        (SchemaObject::Column, Some(o)) => Target::Column(o),
        (SchemaObject::PrimaryKey, Some(o)) => Target::PrimaryKey(o),
        (SchemaObject::Unique, Some(o)) => Target::Unique(o),
        (SchemaObject::ForeignKey, Some(o)) => Target::ForeignKey(o),
        (SchemaObject::Check, Some(o)) => Target::Check(o),
        (SchemaObject::Index, Some(o)) => Target::Index(o),
    })
}

fn key_label(table: &TableModel, key: &KeyModel) -> String {
    key.name
        .clone()
        .unwrap_or_else(|| format!("({})", table.column_names(&key.columns).join(", ")))
}

fn foreign_key_label(table: &TableModel, key: &ForeignKeyModel) -> String {
    key.name.clone().unwrap_or_else(|| {
        format!(
            "({}) → {}",
            table.column_names(&key.columns).join(", "),
            key.referenced_table
        )
    })
}

fn check_label(check: &CheckModel) -> String {
    check
        .name
        .clone()
        .unwrap_or_else(|| format!("CHECK ({})", check.expression))
}

/// Baseline objects the draft no longer has, in baseline order.
fn removed<'a, T>(old: &'a [T], new: &[T], origin: impl Fn(&T) -> Origin) -> Vec<&'a T> {
    let kept: HashSet<Origin> = new.iter().map(&origin).collect();
    old.iter()
        .filter(|item| !kept.contains(&origin(*item)))
        .collect()
}

fn make_row<'a>(
    target: Target,
    label: String,
    changes: impl Iterator<Item = &'a SchemaChange>,
    issues: usize,
) -> Row {
    let mut state = RowState::Unchanged;
    let mut deltas = Vec::new();
    let mut destructive = None;
    for change in changes {
        destructive = destructive.or(change.destructive);
        match &change.kind {
            ChangeKind::Added => state = RowState::Added,
            ChangeKind::Removed => state = RowState::Removed,
            ChangeKind::Renamed { from, to } => deltas.push(Delta {
                attr: Attr::Name,
                before: from.clone(),
                after: to.clone(),
                destructive: change.destructive,
            }),
            ChangeKind::Changed {
                field,
                before,
                after,
            } => deltas.push(Delta {
                attr: Attr::Field(*field),
                before: before.clone(),
                after: after.clone(),
                destructive: change.destructive,
            }),
        }
    }
    if state == RowState::Unchanged && !deltas.is_empty() {
        state = RowState::Changed;
    }
    let description = if deltas.is_empty() {
        format!("{label}, {}", state.label())
    } else {
        let text = deltas
            .iter()
            .map(|delta| format!("{} to {}", delta.before, delta.after))
            .collect::<Vec<_>>()
            .join("; ");
        format!("{label}, {}: {text}", state.label())
    };
    Row {
        target,
        label,
        state,
        deltas,
        destructive,
        issues,
        description,
    }
}

/// What this engine lets the editor offer. Everything else stays hidden so
/// the user never builds a draft the planner can only refuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub engine: SchemaEngine,
    /// Column reorder (`FIRST` / `AFTER`); MySQL only.
    pub reorder: bool,
    /// Table and column comments; SQLite has none.
    pub comments: bool,
    pub charset: bool,
    pub on_update: bool,
    pub invisible: bool,
    pub deferrable: bool,
    pub check_enforced: bool,
    pub include: bool,
    pub predicate: bool,
    pub index_method: bool,
    pub referenced_schema: bool,
    /// SQLite `STRICT` and `WITHOUT ROWID`.
    pub sqlite_options: bool,
    /// MySQL engine, character set, collation and row format.
    pub mysql_options: bool,
}

impl Capabilities {
    pub fn of(baseline: &SchemaBaseline) -> Self {
        let engine = baseline.engine;
        let postgres = engine == SchemaEngine::Postgres;
        let mysql = engine == SchemaEngine::MySql;
        let sqlite = engine == SchemaEngine::Sqlite;
        Self {
            engine,
            reorder: baseline.server.features.column_reorder,
            comments: !sqlite,
            charset: mysql,
            on_update: mysql,
            invisible: mysql,
            deferrable: postgres,
            check_enforced: mysql,
            include: postgres,
            predicate: postgres || sqlite,
            index_method: postgres || mysql,
            referenced_schema: !sqlite,
            sqlite_options: sqlite,
            mysql_options: mysql,
        }
    }
    /// Identity kinds per engine: PostgreSQL identity columns, MySQL
    /// `AUTO_INCREMENT`, SQLite `AUTOINCREMENT`.
    pub fn identities(&self) -> &'static [Identity] {
        match self.engine {
            SchemaEngine::Postgres => &[Identity::None, Identity::Always, Identity::ByDefault],
            SchemaEngine::MySql | SchemaEngine::Sqlite => {
                &[Identity::None, Identity::AutoIncrement]
            }
        }
    }
    pub fn default_type(&self) -> &'static str {
        match self.engine {
            SchemaEngine::Postgres => "text",
            SchemaEngine::MySql => "varchar(255)",
            SchemaEngine::Sqlite => "TEXT",
        }
    }
}

pub fn identity_label(identity: Identity) -> &'static str {
    match identity {
        Identity::None => "None",
        Identity::Always => "Always",
        Identity::ByDefault => "By default",
        Identity::AutoIncrement => "Auto increment",
    }
}

pub fn action_label(action: ReferentialAction) -> &'static str {
    action.sql()
}

pub const ACTIONS: [ReferentialAction; 5] = [
    ReferentialAction::NoAction,
    ReferentialAction::Restrict,
    ReferentialAction::Cascade,
    ReferentialAction::SetNull,
    ReferentialAction::SetDefault,
];

pub const DEFERRABLE: [(Deferrable, &str); 3] = [
    (Deferrable::Never, "Not deferrable"),
    (Deferrable::InitiallyImmediate, "Immediate"),
    (Deferrable::InitiallyDeferred, "Deferred"),
];

/// A free-text input of the inspector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    /// Table, column, constraint or index name.
    Name,
    /// Table or column comment.
    Comment,
    DataType,
    Default,
    /// Generated column expression.
    Generated,
    /// Column collation, or the MySQL table default collation.
    Collation,
    /// MySQL column charset, or the table default character set.
    Charset,
    OnUpdate,
    /// Key or foreign key columns, comma separated.
    Columns,
    ReferencedSchema,
    ReferencedTable,
    ReferencedColumns,
    /// Check expression.
    Expression,
    /// Index members: `a, b DESC, (lower(c))`, MySQL prefixes as `c(10)`.
    Members,
    Include,
    Method,
    Predicate,
    Engine,
    RowFormat,
}

impl Input {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Comment => "Comment",
            Self::DataType => "Type",
            Self::Default => "Default",
            Self::Generated => "Generated expression",
            Self::Collation => "Collation",
            Self::Charset => "Character set",
            Self::OnUpdate => "On update",
            Self::Columns => "Columns",
            Self::ReferencedSchema => "Referenced schema",
            Self::ReferencedTable => "Referenced table",
            Self::ReferencedColumns => "Referenced columns",
            Self::Expression => "Check expression",
            Self::Members => "Members",
            Self::Include => "Include",
            Self::Method => "Method",
            Self::Predicate => "Predicate (WHERE)",
            Self::Engine => "Engine",
            Self::RowFormat => "Row format",
        }
    }
    /// Multi-line editors for expressions and comments.
    pub fn multiline(self) -> bool {
        matches!(
            self,
            Self::Comment | Self::Default | Self::Generated | Self::Expression | Self::Predicate
        )
    }
    /// A short hint shown under the field.
    pub fn hint(self) -> Option<&'static str> {
        match self {
            Self::Columns | Self::Include => Some("Column names, comma separated"),
            Self::ReferencedColumns => Some("Names in the referenced table, comma separated"),
            Self::Members => Some("Columns or (expression), optional DESC"),
            Self::ReferencedSchema => Some("Empty means this table's schema"),
            _ => None,
        }
    }
    /// The undoable attribute this input edits.
    pub fn attr(self) -> Attr {
        Attr::Field(match self {
            Self::Name => return Attr::Name,
            Self::Comment => Field::Comment,
            Self::DataType => Field::DataType,
            Self::Default => Field::Default,
            Self::Generated => Field::Generated,
            Self::Collation => Field::Collation,
            Self::Charset => Field::Charset,
            Self::OnUpdate => Field::OnUpdate,
            Self::Columns => Field::Columns,
            Self::ReferencedSchema | Self::ReferencedTable | Self::ReferencedColumns => {
                Field::Reference
            }
            Self::Expression => Field::Expression,
            Self::Members => Field::Members,
            Self::Include => Field::Include,
            Self::Method => Field::Method,
            Self::Predicate => Field::Predicate,
            Self::Engine => Field::Engine,
            Self::RowFormat => Field::RowFormat,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultKind {
    None,
    Literal,
    Expression,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratedKind {
    None,
    Virtual,
    Stored,
}

/// A non-text control of the inspector: checkboxes and choice chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Nullable(bool),
    DefaultKind(DefaultKind),
    GeneratedKind(GeneratedKind),
    Identity(Identity),
    Invisible(bool),
    Strict(bool),
    WithoutRowid(bool),
    Unique(bool),
    Enforced(bool),
    OnUpdate(ReferentialAction),
    OnDelete(ReferentialAction),
    Deferrable(Deferrable),
}

impl Choice {
    pub fn attr(self) -> Attr {
        Attr::Field(match self {
            Self::Nullable(_) => Field::Nullable,
            Self::DefaultKind(_) => Field::Default,
            Self::GeneratedKind(_) => Field::Generated,
            Self::Identity(_) => Field::Identity,
            Self::Invisible(_) => Field::Invisible,
            Self::Strict(_) => Field::Strict,
            Self::WithoutRowid(_) => Field::WithoutRowid,
            Self::Unique(_) => Field::Unique,
            Self::Enforced(_) => Field::Enforced,
            Self::OnUpdate(_) => Field::OnUpdateAction,
            Self::OnDelete(_) => Field::OnDeleteAction,
            Self::Deferrable(_) => Field::Deferrable,
        })
    }
}

fn optional(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn optional_raw(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}

pub struct SchemaEditModel {
    request: SchemaEditRequest,
    baseline: SchemaBaseline,
    draft: SchemaDraft,
    caps: Capabilities,
    changes: SchemaChangeSet,
    issues: Vec<DraftIssue>,
    /// Fields and objects the last re-base found changed on both sides; the
    /// draft's value was kept. Informational: they never block review.
    conflicts: Vec<RebaseConflict>,
    view: View,
}

impl SchemaEditModel {
    pub fn new(request: SchemaEditRequest, baseline: SchemaBaseline) -> Self {
        let draft = SchemaDraft::from_baseline(&baseline);
        Self::restore(request, baseline, draft)
    }

    /// A journaled draft edited against this exact baseline.
    pub fn restore(
        request: SchemaEditRequest,
        baseline: SchemaBaseline,
        draft: SchemaDraft,
    ) -> Self {
        let caps = Capabilities::of(&baseline);
        let mut model = Self {
            request,
            baseline,
            draft,
            caps,
            changes: SchemaChangeSet::default(),
            issues: Vec::new(),
            conflicts: Vec::new(),
            view: View::default(),
        };
        model.recompute();
        model
    }

    /// A journaled draft edited against `old`, re-based onto a fresh
    /// capture of the same table that differs from it.
    pub fn rebased(
        request: SchemaEditRequest,
        old: &SchemaBaseline,
        baseline: SchemaBaseline,
        draft: &SchemaDraft,
    ) -> Self {
        let (draft, conflicts) = rebase_draft(old, &baseline, draft);
        let mut model = Self::restore(request, baseline, draft);
        model.conflicts = conflicts;
        model
    }

    fn recompute(&mut self) {
        self.changes = diff(&self.baseline, &self.draft);
        self.issues = validate(&self.baseline, &self.draft);
        self.view = self.build_view();
    }

    fn build_view(&self) -> View {
        let mut changes: HashMap<Target, Vec<&SchemaChange>> = HashMap::new();
        let mut objects = HashSet::new();
        for change in &self.changes.changes {
            objects.insert((change.object, change.origin));
            if let Some(target) = target_of(change.object, change.origin) {
                changes.entry(target).or_default().push(change);
            }
        }
        let mut issues: HashMap<Target, usize> = HashMap::new();
        for issue in &self.issues {
            if let Some(target) = target_of(issue.object, issue.origin) {
                *issues.entry(target).or_default() += 1;
            }
        }
        let mut view = View {
            positions: self
                .draft
                .table
                .columns
                .iter()
                .enumerate()
                .map(|(index, column)| (column.origin, index + 1))
                .collect(),
            review_count: objects.len(),
            ..View::default()
        };
        for group in ChangeGroup::ALL {
            let section = view.sections.len();
            let rows: Vec<Row> = self
                .labeled(group)
                .into_iter()
                .map(|(target, label)| {
                    make_row(
                        target,
                        label,
                        changes.get(&target).into_iter().flatten().copied(),
                        issues.get(&target).copied().unwrap_or(0),
                    )
                })
                .collect();
            view.lines.push(Line::Title(group));
            if !rows.is_empty() {
                view.lines.push(Line::Head(group));
            }
            for (index, row) in rows.iter().enumerate() {
                view.at
                    .insert(row.target, (section, index, view.lines.len()));
                view.lines.push(Line::Row(group, index));
            }
            view.sections.push((group, rows));
        }
        view
    }

    /// The targets of one section with their labels, in display order:
    /// draft objects first, then the baseline objects the draft removed.
    fn labeled(&self, group: ChangeGroup) -> Vec<(Target, String)> {
        let old = &self.baseline.table;
        let new = &self.draft.table;
        let mut out = Vec::new();
        match group {
            ChangeGroup::Table => out.push((Target::Table, new.name.clone())),
            ChangeGroup::Columns => {
                let label = |c: &ColumnModel| (Target::Column(c.origin), c.name.clone());
                out.extend(new.columns.iter().map(label));
                out.extend(
                    removed(&old.columns, &new.columns, |c| c.origin)
                        .into_iter()
                        .map(label),
                );
            }
            ChangeGroup::Keys => {
                if let Some(key) = &new.primary_key {
                    out.push((Target::PrimaryKey(key.origin), key_label(new, key)));
                }
                if let Some(key) = &old.primary_key
                    && new
                        .primary_key
                        .as_ref()
                        .is_none_or(|k| k.origin != key.origin)
                {
                    out.push((Target::PrimaryKey(key.origin), key_label(old, key)));
                }
                out.extend(
                    new.uniques
                        .iter()
                        .map(|k| (Target::Unique(k.origin), key_label(new, k))),
                );
                out.extend(
                    removed(&old.uniques, &new.uniques, |k| k.origin)
                        .into_iter()
                        .map(|k| (Target::Unique(k.origin), key_label(old, k))),
                );
            }
            ChangeGroup::ForeignKeys => {
                out.extend(
                    new.foreign_keys
                        .iter()
                        .map(|k| (Target::ForeignKey(k.origin), foreign_key_label(new, k))),
                );
                out.extend(
                    removed(&old.foreign_keys, &new.foreign_keys, |k| k.origin)
                        .into_iter()
                        .map(|k| (Target::ForeignKey(k.origin), foreign_key_label(old, k))),
                );
            }
            ChangeGroup::Checks => {
                let label = |c: &CheckModel| (Target::Check(c.origin), check_label(c));
                out.extend(new.checks.iter().map(label));
                out.extend(
                    removed(&old.checks, &new.checks, |c| c.origin)
                        .into_iter()
                        .map(label),
                );
            }
            ChangeGroup::Indexes => {
                let label = |i: &IndexModel| (Target::Index(i.origin), i.name.clone());
                out.extend(new.indexes.iter().map(label));
                out.extend(
                    removed(&old.indexes, &new.indexes, |i| i.origin)
                        .into_iter()
                        .map(label),
                );
            }
        }
        out
    }

    pub fn request(&self) -> &SchemaEditRequest {
        &self.request
    }
    pub fn baseline(&self) -> &SchemaBaseline {
        &self.baseline
    }
    pub fn draft(&self) -> &SchemaDraft {
        &self.draft
    }
    pub fn caps(&self) -> Capabilities {
        self.caps
    }
    pub fn changes(&self) -> &SchemaChangeSet {
        &self.changes
    }
    pub fn issues(&self) -> &[DraftIssue] {
        &self.issues
    }
    pub fn conflicts(&self) -> &[RebaseConflict] {
        &self.conflicts
    }
    pub fn editable(&self) -> bool {
        self.baseline.editable.is_editable()
    }
    pub fn read_only_reasons(&self) -> &[String] {
        match &self.baseline.editable {
            Editability::Editable => &[],
            Editability::ReadOnly { reasons } => reasons,
        }
    }
    pub fn has_changes(&self) -> bool {
        !self.changes.is_empty()
    }
    /// The "Review N" count: distinct objects changed.
    pub fn review_count(&self) -> usize {
        self.view.review_count
    }
    /// Why Review is unavailable, or `None` when the draft can be reviewed.
    pub fn review_block(&self) -> Option<String> {
        if let Editability::ReadOnly { reasons } = &self.baseline.editable {
            return Some(
                reasons
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "This table is read-only".into()),
            );
        }
        if self.changes.is_empty() {
            return Some("No changes to review".into());
        }
        match self.issues.len() {
            0 => None,
            1 => Some(self.issues[0].message.clone()),
            count => Some(format!("Fix the {count} problems first")),
        }
    }

    /// Takes a fresh capture of the same table. When nothing changed on the
    /// server the draft carries over as is (only the drift token moves) and
    /// `true` is returned. Otherwise the draft is re-based three-way: edits
    /// the server did not touch carry over, server changes the draft did not
    /// touch show through, and values both changed keep the draft's and are
    /// listed in [`Self::conflicts`]. Conflicts describe this capture only.
    pub fn rebase(&mut self, baseline: SchemaBaseline) -> bool {
        let same = baseline.engine == self.baseline.engine && baseline.table == self.baseline.table;
        self.conflicts.clear();
        if baseline.engine != self.baseline.engine {
            self.draft = SchemaDraft::from_baseline(&baseline);
        } else if !same {
            let (draft, conflicts) = rebase_draft(&self.baseline, &baseline, &self.draft);
            self.draft = draft;
            self.conflicts = conflicts;
        }
        self.caps = Capabilities::of(&baseline);
        self.baseline = baseline;
        self.recompute();
        same
    }

    /// Discard: the draft equals the baseline again.
    pub fn discard(&mut self) {
        self.draft = SchemaDraft::from_baseline(&self.baseline);
        self.conflicts.clear();
        self.recompute();
    }

    // -- Issues ------------------------------------------------------------

    pub fn issues_for(&self, target: Target) -> impl Iterator<Item = &DraftIssue> {
        self.issues
            .iter()
            .filter(move |issue| issue.object == target.object() && issue.origin == target.origin())
    }

    // -- Rows --------------------------------------------------------------

    /// Rows of one section in display order: draft objects first, then the
    /// baseline objects the draft removed. Precomputed per edit.
    pub fn rows(&self, group: ChangeGroup) -> &[Row] {
        self.view
            .sections
            .iter()
            .find(|(section, _)| *section == group)
            .map(|(_, rows)| rows.as_slice())
            .unwrap_or_default()
    }

    /// Every line of the draft table, for the virtualized list.
    pub fn lines(&self) -> &[Line] {
        &self.view.lines
    }

    /// The line showing `target`'s row.
    pub fn line_of(&self, target: Target) -> Option<usize> {
        self.view.at.get(&target).map(|(_, _, line)| *line)
    }

    fn cached_row(&self, target: Target) -> Option<&Row> {
        let (section, index, _) = *self.view.at.get(&target)?;
        self.view.sections.get(section)?.1.get(index)
    }

    fn changes_for(&self, target: Target) -> impl Iterator<Item = &SchemaChange> {
        let object = target.object();
        let origin = target.origin();
        self.changes
            .changes
            .iter()
            .filter(move |change| change.object == object && change.origin == origin)
    }

    pub fn row(&self, target: Target) -> Row {
        match self.cached_row(target) {
            Some(row) => row.clone(),
            None => make_row(
                target,
                self.label(target),
                self.changes_for(target),
                self.issues_for(target).count(),
            ),
        }
    }

    /// True when the target exists in the draft (not removed).
    pub fn in_draft(&self, target: Target) -> bool {
        let table = &self.draft.table;
        match target {
            Target::Table => true,
            Target::Column(o) => table.column(o).is_some(),
            Target::PrimaryKey(o) => table.primary_key.as_ref().is_some_and(|k| k.origin == o),
            Target::Unique(o) => table.uniques.iter().any(|k| k.origin == o),
            Target::ForeignKey(o) => table.foreign_key(o).is_some(),
            Target::Check(o) => table.check(o).is_some(),
            Target::Index(o) => table.index(o).is_some(),
        }
    }

    fn in_baseline(&self, target: Target) -> bool {
        let table = &self.baseline.table;
        match target {
            Target::Table => true,
            Target::Column(o) => table.column(o).is_some(),
            Target::PrimaryKey(o) => table.primary_key.as_ref().is_some_and(|k| k.origin == o),
            Target::Unique(o) => table.uniques.iter().any(|k| k.origin == o),
            Target::ForeignKey(o) => table.foreign_key(o).is_some(),
            Target::Check(o) => table.check(o).is_some(),
            Target::Index(o) => table.index(o).is_some(),
        }
    }

    pub fn label(&self, target: Target) -> String {
        if let Some(row) = self.cached_row(target) {
            return row.label.clone();
        }
        let table = if self.in_draft(target) {
            &self.draft.table
        } else {
            &self.baseline.table
        };
        match target {
            Target::Table => table.name.clone(),
            Target::Column(o) => table.column_name(o).unwrap_or("?").to_owned(),
            Target::PrimaryKey(o) => table
                .primary_key
                .as_ref()
                .filter(|k| k.origin == o)
                .map_or_else(|| "?".into(), |key| key_label(table, key)),
            Target::Unique(o) => table
                .uniques
                .iter()
                .find(|k| k.origin == o)
                .map_or_else(|| "?".into(), |key| key_label(table, key)),
            Target::ForeignKey(o) => table
                .foreign_key(o)
                .map_or_else(|| "?".into(), |key| foreign_key_label(table, key)),
            Target::Check(o) => table.check(o).map_or_else(|| "?".into(), check_label),
            Target::Index(o) => table
                .index(o)
                .map_or_else(|| "?".into(), |index| index.name.clone()),
        }
    }

    // -- Draft objects (for rendering cells) ---------------------------------

    /// The draft object, or the baseline object when it was removed.
    pub fn column(&self, origin: Origin) -> Option<&ColumnModel> {
        self.draft
            .table
            .column(origin)
            .or_else(|| self.baseline.table.column(origin))
    }
    pub fn key(&self, origin: Origin) -> Option<&KeyModel> {
        self.draft
            .table
            .key(origin)
            .or_else(|| self.baseline.table.key(origin))
    }
    pub fn foreign_key(&self, origin: Origin) -> Option<&ForeignKeyModel> {
        self.draft
            .table
            .foreign_key(origin)
            .or_else(|| self.baseline.table.foreign_key(origin))
    }
    pub fn check(&self, origin: Origin) -> Option<&CheckModel> {
        self.draft
            .table
            .check(origin)
            .or_else(|| self.baseline.table.check(origin))
    }
    pub fn index(&self, origin: Origin) -> Option<&IndexModel> {
        self.draft
            .table
            .index(origin)
            .or_else(|| self.baseline.table.index(origin))
    }
    /// Column names of `origins`, resolved where the object lives.
    pub fn column_names(&self, target: Target, origins: &[Origin]) -> String {
        let table = if self.in_draft(target) {
            &self.draft.table
        } else {
            &self.baseline.table
        };
        origins_text(origins, table)
    }
    pub fn members_text(&self, target: Target, index: &IndexModel) -> String {
        let table = if self.in_draft(target) {
            &self.draft.table
        } else {
            &self.baseline.table
        };
        members_text(&index.members, table)
    }
    /// Position of a column in the draft (1-based), for the `#` cell.
    pub fn position(&self, origin: Origin) -> Option<usize> {
        self.view.positions.get(&origin).copied()
    }

    // -- Inputs --------------------------------------------------------------

    /// The text inputs the inspector shows for `target`, in order. Removed
    /// objects show none: they can only be restored.
    pub fn inputs(&self, target: Target) -> Vec<Input> {
        if !self.in_draft(target) {
            return Vec::new();
        }
        let caps = self.caps;
        let mut inputs = vec![Input::Name];
        match target {
            Target::Table => {
                if caps.comments {
                    inputs.push(Input::Comment);
                }
                if caps.mysql_options {
                    inputs.extend([
                        Input::Engine,
                        Input::Charset,
                        Input::Collation,
                        Input::RowFormat,
                    ]);
                }
            }
            Target::Column(_) => {
                inputs.extend([
                    Input::DataType,
                    Input::Default,
                    Input::Generated,
                    Input::Collation,
                ]);
                if caps.charset {
                    inputs.push(Input::Charset);
                }
                if caps.on_update {
                    inputs.push(Input::OnUpdate);
                }
                if caps.comments {
                    inputs.push(Input::Comment);
                }
            }
            Target::PrimaryKey(_) | Target::Unique(_) => inputs.push(Input::Columns),
            Target::ForeignKey(_) => {
                inputs.push(Input::Columns);
                if caps.referenced_schema {
                    inputs.push(Input::ReferencedSchema);
                }
                inputs.extend([Input::ReferencedTable, Input::ReferencedColumns]);
            }
            Target::Check(_) => inputs.push(Input::Expression),
            Target::Index(_) => {
                inputs.push(Input::Members);
                if caps.include {
                    inputs.push(Input::Include);
                }
                if caps.index_method {
                    inputs.push(Input::Method);
                }
                if caps.predicate {
                    inputs.push(Input::Predicate);
                }
            }
        }
        inputs
    }

    /// Whether an input currently accepts text (a default needs a kind, a
    /// generated expression needs the column to be generated).
    pub fn input_enabled(&self, target: Target, input: Input) -> bool {
        match (target, input) {
            (Target::Column(o), Input::Default) => self
                .draft
                .table
                .column(o)
                .is_some_and(|c| !c.default.is_none()),
            (Target::Column(o), Input::Generated) => self
                .draft
                .table
                .column(o)
                .is_some_and(|c| c.generated.is_some()),
            _ => self.in_draft(target),
        }
    }

    /// The current text of an input.
    pub fn input_value(&self, target: Target, input: Input) -> String {
        let table = &self.draft.table;
        let text = |value: &Option<String>| value.clone().unwrap_or_default();
        match target {
            Target::Table => match input {
                Input::Name => table.name.clone(),
                Input::Comment => text(&table.comment),
                Input::Engine => text(&table.options.engine),
                Input::Charset => text(&table.options.charset),
                Input::Collation => text(&table.options.collation),
                Input::RowFormat => text(&table.options.row_format),
                _ => String::new(),
            },
            Target::Column(o) => {
                let Some(column) = table.column(o) else {
                    return String::new();
                };
                match input {
                    Input::Name => column.name.clone(),
                    Input::DataType => column.data_type.clone(),
                    Input::Default => match &column.default {
                        DefaultValue::None => String::new(),
                        DefaultValue::Literal(text) => text.clone(),
                        DefaultValue::Expression(sql) => sql.0.clone(),
                    },
                    Input::Generated => column
                        .generated
                        .as_ref()
                        .map(|g| g.expression.0.clone())
                        .unwrap_or_default(),
                    Input::Collation => text(&column.collation),
                    Input::Charset => text(&column.extra.charset),
                    Input::OnUpdate => column
                        .extra
                        .on_update
                        .as_ref()
                        .map(|sql| sql.0.clone())
                        .unwrap_or_default(),
                    Input::Comment => text(&column.comment),
                    _ => String::new(),
                }
            }
            Target::PrimaryKey(o) | Target::Unique(o) => {
                let Some(key) = table.key(o) else {
                    return String::new();
                };
                match input {
                    Input::Name => text(&key.name),
                    Input::Columns => origins_text(&key.columns, table),
                    _ => String::new(),
                }
            }
            Target::ForeignKey(o) => {
                let Some(key) = table.foreign_key(o) else {
                    return String::new();
                };
                match input {
                    Input::Name => text(&key.name),
                    Input::Columns => origins_text(&key.columns, table),
                    Input::ReferencedSchema => text(&key.referenced_schema),
                    Input::ReferencedTable => key.referenced_table.clone(),
                    Input::ReferencedColumns => key.referenced_columns.join(", "),
                    _ => String::new(),
                }
            }
            Target::Check(o) => {
                let Some(check) = table.check(o) else {
                    return String::new();
                };
                match input {
                    Input::Name => text(&check.name),
                    Input::Expression => check.expression.0.clone(),
                    _ => String::new(),
                }
            }
            Target::Index(o) => {
                let Some(index) = table.index(o) else {
                    return String::new();
                };
                match input {
                    Input::Name => index.name.clone(),
                    Input::Members => members_text(&index.members, table),
                    Input::Include => origins_text(&index.include, table),
                    Input::Method => text(&index.method),
                    Input::Predicate => index
                        .predicate
                        .as_ref()
                        .map(|sql| sql.0.clone())
                        .unwrap_or_default(),
                    _ => String::new(),
                }
            }
        }
    }

    /// Applies one input's text. `Ok(true)` when the draft changed. A text
    /// that cannot be parsed (an unknown column name) leaves the draft as it
    /// was and returns the reason for the inline error.
    pub fn set_input(&mut self, target: Target, input: Input, text: &str) -> Result<bool, String> {
        if text.len() > INPUT_BYTES {
            return Err(format!(
                "{} is longer than {} KiB",
                input.label(),
                INPUT_BYTES / 1024
            ));
        }
        if !self.in_draft(target) {
            return Err("Restore this object before editing it".into());
        }
        let engine = self.caps.engine;
        let before = self.draft.table.clone();
        // Parse member lists against the draft before taking a mutable borrow.
        let origins = match input {
            Input::Columns | Input::Include => {
                Some(names_to_origins(text, &self.draft.table, engine)?)
            }
            _ => None,
        };
        let members = match (target, input) {
            (Target::Index(o), Input::Members) => {
                let existing = self
                    .draft
                    .table
                    .index(o)
                    .map(|i| i.members.clone())
                    .unwrap_or_default();
                Some(parse_members(text, &self.draft.table, engine, &existing)?)
            }
            _ => None,
        };
        let table = &mut self.draft.table;
        match target {
            Target::Table => match input {
                Input::Name => table.name = text.to_owned(),
                Input::Comment => table.comment = optional_raw(text),
                Input::Engine => table.options.engine = optional(text),
                Input::Charset => table.options.charset = optional(text),
                Input::Collation => table.options.collation = optional(text),
                Input::RowFormat => table.options.row_format = optional(text),
                _ => return Err(format!("{} does not apply to the table", input.label())),
            },
            Target::Column(o) => {
                let Some(column) = table.column_mut(o) else {
                    return Ok(false);
                };
                match input {
                    Input::Name => column.name = text.to_owned(),
                    Input::DataType => column.data_type = text.trim().to_owned(),
                    Input::Default => {
                        column.default = match &column.default {
                            DefaultValue::Expression(_) => {
                                DefaultValue::Expression(SqlFragment::new(text))
                            }
                            // Typing into an unset default makes it a literal.
                            DefaultValue::Literal(_) | DefaultValue::None => {
                                DefaultValue::Literal(text.to_owned())
                            }
                        }
                    }
                    Input::Generated => match &mut column.generated {
                        Some(generated) => generated.expression = SqlFragment::new(text),
                        None => return Err("Make the column generated first".into()),
                    },
                    Input::Collation => column.collation = optional(text),
                    Input::Charset => column.extra.charset = optional(text),
                    Input::OnUpdate => {
                        column.extra.on_update = optional(text).map(SqlFragment::new)
                    }
                    Input::Comment => column.comment = optional_raw(text),
                    _ => return Err(format!("{} does not apply to a column", input.label())),
                }
            }
            Target::PrimaryKey(o) | Target::Unique(o) => {
                let key = if matches!(target, Target::PrimaryKey(_)) {
                    table.primary_key.as_mut().filter(|k| k.origin == o)
                } else {
                    table.uniques.iter_mut().find(|k| k.origin == o)
                };
                let Some(key) = key else {
                    return Ok(false);
                };
                match input {
                    Input::Name => key.name = optional(text),
                    Input::Columns => key.columns = origins.unwrap_or_default(),
                    _ => return Err(format!("{} does not apply to a key", input.label())),
                }
            }
            Target::ForeignKey(o) => {
                let Some(key) = table.foreign_keys.iter_mut().find(|k| k.origin == o) else {
                    return Ok(false);
                };
                match input {
                    Input::Name => key.name = optional(text),
                    Input::Columns => key.columns = origins.unwrap_or_default(),
                    Input::ReferencedSchema => {
                        key.referenced_schema = optional(text).map(|s| unquote(&s).to_owned())
                    }
                    Input::ReferencedTable => {
                        key.referenced_table = unquote(text.trim()).to_owned()
                    }
                    Input::ReferencedColumns => {
                        key.referenced_columns = split_list(text)
                            .iter()
                            .map(|name| unquote(name).to_owned())
                            .collect()
                    }
                    _ => {
                        return Err(format!("{} does not apply to a foreign key", input.label()));
                    }
                }
            }
            Target::Check(o) => {
                let Some(check) = table.checks.iter_mut().find(|c| c.origin == o) else {
                    return Ok(false);
                };
                match input {
                    Input::Name => check.name = optional(text),
                    Input::Expression => check.expression = SqlFragment::new(text),
                    _ => return Err(format!("{} does not apply to a check", input.label())),
                }
            }
            Target::Index(o) => {
                let Some(index) = table.indexes.iter_mut().find(|i| i.origin == o) else {
                    return Ok(false);
                };
                match input {
                    Input::Name => index.name = text.to_owned(),
                    Input::Members => index.members = members.unwrap_or_default(),
                    Input::Include => index.include = origins.unwrap_or_default(),
                    Input::Method => index.method = optional(text),
                    Input::Predicate => index.predicate = optional(text).map(SqlFragment::new),
                    _ => return Err(format!("{} does not apply to an index", input.label())),
                }
            }
        }
        let changed = self.draft.table != before;
        if changed {
            self.recompute();
        }
        Ok(changed)
    }

    // -- Choices -------------------------------------------------------------

    pub fn default_kind(&self, origin: Origin) -> DefaultKind {
        match self.column(origin).map(|c| &c.default) {
            Some(DefaultValue::Literal(_)) => DefaultKind::Literal,
            Some(DefaultValue::Expression(_)) => DefaultKind::Expression,
            _ => DefaultKind::None,
        }
    }

    pub fn generated_kind(&self, origin: Origin) -> GeneratedKind {
        match self.column(origin).and_then(|c| c.generated.as_ref()) {
            None => GeneratedKind::None,
            Some(generated) if generated.stored => GeneratedKind::Stored,
            Some(_) => GeneratedKind::Virtual,
        }
    }

    /// Applies a checkbox or chip. Returns true when the draft changed.
    pub fn set_choice(&mut self, target: Target, choice: Choice) -> bool {
        if !self.in_draft(target) {
            return false;
        }
        let before = self.draft.table.clone();
        let table = &mut self.draft.table;
        match (target, choice) {
            (Target::Table, Choice::Strict(value)) => table.options.strict = value,
            (Target::Table, Choice::WithoutRowid(value)) => table.options.without_rowid = value,
            (Target::Column(o), choice) => {
                let Some(column) = table.column_mut(o) else {
                    return false;
                };
                match choice {
                    Choice::Nullable(value) => column.nullable = value,
                    Choice::DefaultKind(kind) => {
                        let text = match &column.default {
                            DefaultValue::None => String::new(),
                            DefaultValue::Literal(text) => text.clone(),
                            DefaultValue::Expression(sql) => sql.0.clone(),
                        };
                        column.default = match kind {
                            DefaultKind::None => DefaultValue::None,
                            DefaultKind::Literal => DefaultValue::Literal(text),
                            DefaultKind::Expression => DefaultValue::Expression(SqlFragment(text)),
                        };
                    }
                    Choice::GeneratedKind(kind) => {
                        let expression = column
                            .generated
                            .as_ref()
                            .map(|g| g.expression.clone())
                            .unwrap_or_else(|| SqlFragment::new(""));
                        column.generated = match kind {
                            GeneratedKind::None => None,
                            GeneratedKind::Virtual => Some(Generated {
                                expression,
                                stored: false,
                            }),
                            GeneratedKind::Stored => Some(Generated {
                                expression,
                                stored: true,
                            }),
                        };
                    }
                    Choice::Identity(identity) => {
                        column.identity = identity;
                        // PostgreSQL identity columns are NOT NULL.
                        if matches!(identity, Identity::Always | Identity::ByDefault) {
                            column.nullable = false;
                        }
                    }
                    Choice::Invisible(value) => column.extra.invisible = value,
                    _ => return false,
                }
            }
            (Target::Index(o), Choice::Unique(value)) => {
                let Some(index) = table.indexes.iter_mut().find(|i| i.origin == o) else {
                    return false;
                };
                index.unique = value;
            }
            (Target::Check(o), Choice::Enforced(value)) => {
                let Some(check) = table.checks.iter_mut().find(|c| c.origin == o) else {
                    return false;
                };
                check.enforced = value;
            }
            (Target::ForeignKey(o), choice) => {
                let Some(key) = table.foreign_keys.iter_mut().find(|k| k.origin == o) else {
                    return false;
                };
                match choice {
                    Choice::OnUpdate(action) => key.on_update = action,
                    Choice::OnDelete(action) => key.on_delete = action,
                    Choice::Deferrable(value) => key.deferrable = value,
                    _ => return false,
                }
            }
            _ => return false,
        }
        let changed = self.draft.table != before;
        if changed {
            self.recompute();
        }
        changed
    }

    // -- Per-field undo ------------------------------------------------------

    /// Reverts one attribute of one object to its baseline value. New and
    /// removed objects have no baseline attribute to revert to.
    pub fn revert(&mut self, target: Target, attr: Attr) -> bool {
        if !self.in_draft(target) || !self.in_baseline(target) {
            return false;
        }
        let old = self.baseline.table.clone();
        let before = self.draft.table.clone();
        let table = &mut self.draft.table;
        match target {
            Target::Table => match attr {
                Attr::Name => table.name = old.name.clone(),
                Attr::Field(Field::Comment) => table.comment = old.comment.clone(),
                Attr::Field(Field::Strict) => table.options.strict = old.options.strict,
                Attr::Field(Field::WithoutRowid) => {
                    table.options.without_rowid = old.options.without_rowid
                }
                Attr::Field(Field::Engine) => table.options.engine = old.options.engine.clone(),
                Attr::Field(Field::Charset) => table.options.charset = old.options.charset.clone(),
                Attr::Field(Field::Collation) => {
                    table.options.collation = old.options.collation.clone()
                }
                Attr::Field(Field::RowFormat) => {
                    table.options.row_format = old.options.row_format.clone()
                }
                Attr::Field(_) => return false,
            },
            Target::Column(o) => {
                let Some(original) = old.column(o) else {
                    return false;
                };
                if attr == Attr::Field(Field::Position) {
                    let position = old.columns.iter().position(|c| c.origin == o).unwrap_or(0);
                    self.draft.move_column(o, position);
                } else {
                    let Some(column) = table.column_mut(o) else {
                        return false;
                    };
                    match attr {
                        Attr::Name => column.name = original.name.clone(),
                        Attr::Field(Field::DataType) => {
                            column.data_type = original.data_type.clone()
                        }
                        Attr::Field(Field::Nullable) => column.nullable = original.nullable,
                        Attr::Field(Field::Default) => column.default = original.default.clone(),
                        Attr::Field(Field::Generated) => {
                            column.generated = original.generated.clone()
                        }
                        Attr::Field(Field::Identity) => column.identity = original.identity,
                        Attr::Field(Field::Collation) => {
                            column.collation = original.collation.clone()
                        }
                        Attr::Field(Field::Comment) => column.comment = original.comment.clone(),
                        Attr::Field(Field::Charset) => {
                            column.extra.charset = original.extra.charset.clone()
                        }
                        Attr::Field(Field::OnUpdate) => {
                            column.extra.on_update = original.extra.on_update.clone()
                        }
                        Attr::Field(Field::Invisible) => {
                            column.extra.invisible = original.extra.invisible
                        }
                        Attr::Field(_) => return false,
                    }
                }
            }
            Target::PrimaryKey(o) | Target::Unique(o) => {
                let Some(original) = old.key(o).cloned() else {
                    return false;
                };
                let key = if matches!(target, Target::PrimaryKey(_)) {
                    table.primary_key.as_mut().filter(|k| k.origin == o)
                } else {
                    table.uniques.iter_mut().find(|k| k.origin == o)
                };
                let Some(key) = key else {
                    return false;
                };
                match attr {
                    Attr::Name => key.name = original.name,
                    Attr::Field(Field::Columns) => key.columns = original.columns,
                    Attr::Field(_) => return false,
                }
            }
            Target::ForeignKey(o) => {
                let Some(original) = old.foreign_key(o).cloned() else {
                    return false;
                };
                let Some(key) = table.foreign_keys.iter_mut().find(|k| k.origin == o) else {
                    return false;
                };
                match attr {
                    Attr::Name => key.name = original.name,
                    Attr::Field(Field::Columns) => key.columns = original.columns,
                    Attr::Field(Field::Reference) => {
                        key.referenced_schema = original.referenced_schema;
                        key.referenced_table = original.referenced_table;
                        key.referenced_columns = original.referenced_columns;
                    }
                    Attr::Field(Field::OnUpdateAction) => key.on_update = original.on_update,
                    Attr::Field(Field::OnDeleteAction) => key.on_delete = original.on_delete,
                    Attr::Field(Field::Deferrable) => key.deferrable = original.deferrable,
                    Attr::Field(_) => return false,
                }
            }
            Target::Check(o) => {
                let Some(original) = old.check(o).cloned() else {
                    return false;
                };
                let Some(check) = table.checks.iter_mut().find(|c| c.origin == o) else {
                    return false;
                };
                match attr {
                    Attr::Name => check.name = original.name,
                    Attr::Field(Field::Expression) => check.expression = original.expression,
                    Attr::Field(Field::Enforced) => check.enforced = original.enforced,
                    Attr::Field(_) => return false,
                }
            }
            Target::Index(o) => {
                let Some(original) = old.index(o).cloned() else {
                    return false;
                };
                let Some(index) = table.indexes.iter_mut().find(|i| i.origin == o) else {
                    return false;
                };
                match attr {
                    Attr::Name => index.name = original.name,
                    Attr::Field(Field::Unique) => index.unique = original.unique,
                    Attr::Field(Field::Method) => index.method = original.method,
                    Attr::Field(Field::Members) => index.members = original.members,
                    Attr::Field(Field::Include) => index.include = original.include,
                    Attr::Field(Field::Predicate) => index.predicate = original.predicate,
                    Attr::Field(_) => return false,
                }
            }
        }
        let changed = self.draft.table != before;
        if changed {
            self.recompute();
        }
        changed
    }

    /// Reverts every attribute of an existing object (its row's Undo). A new
    /// object is removed instead; a removed one is restored.
    pub fn revert_object(&mut self, target: Target) -> Result<bool, String> {
        if !self.in_baseline(target) {
            return Ok(self.remove(target));
        }
        if !self.in_draft(target) {
            return self.restore_object(target).map(|()| true);
        }
        let old = self.baseline.table.clone();
        let before = self.draft.table.clone();
        match target {
            Target::Table => {
                let table = &mut self.draft.table;
                table.name = old.name;
                table.comment = old.comment;
                table.options = old.options;
            }
            Target::Column(o) => {
                self.draft.revert_column(&self.baseline, o);
                let position = old.columns.iter().position(|c| c.origin == o).unwrap_or(0);
                self.draft.move_column(o, position);
            }
            Target::PrimaryKey(_) => self.draft.table.primary_key = old.primary_key,
            Target::Unique(o) => {
                replace(&mut self.draft.table.uniques, &old.uniques, o, |k| k.origin)
            }
            Target::ForeignKey(o) => replace(
                &mut self.draft.table.foreign_keys,
                &old.foreign_keys,
                o,
                |k| k.origin,
            ),
            Target::Check(o) => replace(&mut self.draft.table.checks, &old.checks, o, |c| c.origin),
            Target::Index(o) => {
                replace(&mut self.draft.table.indexes, &old.indexes, o, |i| i.origin)
            }
        }
        let changed = self.draft.table != before;
        if changed {
            self.recompute();
        }
        Ok(changed)
    }

    // -- Add, remove, restore, move -------------------------------------------

    fn constraint_names(&self) -> Vec<String> {
        let table = &self.draft.table;
        table
            .primary_key
            .iter()
            .chain(table.uniques.iter())
            .filter_map(|k| k.name.clone())
            .chain(table.foreign_keys.iter().filter_map(|k| k.name.clone()))
            .chain(table.checks.iter().filter_map(|c| c.name.clone()))
            .collect()
    }

    fn unique_name(&self, base: &str, taken: &[String]) -> String {
        let engine = self.caps.engine;
        (1..)
            .map(|n| format!("{base}{n}"))
            .find(|candidate| !taken.iter().any(|name| same_name(engine, name, candidate)))
            .unwrap_or_else(|| base.to_owned())
    }

    pub fn add_column(&mut self) -> Target {
        let taken: Vec<String> = self
            .draft
            .table
            .columns
            .iter()
            .map(|c| c.name.clone())
            .collect();
        let name = self.unique_name("column_", &taken);
        let origin = self.draft.add_column(name, self.caps.default_type());
        self.recompute();
        Target::Column(origin)
    }

    pub fn add_primary_key(&mut self) -> Result<Target, String> {
        if self.draft.table.primary_key.is_some() {
            return Err("The table already has a primary key".into());
        }
        let origin = self.draft.new_origin();
        let name = (self.caps.engine == SchemaEngine::Postgres)
            .then(|| format!("{}_pkey", self.draft.table.name));
        self.draft.table.primary_key = Some(KeyModel {
            origin,
            name,
            columns: Vec::new(),
        });
        self.recompute();
        Ok(Target::PrimaryKey(origin))
    }

    pub fn add_unique(&mut self) -> Target {
        let name = self.unique_name(
            &format!("{}_key", self.draft.table.name),
            &self.constraint_names(),
        );
        let origin = self.draft.new_origin();
        self.draft.table.uniques.push(KeyModel {
            origin,
            name: Some(name),
            columns: Vec::new(),
        });
        self.recompute();
        Target::Unique(origin)
    }

    pub fn add_foreign_key(&mut self) -> Target {
        let name = self.unique_name(
            &format!("{}_fkey", self.draft.table.name),
            &self.constraint_names(),
        );
        let origin = self.draft.new_origin();
        self.draft.table.foreign_keys.push(ForeignKeyModel {
            origin,
            name: Some(name),
            columns: Vec::new(),
            referenced_schema: None,
            referenced_table: String::new(),
            referenced_columns: Vec::new(),
            on_update: ReferentialAction::NoAction,
            on_delete: ReferentialAction::NoAction,
            deferrable: Deferrable::Never,
        });
        self.recompute();
        Target::ForeignKey(origin)
    }

    pub fn add_check(&mut self) -> Target {
        let name = self.unique_name(
            &format!("{}_check", self.draft.table.name),
            &self.constraint_names(),
        );
        let origin = self.draft.new_origin();
        self.draft.table.checks.push(CheckModel {
            origin,
            name: Some(name),
            expression: SqlFragment::new(""),
            enforced: true,
        });
        self.recompute();
        Target::Check(origin)
    }

    pub fn add_index(&mut self) -> Target {
        let taken: Vec<String> = self
            .draft
            .table
            .indexes
            .iter()
            .map(|i| i.name.clone())
            .collect();
        let name = self.unique_name(&format!("{}_idx", self.draft.table.name), &taken);
        let origin = self.draft.new_origin();
        self.draft.table.indexes.push(IndexModel {
            origin,
            name,
            unique: false,
            method: None,
            members: Vec::new(),
            include: Vec::new(),
            predicate: None,
        });
        self.recompute();
        Target::Index(origin)
    }

    /// Removes an object. Removing a column also removes the keys, indexes
    /// and foreign keys that use it, as `DROP COLUMN` does; they stay visible
    /// as removed rows and can be restored on their own.
    pub fn remove(&mut self, target: Target) -> bool {
        let changed = match target {
            Target::Table => false,
            Target::Column(o) => self.draft.remove_column(o),
            Target::PrimaryKey(o) => {
                let table = &mut self.draft.table;
                let present = table.primary_key.as_ref().is_some_and(|k| k.origin == o);
                if present {
                    table.primary_key = None;
                }
                present
            }
            Target::Unique(o) => retain(&mut self.draft.table.uniques, o, |k| k.origin),
            Target::ForeignKey(o) => retain(&mut self.draft.table.foreign_keys, o, |k| k.origin),
            Target::Check(o) => retain(&mut self.draft.table.checks, o, |c| c.origin),
            Target::Index(o) => retain(&mut self.draft.table.indexes, o, |i| i.origin),
        };
        if changed {
            self.recompute();
        }
        changed
    }

    /// Puts a removed baseline object back, unchanged.
    pub fn restore_object(&mut self, target: Target) -> Result<(), String> {
        if self.in_draft(target) {
            return Ok(());
        }
        let old = &self.baseline.table;
        let table = &mut self.draft.table;
        match target {
            Target::Table => {}
            Target::Column(o) => {
                let Some(position) = old.columns.iter().position(|c| c.origin == o) else {
                    return Err("The column is not part of the captured table".into());
                };
                let column = old.columns[position].clone();
                let position = position.min(table.columns.len());
                table.columns.insert(position, column);
            }
            Target::PrimaryKey(o) => {
                if table.primary_key.is_some() {
                    return Err("Remove the new primary key before restoring this one".into());
                }
                table.primary_key = old.primary_key.clone().filter(|k| k.origin == o);
            }
            Target::Unique(o) => push_from(&mut table.uniques, &old.uniques, o, |k| k.origin),
            Target::ForeignKey(o) => {
                push_from(&mut table.foreign_keys, &old.foreign_keys, o, |k| k.origin)
            }
            Target::Check(o) => push_from(&mut table.checks, &old.checks, o, |c| c.origin),
            Target::Index(o) => push_from(&mut table.indexes, &old.indexes, o, |i| i.origin),
        }
        self.recompute();
        Ok(())
    }

    /// Moves a column one place up or down. Only offered with `reorder`.
    pub fn move_column(&mut self, origin: Origin, up: bool) -> bool {
        if !self.caps.reorder {
            return false;
        }
        let Some(index) = self
            .draft
            .table
            .columns
            .iter()
            .position(|c| c.origin == origin)
        else {
            return false;
        };
        let to = if up {
            match index.checked_sub(1) {
                Some(to) => to,
                None => return false,
            }
        } else if index + 1 < self.draft.table.columns.len() {
            index + 1
        } else {
            return false;
        };
        self.draft.move_column(origin, to);
        self.recompute();
        true
    }

    /// True when `attr` of `target` differs from the baseline (the field's
    /// Undo button is shown).
    pub fn changed(&self, target: Target, attr: Attr) -> bool {
        if let Some(row) = self.cached_row(target) {
            return row.delta(attr).is_some();
        }
        self.changes_for(target)
            .any(|change| match (&change.kind, attr) {
                (ChangeKind::Renamed { .. }, Attr::Name) => true,
                (ChangeKind::Changed { field, .. }, Attr::Field(wanted)) => *field == wanted,
                _ => false,
            })
    }

    /// The baseline text of a changed attribute, for "was …" hints.
    pub fn before(&self, target: Target, attr: Attr) -> Option<String> {
        if let Some(row) = self.cached_row(target) {
            return row.delta(attr).map(|delta| delta.before.clone());
        }
        self.changes_for(target)
            .find_map(|change| match (&change.kind, attr) {
                (ChangeKind::Renamed { from, .. }, Attr::Name) => Some(from.clone()),
                (ChangeKind::Changed { field, before, .. }, Attr::Field(wanted))
                    if *field == wanted =>
                {
                    Some(before.clone())
                }
                _ => None,
            })
    }
}

fn retain<T>(items: &mut Vec<T>, origin: Origin, of: impl Fn(&T) -> Origin) -> bool {
    let before = items.len();
    items.retain(|item| of(item) != origin);
    items.len() != before
}

fn push_from<T: Clone>(items: &mut Vec<T>, old: &[T], origin: Origin, of: impl Fn(&T) -> Origin) {
    if let Some(item) = old.iter().find(|item| of(item) == origin) {
        items.push(item.clone());
    }
}

fn replace<T: Clone>(items: &mut [T], old: &[T], origin: Origin, of: impl Fn(&T) -> Origin) {
    if let (Some(slot), Some(original)) = (
        items.iter_mut().find(|item| of(item) == origin),
        old.iter().find(|item| of(item) == origin),
    ) {
        *slot = original.clone();
    }
}
