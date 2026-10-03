use dbunk_lib::backend::table_seed::TableSeedAttemptId;

/// An acknowledgement is executable only while its exact request remains live.
#[derive(Default)]
pub(super) struct SaveFence {
    serial: u64,
    pub(super) pending: Option<(u64, TableSeedAttemptId)>,
}
impl SaveFence {
    pub(super) fn begin(&mut self, id: TableSeedAttemptId) -> Option<u64> {
        if self.pending.is_some() {
            return None;
        }
        self.serial = self.serial.checked_add(1)?;
        self.pending = Some((self.serial, id));
        Some(self.serial)
    }
    pub(super) fn cancel(&mut self, id: TableSeedAttemptId) {
        if self.pending.is_some_and(|(_, pending)| pending == id) {
            self.pending = None;
        }
    }
    pub(super) fn accept(&mut self, serial: u64) -> Option<TableSeedAttemptId> {
        if self.pending.is_some_and(|(pending, _)| pending == serial) {
            self.pending.take().map(|(_, id)| id)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelled_save_ack_cannot_dispatch_or_consume_a_new_request() {
        let first = TableSeedAttemptId::new();
        let second = TableSeedAttemptId::new();
        let mut fence = SaveFence::default();
        let old = fence.begin(first).unwrap();
        fence.cancel(first);
        let new = fence.begin(second).unwrap();
        assert_eq!(fence.accept(old), None);
        assert_eq!(fence.accept(new), Some(second));
        assert_eq!(fence.accept(new), None);
    }

    #[test]
    fn unrelated_cancellation_and_overlapping_save_cannot_replace_authority() {
        let id = TableSeedAttemptId::new();
        let other = TableSeedAttemptId::new();
        let mut fence = SaveFence::default();
        let request = fence.begin(id).unwrap();
        assert!(fence.begin(other).is_none());
        fence.cancel(other);
        assert_eq!(fence.accept(request + 1), None);
        assert_eq!(fence.accept(request), Some(id));
        assert!(fence.begin(other).unwrap() > request);
    }

    #[test]
    fn revision_exhaustion_never_reuses_an_old_save_identity() {
        let mut fence = SaveFence {
            serial: u64::MAX,
            pending: None,
        };
        assert!(fence.begin(TableSeedAttemptId::new()).is_none());
        assert!(fence.pending.is_none());
    }
}
