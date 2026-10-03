//! Profile-local audit pages use the joined SQLite lane, never a PostgreSQL
//! connection. Cancellation discards publication after the owned read settles.
use super::*;
use crate::{
    controller::{LibraryCommand, LibraryControls, LibraryDelivery, LibraryReply},
    safety_audit_model::Capture,
};

#[derive(Default)]
pub(super) struct Audit {
    controls: Option<LibraryControls>,
    receiver: Option<async_channel::Receiver<LibraryDelivery>>,
    pub pending: Option<u64>,
    pub capture: Option<Capture>,
    pub selected: Option<i64>,
    pub stale: bool,
}
impl Audit {
    pub fn has_pending(&self) -> bool {
        self.receiver.as_ref().is_some_and(|receiver| {
            !receiver.is_empty() || receiver.is_closed() && self.pending.is_some()
        })
    }
    pub fn clear(&mut self) {
        self.capture = None;
        self.selected = None;
        self.stale = false;
    }
}
impl Drop for Audit {
    fn drop(&mut self) {
        if let Some(controls) = &self.controls {
            controls.stop();
        }
    }
}
impl AdminView {
    pub(super) fn load_audit(&mut self, next: bool, cx: &mut Context<Self>) {
        let action = if next {
            Action::AuditNext
        } else {
            Action::Refresh
        };
        if self.section != Section::Audit || !self.enabled(action) {
            return;
        }
        let Some(connection) = self.connection.clone() else {
            return;
        };
        if self
            .audit
            .controls
            .as_ref()
            .is_some_and(LibraryControls::is_closed)
        {
            self.audit.controls = None;
            self.audit.receiver = None;
        }
        if self.audit.controls.is_none() {
            match self.host.open_library(self.id.clone(), self.wake.clone()) {
                Ok((controls, receiver)) => {
                    self.audit.controls = Some(controls);
                    self.audit.receiver = Some(receiver);
                }
                Err(error) => {
                    self.status = error.into();
                    cx.notify();
                    return;
                }
            }
        }
        let cursor = if next {
            self.audit.capture.as_ref().and_then(Capture::next_cursor)
        } else {
            None
        };
        let id = match self.read.begin() {
            Ok(id) => id,
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        match self
            .audit
            .controls
            .as_ref()
            .expect("local worker admitted")
            .send(LibraryCommand::SafetyAudit(id, connection, cursor))
        {
            Ok(()) => {
                self.audit.pending = Some(id);
                self.audit.stale = self.audit.capture.is_some();
                self.status = "Reading retained safety overrides from this profile".into();
            }
            Err(error) => {
                self.read.settle(id);
                self.status = error.into();
            }
        }
        cx.notify();
    }
    pub(super) fn drain_audit(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(receiver) = &self.audit.receiver else {
            return false;
        };
        let delivery = match receiver.try_recv() {
            Ok(delivery) => delivery,
            Err(async_channel::TryRecvError::Closed) if self.audit.pending.is_some() => {
                let id = self.audit.pending.take().expect("pending local read");
                cx.notify();
                if self.read.settle(id) != Reply::Stale {
                    self.status = "Local audit worker closed; previous capture retained. Reopen this tab to retry".into();
                    cx.notify();
                }
                return true;
            }
            Err(_) => return false,
        };
        let Some(id) = self.audit.pending.take() else {
            return true;
        };
        // Even a stale/cancelled read releases the pending control barrier.
        cx.notify();
        let result = match delivery.result {
            Ok(LibraryReply::SafetyAudit(reply_id, result)) if reply_id == id => result,
            Ok(_) => Err("Unexpected reply in local safety audit".into()),
            Err(error) => Err(error),
        };
        match self.read.settle(id) {
            Reply::Stale => return true,
            Reply::Cancelled => {
                self.status = "Local audit read discarded; previous capture retained".into()
            }
            Reply::Current => match result {
                Ok(page) => match Capture::new(
                    page,
                    self.connection.as_deref().unwrap_or_default(),
                    self.budget.clone(),
                ) {
                    Ok(capture) => {
                        // Keep identity, not the row index, across refresh/pages.
                        self.audit.selected = self
                            .audit
                            .selected
                            .filter(|id| capture.index_for_key(*id).is_some());
                        self.audit.capture = Some(capture);
                        self.audit.stale = false;
                        self.status =
                            "Retained successful safety overrides loaded from this profile".into();
                        if self.section == Section::Audit {
                            self.scroll.scroll_to_item(
                                self.selected_index().unwrap_or(0),
                                gpui::ScrollStrategy::Top,
                            );
                            self.detail_scroll.set_offset(gpui::point(px(0.), px(0.)));
                        }
                    }
                    Err(error) => self.status = error.into(),
                },
                Err(error) => {
                    self.status =
                        format!("Local audit read failed: {error}; previous capture retained")
                }
            },
        }
        cx.notify();
        true
    }
}
