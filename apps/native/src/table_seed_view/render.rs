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
        div()
            .id(("seed-action", index))
            .role(Role::Button)
            .aria_label(text.clone())
            .track_focus(&self.buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .px_2()
            .py_1()
            .border_1()
            .border_color(crate::style::line())
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
            })
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .child(text)
            .into_any_element()
    }
}
impl Render for TableSeedView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_fields(window, cx);
        let mut content=div().flex().flex_col().gap_2()
            .child(div().flex().flex_wrap().gap_1().children((0..ACTIONS.len()).filter(|index|!matches!(ACTIONS[*index].0,Action::Mode|Action::SaveColumn)).map(|index|self.button(index,cx))))
            .child(label("seed-status",self.status.clone()))
            .child(label("seed-boundaries","Generate rows on the backend in one destination transaction. DEFAULT leaves database defaults authoritative. Exact review freezes the seed and clock. Cancellation after COMMIT cannot undo rows. Jobs and uncertain intent survive tab closure.".into()));
        if let Some(message) = self.store.read(cx).message() {
            content = content.child(label("seed-store-message", message.to_owned()));
        }
        for (index, field) in self.fields.iter().take(4).enumerate() {
            content = content.child(
                div()
                    .flex()
                    .gap_2()
                    .child(FIELDS[index].0)
                    .child(div().flex_1().child(field.clone())),
            );
        }
        if self.choosing {
            content = content.child(
                div()
                    .id("seed-connections")
                    .role(Role::ListBox)
                    .aria_label("Stored PostgreSQL destinations")
                    .track_focus(&self.choice_focus)
                    .tab_stop(true)
                    .child(self.choices(cx)),
            );
        }
        if !self.ids(cx).is_empty() {
            content = content.child(
                div()
                    .id("seed-jobs")
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
                        "Editable recipe: {} / {}.{} · {} columns{}",
                        recipe.endpoint.connection_id,
                        recipe.endpoint.schema,
                        recipe.endpoint.table,
                        recipe.columns.len(),
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
                            "{} ({}) · nullable: {} · default: {} · generated: {} · identity: {}{}",
                            column.name,
                            column.data_type,
                            column.nullable,
                            column.has_default,
                            column.generated,
                            column.identity,
                            if self.column_dirty {
                                " · unsaved changes"
                            } else {
                                ""
                            }
                        ),
                    ))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(self.button(4, cx))
                            .child(self.button(5, cx)),
                    );
                for (index, field) in self
                    .fields
                    .iter()
                    .enumerate()
                    .skip(4)
                    .filter(|(_, field)| self.field_visible(field, cx))
                {
                    content = content.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(FIELDS[index].0)
                            .child(div().flex_1().child(field.clone())),
                    );
                }
                if self.choosing_mode {
                    content = content.child(
                        div()
                            .id("seed-modes")
                            .role(Role::ListBox)
                            .aria_label("Column source and generator choices")
                            .track_focus(&self.mode_focus)
                            .tab_stop(true)
                            .child(self.modes(cx)),
                    );
                }
            }
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
                .is_some_and(|recipe| recipe.attempt == review.attempt_id())
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
        div().id("seed-view").key_context("TableSeed").role(Role::Group).aria_label("PostgreSQL table seed").track_focus(&self.root).size_full().bg(crate::style::bg()).text_color(crate::style::text())
            .on_action(cx.listener(|this, _: &NextControl, window, cx| this.focus_control(false, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousControl, window, cx| this.focus_control(true, window, cx)))
            .capture_key_down(cx.listener(Self::key)).overflow_y_scroll().track_scroll(&self.scroll)
            .child(div().id("seed-review-and-receipt").role(Role::Group).aria_label("Seed setup, exact review and receipt; Page Up and Page Down scroll, Home and End from this group reach either end").track_focus(&self.details_focus).tab_stop(true).child(content))
    }
}
fn label(id: &'static str, text: String) -> AnyElement {
    div()
        .id(id)
        .role(Role::Label)
        .aria_label(text.clone())
        .child(text)
        .into_any_element()
}
