use super::*;
use gpui::{AnyElement, SharedString};
impl OverviewView {
    fn button(&self, action: Action, label: String, cx: &Context<Self>) -> AnyElement {
        let (id, focus, selected) = match action {
            Action::Scope(index) => (
                ACTIONS.len() + index,
                &self.scopes[index],
                Some(self.scope == Scope::ALL[index]),
            ),
            Action::Section(index) => (
                ACTIONS.len() + Scope::ALL.len() + index,
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
        let enabled = self.enabled(action, cx);
        let weak = cx.weak_entity();
        let label: SharedString = label.into();
        let button = match selected {
            Some(selected) => crate::ui::segment(("overview-action", id), label, selected, enabled),
            None => crate::ui::tool_button(("overview-action", id), label, None, enabled, false),
        };
        button
            .role(if selected.is_some() {
                Role::Tab
            } else {
                Role::Button
            })
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
    fn recent_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.recent_count();
        let summary = if self.recent_loading {
            "Reading recent queries; previous list retained until the read settles".to_owned()
        } else {
            self.recent.as_ref().map_or_else(
                || {
                    "Refresh recent queries to read this connection's history from this profile"
                        .into()
                },
                Recent::summary,
            )
        };
        let focus = self.recent_focus.min(count.saturating_sub(1));
        let rows = self.recent.as_ref().map(|recent| {
            let revision = recent.revision;
            recent
                .entries
                .iter()
                .take(count)
                .enumerate()
                .map(|(index, entry)| {
                    let action = Action::OpenRecent(revision, index);
                    let enabled = self.enabled(action, cx);
                    let label: SharedString = entry.label.clone().into();
                    let weak = cx.weak_entity();
                    div()
                        .id(("overview-recent-row", index))
                        .role(Role::Button)
                        .aria_label(format!(
                            "Recent query {} of {count}: {label}. Enter or Space opens the SQL as an unexecuted draft",
                            index + 1
                        ))
                        .track_focus(&self.recent_rows[index])
                        .tab_stop(enabled && index == focus)
                        .tab_index(0)
                        .a11y_synthetic_children(move |builder| {
                            if !enabled {
                                builder.parent_node().set_disabled();
                            }
                        })
                        .h(px(crate::style::ROW))
                        .mx(px(crate::style::S2))
                        .px(px(crate::style::S3))
                        .rounded(px(crate::style::R_SM))
                        .flex()
                        .items_center()
                        .truncate()
                        .text_color(if enabled { crate::style::text() } else { crate::style::faint() })
                        .when(enabled, |row| row.cursor_pointer().hover(|style| style.bg(crate::style::hover())))
                        .focus(|style| style.bg(crate::style::hover()))
                        // GPUI activates a focused clickable element on
                        // Enter/Space key-up; no key-down duplicate.
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.activate(action, window, cx)
                        }))
                        .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                            weak.update(cx, |this, cx| this.activate(action, window, cx))
                                .ok();
                        })
                        .child(label)
                        .into_any_element()
                })
                .collect::<Vec<_>>()
        });
        div()
            .id("overview-recent")
            .role(Role::Group)
            .aria_label(format!(
                "Recent queries for this connection, {count} listed; Up and Down move, Enter or Space opens as a draft"
            ))
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(crate::style::line())
            .child(div().h(px(crate::style::TOOLBAR)).px(px(crate::style::INSET_CONTENT)).flex().items_center().gap(px(crate::style::S2)).child(crate::ui::section_label("Recent queries")).child(crate::ui::grow()).child(self.button(Action::RefreshRecent,"Refresh recent queries".into(),cx)))
            .child(label("overview-recent-summary", summary))
            .child(
                div()
                    .id("overview-recent-rows")
                    .max_h(px(200.))
                    .overflow_y_scroll()
                    .track_scroll(&self.recent_scroll)
                    .when_some(rows, |list, rows| list.children(rows)),
            )
            .into_any_element()
    }
    fn rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self
            .capture
            .as_ref()
            .map_or(0, |capture| capture.count(self.section));
        uniform_list(
            "overview-rows",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                let focused = this.list.is_focused(window);
                let section = this.section;
                let revision = this.capture_revision;
                range
                    .filter_map(|index| {
                        let label: SharedString =
                            this.capture.as_ref()?.row_label(section, index)?.into();
                        let weak = cx.weak_entity();
                        Some(
                            crate::ui::list_row(
                                ("overview-row", index),
                                this.selected == Some(index),
                                focused,
                            )
                            .role(Role::ListBoxOption)
                            .aria_label(label.clone())
                            .aria_selected(this.selected == Some(index))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_row(revision, section, index, window, cx)
                            }))
                            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                                weak.update(cx, |this, cx| {
                                    this.select_row(revision, section, index, window, cx)
                                })
                                .ok();
                            })
                            .child(crate::ui::row_label(label)),
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
impl Render for OverviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_fields(window, cx);
        let count = self
            .capture
            .as_ref()
            .map_or(0, |capture| capture.count(self.section));
        let scope_matches = self.scope_matches(cx);
        let capture_status = self.capture.as_ref().map(Capture::status);
        let focused = [0, 1].map(|index| {
            self.fields
                .get(index)
                .is_some_and(|field| field.focus_handle(cx).contains_focused(window, cx))
        });
        div().id("overview-view").key_context("Overview").role(Role::Group).aria_label("Read-only PostgreSQL overview and statistics").track_focus(&self.root).flex().flex_col().size_full().min_h_0().bg(crate::style::bg()).text_color(crate::style::text()).text_size(px(crate::style::FONT))
            .on_action(cx.listener(|this,_:&NextControl,window,cx|{if !this.composing(window,cx){this.focus_control(false,window,cx);cx.stop_propagation();}}))
            .on_action(cx.listener(|this,_:&PreviousControl,window,cx|{if !this.composing(window,cx){this.focus_control(true,window,cx);cx.stop_propagation();}}))
            .capture_action(|_:&editor::actions::ToggleSoftWrap,_,cx|cx.stop_propagation())
            .capture_action(cx.listener(|this,_:&editor::actions::Cancel,window,cx|{if !this.composing(window,cx){this.activate(Action::Back,window,cx);cx.stop_propagation();}}))
            .capture_key_down(cx.listener(|this,event:&KeyDownEvent,window,cx|this.key(event,window,cx)))
            .child(crate::ui::toolbar_strip()
                .child(self.button(Action::Back,"Administration".into(),cx))
                .child(crate::ui::separator())
                .child(div().flex_none().font_weight(gpui::FontWeight::SEMIBOLD).text_color(crate::style::text()).child("Overview statistics"))
                .child(div().id("overview-connection-header").role(Role::Label).aria_label(format!("Connection: {}",self.header)).min_w_0().overflow_hidden().text_ellipsis().text_color(crate::style::dim()).child(self.header.clone()))
                .child(crate::ui::grow())
                .children(ACTIONS.iter().skip(1).take(4).filter(|(action,_)|matches!(action,Action::Refresh)||self.enabled(*action,cx)).map(|(action,label)|self.button(*action,(*label).into(),cx))))
            .child(div().flex_none().px(px(crate::style::INSET_CONTENT)).py(px(crate::style::S3)).flex().flex_wrap().items_end().gap(px(crate::style::S4))
                .child(crate::ui::labelled("Scope",crate::ui::segment_group().id("overview-scopes").role(Role::TabList).aria_label("Requested overview scope").children(Scope::ALL.iter().enumerate().map(|(index,scope)|self.button(Action::Scope(index),scope.label().into(),cx))),None))
                .when(self.scope!=Scope::Database,|row|row.when_some(self.fields.first(),|row,field|row.child(div().w(px(200.)).child(crate::ui::labelled("Schema",crate::ui::input_well(false,focused[0]).child(div().flex_1().min_w_0().child(field.clone())),None)))))
                .when(self.scope==Scope::Relation,|row|row.when_some(self.fields.get(1),|row,field|row.child(div().w(px(200.)).child(crate::ui::labelled("Relation",crate::ui::input_well(false,focused[1]).child(div().flex_1().min_w_0().child(field.clone())),None)))))
                .child(crate::ui::info_hint("overview-boundaries","Estimates are not exact counts. Pages are fresh captures; only one page is retained. Sizes are per relation, not recursive partition totals.")))
            .when_some(capture_status,|view,status|view.child(label("overview-capture-identity",status)))
            .when(self.capture.is_some()&&(!self.capture_current||!self.ready),|view|view.child(label("overview-stale","Retained capture may be stale. Refresh preserves its identity. To inspect a replacement, return to Administration and Clear captures.".into())))
            .when(self.capture.is_some()&&!scope_matches,|view|view.child(label("overview-scope-changed","Controls differ from the captured scope. Refresh to inspect this scope; Next is disabled.".into())))
            .child(div().flex_none().px(px(crate::style::INSET_CONTENT)).pb(px(crate::style::S3)).flex().child(crate::ui::segment_group().id("overview-sections").role(Role::TabList).aria_label("Overview sections").children(Section::ALL.iter().enumerate().map(|(index,section)|self.button(Action::Section(index),format!("{} ({})",section.label(),self.capture.as_ref().map_or(0,|capture|capture.count(*section))),cx)))))
            .child(div().flex().flex_1().min_h_0()
                .child(div().id("overview-list").role(Role::ListBox).aria_label(format!("{} captured {}; arrows select, Enter inspects",count,self.section.label())).track_focus(&self.list).tab_stop(true).tab_index(0).w(px(360.)).min_h_0().py(px(crate::style::S2)).flex().flex_col().border_r_1().border_color(crate::style::line())
                    .when(count==0,|view|view.child(div().px(px(crate::style::INSET_CONTENT)).py(px(crate::style::S3)).text_color(crate::style::dim()).child(self.capture.as_ref().map_or("Connect and refresh to capture statistics",|capture|capture.empty_label(self.section)))))
                    .when(count>0,|view|view.child(self.rows(cx))))
                .child(div().id("overview-selected-details").role(Role::Group).aria_label("Exact selected overview statistics, read only").flex_1().min_w_0().min_h_0().when_some(self.editor.as_ref(),|view,editor|view.child(editor.accessible.clone()))))
            .child(self.recent_section(cx))
            .child(label("overview-runtime-status",self.status.clone()))
            .when_some(self.message.as_ref(),|view,message|view.child(div().px(px(crate::style::INSET_CONTENT)).py(px(crate::style::S2)).child(crate::ui::shake(format!("overview-message-shake-{message}"),crate::ui::error_banner("overview-message",message.clone())))))
    }
}
fn label(id: &'static str, text: String) -> AnyElement {
    div()
        .id(id)
        .role(Role::Status)
        .aria_label(text.clone())
        .px(px(crate::style::INSET_CONTENT))
        .py(px(crate::style::S2))
        .text_size(px(crate::style::FONT_SMALL))
        .text_color(crate::style::dim())
        .child(text)
        .into_any_element()
}
