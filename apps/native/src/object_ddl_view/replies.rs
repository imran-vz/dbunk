use super::*;
impl ObjectDdlView {
    pub fn receive(&mut self, message: TableMessage, cx: &mut Context<Self>) {
        let mut invalidate = false;
        match message {
            TableMessage::TableDdlObserved(id, DdlObserved::Object(result))
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| matches!(p, Pending::Observe { .. }) && p.id() == id) =>
            {
                let Some(Pending::Observe {
                    operations,
                    cancelled,
                    ..
                }) = self.pending.take()
                else {
                    unreachable!()
                };
                if cancelled {
                    self.message = "Observation cancelled; late target discarded".into();
                } else {
                    self.message = match result {
                        Ok(target)
                            if target.retained_bytes() <= TOKEN_BYTES
                                && target.operations() == operations.as_slice() =>
                        {
                            let description = target.description().clone();
                            match self.next_id().and_then(|next| {
                                self.controls
                                    .as_ref()
                                    .ok_or("Objects document disconnected")?
                                    .send(TableCommand::ObjectDdlReview(next, target))?;
                                self.pending = Some(Pending::Review {
                                    id: next,
                                    operations,
                                    target: description,
                                    cancelled: false,
                                });
                                Ok(())
                            }) {
                                Ok(()) => {
                                    "Rendering exact SQL for the observed identities; no change sent"
                                        .into()
                                }
                                Err(error) => error.into(),
                            }
                        }
                        Ok(_) => {
                            "Observed target does not match the requested operation or exceeds bounds"
                                .into()
                        }
                        Err(error) => format!("Observation refused: {error}"),
                    };
                }
            }
            TableMessage::TableDdlReviewed(id, DdlReviewed::Object(result))
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| matches!(p, Pending::Review { .. }) && p.id() == id) =>
            {
                let Some(Pending::Review {
                    operations,
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
                                && review.operations() == operations.as_slice() =>
                        {
                            match self.recovery.stage(&review) {
                                Ok(()) => {
                                    self.message = if review.preview().confirmation_required {
                                        "Review the exact identities, impact and SQL below. Stored policy will require confirmation after Apply."
                                    } else {
                                        "Review the exact identities, impact and SQL below. Apply requires a durable save first."
                                    }
                                    .into();
                                    self.review = Some(review);
                                }
                                Err(error) => self.message = error.into(),
                            }
                        }
                        Ok(_) => self.message =
                            "Review identity or operation mismatch; no executable token retained"
                                .into(),
                        Err(error) => self.message = format!("Review refused: {error}"),
                    }
                }
            }
            TableMessage::TableDdlApplied(id, DdlApplied::Object(result))
                if self
                    .flow
                    .as_ref()
                    .is_some_and(|f| f.id() == id && f.dispatched()) =>
            {
                match result {
                    Ok(submission) => {
                        match *submission {
                            ObjectDdlSubmission::NeedsConfirmation(confirmation) => {
                                if confirmation.retained_bytes() <= TOKEN_BYTES
                                    && self.recovery.matches_review(confirmation.review())
                                {
                                    self.recovery.not_sent();
                                    self.flow
                                        .as_mut()
                                        .unwrap()
                                        .needs_confirmation(Token::Confirmation(confirmation));
                                    self.message = "Stored policy requires confirmation of this exact review. No change sent; confirmation requires its own save acknowledgement.".into();
                                } else {
                                    self.flow = None;
                                    invalidate = true;
                                    self.message = "Confirmation mismatch; recovery remains unknown. Reconcile explicitly.".into();
                                }
                            }
                            ObjectDdlSubmission::Finished(receipt) => {
                                self.flow = None;
                                self.receipt = render::receipt_text(&receipt);
                                match self.recovery.receipt(&receipt) {
                                    Settlement::Applied => {
                                        invalidate = true;
                                        self.message = "Change applied. Refresh Objects before another operation.".into();
                                    }
                                    Settlement::Partial => {
                                        invalidate = true;
                                        self.message = "Stopped after a committed prefix. Committed statements remain; the rest were not applied. Inspect before another change.".into();
                                    }
                                    Settlement::Staged => {
                                        // Known rollback can still leave server-hook external effects.
                                        invalidate = !matches!(
                                            receipt.outcome,
                                            ObjectDdlOutcome::NotDispatched { .. }
                                        );
                                        self.message = "No statement committed. Intent retained; another attempt requires fresh observation and review.".into();
                                    }
                                    Settlement::Unknown => {
                                        invalidate = true;
                                        self.message = "Outcome or receipt identity unknown. Recovery retained; inspect and reconcile explicitly.".into();
                                    }
                                }
                            }
                        }
                    }
                    Err(error) => {
                        self.flow = None;
                        self.recovery.submission_error(&error);
                        invalidate = *error == ObjectDdlError::OutcomeUnavailable;
                        self.message = format!("Object change: {error}. Recovery retained.");
                    }
                }
            }
            _ => return,
        }
        self.publish(cx);
        // The worker has returned and flow settled before parent invalidation.
        if invalidate {
            cx.emit(ObjectDdlEvent::DatabaseChanged(
                self.recovery.connection().to_owned(),
            ));
        }
    }
}
