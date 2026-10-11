//! Draft mode rendering: the toolbar with Discard and `Review N ⌘S`, the
//! tinted section tables (added `ok_fill`, changed `edited_fill` with old →
//! new, removed `deleted_fill` struck through) and the inspector with one
//! Undo per changed field.
use super::*;
use crate::{
    schema_edit_model::{
        ACTIONS, DEFERRABLE, DefaultKind, GeneratedKind, Row, RowState, action_label,
        identity_label,
    },
    structure_table::{self as table, ColumnSpec, INSET, fill, fixed, mono},
    style,
};
use dbunk_lib::backend::schema_edit::{
    Field, Identity, describe_default, describe_generated, describe_identity,
};
use gpui::{AnyElement, Div, KeyDownEvent, Role, SharedString, Stateful, div, prelude::*, px};

const INSPECTOR: f32 = 320.;
const ROW_ACTIONS: f32 = 96.;

const COLUMNS: &[ColumnSpec] = &[
    mono(fixed("#", 40.)),
    fill("Name", 140.),
    mono(fill("Type", 120.)),
    mono(fill("Default", 110.)),
    fixed("Nullable", 80.),
    fill("Attributes", 140.),
];
const KEYS: &[ColumnSpec] = &[
    fixed("Kind", 80.),
    fill("Name", 160.),
    mono(fill("Columns", 200.)),
];
const FOREIGN_KEYS: &[ColumnSpec] = &[
    fill("Name", 140.),
    mono(fill("Columns", 110.)),
    mono(fill("References", 170.)),
    fixed("On update", 96.),
    fixed("On delete", 96.),
];
const CHECKS: &[ColumnSpec] = &[fill("Name", 160.), mono(fill("Expression", 240.))];
const INDEXES: &[ColumnSpec] = &[
    fill("Name", 160.),
    mono(fill("Members", 180.)),
    fixed("Unique", 64.),
    fixed("Method", 80.),
];
const TABLE: &[ColumnSpec] = &[
    fill("Name", 160.),
    fill("Comment", 200.),
    fill("Options", 160.),
];
const ACTIONS_SPEC: ColumnSpec = fixed("", ROW_ACTIONS);

fn specs(group: ChangeGroup) -> &'static [ColumnSpec] {
    match group {
        ChangeGroup::Columns => COLUMNS,
        ChangeGroup::Keys => KEYS,
        ChangeGroup::ForeignKeys => FOREIGN_KEYS,
        ChangeGroup::Checks => CHECKS,
        ChangeGroup::Indexes => INDEXES,
        ChangeGroup::Table => TABLE,
    }
}

/// One display cell: the draft value and, when changed, the baseline value.
struct Cell {
    text: String,
    before: Option<String>,
    changed: bool,
}

impl Cell {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            before: None,
            changed: false,
        }
    }
    /// The value with its delta's baseline text, when the row has one.
    fn delta(text: impl Into<String>, row: &Row, attr: Attr) -> Self {
        let delta = row.delta(attr);
        Self {
            text: text.into(),
            before: delta.map(|delta| delta.before.clone()),
            changed: delta.is_some(),
        }
    }
    fn flagged(text: impl Into<String>, changed: bool) -> Self {
        Self {
            text: text.into(),
            before: None,
            changed,
        }
    }
}

fn clip(text: &str) -> String {
    table::clip(text)
}

fn render_cell(spec: &ColumnSpec, cell: Cell, removed: bool) -> AnyElement {
    let base = div()
        .h_full()
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(4.))
        .overflow_hidden()
        .whitespace_nowrap()
        .flex_basis(px(spec.width))
        .flex_shrink_1()
        .min_w(px(spec.width.min(32.)))
        .when(spec.fill, |cell| cell.flex_grow_1())
        .when(!spec.fill, |cell| cell.flex_grow_0())
        .when(spec.mono, |cell| cell.font_family(style::MONO));
    let text = clip(&cell.text);
    if removed {
        return base
            .text_color(style::faint())
            .child(div().min_w_0().truncate().line_through().child(text))
            .into_any_element();
    }
    match cell.before {
        Some(before) => base
            .child(
                div()
                    .flex_shrink_1()
                    .min_w_0()
                    .truncate()
                    .text_color(style::faint())
                    .line_through()
                    .child(clip(&before)),
            )
            .child(div().flex_none().text_color(style::faint()).child("→"))
            .child(
                div()
                    .flex_shrink_1()
                    .min_w_0()
                    .truncate()
                    .text_color(style::warn())
                    .child(text),
            )
            .into_any_element(),
        None => base
            .when(cell.changed, |cell| cell.text_color(style::warn()))
            .when(cell.text.is_empty(), |cell| {
                cell.text_color(style::faint()).child("—")
            })
            .when(!cell.text.is_empty(), |cell| {
                cell.child(div().min_w_0().truncate().child(text))
            })
            .into_any_element(),
    }
}

