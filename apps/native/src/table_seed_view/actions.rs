use super::*;
impl TableSeedView {
    pub(super) fn enabled(&self, action: Action, cx: &gpui::App) -> bool {
        let store = self.store.read(cx);
        let observation = self.observation(cx);
        let terminal = observation
            .is_none_or(|job| job.phase.terminal() && job.cleanup == TableSeedCleanup::Complete);
        let setup = self.editable
            && !store.busy()
            && !self.pending_preset
            && self.fields.len() == FIELDS.len()
            && self.connections_current
            && self.connection.is_some();
        match action {
            Action::Connection => {
                self.editable
                    && self.connections_current
                    && self
                        .connections
                        .as_ref()
                        .is_some_and(|choices| !choices.rows.is_empty())
            }
            Action::Load => setup && !self.column_dirty,
            Action::Inspect => {
                self.fields.len() == FIELDS.len()
                    && self._setup_lease.is_some()
                    && self.editable
                    && !store.busy()
                    && !self.column_dirty
                    && observation.is_some_and(|job| {
                        matches!(
                            job.phase,
                            TableSeedPhase::NeedsRecipe | TableSeedPhase::ReadyReview
                        )
                    })
            }
            Action::Prepare => setup && self.recipe.is_some(),
            Action::Mode | Action::SaveColumn => self.editable && self.column.is_some(),
            Action::Refresh => self.editable,
            Action::Review => {
                self.editable
                    && !store.busy()
                    && observation.is_some_and(|job| {
                        matches!(
                            job.phase,
                            TableSeedPhase::ReadyReview | TableSeedPhase::AwaitingConfirmation
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
                        .is_some_and(|record| record.state == WorkspaceTableSeedState::Staged)
                    && self.selected.is_some_and(|id| !self.setup_changed(id, cx))
            }
            Action::Cancel => {
                self.editable
                    && store.observation_current()
                    && observation.is_some_and(|job| !job.phase.terminal())
            }
            Action::Reconcile => {
                self.editable
                    && store.observation_current()
                    && terminal
                    && self.journal(cx).is_some_and(|record| {
                        matches!(
                            record.state,
                            WorkspaceTableSeedState::Staged
                                | WorkspaceTableSeedState::Applying
                                | WorkspaceTableSeedState::Unknown
                        )
                    })
            }
            Action::Dismiss => {
                self.editable
                    && store.observation_current()
                    && terminal
                    && self.selected.is_some()
                    && self.journal(cx).is_none_or(|record| {
                        matches!(
                            record.state,
                            WorkspaceTableSeedState::Completed { .. }
                                | WorkspaceTableSeedState::RolledBack
                                | WorkspaceTableSeedState::NotStarted
                                | WorkspaceTableSeedState::Reconciled
                        )
                    })
            }
        }
    }
    pub(super) fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_column_dirty(cx);
        if !self.enabled(action, cx) {
            return;
        }
        if self.composing(window, cx) {
            self.status = "Finish text composition before changing seed setup or actions.".into();
            cx.notify();
            return;
        }
        match action {
            Action::Connection => {
                self.choosing = true;
                self.choice = None;
                window.focus(&self.choice_focus, cx);
            }
            Action::Load | Action::Prepare => {
                if matches!(action, Action::Prepare) && self.column_dirty && !self.save_column(cx) {
                    return;
                }
                match self.intent(matches!(action, Action::Prepare), cx) {
                    Ok(intent) => {
                        // Only an explicit Prepare replaces the exact unstarted
                        // inspection. A different selected running job is untouched.
                        if matches!(action, Action::Prepare) {
                            let recipe = self.recipe.as_ref().expect("enabled recipe");
                            if !recipe.discarded {
                                let id = recipe.attempt;
                                if !self
                                    .store
                                    .update(cx, |store, cx| store.discard_preparation(id, cx))
                                {
                                    return;
                                }
                                self.recipe.as_mut().expect("recipe").discarded = true;
                            }
                        }
                        let endpoint = intent.endpoint.clone();
                        let row_count = intent.row_count;
                        let seed = intent.seed;
                        if let Some(id) = self.store.update(cx, |store, cx| store.begin(intent, cx))
                        {
                            self.setup_anchor = Some(recipe::SetupAnchor {
                                attempt: id,
                                endpoint,
                                row_count,
                                seed,
                            });
                            if matches!(action, Action::Prepare) {
                                let recipe = self.recipe.as_mut().expect("prepared recipe");
                                recipe.attempt = id;
                                recipe.row_count = row_count;
                                recipe.seed = seed;
                                recipe.changed = false;
                                recipe.discarded = false;
                            }
                            self.selected = Some(id);
                            self.armed = None;
                            self.status="Preparation is read-only. Select Edit recipe when columns are ready, or Review to inspect the resolved recipe.".into();
                            window.focus(&self.job_focus, cx);
                        }
                    }
                    Err(error) => self.status = error.into(),
                }
            }
            Action::Inspect => {
                if let Some(id) = self.selected {
                    self.store.update(cx, |store, cx| store.inspect(id, cx));
                    let result = self
                        .store
                        .read(cx)
                        .inspection_payload()
                        .filter(|inspection| inspection.attempt_id() == id)
                        .map(Recipe::capture);
                    match result {
                        Some(Ok(recipe)) => {
                            let inspection = self
                                .store
                                .read(cx)
                                .inspection_payload()
                                .expect("inspected metadata");
                            let endpoint = inspection.intent().endpoint.clone();
                            let rows = inspection.intent().row_count.to_string();
                            let seed = inspection
                                .intent()
                                .seed
                                .map_or_else(String::new, |seed| seed.to_string());
                            self.connection = Some(endpoint.connection_id);
                            self.preset = [endpoint.schema, endpoint.table];
                            for (index, value) in
                                [self.preset[0].clone(), self.preset[1].clone(), rows, seed]
                                    .into_iter()
                                    .enumerate()
                            {
                                self.fields[index]
                                    .update(cx, |field, cx| field.set_value(value, window, cx))
                                    .ok();
                            }
                            self.pending_preset = false;
                            self.recipe = Some(recipe);
                            self.column = None;
                            self.column_dirty = false;
                            self.choosing_mode = false;
                            self.status="Column edits are local until Prepare edited recipe. Save a changed column before selecting another.".into();
                            if !self.recipe.as_ref().expect("recipe").columns.is_empty() {
                                self.select_column(id, 0, window, cx);
                            }
                        }
                        Some(Err(error)) => self.status = error.into(),
                        None => {}
                    }
                }
            }
            Action::Mode => {
                self.choosing_mode = true;
                self.mode_choice = (0..30).find(|index| recipe::mode_at(*index) == Some(self.mode));
                window.focus(&self.mode_focus, cx);
            }
            Action::SaveColumn => {
                self.save_column(cx);
            }
            Action::Refresh => self.store.update(cx, |store, cx| store.refresh(cx)),
            Action::Review => {
                if let Some(id) = self.selected {
                    self.store.update(cx, |store, cx| store.review(id, cx));
                }
            }
            Action::Apply => {
                if let Some(id) = self.selected {
                    self.store.update(cx, |store, cx| store.apply(id, cx));
                    self.status = "Seed apply requested. Check the attempt's transaction outcome and cleanup status.".into();
                    // Applying retires the review controls. Keep keyboard
                    // navigation on the durable receipt that remains mounted.
                    window.focus(&self.details_focus, cx);
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
                    self.armed = None;
                }
            }
        }
        cx.notify();
    }
    pub(super) fn setup_changed(&self, id: TableSeedAttemptId, cx: &gpui::App) -> bool {
        if let Some(recipe) = self.recipe.as_ref().filter(|recipe| recipe.attempt == id) {
            return recipe.changed
                || self.column_dirty
                || self.intent(false, cx).map_or(true, |intent| {
                    recipe.endpoint != intent.endpoint
                        || recipe.row_count != intent.row_count
                        || recipe.seed != intent.seed
                });
        }
        self.setup_anchor
            .as_ref()
            .filter(|anchor| anchor.attempt == id)
            .is_some_and(|anchor| {
                self.intent(false, cx)
                    .map_or(true, |intent| !anchor.matches(&intent))
            })
    }
    fn intent(&self, with_recipe: bool, cx: &gpui::App) -> Result<TableSeedIntent, &'static str> {
        if self.pending_preset || self.fields.len() != FIELDS.len() {
            return Err("Seed fields are unavailable");
        }
        let connection = self
            .connection
            .as_ref()
            .ok_or("Choose a destination connection")?;
        if !self.connections_current
            || !self
                .connections
                .as_ref()
                .is_some_and(|choices| choices.rows.iter().any(|(id, _)| id == connection))
        {
            return Err("Destination connection is unavailable");
        }
        let schema = self.fields[0].read(cx).value(cx)?;
        let table = self.fields[1].read(cx).value(cx)?;
        if !valid_name(&schema) || !valid_name(&table) {
            return Err("Schema and table must each contain 1–63 UTF-8 bytes without NUL");
        }
        let endpoint = TableSeedEndpoint {
            connection_id: connection.clone(),
            schema,
            table,
        };
        let rows = parse_row_count(&self.fields[2].read(cx).value(cx)?)?;
        let seed = parse_seed(&self.fields[3].read(cx).value(cx)?)?;
        let columns = if with_recipe {
            let recipe = self
                .recipe
                .as_ref()
                .ok_or("Load and inspect columns first")?;
            if recipe.endpoint != endpoint {
                return Err(
                    "Edited destination differs from inspected columns; load its columns before preparing",
                );
            }
            recipe.specs()?
        } else {
            Vec::new()
        };
        TableSeedIntent::new(endpoint, rows, seed, columns)
            .map_err(|_| "Seed recipe exceeds its validated request bounds")
    }
    fn save_column(&mut self, cx: &mut Context<Self>) -> bool {
        let result = (|| {
            let index = self.column.ok_or("Select a column")?;
            let draft = ColumnDraft {
                mode: self.mode,
                constant: self.fields[4].read(cx).value(cx)?,
                values_text: self.fields[5].read(cx).value(cx)?,
                null_percent: self.fields[6].read(cx).value(cx)?,
            };
            self.recipe
                .as_mut()
                .ok_or("Inspect a recipe first")?
                .save(index, draft)
        })();
        match result {
            Ok(()) => {
                self.column_dirty = false;
                self.status =
                    "Column saved locally. Prepare edited recipe to obtain a new exact review."
                        .into();
            }
            Err(error) => self.status = error.into(),
        }
        cx.notify();
        result.is_ok()
    }
    pub(super) fn select_column(
        &mut self,
        id: TableSeedAttemptId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable || self.composing(window, cx) {
            return;
        }
        self.refresh_column_dirty(cx);
        if self.column_dirty {
            self.status = "Save the current column before selecting another.".into();
            cx.notify();
            return;
        }
        let Some(recipe) = self.recipe.as_ref().filter(|recipe| recipe.attempt == id) else {
            return;
        };
        let Some(draft) = recipe.drafts.get(index) else {
            return;
        };
        let values = [
            draft.constant.clone(),
            draft.values_text.clone(),
            draft.null_percent.clone(),
        ];
        self.mode = draft.mode;
        self.column = Some(index);
        self.choosing_mode = false;
        for (offset, value) in values.into_iter().enumerate() {
            self.fields[offset + 4]
                .update(cx, |field, cx| field.set_value(value, window, cx))
                .ok();
        }
        self.column_dirty = false;
        window.focus(&self.column_focus, cx);
        cx.notify();
    }
    pub(super) fn select_mode(
        &mut self,
        id: TableSeedAttemptId,
        column: usize,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable
            || !self.choosing_mode
            || self.composing(window, cx)
            || self.column != Some(column)
            || !self
                .recipe
                .as_ref()
                .is_some_and(|recipe| recipe.attempt == id)
        {
            return;
        }
        if let Some(mode) = recipe::mode_at(index) {
            self.mode = mode;
            self.column_dirty = true;
            self.choosing_mode = false;
            window.focus(&self.buttons[4], cx);
            cx.notify();
        }
    }
    pub(super) fn select_job(
        &mut self,
        id: TableSeedAttemptId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composing(window, cx) || !self.ids(cx).contains(&id) {
            return;
        }
        self.selected = Some(id);
        self.armed = None;
        self.review_column = None;
        window.focus(&self.job_focus, cx);
        cx.notify();
    }
    pub(super) fn choose(
        &mut self,
        index: usize,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable
            || revision != self.connection_revision
            || !self.choosing
            || !self.connections_current
            || self.composing(window, cx)
        {
            return;
        }
        if let Some((id, _)) = self
            .connections
            .as_ref()
            .and_then(|choices| choices.rows.get(index))
        {
            let id = id.clone();
            self.bind_connection(id, cx);
            self.choosing = false;
            self.choice = None;
            window.focus(&self.buttons[0], cx);
            cx.notify();
        }
    }
}
