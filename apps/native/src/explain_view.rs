//! Immutable EXPLAIN output from a complete Query Session result. No database
//! access or execution lives here; ANALYZE follows ordinary query policy.
use crate::results::{ResultModel, TerminalStatus, encoded_size};
use dbunk_lib::backend::{
    explain::{ExplainPlan, MAX_PLAN_BYTES, PlanNode},
    select_sql_range,
};
use gpui::{
    App, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, Window,
    div, prelude::*, px, uniform_list,
};
use std::{cell::Cell, rc::Rc};

const RETAINED_LIMIT: usize = 128 * 1024 * 1024;

pub fn draft(sql: &str, analyze: bool) -> Result<String, &'static str> {
    if sql.len() > MAX_PLAN_BYTES - 64 {
        return Err("EXPLAIN source exceeds 1 MiB");
    }
    let first = select_sql_range(sql, &(0..0), false).map_err(|_| "SQL cannot be parsed")?;
    let last = select_sql_range(sql, &(sql.len()..sql.len()), false)
        .map_err(|_| "SQL cannot be parsed")?;
    if first.is_none() || first != last {
        return Err("Select exactly one statement to explain");
    }
    Ok(format!(
        "EXPLAIN ({})\n{sql}",
        if analyze {
            "ANALYZE, BUFFERS, FORMAT JSON"
        } else {
            "FORMAT JSON"
        }
    ))
}

