use super::*;
use crate::{
    structure_table::{self as table, INSET},
    style,
};
use gpui::{AnyElement, SharedString, Stateful};

const OUTLINE: f32 = 176.;
const DETAILS: f32 = 340.;
/// Height of the exact catalog text under Diagnostics.
const DIAGNOSTICS: f32 = 220.;

impl StructureView {
    /// Focus, AX, click and keyboard wiring shared by every control.
    fn wire(&self, action: Action, element: Stateful<gpui::Div>, cx: &Context<Self>) -> AnyElement {
        let (focus, selected) = match action {
            Action::Section(index) => (
                &self.sections[index],
                Some(self.section == Section::ALL[index]),
            ),
            _ => {
                let index = ACTIONS
                    .iter()
                    .position(|(item, _)| *item == action)
                    .unwrap();
                (&self.buttons[index], None)
            }
        };
        let enabled = self.enabled(action);
        let weak = cx.weak_entity();
        element
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
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .into_any_element()
    }
    fn button(&self, action: Action, icon: Option<&'static str>, cx: &Context<Self>) -> AnyElement {
        let index = ACTIONS
            .iter()
            .position(|(item, _)| *item == action)
            .unwrap();
        let selected = action == Action::Details && self.show_details;
        let button = crate::ui::pressed(
            crate::ui::tool_button(
                ("structure-control", index),
                ACTIONS[index].1,
                icon,
                self.enabled(action),
                false,
            ),
            selected,
        )
        .role(Role::Button);
        self.wire(action, button, cx)
    }
    fn toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let identity = self.identity();
        crate::ui::toolbar_strip()
            .id("structure-identity")
            .role(Role::Toolbar)
            .aria_label(format!(
                "{} structure, relation OID {}, captured {}",
                self.capture.qualified_name(),
                identity.relation_oid,
                self.capture.captured_at()
            ))
            .child(
                gpui::svg()
                    .path("icons/table.svg")
                    .size(px(style::ICON))
                    .flex_none()
                    .text_color(style::kind_table()),
            )
            .child(crate::ui::crumbs(
                format!("{}.", self.capture.schema()),
                self.capture.table().to_owned(),
            ))
            .child(crate::ui::badge(self.capture.kind_label()))
            .child(
                div()
                    .id("structure-updated")
                    .min_w_0()
                    .truncate()
                    .text_size(px(style::FONT_SMALL))
                    .text_color(style::dim())
                    .tooltip(crate::ui::tooltip(format!(
                        "Captured {}",
                        self.capture.captured_at()
                    )))
                    .tooltip_show_delay(crate::ui::tooltip_delay())
                    .child(
                        super::details::updated_label(self.capture.captured_at())
                            .unwrap_or_else(|| format!("Captured {}", self.capture.captured_at())),
                    ),
            )
            .child(crate::ui::grow())
            .when(self.busy, |bar| {
                bar.child(self.button(Action::Cancel, Some("icons/stop.svg"), cx))
            })
            .child(self.button(Action::EditStructure, Some("icons/pencil.svg"), cx))
            .child(self.button(Action::Refresh, Some("icons/rotate_cw.svg"), cx))
            .child(self.button(
                Action::Details,
                Some("icons/threads_sidebar_right_open.svg"),
                cx,
            ))
            .child(crate::ui::separator())
            .child(self.button(Action::Back, Some("icons/arrow_left.svg"), cx))
            .into_any_element()
    }
    /// Section outline: every section with its count; empty ones are faint.
    fn outline(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .id("structure-sections")
            .role(Role::TabList)
            .aria_label("Table structure sections")
            .flex_none()
            .w(px(OUTLINE))
            .h_full()
            .py(px(style::S4))
            .flex()
            .flex_col()
            .gap(px(1.))
            .border_r_1()
            .border_color(style::line_soft())
            .bg(style::panel())
            .children(Section::ALL.iter().enumerate().map(|(index, section)| {
                let count = self.capture.count(*section);
                let selected = self.section == *section;
                let label = format!("{} ({count})", section.label());
                // The shared list row: full-width inset pill, `raised` for
                // the open section (the page, not the outline, holds focus).
                let item = crate::ui::list_row(
                    ("structure-control", ACTIONS.len() + index),
                    selected,
                    false,
                )
                .role(Role::Tab)
                .aria_label(label)
                .flex_none()
                .gap(px(style::S3))
                .text_sm()
                .cursor_pointer()
                .text_color(if selected {
                    style::text()
                } else if count == 0 {
                    style::faint()
                } else {
                    style::dim()
                })
                .child(div().flex_1().min_w_0().truncate().child(section.label()))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(style::FONT_SMALL))
                        .text_color(style::dim())
                        .child(count.to_string()),
                );
                self.wire(Action::Section(index), item, cx)
            }))
            .into_any_element()
    }
    /// A section's column layout; Columns uses the content-fitted one.
    fn layout(&self, section: Section) -> &[table::ColumnSpec] {
        if section == Section::Columns && !self.columns_layout.is_empty() {
            self.columns_layout.as_slice()
        } else {
            section.columns()
        }
    }
    fn item(&self, position: usize, cx: &Context<Self>) -> AnyElement {
        let line = table::line();
        let Some(item) = self.page.item(position) else {
            return line.into_any_element();
        };
        match item {
            Item::Title(section) => table::title_line(
                section.label(),
                self.capture.count(section),
                Some(self.capture.empty_label(section)),
            )
            .into_any_element(),
            Item::Note(_) => line
                .flex()
                .items_center()
                .child(
                    div()
                        .id("structure-acl-scope")
                        .role(Role::Label)
                        .aria_label(crate::table_structure_model::RELATION_ACL_SCOPE)
                        .min_w_0()
                        .truncate()
                        .text_size(px(style::FONT_SMALL))
                        .text_color(style::dim())
                        .tooltip(crate::ui::tooltip(
                            crate::table_structure_model::RELATION_ACL_SCOPE,
                        ))
                        .child(crate::table_structure_model::RELATION_ACL_SCOPE),
                )
                .into_any_element(),
            Item::Head(section) => table::head_line(self.layout(section)).into_any_element(),
            Item::Row(section, index) => {
                let selected = self.section == section && self.selected == Some(index);
                let last = index + 1 == self.capture.count(section);
                let label: SharedString = self
                    .capture
                    .row_label(section, index)
                    .unwrap_or_default()
                    .into();
                let navigable = self.capture.navigation(section, index).is_some();
                let mut cells = self
                    .capture
                    .cells(section, index)
                    .unwrap_or_default()
                    .into_iter();
                let weak = cx.weak_entity();
                line.id(("structure-row", position))
                    .role(Role::ListBoxOption)
                    .aria_label(label)
                    .aria_selected(selected)
                    .cursor_pointer()
                    .group("structure-row")
                    .child(
                        table::card(false, last)
                            .text_color(style::text())
                            .bg(if selected {
                                style::select()
                            } else {
                                style::bg()
                            })
                            .when(!selected, |card| {
                                card.group_hover("structure-row", |s| s.bg(style::row_hover()))
                            })
                            .children(
                                self.layout(section)
                                    .iter()
                                    .map(|spec| table::cell(spec, cells.next())),
                            ),
                    )
                    .on_click(
                        cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                            this.select_row(section, index, window, cx);
                            // Double-click follows a relationship to its table.
                            if navigable && event.click_count() == 2 {
                                this.activate(Action::Open, window, cx);
                            }
                        }),
                    )
                    .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                        weak.update(cx, |this, cx| this.select_row(section, index, window, cx))
                            .ok();
                    })
                    .into_any_element()
            }
        }
    }
    fn page(&self, cx: &mut Context<Self>) -> AnyElement {
        let label = format!(
            "{} rows in {} sections; arrows select, Enter inspects",
            self.page.rows,
            Section::ALL.len()
        );
        div()
            .id("structure-list")
            .role(Role::ListBox)
            .aria_label(label)
            .track_focus(&self.list)
            .tab_stop(true)
            .tab_index(0)
            .flex_1()
            .min_w_0()
            .h_full()
            .pb(px(style::S5))
            .child(
                uniform_list(
                    "structure-rows",
                    self.page.lines,
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range.map(|position| this.item(position, cx)).collect()
                    }),
                )
                .track_scroll(&self.scroll)
                .size_full(),
            )
            .into_any_element()
    }
    /// The Diagnostics disclosure: the exact catalog text, ids and capture
    /// time, read only.
    fn diagnostics_toggle(&self, cx: &Context<Self>) -> AnyElement {
        let open = self.show_raw;
        let weak = cx.weak_entity();
        div()
            .id("structure-diagnostics")
            .role(Role::Button)
            .aria_label("Diagnostics")
            .aria_expanded(open)
            .flex_none()
            .h(px(style::CONTROL + style::S2))
            .px(px(style::S5))
            .flex()
            .items_center()
            .gap(px(style::S3))
            .border_t_1()
            .border_color(style::line_soft())
            .text_sm()
            .text_color(style::dim())
            .cursor_pointer()
            .hover(|s| s.text_color(style::text()))
            .child(
                gpui::svg()
                    .path(if open {
                        "icons/chevron_down.svg"
                    } else {
                        "icons/chevron_right.svg"
                    })
                    .size(px(style::ICON))
                    .flex_none()
                    .text_color(style::dim()),
            )
            .child("Diagnostics")
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_raw = !this.show_raw;
                cx.notify();
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    this.show_raw = !this.show_raw;
                    cx.notify();
                })
                .ok();
            })
            .into_any_element()
    }
    /// Exact selected details with the actions that apply to the selection.
    fn details(&self, cx: &Context<Self>) -> AnyElement {
        let title: SharedString = self
            .selected
            .and_then(|index| self.capture.row_label(self.section, index))
            .map_or_else(
                || self.section.label().to_owned(),
                |label| format!("{} · {label}", self.section.label()),
            )
            .into();
        let navigable = matches!(
            self.section,
            Section::ForeignKeys | Section::ReferencedBy | Section::Parents | Section::Children
        );
        let editable = matches!(self.section, Section::Overview | Section::Columns);
        div()
            .id("structure-selected-details")
            .role(Role::Group)
            .aria_label("Exact selected structure details, read only")
            .flex_none()
            .w(px(DETAILS))
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(style::line())
            .child(
                crate::ui::toolbar_strip()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(style::text())
                            .child(title),
                    )
                    .when(navigable, |bar| {
                        bar.child(self.button(Action::Open, Some("icons/arrow_up_right.svg"), cx))
                    })
                    .when(editable, |bar| {
                        bar.child(self.button(Action::Edit, Some("icons/pencil.svg"), cx))
                    })
                    .child(self.button(Action::Copy, Some("icons/copy.svg"), cx)),
            )
            .child(
                div()
                    .id("structure-details-body")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .map(|body| match &self.editor {
                        Some(editor) => body
                            .child(
                                div()
                                    .id("structure-properties")
                                    .role(Role::Group)
                                    .aria_label("Selected structure properties")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .px(px(style::S5))
                                    .py(px(style::S4))
                                    .child(crate::ui::property_list(self.properties.clone())),
                            )
                            .child(self.diagnostics_toggle(cx))
                            .when(self.show_raw, |body| {
                                body.child(
                                    div()
                                        .flex_none()
                                        .h(px(DIAGNOSTICS))
                                        .border_t_1()
                                        .border_color(style::line_soft())
                                        .child(editor.accessible.clone()),
                                )
                            }),
                        None => body
                            .p(px(style::S5))
                            .text_sm()
                            .text_color(style::dim())
                            .child(if self.capture.count(self.section) == 0 {
                                self.capture.empty_label(self.section)
                            } else {
                                "Select a row to see its details"
                            }),
                    }),
            )
            .into_any_element()
    }
}
impl Render for StructureView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.message.clone().unwrap_or_else(|| self.status.clone());
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
            .bg(style::bg())
            .text_color(style::text())
            .text_size(px(style::FONT))
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
            .child(self.toolbar(cx))
            .when(!self.capture_current || !self.ready, |view| {
                view.child(
                    div()
                        .id("structure-stale")
                        .role(Role::Status)
                        .aria_label("Retained capture may be stale. Refresh before navigation.")
                        .flex_none()
                        .px(px(INSET))
                        .py(px(style::S2))
                        .bg(style::warn_fill())
                        .border_b_1()
                        .border_color(style::line_soft())
                        .text_color(style::warn())
                        .child("Retained capture may be stale. Refresh before navigation."),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.outline(cx))
                    .child(self.page(cx))
                    .when(self.show_details, |body| body.child(self.details(cx))),
            )
            .child(
                crate::ui::status_line().child(
                    div()
                        .id("structure-runtime-status")
                        .role(Role::Status)
                        .aria_label(status.clone())
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(style::dim())
                        .child(status),
                ),
            )
    }
}
