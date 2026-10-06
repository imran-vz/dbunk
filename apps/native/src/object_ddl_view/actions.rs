use super::*;
impl ObjectDdlView {
    pub(super) fn enabled(&self, action: Action) -> bool {
        match action {
            Action::Back => !self.has_pending(),
            Action::Cancel => self.pending.is_some() || self.flow.is_some(),
            _ if !self.editable => false,
            Action::Mode => self.editable_recipe() && self.purpose.is_some() && !self.uses_form(),
            Action::Option => self.editable_recipe() && self.creating(),
            Action::Review => self.ready && self.editable_recipe() && self.purpose.is_some(),
            Action::Edit => {
                !self.has_pending()
                    && !self.recovery.unknown()
                    && self.flow.is_none()
                    && self.review.is_some()
            }
            Action::Apply => {
                self.ready
                    && self.review.is_some()
                    && self.flow.is_none()
                    && !self.recovery.unknown()
            }
            Action::Confirm => self.ready && self.flow.as_ref().is_some_and(ApplyFlow::confirming),
            Action::Discard => {
                !self.has_pending() && self.flow.is_none() && self.recovery.journal().is_some()
            }
        }
    }
    pub(super) fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action) {
            return;
        }
        if self.composing(window, cx) {
            self.message = "Finish text composition before changing this review".into();
            cx.notify();
            return;
        }
        let result = self.act(action, cx);
        if let Err(error) = result {
            self.message = error.into();
        }
        self.publish(cx);
    }
    fn field_text(
        field: Option<&Entity<Field>>,
        cx: &Context<Self>,
    ) -> Result<String, &'static str> {
        field.map_or(Ok(String::new()), |field| field.read(cx).value(cx))
    }
    fn act(&mut self, action: Action, cx: &mut Context<Self>) -> Result<(), &'static str> {
        match action {
            Action::Back => cx.emit(ObjectDdlEvent::Back),
            Action::Mode => {
                if self.creating() {
                    self.draft.materialized = !self.draft.materialized;
                } else {
                    self.draft.cascade = !self.draft.cascade;
                }
                self.armed = false;
            }
            Action::Option => {
                if self.draft.materialized {
                    self.draft.with_data = !self.draft.with_data;
                } else {
                    self.draft.or_replace = !self.draft.or_replace;
                }
            }
            Action::Edit => {
                self.review = None;
                self.message = "Edit locally, then observe and review again. The previous staged journal remains until a new review is accepted.".into();
            }
            Action::Review => {
                let purpose = self.purpose.as_ref().ok_or("Select an Objects row first")?;
                let name = Self::field_text(self.name.as_ref(), cx)?;
                let body = Self::field_text(self.body.as_ref(), cx)?;
                let operations = self.draft.operations(purpose, name, body)?;
                let id = self.next_id()?;
                self.controls.as_ref().ok_or("Connect first")?.send(
                    TableCommand::ObjectDdlObserve(
                        id,
                        ObjectDdlRequest {
                            operations: operations.clone(),
                        },
                    ),
                )?;
                self.pending = Some(Pending::Observe {
                    id,
                    operations,
                    cancelled: false,
                });
                self.armed = false;
                self.receipt.clear();
                self.message = match self.purpose {
                    Some(Purpose::Drop(_)) => {
                        "Observing the exact identity and its drop impact; no change sent"
                    }
                    Some(Purpose::CreateIndex { .. }) => {
                        "Observing the table and confirming the index name is free; no change sent"
                    }
                    Some(Purpose::AddEnumValue { .. }) => {
                        "Observing the exact enum type; no change sent"
                    }
                    _ => "Observing the target schema and name; no change sent",
                }
                .into();
            }
            Action::Apply | Action::Confirm => {
                let id = self.next_id()?;
                if matches!(action, Action::Apply) {
                    if !self
                        .review
                        .as_ref()
                        .is_some_and(|r| self.recovery.matches_review(r))
                    {
                        return Err("Review no longer matches the durable intent");
                    }
                    self.flow = Some(ApplyFlow::new(
                        id,
                        Token::Review(self.review.take().unwrap()),
                    ));
                } else if !self.flow.as_mut().is_some_and(|f| f.confirm(id)) {
                    return Err("Confirmation no longer available");
                }
                if !self.recovery.mark_unknown() {
                    self.flow = None;
                    return Err("Saveable intent missing; nothing dispatched");
                }
                self.message =
                    "Saving the exact uncertain recovery revision before dispatch".into();
                self.publish(cx);
                cx.emit(ObjectDdlEvent::PersistApply(id));
            }
            Action::Cancel => {
                if self.not_sent() {
                    self.message = "Cancelled before dispatch; late save acknowledgements cannot release this change".into();
                } else {
                    if let Some(pending) = &mut self.pending {
                        pending.cancel();
                    }
                    if let Some(controls) = &self.controls {
                        controls.cancel();
                    }
                    self.message =
                        "Cancellation requested. Waiting for owned cleanup and the actual outcome."
                            .into();
                }
            }
            Action::Discard => {
                if self.recovery.unknown() && !self.armed {
                    self.armed = true;
                    self.message = "Inspect and reconcile the database first. Activate Discard reconciled recovery to remove this local record; it neither retries nor undoes SQL.".into();
                } else if self.recovery.discard(self.armed) {
                    self.review = None;
                    self.armed = false;
                    self.receipt.clear();
                    self.message =
                        "Recovery discarded. Select a fresh Objects row for another change.".into();
                }
            }
        }
        Ok(())
    }
    pub fn apply_saved(&mut self, id: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        if self.flow.as_ref().is_none_or(|f| !f.waiting_for(id)) {
            return;
        }
        if let Err(error) = result {
            self.not_sent();
            self.message = format!("Object change not sent: {error}");
        } else if !self.recovery.unknown() {
            self.not_sent();
            self.message = "Exact uncertain recovery was not preserved; nothing dispatched".into();
        } else if let Some(token) = self.flow.as_mut().and_then(|f| f.saved(id)) {
            let exact = match &token {
                Token::Review(review) => self.recovery.matches_review(review),
                Token::Confirmation(confirmation) => {
                    self.recovery.matches_review(confirmation.review())
                }
            };
            if !exact {
                self.flow = None;
                self.message = "Saved intent no longer matches authority; no dispatch, recovery remains unknown".into();
                self.publish(cx);
                return;
            }
            let command = match token {
                Token::Review(review) => TableCommand::ObjectDdlApply(id, review),
                Token::Confirmation(token) => TableCommand::ObjectDdlConfirm(id, token),
            };
            match self
                .controls
                .as_ref()
                .ok_or("Objects document disconnected")
                .and_then(|c| c.send(command))
            {
                Ok(()) => {
                    self.message =
                        "Exact recovery saved; object change dispatched. Waiting for its receipt."
                            .into()
                }
                Err(error) => {
                    self.flow = None;
                    self.recovery.not_sent();
                    self.message = format!("Object change not sent: {error}");
                }
            }
        }
        self.publish(cx);
    }
}
