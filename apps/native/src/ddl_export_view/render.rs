use super::*;
use gpui::{AnyElement, SharedString};
impl DdlExportView {
    fn button(&self, action: Action, label: &str, cx: &Context<Self>) -> AnyElement {
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
        let enabled = self.enabled(action);
        let weak = cx.weak_entity();
        let label: SharedString = label.to_owned().into();
        let button = match selected {
            Some(selected) => crate::ui::segment(("ddl-action", id), label, selected, enabled),
            None => crate::ui::tool_button(("ddl-action", id), label, None, enabled, false),
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
}
impl Render for DdlExportView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_fields(window, cx);
        let capture_status = self
            .capture
            .as_ref()
            .map(|c| c.status(self.section, self.page));
        let scope_changed = self
            .capture
            .as_ref()
            .is_some_and(|capture| self.request(cx).is_ok_and(|r| !capture.matches(&r)));
        div().id("ddl-export-view").key_context("DdlExport").role(Role::Group).aria_label("Read-only PostgreSQL DDL export").track_focus(&self.root).flex().flex_col().size_full().min_h_0().bg(crate::style::bg()).text_color(crate::style::text()).text_size(gpui::px(crate::style::FONT))
            .on_action(cx.listener(|this,_:&NextControl,window,cx|{if !this.composing(window,cx){this.focus_control(false,window,cx);cx.stop_propagation();}}))
            .on_action(cx.listener(|this,_:&PreviousControl,window,cx|{if !this.composing(window,cx){this.focus_control(true,window,cx);cx.stop_propagation();}}))
            .capture_action(|_:&editor::actions::ToggleSoftWrap,_,cx|cx.stop_propagation())
            .capture_action(cx.listener(|this,_:&editor::actions::Cancel,window,cx|{if !this.composing(window,cx){this.activate(Action::Back,window,cx);cx.stop_propagation();}}))
            .capture_key_down(cx.listener(|this,event:&KeyDownEvent,window,cx|this.key(event,window,cx)))
            .child(crate::ui::toolbar().children(ACTIONS.iter().map(|(action,label)|self.button(*action,label,cx))))
            .child(crate::ui::segmented().id("ddl-scopes").role(Role::TabList).aria_label("DDL capture scope").children(Scope::ALL.iter().enumerate().map(|(i,s)|self.button(Action::Scope(i),s.label(),cx))))
            .when(self.scope!=Scope::Database,|view|view.when_some(self.fields.first(),|view,field|view.child(div().flex().items_center().gap_2().px_2().py_1().child(div().text_color(crate::style::dim()).child("Schema")).child(div().flex_1().child(field.clone())))))
            .when(self.scope==Scope::Relation,|view|view.when_some(self.fields.get(1),|view,field|view.child(div().flex().items_center().gap_2().px_2().py_1().child(div().text_color(crate::style::dim()).child("Relation")).child(div().flex_1().child(field.clone())))))
            .child(label("ddl-boundaries","DDL reconstruction only, not a complete database dump or dependency-ordered migration. No SQL is executed. Review Capture and omissions before use.".into()))
            .when_some(capture_status,|view,status|view.child(label("ddl-capture-status",status)))
            .when(self.capture.is_some()&&(!self.capture_current||!self.ready),|view|view.child(label("ddl-stale","Retained historical capture. Refresh preserves observed identity; Clear capture permits inspecting a replacement. Saving uses the retained artifact.".into())))
            .when(scope_changed,|view|view.child(label("ddl-scope-changed","Controls differ from the captured scope. Capture DDL reads those names; preview and Save still use the retained capture.".into())))
            .child(crate::ui::segmented().id("ddl-sections").role(Role::TabList).aria_label("DDL capture sections").children(Section::ALL.iter().enumerate().map(|(i,s)|self.button(Action::Section(i),s.label(),cx))))
            .child(div().id("ddl-preview").role(Role::Group).aria_label("Exact read-only DDL preview page").flex_1().min_h_0().min_w_0().when_some(self.editor.as_ref(),|view,editor|view.child(editor.accessible.clone())).when(self.editor.is_none(),|view|view.child(div().p_2().text_color(crate::style::faint()).child("Connect and capture a scope to inspect its SQL and omissions."))))
            .child(label("ddl-runtime-status",self.status.clone()))
            .when_some(self.message.as_ref(),|view,message|view.child(label("ddl-message",message.clone())))
    }
}
fn label(id: &'static str, text: String) -> AnyElement {
    div()
        .id(id)
        .role(Role::Status)
        .aria_label(text.clone())
        .px_2()
        .py_1()
        .text_color(crate::style::dim())
        .child(text)
        .into_any_element()
}
