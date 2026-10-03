use super::*;
impl TableCopyView {
    fn button(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let (action, default) = ACTIONS[index];
        let store = self.store.read(cx);
        let label = match action {
            Action::Source => format!("Source: {}", self.connection_label(self.source.as_deref())),
            Action::Destination => format!(
                "Destination: {}",
                self.connection_label(self.destination.as_deref())
            ),
            Action::Apply
                if self
                    .selected
                    .is_some_and(|id| store.confirmation_pending(id)) =>
            {
                "Confirm exact table copy".into()
            }
            Action::Reconcile if self.armed.is_some() && self.armed == self.selected => {
                "I inspected the destination; reconcile this attempt".into()
            }
            _ => default.into(),
        };
        let enabled = self.enabled(action, cx);
        let weak = cx.weak_entity();
        div()
            .id(("table-copy-action", index))
            .role(Role::Button)
            .aria_label(label.clone())
            .track_focus(&self.buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .px_2()
            .py_1()
            .border_1()
            .border_color(rgb(0x444444))
            .text_color(if enabled {
                rgb(0xffffff)
            } else {
                rgb(0x888888)
            })
            .focus(|style| style.bg(rgb(0x222222)))
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
            .child(label)
            .into_any_element()
    }
    fn job_label(&self, id: TableCopyAttemptId, cx: &gpui::App) -> String {
        let store = self.store.read(cx);
        let record = store.journal().iter().find(|job| job.attempt_id == id);
        let observation = store.jobs().iter().find(|job| job.attempt_id == id);
        let intent = record
            .map(|job| &job.description.intent)
            .or_else(|| observation.map(|job| &job.intent));
        let state = observation.map_or_else(
            || {
                record.map_or_else(
                    || "Unavailable".into(),
                    |job| format!("Recovered: {:?}", job.state),
                )
            },
            |job| format!("{:?}: {:?}", job.phase, job.outcome),
        );
        intent.map_or_else(
            || format!("{id}: unavailable"),
            |intent| {
                format!(
                    "{state} | {}.{} → {}.{}",
                    intent.source.schema,
                    intent.source.table,
                    intent.destination.schema,
                    intent.destination.table
                )
            },
        )
    }
    fn details(&self, cx: &gpui::App) -> Option<String> {
        let id = self.selected?;
        let record = self.journal(cx);
        let observation = self.observation(cx);
        if record.is_none() && observation.is_none() {
            return Some(format!(
                "Attempt {id} is no longer available. No other attempt was selected."
            ));
        }
        let mut text = format!("Attempt {id}");
        if let Some(record) = record {
            text.push_str(&format!(
                "\nDurable recovery: {:?}\n{}",
                record.state,
                description_text(&record.description)
            ));
            if let Some(failure) = record.failure {
                text.push_str(&format!("\nRecorded failure: {failure}"));
            }
            if let Some(diagnostic) = &record.diagnostic {
                text.push_str(&format!("\nRecorded diagnostic: {diagnostic:?}"));
            }
        }
        if let Some(job) = observation {
            text.push_str(&format!("\nObserved phase: {:?}\nTransaction outcome: {:?}\nCleanup: {:?}\nTransferred: {} bytes",job.phase,job.outcome,job.cleanup,job.bytes_processed));
            if record.is_none() {
                text.push_str(&format!(
                    "\nSource: {} / {}.{}\nDestination: {} / {}.{}",
                    job.intent.source.connection_id,
                    job.intent.source.schema,
                    job.intent.source.table,
                    job.intent.destination.connection_id,
                    job.intent.destination.schema,
                    job.intent.destination.table
                ));
            }
            if let Some(failure) = job.failure {
                text.push_str(&format!("\nFailure: {failure}"));
            }
            if let Some(diagnostic) = &job.diagnostic {
                text.push_str(&format!("\nDiagnostic: {diagnostic:?}"));
            }
        } else {
            text.push_str("\nRecovered record only. This does not recreate an executable review or prove rollback. Inspect the destination before reconciliation.");
        }
        Some(text)
    }
    fn choices(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self
            .connections
            .as_ref()
            .map_or(0, |choices| choices.rows.len());
        uniform_list(
            "table-copy-connections",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|index| {
                        let (id, name) = this.connections.as_ref()?.rows.get(index)?;
                        let label = format!("{name} ({id})");
                        let revision = this.connection_revision;
                        let weak = cx.weak_entity();
                        Some(
                            div()
                                .id(("table-copy-connection", index))
                                .role(Role::ListBoxOption)
                                .aria_label(label.clone())
                                .aria_selected(this.choice == Some(index))
                                .bg(if this.choice == Some(index) {
                                    rgb(0x222222)
                                } else {
                                    rgb(0)
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.choose(index, revision, window, cx)
                                }))
                                .on_a11y_action(
                                    gpui::accesskit::Action::Click,
                                    move |_, window, cx| {
                                        weak.update(cx, |this, cx| {
                                            this.choose(index, revision, window, cx)
                                        })
                                        .ok();
                                    },
                                )
                                .child(label),
                        )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.choices_scroll)
        .h(px(120.))
        .into_any_element()
    }
    fn jobs_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.ids(cx).len();
        uniform_list(
            "table-copy-job-list",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let ids = this.ids(cx);
                range
                    .filter_map(|index| {
                        let id = *ids.get(index)?;
                        let label = this.job_label(id, cx);
                        let weak = cx.weak_entity();
                        Some(
                            div()
                                .id(("table-copy-job", index))
                                .role(Role::ListBoxOption)
                                .aria_label(label.clone())
                                .aria_selected(this.selected == Some(id))
                                .bg(if this.selected == Some(id) {
                                    rgb(0x222222)
                                } else {
                                    rgb(0)
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.select(id, cx);
                                    window.focus(&this.job_focus, cx);
                                }))
                                .on_a11y_action(
                                    gpui::accesskit::Action::Click,
                                    move |_, window, cx| {
                                        weak.update(cx, |this, cx| {
                                            this.select(id, cx);
                                            window.focus(&this.job_focus, cx);
                                        })
                                        .ok();
                                    },
                                )
                                .child(label),
                        )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.jobs_scroll)
        .h(px(140.))
        .into_any_element()
    }
    fn columns(&self, cx: &mut Context<Self>) -> AnyElement {
        let review = self
            .store
            .read(cx)
            .review_payload()
            .filter(|review| Some(review.attempt_id()) == self.selected);
        let count = review.map_or(0, |review| review.columns().len());
        uniform_list(
            "table-copy-columns",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|index| {
                        let review = this
                            .store
                            .read(cx)
                            .review_payload()
                            .filter(|review| Some(review.attempt_id()) == this.selected)?;
                        let attempt = review.attempt_id();
                        let column = review.columns().get(index)?;
                        let label = format!("{}: {}", column.name, column_action(column.action));
                        let weak = cx.weak_entity();
                        Some(
                            div()
                                .id(("table-copy-column", index))
                                .role(Role::ListBoxOption)
                                .aria_label(label.clone())
                                .aria_selected(this.column == Some(index))
                                .bg(if this.column == Some(index) {
                                    rgb(0x222222)
                                } else {
                                    rgb(0)
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.select_column(attempt, index, window, cx)
                                }))
                                .on_a11y_action(
                                    gpui::accesskit::Action::Click,
                                    move |_, window, cx| {
                                        weak.update(cx, |this, cx| {
                                            this.select_column(attempt, index, window, cx)
                                        })
                                        .ok();
                                    },
                                )
                                .child(label),
                        )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.columns_scroll)
        .h(px(130.))
        .into_any_element()
    }
    fn select_column(
        &mut self,
        attempt: TableCopyAttemptId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if Some(attempt) == self.selected
            && self.store.read(cx).review_payload().is_some_and(|review| {
                review.attempt_id() == attempt && index < review.columns().len()
            })
        {
            self.column = Some(index);
            window.focus(&self.column_focus, cx);
            cx.notify();
        }
    }
}
impl Render for TableCopyView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_fields(window, cx);
        let mut content = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().flex().flex_wrap().gap_1().children(
                (0..ACTIONS.len()).map(|index| self.button(index, cx)),
            ))
            .child(label("table-copy-status", self.status.clone()))
            .child(label(
                "table-copy-boundaries",
                "Append matching columns in one destination transaction. No table creation or replacement. Identity values are copied; destination defaults and generated expressions remain authoritative. Cancellation after COMMIT cannot undo copied rows. Jobs survive tab closure.".into(),
            ));
        if let Some(message) = self.store.read(cx).message() {
            content = content.child(label("table-copy-store-message", message.to_owned()));
        }
        for (index, field) in self.fields.iter().enumerate() {
            content = content.child(
                div()
                    .flex()
                    .gap_2()
                    .child(FIELD_LABELS[index])
                    .child(div().flex_1().child(field.clone())),
            );
        }
        if self.choosing.is_some() {
            content = content.child(
                div()
                    .id("table-copy-connection-choices")
                    .role(Role::ListBox)
                    .aria_label("Stored PostgreSQL connection choices")
                    .track_focus(&self.choice_focus)
                    .tab_stop(true)
                    .child(self.choices(cx)),
            );
        }
        if !self.ids(cx).is_empty() {
            content = content.child(
                div()
                    .id("table-copy-jobs")
                    .role(Role::ListBox)
                    .aria_label("Table copy attempts and durable recovery records")
                    .track_focus(&self.job_focus)
                    .tab_stop(true)
                    .child(self.jobs_list(cx)),
            );
        }
        if let Some(details) = self.details(cx) {
            content = content.child(label("table-copy-details", details));
        }
        let review_text=self.store.read(cx).review_payload().filter(|review|Some(review.attempt_id())==self.selected).map(|review| {
            let draft_differs=self.intent(cx).map_or(true,|intent|intent!=review.description().intent);
            format!("Exact accepted review\n{}\n{}",description_text(review.description()),
                if draft_differs {"Editable setup differs from this accepted review. Apply targets only the accepted endpoints above."} else {"Editable setup matches this accepted review."})
        });
        let column_text = self
            .store
            .read(cx)
            .review_payload()
            .filter(|review| Some(review.attempt_id()) == self.selected)
            .and_then(|review| self.column.and_then(|index| review.columns().get(index)))
            .map(|column| {
                format!(
                    "Column: {}\nAction: {}\nSource type: {}\nDestination type: {}",
                    column.name,
                    column_action(column.action),
                    column
                        .source_type
                        .as_deref()
                        .unwrap_or("No matching source column"),
                    column.destination_type
                )
            });
        if let Some(review_text) = review_text {
            content = content.child(label("table-copy-exact-review", review_text));
            content = content.child(
                div()
                    .id("table-copy-column-review")
                    .role(Role::ListBox)
                    .aria_label("Exact name-mapped destination column review")
                    .track_focus(&self.column_focus)
                    .tab_stop(true)
                    .child(self.columns(cx)),
            );
            if let Some(column_text) = column_text {
                content = content.child(label("table-copy-column-details", column_text));
            }
        }
        div()
            .id("table-copy-view")
            .role(Role::Group)
            .aria_label("PostgreSQL table copy")
            .track_focus(&self.root)
            .size_full()
            .bg(rgb(0))
            .text_color(rgb(0xffffff))
            .capture_key_down(cx.listener(Self::key))
            .overflow_y_scroll()
            .track_scroll(&self.details_scroll)
            .child(content)
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
fn column_action(action: TableCopyColumnAction) -> &'static str {
    match action {
        TableCopyColumnAction::Copy => "Copy source value",
        TableCopyColumnAction::CopyIdentity => "Copy source identity value",
        TableCopyColumnAction::DefaultOrNull => "Destination default or NULL",
        TableCopyColumnAction::Generated => "Destination generated expression",
    }
}
fn description_text(description: &dbunk_lib::backend::table_copy::TableCopyDescription) -> String {
    let source = &description.source_connection;
    let target = &description.destination_connection;
    format!(
        "Source: {} ({}) / {}.{}\nSource endpoint: {}:{} / {} / user {} / environment {}\nDestination: {} ({}) / {}.{}\nDestination endpoint: {}:{} / {} / user {} / environment {}\nDestination safe mode: {}; read-only: {}\nColumns: {} copied ({} identity), {} defaulted or NULL, {} generated\nMapping identity: {}",
        source.connection_name,
        description.intent.source.connection_id,
        description.intent.source.schema,
        description.intent.source.table,
        source.host,
        source.port,
        source.database,
        source.user,
        source.environment,
        target.connection_name,
        description.intent.destination.connection_id,
        description.intent.destination.schema,
        description.intent.destination.table,
        target.host,
        target.port,
        target.database,
        target.user,
        target.environment,
        target.safe_mode,
        target.read_only,
        description.copied_columns,
        description.identity_columns,
        description.defaulted_columns,
        description.generated_columns,
        description.mapping_sha256
    )
}
