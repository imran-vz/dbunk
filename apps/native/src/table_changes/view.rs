use super::*;

impl TableChanges {
    fn button(
        &mut self,
        label: &str,
        action: Action,
        enabled: bool,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let label = SharedString::from(label.to_string());
        let key = SharedString::from(match action {
            Action::Include(id, _) => format!("include-{id}"),
            _ => format!("change-{action:?}"),
        });
        let checked = match action {
            Action::RawValue => Some(self.edit.as_ref().is_some_and(|edit| edit.raw)),
            Action::Null => Some(self.edit.as_ref().is_some_and(|edit| edit.null)),
            Action::Include(_, next) => Some(!next),
            _ => None,
        };
        let focus = self
            .buttons
            .entry(key.to_string())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let enabled = enabled && self.enabled;
        let weak = cx.weak_entity();
        self.rendered_buttons.push(focus.clone());
        if enabled {
            self.visible_buttons.push(focus.clone());
        }
        div()
            .id(key)
            .role(if checked.is_some() {
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
                rgb(0xffffff)
            } else {
                rgb(0x888888)
            })
            .focus(|style| style.bg(rgb(0x202020)))
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
                if let Some(checked) = checked {
                    builder
                        .parent_node()
                        .set_toggled(gpui::accesskit::Toggled::from(checked));
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if enabled {
                    this.activate(action, window, cx);
                }
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                if enabled {
                    let _ = weak.update(cx, |this, cx| this.activate(action, window, cx));
                }
            })
            .child(label)
            .into_any_element()
    }
    pub fn focus_handles(&self, cx: &App) -> Vec<FocusHandle> {
        self.edit_handles(cx, false)
            .into_iter()
            .chain(self.visible_buttons.iter().cloned())
            .collect()
    }
}
impl Render for TableChanges {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.visible_buttons.clear();
        self.rendered_buttons.clear();
        let len = self.draft.as_ref().map_or(0, MutationDraft::len);
        let pending = self.pending();
        let unknown = self
            .draft
            .as_ref()
            .is_some_and(MutationDraft::outcome_unknown)
            && self.applying.is_none();
        let mut content = div()
            .id("table-change-review")
            .role(Role::Group)
            .aria_label(if self.is_query() {
                "Query result changes"
            } else {
                "Table changes"
            })
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let Some(editor) = this.edit.as_ref().map(|edit| edit.editor.clone()) else {
                    return;
                };
                if !editor.focus_handle(cx).contains_focused(window, cx) {
                    return;
                }
                let action = if event.keystroke.key == "escape" {
                    Some(Action::CancelEdit)
                } else if event.keystroke.key == "enter" && event.keystroke.modifiers.platform {
                    Some(Action::Stage)
                } else {
                    None
                };
                if let Some(action) = action {
                    let composing = editor.update(cx, |editor, cx| {
                        gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
                    });
                    if !composing {
                        this.activate(action, window, cx);
                        cx.stop_propagation();
                    }
                }
            }))
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(rgb(0x333333));
        if pending {
            content = content.child(self.button(
                "Cancel change operation",
                Action::CancelPending,
                true,
                cx,
            ));
        }
        if self.is_query() {
            content = content.child(div().px_2().text_sm().child(
                "Result edits commit separately from the SQL transaction. Apply never reruns the query."
            ));
        }
        if self.page.is_some() {
            let available = self.key_available();
            let stored = self.key.stored.as_ref().map(|key| key.columns.join(", "));
            let summary = if !self.key.loaded {
                "Virtual key: unavailable".into()
            } else {
                format!("Virtual key: {}", stored.as_deref().unwrap_or("none"))
            };
            let mut key = div()
                .id("virtual-key-controls")
                .role(Role::Group)
                .aria_label("Virtual key")
                .flex()
                .flex_col()
                .px_2()
                .child(
                    div()
                        .id("virtual-key-current")
                        .role(Role::Label)
                        .aria_label(summary.clone())
                        .child(summary),
                );
            if self.key.editing {
                let sources = self
                    .analysis
                    .as_ref()
                    .map(|analysis| {
                        source_key_columns(
                            analysis,
                            self.source.relation().expect("table key controls"),
                        )
                    })
                    .unwrap_or_default();
                let selected = sources
                    .get(self.key.cursor)
                    .map(|value| (*value).to_owned());
                key = key.child("You claim these columns uniquely identify each row. Full-row guards apply; proven keys take precedence.");
                let choices = div()
                    .flex()
                    .flex_wrap()
                    .child(self.button(
                        &format!("Key column: {}", selected.as_deref().unwrap_or("none")),
                        Action::KeyColumn,
                        available && selected.is_some(),
                        cx,
                    ))
                    .child(self.button(
                        "Add key column",
                        Action::KeyAdd,
                        available && selected.is_some(),
                        cx,
                    ));
                key = key.child(choices);
                let mut selected_columns = div()
                    .id("selected-key-columns")
                    .max_h(px(120.))
                    .overflow_y_scroll();
                for (index, column) in self.key.columns.clone().iter().enumerate() {
                    selected_columns = selected_columns.child(self.button(
                        &format!("Remove key column {}: {}", index + 1, column),
                        Action::KeyRemove(index),
                        available,
                        cx,
                    ));
                }
                key = key.child(selected_columns);
                key = key.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .child(self.button(
                            "Save virtual key",
                            Action::KeySave,
                            available
                                && valid_key_columns(&self.key.columns)
                                && self.analysis.is_some(),
                            cx,
                        ))
                        .child(self.button(
                            "Cancel key selection",
                            Action::KeyCancel,
                            !pending,
                            cx,
                        )),
                );
            } else {
                key = key.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .child(self.button(
                            "Choose virtual key",
                            Action::KeyEdit,
                            available && self.key.loaded && self.analysis.is_some(),
                            cx,
                        ))
                        .child(self.button(
                            "Clear virtual key",
                            Action::KeyClear,
                            available && self.key.loaded && self.key.stored.is_some(),
                            cx,
                        ))
                        .child(self.button("Reload virtual key", Action::KeyReload, available, cx)),
                );
            }
            if !self.key.status.is_empty() {
                key = key.child(
                    div()
                        .id("virtual-key-status")
                        .role(Role::Status)
                        .aria_label(self.key.status.clone())
                        .child(self.key.status.clone()),
                );
            }
            content = content.child(key);
        }
        if let Some(edit) = &self.edit {
            if let Some(label) = edit.context.label() {
                content = content.child(
                    div()
                        .id("table-edit-context")
                        .role(Role::Label)
                        .aria_label(label.clone())
                        .child(label),
                );
            }
            if let Some(array) = edit.array.clone() {
                let enabled = self.enabled && !pending && !edit.null;
                let label = format!(
                    "Array value for {}",
                    edit.column.as_deref().unwrap_or("cell")
                );
                content = content.child(
                    div()
                        .id("array-column-name")
                        .role(Role::Label)
                        .aria_label(label.clone())
                        .child(label),
                );
                array.update(cx, |array, cx| array.set_enabled(enabled, cx));
                content = content.child(array);
            } else {
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .p_2()
                        .child(
                            edit.column
                                .clone()
                                .unwrap_or_else(|| "Insert JSON object".into()),
                        )
                        .child(
                            div()
                                .h(px(if edit.kind.is_some() || edit.row.is_none() {
                                    120.
                                } else {
                                    28.
                                }))
                                .flex_1()
                                .child(edit.accessible.clone()),
                        ),
                );
            }
            let null = self.edit.as_ref().unwrap().null;
            let cell = self.edit.as_ref().unwrap().row.is_some();
            let mut buttons = div()
                .flex()
                .child(self.button("Stage change", Action::Stage, !pending, cx))
                .child(self.button("Cancel edit", Action::CancelEdit, !pending, cx));
            if matches!(
                self.edit.as_ref().unwrap().context,
                batch_edit::EditContext::Bulk(_)
            ) {
                let label = format!(
                    "Bulk column: {}",
                    self.edit
                        .as_ref()
                        .unwrap()
                        .column
                        .as_deref()
                        .unwrap_or("none")
                );
                buttons = buttons.child(self.button(&label, Action::BulkColumn, !pending, cx));
            }
            if cell {
                buttons = buttons.child(self.button(
                    if null { "NULL selected" } else { "Use NULL" },
                    Action::Null,
                    !pending,
                    cx,
                ));
            }
            let kind = self.edit.as_ref().unwrap().kind;
            if kind.is_some() {
                buttons = buttons.child(self.button("Raw literal", Action::RawValue, !pending, cx));
            }
            if kind == Some(Kind::Json) {
                buttons = buttons.child(self.button(
                    "Pretty JSON",
                    Action::FormatValue,
                    !pending && !null && !self.edit.as_ref().unwrap().raw,
                    cx,
                ));
            }
            content = content.child(buttons);
            if kind == Some(Kind::Geometry) {
                content = content.child(
                    div()
                        .id("geometry-validation")
                        .role(Role::Label)
                        .aria_label("WKT prefix check only; database validation is still required")
                        .child("WKT prefix check only; database validation is still required"),
                );
                content = content.child(self.button("Copy EWKT", Action::CopyLiteral, true, cx));
                let edit = self.edit.as_ref().unwrap();
                if !edit.null && !edit.raw {
                    let text = edit.editor.read(cx).text(cx);
                    match crate::geometry_preview::parse(&text) {
                        Ok(preview) => {
                            let summary = format!(
                                "{:?} · {} points · bounds {}",
                                preview.shape,
                                preview.points.len(),
                                preview.bounds
                            );
                            content = content
                                .child(
                                    div()
                                        .id("geometry-preview")
                                        .role(Role::Image)
                                        .aria_label(format!("Geometry preview: {summary}"))
                                        .border_1()
                                        .border_color(rgb(0x444444))
                                        .child(crate::geometry_preview::render(&preview)),
                                )
                                .child(div().text_xs().child(summary));
                        }
                        Err(message) => {
                            content = content.child(
                                div()
                                    .id("geometry-preview-refusal")
                                    .role(Role::Label)
                                    .aria_label(message)
                                    .text_xs()
                                    .text_color(rgb(0xfbbf24))
                                    .child(message),
                            );
                        }
                    }
                }
            }
        }
        if self.unrestored.is_some() {
            content = content
                .child(
                    "Saved changes retained; recovery unavailable. Export drafts before discarding them.",
                )
                .child(self.button("Retry saved changes", Action::RetryRecovery, !pending, cx))
                .child(self.button(
                    "Discard changes",
                    Action::Discard,
                    self.applying.is_none(),
                    cx,
                ));
        }
        if len > 0 {
            let changes = self
                .draft
                .as_ref()
                .unwrap()
                .changes()
                .map(|(id, included, operation)| (id, included, change_summary(operation)))
                .collect::<Vec<_>>();
            let selectable = !pending && !unknown && self.edit.is_none();
            let mut rows = Vec::with_capacity(changes.len());
            for (index, (id, included, text)) in changes.into_iter().enumerate() {
                rows.push(
                    div()
                        .flex()
                        .items_center()
                        .px_2()
                        .child(self.button(
                            &format!("Include change {}", index + 1),
                            Action::Include(id, !included),
                            selectable,
                            cx,
                        ))
                        .child(div().flex_1().child(text))
                        .child(self.button(
                            &format!("Remove change {}", index + 1),
                            Action::Remove(id),
                            selectable,
                            cx,
                        )),
                );
            }
            content = content.child(
                div()
                    .id("pending-table-changes")
                    .max_h(px(120.))
                    .overflow_y_scroll()
                    .children(rows),
            );
            let mut buttons = div()
                .flex()
                .flex_wrap()
                .child(self.button("Review changes", Action::Review, !pending && !unknown, cx))
                .child(self.button(
                    "Discard changes",
                    Action::Discard,
                    self.applying.is_none(),
                    cx,
                ));
            if self.review.is_some() {
                buttons = buttons
                    .child(self.button("Apply reviewed changes", Action::Apply, !pending, cx))
                    .child(self.button("Cancel review", Action::CancelReview, true, cx));
            }
            if self
                .applying
                .as_ref()
                .is_some_and(|pending| pending.flow.confirming())
            {
                buttons = buttons
                    .child(self.button("Confirm changes", Action::Confirm, true, cx))
                    .child(self.button("Cancel confirmation", Action::CancelReview, true, cx));
            }
            if unknown {
                content=content.child("Previous apply outcome is unknown. Inspect the database before reconciling; nothing will be retried automatically.");
                buttons = buttons.child(self.button(
                    "I checked the previous outcome",
                    Action::Reconcile,
                    !pending,
                    cx,
                ));
            }
            content = content.child(buttons);
        }
        let preview = self
            .review
            .as_ref()
            .map(|(_, review)| review.preview())
            .or_else(|| {
                self.applying
                    .as_ref()
                    .and_then(|pending| match pending.flow.token() {
                        Some(Token::Review(review)) => Some(review.preview()),
                        Some(Token::Confirmation(confirmation)) => Some(confirmation.preview()),
                        None => None,
                    })
            });
        if let Some(preview) = preview {
            content =
                content.child(
                    div()
                        .id("reviewed-table-sql")
                        .role(Role::Group)
                        .aria_label("Exact SQL and bound values")
                        .max_h(px(160.))
                        .overflow_y_scroll()
                        .children(preview.statements.iter().enumerate().map(
                            |(index, statement)| {
                                let text = format!(
                                    "{}\n{}",
                                    statement.sql,
                                    serde_json::to_string(&statement.params).unwrap_or_default()
                                );
                                div()
                                    .id(("review-statement", index))
                                    .role(Role::Label)
                                    .aria_label(text.clone())
                                    .p_2()
                                    .child(text)
                            },
                        )),
                );
        }
        if self.discard {
            content = content
                .child("Discard all staged changes?")
                .child(self.button(
                    "Confirm discard changes",
                    Action::ConfirmDiscard,
                    !pending,
                    cx,
                ))
                .child(self.button("Keep changes", Action::CancelDiscard, true, cx));
        }
        // Removed rows must not leave focus handles accumulating for this tab.
        self.buttons
            .retain(|_, handle| self.rendered_buttons.contains(handle));
        content.child(
            div()
                .id("table-change-status")
                .role(Role::Status)
                .aria_label(self.message.clone())
                .a11y_synthetic_children(|builder| {
                    builder
                        .parent_node()
                        .set_live(gpui::accesskit::Live::Polite)
                })
                .px_2()
                .child(self.message.clone()),
        )
    }
}

fn change_summary(operation: &MutationOp) -> String {
    let (verb, table, values) = match operation {
        MutationOp::Insert { table, values } => ("Insert", table, values.as_slice()),
        MutationOp::Update { table, set, .. } => ("Update", table, set.as_slice()),
        MutationOp::Delete { table, .. } => ("Delete", table, &[][..]),
    };
    // Values are shown in the exact SQL review. Keep this bounded list cheap
    // to paint even when a staged value is several megabytes long.
    let mut text = format!("{verb} {}.{}", table.schema, table.table);
    for value in values {
        if text.len() >= 512 {
            break;
        }
        text.push(' ');
        text.extend(value.column.chars().take(128));
    }
    text.chars().take(512).collect()
}
