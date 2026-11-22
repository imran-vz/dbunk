use super::*;
mod details;
mod lists;
impl TableSeedView {
    fn button(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let (action, default) = ACTIONS[index];
        let store = self.store.read(cx);
        let text = match action {
            Action::Connection => format!("Destination: {}", self.connection_label()),
            Action::Mode => format!("Source: {}", recipe::mode_label(self.mode)),
            Action::Realism => match self.realism {
                TableSeedRealism::FictionalSafe => "Contact details: fictional-safe".into(),
                TableSeedRealism::Realistic => "Contact details: realistic".into(),
            },
            Action::Apply
                if self
                    .selected
                    .is_some_and(|id| store.confirmation_pending(id)) =>
            {
                "Confirm exact seed recipe".into()
            }
            Action::Reconcile if self.armed.is_some() && self.armed == self.selected => {
                "I inspected the destination; reconcile".into()
            }
            _ => default.into(),
        };
        let enabled = self.enabled(action, cx);
        let weak = cx.weak_entity();
        let button = if matches!(action, Action::Apply) {
            crate::ui::tool_button_accent(("seed-action", index), text, None, enabled)
        } else {
            crate::ui::tool_button(("seed-action", index), text, None, enabled, false)
        };
        button
            .track_focus(&self.buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .into_any_element()
    }
}
impl TableSeedView {
    /// A stored connection's display name, never its id (D14).
    fn connection_name(&self, id: &str) -> String {
        self.connections
            .as_ref()
            .and_then(|choices| choices.rows.iter().find(|(key, _)| key.as_str() == id))
            .map_or_else(|| "Unavailable connection".into(), |(_, name)| name.clone())
    }
}
impl Render for TableSeedView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_fields(window, cx);
        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(crate::style::S4))
            .pb(px(crate::style::INSET_CONTENT))
            .child(
                crate::ui::toolbar()
                    .children(
                        (0..ACTIONS.len())
                            .filter(|index| {
                                !matches!(ACTIONS[*index].0, Action::Mode | Action::SaveColumn)
                            })
                            .map(|index| self.button(index, cx)),
                    )
                    .child(crate::ui::grow())
                    .child(crate::ui::info_hint(
                        "seed-boundaries",
                        "Generate rows on the backend in one destination transaction. DEFAULT leaves database defaults authoritative. Exact review freezes the seed and clock. Cancellation after COMMIT cannot undo rows. Jobs and uncertain intent survive tab closure.",
                    )),
            )
            .child(label("seed-status", self.status.clone()).text_color(crate::style::dim()));
        if let Some(message) = self.store.read(cx).message() {
            content = content.child(label("seed-store-message", message.to_owned()));
        }
        content = content.child(
            field_column().children(
                RUN_FIELDS
                    .iter()
                    .filter_map(|index| self.fields.get(*index).map(|field| (*index, field)))
                    .map(|(index, field)| field_row(FIELDS[index].0, field.clone())),
            ),
        );
        if self.choosing {
            content = content.child(
                div()
                    .id("seed-connections")
                    .mx(px(crate::style::INSET_CONTENT))
                    .py(px(crate::style::S2))
                    .rounded(px(crate::style::R_MD))
                    .border_1()
                    .border_color(crate::style::line_soft())
                    .role(Role::ListBox)
                    .aria_label("Stored PostgreSQL, MySQL and SQLite destinations")
                    .track_focus(&self.choice_focus)
                    .tab_stop(true)
                    .child(self.choices(cx)),
            );
        }
        if !self.ids(cx).is_empty() {
            content = content.child(
                div()
                    .id("seed-jobs")
                    .mx(px(crate::style::INSET_CONTENT))
                    .py(px(crate::style::S2))
                    .rounded(px(crate::style::R_MD))
                    .border_1()
                    .border_color(crate::style::line_soft())
                    .role(Role::ListBox)
                    .aria_label("Seed attempts and durable recovery records")
                    .track_focus(&self.job_focus)
                    .tab_stop(true)
                    .child(self.jobs_list(cx)),
            );
        }
        if let Some(recipe) = &self.recipe {
            content = content
                .child(label(
                    "seed-recipe-target",
                    format!(
                        "Editable recipe: {} / {}.{} · {}{}",
                        self.connection_name(&recipe.setup.endpoint.connection_id),
                        recipe.setup.endpoint.schema,
                        recipe.setup.endpoint.table,
                        plural(recipe.columns.len(), "column", "columns"),
                        if recipe.changed {
                            " · local edits require Prepare"
                        } else {
                            ""
                        }
                    ),
                ))
                .child(
                    div()
                        .id("seed-columns")
                        .mx(px(crate::style::INSET_CONTENT))
                        .py(px(crate::style::S2))
                        .rounded(px(crate::style::R_MD))
                        .border_1()
                        .border_color(crate::style::line_soft())
                        .role(Role::ListBox)
                        .aria_label("Editable seed columns")
                        .track_focus(&self.column_focus)
                        .tab_stop(true)
                        .child(self.columns(cx)),
                );
            if let Some(column) = self.column.and_then(|index| recipe.columns.get(index)) {
                content = content
                    .child(label(
                        "seed-column-metadata",
                        format!(
                            "{} ({}) · {}{}{}{}{}",
                            column.name,
                            column.data_type,
                            if column.nullable { "null" } else { "not null" },
                            if column.has_default {
                                " · has default"
                            } else {
                                ""
                            },
                            if column.generated {
                                " · generated"
                            } else {
                                ""
                            },
                            if column.identity { " · identity" } else { "" },
                            if self.column_dirty {
                                " · unsaved changes"
                            } else {
                                ""
                            }
                        ),
                    ))
                    .child(
                        crate::ui::toolbar()
                            .child(self.button(4, cx))
                            .child(self.button(5, cx)),
                    );
                let column_fields: Vec<_> = self
                    .fields
                    .iter()
                    .enumerate()
                    .filter(|(index, field)| column_field(*index) && self.field_visible(field, cx))
                    .map(|(index, field)| field_row(FIELDS[index].0, field.clone()))
                    .collect();
                content = content.child(field_column().children(column_fields));
                if self.choosing_mode {
                    content = content.child(
                        div()
                            .id("seed-modes")
                            .mx(px(crate::style::INSET_CONTENT))
                            .py(px(crate::style::S2))
                            .rounded(px(crate::style::R_MD))
                            .border_1()
                            .border_color(crate::style::line_soft())
                            .role(Role::ListBox)
                            .aria_label("Column source and generator choices")
                            .track_focus(&self.mode_focus)
                            .tab_stop(true)
                            .child(self.modes(cx)),
                    );
                }
            }
        }
        if let Some(preview) = &self.preview {
            content = content.child(label("seed-preview", preview.clone()));
        }
        let review = self
            .store
            .read(cx)
            .review_payload()
            .filter(|review| Some(review.attempt_id()) == self.selected);
        if let Some(review) = review {
            let setup_note = if self.setup_changed(review.attempt_id(), cx) {
                "Setup changed. Apply is disabled for this attempt; prepare and review the changes."
            } else if self
                .recipe
                .as_ref()
                .is_some_and(|recipe| recipe.setup.attempt == review.attempt_id())
                || self
                    .setup_anchor
                    .as_ref()
                    .is_some_and(|anchor| anchor.attempt == review.attempt_id())
            {
                "Apply uses this exact accepted recipe."
            } else {
                "This selected app-owned job is independent of the editable setup. Apply uses only the exact accepted recipe below."
            };
            content = content
                .child(label(
                    "seed-exact-review",
                    format!(
                        "Exact accepted review\n{}\n{}",
                        details::description(review.description()),
                        setup_note
                    ),
                ))
                .child(
                    div()
                        .id("seed-reviewed-columns")
                        .mx(px(crate::style::INSET_CONTENT))
                        .py(px(crate::style::S2))
                        .rounded(px(crate::style::R_MD))
                        .border_1()
                        .border_color(crate::style::line_soft())
                        .role(Role::ListBox)
                        .aria_label("Exact resolved seed column review")
                        .track_focus(&self.review_focus)
                        .tab_stop(true)
                        .child(self.review_columns(cx)),
                );
            if let Some(text) = self.review_column_text(cx) {
                content = content.child(label("seed-reviewed-column-detail", text));
            }
        }
        if let Some(text) = self.details(cx) {
            content = content.child(label("seed-attempt-details", text));
        }
        div().id("seed-view").key_context("TableSeed").role(Role::Group).aria_label("Table seed").track_focus(&self.root).size_full().bg(crate::style::bg()).text_color(crate::style::text()).text_size(px(crate::style::FONT))
            .on_action(cx.listener(|this, _: &NextControl, window, cx| this.focus_control(false, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousControl, window, cx| this.focus_control(true, window, cx)))
            .capture_key_down(cx.listener(Self::key)).overflow_y_scroll().track_scroll(&self.scroll)
            .child(div().id("seed-review-and-receipt").role(Role::Group).aria_label("Seed setup, exact review and receipt; Page Up and Page Down scroll, Home and End from this group reach either end").track_focus(&self.details_focus).tab_stop(true).child(content))
    }
}
fn label(id: &'static str, text: String) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Label)
        .aria_label(text.clone())
        .px(px(crate::style::INSET_CONTENT))
        .whitespace_normal()
        .child(text)
}
/// A tool-form column: at most `TOOL_COLUMN` wide, content inset.
fn field_column() -> gpui::Div {
    div()
        .w_full()
        .max_w(px(
            crate::style::TOOL_COLUMN + 2. * crate::style::INSET_CONTENT
        ))
        .px(px(crate::style::INSET_CONTENT))
        .flex()
        .flex_col()
        .gap(px(crate::style::S5))
}
/// A labelled bounded field in a visible well.
fn field_row(name: &'static str, field: Entity<Field>) -> gpui::Div {
    crate::ui::labelled(name, crate::object_ddl_view::well(field), None)
}
/// `1 column`, `2 columns`.
pub(super) fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}
