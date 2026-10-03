//! Exact metadata/definition text in a read-only editor. Replacing the editor on
//! view changes drops its old buffer instead of accumulating invisible undo text.
use crate::{accessible_editor::AccessibleEditor, results::encoded_size};
use dbunk_lib::backend::objects::{PgObjectDescription, PgObjectFacts, PgObjectKind, PgTypeClass};
use editor::Editor;
use gpui::{
    ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role,
    Window, div, prelude::*,
};
use std::{cell::Cell, rc::Rc};
const LIMIT: usize = 128 * 1024 * 1024;
pub struct Details {
    json: String,
    definition: Option<String>,
    notice: String,
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Details {
    pub fn drop_impact(
        reference: dbunk_lib::backend::objects::PgObjectRef,
        impact: dbunk_lib::backend::objects::PgDropImpact,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        let notice = if impact.truncated {
            "Incomplete downstream drop impact. Additional dependents may exist, including when this list is empty. No DDL is executed."
        } else {
            "Downstream drop impact from a read-only snapshot. This is not an upstream reference list or authorization to drop the object."
        }.to_owned();
        let json = serde_json::to_string_pretty(
            &serde_json::json!({"reference": reference, "dropImpact": impact}),
        )
        .map_err(|_| "Drop impact cannot be encoded")?;
        if json.len() > 4 * 1024 * 1024 {
            return Err("Formatted drop impact exceeds 4 MiB");
        }
        let bytes = encoded_size(&(&json, &notice)).saturating_add(encoded_size(&json));
        if bytes > LIMIT.saturating_sub(budget.get()) {
            return Err("Workspace memory budget is full; close another tool or clear results");
        }
        budget.set(budget.get() + bytes);
        Ok(Self {
            json,
            definition: None,
            notice,
            budget,
            bytes,
        })
    }

    pub fn new(
        description: PgObjectDescription,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        let json = serde_json::to_string_pretty(&description)
            .map_err(|_| "Object metadata cannot be encoded")?;
        if json.len() > 16 * 1024 * 1024 {
            return Err("Formatted object metadata exceeds 16 MiB");
        }
        let notice = match description.reference.kind {
            PgObjectKind::Table => {
                "Reconstructed SQL omits INHERITS, storage options, policies, triggers and grants."
            }
            PgObjectKind::ForeignTable => {
                "Reconstructed SQL omits partition membership, policies, triggers and grants."
            }
            PgObjectKind::Type => match description.facts {
                PgObjectFacts::Type {
                    class: PgTypeClass::Composite,
                    ..
                } => "Reconstructed SQL omits composite attribute collations.",
                PgObjectFacts::Type {
                    class: PgTypeClass::Multirange,
                    ..
                } => "Multirange SQL creates its parent range with the multirange name.",
                _ => "",
            },
            PgObjectKind::Domain => {
                "Reconstructed SQL retains CHECK expressions without their constraint names."
            }
            _ => "",
        }
        .to_owned();
        let definition = description.definition_sql;
        // JSON and definition plus the largest active editor text representation.
        let bytes = encoded_size(&(&json, &definition, &notice))
            .saturating_add(encoded_size(&json).max(encoded_size(&definition)));
        if bytes > LIMIT.saturating_sub(budget.get()) {
            return Err("Workspace memory budget is full; close another tool or clear results");
        }
        budget.set(budget.get() + bytes);
        Ok(Self {
            json,
            definition,
            notice,
            budget,
            bytes,
        })
    }
}
impl Drop for Details {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}
pub struct Close;
#[derive(Clone, Copy)]
enum Action {
    Json,
    Definition,
    Copy,
    Back,
}
pub struct DetailsView {
    data: Details,
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    root: FocusHandle,
    buttons: Vec<FocusHandle>,
    definition: bool,
    status: String,
}
impl EventEmitter<Close> for DetailsView {}
impl DetailsView {
    pub fn new(data: Details, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (editor, accessible) = editor(&data.json, window, cx);
        let status = String::new();
        Self {
            data,
            editor,
            accessible,
            root: cx.focus_handle(),
            buttons: (0..4).map(|_| cx.focus_handle()).collect(),
            definition: false,
            status,
        }
    }
    pub fn contains_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.root.contains_focused(window, cx)
    }
    pub fn focus(&self, cx: &gpui::App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Action::Back => cx.emit(Close),
            Action::Copy => {
                let text = if self.definition {
                    self.data.definition.as_deref().unwrap_or("")
                } else {
                    &self.data.json
                };
                cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
                self.status = "Copied exact displayed text".into();
            }
            Action::Json | Action::Definition => {
                let definition = matches!(action, Action::Definition);
                if definition && self.data.definition.is_none() {
                    self.status = "This object has no definition SQL".into();
                    cx.notify();
                    return;
                }
                let text = if definition {
                    self.data.definition.as_deref().unwrap()
                } else {
                    &self.data.json
                };
                let (editor, accessible) = editor(text, window, cx);
                self.editor = editor;
                self.accessible = accessible;
                self.definition = definition;
                self.status.clear();
                window.focus(&self.editor.focus_handle(cx), cx);
            }
        }
        cx.notify();
    }
    fn button(
        &self,
        index: usize,
        label: &'static str,
        action: Action,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let weak = cx.entity().downgrade();
        let enabled = !(matches!(action, Action::Definition) && self.data.definition.is_none());
        div()
            .id(("object-details-action", index))
            .role(Role::Button)
            .aria_label(label)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .track_focus(&self.buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .focus(|style| style.bg(crate::style::hover()))
            .text_color(if enabled {
                crate::style::text()
            } else {
                crate::style::dim()
            })
            .px_2()
            .py_1()
            .border_1()
            .border_color(crate::style::line())
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .into_any_element()
    }
}
fn editor(
    text: &str,
    window: &mut Window,
    cx: &mut Context<DetailsView>,
) -> (Entity<Editor>, Entity<AccessibleEditor>) {
    let editor = cx.new(|cx| {
        let mut editor = Editor::for_buffer(
            cx.new(|cx| language::Buffer::local(text.to_owned(), cx)),
            None,
            window,
            cx,
        );
        editor.set_read_only(true);
        editor
    });
    let accessible = cx.new(|cx| {
        AccessibleEditor::new(
            editor.clone(),
            "Read-only object metadata or definition",
            cx,
        )
    });
    (editor, accessible)
}
impl Render for DetailsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("object-details")
            .role(Role::Group)
            .aria_label("Object details, read only")
            .track_focus(&self.root)
            .flex()
            .flex_col()
            .size_full()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let m = &event.keystroke.modifiers;
                if m.control || m.alt || m.platform {
                    return;
                }
                if event.keystroke.key == "escape" {
                    cx.emit(Close);
                    cx.stop_propagation();
                    return;
                }
                if event.keystroke.key != "tab" {
                    return;
                }
                let handles = this
                    .buttons
                    .iter()
                    .cloned()
                    .chain(std::iter::once(this.editor.focus_handle(cx)))
                    .collect::<Vec<_>>();
                let current = handles.iter().position(|h| h.is_focused(window));
                let next = if m.shift {
                    current.map_or(handles.len() - 1, |i| {
                        (i + handles.len() - 1) % handles.len()
                    })
                } else {
                    current.map_or(0, |i| (i + 1) % handles.len())
                };
                window.focus(&handles[next], cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .flex()
                    .gap_1()
                    .flex_shrink_0()
                    .child(self.button(0, "Catalog", Action::Back, cx))
                    .child(self.button(1, "Metadata JSON", Action::Json, cx))
                    .child(self.button(2, "Definition SQL", Action::Definition, cx))
                    .child(self.button(3, "Copy displayed text", Action::Copy, cx)),
            )
            .when(!self.data.notice.is_empty(), |root| {
                root.child(
                    div()
                        .id("object-definition-scope")
                        .role(Role::Label)
                        .aria_label(self.data.notice.clone())
                        .child(self.data.notice.clone()),
                )
            })
            .child(
                div()
                    .id("object-details-status")
                    .role(Role::Label)
                    .aria_label(self.status.clone())
                    .child(self.status.clone()),
            )
            .child(div().flex_1().min_h_0().child(self.accessible.clone()))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::objects::{PgObjectFacts, PgObjectKind, PgObjectRef};
    #[test]
    fn truncated_empty_drop_impact_keeps_uncertainty_and_exact_identity() {
        use dbunk_lib::backend::objects::PgDropImpact;
        let budget = Rc::new(Cell::new(11));
        let details = Details::drop_impact(
            PgObjectRef {
                kind: PgObjectKind::Function,
                schema: Some("exact.schema".into()),
                name: "f".into(),
                identity_args: Some("value integer".into()),
            },
            PgDropImpact {
                dependents: vec![],
                truncated: true,
            },
            budget.clone(),
        )
        .unwrap();
        assert!(details.notice.contains("including when this list is empty"));
        assert!(details.json.contains("value integer"));
        assert!(details.definition.is_none());
        drop(details);
        assert_eq!(budget.get(), 11);
    }
    #[test]
    fn metadata_retention_is_atomic_and_released() {
        let budget = Rc::new(Cell::new(LIMIT));
        let description = || PgObjectDescription {
            reference: PgObjectRef {
                kind: PgObjectKind::Schema,
                schema: None,
                name: "quoted\"雪".into(),
                identity_args: None,
            },
            owner: None,
            comment: Some(String::new()),
            definition_sql: Some("SELECT 'exact\\n';".into()),
            facts: PgObjectFacts::Schema,
        };
        assert!(Details::new(description(), budget.clone()).is_err());
        assert_eq!(budget.get(), LIMIT);
        budget.set(19);
        let data = Details::new(description(), budget.clone()).unwrap();
        assert_eq!(data.definition.as_deref(), Some("SELECT 'exact\\n';"));
        assert!(data.json.contains("quoted\\\"雪"));
        assert!(budget.get() > 19);
        drop(data);
        assert_eq!(budget.get(), 19);
    }
}
