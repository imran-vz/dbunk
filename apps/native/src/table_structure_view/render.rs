use super::*;
use gpui::{AnyElement, SharedString};

impl StructureView {
    fn button(&self, action: Action, label: String, cx: &Context<Self>) -> AnyElement {
        let (id, focus, selected) = match action {
            Action::Section(index) => (
                ACTIONS.len() + index,
                &self.sections[index],
                Some(self.section == Section::ALL[index]),
            ),
            _ => {
                let index = ACTIONS
                    .iter()
                    .position(|(item, _)| *item == action)
                    .unwrap();
                (index, &self.buttons[index], None)
            }
        };
        let enabled = self.enabled(action);
        let weak = cx.weak_entity();
        let label: SharedString = label.into();
        div()
            .id(("structure-control", id))
            .role(if selected.is_some() {
                Role::Tab
            } else {
                Role::Button
            })
            .aria_label(label.clone())
            .track_focus(focus)
            .tab_stop(enabled)
            .tab_index(0)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
                if let Some(selected) = selected {
                    builder.parent_node().set_selected(selected);
                }
            })
            .px_2()
            .py_1()
            .border_1()
            .border_color(crate::style::line())
            .bg(if selected == Some(true) {
                crate::style::hover()
            } else {
                crate::style::bg()
            })
            .text_color(if enabled {
                crate::style::text()
            } else {
                crate::style::dim()
            })
            .focus(|style| style.bg(crate::style::line()))
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .child(label)
            .into_any_element()
    }
    fn rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.capture.count(self.section);
        uniform_list(
            "structure-rows",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let section = this.section;
                range
                    .filter_map(|index| {
                        let label: SharedString = this.capture.row_label(section, index)?.into();
                        let weak = cx.weak_entity();
                        Some(
                            div()
                                .id(("structure-row", index))
                                .role(Role::ListBoxOption)
                                .aria_label(label.clone())
                                .aria_selected(this.selected == Some(index))
                                .h(px(28.))
                                .px_2()
                                .overflow_hidden()
                                .bg(if this.selected == Some(index) {
                                    crate::style::hover()
                                } else {
                                    crate::style::bg()
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.select_row(section, index, window, cx)
                                }))
                                .on_a11y_action(
                                    gpui::accesskit::Action::Click,
                                    move |_, window, cx| {
                                        weak.update(cx, |this, cx| {
                                            this.select_row(section, index, window, cx)
                                        })
                                        .ok();
                                    },
                                )
                                .child(label),
                        )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .h_full()
        .into_any_element()
    }
}
impl Render for StructureView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let identity = self.identity();
        let header = format!(
            "{} | Database OID {} | Relation OID {} | Captured {}",
            self.capture.qualified_name(),
            identity.database_oid,
            identity.relation_oid,
            self.capture.captured_at()
        );
        let count = self.capture.count(self.section);
        div()
            .id("table-structure")
            .key_context("TableStructure")
            .role(Role::Group)
            .aria_label("Table structure, read only")
            .track_focus(&self.root)
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .text_xs()
            .on_action(cx.listener(|this, _: &NextControl, window, cx| {
                this.focus_control(false, window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &PreviousControl, window, cx| {
                this.focus_control(true, window, cx);
                cx.stop_propagation();
            }))
            .capture_action(|_: &editor::actions::ToggleSoftWrap, _, cx| {
                // The selected-detail allowance counts logical AX runs.
                cx.stop_propagation();
            })
            .capture_action(
                cx.listener(|this, _: &editor::actions::Cancel, window, cx| {
                    this.activate(Action::Back, window, cx);
                    cx.stop_propagation();
                }),
            )
            .capture_key_down(
                cx.listener(|this, event: &KeyDownEvent, window, cx| this.key(event, window, cx)),
            )
            .child(
                div().flex().flex_wrap().gap_1().p_2().children(
                    ACTIONS
                        .iter()
                        .map(|(action, label)| self.button(*action, (*label).into(), cx)),
                ),
            )
            .child(
                div()
                    .id("structure-identity")
                    .role(Role::Label)
                    .aria_label(header.clone())
                    .px_2()
                    .py_1()
                    .child(header),
            )
            .when(!self.capture_current || !self.ready, |view| {
                view.child(
                    div()
                        .id("structure-stale")
                        .role(Role::Status)
                        .aria_label("Retained capture may be stale. Refresh before navigation.")
                        .px_2()
                        .py_1()
                        .child("Retained capture may be stale. Refresh before navigation."),
                )
            })
            .child(
                div()
                    .id("structure-sections")
                    .role(Role::TabList)
                    .aria_label("Table structure sections")
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .children(Section::ALL.iter().enumerate().map(|(index, section)| {
                        self.button(
                            Action::Section(index),
                            format!("{} ({})", section.label(), self.capture.count(*section)),
                            cx,
                        )
                    })),
            )
            .when(self.section == Section::RelationGrants, |view| {
                view.child(
                    div()
                        .id("structure-acl-scope")
                        .role(Role::Label)
                        .aria_label(crate::table_structure_model::RELATION_ACL_SCOPE)
                        .px_2()
                        .py_1()
                        .child(crate::table_structure_model::RELATION_ACL_SCOPE),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("structure-list")
                            .role(Role::ListBox)
                            .aria_label(format!(
                                "{} captured {}; arrows select, Enter inspects",
                                count,
                                self.section.label()
                            ))
                            .track_focus(&self.list)
                            .tab_stop(true)
                            .tab_index(0)
                            .w(px(340.))
                            .min_h_0()
                            .border_r_1()
                            .border_color(crate::style::line())
                            .when(count == 0, |list| {
                                list.child(
                                    div().p_2().child(self.capture.empty_label(self.section)),
                                )
                            })
                            .when(count > 0, |list| list.child(self.rows(cx))),
                    )
                    .child(
                        div()
                            .id("structure-selected-details")
                            .role(Role::Group)
                            .aria_label("Exact selected structure details, read only")
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .when_some(self.editor.as_ref(), |view, editor| {
                                view.child(editor.accessible.clone())
                            }),
                    ),
            )
            .child(
                div()
                    .id("structure-runtime-status")
                    .role(Role::Status)
                    .aria_label(self.status.clone())
                    .px_2()
                    .py_1()
                    .child(self.status.clone()),
            )
            .when_some(self.message.as_ref(), |view, message| {
                view.child(
                    div()
                        .id("structure-message")
                        .role(Role::Status)
                        .aria_label(message.clone())
                        .px_2()
                        .py_1()
                        .child(message.clone()),
                )
            })
    }
}