fn tint(state: RowState) -> gpui::Rgba {
    match state {
        RowState::Unchanged => style::bg(),
        RowState::Added => style::ok_fill(),
        RowState::Changed => style::edited_fill(),
        RowState::Removed => style::deleted_fill(),
    }
}

fn state_label(state: RowState) -> &'static str {
    match state {
        RowState::Unchanged => "unchanged",
        RowState::Added => "added",
        RowState::Changed => "changed",
        RowState::Removed => "removed",
    }
}

fn yes(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

impl SchemaEditView {
    fn cells(&self, row: &Row) -> Vec<Cell> {
        let Some(model) = &self.model else {
            return Vec::new();
        };
        let field = Attr::Field;
        match row.target {
            Target::Table => {
                let table = &model.draft().table;
                let options = &table.options;
                let mut parts = Vec::new();
                if options.strict {
                    parts.push("STRICT".to_owned());
                }
                if options.without_rowid {
                    parts.push("WITHOUT ROWID".to_owned());
                }
                for (label, value) in [
                    ("engine", &options.engine),
                    ("charset", &options.charset),
                    ("collation", &options.collation),
                    ("row format", &options.row_format),
                ] {
                    if let Some(value) = value {
                        parts.push(format!("{label} {value}"));
                    }
                }
                let options_changed = row.deltas.iter().any(|delta| {
                    !matches!(delta.attr, Attr::Name | Attr::Field(Field::Comment))
                });
                vec![
                    Cell::delta(table.name.clone(), row, Attr::Name),
                    Cell::delta(
                        table.comment.clone().unwrap_or_default(),
                        row,
                        field(Field::Comment),
                    ),
                    Cell::flagged(parts.join(", "), options_changed),
                ]
            }
            Target::Column(origin) => {
                let Some(column) = model.column(origin) else {
                    return Vec::new();
                };
                let mut attributes = Vec::new();
                if column.identity != Identity::None {
                    attributes.push(describe_identity(column.identity).to_owned());
                }
                if column.generated.is_some() {
                    attributes.push(describe_generated(&column.generated));
                }
                if let Some(collation) = &column.collation {
                    attributes.push(format!("collate {collation}"));
                }
                if let Some(charset) = &column.extra.charset {
                    attributes.push(format!("charset {charset}"));
                }
                if column.extra.invisible {
                    attributes.push("invisible".into());
                }
                if let Some(comment) = &column.comment {
                    attributes.push(format!("“{comment}”"));
                }
                let attributes_changed = row.deltas.iter().any(|delta| {
                    matches!(
                        delta.attr,
                        Attr::Field(
                            Field::Identity
                                | Field::Generated
                                | Field::Collation
                                | Field::Charset
                                | Field::Invisible
                                | Field::Comment
                                | Field::OnUpdate
                        )
                    )
                });
                let position = model
                    .position(origin)
                    .map_or_else(|| "—".to_owned(), |p| p.to_string());
                vec![
                    Cell::delta(position, row, field(Field::Position)),
                    Cell::delta(column.name.clone(), row, Attr::Name),
                    Cell::delta(column.data_type.clone(), row, field(Field::DataType)),
                    Cell::delta(
                        if column.default.is_none() {
                            String::new()
                        } else {
                            describe_default(&column.default)
                        },
                        row,
                        field(Field::Default),
                    ),
                    Cell::delta(
                        if column.nullable { "NULL" } else { "NOT NULL" },
                        row,
                        field(Field::Nullable),
                    ),
                    Cell::flagged(attributes.join(", "), attributes_changed),
                ]
            }
            Target::PrimaryKey(origin) | Target::Unique(origin) => {
                let Some(key) = model.key(origin) else {
                    return Vec::new();
                };
                vec![
                    Cell::plain(if matches!(row.target, Target::PrimaryKey(_)) {
                        "Primary"
                    } else {
                        "Unique"
                    }),
                    Cell::delta(key.name.clone().unwrap_or_default(), row, Attr::Name),
                    Cell::delta(
                        model.column_names(row.target, &key.columns),
                        row,
                        field(Field::Columns),
                    ),
                ]
            }
            Target::ForeignKey(origin) => {
                let Some(key) = model.foreign_key(origin) else {
                    return Vec::new();
                };
                let table = match &key.referenced_schema {
                    Some(schema) => format!("{schema}.{}", key.referenced_table),
                    None => key.referenced_table.clone(),
                };
                let reference = if key.referenced_columns.is_empty() {
                    table
                } else {
                    format!("{table}({})", key.referenced_columns.join(", "))
                };
                vec![
                    Cell::delta(key.name.clone().unwrap_or_default(), row, Attr::Name),
                    Cell::delta(
                        model.column_names(row.target, &key.columns),
                        row,
                        field(Field::Columns),
                    ),
                    Cell::delta(reference, row, field(Field::Reference)),
                    Cell::delta(action_label(key.on_update), row, field(Field::OnUpdateAction)),
                    Cell::delta(action_label(key.on_delete), row, field(Field::OnDeleteAction)),
                ]
            }
            Target::Check(origin) => {
                let Some(check) = model.check(origin) else {
                    return Vec::new();
                };
                vec![
                    Cell::delta(check.name.clone().unwrap_or_default(), row, Attr::Name),
                    Cell::delta(check.expression.0.clone(), row, field(Field::Expression)),
                ]
            }
            Target::Index(origin) => {
                let Some(index) = model.index(origin) else {
                    return Vec::new();
                };
                vec![
                    Cell::delta(index.name.clone(), row, Attr::Name),
                    Cell::delta(
                        model.members_text(row.target, index),
                        row,
                        field(Field::Members),
                    ),
                    Cell::delta(yes(index.unique), row, field(Field::Unique)),
                    Cell::delta(
                        index.method.clone().unwrap_or_default(),
                        row,
                        field(Field::Method),
                    ),
                ]
            }
        }
    }

    fn icon(
        &self,
        id: (&'static str, usize),
        label: &'static str,
        icon: &'static str,
        enabled: bool,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        ui::icon_button(id, label, icon, enabled)
            .tab_index(0)
            .when(enabled, |button| {
                button.on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    action(this, window, cx);
                }))
            })
    }

    fn row_actions(&self, number: usize, row: &Row, cx: &Context<Self>) -> AnyElement {
        let editable = self.can_edit();
        let target = row.target;
        let reorder = self.model.as_ref().is_some_and(|m| m.caps().reorder)
            && matches!(target, Target::Column(_))
            && row.state != RowState::Removed;
        let mut actions = div()
            .h_full()
            .flex_none()
            .w(px(ROW_ACTIONS))
            .px(px(4.))
            .flex()
            .items_center()
            .justify_end()
            .gap(px(2.));
        if row.issues > 0 {
            actions = actions.child(
                div()
                    .id(("schema-edit-issue", number))
                    .flex_none()
                    .child(
                        gpui::svg()
                            .path("icons/warning.svg")
                            .size(px(style::ICON))
                            .text_color(style::bad_text()),
                    )
                    .tooltip(ui::tooltip(format!(
                        "{} problem{}; select the row to see them",
                        row.issues,
                        if row.issues == 1 { "" } else { "s" }
                    )))
                    .tooltip_show_delay(ui::tooltip_delay()),
            );
        }
        if reorder {
            actions = actions
                .child(self.icon(
                    ("schema-edit-up", number),
                    "Move column up",
                    "icons/arrow_up.svg",
                    editable,
                    cx,
                    move |this, _, cx| this.move_column(target, true, cx),
                ))
                .child(self.icon(
                    ("schema-edit-down", number),
                    "Move column down",
                    "icons/arrow_down.svg",
                    editable,
                    cx,
                    move |this, _, cx| this.move_column(target, false, cx),
                ));
        }
        match row.state {
            RowState::Removed => {
                actions = actions.child(self.icon(
                    ("schema-edit-restore", number),
                    "Restore",
                    "icons/rotate_ccw.svg",
                    editable,
                    cx,
                    move |this, _, cx| this.restore(target, cx),
                ));
            }
            state => {
                if state == RowState::Changed {
                    actions = actions.child(self.icon(
                        ("schema-edit-undo-row", number),
                        "Undo every change to this row",
                        "icons/undo.svg",
                        editable,
                        cx,
                        move |this, _, cx| this.undo_row(target, cx),
                    ));
                }
                if target != Target::Table {
                    actions = actions.child(self.icon(
                        ("schema-edit-remove", number),
                        "Remove",
                        "icons/trash.svg",
                        editable,
                        cx,
                        move |this, _, cx| this.remove(target, cx),
                    ));
                }
            }
        }
        actions.into_any_element()
    }

    fn section(&self, group: ChangeGroup, start: usize, cx: &Context<Self>) -> (AnyElement, usize) {
        let Some(model) = &self.model else {
            return (div().into_any_element(), 0);
        };
        let rows = model.rows(group);
        let count = rows.len();
        let editable = self.can_edit();
        let mut title = table::title_line(group.label(), count, None).child(ui::grow());
        let add = |id: &'static str, label: &'static str, primary: bool| {
            ui::tool_button(id, label, Some("icons/plus.svg"), editable, false)
                .tab_index(0)
                .when(editable, |button| {
                    button.on_click(cx.listener(move |this, _, _, cx| this.add(group, primary, cx)))
                })
        };
        title = match group {
            ChangeGroup::Columns => title.child(add("schema-edit-add-column", "Column", false)),
            ChangeGroup::Keys => {
                let has_primary = model.draft().table.primary_key.is_some();
                title
                    .when(!has_primary, |title| {
                        title.child(add("schema-edit-add-primary", "Primary key", true))
                    })
                    .child(add("schema-edit-add-unique", "Unique", false))
            }
            ChangeGroup::ForeignKeys => title.child(add("schema-edit-add-fk", "Foreign key", false)),
            ChangeGroup::Checks => title.child(add("schema-edit-add-check", "Check", false)),
            ChangeGroup::Indexes => title.child(add("schema-edit-add-index", "Index", false)),
            ChangeGroup::Table => title,
        };
        let mut section = div().flex().flex_col().child(title);
        if count == 0 {
            return (section.into_any_element(), 0);
        }
        let specs = specs(group);
        let mut head: Vec<ColumnSpec> = specs.to_vec();
        head.push(ACTIONS_SPEC);
        section = section.child(table::head_line(&head));
        let mut list = div()
            .id(("schema-edit-section", start))
            .role(Role::List)
            .aria_label(SharedString::from(format!("{} {count}", group.label())))
            .flex()
            .flex_col();
        for (index, row) in rows.iter().enumerate() {
            let number = start + index;
            let removed = row.state == RowState::Removed;
            let selected = self.selected == row.target;
            let cells = self.cells(row);
            let target = row.target;
            let deltas = row
                .deltas
                .iter()
                .map(|delta| format!("{} to {}", delta.before, delta.after))
                .collect::<Vec<_>>()
                .join("; ");
            let label = if deltas.is_empty() {
                format!("{}, {}", row.label, state_label(row.state))
            } else {
                format!("{}, {}: {deltas}", row.label, state_label(row.state))
            };
            list = list.child(
                table::line()
                    .id(("schema-edit-row", number))
                    .role(Role::ListItem)
                    .aria_label(SharedString::from(label))
                    .aria_selected(selected)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select(target, window, cx)
                    }))
                    .child(
                        table::card(false, index + 1 == count)
                            .text_color(style::text())
                            .bg(if selected && row.state == RowState::Unchanged {
                                style::select()
                            } else {
                                tint(row.state)
                            })
                            .when(selected, |card| card.border_color(style::accent()))
                            .children(
                                specs
                                    .iter()
                                    .zip(cells)
                                    .map(|(spec, cell)| render_cell(spec, cell, removed)),
                            )
                            .child(self.row_actions(number, row, cx)),
                    ),
            );
        }
        (section.child(list).into_any_element(), count)
    }

    fn sections(&self, cx: &Context<Self>) -> AnyElement {
        let mut page = div()
            .id("schema-edit-sections")
            .role(Role::Group)
            .aria_label("Draft table structure")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .pt(px(4.))
            .pb(px(12.));
        let mut start = 0;
        for group in ChangeGroup::ALL {
            let (section, count) = self.section(group, start, cx);
            // Leave room so row ids never collide across sections.
            start += count + 1;
            page = page.child(section);
        }
        page.into_any_element()
    }

    /// The per-field Undo, shown only while the field differs from the
    /// baseline. `slot` keeps ids unique when several inputs share an attr.
    fn undo_button(&self, slot: usize, target: Target, attr: Attr, cx: &Context<Self>) -> Option<AnyElement> {
        let model = self.model.as_ref()?;
        if !model.changed(target, attr) {
            return None;
        }
        let key = match attr {
            Attr::Name => 0,
            Attr::Field(field) => 1 + field as usize,
        };
        Some(
            self.icon(
                ("schema-edit-undo", key * 64 + slot.min(63)),
                "Undo this field",
                "icons/undo.svg",
                self.can_edit(),
                cx,
                move |this, _, cx| this.undo(target, attr, cx),
            )
            .into_any_element(),
        )
    }

    fn check_row(
        &self,
        id: &'static str,
        label: &'static str,
        checked: bool,
        choice: Choice,
        cx: &Context<Self>,
    ) -> AnyElement {
        let enabled = self.can_edit();
        let target = self.selected;
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .id(id)
                    .role(Role::CheckBox)
                    .aria_label(label)
                    .aria_toggled(checked.into())
                    .tab_index(0)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(4.))
                    .h(px(22.))
                    .rounded(px(4.))
                    .text_color(if enabled { style::text() } else { style::faint() })
                    .when(enabled, |row| {
                        row.cursor_pointer()
                            .hover(|s| s.bg(style::hover()))
                            .on_click(cx.listener(move |this, _, _, cx| this.choose(choice, cx)))
                    })
                    .focus(|s| s.border_color(style::accent()))
                    .child(ui::check_box(checked))
                    .child(label),
            )
            .children(self.undo_button(63, target, choice.attr(), cx))
            .into_any_element()
    }

    /// A labelled row of choice chips (one selected).
    fn chips(
        &self,
        id: &'static str,
        label: &'static str,
        options: Vec<(SharedString, bool, Choice)>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let enabled = self.can_edit();
        let target = self.selected;
        let attr = options.first().map(|(_, _, choice)| choice.attr());
        let mut group = ui::segment_group().flex_wrap().h_auto();
        for (index, (text, selected, choice)) in options.into_iter().enumerate() {
            group = group.child(
                ui::segment((id, index), text, selected, enabled)
                    .role(Role::RadioButton)
                    .aria_selected(selected)
                    .tab_index(0)
                    .when(enabled, |chip| {
                        chip.on_click(cx.listener(move |this, _, _, cx| this.choose(choice, cx)))
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(style::FONT_SMALL))
                    .text_color(style::dim())
                    .child(label)
                    .children(attr.and_then(|attr| self.undo_button(63, target, attr, cx))),
            )
            .child(group)
            .into_any_element()
    }

    fn choices(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(model) = &self.model else {
            return Vec::new();
        };
        let target = self.selected;
        if !model.in_draft(target) {
            return Vec::new();
        }
        let caps = model.caps();
        let mut out = Vec::new();
        match target {
            Target::Table if caps.sqlite_options => {
                let options = &model.draft().table.options;
                out.push(self.check_row(
                    "schema-edit-strict",
                    "STRICT",
                    options.strict,
                    Choice::Strict(!options.strict),
                    cx,
                ));
                out.push(self.check_row(
                    "schema-edit-without-rowid",
                    "WITHOUT ROWID",
                    options.without_rowid,
                    Choice::WithoutRowid(!options.without_rowid),
                    cx,
                ));
            }
            Target::Column(origin) => {
                let Some(column) = model.column(origin) else {
                    return out;
                };
                out.push(self.check_row(
                    "schema-edit-nullable",
                    "Nullable",
                    column.nullable,
                    Choice::Nullable(!column.nullable),
                    cx,
                ));
                let kind = model.default_kind(origin);
                out.push(self.chips(
                    "schema-edit-default-kind",
                    "Default",
                    [
                        (DefaultKind::None, "None"),
                        (DefaultKind::Literal, "Literal"),
                        (DefaultKind::Expression, "Expression"),
                    ]
                    .into_iter()
                    .map(|(value, text)| (text.into(), value == kind, Choice::DefaultKind(value)))
                    .collect(),
                    cx,
                ));
                let generated = model.generated_kind(origin);
                out.push(self.chips(
                    "schema-edit-generated-kind",
                    "Generated",
                    [
                        (GeneratedKind::None, "No"),
                        (GeneratedKind::Virtual, "Virtual"),
                        (GeneratedKind::Stored, "Stored"),
                    ]
                    .into_iter()
                    .map(|(value, text)| {
                        (text.into(), value == generated, Choice::GeneratedKind(value))
                    })
                    .collect(),
                    cx,
                ));
                out.push(self.chips(
                    "schema-edit-identity",
                    "Identity",
                    caps.identities()
                        .iter()
                        .map(|identity| {
                            (
                                identity_label(*identity).into(),
                                *identity == column.identity,
                                Choice::Identity(*identity),
                            )
                        })
                        .collect(),
                    cx,
                ));
                if caps.invisible {
                    out.push(self.check_row(
                        "schema-edit-invisible",
                        "Invisible",
                        column.extra.invisible,
                        Choice::Invisible(!column.extra.invisible),
                        cx,
                    ));
                }
            }
            Target::Index(origin) => {
                if let Some(index) = model.index(origin) {
                    out.push(self.check_row(
                        "schema-edit-unique",
                        "Unique",
                        index.unique,
                        Choice::Unique(!index.unique),
                        cx,
                    ));
                }
            }
            Target::Check(origin) if caps.check_enforced => {
                if let Some(check) = model.check(origin) {
                    out.push(self.check_row(
                        "schema-edit-enforced",
                        "Enforced",
                        check.enforced,
                        Choice::Enforced(!check.enforced),
                        cx,
                    ));
                }
            }
            Target::ForeignKey(origin) => {
                let Some(key) = model.foreign_key(origin) else {
                    return out;
                };
                out.push(self.chips(
                    "schema-edit-on-update",
                    "On update",
                    ACTIONS
                        .iter()
                        .map(|action| {
                            (
                                action_label(*action).into(),
                                *action == key.on_update,
                                Choice::OnUpdate(*action),
                            )
                        })
                        .collect(),
                    cx,
                ));
                out.push(self.chips(
                    "schema-edit-on-delete",
                    "On delete",
                    ACTIONS
                        .iter()
                        .map(|action| {
                            (
                                action_label(*action).into(),
                                *action == key.on_delete,
                                Choice::OnDelete(*action),
                            )
                        })
                        .collect(),
                    cx,
                ));
                if caps.deferrable {
                    out.push(self.chips(
                        "schema-edit-deferrable",
                        "Deferrable",
                        DEFERRABLE
                            .iter()
                            .map(|(value, text)| {
                                ((*text).into(), *value == key.deferrable, Choice::Deferrable(*value))
                            })
                            .collect(),
                        cx,
                    ));
                }
            }
            _ => {}
        }
        out
    }

    fn inspector(&self, cx: &Context<Self>) -> AnyElement {
        let mut panel = div()
            .id("schema-edit-inspector")
            .role(Role::Group)
            .aria_label("Selected object")
            .flex_none()
            .w(px(INSPECTOR))
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(12.))
            .border_l_1()
            .border_color(style::line())
            .bg(style::panel());
        let Some(model) = &self.model else {
            return panel
                .child(div().text_color(style::faint()).child("Waiting for the structure…"))
                .into_any_element();
        };
        let target = self.selected;
        let row = model.row(target);
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(format!("{} {}", target.object().label(), row.label)),
                )
                .when(row.state != RowState::Unchanged, |header| {
                    header.child(ui::badge(state_label(row.state)))
                }),
        );
        let mut reasons: Vec<&'static str> = row
            .deltas
            .iter()
            .filter_map(|delta| delta.destructive)
            .map(|reason| reason.label())
            .collect();
        if let Some(reason) = row.destructive
            && !reasons.contains(&reason.label())
        {
            reasons.push(reason.label());
        }
        for reason in reasons {
            panel = panel.child(
                div()
                    .text_size(px(style::FONT_SMALL))
                    .text_color(style::warn())
                    .child(format!("Destructive: {reason}")),
            );
        }
        let issues: Vec<String> = model
            .issues_for(target)
            .map(|issue| issue.message.clone())
            .collect();
        for (index, message) in issues.into_iter().enumerate() {
            panel = panel.child(
                div()
                    .id(("schema-edit-object-issue", index))
                    .role(Role::Alert)
                    .aria_label(SharedString::from(message.clone()))
                    .text_size(px(style::FONT_SMALL))
                    .text_color(style::bad_text())
                    .child(message),
            );
        }
        if row.state == RowState::Removed {
            return panel
                .child(
                    div()
                        .text_color(style::faint())
                        .child("Removed from the draft. Restore it to edit."),
                )
                .child(
                    ui::button("schema-edit-restore-selected", "Restore", ui::Variant::Secondary, self.can_edit())
                        .tab_index(0)
                        .when(self.can_edit(), |button| {
                            button.on_click(cx.listener(move |this, _, _, cx| this.restore(target, cx)))
                        }),
                )
                .into_any_element();
        }
        panel = panel.children(self.choices(cx));
        for (slot, input) in self.inputs.iter().enumerate() {
            let attr = input.input.attr();
            let error = self
                .input_error
                .as_ref()
                .filter(|(which, _)| *which == input.input)
                .map(|(_, error)| error.clone());
            let note: Option<(SharedString, bool)> = match (&error, model.before(target, attr)) {
                (Some(error), _) => Some((error.clone().into(), true)),
                (None, Some(before)) if input.input.attr() != Attr::Field(Field::Reference) => {
                    Some((format!("was {}", clip(&before)).into(), false))
                }
                _ => input.input.hint().map(|hint| (hint.into(), false)),
            };
            let frame = ui::input_frame(error.is_some())
                .child(div().flex_1().min_w_0().child(input.field.clone()));
            panel = panel.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(ui::labelled(input.input.label(), frame, note)),
                    )
                    .children(self.undo_button(slot, target, attr, cx).map(
                        |button| div().pt(px(14.)).child(button).into_any_element(),
                    )),
            );
        }
        panel.into_any_element()
    }

    fn toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let model = self.model.as_ref();
        let count = model.map_or(0, SchemaEditModel::review_count);
        let issues = model.map_or(0, |m| m.issues().len());
        let destructive = model.map_or(0, |m| m.changes().destructive().count());
        let block = self.review_block();
        let review_enabled = block.is_none();
        let discard_enabled = self.can_edit() && model.is_some_and(SchemaEditModel::has_changes);
        let idle = self.observing.is_none()
            && self.review.is_none()
            && !self.has_pending();
        let review = ui::tool_button_accent(
            "schema-edit-review",
            format!("Review {count}"),
            Some("⌘S"),
            review_enabled,
        )
        .aria_keyshortcuts("Meta+S")
        .tab_index(0)
        .when(review_enabled, |button| {
            button.on_click(cx.listener(|this, _, window, cx| this.open_review(window, cx)))
        });
        // The refusal reason goes on a wrapper so the button keeps one tooltip.
        let review = match block {
            Some(reason) => div()
                .id("schema-edit-review-reason")
                .flex_none()
                .tooltip(ui::tooltip(reason))
                .tooltip_show_delay(ui::tooltip_delay())
                .child(review)
                .into_any_element(),
            None => review.into_any_element(),
        };
        ui::toolbar_strip()
            .id("schema-edit-toolbar")
            .role(Role::Toolbar)
            .aria_label("Structure draft")
            .child(
                gpui::svg()
                    .path("icons/pencil.svg")
                    .size(px(style::ICON))
                    .flex_none()
                    .text_color(style::warn()),
            )
            .child(ui::crumbs(
                format!("{}.", self.request.schema),
                self.request.table.clone(),
            ))
            .child(ui::badge("draft"))
            .when_some(model, |bar, model| bar.child(ui::badge(model.baseline().engine.label())))
            .child(ui::grow())
            .when(destructive > 0, |bar| {
                bar.child(
                    div()
                        .flex_none()
                        .text_color(style::warn())
                        .child(format!("{destructive} destructive")),
                )
            })
            .when(issues > 0, |bar| {
                bar.child(
                    div()
                        .flex_none()
                        .text_color(style::bad_text())
                        .child(format!("{issues} problem{}", if issues == 1 { "" } else { "s" })),
                )
            })
            .child(
                ui::tool_button("schema-edit-discard", "Discard", None, discard_enabled, false)
                    .aria_label("Discard the structure draft")
                    .tab_index(0)
                    .when(discard_enabled, |button| {
                        button.on_click(cx.listener(|this, _, _, cx| this.discard(cx)))
                    }),
            )
            .child(review)
            .child(ui::separator())
            .child(self.icon(
                ("schema-edit-reload", 0),
                "Reload structure",
                "icons/rotate_cw.svg",
                idle && self.connected(),
                cx,
                |this, _, cx| this.observe(cx),
            ))
            .child(self.icon(
                ("schema-edit-close", 0),
                "Back to the structure",
                "icons/close.svg",
                self.review.is_none(),
                cx,
                |this, _, cx| this.close(cx),
            ))
            .into_any_element()
    }

    fn notices(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let note = |id: &'static str, text: String, warn: bool| {
            div()
                .id(id)
                .role(Role::Status)
                .aria_label(SharedString::from(text.clone()))
                .flex_none()
                .px(px(INSET))
                .py(px(4.))
                .bg(if warn { style::warn_fill() } else { style::panel() })
                .border_b_1()
                .border_color(style::line_soft())
                .text_color(if warn { style::warn() } else { style::dim() })
                .child(text)
        };
        if let Some(model) = &self.model
            && !model.editable()
        {
            let reasons = model.read_only_reasons().join(" ");
            out.push(
                note(
                    "schema-edit-read-only",
                    format!("Read only. {reasons}"),
                    true,
                )
                .into_any_element(),
            );
        }
        if let Some(reason) = self.policy.read_only_reason() {
            out.push(note("schema-edit-connection-read-only", reason.into(), true).into_any_element());
        }
        if self.recovery.unknown() {
            let attempt = self.recovery.attempt_id().unwrap_or("unknown");
            out.push(
                div()
                    .id("schema-edit-unknown")
                    .role(Role::Alert)
                    .aria_label("Outcome unknown")
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(INSET))
                    .py(px(5.))
                    .bg(style::bad_fill())
                    .border_b_1()
                    .border_color(style::bad_line())
                    .text_color(style::bad_text())
                    .child(div().flex_1().min_w_0().child(format!(
                        "Outcome unknown for attempt {attempt}. Inspect the table and reload; the change is never replayed."
                    )))
                    .child(
                        ui::button(
                            "schema-edit-reconcile",
                            if self.armed { "Discard record" } else { "Reconcile" },
                            ui::Variant::Danger,
                            !self.has_pending(),
                        )
                        .tab_index(0)
                        .on_click(cx.listener(|this, _, _, cx| this.reconcile(cx))),
                    )
                    .into_any_element(),
            );
        }
        if self.memory_only {
            out.push(
                note(
                    "schema-edit-memory-only",
                    "This draft is too large to keep across restarts; it stays in memory only."
                        .into(),
                    false,
                )
                .into_any_element(),
            );
        }
        if let Some(model) = &self.model {
            let table_issues: Vec<String> = model
                .issues()
                .iter()
                .filter(|issue| issue.origin.is_none())
                .map(|issue| issue.message.clone())
                .collect();
            if !table_issues.is_empty() {
                out.push(
                    div()
                        .id("schema-edit-table-issues")
                        .role(Role::Alert)
                        .aria_label(SharedString::from(table_issues.join(". ")))
                        .flex_none()
                        .px(px(INSET))
                        .py(px(4.))
                        .text_color(style::bad_text())
                        .child(table_issues.join(" · "))
                        .into_any_element(),
                );
            }
        }
        if let Some(seq) = self.failure.shown(&self.message) {
            out.push(
                ui::error_strip("schema-edit-error", seq, self.message.clone()).into_any_element(),
            );
        }
        out
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if keystroke.key == "s"
            && modifiers.platform
            && !modifiers.shift
            && !modifiers.alt
            && !modifiers.control
        {
            // ⌘S opens the review; the dialog itself gates the apply.
            if self.review.is_none() && self.review_block().is_none() {
                self.open_review(window, cx);
            }
            cx.stop_propagation();
            return;
        }
        if keystroke.key == "escape" && !modifiers.modified() && self.review.is_some() {
            self.cancel_review(cx);
            cx.stop_propagation();
        }
    }
}

