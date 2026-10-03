//! A saved draft may release a write once. Cancellation before dispatch revokes
//! that release even when a late SQLite acknowledgement arrives afterward.
pub(crate) struct ApplyFlow<T> {
    id: u64,
    phase: Phase<T>,
}
enum Phase<T> {
    Saving(T),
    Dispatched,
    Confirmation(T),
    Cancelled,
}
impl<T> ApplyFlow<T> {
    pub fn new(id: u64, token: T) -> Self {
        Self {
            id,
            phase: Phase::Saving(token),
        }
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn waiting_for(&self, id: u64) -> bool {
        self.id == id && matches!(self.phase, Phase::Saving(_))
    }
    pub fn dispatched(&self) -> bool {
        matches!(self.phase, Phase::Dispatched)
    }
    pub fn confirming(&self) -> bool {
        matches!(self.phase, Phase::Confirmation(_))
    }
    pub fn token(&self) -> Option<&T> {
        match &self.phase {
            Phase::Saving(token) | Phase::Confirmation(token) => Some(token),
            _ => None,
        }
    }
    pub fn saved(&mut self, id: u64) -> Option<T> {
        if !self.waiting_for(id) {
            return None;
        }
        let Phase::Saving(token) = std::mem::replace(&mut self.phase, Phase::Dispatched) else {
            unreachable!()
        };
        Some(token)
    }
    pub fn needs_confirmation(&mut self, token: T) {
        self.phase = Phase::Confirmation(token);
    }
    pub fn confirm(&mut self, id: u64) -> bool {
        if !self.confirming() {
            return false;
        }
        let Phase::Confirmation(token) = std::mem::replace(&mut self.phase, Phase::Cancelled)
        else {
            unreachable!()
        };
        self.id = id;
        self.phase = Phase::Saving(token);
        true
    }
    pub fn cancel_before_dispatch(&mut self) -> bool {
        if self.dispatched() {
            return false;
        }
        self.phase = Phase::Cancelled;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelled_save_cannot_release_a_late_or_duplicate_write() {
        let mut flow = ApplyFlow::new(7, "reviewed plan");
        assert_eq!(flow.saved(6), None);
        assert!(flow.cancel_before_dispatch());
        assert_eq!(flow.saved(7), None);
        assert!(flow.token().is_none());
        let mut flow = ApplyFlow::new(8, "next plan");
        assert_eq!(flow.saved(8), Some("next plan"));
        assert_eq!(flow.saved(8), None);
        assert!(!flow.cancel_before_dispatch()); // only the backend can settle it
    }
    #[test]
    fn confirmation_has_a_new_durable_fence_and_is_cancellable() {
        let mut flow = ApplyFlow::new(2, "review");
        assert_eq!(flow.saved(2), Some("review"));
        flow.needs_confirmation("one-use confirmation");
        assert!(flow.confirm(3));
        assert!(!flow.confirm(4));
        assert_eq!(flow.saved(2), None);
        assert!(flow.cancel_before_dispatch());
        assert_eq!(flow.saved(3), None);
    }
}
