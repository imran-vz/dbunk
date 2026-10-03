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
    fn button(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (action, default) = ACTIONS[index];
        let label = self.label(action, default);
        let enabled = self.enabled(action);
        let weak = cx.weak_entity();
        div()
            .id(("map-control", index))
            .role(Role::Button)
            .aria_label(label.clone())
            .a11y_synthetic_children(move |b| {
                if !enabled {
                    b.parent_node().set_disabled();
                }
            })
            .track_focus(&self.buttons[index])
            .tab_index(0)
            .tab_stop(enabled)
            .px_2()
            .py_1()
            .text_color(rgb(if enabled { 0xffffff } else { 0x777777 }))
            .focus(|s| s.bg(rgb(0x333333)))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
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
        div()
            .id(("map-scope", index))
            .role(Role::Tab)
            .aria_label(label)
            .aria_selected(selected)
            .a11y_synthetic_children(move |b| {
                if !enabled {
                    b.parent_node().set_disabled();
                }
            })
            .track_focus(&self.scope_buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .px_2()
            .py_1()
            .when(selected, |s| s.bg(rgb(0x252525)))
            .focus(|s| s.bg(rgb(0x333333)))
            .child(label)
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
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| {
                        let label = this.row_label(index);
                        let is_selected = selected == Some(index);
                        let selection = this.selection_at(index);
                        let weak = cx.weak_entity();
                        div()
                            .id(("map-object", index))
                            .role(Role::ListBoxOption)
                            .aria_label(label.clone())
                            .aria_selected(is_selected)
                            .h(px(25.))
                            .px_2()
                            .truncate()
                            .when(is_selected, |row| row.bg(rgb(0x252525)))
                            .child(label)
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
            .w(px(380.)).min_h_0().child(rows)
    }
}
impl Render for SchemaMapView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let toolbar = div()
            .flex()
            .flex_wrap()
            .children((0..ACTIONS.len()).map(|i| self.button(i, cx)));
        let scopes = div()
            .id("map-scopes")
            .role(Role::TabList)
            .aria_label("Map setup scope")
            .flex()
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
            .p_2()
            .child(self.detail_text.clone());
        let inspection = div()
            .flex()
            .h(px(170.))
            .min_h_0()
            .child(self.object_list(cx))
            .child(details);
        div().id("schema-map-tool").role(Role::Group)
            .aria_label(format!("PostgreSQL schema map for {}", self.connection.as_deref().unwrap_or("unbound connection")))
            .track_focus(&self.root).size_full().flex().flex_col()
            .bg(rgb(0)).text_color(rgb(0xffffff)).key_context("SchemaMap")
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
            .child(toolbar).child(scopes)
            .when(self.scope != Scope::Database, |view| {
                view.child(div().flex().gap_2().px_2()
                    .child(div().flex_1().min_w_0().child("Schema").children(self.fields.first().cloned()))
                    .when(self.scope == Scope::Relation, |row| {
                        row.child(div().flex_1().min_w_0().child("Table").children(self.fields.get(1).cloned()))
                    }))
            })
            .child(div().id("map-status").role(Role::Status)
                .aria_label(self.status.clone()).px_2().child(self.status.clone()))
            .child(div().id("map-captured-scope").role(Role::Label)
                .aria_label(captured.clone()).px_2().child(captured))
            .when(self.dirty, |view| view.child(div().id("map-unsaved").role(Role::Status)
                .aria_label("Unsaved map settings. Save or explicitly clear before Refresh.")
                .px_2().child("Unsaved map settings. Save or explicitly clear before Refresh.")))
            .child(self.canvas(cx)).child(inspection)
            .child(div().id("map-coverage").role(Role::Label)
                .aria_label("Read-only table graph, including partitioned tables. Inherited foreign-key and trigger copies are omitted. Direct-neighbor scope shows only focal edges. SVG and PNG save the current viewport on white; no row data or DDL is executed.")
                .px_2().text_xs().child("Read-only table graph. Inherited FK/trigger copies omitted. Direct-neighbor scope keeps focal edges. SVG/PNG export this viewport on white."))
    }
}
