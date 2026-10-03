use super::*;
impl TableCopyView {
    pub(super) fn enabled(&self, action: Action, cx: &gpui::App) -> bool {
        let store = self.store.read(cx);
        let current = store.observation_current();
        let observation = self.observation(cx);
        let terminal = observation
            .is_none_or(|job| job.phase.terminal() && job.cleanup == TableCopyCleanup::Complete);
        match action {
            Action::Refresh => self.editable,
            Action::Source | Action::Destination => {
                self.editable
                    && self.connections_current
                    && self
                        .connections
                        .as_ref()
                        .is_some_and(|choices| !choices.rows.is_empty())
            }
            Action::Prepare => {
                self.editable
                    && !store.busy()
                    && !self.pending_seed
                    && self.fields.len() == 4
                    && self.connections_current
                    && self.source.is_some()
                    && self.destination.is_some()
            }
            Action::Review => {
                self.editable
                    && !store.busy()
                    && observation.is_some_and(|job| {
                        matches!(
                            job.phase,
                            TableCopyPhase::ReadyReview | TableCopyPhase::AwaitingConfirmation
                        )
                    })
            }
            Action::Apply => {
                self.editable
                    && !store.busy()
                    && store
                        .review_payload()
                        .is_some_and(|review| Some(review.attempt_id()) == self.selected)
                    && self
                        .journal(cx)
                        .is_some_and(|job| job.state == WorkspaceTableCopyState::Staged)
            }
            Action::Cancel => {
                self.editable && current && observation.is_some_and(|job| !job.phase.terminal())
            }
            Action::Reconcile => {
                self.editable
                    && current
                    && terminal
                    && self.journal(cx).is_some_and(|job| {
                        matches!(
                            job.state,
                            WorkspaceTableCopyState::Staged
                                | WorkspaceTableCopyState::Applying
                                | WorkspaceTableCopyState::Unknown
                        )
                    })
            }
            Action::Dismiss => {
                self.editable
                    && current
                    && terminal
                    && self.selected.is_some()
                    && self.journal(cx).is_none_or(|job| {
                        matches!(
                            job.state,
                            WorkspaceTableCopyState::Completed { .. }
                                | WorkspaceTableCopyState::RolledBack
                                | WorkspaceTableCopyState::NotStarted
                                | WorkspaceTableCopyState::Reconciled
                        )
                    })
            }
        }
    }
    pub(super) fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action, cx) {
            return;
        }
        if self.composing(window, cx) {
            self.status =
                "Finish text composition before changing table copy setup or actions.".into();
            cx.notify();
            return;
        }
        match action {
            Action::Source | Action::Destination => {
                self.choosing = Some(if matches!(action, Action::Source) {
                    Side::Source
                } else {
                    Side::Destination
                });
                self.choice = None;
                window.focus(&self.choice_focus, cx);
            }
            Action::Prepare => match self.intent(cx) {
                Ok(intent) => {
                    if let Some(id) = self.store.update(cx, |store, cx| store.begin(intent, cx)) {
                        self.select(id, cx);
                        window.focus(&self.job_focus, cx);
                    }
                }
                Err(error) => self.status = error.into(),
            },
            Action::Refresh => self.store.update(cx, |store, cx| store.refresh(cx)),
            Action::Review => {
                if let Some(id) = self.selected {
                    self.store.update(cx, |store, cx| store.review(id, cx));
                    self.column = None;
                }
            }
            Action::Apply => {
                if let Some(id) = self.selected {
                    self.store.update(cx, |store, cx| store.apply(id, cx));
                }
            }
            Action::Cancel => {
                if let Some(id) = self.selected {
                    self.store.update(cx, |store, cx| store.cancel(id, cx));
                }
            }
            Action::Reconcile => {
                if let Some(id) = self.selected {
                    if self.armed == Some(id) {
                        self.store.update(cx, |store, cx| store.reconcile(id, cx));
                        self.armed = None;
                    } else {
                        self.armed = Some(id);
                        self.status="Inspect the destination independently, then acknowledge reconciliation. This does not claim success or rollback.".into();
                    }
                }
            }
            Action::Dismiss => {
                if let Some(id) = self.selected {
                    self.store.update(cx, |store, cx| store.dismiss(id, cx));
                    // Preserve missing selection identity; never silently select another job.
                    self.column = None;
                    self.armed = None;
                }
            }
        }
        cx.notify();
    }
    pub(super) fn intent(&self, cx: &gpui::App) -> Result<TableCopyIntent, &'static str> {
        let source = self.source.as_ref().ok_or("Choose a source connection")?;
        let destination = self
            .destination
            .as_ref()
            .ok_or("Choose a destination connection")?;
        let choices = self
            .connections
            .as_ref()
            .ok_or("Connection choices unavailable")?;
        if ![source, destination]
            .iter()
            .all(|id| choices.rows.iter().any(|(key, _)| key == *id))
        {
            return Err("A selected connection is no longer available");
        }
        let values = self
            .fields
            .iter()
            .map(|field| field.read(cx).value(cx))
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != 4 || !values.iter().all(|value| valid_name(value)) {
            return Err("All schema and table names must contain 1–63 UTF-8 bytes without NUL");
        }
        TableCopyIntent::new(
            TableCopyEndpoint {
                connection_id: source.clone(),
                schema: values[0].clone(),
                table: values[1].clone(),
            },
            TableCopyEndpoint {
                connection_id: destination.clone(),
                schema: values[2].clone(),
                table: values[3].clone(),
            },
        )
        .map_err(|_| "Table copy endpoints are invalid")
    }
    pub(super) fn select(&mut self, id: TableCopyAttemptId, cx: &mut Context<Self>) {
        if self.selected != Some(id) {
            self.selected = Some(id);
            self.column = None;
            self.armed = None;
        }
        cx.notify();
    }
    pub(super) fn choose(
        &mut self,
        index: usize,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if revision != self.connection_revision
            || !self.editable
            || !self.connections_current
            || self.composing(window, cx)
        {
            return;
        }
        let Some(side) = self.choosing else {
            return;
        };
        let Some((id, _)) = self
            .connections
            .as_ref()
            .and_then(|choices| choices.rows.get(index))
        else {
            return;
        };
        let id = id.clone();
        let (selected, offset) = match side {
            Side::Source => (&mut self.source, 0),
            Side::Destination => (&mut self.destination, 2),
        };
        if selected.as_ref() != Some(&id) {
            *selected = Some(id);
            for index in offset..offset + 2 {
                self.seeds[index].clear();
                if let Some(field) = self.fields.get(index) {
                    field
                        .update(cx, |field, cx| field.set_value(String::new(), window, cx))
                        .ok();
                }
            }
        }
        self.choosing = None;
        self.choice = None;
        window.focus(&self.buttons[if side == Side::Source { 0 } else { 1 }], cx);
        cx.notify();
    }
    pub(super) fn key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composing(window, cx) {
            return;
        }
        let modifiers = &event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        let key = event.keystroke.key.as_str();
        if key == "tab" {
            let handles = self.focus_order(cx);
            if !handles.is_empty() {
                let current = handles.iter().position(|handle| handle.is_focused(window));
                let next = if modifiers.shift {
                    current.map_or(handles.len() - 1, |index| {
                        (index + handles.len() - 1) % handles.len()
                    })
                } else {
                    current.map_or(0, |index| (index + 1) % handles.len())
                };
                window.focus(&handles[next], cx);
                cx.stop_propagation();
                window.prevent_default();
            }
        } else if self.choosing.is_some() && self.choice_focus.is_focused(window) {
            let count = self
                .connections
                .as_ref()
                .map_or(0, |choices| choices.rows.len());
            if let Some(index) = move_index(self.choice, count, key) {
                self.choice = Some(index);
                self.choices_scroll
                    .scroll_to_item(index, ScrollStrategy::Nearest);
            } else if key == "enter" {
                if let Some(index) = self.choice {
                    self.choose(index, self.connection_revision, window, cx);
                }
            } else if key == "escape" {
                self.choosing = None;
                window.focus(&self.buttons[0], cx);
            } else {
                return;
            }
            cx.stop_propagation();
            cx.notify();
        } else if self.job_focus.is_focused(window) {
            let ids = self.ids(cx);
            let current = self
                .selected
                .and_then(|id| ids.iter().position(|item| *item == id));
            if let Some(index) = move_index(current, ids.len(), key) {
                self.select(ids[index], cx);
                self.jobs_scroll
                    .scroll_to_item(index, ScrollStrategy::Nearest);
                cx.stop_propagation();
            }
        } else if self.column_focus.is_focused(window) {
            let count = self
                .store
                .read(cx)
                .review_payload()
                .filter(|review| Some(review.attempt_id()) == self.selected)
                .map_or(0, |review| review.columns().len());
            if let Some(index) = move_index(self.column, count, key) {
                self.column = Some(index);
                self.columns_scroll
                    .scroll_to_item(index, ScrollStrategy::Nearest);
                cx.stop_propagation();
                cx.notify();
            }
        }
    }
}
fn move_index(current: Option<usize>, count: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match key {
        "down" => Some(current.map_or(0, |index| (index + 1).min(count - 1))),
        "up" => Some(current.map_or(count - 1, |index| index.saturating_sub(1))),
        "home" => Some(0),
        "end" => Some(count - 1),
        _ => None,
    }
}
