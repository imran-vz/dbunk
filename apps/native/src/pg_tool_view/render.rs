use super::*;

impl PgToolView {
    pub(super) fn button(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let (action, default_label) = ACTIONS[index];
        let enabled = self.enabled(action, cx);
        let selected = self.setup.as_ref().is_some_and(|setup| match action {
            Action::Backup => setup.operation() == Operation::Backup,
            Action::Restore => setup.operation() == Operation::Restore,
            Action::Plain => setup.format() == Format::Plain,
            Action::Custom => setup.format() == Format::Custom,
            Action::Database => self.scope == ScopeChoice::Database,
            Action::Schema => self.scope == ScopeChoice::Schema,
            Action::Table => self.scope == ScopeChoice::Table,
            Action::Clean => setup.clean(),
            Action::SchemaChoices => self.metadata.kind == choices::ChoiceKind::Schema,
            Action::TableChoices => self.metadata.kind == choices::ChoiceKind::Table,
            _ => false,
        });
        let label = if matches!(action, Action::Dismiss)
            && self.selected_row(cx).is_some_and(choices::unknown_restore)
        {
            if self.unknown_ack == self.selected {
                "I inspected the target; dismiss unknown outcome"
            } else {
                "Inspect target before dismissing unknown outcome"
            }
        } else if matches!(action, Action::Clean) {
            if self
                .setup
                .as_ref()
                .is_some_and(|setup| setup.operation() == Operation::Restore)
            {
                "Drop existing objects before restore"
            } else {
                "Include drop statements in backup"
            }
        } else {
            default_label
        };
        let toggle = matches!(
            action,
            Action::Backup
                | Action::Restore
                | Action::Plain
                | Action::Custom
                | Action::Database
                | Action::Schema
                | Action::Table
                | Action::Clean
                | Action::SchemaChoices
                | Action::TableChoices
        );
        let weak = cx.weak_entity();
        let id = ("pg-tool-control", index);
        let base = if matches!(action, Action::Clean) {
            crate::overview_view::chrome::check_row(id, label, selected, enabled)
        } else if toggle {
            crate::ui::segment(id, label, selected, enabled)
        } else if index == self.next_step(cx) {
            crate::ui::tool_button_accent(id, label, None, enabled)
        } else {
            crate::ui::tool_button(id, label, None, enabled, false)
        };
        base.role(if toggle { Role::CheckBox } else { Role::Button })
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
                if toggle {
                    builder
                        .parent_node()
                        .set_toggled(gpui::accesskit::Toggled::from(selected));
                }
            })
            .track_focus(&self.buttons[index])
            .tab_index(0)
            .tab_stop(enabled)
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .into_any_element()
    }
    fn review_text(&self, cx: &gpui::App) -> Option<String> {
        let store = self.store.read(cx);
        let review = store
            .confirmation()
            .map(|confirmation| confirmation.review())
            .or_else(|| store.review())?;
        if Some(review.attempt_id()) != self.selected
            || Some(review.observation().connection_id.as_str()) != self.connection.as_deref()
        {
            return None;
        }
        let target = review.target();
        let job = review.observation();
        let scope = match &job.scope {
            Scope::Database => "Entire database".into(),
            Scope::Schema { schema } => format!("Schema: {schema}"),
            Scope::Table { schema, table } => format!("Table: {schema}.{table}"),
        };
        Some(format!(
            "Exact reviewed target\nConnection: {}\nHost: {}\nPort: {}\nDatabase: {}\nUser: {}\nEnvironment: {}\nSafe mode: {}\nRead-only policy: {}\nOperation: {}\nFormat: {}\nScope: {}\nFile: {}\nImmutable copied source size: {}\nCleanup option: {}\n{}\n{}",
            target.connection_name,
            target.host,
            target.port,
            target.database,
            target.user,
            target.environment,
            target.safe_mode,
            if target.read_only {
                "Enabled"
            } else {
                "Disabled"
            },
            pg_tool_jobs::operation_label(job.kind),
            if job.format == Format::Plain {
                "Plain SQL"
            } else {
                "Custom archive"
            },
            scope,
            job.file_name,
            job.source_bytes
                .map_or_else(|| "Unavailable".into(), |bytes| format!("{bytes} B")),
            if job.clean { "Enabled" } else { "Disabled" },
            if review.requires_confirmation() {
                "Stored safety policy requires confirmation of this exact restore"
            } else {
                "Current stored policy permits this reviewed operation"
            },
            if job.kind == Operation::Restore {
                "Only restore trusted files. SQL runs with this connection's privileges. This targets the database, not only the contextual table. Cancellation cannot undo a committed restore."
            } else {
                "Backup publishes a new file only after success. Existing files are never replaced."
            }
        ))
    }
}
/// Backup / Restore job table: file, operation and state.
fn job_columns() -> Vec<crate::ui::TableColumn> {
    vec![
        crate::ui::TableColumn::flex("File").mono(),
        crate::ui::TableColumn::fixed("Operation", 100.),
        crate::ui::TableColumn::fixed("State", 180.),
    ]
}
const JOBS_NOTE: &str = "This session keeps up to one hour and 32 finished jobs. Missing records do not establish whether a queued start was admitted. Closing this setup leaves admitted jobs running. Cancel active jobs and wait for cleanup before changing connections or credentials. Client preflight reports tool availability; restore does not create a database or remap owners.";
impl Render for PgToolView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::overview_view::chrome::{self, ToolConnection};
        use crate::style;
        self.rebuild_fields(window, cx);
        let filename = self
            .setup
            .as_ref()
            .and_then(Setup::filename)
            .and_then(|name| name.to_str())
            .map(str::to_owned);
        let backup = self
            .setup
            .as_ref()
            .is_none_or(|setup| setup.operation() == Operation::Backup);
        let jobs = self
            .visible_jobs(cx)
            .into_iter()
            .filter_map(|index| {
                let capture = self.store.read(cx).capture()?;
                Some((capture.key(index)?, capture.row_label(index)?))
            })
            .collect::<Vec<_>>();
        let details = self.selected.and_then(|selected| {
            let capture = self.store.read(cx).capture()?;
            capture.details(capture.index_for_key(selected)?)
        });
        let review = self.review_text(cx);
        let store_message = self.store.read(cx).message().map(str::to_owned);
        let status = self.status.clone();
        let unresolved = self.store.read(cx).capture().is_some_and(|capture| {
            capture.rows().iter().any(|job| {
                Some(job.connection_id.as_str()) == self.connection.as_deref()
                    && choices::unknown_restore(job)
            })
        });
        let limits = self
            .store
            .read(cx)
            .capture()
            .map(|capture| capture.limits());
        let jobs_note = limits.map_or_else(
            || JOBS_NOTE.to_owned(),
            |limits| format!("{limits}\n{JOBS_NOTE}"),
        );
        let choices = backup.then(|| self.choices_element(cx));
        let fields = self.fields.clone();
        let field_focus = fields.as_ref().map(|(schema, table)| {
            (
                schema.focus_handle(cx).contains_focused(window, cx),
                table.focus_handle(cx).contains_focused(window, cx),
            )
        });
        let parsed_review = review.as_deref().map(chrome::parse_details);
        let parsed_job = details.as_deref().map(chrome::parse_details);
        let columns = job_columns();
        let list_focused = self.list.is_focused(window);
        let connection = ToolConnection::lookup(&self.connections, self.connection.as_deref());
        let context = chrome::connection_context(connection, self.connection.is_some());
        let shown = |index: usize| self.enabled(ACTIONS[index].0, cx);
        let review_step = review.is_some() || [11, 12, 13].into_iter().any(shown);
        let header_actions = vec![
            crate::ui::segment_group()
                .children((0..2).map(|index| self.button(index, cx)))
                .into_any_element(),
            self.button(16, cx),
        ];
        let well = |field: Entity<Field>, label: &'static str, focused: bool| {
            crate::ui::labelled(
                label,
                crate::ui::input_well(false, focused).child(div().flex_1().min_w_0().child(field)),
                None,
            )
        };
        div().id("pg-tool-setup").role(Role::Group).aria_label("PostgreSQL backup and restore")
            .track_focus(&self.focus).size_full().flex().flex_col().bg(style::bg()).text_color(style::text()).text_size(px(style::FONT))
            .capture_key_down(cx.listener(|this,event:&KeyDownEvent,window,cx|{
                if this.composing(window,cx){return;}
                let modifiers=event.keystroke.modifiers;
                if modifiers.platform||modifiers.control||modifiers.alt{return;}
                if event.keystroke.key=="tab" {
                    let order=this.focus_order(cx);let current=order.iter().position(|handle|handle.contains_focused(window,cx));
                    let next=if modifiers.shift{current.map_or(order.len()-1,|index|(index+order.len()-1)%order.len())}else{current.map_or(0,|index|(index+1)%order.len())};
                    window.focus(&order[next],cx);cx.stop_propagation();return;
                }
                if this.details.is_focused(window)&&event.keystroke.key=="escape"{window.focus(&this.list,cx);cx.stop_propagation();return;}
                if this.choice_key(event,window,cx){cx.stop_propagation();return;}
                if !this.list.is_focused(window){return;}
                let jobs=this.visible_jobs(cx);
                let current=jobs.iter().position(|index|this.store.read(cx).capture().and_then(|capture|capture.key(*index))==this.selected);
                let next=match event.keystroke.key.as_str(){
                    "up"=>current.unwrap_or(0).saturating_sub(1),"down"=>current.map_or(0,|index|(index+1).min(jobs.len().saturating_sub(1))),
                    "home"=>0,"end"=>jobs.len().saturating_sub(1),
                    "enter" if this.selected_row(cx).is_some()=>{window.focus(&this.details,cx);cx.stop_propagation();return;},_=>return,
                };
                this.unknown_ack=None;
                this.selected=jobs.get(next).and_then(|index|this.store.read(cx).capture().and_then(|capture|capture.key(*index)));
                cx.notify();cx.stop_propagation();
            }))
            .child(crate::ui::tool_header("Backup / Restore", context, header_actions))
            .child(
                div()
                    .id("pg-tool-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(px(style::INSET_CONTENT))
                    .py(px(style::S5))
                    .flex()
                    .flex_col()
                    .gap(px(style::S7))
                    // What to back up, or how to read the restore file.
                    .child(
                        chrome::tool_column()
                            .child(crate::ui::heading(if backup { "What to back up" } else { "Restore from" }))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_end()
                                    .gap(px(style::S6))
                                    .child(crate::ui::labelled(
                                        "Format",
                                        crate::ui::segment_group().children((2..4).map(|index| self.button(index, cx))),
                                        None,
                                    ))
                                    .when(backup, |row| {
                                        row.child(crate::ui::labelled(
                                            "Scope",
                                            crate::ui::segment_group().children((4..7).map(|index| self.button(index, cx))),
                                            None,
                                        ))
                                    }),
                            )
                            .when(backup && self.scope != ScopeChoice::Database, |column| {
                                column.when_some(fields.zip(field_focus), |column, ((schema, table), (schema_focused, table_focused))| {
                                    column.child(
                                        div()
                                            .flex()
                                            .gap(px(style::S5))
                                            .child(div().flex_1().min_w_0().child(well(schema, "Schema", schema_focused)))
                                            .child(div().flex_1().min_w_0().when(self.scope == ScopeChoice::Table, |cell| {
                                                cell.child(well(table, "Table", table_focused))
                                            })),
                                    )
                                })
                            })
                            .children(choices)
                            .child(self.button(7, cx)),
                    )
                    // File.
                    .child(
                        chrome::tool_column()
                            .child(crate::ui::heading(if backup { "Destination file" } else { "Backup file" }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(style::S3))
                                    .child(
                                        div()
                                            .id("pg-tool-selected-file")
                                            .role(Role::Label)
                                            .aria_label(format!(
                                                "Selected file: {}",
                                                filename.as_deref().unwrap_or("No file selected")
                                            ))
                                            .flex_1()
                                            .min_w_0()
                                            .h(px(style::FIELD))
                                            .px(px(style::S4))
                                            .flex()
                                            .items_center()
                                            .rounded(px(style::R_MD))
                                            .border_1()
                                            .border_color(style::line())
                                            .bg(style::bg())
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .map(|well| match &filename {
                                                Some(name) => well
                                                    .font_family(style::MONO)
                                                    .text_size(px(style::FONT_DATA))
                                                    .child(name.clone()),
                                                None => well
                                                    .text_color(style::faint())
                                                    .child("No file selected"),
                                            }),
                                    )
                                    .child(self.button(8, cx)),
                            )
                            .child(chrome::note(if backup {
                                "Choose a new destination. Existing files are never replaced."
                            } else {
                                "Restore targets the database. Select plain or custom format explicitly; the filename does not determine it."
                            }))
                            .child(div().flex().child(self.button(9, cx))),
                    )
                    .when_some(store_message, |body, message| {
                        body.child(div().max_w(px(style::TOOL_COLUMN)).child(crate::ui::shake(
                            format!("pg-tool-observation-error-shake-{message}"),
                            crate::ui::error_banner("pg-tool-observation-error", message),
                        )))
                    })
                    // Review and start: revealed with a prepared job.
                    .when(review_step, |body| {
                        body.child(
                            chrome::tool_column()
                                .child(crate::ui::heading("Review"))
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap(px(style::S2))
                                        .children([11, 12, 13].into_iter().filter(|index| shown(*index)).map(|index| self.button(index, cx))),
                                )
                                .when_some(review.clone().zip(parsed_review), |column, (review, parsed)| {
                                    column.child(
                                        div()
                                            .id("pg-tool-review")
                                            .role(Role::Label)
                                            .aria_label(review)
                                            .flex()
                                            .flex_col()
                                            .children(chrome::detail_rows("pg-tool-review-row", &parsed))
                                            .children(parsed.notes.iter().skip(1).map(|line| {
                                                chrome::note(line.clone()).whitespace_normal()
                                            })),
                                    )
                                }),
                        )
                    })
                    .when(unresolved, |body| {
                        body.child(
                            div()
                                .id("pg-tool-unknown-outcome")
                                .role(Role::Alert)
                                .max_w(px(style::TOOL_COLUMN))
                                .text_color(style::warn())
                                .child("An unknown restore outcome remains. Inspect the database, then acknowledge and dismiss that job before preparing another."),
                        )
                    })
                    // Jobs observed in this session.
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(style::S3))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(px(style::S2))
                                    .child(crate::ui::heading("Recent jobs"))
                                    .child(crate::ui::info_hint("pg-tool-job-notes", jobs_note))
                                    .child(crate::ui::grow())
                                    .children([14, 15].into_iter().filter(|index| shown(*index)).map(|index| self.button(index, cx)))
                                    .child(self.button(10, cx)),
                            )
                            .child(
                                div()
                                    .id("pg-tool-jobs")
                                    .role(Role::ListBox)
                                    .aria_label("Recent backup and restore jobs")
                                    .track_focus(&self.list)
                                    .tab_index(0)
                                    .flex()
                                    .flex_col()
                                    .when(jobs.is_empty(), |list| {
                                        list.child(
                                            div()
                                                .py(px(style::S3))
                                                .text_color(style::dim())
                                                .child("No jobs yet for this connection."),
                                        )
                                    })
                                    .when(!jobs.is_empty(), |list| list.child(crate::ui::data_table_header(&columns)))
                                    .children(jobs.into_iter().enumerate().map(|(index, (attempt, label))| {
                                        let selected = self.selected == Some(attempt);
                                        let weak = cx.weak_entity();
                                        let cells = chrome::split_cells(&label, columns.len())
                                            .into_iter()
                                            .zip(columns.iter())
                                            .map(|(value, column)| crate::ui::data_cell(column, Some(value.into())))
                                            .collect::<Vec<_>>();
                                        crate::ui::data_table_row(("pg-tool-job", index), cells, selected, list_focused)
                                            .role(Role::ListBoxOption)
                                            .aria_label(label)
                                            .aria_selected(selected)
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.unknown_ack = None;
                                                this.selected = Some(attempt);
                                                window.focus(&this.list, cx);
                                                cx.notify();
                                            }))
                                            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                                                weak.update(cx, |this, cx| {
                                                    this.unknown_ack = None;
                                                    this.selected = Some(attempt);
                                                    window.focus(&this.list, cx);
                                                    cx.notify();
                                                })
                                                .ok();
                                            })
                                    })),
                            )
                            .when_some(details.zip(parsed_job), |body, (details, parsed)| {
                                body.child(
                                    div()
                                        .id("pg-tool-details")
                                        .role(Role::Label)
                                        .aria_label(details)
                                        .track_focus(&self.details)
                                        .tab_index(0)
                                        .max_w(px(style::TOOL_COLUMN))
                                        .pt(px(style::S3))
                                        .flex()
                                        .flex_col()
                                        .children(chrome::detail_rows("pg-tool-job-row", &parsed))
                                        .children(parsed.notes.iter().filter(|line| !line.trim().is_empty()).map(|line| {
                                            chrome::note(line.clone()).whitespace_normal()
                                        })),
                                )
                            })
                            .when(self.selected.is_some() && self.selected_row(cx).is_none(), |body| {
                                body.child(chrome::note(
                                    "Selected job is missing or expired. Refresh observation; another job has not been selected.",
                                ))
                            }),
                    ),
            )
            .child(
                crate::ui::status_line().child(
                    div()
                        .id("pg-tool-status")
                        .role(Role::Label)
                        .aria_label(status.clone())
                        .child(status),
                ),
            )
    }
}
