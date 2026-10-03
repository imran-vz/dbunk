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
        div()
            .id(("pg-tool-control", index))
            .role(if toggle { Role::CheckBox } else { Role::Button })
            .aria_label(label)
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
            .px_2()
            .py_1()
            .text_color(if enabled {
                crate::style::text()
            } else {
                crate::style::dim()
            })
            .when(selected, |element| element.bg(crate::style::select()))
            .focus(|style| style.bg(crate::style::line()))
            .child(label)
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
impl Render for PgToolView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.rebuild_fields(window, cx);
        let connection = self
            .connection
            .as_deref()
            .unwrap_or("Select a connection")
            .to_owned();
        let filename = self
            .setup
            .as_ref()
            .and_then(Setup::filename)
            .and_then(|name| name.to_str())
            .unwrap_or("No file selected")
            .to_owned();
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
        let fields = self.fields.clone();
        div().id("pg-tool-setup").role(Role::Group).aria_label("PostgreSQL backup and restore")
            .track_focus(&self.focus).size_full().flex().flex_col().bg(crate::style::bg()).text_color(crate::style::text())
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
            .when_some(limits, |body, limits| body.child(div().px_2().child(limits)))
            .child(div().flex().flex_wrap().p_2().child("Backup / Restore").children((0..2).map(|index|self.button(index,cx))))
            .child(div().id("pg-tool-connection").px_2().role(Role::Label).aria_label(format!("Connection: {connection}")).child(format!("Connection: {connection}")))
            .child(div().id("pg-tool-body").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.scroll)
                .child(div().flex().flex_wrap().p_2().children((2..8).filter(|index|backup || !matches!(index,4..=6)).map(|index|self.button(index,cx))))
                .when(backup&&self.scope!=ScopeChoice::Database,|body|body.when_some(fields,|body,(schema,table)|{
                    body.child(div().px_2().child("Schema").child(schema)).when(self.scope==ScopeChoice::Table,|body|body.child(div().px_2().child("Table").child(table)))
                }))
                .when(backup,|body|body.child(self.choices_element(cx)))
                .child(div().id("pg-tool-selected-file").px_2().py_1().role(Role::Label).aria_label(format!("Selected file: {filename}")).child(filename))
                .child(div().px_2().child(if backup{"Choose a new destination. Existing files are never replaced."}else{"Restore targets the database. Select plain or custom format explicitly; the filename does not determine it."}))
                .child(div().flex().flex_wrap().p_2().children([8,9,16].map(|index|self.button(index,cx))))
                .child(div().id("pg-tool-status").px_2().role(Role::Label).aria_label(status.clone()).child(status))
                .when_some(store_message,|body,message|body.child(div().id("pg-tool-observation-error").px_2().role(Role::Alert).child(message)))
                .when_some(review,|body,review|body.child(div().id("pg-tool-review").p_2().role(Role::Label).aria_label(review.clone()).whitespace_normal().child(review)))
                .child(div().flex().flex_wrap().p_2().children((10..16).map(|index|self.button(index,cx))))
                .when(unresolved,|body|body.child(div().id("pg-tool-unknown-outcome").role(Role::Alert).p_2().child("An unknown restore outcome remains. Inspect the database, then acknowledge and dismiss that job before preparing another.")))
                .child(div().px_2().child("Recent jobs · This session · Up to one hour and 32 finished jobs"))
                .child(div().id("pg-tool-jobs").role(Role::ListBox).aria_label("Recent backup and restore jobs").track_focus(&self.list).tab_index(0).min_h(px(28.))
                    .when(jobs.is_empty(),|list|list.child("No observed jobs for this connection. Missing records do not establish whether a queued start was admitted."))
                    .children(jobs.into_iter().enumerate().map(|(index,(attempt,label))|{
                        let selected=self.selected==Some(attempt);let weak=cx.weak_entity();
                        div().id(("pg-tool-job",index)).role(Role::ListBoxOption).aria_label(label.clone()).aria_selected(selected).px_2().py_1()
                            .when(selected,|row|row.bg(crate::style::select())).child(label)
                            .on_click(cx.listener(move|this,_,window,cx|{this.unknown_ack=None;this.selected=Some(attempt);window.focus(&this.list,cx);cx.notify();}))
                            .on_a11y_action(gpui::accesskit::Action::Click,move|_,window,cx|{weak.update(cx,|this,cx|{this.unknown_ack=None;this.selected=Some(attempt);window.focus(&this.list,cx);cx.notify();}).ok();})
                    })))
                .when_some(details,|body,details|body.child(div().id("pg-tool-details").role(Role::Label).aria_label(details.clone()).track_focus(&self.details).tab_index(0).p_2().whitespace_normal().child(details)))
                .when(self.selected.is_some()&&self.selected_row(cx).is_none(),|body|body.child(div().p_2().child("Selected job is missing or expired. Refresh observation; another job has not been selected.")))
                .child(div().p_2().child("Closing this setup leaves admitted jobs running. Cancel active jobs and wait for cleanup before changing connections or credentials. Client preflight reports tool availability; restore does not create a database or remap owners.")))
    }
}
