//! A bounded array list with one text editor. Element controls never stage a
//! mutation themselves; the parent keeps the original literal and row guards.
use super::*;
use cell_value::{ArrayElements, ValueError};

const PAGE_ITEMS: usize = 16;
// Raw input is capped at 1 MiB, but encoded control characters can expand
// sixfold. Cover model, cached literal, field snapshot and staging/format copies.
const RESERVATION: usize = 32 * 1024 * 1024;
#[derive(Clone, Copy, Debug)]
enum ArrayAction {
    Select(usize),
    Previous,
    Next,
    Add,
    Remove,
    Null,
}
pub(super) enum ArrayEvent {
    Stage,
    Cancel,
}
pub(super) struct ArrayView {
    model: ArrayElements,
    selected: usize,
    start: usize,
    null: bool,
    enabled: bool,
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    initial_preview: String,
    message: Option<String>,
    focus: HashMap<String, FocusHandle>,
    order: Vec<FocusHandle>,
    rendered: Vec<FocusHandle>,
    budget: Rc<Cell<usize>>,
    history: literal_guard::History,
    field_events: Option<gpui::Subscription>,
}
impl EventEmitter<ArrayEvent> for ArrayView {}

pub(super) fn create(
    text: &str,
    budget: Rc<Cell<usize>>,
    window: &mut Window,
    cx: &mut Context<TableChanges>,
) -> Result<Entity<ArrayView>, String> {
    if budget.get() > 128 * 1024 * 1024 - RESERVATION {
        return Err("Array editor needs 32 MiB of workspace allowance; use the raw literal or clear another result".into());
    }
    let model = ArrayElements::parse(text).map_err(|error| error.to_string())?;
    let initial_preview = preview(text);
    budget.set(budget.get() + RESERVATION);
    Ok(cx.new(|cx| {
        let editor = cx.new(|cx| {
            Editor::for_buffer(
                cx.new(|cx| language::Buffer::local("", cx)),
                None,
                window,
                cx,
            )
        });
        let accessible =
            cx.new(|cx| AccessibleEditor::new(editor.clone(), "Selected array element text", cx));
        let mut view = ArrayView {
            model,
            selected: 0,
            start: 0,
            null: false,
            enabled: true,
            editor,
            accessible,
            initial_preview,
            message: None,
            focus: HashMap::from([
                ("array-add".into(), cx.focus_handle()),
                ("array-null".into(), cx.focus_handle()),
            ]),
            order: vec![],
            rendered: vec![],
            budget,
            history: literal_guard::History::new(String::new()),
            field_events: None,
        };
        view.show_selected(window, cx);
        view
    }))
}
pub(super) fn literal_editor(
    text: String,
    label: String,
    window: &mut Window,
    cx: &mut Context<TableChanges>,
) -> (Entity<Editor>, Entity<AccessibleEditor>) {
    let editor = cx.new(|cx| {
        Editor::for_buffer(
            cx.new(|cx| language::Buffer::local(text, cx)),
            None,
            window,
            cx,
        )
    });
    let accessible = cx.new(|cx| AccessibleEditor::new(editor.clone(), label, cx));
    (editor, accessible)
}
impl Drop for ArrayView {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(RESERVATION));
    }
}
impl ArrayView {
    pub(super) fn handles(&self, cx: &App, disabled: bool) -> Vec<FocusHandle> {
        let mut handles = if disabled {
            self.rendered.clone()
        } else {
            self.order.clone()
        };
        if disabled || (self.enabled && !self.null && !self.model.items().is_empty()) {
            handles.push(self.editor.focus_handle(cx));
        }
        handles
    }
    pub(super) fn focus(&self, window: &mut Window, cx: &mut App) {
        let fallback = if self.model.items().is_empty() {
            "array-add"
        } else {
            "array-null"
        };
        let focus = if self.null || self.model.items().is_empty() {
            self.focus
                .get(fallback)
                .cloned()
                .unwrap_or_else(|| self.editor.focus_handle(cx))
        } else {
            self.editor.focus_handle(cx)
        };
        window.focus(&focus, cx);
    }
    pub(super) fn composition_active(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.editor.focus_handle(cx).contains_focused(window, cx)
            && self.editor.update(cx, |editor, cx| {
                gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
            })
    }
    pub(super) fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.editor.update(cx, |editor, _| {
                editor.set_read_only(!enabled || self.null || self.model.items().is_empty())
            });
            cx.notify();
        }
    }
    /// None means the literal is untouched and the parent retains its spelling.
    pub(super) fn replacement(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Option<String>, String> {
        if self.editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
        }) {
            return Err(
                "Finish composing the array element before staging or switching modes".into(),
            );
        }
        self.flush(cx).map_err(|error| error.to_string())?;
        Ok(self.model.literal().map(str::to_owned))
    }
    fn flush(&mut self, cx: &mut Context<Self>) -> Result<(), ValueError> {
        if self.model.items().is_empty() {
            return Ok(());
        }
        if self.editor.read(cx).buffer().read(cx).len(cx).0 > cell_value::MAX_VALUE_BYTES {
            return Err(ValueError::InputTooLarge);
        }
        let value = if self.null {
            None
        } else {
            Some(self.editor.read(cx).text(cx))
        };
        self.model.replace(self.selected, value)
    }
    fn show_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = self
            .selected
            .min(self.model.items().len().saturating_sub(1));
        let value = self.model.items().get(self.selected);
        self.null = value.is_some_and(Option::is_none);
        // A different element gets a fresh buffer so its hidden undo history
        // cannot retain every previously selected value.
        let text = value.and_then(Option::as_deref).unwrap_or("").to_owned();
        self.history = literal_guard::History::new(text.clone());
        let read_only = !self.enabled || self.null || value.is_none();
        self.editor = cx.new(|cx| {
            let mut editor = Editor::for_buffer(
                cx.new(|cx| language::Buffer::local(text, cx)),
                None,
                window,
                cx,
            );
            editor.set_read_only(read_only);
            editor
        });
        self.accessible = cx.new(|cx| {
            AccessibleEditor::new(
                self.editor.clone(),
                format!("Array element {} text", self.selected + 1),
                cx,
            )
        });
        self.field_events = Some(self.subscribe_field(window, cx));
    }
    fn subscribe_field(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Subscription {
        cx.subscribe_in(&self.editor, window, |this, editor, event, window, cx| {
            if editor != &this.editor || !literal_guard::relevant(event) {
                return;
            }
            this.check_field(
                matches!(event, editor::EditorEvent::BufferEdited),
                window,
                cx,
            );
        })
    }
    fn check_field(&mut self, edited: bool, window: &mut Window, cx: &mut Context<Self>) {
        let length = self.editor.read(cx).buffer().read(cx).len(cx).0;
        let marked = self.editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
        });
        let change = self.history.change(length, marked, edited);
        if change == literal_guard::Change::Keep {
            if !marked {
                self.history.committed = self.editor.read(cx).text(cx);
            }
            return;
        }
        let refused = change == literal_guard::Change::Refuse;
        let value = if refused {
            self.history.committed.clone()
        } else {
            self.editor.read(cx).text(cx)
        };
        self.history = literal_guard::History::new(value.clone());
        let (editor, accessible) = literal_guard::replace(
            &self.editor,
            value,
            refused,
            format!("Array element {} text", self.selected + 1),
            true,
            window,
            cx,
        );
        editor.update(cx, |editor, _| {
            editor.set_read_only(!self.enabled || self.null || self.model.items().is_empty())
        });
        self.editor = editor;
        self.accessible = accessible;
        cx.defer_in(window, |this, window, cx| {
            this.field_events = Some(this.subscribe_field(window, cx));
        });
        self.message = Some(literal_guard::notice(refused).into());
        cx.notify();
    }
    fn action(&mut self, action: ArrayAction, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        if self.editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
        }) {
            return;
        }
        self.message = None;
        // Removing an element explicitly discards its in-progress field. Every
        // other action keeps invalid input in place and refuses navigation.
        let result = if matches!(action, ArrayAction::Remove) {
            Ok(())
        } else {
            self.flush(cx)
        };
        if let Err(error) = result {
            self.message = Some(error.to_string());
            cx.notify();
            return;
        }
        let result = match action {
            ArrayAction::Select(index) => {
                self.selected = index;
                Ok(())
            }
            ArrayAction::Previous => {
                self.start = self.start.saturating_sub(PAGE_ITEMS);
                self.selected = self.start;
                Ok(())
            }
            ArrayAction::Next => {
                self.start =
                    (self.start + PAGE_ITEMS).min(self.model.items().len().saturating_sub(1));
                self.selected = self.start;
                Ok(())
            }
            ArrayAction::Add => self.model.append().map(|index| {
                self.selected = index;
                self.start = index / PAGE_ITEMS * PAGE_ITEMS;
            }),
            ArrayAction::Remove => self.model.remove(self.selected).map(|()| {
                self.selected = self
                    .selected
                    .min(self.model.items().len().saturating_sub(1));
                self.start = self.selected / PAGE_ITEMS * PAGE_ITEMS;
            }),
            ArrayAction::Null => {
                let next_null = !self.null;
                let value = if next_null {
                    None
                } else {
                    Some(self.editor.read(cx).text(cx))
                };
                self.model.replace(self.selected, value).map(|()| {
                    self.null = next_null;
                })
            }
        };
        match result {
            Ok(()) if matches!(action, ArrayAction::Null) => {
                self.editor
                    .update(cx, |editor, _| editor.set_read_only(self.null));
            }
            Ok(()) => {
                self.show_selected(window, cx);
                self.focus(window, cx);
            }
            Err(error) => self.message = Some(error.to_string()),
        }
        cx.notify();
    }
    fn button(
        &mut self,
        id: String,
        label: String,
        action: ArrayAction,
        enabled: bool,
        selected: Option<bool>,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let focus = self
            .focus
            .entry(id.clone())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let enabled = enabled && self.enabled;
        self.rendered.push(focus.clone());
        if enabled {
            self.order.push(focus.clone());
        }
        let weak = cx.weak_entity();
        div()
            .id(SharedString::from(id))
            .role(if selected.is_some() {
                Role::CheckBox
            } else {
                Role::Button
            })
            .aria_label(label.clone())
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(enabled)
            .px_2()
            .py_1()
            .text_color(if enabled {
                crate::style::text()
            } else {
                crate::style::dim()
            })
            .focus(|style| style.bg(crate::style::hover()))
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
                if let Some(selected) = selected {
                    builder
                        .parent_node()
                        .set_toggled(gpui::accesskit::Toggled::from(selected));
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if enabled {
                    this.action(action, window, cx);
                }
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                if enabled {
                    let _ = weak.update(cx, |this, cx| this.action(action, window, cx));
                }
            })
            .child(label)
            .into_any_element()
    }
}
impl Render for ArrayView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.order.clear();
        self.rendered.clear();
        let count = self.model.items().len();
        let mut rows = div()
            .id("array-elements")
            .max_h(px(120.))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        for index in self.start..(self.start + PAGE_ITEMS).min(count) {
            let item = &self.model.items()[index];
            let label = format!(
                "Element {}: {}",
                index + 1,
                item.as_deref()
                    .map_or("NULL".into(), |text| if text.is_empty() {
                        "empty text".into()
                    } else {
                        format!("text {}", preview(text))
                    })
            );
            rows = rows.child(self.button(
                format!("array-item-{index}"),
                label,
                ArrayAction::Select(index),
                true,
                Some(index == self.selected),
                cx,
            ));
        }
        let mut actions = div().flex().flex_wrap();
        for (id, label, action, enabled) in [
            (
                "array-prev",
                "Previous elements",
                ArrayAction::Previous,
                self.start > 0,
            ),
            (
                "array-next",
                "Next elements",
                ArrayAction::Next,
                self.start + PAGE_ITEMS < count,
            ),
            (
                "array-add",
                "Add element",
                ArrayAction::Add,
                count < cell_value::MAX_ARRAY_ITEMS,
            ),
            (
                "array-remove",
                "Remove selected element",
                ArrayAction::Remove,
                count > 0,
            ),
        ] {
            actions =
                actions.child(self.button(id.into(), label.into(), action, enabled, None, cx));
        }
        actions = actions.child(self.button(
            "array-null".into(),
            "Element is NULL".into(),
            ArrayAction::Null,
            count > 0,
            Some(self.null),
            cx,
        ));
        self.focus
            .retain(|_, handle| self.rendered.contains(handle));
        let preview = self
            .model
            .literal()
            .map(preview)
            .unwrap_or_else(|| self.initial_preview.clone());
        div()
            .id("array-element-editor")
            .role(Role::Group)
            .aria_label("Array elements")
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let focused = this
                    .handles(cx, true)
                    .iter()
                    .any(|focus| focus.contains_focused(window, cx));
                if !focused {
                    return;
                }
                if event.keystroke.key == "escape"
                    || (event.keystroke.key == "enter" && event.keystroke.modifiers.platform)
                {
                    let composing = this.editor.update(cx, |editor, cx| {
                        gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
                    });
                    if !composing {
                        cx.emit(if event.keystroke.key == "escape" {
                            ArrayEvent::Cancel
                        } else {
                            ArrayEvent::Stage
                        });
                        cx.stop_propagation();
                    }
                }
            }))
            .flex()
            .flex_col()
            .child(
                div()
                    .id("array-selected-position")
                    .role(Role::Label)
                    .aria_label(format!(
                        "{count} elements, selected {}",
                        if count == 0 { 0 } else { self.selected + 1 }
                    ))
                    .child(format!(
                        "{count} elements · selected {}",
                        if count == 0 { 0 } else { self.selected + 1 }
                    )),
            )
            .child(rows)
            .child(actions)
            .child(div().h(px(65.)).child(self.accessible.clone()))
            .child(
                div()
                    .id("array-literal-preview")
                    .role(Role::Label)
                    .aria_label(format!("Last validated literal preview: {preview}"))
                    .child(format!("Last validated literal preview: {preview}")),
            )
            .children(self.message.clone().map(|message| {
                div()
                    .id("array-edit-error")
                    .role(Role::Status)
                    .aria_label(message.clone())
                    .child(message)
            }))
    }
}
fn preview(text: &str) -> String {
    let mut chars = text.chars();
    let mut value = chars.by_ref().take(160).collect::<String>();
    if chars.next().is_some() {
        value.push_str("… (preview)");
    }
    value
}