impl Render for SchemaEditView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.inputs_stale {
            self.rebuild_inputs(window, cx);
        }
        let dialog = self.render_review(window, cx);
        if self.focus_request {
            self.focus_request = false;
            if self.review.is_none() {
                window.focus(&self.focus, cx);
            }
        }
        let summary = match &self.model {
            Some(model) if model.has_changes() => format!(
                "{} change{} · {} problem{}",
                model.review_count(),
                if model.review_count() == 1 { "" } else { "s" },
                model.issues().len(),
                if model.issues().len() == 1 { "" } else { "s" }
            ),
            Some(_) => "No changes".to_owned(),
            None => String::new(),
        };
        let status = if self.failure.shown(&self.message).is_some() {
            String::new()
        } else {
            self.message.clone()
        };
        div()
            .id("schema-edit")
            .key_context("SchemaEdit")
            .role(Role::Group)
            .aria_label("Structure draft editor")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(style::bg())
            .text_color(style::text())
            .text_size(px(style::FONT))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.key_down(event, window, cx)
            }))
            .child(self.toolbar(cx))
            .children(self.notices(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.sections(cx))
                    .child(self.inspector(cx)),
            )
            .child(
                ui::status_line()
                    .child(
                        div()
                            .id("schema-edit-status")
                            .role(Role::Status)
                            .aria_label(SharedString::from(status.clone()))
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(style::dim())
                            .child(status),
                    )
                    .child(summary),
            )
            .children(dialog)
    }
}
