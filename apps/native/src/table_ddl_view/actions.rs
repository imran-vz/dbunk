use super::*;
impl TableDdlView {
    pub(super) fn enabled(&self, action: Action) -> bool {
        match action {
            Action::Back => !self.has_pending(),
            Action::Cancel => self.pending.is_some() || self.flow.is_some(),
            _ if !self.editable => false,
            Action::Operation | Action::Remove => {
                self.editable_recipe() && (!matches!(action, Action::Remove) || !self.rename)
            }
            Action::Review => self.ready && self.editable_recipe() && self.selection.is_some(),
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
            self.fail("Finish text composition before changing this review");
            cx.notify();
            return;
        }
        let result = self.act(action, cx);
        if let Err(error) = result {
            self.fail(error);
        }
        self.publish(cx);
    }
    fn act(&mut self, action: Action, cx: &mut Context<Self>) -> Result<(), &'static str> {
        match action {
            Action::Back => cx.emit(TableDdlEvent::Back),
            Action::Operation => {
                self.rename = !self.rename;
                self.armed = false;
            }
            Action::Remove => self.remove_comment = !self.remove_comment,
            Action::Edit => {
                self.review = None;
                self.message="Edit locally, then observe and review again. The previous staged journal remains until a new review is accepted.".into();
            }
            Action::Review => {
                let value = self
                    .value
                    .as_ref()
                    .ok_or("Value editor unavailable")?
                    .read(cx)
                    .value(cx)?;
                let intent = model::intent(self.rename, self.remove_comment, value)?;
                let id = self.next_id()?;
                let selection = self
                    .selection
                    .as_ref()
                    .ok_or("Choose a target from Structure")?;
                self.controls.as_ref().ok_or("Connect first")?.send(
                    TableCommand::TableDdlObserve(id, selection.request(), selection.attnum()),
                )?;
                self.pending = Some(Pending::Observe {
                    id,
                    intent,
                    cancelled: false,
                });
                self.armed = false;
                self.receipt.clear();
                self.message = "Observing the exact selected target; no change sent".into();
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
                cx.emit(TableDdlEvent::PersistApply(id));
            }
            Action::Cancel => {
                if self.not_sent() {
                    self.message="Cancelled before dispatch; late save acknowledgements cannot release this change".into();
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
                    self.message="Inspect and reconcile the database first. Activate Discard reconciled recovery to remove this local record; it neither retries nor undoes SQL.".into();
                } else if self.recovery.discard(self.armed) {
                    self.review = None;
                    self.selection = None;
                    self.armed = false;
                    self.receipt.clear();
                    self.message =
                        "Recovery discarded. Return to Structure and select a fresh target.".into();
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
            self.fail(format!("Table change not sent: {error}"));
        } else if !self.recovery.unknown() {
            self.not_sent();
            self.fail("Exact uncertain recovery was not preserved; nothing dispatched");
        } else if let Some(token) = self.flow.as_mut().and_then(|f| f.saved(id)) {
            let exact = match &token {
                Token::Review(review) => self.recovery.matches_review(review),
                Token::Confirmation(confirmation) => {
                    self.recovery.matches_review(confirmation.review())
                }
            };
            if !exact {
                self.flow = None;
                self.fail("Saved intent no longer matches authority; no dispatch, recovery remains unknown");
                self.publish(cx);
                return;
            }
            let command = match token {
                Token::Review(review) => TableCommand::TableDdlApply(id, review),
                Token::Confirmation(token) => TableCommand::TableDdlConfirm(id, token),
            };
            match self
                .controls
                .as_ref()
                .ok_or("Objects document disconnected")
                .and_then(|c| c.send(command))
            {
                Ok(()) => {
                    self.message =
                        "Exact recovery saved; table change dispatched. Waiting for its receipt."
                            .into()
                }
                Err(error) => {
                    self.flow = None;
                    self.recovery.not_sent();
                    self.fail(format!("Table change not sent: {error}"));
                }
            }
        }
        self.publish(cx);
    }
}
