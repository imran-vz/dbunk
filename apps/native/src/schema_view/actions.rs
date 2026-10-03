use super::*;
use gpui::ClipboardItem;
impl SchemaView {
    pub(super) fn enabled(&self, action: Action) -> bool {
        match action {
            Action::Cancel => self.has_pending() || self.applying.is_some(),
            Action::Back => !self.has_pending(),
            Action::Copy => !self.preview.is_empty() || self.recovery.changes().is_some(),
            _ if !self.editable => false,
            Action::Review => {
                self.ready
                    && !self.has_pending()
                    && !self.recovery.unknown()
                    && self.review.is_none()
                    && self.applying.is_none()
            }
            Action::Apply => {
                self.ready
                    && !self.has_pending()
                    && !self.recovery.unknown()
                    && self.review.is_some()
            }
            Action::Confirm => {
                self.ready && self.applying.as_ref().is_some_and(ApplyFlow::confirming)
            }
            Action::Comment => {
                !self.has_pending()
                    && !self.recovery.unknown()
                    && self.review.is_none()
                    && self.applying.is_none()
            }
            Action::Edit | Action::Discard => {
                !self.has_pending()
                    && self.recovery.changes().is_some()
                    && (!matches!(action, Action::Edit) || !self.recovery.unknown())
            }
        }
    }
    pub(super) fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action) {
            return;
        }
        if self.composing(window, cx) {
            self.message = "Finish composing before changing the schema review".into();
            cx.notify();
            return;
        }
        match action {
            Action::Back => {
                cx.emit(SchemaEvent::Back);
                return;
            }
            Action::Comment => self.include_comment = !self.include_comment,
            Action::Copy => {
                let text = if self.preview.is_empty() {
                    self.recovery_text()
                } else {
                    self.preview.clone()
                };
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                self.message = if cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref()
                    == Some(text.as_str())
                {
                    "Review copied"
                } else {
                    "Clipboard write could not be verified"
                }
                .into();
            }
            Action::Review => {
                let intent = self.name.read(cx).value(cx).and_then(|name| {
                    let comment = if self.include_comment { Some(self.comment.read(cx).value(cx)?) } else { None };
                    CreateSchemaIntent::new(name, comment).map_err(|error| match error { dbunk_lib::backend::schema_ddl::CreateSchemaIntentError::Name => "Schema name must be nonblank, at most 63 UTF-8 bytes and contain no NUL", dbunk_lib::backend::schema_ddl::CreateSchemaIntentError::Comment => "Comment exceeds 4 KiB or contains NUL" })
                });
                match intent {
                    Ok(intent) => {
                        let id = self.next_id();
                        match self
                            .controls
                            .as_ref()
                            .ok_or("Connect Objects first")
                            .and_then(|controls| {
                                controls.send(TableCommand::SchemaReview(id, intent.clone()))
                            }) {
                            Ok(()) => {
                                self.reviewing = Some((id, intent, false));
                                self.message =
                                    "Generating bounded schema SQL; no write sent".into();
                            }
                            Err(error) => self.message = error.into(),
                        }
                    }
                    Err(error) => self.message = error.into(),
                }
            }
            Action::Apply => {
                if !self.recovery.dispatching() {
                    return;
                }
                let id = self.next_id();
                let review = self.review.take().unwrap();
                self.applying = Some(ApplyFlow::new(id, Token::Review(review)));
                self.message = "Saving exact recovery revision before Apply".into();
                self.publish(cx);
                cx.emit(SchemaEvent::PersistApply(id));
                return;
            }
            Action::Confirm => {
                let id = self.next_id();
                if !self.recovery.dispatching() {
                    return;
                }
                if !self.applying.as_mut().is_some_and(|flow| flow.confirm(id)) {
                    self.recovery.not_sent();
                    return;
                }
                self.message = "Saving confirmation recovery revision before Apply".into();
                self.publish(cx);
                cx.emit(SchemaEvent::PersistApply(id));
                return;
            }
            Action::Cancel => {
                if let Some(pending) = &mut self.reviewing {
                    pending.2 = true;
                    if let Some(controls) = &self.controls {
                        controls.cancel();
                    }
                    self.message = "Review cancellation requested".into();
                } else if self.applying.as_ref().is_some_and(ApplyFlow::dispatched) {
                    if let Some(controls) = &self.controls {
                        controls.cancel();
                    }
                    self.message = "Cancellation requested. A commit already admitted may still succeed; waiting for its receipt.".into();
                } else {
                    self.cancel_before_dispatch();
                    self.message = "Apply cancelled before dispatch; late save acknowledgements cannot send it".into();
                }
            }
            Action::Edit => {
                self.review = None;
                self.applying = None;
                self.preview.clear();
                self.recovery.not_sent();
                self.reconcile_armed = false;
                self.message =
                    "Saved intent retained. Field edits remain local until Review.".into();
            }
            Action::Discard if self.recovery.unknown() && !self.reconcile_armed => {
                self.reconcile_armed = true;
                self.message = "Inspect the database and reconcile this attempt first. Discard recovery only removes this local record; it neither retries nor undoes SQL.".into();
            }
            Action::Discard => {
                self.recovery.discard();
                self.review = None;
                self.applying = None;
                self.preview.clear();
                self.reconcile_armed = false;
                self.message = "Local schema draft discarded; no database action sent".into();
            }
        }
        self.publish(cx);
    }
    pub(super) fn recovery_text(&self) -> String {
        self.recovery
            .changes()
            .map(|changes| {
                format!(
                    "Attempt: {}\nSchema: {}\nComment: {}\nState: {:?}",
                    changes.attempt_id.as_str(),
                    changes.intent.name(),
                    changes
                        .intent
                        .comment()
                        .map_or("<not included>", |text| text),
                    changes.apply_state
                )
            })
            .unwrap_or_default()
    }
}
