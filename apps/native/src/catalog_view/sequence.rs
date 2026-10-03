//! Sequence inspect/change tool opened from a selected sequence row. It holds
//! no durable recovery; unknown outcomes live only in its in-session view.
use super::*;
use crate::sequence_view::{Lease, SequenceEvent, SequenceView};
use dbunk_lib::backend::objects::PgObjectKind;

impl CatalogView {
    pub(super) fn selected_sequence(&self) -> Option<dbunk_lib::backend::objects::PgObjectRef> {
        self.visible
            .get(self.selected)
            .and_then(|i| self.catalog.as_ref()?.rows.get(*i))
            .and_then(|row| row.reference())
            .filter(|reference| reference.kind == PgObjectKind::Sequence)
    }
    pub(super) fn has_sequence_changes(&self, cx: &gpui::App) -> bool {
        self.sequence
            .as_ref()
            .is_some_and(|view| view.read(cx).has_changes())
    }
    pub(super) fn open_sequence(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked_by_other_lane(super::object_ddl::Lane::Sequence, cx) {
            self.status =
                "Finish or reconcile the current Objects operation before a sequence change".into();
            return;
        }
        let selected = self.selected_sequence();
        // Keep a view that owes a reply or discloses an unknown outcome. A new
        // selection otherwise starts a fresh tool with no prior authority.
        let keep = self.sequence.as_ref().is_some_and(|view| {
            let view = view.read(cx);
            view.has_changes() || selected.is_none() || view.reference() == selected.as_ref()
        });
        if !keep {
            self.sequence = None;
            self.sequence_events = None;
            let Some(reference) = selected else {
                self.status = "Select a sequence".into();
                return;
            };
            let Some(lease) = Lease::admit(self.budget.clone()) else {
                self.status = "Sequence tool needs 256 KiB of shared allowance. Clear another capture and retry.".into();
                return;
            };
            let next = self.apply_ids.clone();
            let view = cx.new(|cx| SequenceView::new(lease, next, reference, window, cx));
            self.sequence_events =
                Some(
                    cx.subscribe_in(&view, window, |this, _, event, window, cx| {
                        match event {
                            SequenceEvent::Changed => cx.emit(CatalogEvent::Changed),
                            SequenceEvent::Activity(busy) => this.sequence_busy = *busy,
                            SequenceEvent::Back => {
                                this.show_sequence = false;
                                window.focus(&this.list, cx);
                            }
                        }
                        cx.notify();
                    }),
                );
            self.sequence = Some(view);
        }
        let Some(view) = self.sequence.clone() else {
            self.status = "Select a sequence".into();
            return;
        };
        self.show_lane(super::object_ddl::Lane::Sequence);
        window.focus(&view.focus_handle(cx), cx);
    }
}
