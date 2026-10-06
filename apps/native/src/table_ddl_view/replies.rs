use super::*;
impl TableDdlView {
    pub fn receive(&mut self, message: TableMessage, cx: &mut Context<Self>) {
        let mut invalidate = false;
        match message {
            TableMessage::TableDdlObserved(id, DdlObserved::Table(result))
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| matches!(p, Pending::Observe { .. }) && p.id() == id) =>
            {
                let Some(Pending::Observe {
                    intent, cancelled, ..
                }) = self.pending.take()
                else {
                    unreachable!()
                };
                if cancelled {
                    self.message = "Observation cancelled; late target discarded".into();
                } else {
                    match result {
                        Ok(target)
                            if target.retained_bytes() <= TOKEN_BYTES
                                && self
                                    .selection
                                    .as_ref()
                                    .is_some_and(|s| s.matches(target.description())) =>
                        {
                            let description = target.description().clone();
                            match self.next_id().and_then(|next| {
                                self.controls
                                    .as_ref()
                                    .ok_or("Objects document disconnected")?
                                    .send(TableCommand::TableDdlReview(
                                        next,
                                        target,
                                        intent.clone(),
                                    ))?;
                                self.pending = Some(Pending::Review {
                                    id: next,
                                    intent,
                                    target: description,
                                    cancelled: false,
                                });
                                Ok(())
                            }) {
                                Ok(()) => {
                                    self.message = "Rendering exact SQL for the observed target; no change sent"
                                        .into()
                                }
                                Err(error) => self.fail(error),
                            }
                        }
                        Ok(_) => self.fail(
                            "Observed target does not match the selected identity or exceeds bounds",
                        ),
                        Err(error) => self.fail(format!("Observation refused: {error}")),
                    }
                }
            }
            TableMessage::TableDdlReviewed(id, DdlReviewed::Table(result))
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| matches!(p, Pending::Review { .. }) && p.id() == id) =>
            {
                let Some(Pending::Review {
                    intent,
                    target,
                    cancelled,
                    ..
                }) = self.pending.take()
                else {
                    unreachable!()
                };
                if cancelled {
                    self.message = "Review cancelled; late authority discarded".into();
                } else {
                    match result {
                        Ok(review)
                            if review.retained_bytes() <= TOKEN_BYTES
                                && review.target() == &target
                                && review.intent() == &intent =>
                        {
                            match self.recovery.stage(&review) {
                                Ok(()) => {
                                    self.review = Some(review);
                                    self.message="Review the exact target, SQL and deadline below. Apply requires a durable save first.".into();
                                }
                                Err(error) => self.fail(error),
                            }
                        }
                        Ok(_) => self.fail(
                            "Review identity or intent mismatch; no executable token retained",
                        ),
                        Err(error) => self.fail(format!("Review refused: {error}")),
                    }
                }
            }
            TableMessage::TableDdlApplied(id, DdlApplied::Table(result))
                if self
                    .flow
                    .as_ref()
                    .is_some_and(|f| f.id() == id && f.dispatched()) =>
            {
                match result {
                    Ok(submission) => match *submission {
                        TableDdlSubmission::NeedsConfirmation(confirmation) => {
                            if confirmation.retained_bytes() <= TOKEN_BYTES
                                && self.recovery.matches_review(confirmation.review())
                            {
                                self.recovery.not_sent();
                                self.flow
                                    .as_mut()
                                    .unwrap()
                                    .needs_confirmation(Token::Confirmation(confirmation));
                                self.message="Stored policy requires confirmation of this exact review. No change sent; confirmation requires its own save acknowledgement.".into();
                            } else {
                                self.flow = None;
                                invalidate = true;
                                self.fail("Confirmation mismatch; recovery remains unknown. Reconcile explicitly.");
                            }
                        }
                        TableDdlSubmission::Finished(receipt) => {
                            self.flow = None;
                            match self.recovery.receipt(&receipt) {
                                Settlement::Applied => {
                                    invalidate = true;
                                    self.selection = None;
                                    self.receipt = format!(
                                        "Applied transactional table change. Attempt {}. {:?}",
                                        receipt.attempt_id.as_str(),
                                        receipt.outcome
                                    );
                                    self.message="Change applied. Refresh Structure before another operation.".into();
                                }
                                Settlement::Staged => {
                                    // Known rollback can still leave server-hook external effects.
                                    invalidate = matches!(
                                        receipt.outcome,
                                        TableDdlOutcome::RolledBack { .. }
                                    );
                                    self.receipt = format!(
                                        "Attempt {}: {:?}",
                                        receipt.attempt_id.as_str(),
                                        receipt.outcome
                                    );
                                    self.message="No transactional table change remains. Intent retained; another attempt requires fresh observation and review.".into();
                                }
                                Settlement::Unknown => {
                                    invalidate = true;
                                    self.fail("Outcome or receipt identity unknown. Recovery retained; inspect and reconcile explicitly.");
                                }
                            }
                        }
                    },
                    Err(error) => {
                        self.flow = None;
                        self.recovery.submission_error(&error);
                        invalidate = *error == TableDdlError::OutcomeUnavailable;
                        self.fail(format!("Table change: {error}. Recovery retained."));
                    }
                }
            }
            _ => return,
        }
        self.publish(cx);
        // The worker has returned and flow settled before parent invalidation.
        if invalidate {
            cx.emit(TableDdlEvent::DatabaseChanged(
                self.recovery.connection().to_owned(),
            ));
        }
    }
}