pub struct PlanData {
    plan: ExplainPlan,
    sql: String,
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl PlanData {
    pub fn from_result(
        model: &ResultModel,
        sql: &str,
        runtime_ms: u64,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, String> {
        let complete = model.completion.as_ref().is_some_and(|done| {
            done.status == TerminalStatus::Completed
                && done.omitted_rows == 0
                && done.omitted_result_sets == 0
                && done.omitted_metadata_bytes == 0
                && done.truncation_reasons.is_empty()
        }) && !model.retention_limited
            && model.native_omitted_rows == 0
            && model.native_omitted_metadata == 0
            && model.native_omitted_result_sets == 0;
        if !complete {
            return Err("Plan is incomplete; rerun without truncation or cancellation".into());
        }
        let [set] = model.sets.as_slice() else {
            return Err("EXPLAIN requires exactly one result set".into());
        };
        if set.partial
            || set.omitted_rows != 0
            || set.row_count != Some(1)
            || set.columns.len() != 1
            || set.rows.len() != 1
        {
            return Err("EXPLAIN requires one complete plan row".into());
        }
        let raw = set.rows[0]
            .first()
            .and_then(Option::as_deref)
            .ok_or("EXPLAIN plan is missing")?;
        if sql.len() > MAX_PLAN_BYTES {
            return Err("EXPLAIN source exceeds 1 MiB".into());
        }
        let plan = ExplainPlan::parse_json(raw, runtime_ms, true)
            .map_err(|error| format!("Plan cannot be displayed: {error:?}"))?;
        // Include the bounded visible-index and collapse arrays in this lease.
        let bytes = encoded_size(&(&plan, sql)).saturating_add(plan.nodes.len() * 32);
        if bytes > RETAINED_LIMIT.saturating_sub(budget.get()) {
            return Err("Workspace memory budget is full; plan rows were retained".into());
        }
        budget.set(budget.get() + bytes);
        Ok(Self {
            plan,
            sql: sql.into(),
            budget,
            bytes,
        })
    }
}
impl Drop for PlanData {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Tree,
    Json,
    Sql,
}
#[derive(Clone, Copy)]
enum Action {
    Mode(Mode),
    Copy,
    Close,
}
pub struct Close;
pub struct ExplainView {
    data: PlanData,
    mode: Mode,
    selected: usize,
    collapsed: Vec<bool>,
    visible: Vec<usize>,
    focus: FocusHandle,
    tree: FocusHandle,
    buttons: [FocusHandle; 5],
    status: String,
    scroll: gpui::UniformListScrollHandle,
}
impl EventEmitter<Close> for ExplainView {}
impl ExplainView {
    pub fn new(data: PlanData, cx: &mut Context<Self>) -> Self {
        let count = data.plan.nodes.len();
        Self {
            data,
            mode: Mode::Tree,
            selected: 0,
            collapsed: vec![false; count],
            visible: (0..count).collect(),
            focus: cx.focus_handle(),
            tree: cx.focus_handle(),
            buttons: std::array::from_fn(|_| cx.focus_handle()),
            status: String::new(),
            scroll: gpui::UniformListScrollHandle::new(),
        }
    }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.tree, cx);
    }
    fn activate(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::Mode(mode) => self.mode = mode,
            Action::Copy => {
                let text = match self.mode {
                    Mode::Tree | Mode::Json => &self.data.plan.raw,
                    Mode::Sql => &self.data.sql,
                };
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                self.status = "Copied exact retained text".into();
            }
            Action::Close => cx.emit(Close),
        }
        cx.notify();
    }
    fn rebuild(&mut self) {
        self.visible.clear();
        for (index, node) in self.data.plan.nodes.iter().enumerate() {
            let mut parent = node.parent;
            let mut hidden = false;
            while let Some(index) = parent {
                if self.collapsed[index] {
                    hidden = true;
                    break;
                }
                parent = self.data.plan.nodes[index].parent;
            }
            if !hidden {
                self.visible.push(index);
            }
        }
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
    }
    fn button(
        &self,
        index: usize,
        label: &'static str,
        action: Action,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let weak = cx.weak_entity();
        let button = match action {
            Action::Mode(mode) => crate::ui::segment(
                ("plan-control", index),
                label,
                std::mem::discriminant(&self.mode) == std::mem::discriminant(&mode),
                true,
            ),
            Action::Copy => crate::ui::tool_button(
                ("plan-control", index),
                label,
                Some("icons/copy.svg"),
                true,
                false,
            ),
            _ => crate::ui::tool_button(("plan-control", index), label, None, true, false),
        };
        button
            .track_focus(&self.buttons[index])
            .tab_stop(true)
            .tab_index(0)
            .on_click(cx.listener(move |this, _, _, cx| this.activate(action, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, _, cx| {
                weak.update(cx, |this, cx| this.activate(action, cx)).ok();
            })
    }
}
impl Focusable for ExplainView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for ExplainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.tree.is_focused(window) && !matches!(self.mode, Mode::Tree) {
            window.focus(
                &self.buttons[if matches!(self.mode, Mode::Json) {
                    1
                } else {
                    2
                }],
                cx,
            );
        }
        let node = &self.data.plan.nodes[self.visible[self.selected]];
        let details = node_detail(node);
        let summary = format!(
            "{} nodes · planning {} ms · execution {} ms · observed {} ms",
            self.data.plan.nodes.len(),
            number(self.data.plan.planning_ms),
            number(self.data.plan.execution_ms),
            self.data.plan.runtime_ms
        );
        div()
            .id("explain-view")
            .role(Role::Group)
            .aria_label("EXPLAIN plan")
            .track_focus(&self.focus)
            .key_context("ExplainView")
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .text_sm()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                let modifiers = event.keystroke.modifiers;
                if key == "escape" {
                    cx.emit(Close);
                    cx.stop_propagation();
                } else if key == "tab"
                    && !modifiers.control
                    && !modifiers.alt
                    && !modifiers.platform
                {
                    let mut handles = this.buttons.to_vec();
                    if matches!(this.mode, Mode::Tree) {
                        handles.push(this.tree.clone());
                    }
                    let current = handles.iter().position(|focus| focus.is_focused(window));
                    let next = if modifiers.shift {
                        current.map_or(handles.len() - 1, |i| {
                            (i + handles.len() - 1) % handles.len()
                        })
                    } else {
                        current.map_or(0, |i| (i + 1) % handles.len())
                    };
                    window.focus(&handles[next], cx);
                    cx.stop_propagation();
                } else if this.tree.is_focused(window) && matches!(this.mode, Mode::Tree) {
                    let index = this.visible[this.selected];
                    match key {
                        "up" => this.selected = this.selected.saturating_sub(1),
                        "down" => this.selected = (this.selected + 1).min(this.visible.len() - 1),
                        "left" => {
                            this.collapsed[index] = true;
                            this.rebuild();
                        }
                        "right" => {
                            this.collapsed[index] = false;
                            this.rebuild();
                        }
                        "space" | "enter" => {
                            this.collapsed[index] = !this.collapsed[index];
                            this.rebuild();
                        }
                        _ => return,
                    }
                    this.scroll
                        .scroll_to_item(this.selected, gpui::ScrollStrategy::Center);
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(
                crate::ui::segmented()
                    .child(self.button(0, "Tree", Action::Mode(Mode::Tree), cx))
                    .child(self.button(1, "JSON", Action::Mode(Mode::Json), cx))
                    .child(self.button(2, "Executed SQL", Action::Mode(Mode::Sql), cx))
                    .child(crate::ui::separator())
                    .child(
                        div()
                            .id("plan-summary")
                            .role(Role::Label)
                            .aria_label(summary.clone())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(crate::style::MONO)
                            .text_size(px(crate::style::FONT_SMALL))
                            .text_color(crate::style::faint())
                            .child(summary),
                    )
                    .child(self.button(3, "Copy", Action::Copy, cx))
                    .child(self.button(4, "Return to results", Action::Close, cx)),
            )
            .when(matches!(self.mode, Mode::Tree), |pane| {
                pane.child(
                    div()
                        .id("plan-tree")
                        .role(Role::Tree)
                        .aria_label("Plan nodes; arrows navigate and collapse")
                        .aria_value(details.clone())
                        .track_focus(&self.tree)
                        .tab_stop(true)
                        .tab_index(0)
                        .flex_1()
                        .min_h_0()
                        .child(
                            uniform_list(
                                "plan-rows",
                                self.visible.len(),
                                cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                                    range
                                        .map(|row| {
                                            let index = this.visible[row];
                                            let node = &this.data.plan.nodes[index];
                                            let label = node_summary(node);
                                            div()
                                                .id(("plan-node", index))
                                                .role(Role::TreeItem)
                                                .aria_label(label.clone())
                                                .aria_selected(row == this.selected)
                                                .when(!node.children.is_empty(), |item| {
                                                    item.aria_expanded(!this.collapsed[index])
                                                })
                                                .h(px(crate::style::ROW))
                                                .pl(px((node.depth * 14 + 8) as f32))
                                                .pr_2()
                                                .flex()
                                                .items_center()
                                                .gap(px(4.))
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .font_family(crate::style::MONO)
                                                .hover(|item| item.bg(crate::style::hover()))
                                                .child(
                                                    div()
                                                        .w(px(crate::style::ICON))
                                                        .flex_none()
                                                        .text_color(crate::style::faint())
                                                        .when(!node.children.is_empty(), |caret| {
                                                            caret.child(
                                                                gpui::svg()
                                                                    .path(if this.collapsed[index] {
                                                                        "icons/chevron_right.svg"
                                                                    } else {
                                                                        "icons/chevron_down.svg"
                                                                    })
                                                                    .size(px(crate::style::ICON))
                                                                    .text_color(crate::style::faint()),
                                                            )
                                                        }),
                                                )
                                                .when(row == this.selected, |item| {
                                                    item.bg(crate::style::select())
                                                })
                                                .child(label)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.selected = row;
                                                        window.focus(&this.tree, cx);
                                                        cx.notify();
                                                    },
                                                ))
                                        })
                                        .collect()
                                }),
                            )
                            .track_scroll(&self.scroll)
                            .h_full(),
                        ),
                )
                .child(
                    div()
                        .id("plan-node-details")
                        .role(Role::Label)
                        .aria_label(details.clone())
                        .max_h(px(140.))
                        .overflow_y_scroll()
                        .px_2()
                        .py_1()
                        .border_t_1()
                        .border_color(crate::style::line())
                        .bg(crate::style::panel())
                        .font_family(crate::style::MONO)
                        .text_color(crate::style::dim())
                        .child(details),
                )
            })
            .when(!matches!(self.mode, Mode::Tree), |pane| {
                let text = if matches!(self.mode, Mode::Json) {
                    self.data.plan.raw.clone()
                } else {
                    self.data.sql.clone()
                };
                pane.child(
                    div()
                        .id("plan-text")
                        .role(Role::Label)
                        .aria_label(text.clone())
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px_2()
                        .py_1()
                        .font_family(crate::style::MONO)
                        .child(text),
                )
            })
            .child(
                crate::ui::status_line().child(
                    div()
                        .id("plan-status")
                        .role(Role::Label)
                        .aria_label(self.status.clone())
                        .child(self.status.clone()),
                ),
            )
    }
}
fn number(value: Option<f64>) -> String {
    value.map_or("unknown".into(), |value| value.to_string())
}
fn node_summary(node: &PlanNode) -> String {
    format!(
        "{}{} · cost {} · rows {} → {} · total {} ms",
        node.node_type.chars().take(256).collect::<String>(),
        node.relation.as_ref().map_or(String::new(), |name| format!(
            " · {}",
            name.chars().take(256).collect::<String>()
        )),
        number(node.total_cost),
        number(node.plan_rows),
        number(node.actual_rows),
        number(node.actual_total_ms)
    )
}
fn node_detail(node: &PlanNode) -> String {
    format!(
        "{}\nStartup cost {} · actual startup {} ms · loops {}\nInclusive {} ms · derived self {} ms{} · estimate {:?}\nBuffers: {}",
        node_summary(node),
        number(node.startup_cost),
        number(node.actual_startup_ms),
        number(node.actual_loops),
        number(node.inclusive_ms),
        number(node.self_ms),
        if node.has_unknown_child_timing {
            " (unknown child timing)"
        } else {
            ""
        },
        node.estimate,
        node.buffers
            .iter()
            .map(|(kind, count)| format!("{kind:?}={count}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::results::{Completion, ResultSet};
    fn model() -> ResultModel {
        let mut model = ResultModel::default();
        model.sets = vec![ResultSet {
            columns: vec![Some("QUERY PLAN".into())],
            rows: vec![Rc::from(vec![Some(
                "[{\"Plan\":{\"Node Type\":\"Result\",\"Plan Rows\":1}}]".into(),
            )])],
            row_count: Some(1),
            ..Default::default()
        }];
        model.completion = Some(Completion {
            status: TerminalStatus::Completed,
            omitted_rows: 0,
            omitted_result_sets: 0,
            omitted_notices: 0,
            omitted_metadata_bytes: 0,
            truncation_reasons: vec![],
            error: None,
            refusal: None,
        });
        model
    }
    #[test]
    fn explain_drafts_refuse_script_suffixes_without_misreading_literals() {
        assert!(draft("SELECT 1; DELETE FROM rows", true).is_err());
        assert!(draft("-- only", false).is_err());
        assert_eq!(
            draft("SELECT ';', $$a;b$$; -- exact", true).unwrap(),
            "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)\nSELECT ';', $$a;b$$; -- exact"
        );
        assert!(draft(&"x".repeat(MAX_PLAN_BYTES), false).is_err());
    }
    #[test]
    fn partial_and_cancelled_plans_refuse_and_budget_is_released() {
        let budget = Rc::new(Cell::new(0));
        let mut model = model();
        let data = PlanData::from_result(&model, "EXPLAIN SELECT 1", 3, budget.clone()).unwrap();
        assert_eq!(data.sql, "EXPLAIN SELECT 1");
        assert!(budget.get() > 0);
        drop(data);
        assert_eq!(budget.get(), 0);
        budget.set(RETAINED_LIMIT);
        assert!(PlanData::from_result(&model, "SELECT 1", 0, budget.clone()).is_err());
        assert_eq!(budget.get(), RETAINED_LIMIT);
        budget.set(0);
        model.sets[0].partial = true;
        assert!(PlanData::from_result(&model, "SELECT 1", 0, budget.clone()).is_err());
        model.sets[0].partial = false;
        model.completion.as_mut().unwrap().status = TerminalStatus::Cancelled;
        assert!(PlanData::from_result(&model, "SELECT 1", 0, budget.clone()).is_err());
        assert_eq!(budget.get(), 0);
    }
}
