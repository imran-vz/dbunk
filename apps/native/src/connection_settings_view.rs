//! Saved connection metadata. The parent supplies local records and owns Edit.
use crate::{accessible_editor::AccessibleEditor, connection_settings_model::Capture};
use dbunk_lib::backend::DevelopmentConnection;
use editor::Editor;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role,
    UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use std::{cell::Cell, rc::Rc};
gpui::actions!(connection_settings, [NextControl, PreviousControl]);
pub enum SettingsEvent {
    Back,
    Edit(String),
}
struct SelectedEditor {
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
}
pub struct ConnectionSettingsView {
    budget: Rc<Cell<usize>>,
    capture: Option<Capture>,
    editor: Option<SelectedEditor>,
    selected: usize,
    revision: u64,
    current: bool,
    editable: bool,
    message: &'static str,
    root: FocusHandle,
    list: FocusHandle,
    buttons: [FocusHandle; 2],
    scroll: UniformListScrollHandle,
}
impl EventEmitter<SettingsEvent> for ConnectionSettingsView {}
impl ConnectionSettingsView {
    pub fn new(budget: Rc<Cell<usize>>, cx: &mut Context<Self>) -> Self {
        Self {
            budget,
            capture: None,
            editor: None,
            selected: 0,
            revision: 0,
            current: false,
            editable: true,
            message: "Saved connection metadata is unavailable",
            root: cx.focus_handle(),
            list: cx.focus_handle(),
            buttons: [cx.focus_handle(), cx.focus_handle()],
            scroll: UniformListScrollHandle::new(),
        }
    }
    pub fn receive(
        &mut self,
        record: Option<&DevelopmentConnection>,
        expected_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.current = false;
        let (Some(record), Some(expected_id)) = (record, expected_id) else {
            self.editor = None;
            self.capture = None;
            self.message = "Saved connection no longer exists or this tab is unbound";
            cx.notify();
            return;
        };
        match Capture::new(record, expected_id, self.budget.clone()) {
            Ok(capture) => {
                // The new reservation overlaps the old editor and capture. Drop
                // the old editor before releasing its capture's allowance.
                self.editor = None;
                self.capture = Some(capture);
                self.selected = 0;
                self.revision = self.revision.wrapping_add(1);
                self.current = true;
                self.message = "Saved metadata only. Passwords are excluded; TLS files are not read. Defaults are not current session settings.";
            }
            Err(error) => self.message = error,
        }
        cx.notify();
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        if self.editable != editable {
            self.editable = editable;
            cx.notify();
        }
    }
    pub fn focus(&self) -> FocusHandle {
        if self.capture.is_some() {
            self.list.clone()
        } else {
            self.buttons[0].clone()
        }
    }
    pub fn contains_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.root.contains_focused(window, cx)
    }
    fn can_edit(&self) -> bool {
        self.editable && self.current && self.capture.as_ref().is_some_and(Capture::editable)
    }
    fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        match index {
            0 => cx.emit(SettingsEvent::Back),
            1 if self.can_edit() => cx.emit(SettingsEvent::Edit(
                self.capture.as_ref().unwrap().id().to_owned(),
            )),
            _ => {}
        }
    }
    fn select(&mut self, revision: u64, index: usize, cx: &mut Context<Self>) {
        if revision != self.revision
            || self
                .capture
                .as_ref()
                .is_none_or(|capture| index >= capture.count())
        {
            return;
        }
        self.selected = index;
        self.editor = None;
        self.scroll
            .scroll_to_item(index, gpui::ScrollStrategy::Nearest);
        cx.notify();
    }
    fn focus_control(&self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut order = vec![self.buttons[0].clone()];
        if self.can_edit() {
            order.push(self.buttons[1].clone());
        }
        if self.capture.is_some() {
            order.push(self.list.clone());
        }
        if let Some(editor) = &self.editor {
            order.push(editor.editor.focus_handle(cx));
        }
        let at = order
            .iter()
            .position(|focus| focus.contains_focused(window, cx));
        let next = if reverse {
            at.map_or(order.len() - 1, |i| (i + order.len() - 1) % order.len())
        } else {
            at.map_or(0, |i| (i + 1) % order.len())
        };
        window.focus(&order[next], cx);
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" => self.activate(0, cx),
            "tab" => self.focus_control(modifiers.shift, window, cx),
            "up" | "down" | "home" | "end" if self.list.is_focused(window) => {
                let count = self.capture.as_ref().map_or(0, Capture::count);
                let index = match event.keystroke.key.as_str() {
                    "up" => self.selected.saturating_sub(1),
                    "down" => (self.selected + 1).min(count.saturating_sub(1)),
                    "home" => 0,
                    _ => count.saturating_sub(1),
                };
                self.select(self.revision, index, cx);
            }
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }
    fn button(&self, index: usize, cx: &Context<Self>) -> gpui::AnyElement {
        let enabled = index == 0 || self.can_edit();
        let label = if index == 0 {
            "Administration"
        } else {
            "Edit connection"
        };
        let weak = cx.weak_entity();
        div()
            .id(("connection-settings-action", index))
            .role(Role::Button)
            .aria_label(label)
            .track_focus(&self.buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .border_1()
            .border_color(crate::style::line())
            .px_2()
            .py_1()
            .text_color(if enabled {
                crate::style::text()
            } else {
                crate::style::dim()
            })
            .focus(|s| s.bg(crate::style::line()))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.buttons[index], cx);
                this.activate(index, cx);
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| {
                    window.focus(&this.buttons[index], cx);
                    this.activate(index, cx);
                })
                .ok();
            })
            .child(label)
            .into_any_element()
    }
}
impl Render for ConnectionSettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.editor.is_none()
            && let Some(text) = self.capture.as_ref().and_then(|c| c.details(self.selected))
        {
            let editor = cx.new(|cx| {
                let buffer = cx.new(|cx| language::Buffer::local(text, cx));
                let mut editor = Editor::for_buffer(buffer, None, window, cx);
                editor.set_read_only(true);
                editor.set_soft_wrap_mode(language::language_settings::SoftWrap::None, cx);
                editor
            });
            let accessible = cx.new(|cx| {
                AccessibleEditor::new(
                    editor.clone(),
                    "Exact saved connection setting, read only",
                    cx,
                )
            });
            self.editor = Some(SelectedEditor { editor, accessible });
        }
        let count = self.capture.as_ref().map_or(0, Capture::count);
        div().id("connection-settings").key_context("ConnectionSettings").role(Role::Group).aria_label("Saved connection settings").track_focus(&self.root).size_full().flex().flex_col().bg(crate::style::bg()).text_color(crate::style::text()).text_xs()
            .capture_action(|_:&editor::actions::ToggleSoftWrap,_,cx|cx.stop_propagation())
            .capture_action(cx.listener(|this,_:&editor::actions::Cancel,_,cx|{this.activate(0,cx);cx.stop_propagation();}))
            .on_action(cx.listener(|this,_:&NextControl,window,cx|{this.focus_control(false,window,cx);cx.stop_propagation();}))
            .on_action(cx.listener(|this,_:&PreviousControl,window,cx|{this.focus_control(true,window,cx);cx.stop_propagation();}))
            .capture_key_down(cx.listener(|this,event,window,cx|this.key(event,window,cx)))
            .child(div().flex().gap_2().p_2().child(self.button(0,cx)).child(self.button(1,cx)))
            .child(div().id("connection-settings-status").role(Role::Status).aria_label(self.message).px_2().py_1().child(self.message))
            .when(!self.current&&self.capture.is_some(),|view|view.child(div().px_2().child("Previous saved metadata retained; Edit disabled until metadata reload succeeds")))
            .child(div().flex().flex_1().min_h_0()
                .child(div().id("connection-settings-fields").role(Role::ListBox).aria_label("Saved connection fields; Up and Down select").track_focus(&self.list).tab_stop(count>0).tab_index(0).w(px(280.)).min_h_0().border_r_1().border_color(crate::style::line()).child(
                    uniform_list("connection-setting-rows",count,cx.processor(|this,range:std::ops::Range<usize>,_,cx|{range.filter_map(|index|{let label=this.capture.as_ref()?.label(index)?;let revision=this.revision;let weak=cx.weak_entity();Some(div().id(("connection-setting",index)).role(Role::ListBoxOption).aria_label(label).aria_selected(this.selected==index).h(px(28.)).px_2().truncate().bg(if this.selected==index{crate::style::hover()}else{crate::style::bg()}).on_click(cx.listener(move |this,_,window,cx|{this.select(revision,index,cx);window.focus(&this.list,cx);})).on_a11y_action(gpui::accesskit::Action::Click,move |_,_,cx|{weak.update(cx,|this,cx|this.select(revision,index,cx)).ok();}).child(label))}).collect()})).track_scroll(&self.scroll).h_full()))
                .child(div().flex_1().min_w_0().min_h_0().when_some(self.editor.as_ref(),|view,editor|view.child(editor.accessible.clone()))))
    }
}

impl Drop for ConnectionSettingsView {
    fn drop(&mut self) {
        // Editor buffers and AX nodes must be released before their allowance.
        self.editor = None;
        self.capture = None;
    }
}
