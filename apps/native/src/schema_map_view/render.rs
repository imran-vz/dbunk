use super::*;
use gpui::uniform_list;
impl SchemaMapView {
    fn label(&self, action: Action, default: &str) -> String {
        let prefs = self.working.as_ref().map(|v| v.prefs).unwrap_or_default();
        match action {
            Action::Routing => format!("Routing: {:?}", prefs.routing),
            Action::Attributes => format!("Attributes: {:?}", prefs.attributes),
            Action::Types => format!("Types: {}", prefs.show_types),
            Action::Nulls => format!("NULL: {}", prefs.show_nulls),
            Action::Comments => format!("Comments: {}", prefs.show_comments),
            Action::Clear if self.dirty => "Clear map / discard unsaved settings".into(),
            _ => default.into(),
        }
    }
    fn button(&self, index: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        self.button_as(index, false, cx)
    }
    /// A map control. On/off options are check boxes (their AX label keeps
    /// the `Types: true` form); `page` renders the empty state's primary.
    fn button_as(&self, index: usize, page: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (action, default) = ACTIONS[index];
        let label = self.label(action, default);
        let enabled = self.enabled(action);
        let weak = cx.weak_entity();
        let prefs = self.working.as_ref().map(|v| v.prefs).unwrap_or_default();
        let checked = match action {
            Action::Types => Some(prefs.show_types),
            Action::Nulls => Some(prefs.show_nulls),
            Action::Comments => Some(prefs.show_comments),
            _ => None,
        };
        let id = ("map-control", index);
        let base = if let Some(checked) = checked {
            crate::overview_view::chrome::check_row(id, default, checked, enabled).aria_label(label)
        } else if page {
            crate::ui::button(id, label, crate::ui::Variant::Primary, enabled)
        } else if matches!(action, Action::Connect | Action::Refresh) {
            crate::ui::tool_button_accent(id, label, None, enabled)
        } else {
            let icon = matches!(action, Action::Routing | Action::Attributes)
                .then_some("icons/chevron_up_down.svg");
            crate::ui::tool_button(id, label, icon, enabled, false)
        };
        base.a11y_synthetic_children(move |b| {
            if !enabled {
                b.parent_node().set_disabled();
            }
            if let Some(checked) = checked {
                b.parent_node()
                    .set_toggled(gpui::accesskit::Toggled::from(checked));
            }
        })
        .track_focus(&self.buttons[index])
        .tab_index(0)
        .tab_stop(enabled)
        .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
        .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
            weak.update(cx, |this, cx| this.activate(action, window, cx))
                .ok();
        })
        .into_any_element()
    }
    fn scope_button(
        &self,
        index: usize,
        scope: Scope,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let enabled = self.enabled(Action::Scope(scope));
        let selected = self.scope == scope;
        let weak = cx.weak_entity();
        crate::ui::segment(("map-scope", index), label, selected, enabled)
            .role(Role::Tab)
            .aria_selected(selected)
            .a11y_synthetic_children(move |b| {
                if !enabled {
                    b.parent_node().set_disabled();
                }
            })
            .track_focus(&self.scope_buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate(Action::Scope(scope), window, cx)
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| {
                    this.activate(Action::Scope(scope), window, cx)
                })
                .ok();
            })
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let m = event.keystroke.modifiers;
                if m.control || m.alt || m.platform {
                    return;
                }
                if matches!(event.keystroke.key.as_str(), "left" | "right") {
                    let next = if event.keystroke.key == "left" {
                        (index + 2) % 3
                    } else {
                        (index + 1) % 3
                    };
                    let scope = [Scope::Database, Scope::Schema, Scope::Relation][next];
                    this.activate(Action::Scope(scope), window, cx);
                    window.focus(&this.scope_buttons[next], cx);
                    cx.stop_propagation();
                }
            }))
    }
}
impl SchemaMapView {
    fn object_list(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let count = self.count();
        let selected = self.selected_index();
        let key = self.scene.as_ref().map(Scene::key);
        let rows = uniform_list(
            "map-object-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                let focused = this.list.is_focused(window);
                range
                    .map(|index| {
                        let label = this.row_label(index);
                        let is_selected = selected == Some(index);
                        let selection = this.selection_at(index);
                        let weak = cx.weak_entity();
                        crate::ui::list_row(("map-object", index), is_selected, focused)
                            .role(Role::ListBoxOption)
                            .aria_label(label.clone())
                            .aria_selected(is_selected)
                            .font_family(crate::style::MONO)
                            .text_size(px(crate::style::FONT_DATA))
                            .child(crate::ui::row_label(label))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.scene.as_ref().map(Scene::key) == key {
                                    this.select(selection);
                                    window.focus(&this.list, cx);
                                    cx.notify();
                                }
                            }))
                            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                                weak.update(cx, |this, cx| {
                                    if this.scene.as_ref().map(Scene::key) == key {
                                        this.select(selection);
                                        window.focus(&this.list, cx);
                                        cx.notify();
                                    }
                                })
                                .ok();
                            })
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();
        div().id("map-objects").role(Role::ListBox)
            .aria_label(format!("{count} tables and foreign keys. Arrows select; Shift+Arrow moves a selected table 10 world units and saves; Enter inspects; Open selected table rechecks its OID."))
            .track_focus(&self.list).tab_stop(self.scene.is_some()).tab_index(0)
            .w(px(OBJECT_LIST)).min_h_0().py(px(crate::style::S2)).border_r_1().border_color(crate::style::line_soft())
            .flex().flex_col()
            .child(rows)
    }
}
/// Width of the map's table and foreign-key list.
const OBJECT_LIST: f32 = 340.;
/// Height of the inspection pane (object list and details) under the map.
const INSPECTION: f32 = 200.;
const COVERAGE: &str = "Read-only table graph, including partitioned tables. Inherited foreign-key and trigger copies are omitted. Direct-neighbor scope shows only focal edges. SVG and PNG save the current viewport on white; no row data or DDL is executed.";
impl Render for SchemaMapView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::overview_view::chrome::{self, ToolConnection};
        use crate::style;
        let captured = self
            .scene
            .as_ref()
            .map(|scene| {
                let scope = match &scene.snapshot().scope {
                    SchemaMapScope::Database => "Database".into(),
                    SchemaMapScope::Schema { name, .. } => format!("Schema {name:?}"),
                    SchemaMapScope::Relation { schema, table, .. } => {
                        format!("Table {schema:?}.{table:?}")
                    }
                };
                format!(
                    "Captured {} · database {:?} OID {} · {} · {} tables / {} foreign keys{}",
                    scope,
                    scene.snapshot().database,
                    scene.snapshot().database_oid,
                    scene.snapshot().captured_at,
                    scene.nodes().len(),
                    scene.edges().len(),
                    if self.current { "" } else { " · STALE" },
                )
            })
            .unwrap_or_else(|| "No captured map".into());
        // Resting summary: counts and capture time; the full capture (with
        // OIDs and the exact timestamp) stays in the tooltip and AX label.
        let summary = self.scene.as_ref().map(|scene| {
            let tables = scene.nodes().len();
            let keys = scene.edges().len();
            (
                format!(
                    "{tables} {} · {keys} {}",
                    if tables == 1 { "table" } else { "tables" },
                    if keys == 1 {
                        "foreign key"
                    } else {
                        "foreign keys"
                    }
                ),
                scene.snapshot().captured_at.to_string(),
            )
        });
        let has_map = self.scene.is_some();
        let failure = self.failure.shown(&self.status);
        let shown = |index: usize| self.enabled(ACTIONS[index].0);
        let primary = if self.controls.is_none() { 0 } else { 1 };
        let connection = ToolConnection::lookup(&self.connections, self.connection.as_deref());
        let context = chrome::connection_context(connection, self.connection.is_some());
        let mut header_actions = Vec::new();
        if has_map {
            header_actions.push(self.button(primary, cx));
        }
        if shown(2) {
            header_actions.push(self.button(2, cx));
        }
        let scopes = crate::ui::segment_group()
            .id("map-scopes")
            .role(Role::TabList)
            .aria_label("Map setup scope")
            .children(
                [
                    (Scope::Database, "Database"),
                    (Scope::Schema, "Schema"),
                    (Scope::Relation, "Table and direct neighbors"),
                ]
                .into_iter()
                .enumerate()
                .map(|(i, (scope, label))| self.scope_button(i, scope, label, cx)),
            );
        let setup_actions = [12, 13, 14, 3]
            .into_iter()
            .filter(|index| shown(*index))
            .map(|index| self.button(index, cx))
            .collect::<Vec<_>>();
        let glossary = self.button(22, cx);
        let field_well = |field: Option<Entity<Field>>, label: &'static str, focused: bool| {
            field.map(|field| {
                crate::ui::labelled(
                    label,
                    crate::ui::input_well(false, focused)
                        .child(div().flex_1().min_w_0().child(field)),
                    None,
                )
            })
        };
        let schema_focused = self
            .fields
            .first()
            .is_some_and(|field| field.focus_handle(cx).contains_focused(window, cx));
        let table_focused = self
            .fields
            .get(1)
            .is_some_and(|field| field.focus_handle(cx).contains_focused(window, cx));
        let map_toolbar = has_map.then(|| {
            div()
                .flex_none()
                .min_h(px(style::TOOLBAR))
                .px(px(style::INSET_BAR))
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(style::S2))
                .border_b_1()
                .border_color(style::line_soft())
                .children([7, 8].map(|index| self.button(index, cx)))
                .child(crate::ui::separator())
                .children([9, 10, 11].map(|index| self.button(index, cx)))
                .child(crate::ui::grow())
                .children([4, 6, 5].map(|index| self.button(index, cx)))
                .child(crate::ui::separator())
                .children(
                    [19, 21, 20]
                        .into_iter()
                        .filter(|index| shown(*index))
                        .map(|index| self.button(index, cx)),
                )
        });
        let detail_actions = [15, 16, 17, 18]
            .into_iter()
            .filter(|index| shown(*index))
            .map(|index| self.button(index, cx))
            .collect::<Vec<_>>();
        let show_details = has_map || !self.detail_text.is_empty();
        let details = div()
            .id("map-details")
            .role(Role::Label)
            .aria_label(format!(
                "Selected map detail page {}. {}",
                self.detail_page + 1,
                self.detail_text
            ))
            .track_focus(&self.details)
            .tab_stop(self.selection.is_some() || self.glossary)
            .tab_index(0)
            .flex_1()
            .min_w_0()
            .overflow_scroll()
            .track_scroll(&self.detail_scroll)
            .px(px(style::INSET_CONTENT))
            .py(px(style::S3))
            .font_family(style::MONO)
            .text_size(px(style::FONT_DATA))
            .focus(|s| s.bg(style::row_hover()))
            .child(self.detail_text.clone());
        let empty = (!has_map).then(|| {
            let (title, body) = if self.controls.is_none() {
                (
                    "No schema map yet",
                    "Connect, then load the map for the chosen scope.",
                )
            } else if self.pending.is_some() || self.opening {
                ("Loading map…", "Reading tables and foreign keys.")
            } else {
                ("No schema map yet", "Load the map for the chosen scope.")
            };
            let action = ((!self.opening && self.pending.is_none()) || self.controls.is_none())
                .then(|| self.button_as(primary, true, cx));
            crate::ui::empty_state("icons/list_tree.svg", title, Some(body.into()), action)
        });
        let inspection = div()
            .flex()
            .flex_none()
            .h(px(INSPECTION))
            .min_h_0()
            .border_t_1()
            .border_color(style::line())
            .when(has_map, |pane| pane.child(self.object_list(cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_none()
                            .h(px(style::TOOLBAR))
                            .px(px(style::INSET_BAR))
                            .flex()
                            .items_center()
                            .gap(px(style::S2))
                            .child(crate::ui::section_label("Details"))
                            .child(crate::ui::grow())
                            .children(detail_actions),
                    )
                    .child(details),
            );
        div()
            .id("schema-map-tool")
            .role(Role::Group)
            .aria_label(format!(
                "PostgreSQL schema map for {}",
                self.connection.as_deref().unwrap_or("unbound connection")
            ))
            .track_focus(&self.root)
            .size_full()
            .flex()
            .flex_col()
            .bg(style::bg())
            .text_color(style::text())
            .text_size(px(style::FONT))
            .key_context("SchemaMap")
            .on_action(cx.listener(|this, _: &NextControl, window, cx| {
                if !this.composing(window, cx) {
                    this.focus_control(false, window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousControl, window, cx| {
                if !this.composing(window, cx) {
                    this.focus_control(true, window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_key_down(cx.listener(Self::key))
            .child(crate::ui::tool_header(
                "Schema map",
                context,
                header_actions,
            ))
            // Setup: scope, its fields, and map-level actions.
            .child(
                div()
                    .flex_none()
                    .px(px(style::INSET_CONTENT))
                    .py(px(style::S3))
                    .flex()
                    .flex_wrap()
                    .items_end()
                    .gap(px(style::S4))
                    .border_b_1()
                    .border_color(style::line_soft())
                    .child(crate::ui::labelled("Scope", scopes, None))
                    .when(self.scope != Scope::Database, |row| {
                        row.child(div().w(px(200.)).children(field_well(
                            self.fields.first().cloned(),
                            "Schema",
                            schema_focused,
                        )))
                    })
                    .when(self.scope == Scope::Relation, |row| {
                        row.child(div().w(px(200.)).children(field_well(
                            self.fields.get(1).cloned(),
                            "Table",
                            table_focused,
                        )))
                    })
                    .child(crate::ui::grow())
                    .when_some(summary, |row, (counts, at)| {
                        row.child(
                            div()
                                .id("map-captured-scope")
                                .role(Role::Label)
                                .aria_label(captured.clone())
                                .h(px(style::CONTROL))
                                .flex()
                                .items_center()
                                .gap(px(style::S3))
                                .text_color(style::dim())
                                .child(counts)
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(style::S2))
                                        .child("Updated")
                                        .child(chrome::timestamp("map-captured-at", &at)),
                                )
                                .when(!self.current, |row| {
                                    row.child(div().text_color(style::warn()).child("May be stale"))
                                })
                                .child(crate::ui::info_hint(
                                    "map-capture-notes",
                                    format!("{captured}\n{COVERAGE}"),
                                )),
                        )
                    })
                    .children(setup_actions)
                    .child(glossary),
            )
            .when(self.dirty, |view| {
                view.child(
                    div()
                        .id("map-unsaved")
                        .role(Role::Status)
                        .aria_label(
                            "Unsaved map settings. Save or explicitly clear before Refresh.",
                        )
                        .px(px(style::INSET_CONTENT))
                        .py(px(style::S2))
                        .bg(style::warn_fill())
                        .text_color(style::warn())
                        .child("Unsaved map settings. Save or explicitly clear before Refresh."),
                )
            })
            .children(map_toolbar)
            .when_some(empty, |view, empty| {
                view.child(div().flex_1().min_h_0().child(empty))
            })
            .when(has_map, |view| view.child(self.canvas(cx)))
            .when(show_details, |view| view.child(inspection))
            .when_some(failure, |view, seq| {
                view.child(crate::ui::error_strip(
                    "map-error",
                    seq,
                    self.status.clone(),
                ))
            })
            .child(
                crate::ui::status_line()
                    .id("map-status")
                    .role(Role::Status)
                    .aria_label(self.status.clone())
                    .when(failure.is_none(), |status| {
                        status.child(self.status.clone())
                    }),
            )
    }
}
