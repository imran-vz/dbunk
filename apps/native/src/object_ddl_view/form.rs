//! Create index and Add enum value options. Controls only edit the local
//! draft; observation, review and apply stay on the shared lifecycle.
use super::*;
use crate::{
    object_ddl_model::{derived_index_name, parse_index_columns},
    style, ui,
};
use gpui::{AnyElement, SharedString};

/// Column list field bound: every column at its 1 KiB expression limit plus
/// separators and direction words.
pub(super) const COLUMNS_FIELD_BYTES: usize = MAX_OBJECT_DDL_INDEX_COLUMNS * 1100;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Control {
    Method,
    Unique,
    Concurrently,
    Placement,
}
pub(super) const CONTROLS: [Control; 4] = [
    Control::Method,
    Control::Unique,
    Control::Concurrently,
    Control::Placement,
];

impl ObjectDdlView {
    /// Form purposes replace the shared Mode/Option toggles with their own
    /// controls, so those two buttons are hidden rather than mislabelled.
    pub(super) fn shows_action(&self, action: Action) -> bool {
        !(self.uses_form() && matches!(action, Action::Mode | Action::Option))
    }
    fn control_shown(&self, control: Control) -> bool {
        match (&self.purpose, control) {
            (
                Some(Purpose::CreateIndex { .. }),
                Control::Method | Control::Unique | Control::Concurrently,
            ) => true,
            (Some(Purpose::AddEnumValue { .. }), Control::Placement) => true,
            _ => false,
        }
    }
    fn control_enabled(&self, control: Control) -> bool {
        self.editable && self.control_shown(control) && self.editable_recipe()
    }
    /// Enabled form controls in tab order, after the action row.
    pub(super) fn form_focus(&self) -> Vec<FocusHandle> {
        CONTROLS
            .iter()
            .enumerate()
            .filter(|(_, control)| self.control_enabled(**control))
            .map(|(index, _)| self.form_buttons[index].clone())
            .collect()
    }
    fn control_click(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let control = CONTROLS[index];
        if !self.control_enabled(control) {
            return;
        }
        if self.composing(window, cx) {
            self.message = "Finish composition before changing this draft".into();
            cx.notify();
            return;
        }
        window.focus(&self.form_buttons[index], cx);
        match control {
            Control::Method => self.draft.method = self.draft.method.next(),
            Control::Unique => self.draft.unique = !self.draft.unique,
            Control::Concurrently => self.draft.concurrently = !self.draft.concurrently,
            Control::Placement => self.draft.placement = self.draft.placement.next(),
        }
        self.armed = false;
        self.publish(cx);
    }
    fn draft_text(&self, cx: &Context<Self>) -> (String, String) {
        let read = |field: Option<&Entity<Field>>| {
            field
                .and_then(|field| field.read(cx).value(cx).ok())
                .unwrap_or_default()
        };
        (read(self.name.as_ref()), read(self.body.as_ref()))
    }
    /// The inline note under the second field: the first draft problem, or
    /// a ready hint. Untouched drafts show the problem as guidance, not error.
    fn draft_note(&self, cx: &Context<Self>) -> (SharedString, bool) {
        let Some(purpose) = &self.purpose else {
            return ("Select an Objects row first".into(), true);
        };
        let (name, body) = self.draft_text(cx);
        let pristine = name.is_empty() && body.is_empty();
        match self.draft.operations(purpose, name, body) {
            Err(issue) => (issue.into(), !pristine),
            Ok(_) => ("Ready: observe and review shows the exact SQL".into(), false),
        }
    }
    /// What an empty index name resolves to, when the columns parse.
    fn name_note(&self, cx: &Context<Self>) -> Option<(SharedString, bool)> {
        let Some(Purpose::CreateIndex { table, .. }) = &self.purpose else {
            return None;
        };
        let (name, body) = self.draft_text(cx);
        if !name.trim().is_empty() {
            return None;
        }
        let columns = parse_index_columns(&body).ok()?;
        Some((
            format!("Will be named {:?}", derived_index_name(table, &columns)).into(),
            false,
        ))
    }
    fn control(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let control = CONTROLS[index];
        let enabled = self.control_enabled(control);
        let toggled = match control {
            Control::Unique => Some(self.draft.unique),
            Control::Concurrently => Some(self.draft.concurrently),
            Control::Method | Control::Placement => None,
        };
        let label: SharedString = match control {
            Control::Method => format!("Method: {}", self.draft.method.as_str()).into(),
            Control::Unique => "UNIQUE (btree only)".into(),
            Control::Concurrently => {
                "CONCURRENTLY: no write lock, runs outside a transaction".into()
            }
            Control::Placement => format!("Position: {}", self.draft.placement.label()).into(),
        };
        let weak = cx.weak_entity();
        let element = match toggled {
            None => ui::button(
                ("object-ddl-form", index),
                label,
                ui::Variant::Secondary,
                enabled,
            ),
            Some(on) => ui::press(
                div()
                    .id(("object-ddl-form", index))
                    .role(Role::CheckBox)
                    .aria_label(label.clone())
                    .aria_toggled(on.into())
                    .flex_none()
                    .h(px(24.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .text_sm()
                    .whitespace_nowrap()
                    .text_color(if enabled {
                        style::text()
                    } else {
                        style::faint()
                    })
                    .when(enabled, |b| {
                        b.cursor_pointer().hover(|s| s.bg(style::hover()))
                    })
                    .focus(|s| s.border_color(style::accent()))
                    .a11y_synthetic_children(move |b| {
                        if !enabled {
                            b.parent_node().set_disabled();
                        }
                    })
                    .child(ui::check_box(on))
                    .child(label),
            ),
        };
        element
            .track_focus(&self.form_buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            // GPUI activates a focused clickable on Enter/Space key-up through
            // on_click; no key-down handler, so activation happens once.
            .on_click(cx.listener(move |this, _, window, cx| {
                this.control_click(index, window, cx)
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.control_click(index, window, cx))
                    .ok();
            })
            .into_any_element()
    }
    /// The index or enum form, or `None` for drop and view purposes.
    pub(super) fn render_form(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let title = match self.purpose.as_ref()? {
            Purpose::CreateIndex { .. } => "Index",
            Purpose::AddEnumValue { .. } => "New enum value",
            Purpose::Drop(_) | Purpose::CreateView { .. } => return None,
        };
        let (name_label, body_label) = if matches!(self.purpose, Some(Purpose::CreateIndex { .. }))
        {
            (
                "Name (optional)",
                "Columns or expressions, in order; quote mixed-case names",
            )
        } else {
            ("Label", "Neighbor label (BEFORE or AFTER only)")
        };
        let editable = self.editable_recipe();
        let note = editable.then(|| self.draft_note(cx));
        let name_note = if editable { self.name_note(cx) } else { None };
        Some(
            div()
                .id("object-ddl-form")
                .role(Role::Group)
                .aria_label(title)
                .px_2()
                .pt_1()
                .child(
                    ui::section(title)
                        .child(
                            div().flex().flex_wrap().gap(px(4.)).children(
                                (0..CONTROLS.len())
                                    .filter(|i| self.control_shown(CONTROLS[*i]))
                                    .map(|i| self.control(i, cx)),
                            ),
                        )
                        .when_some(self.name.as_ref(), |v, field| {
                            v.child(ui::labelled(name_label, field.clone(), name_note))
                        })
                        .when_some(self.body.as_ref(), |v, field| {
                            v.child(ui::labelled(body_label, field.clone(), note))
                        }),
                )
                .into_any_element(),
        )
    }
    /// Draft summary for the details pane before any review exists.
    pub(super) fn form_draft_summary(&self) -> Option<String> {
        match self.purpose.as_ref()? {
            Purpose::CreateIndex { schema, table } => Some(format!(
                "Draft: {}index on {schema:?}.{table:?} using {}{}\nObserve and review confirms the table exists and the index name is free before any SQL is shown.{}",
                if self.draft.unique { "UNIQUE " } else { "" },
                self.draft.method.as_str(),
                if self.draft.concurrently {
                    ", CONCURRENTLY"
                } else {
                    ""
                },
                if self.draft.concurrently {
                    "\nCONCURRENTLY runs outside a transaction; a failed build can leave an INVALID index."
                } else {
                    ""
                },
            )),
            Purpose::AddEnumValue { schema, name } => Some(format!(
                "Draft: add a label to enum {schema:?}.{name:?} {}\nObserve and review confirms the exact enum type before any SQL is shown. ADD VALUE runs outside a transaction and cannot be rolled back.",
                self.draft.placement.label()
            )),
            Purpose::Drop(_) | Purpose::CreateView { .. } => None,
        }
    }
}
