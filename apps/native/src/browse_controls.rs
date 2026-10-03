//! Browse controls emit candidate state; the table document owns admission,
//! paging and durable acknowledgements. Inspection always belongs to its page.
use crate::{
    accessible_editor::AccessibleEditor,
    browse_preferences::{
        BrowsePreferences, BrowseState, FilterMode, HistoryEntry, Preset, filter_column,
    },
    results::encoded_size,
};
use dbunk_lib::backend::data::*;
use editor::Editor;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, SharedString,
    Window, div, prelude::*, px, rgb,
};
use std::{collections::HashMap, rc::Rc};

pub enum BrowseEvent {
    Apply(BrowseState, bool),
    Mode(FilterMode),
    SavePreset(Preset),
}
#[derive(Clone, Copy)]
enum Action {
    Mode(FilterMode),
    Column,
    Operator,
    Apply,
    Remove(usize),
    Clear,
    AddSort,
    Direction(usize),
    Nulls(usize),
    RemoveSort(usize),
    History,
    ApplyHistory,
    Preset,
    ApplyPreset,
    SavePreset,
    Inspect,
    CopySql,
    CopyParams,
}
const OPERATORS: [&str; 13] = [
    "=",
    "<>",
    ">",
    ">=",
    "<",
    "<=",
    "contains",
    "not contains",
    "starts with",
    "ends with",
    "in list",
    "is null",
    "is not null",
];
struct Field {
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
}
impl Field {
    fn new(label: &'static str, window: &mut Window, cx: &mut Context<BrowseControls>) -> Self {
        let editor = cx.new(|cx| Editor::single_line(window, cx));
        let accessible = cx.new(|cx| AccessibleEditor::field(editor.clone(), label, false, cx));
        Self { editor, accessible }
    }
}
pub struct BrowseControls {
    state: BrowseState,
    history: Vec<HistoryEntry>,
    presets: Vec<Preset>,
    columns: Vec<String>,
    column: usize,
    operator: usize,
    history_index: usize,
    preset_index: usize,
    raw: Field,
    value: Field,
    name: Field,
    pending_raw: Option<String>,
    enabled: bool,
    inspect: bool,
    page: Option<Rc<BrowseTableResult>>,
    message: Option<String>,
    focus: HashMap<String, FocusHandle>,
    tab_order: Vec<FocusHandle>,
}
impl EventEmitter<BrowseEvent> for BrowseControls {}
impl BrowseControls {
    pub fn new(state: BrowseState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            pending_raw: Some(state.raw_filter_text.clone()),
            state,
            history: vec![],
            presets: vec![],
            columns: vec![],
            column: 0,
            operator: 0,
            history_index: 0,
            preset_index: 0,
            raw: Field::new("SQL filter", window, cx),
            value: Field::new("Typed filter value", window, cx),
            name: Field::new("Preset name", window, cx),
            enabled: false,
            inspect: false,
            page: None,
            message: None,
            focus: HashMap::new(),
            tab_order: vec![],
        }
    }
    pub fn state(&self) -> &BrowseState {
        &self.state
    }
    pub fn set_state(&mut self, state: BrowseState, cx: &mut Context<Self>) {
        self.pending_raw = Some(state.raw_filter_text.clone());
        self.state = state;
        self.message = None;
        cx.notify();
    }
    pub fn set_mode(&mut self, mode: FilterMode, cx: &mut Context<Self>) {
        self.state.filter_mode = mode;
        cx.notify();
    }
    pub fn preferences(&mut self, prefs: BrowsePreferences, cx: &mut Context<Self>) {
        self.history = prefs.history;
        self.presets = prefs.presets;
        self.history_index = self.history_index.min(self.history.len().saturating_sub(1));
        self.preset_index = self.preset_index.min(self.presets.len().saturating_sub(1));
        cx.notify();
    }
    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.enabled != enabled {
            self.enabled = enabled;
            for field in [&self.raw, &self.value, &self.name] {
                field
                    .editor
                    .update(cx, |editor, _| editor.set_read_only(!enabled));
            }
            cx.notify();
        }
    }
    pub fn page(&mut self, page: Option<Rc<BrowseTableResult>>, cx: &mut Context<Self>) {
        if let Some(page) = &page {
            let bytes = page
                .columns
                .iter()
                .map(|column| column.name.len())
                .sum::<usize>();
            if bytes <= 64 * 1024 {
                self.columns = page
                    .columns
                    .iter()
                    .map(|column| column.name.clone())
                    .collect();
                self.column = self.column.min(self.columns.len().saturating_sub(1));
            } else {
                self.columns.clear();
                self.message = Some("Column names exceed the filter control budget".into());
            }
        }
        self.page = page;
        cx.notify();
    }
    pub fn composition_active(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        [&self.raw, &self.value, &self.name].iter().any(|field| {
            field.editor.focus_handle(cx).is_focused(window)
                && field
                    .editor
                    .update(cx, |editor, cx| {
                        gpui::EntityInputHandler::marked_text_range(editor, window, cx)
                    })
                    .is_some()
        })
    }
    pub fn focus_handles(&self, _: &App) -> Vec<FocusHandle> {
        self.tab_order.clone()
    }
    fn action(&mut self, action: Action, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled
            && !matches!(
                action,
                Action::Inspect | Action::CopySql | Action::CopyParams
            )
        {
            return;
        }
        self.message = None;
        // Inspect the editor buffer before allocating an owned candidate. Editor
        // drafts remain editable when too large, but cannot enter a request.
        let field = match action {
            Action::Apply if self.state.filter_mode == FilterMode::Raw => Some((&self.raw, 65536)),
            Action::Apply => Some((&self.value, 65536)),
            Action::SavePreset => Some((&self.name, 8192)),
            _ => None,
        };
        if field
            .is_some_and(|(field, limit)| field.editor.read(cx).buffer().read(cx).len(cx).0 > limit)
        {
            self.message =
                Some("Input exceeds the browse setting limit; shorten it before applying".into());
            cx.notify();
            return;
        }
        let mut state = self.state.clone();
        match action {
            Action::Mode(mode) => {
                cx.emit(BrowseEvent::Mode(mode));
            }
            Action::Column => {
                if !self.columns.is_empty() {
                    self.column = (self.column + 1) % self.columns.len();
                }
            }
            Action::Operator => self.operator = (self.operator + 1) % OPERATORS.len(),
            Action::Apply => {
                if state.filter_mode == FilterMode::Raw {
                    state.raw_filter_text = self.raw.editor.read(cx).text(cx);
                } else {
                    let Some(column) = self.columns.get(self.column) else {
                        return;
                    };
                    match build_filter(column, self.operator, &self.value.editor.read(cx).text(cx))
                    {
                        Ok(filter) => {
                            state.apply_filter(filter);
                        }
                        Err(error) => {
                            self.message = Some(error.into());
                            cx.notify();
                            return;
                        }
                    }
                }
                cx.emit(BrowseEvent::Apply(state, true));
            }
            Action::Remove(index) => {
                if let Some(column) = state
                    .typed_filters
                    .get(index)
                    .and_then(filter_column)
                    .map(str::to_owned)
                {
                    state
                        .typed_filters
                        .retain(|filter| filter_column(filter) != Some(&column));
                    cx.emit(BrowseEvent::Apply(state, true));
                }
            }
            Action::Clear => {
                state.typed_filters.clear();
                state.raw_filter_text.clear();
                cx.emit(BrowseEvent::Apply(state, true));
            }
            Action::AddSort => {
                if let Some(column) = self.columns.get(self.column)
                    && !state.sort.iter().any(|key| &key.column == column)
                {
                    state.sort.push(BrowseSortKey {
                        column: column.clone(),
                        direction: BrowseSortDirection::Asc,
                        nulls: BrowseNulls::Default,
                    });
                    cx.emit(BrowseEvent::Apply(state, true));
                }
            }
            Action::Direction(index) => {
                if let Some(key) = state.sort.get_mut(index) {
                    key.direction = match key.direction {
                        BrowseSortDirection::Asc => BrowseSortDirection::Desc,
                        BrowseSortDirection::Desc => BrowseSortDirection::Asc,
                    };
                    cx.emit(BrowseEvent::Apply(state, true));
                }
            }
            Action::Nulls(index) => {
                if let Some(key) = state.sort.get_mut(index) {
                    key.nulls = match key.nulls {
                        BrowseNulls::Default => BrowseNulls::First,
                        BrowseNulls::First => BrowseNulls::Last,
                        BrowseNulls::Last => BrowseNulls::Default,
                    };
                    cx.emit(BrowseEvent::Apply(state, true));
                }
            }
            Action::RemoveSort(index) => {
                if index < state.sort.len() {
                    state.sort.remove(index);
                    cx.emit(BrowseEvent::Apply(state, true));
                }
            }
            Action::History => {
                if !self.history.is_empty() {
                    self.history_index = (self.history_index + 1) % self.history.len();
                }
            }
            Action::ApplyHistory => {
                if let Some(entry) = self.history.get(self.history_index) {
                    cx.emit(BrowseEvent::Apply(entry.state(state.page_size), false));
                }
            }
            Action::Preset => {
                if !self.presets.is_empty() {
                    self.preset_index = (self.preset_index + 1) % self.presets.len();
                }
            }
            Action::ApplyPreset => {
                if let Some(preset) = self.presets.get(self.preset_index) {
                    cx.emit(BrowseEvent::Apply(preset.state.clone(), false));
                }
            }
            Action::SavePreset => {
                let name = self.name.editor.read(cx).text(cx).trim().to_owned();
                if name.is_empty() || name.len() > 8192 {
                    self.message = Some("Preset name must contain 1 to 8192 bytes".into());
                } else {
                    cx.emit(BrowseEvent::SavePreset(Preset { name, state }));
                }
            }
            Action::Inspect => self.inspect = !self.inspect,
            Action::CopySql | Action::CopyParams => {
                if let Some(page) = &self.page {
                    if encoded_size(&page.inspection) > 256 * 1024 {
                        self.message =
                            Some("Query inspection exceeds 256 KiB; copy refused".into());
                    } else {
                        let text = if matches!(action, Action::CopySql) {
                            page.inspection.sql.clone()
                        } else {
                            parameters(&page.inspection.params)
                        };
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    }
                }
            }
        }
        cx.notify();
    }
    fn button(
        &mut self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: Action,
        available: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let id = id.into();
        let label = label.into();
        let focus = self
            .focus
            .entry(id.to_string())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let enabled = available
            && (self.enabled
                || matches!(
                    action,
                    Action::Inspect | Action::CopySql | Action::CopyParams
                ));
        if enabled {
            self.tab_order.push(focus.clone());
        }
        let weak = cx.weak_entity();
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label.clone())
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(enabled)
            .px_2()
            .py_1()
            .text_color(if enabled {
                rgb(0xffffff)
            } else {
                rgb(0x777777)
            })
            .focus(|style| style.bg(rgb(0x222222)))
            .child(label)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if enabled {
                    this.action(action, window, cx);
                }
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                if enabled {
                    weak.update(cx, |this, cx| this.action(action, window, cx))
                        .ok();
                }
            })
            .into_any_element()
    }
}
impl Render for BrowseControls {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(raw) = self.pending_raw.take() {
            self.raw
                .editor
                .update(cx, |editor, cx| editor.set_text(raw, window, cx));
        }
        self.tab_order.clear();
        let mut controls = div()
            .flex()
            .flex_wrap()
            .items_center()
            .child(self.button(
                "browse-typed",
                "Typed",
                Action::Mode(FilterMode::Typed),
                true,
                cx,
            ))
            .child(self.button(
                "browse-raw",
                "WHERE",
                Action::Mode(FilterMode::Raw),
                true,
                cx,
            ));
        if self.state.filter_mode == FilterMode::Raw {
            self.tab_order.push(self.raw.editor.focus_handle(cx));
            controls = controls.child(
                div()
                    .min_w(px(250.))
                    .flex_1()
                    .child(self.raw.accessible.clone()),
            );
        } else {
            controls = controls
                .child(self.button(
                    "browse-column",
                    format!(
                            "Column: {}",
                            self.columns
                                .get(self.column)
                                .map(String::as_str)
                                .unwrap_or("none")
                        ),
                    Action::Column,
                    !self.columns.is_empty(),
                    cx,
                ))
                .child(self.button(
                    "browse-operator",
                    OPERATORS[self.operator],
                    Action::Operator,
                    true,
                    cx,
                ));
            if self.operator < 11 {
                self.tab_order.push(self.value.editor.focus_handle(cx));
                controls = controls.child(div().w(px(220.)).child(self.value.accessible.clone()));
            }
        }
        controls = controls
            .child(self.button(
                "browse-apply",
                "Apply filter",
                Action::Apply,
                self.state.filter_mode == FilterMode::Raw || !self.columns.is_empty(),
                cx,
            ))
            .child(self.button(
                "browse-clear",
                "Clear filters",
                Action::Clear,
                !self.state.typed_filters.is_empty() || !self.state.raw_filter_text.is_empty(),
                cx,
            ));
        let mut active = div()
            .id("browse-active")
            .max_h(px(110.))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        for (index, filter) in self.state.typed_filters.clone().iter().enumerate() {
            active = active.child(
                div()
                    .flex()
                    .items_center()
                    .id(("active-filter", index))
                    .role(Role::Group)
                    .aria_label(filter_summary(filter))
                    .child(filter_summary(filter))
                    .child(self.button(
                        format!("filter-remove-{index}"),
                        format!("Remove filter {}", index + 1),
                        Action::Remove(index),
                        filter_column(filter).is_some(),
                        cx,
                    )),
            );
        }
        for (index, sort) in self.state.sort.clone().iter().enumerate() {
            active = active.child(
                div()
                    .flex()
                    .items_center()
                    .id(("active-sort", index))
                    .role(Role::Group)
                    .aria_label(format!("Sort {}: {}", index + 1, sort.column))
                    .child(format!("{}. {}", index + 1, sort.column))
                    .child(self.button(
                        format!("sort-direction-{index}"),
                        format!("Direction: {:?}", sort.direction),
                        Action::Direction(index),
                        true,
                        cx,
                    ))
                    .child(self.button(
                        format!("sort-nulls-{index}"),
                        format!("NULLs: {:?}", sort.nulls),
                        Action::Nulls(index),
                        true,
                        cx,
                    ))
                    .child(self.button(
                        format!("sort-remove-{index}"),
                        "Remove sort",
                        Action::RemoveSort(index),
                        true,
                        cx,
                    )),
            );
        }
        let mut history = div()
            .flex()
            .flex_wrap()
            .items_center()
            .child(self.button(
                "browse-add-sort",
                "Add sort on column",
                Action::AddSort,
                !self.columns.is_empty(),
                cx,
            ))
            .child(self.button(
                "browse-history",
                format!(
                    "History {}/{}",
                    if self.history.is_empty() {
                        0
                    } else {
                        self.history_index + 1
                    },
                    self.history.len()
                ),
                Action::History,
                !self.history.is_empty(),
                cx,
            ))
            .child(self.button(
                "browse-apply-history",
                "Apply history",
                Action::ApplyHistory,
                !self.history.is_empty(),
                cx,
            ))
            .child(self.button(
                "browse-preset",
                format!(
                        "Preset: {}",
                        self.presets
                            .get(self.preset_index)
                            .map(|preset| preset.name.as_str())
                            .unwrap_or("none")
                    ),
                Action::Preset,
                !self.presets.is_empty(),
                cx,
            ))
            .child(self.button(
                "browse-apply-preset",
                "Apply preset",
                Action::ApplyPreset,
                !self.presets.is_empty(),
                cx,
            ));
        self.tab_order.push(self.name.editor.focus_handle(cx));
        history = history
            .child(div().w(px(140.)).child(self.name.accessible.clone()))
            .child(self.button(
                "browse-save-preset",
                "Save preset",
                Action::SavePreset,
                true,
                cx,
            ))
            .child(self.button(
                "browse-inspect",
                "Inspect query",
                Action::Inspect,
                self.page.is_some(),
                cx,
            ));
        let selected_history = self.history.get(self.history_index).map(|entry| {
            format!(
                "{} · {} · {} sort",
                entry.applied_at,
                entry
                    .typed_filters
                    .iter()
                    .map(filter_summary)
                    .chain(
                        (!entry.raw_filter_text.is_empty()).then(|| entry
                            .raw_filter_text
                            .chars()
                            .take(160)
                            .collect::<String>())
                    )
                    .collect::<Vec<_>>()
                    .join(" AND "),
                entry.sort.len()
            )
        });
        let mut inspection = div().flex().flex_col();
        if self.inspect
            && let Some(page) = self.page.clone()
        {
            if encoded_size(&page.inspection) > 256 * 1024 {
                inspection =
                    inspection.child("Query inspection exceeds the 256 KiB display budget");
            } else {
                inspection = inspection
                    .child(
                        div()
                            .flex()
                            .child(self.button(
                                "browse-copy-sql",
                                "Copy SQL",
                                Action::CopySql,
                                true,
                                cx,
                            ))
                            .child(self.button(
                                "browse-copy-params",
                                "Copy parameters",
                                Action::CopyParams,
                                true,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .id("browse-inspection")
                            .role(Role::Label)
                            .aria_label(format!(
                                "Executed SQL: {}\nParameters: {}",
                                page.inspection.sql,
                                parameters(&page.inspection.params)
                            ))
                            .max_h(px(140.))
                            .overflow_y_scroll()
                            .child(format!(
                                "{}\n{}",
                                page.inspection.sql,
                                parameters(&page.inspection.params)
                            )),
                    )
                    .when(page.omitted_rows > 0 || page.truncated_cells > 0, |node| {
                        node.child(format!(
                            "Partial result: {} omitted rows, {} truncated cells",
                            page.omitted_rows, page.truncated_cells
                        ))
                    });
            }
        }
        div()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" {
                    let focused = [&this.raw.editor, &this.value.editor]
                        .into_iter()
                        .find(|editor| editor.focus_handle(cx).contains_focused(window, cx));
                    if let Some(editor) = focused {
                        let composing = editor.update(cx, |editor, cx| {
                            gpui::EntityInputHandler::marked_text_range(editor, window, cx)
                                .is_some()
                        });
                        if !composing {
                            this.action(Action::Apply, window, cx);
                            cx.stop_propagation();
                        }
                    }
                }
            }))
            .flex()
            .flex_col()
            .text_sm()
            .border_b_1()
            .border_color(rgb(0x333333))
            .child(controls)
            .child(active)
            .child(history)
            .children(selected_history.map(|text| {
                div()
                    .id("selected-browse-history")
                    .role(Role::Label)
                    .aria_label(text.clone())
                    .px_2()
                    .child(text)
            }))
            .children(self.message.clone().map(|text| {
                div()
                    .id("browse-control-message")
                    .role(Role::Status)
                    .aria_label(text.clone())
                    .child(text)
            }))
            .child(inspection)
    }
}
fn filter_summary(filter: &BrowseFilter) -> String {
    let text = match filter {
        BrowseFilter::Comparison {
            column,
            operator,
            value,
        } => format!("{column} {operator:?} {value}"),
        BrowseFilter::TextMatch {
            column,
            operator,
            value,
        } => format!("{column} {operator:?} {value}"),
        BrowseFilter::IsNull { column } => format!("{column} IS NULL"),
        BrowseFilter::IsNotNull { column } => format!("{column} IS NOT NULL"),
        BrowseFilter::InList { column, values } => format!("{column} IN ({})", values.join(", ")),
        BrowseFilter::RawSql { text } => text.clone(),
    };
    if text.chars().count() > 180 {
        format!("{}…", text.chars().take(180).collect::<String>())
    } else {
        text
    }
}
fn parameters(params: &[InspectionParam]) -> String {
    params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            // JSON retains element boundaries and escaped newlines. A comma-joined
            // array would make ["a,b", "c"] indistinguishable from ["a", "b,c"].
            format!(
                "${}={}",
                index + 1,
                serde_json::to_string(param).expect("typed inspection parameter")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_filter(column: &str, operator: usize, input: &str) -> Result<BrowseFilter, &'static str> {
    if column.is_empty() || input.len() > 64 * 1024 {
        return Err("Filter is empty or exceeds 64 KiB");
    }
    if operator == 11 {
        return Ok(BrowseFilter::IsNull {
            column: column.into(),
        });
    }
    if operator == 12 {
        return Ok(BrowseFilter::IsNotNull {
            column: column.into(),
        });
    }
    let value = input.trim();
    if value.is_empty() {
        return Err("Enter a filter value");
    }
    if operator == 10 {
        let values = value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if values.is_empty() {
            return Err("Enter one or more comma-separated values");
        }
        return Ok(BrowseFilter::InList {
            column: column.into(),
            values,
        });
    }
    if operator >= 6 {
        let operator = match operator {
            6 => TextMatchOperator::Contains,
            7 => TextMatchOperator::NotContains,
            8 => TextMatchOperator::StartsWith,
            9 => TextMatchOperator::EndsWith,
            _ => return Err("Unknown operator"),
        };
        return Ok(BrowseFilter::TextMatch {
            column: column.into(),
            operator,
            value: value.into(),
        });
    }
    let operator = match operator {
        0 => ComparisonOperator::Eq,
        1 => ComparisonOperator::Neq,
        2 => ComparisonOperator::Gt,
        3 => ComparisonOperator::Gte,
        4 => ComparisonOperator::Lt,
        5 => ComparisonOperator::Lte,
        _ => return Err("Unknown operator"),
    };
    Ok(BrowseFilter::Comparison {
        column: column.into(),
        operator,
        value: value.into(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspection_parameters_preserve_array_boundaries_and_control_characters() {
        let source = vec![
            InspectionParam::TextArray {
                values: vec!["a,b".into(), "c\n東京".into()],
            },
            InspectionParam::Text {
                value: "\tNULL".into(),
            },
        ];
        let rendered = parameters(&source);
        assert_eq!(rendered.lines().count(), 2);
        for (line, param) in rendered.lines().zip(&source) {
            let (_, json) = line.split_once('=').unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(json).unwrap(),
                serde_json::to_value(param).unwrap()
            );
        }
        assert_ne!(
            parameters(&[InspectionParam::TextArray {
                values: vec!["a".into(), "b,c".into()]
            }]),
            parameters(&[InspectionParam::TextArray {
                values: vec!["a,b".into(), "c".into()]
            }])
        );
    }
    #[test]
    fn typed_editor_matches_baseline_null_empty_and_list_rules() {
        assert!(matches!(
            build_filter("name", 11, ""),
            Ok(BrowseFilter::IsNull { .. })
        ));
        assert!(build_filter("name", 0, "  ").is_err());
        assert!(
            matches!(build_filter("name", 10, " 東京, , É "), Ok(BrowseFilter::InList { values, .. }) if values == ["東京", "É"])
        );
        let mut state = BrowseState {
            raw_filter_text: "enabled".into(),
            ..Default::default()
        };
        state.apply_filter(build_filter("name", 0, "old").unwrap());
        state.apply_filter(build_filter("name", 1, "new").unwrap());
        assert_eq!(state.typed_filters.len(), 1);
        assert_eq!(state.raw_filter_text, "enabled");
    }
}
