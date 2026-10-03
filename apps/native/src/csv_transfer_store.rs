//! Shared CSV observations and explicit commands outlive setup tabs. Abandoned
//! inspection owners are reconciled after preparation; accepted jobs never use
//! the setup tab as their lifecycle owner.
use crate::{
    controller::{CsvCommand, CsvControls, CsvDelivery, CsvReply, Host},
    csv_transfer_model::{Capture, InspectionCapture, Lease, ObservationOrder},
};
use dbunk_lib::backend::csv_transfers::*;
use gpui::{Context, EventEmitter, Task};
use std::{cell::Cell, collections::HashMap, path::PathBuf, rc::Rc, sync::Arc, time::Duration};
mod workbook;

pub struct CaptureChanged;
pub struct InspectionOwner {
    id: uuid::Uuid,
    alive: Rc<Cell<bool>>,
}
impl InspectionOwner {
    pub fn new(id: uuid::Uuid) -> Self {
        Self {
            id,
            alive: Rc::new(Cell::new(true)),
        }
    }
}
impl Drop for InspectionOwner {
    fn drop(&mut self) {
        self.alive.set(false);
    }
}
struct OwnedInspection {
    owner: uuid::Uuid,
    alive: Rc<Cell<bool>>,
    abandoned: bool,
}
impl OwnedInspection {
    fn abandoned(&self) -> bool {
        self.abandoned || !self.alive.get()
    }
}
struct StoreLease(Rc<Cell<usize>>);
const STORE_BYTES: usize = 128 * 1024;
impl Drop for StoreLease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(STORE_BYTES));
    }
}

pub struct CsvStore {
    controls: Option<CsvControls>,
    replies: Option<async_channel::Receiver<CsvDelivery>>,
    order: ObservationOrder,
    pending: Option<u64>,
    pending_list: bool,
    pending_cleanup: bool,
    capture: Option<Capture>,
    current: bool,
    observation_error: bool,
    inspection: Option<InspectionCapture>,
    workbook: Option<(CsvWorkbook, Lease)>,
    review: Option<CsvTransferReview>,
    confirmation: Option<CsvTransferConfirmation>,
    token_lease: Option<Lease>,
    owned: HashMap<CsvInspectionId, OwnedInspection>,
    budget: Rc<Cell<usize>>,
    message: Option<String>,
    _lease: Option<StoreLease>,
    _observe: Task<()>,
}
impl EventEmitter<CaptureChanged> for CsvStore {}
impl CsvStore {
    pub fn new(host: Arc<Host>, budget: Rc<Cell<usize>>, cx: &mut Context<Self>) -> Self {
        let admitted = STORE_BYTES <= (128 * 1024 * 1024usize).saturating_sub(budget.get());
        let lease = admitted.then(|| {
            budget.set(budget.get() + STORE_BYTES);
            StoreLease(budget.clone())
        });
        let (wake, awakened) = async_channel::bounded(1);
        let opened = if admitted {
            host.open_csv_transfers(wake)
        } else {
            Err("CSV observer needs 128 KiB of shared allowance")
        };
        let (controls, replies, message) = match opened {
            Ok((a, b)) => (Some(a), Some(b), None),
            Err(error) => (None, None, Some(error.into())),
        };
        let observe = cx.spawn(async move |this, cx| {
            let mut poll = true;
            loop {
                let delay = this.update(cx, |this, cx| {
                    let changed = this.drain(cx);
                    if (poll || changed) && !this.cleanup_abandoned(cx) {
                        this.refresh(cx);
                    }
                    if !this.owned.is_empty()
                        || this.capture.as_ref().is_some_and(Capture::has_active)
                    {
                        1
                    } else {
                        15
                    }
                });
                let Ok(delay) = delay else { break };
                let timer = cx.background_executor().timer(Duration::from_secs(delay));
                futures_util::pin_mut!(timer);
                let wake = awakened.recv();
                futures_util::pin_mut!(wake);
                poll = match futures_util::future::select(wake, timer).await {
                    futures_util::future::Either::Left((Ok(()), _)) => false,
                    futures_util::future::Either::Left((Err(_), _)) => break,
                    futures_util::future::Either::Right(_) => true,
                };
            }
        });
        Self {
            controls,
            replies,
            order: Default::default(),
            pending: None,
            pending_list: false,
            pending_cleanup: false,
            capture: None,
            current: false,
            observation_error: false,
            inspection: None,
            workbook: None,
            review: None,
            confirmation: None,
            token_lease: None,
            owned: HashMap::new(),
            budget,
            message,
            _lease: lease,
            _observe: observe,
        }
    }
    pub fn observation_current(&self) -> bool {
        self.current && self.controls.is_some()
    }
    pub fn capture(&self) -> Option<&Capture> {
        self.capture.as_ref()
    }
    pub fn inspection(&self) -> Option<&CsvInspectionData> {
        self.inspection.as_ref().map(InspectionCapture::data)
    }
    pub fn review(&self) -> Option<&CsvTransferReview> {
        self.review.as_ref()
    }
    pub fn confirmation(&self) -> Option<&CsvTransferConfirmation> {
        self.confirmation.as_ref()
    }
    pub fn busy(&self) -> bool {
        (self.pending.is_some() && !self.pending_list) || self.controls.is_none()
    }
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
    fn fail(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.message = Some(message.into());
        cx.notify();
    }
    fn send(&mut self, make: impl FnOnce(u64) -> CsvCommand, cx: &mut Context<Self>) -> bool {
        let result = (|| {
            if self.pending.is_some() {
                return Err("A CSV observation or command is pending; retry after it finishes");
            }
            let id = self.order.issue()?;
            let controls = self.controls.as_ref().ok_or("CSV observer unavailable")?;
            let command = make(id);
            let list = matches!(command, CsvCommand::List(_));
            controls.send(command)?;
            self.pending = Some(id);
            self.pending_list = list;
            self.pending_cleanup = false;
            if !list {
                self.current = false;
                self.observation_error = false;
                cx.notify();
            }
            Ok(())
        })();
        match result {
            Ok(()) => true,
            Err(error) => {
                self.fail(error, cx);
                false
            }
        }
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_none() && self.controls.is_some() {
            self.send(CsvCommand::List, cx);
        }
    }
    pub fn abandon_inspection(&mut self, owner: &InspectionOwner, cx: &mut Context<Self>) {
        for item in self
            .owned
            .values_mut()
            .filter(|item| item.owner == owner.id)
        {
            item.abandoned = true;
        }
        self.clear_abandoned_payloads();
        cx.notify();
    }
    fn clear_abandoned_payloads(&mut self) {
        if self.workbook.as_ref().is_some_and(|(workbook, _)| {
            self.owned
                .get(&workbook.data().inspection_id)
                .is_none_or(OwnedInspection::abandoned)
        }) {
            self.workbook = None;
        }
        if self.inspection.as_ref().is_some_and(|capture| {
            self.owned
                .get(&capture.data().inspection_id)
                .is_none_or(OwnedInspection::abandoned)
        }) {
            self.inspection = None;
        }
        if self.review.as_ref().is_some_and(|review| {
            self.owned
                .get(&review.inspection_id())
                .is_some_and(OwnedInspection::abandoned)
        }) {
            self.review = None;
            self.token_lease = None;
        }
    }
    fn cleanup_abandoned(&mut self, cx: &mut Context<Self>) -> bool {
        self.clear_abandoned_payloads();
        if self.pending.is_some() || !self.observation_current() {
            return false;
        }
        let Some(capture) = &self.capture else {
            return false;
        };
        let Some(action) = next_cleanup(
            self.owned
                .iter()
                .map(|(id, owner)| (*id, owner.abandoned())),
            capture.inspections(),
        ) else {
            return false;
        };
        let (id, cancel) = match action {
            CleanupAction::Forget(id) => {
                self.owned.remove(&id);
                return false;
            }
            CleanupAction::Cancel(id) => (id, true),
            CleanupAction::Release(id) => (id, false),
        };
        let sent = if cancel {
            self.send(|request| CsvCommand::CancelInspection(request, id), cx)
        } else {
            self.send(|request| CsvCommand::ReleaseInspection(request, id), cx)
        };
        self.pending_cleanup = sent;
        sent
    }
    pub fn begin_inspection(
        &mut self,
        owner: &InspectionOwner,
        connection: String,
        intent: CsvInspectionIntent,
        cx: &mut Context<Self>,
    ) -> Option<CsvInspectionId> {
        if !self.observation_current() {
            self.fail("Refresh CSV observations before inspecting", cx);
            return None;
        }
        if self.owned.len() > MAX_CSV_INSPECTIONS {
            self.fail("Unused CSV inspections are still being released", cx);
            return None;
        }
        if self.capture.as_ref().is_some_and(|capture| {
            capture.jobs().iter().any(|job| {
                job.connection_id == connection
                    && job.direction == CsvDirection::Import
                    && job.effect == CsvEffect::Unknown
            })
        }) {
            self.fail("Inspect the target and explicitly reconcile its unknown import before inspecting again",cx);
            return None;
        }
        self.abandon_inspection(owner, cx);
        let id = CsvInspectionId::new();
        if !self.send(
            |request| CsvCommand::Inspect(request, id, connection, intent),
            cx,
        ) {
            return None;
        }
        self.owned.insert(
            id,
            OwnedInspection {
                owner: owner.id,
                alive: owner.alive.clone(),
                abandoned: false,
            },
        );
        Some(id)
    }
    pub fn load_inspection(&mut self, id: CsvInspectionId, cx: &mut Context<Self>) {
        if self.owned.get(&id).is_none_or(OwnedInspection::abandoned) {
            self.fail("This CSV setup no longer owns the inspection", cx);
            return;
        }
        self.send(|request| CsvCommand::LoadInspection(request, id), cx);
    }
    pub fn review_import(
        &mut self,
        id: CsvInspectionId,
        mapping: Vec<CsvMapping>,
        cx: &mut Context<Self>,
    ) {
        if !self.can_take_inspection(id, cx) {
            return;
        }
        let inspection = self.inspection.take().unwrap().take();
        if self.send(
            |request| CsvCommand::ReviewImport(request, inspection, mapping),
            cx,
        ) {
            self.clear_tokens();
        }
    }
    pub fn review_export(&mut self, id: CsvInspectionId, path: PathBuf, cx: &mut Context<Self>) {
        if !self.can_take_inspection(id, cx) {
            return;
        }
        let inspection = self.inspection.take().unwrap().take();
        if self.send(
            |request| CsvCommand::ReviewExport(request, inspection, path),
            cx,
        ) {
            self.clear_tokens();
        }
    }
    fn can_take_inspection(&mut self, id: CsvInspectionId, cx: &mut Context<Self>) -> bool {
        if self.pending.is_some() {
            self.fail(
                "CSV observation or command is pending; retry after it finishes",
                cx,
            );
            return false;
        }
        if self
            .inspection()
            .is_none_or(|data| data.inspection_id != id)
            || self.owned.get(&id).is_none_or(OwnedInspection::abandoned)
        {
            self.fail("Load this setup's current inspection before reviewing", cx);
            return false;
        }
        true
    }
    /// Editing an unsubmitted mapping invalidates only its review authority;
    /// the inspection can be reviewed again. Accepted attempts stay app-owned.
    pub fn discard_setup_review(&mut self, id: CsvInspectionId, cx: &mut Context<Self>) {
        if self
            .review
            .as_ref()
            .is_some_and(|review| review.inspection_id() == id && review.attempt_id().is_none())
        {
            self.review = None;
            self.token_lease = None;
            cx.notify();
        }
    }
    fn clear_tokens(&mut self) {
        self.review = None;
        self.confirmation = None;
        self.token_lease = None;
    }
    pub fn begin_transfer(
        &mut self,
        expected_inspection: CsvInspectionId,
        expected_attempt: Option<CsvTransferAttemptId>,
        cx: &mut Context<Self>,
    ) -> Option<CsvTransferAttemptId> {
        if self.pending.is_some() {
            self.fail(
                "CSV observation or command is pending; retry after it finishes",
                cx,
            );
            return None;
        }
        if !self.observation_current() {
            self.fail(
                "Refresh CSV observations before starting the reviewed transfer",
                cx,
            );
            return None;
        }
        if self.review.as_ref().is_some_and(|review| {
            self.capture.as_ref().is_some_and(|capture| {
                capture.unknown_import_on(&review.inspection().data().connection_id)
            })
        }) {
            self.fail(
                "Inspect the target and reconcile its unknown import before another transfer",
                cx,
            );
            return None;
        }
        if self.review.as_ref().is_none_or(|review| {
            review.inspection_id() != expected_inspection || review.attempt_id() != expected_attempt
        }) {
            self.fail(
                "CSV review changed; review the selected inspection or attempt again",
                cx,
            );
            return None;
        }
        let review = self.review.take().unwrap();
        // Reacquired reviews preserve their original attempt. They can only
        // reissue confirmation; the backend refuses duplicate dispatch.
        let id = review.attempt_id().unwrap_or_default();
        let sent = self.send(|request| CsvCommand::Begin(request, id, review), cx);
        self.token_lease = None;
        if sent { Some(id) } else { None }
    }
    pub fn review_transfer(&mut self, id: CsvTransferAttemptId, cx: &mut Context<Self>) {
        if self.send(|request| CsvCommand::ReviewTransfer(request, id), cx) {
            self.clear_tokens();
        }
    }
    pub fn confirm(&mut self, id: CsvTransferAttemptId, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            self.fail(
                "CSV observation or command is pending; retry after it finishes",
                cx,
            );
            return;
        }
        if self
            .confirmation
            .as_ref()
            .is_none_or(|confirmation| confirmation.attempt_id() != id)
        {
            return;
        }
        let confirmation = self.confirmation.take().unwrap();
        self.send(|request| CsvCommand::Confirm(request, confirmation), cx);
        self.token_lease = None;
    }
    pub fn cancel(&mut self, id: CsvTransferAttemptId, cx: &mut Context<Self>) {
        self.send(|request| CsvCommand::Cancel(request, id), cx);
    }
    pub fn release(&mut self, id: CsvTransferAttemptId, cx: &mut Context<Self>) {
        self.send(|request| CsvCommand::Release(request, id), cx);
    }
    fn drain(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(replies) = &self.replies else {
            return false;
        };
        let delivery = match replies.try_recv() {
            Ok(delivery) => delivery,
            Err(async_channel::TryRecvError::Empty) => return false,
            Err(async_channel::TryRecvError::Closed) => {
                self.controls = None;
                self.pending = None;
                self.current = false;
                self.fail("CSV observer closed. Dispatched work may still be owned by the backend; no automatic retry was made",cx);
                return false;
            }
        };
        if self.pending != Some(delivery.request) || !self.order.accept(delivery.request) {
            return false;
        }
        self.pending = None;
        let refresh = !self.pending_list;
        match delivery.result {
            Ok(CsvReply::List(inspections, jobs)) => {
                // Only an observed app-owned attempt detaches setup inspection
                // cleanup. A pre-dispatch queue refusal leaves its owner intact.
                self.owned.retain(|id, _| {
                    inspections
                        .inspections
                        .iter()
                        .any(|row| row.inspection_id == *id)
                });
                for job in &jobs.jobs {
                    self.owned.remove(&job.inspection_id);
                }
                if self.observation_error {
                    self.observation_error = false;
                    self.message = None;
                    cx.notify();
                }
                if self
                    .capture
                    .as_ref()
                    .is_some_and(|capture| capture.matches(&inspections, &jobs))
                {
                    if !self.current {
                        self.current = true;
                        cx.notify();
                    }
                    return false;
                }
                match Capture::new(inspections, jobs, self.budget.clone()) {
                    Ok(capture) => {
                        self.capture = Some(capture);
                        self.current = true;
                        cx.emit(CaptureChanged);
                    }
                    Err(error) => {
                        self.current = false;
                        self.observation_error = true;
                        self.message = Some(error.into());
                    }
                }
            }
            Ok(CsvReply::Inspection(inspection)) => {
                let id = inspection.data().inspection_id;
                if self.owned.get(&id).is_some_and(|owner| !owner.abandoned()) {
                    match InspectionCapture::new(inspection, self.budget.clone()) {
                        Ok(capture) => {
                            self.inspection = Some(capture);
                            self.message = None;
                        }
                        Err(error) => self.message = Some(error.into()),
                    }
                }
            }
            Ok(CsvReply::Workbook(workbook)) => self.accept_workbook(workbook),
            Ok(CsvReply::InspectionObservation(row)) => {
                if !self.pending_cleanup {
                    self.message = Some(format!(
                        "Inspection {} request acknowledged. Refresh observations for its current status.",
                        row.inspection_id
                    ));
                }
            }
            Ok(CsvReply::InspectionReleased(id)) => {
                self.owned.remove(&id);
                if self
                    .workbook()
                    .is_some_and(|workbook| workbook.data().inspection_id == id)
                {
                    self.workbook = None;
                }
                if self
                    .inspection()
                    .is_some_and(|data| data.inspection_id == id)
                {
                    self.inspection = None;
                }
                self.current = false;
                self.capture = None;
            }
            Ok(CsvReply::Review(review)) => match Lease::inspection(self.budget.clone()) {
                Ok(lease) => {
                    self.clear_tokens();
                    self.token_lease = Some(lease);
                    self.review = Some(review);
                    self.message = None;
                }
                Err(error) => self.message = Some(error.into()),
            },
            Ok(CsvReply::Submission(CsvTransferSubmission::NeedsConfirmation(confirmation))) => {
                match Lease::inspection(self.budget.clone()) {
                    Ok(lease) => {
                        self.clear_tokens();
                        self.token_lease = Some(lease);
                        self.confirmation = Some(*confirmation);
                        self.message = None;
                    }
                    Err(error) => self.message = Some(error.into()),
                }
            }
            Ok(CsvReply::Submission(CsvTransferSubmission::Accepted(row)))
            | Ok(CsvReply::Observation(row)) => {
                self.message = Some(format!(
                    "Transfer {} request acknowledged. Refresh observations for current outcome and cleanup.",
                    row.attempt_id
                ));
            }
            Ok(CsvReply::Released(id)) => {
                if self
                    .review
                    .as_ref()
                    .is_some_and(|review| review.attempt_id() == Some(id))
                    || self
                        .confirmation
                        .as_ref()
                        .is_some_and(|confirmation| confirmation.attempt_id() == id)
                {
                    self.clear_tokens();
                }
                self.current = false;
                self.capture = None;
                self.message = Some("CSV transfer released".into());
                if let Err(error) = self.order.fence() {
                    self.message = Some(error.into());
                }
            }
            Err(error) => {
                if self.pending_list {
                    self.current = false;
                    self.observation_error = true;
                }
                self.message = Some(format!(
                    "{error}. Refresh CSV observations before trying again"
                ));
            }
        }
        cx.notify();
        refresh
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CleanupAction {
    Forget(CsvInspectionId),
    Cancel(CsvInspectionId),
    Release(CsvInspectionId),
}
fn next_cleanup(
    owned: impl IntoIterator<Item = (CsvInspectionId, bool)>,
    observations: &[CsvInspectionObservation],
) -> Option<CleanupAction> {
    owned.into_iter().find_map(|(id, abandoned)| {
        if !abandoned {
            return None;
        }
        match observations.iter().find(|row| row.inspection_id == id) {
            None => Some(CleanupAction::Forget(id)),
            Some(row) if row.phase == CsvInspectionPhase::Preparing => {
                Some(CleanupAction::Cancel(id))
            }
            Some(row)
                if row.phase != CsvInspectionPhase::Cancelling
                    && row.cleanup == CsvCleanup::Complete =>
            {
                Some(CleanupAction::Release(id))
            }
            _ => None,
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn observation(
        id: CsvInspectionId,
        phase: CsvInspectionPhase,
        cleanup: CsvCleanup,
    ) -> CsvInspectionObservation {
        CsvInspectionObservation {
            inspection_id: id,
            connection_id: "fixture".into(),
            target: CsvTarget {
                schema: "public".into(),
                table: "owned".into(),
            },
            direction: CsvDirection::Import,
            phase,
            cleanup,
            expires_at: None,
            failure: None,
            diagnostic: None,
        }
    }
    #[test]
    fn abandoned_blocked_cleanup_cannot_starve_another_ready_inspection() {
        let blocked = CsvInspectionId::new();
        let ready = CsvInspectionId::new();
        let rows = [
            observation(blocked, CsvInspectionPhase::Cancelling, CsvCleanup::Pending),
            observation(ready, CsvInspectionPhase::Ready, CsvCleanup::Complete),
        ];
        assert_eq!(
            next_cleanup([(blocked, true), (ready, true)], &rows),
            Some(CleanupAction::Release(ready))
        );
        let mut rows = rows;
        rows[0].phase = CsvInspectionPhase::Failed;
        rows[0].cleanup = CsvCleanup::Failed;
        assert_eq!(
            next_cleanup([(blocked, true), (ready, true)], &rows),
            Some(CleanupAction::Release(ready))
        );
        assert_eq!(next_cleanup([(blocked, true), (ready, false)], &rows), None);
    }
    #[test]
    fn dropped_setup_owner_cancels_preparation_then_releases_only_joined_inspection() {
        let owner = InspectionOwner::new(uuid::Uuid::new_v4());
        let owned = OwnedInspection {
            owner: owner.id,
            alive: owner.alive.clone(),
            abandoned: false,
        };
        let id = CsvInspectionId::new();
        let mut rows = [observation(
            id,
            CsvInspectionPhase::Preparing,
            CsvCleanup::Pending,
        )];
        assert_eq!(next_cleanup([(id, owned.abandoned())], &rows), None);
        drop(owner);
        assert_eq!(
            next_cleanup([(id, owned.abandoned())], &rows),
            Some(CleanupAction::Cancel(id))
        );
        rows[0].phase = CsvInspectionPhase::Cancelled;
        assert_eq!(next_cleanup([(id, owned.abandoned())], &rows), None);
        rows[0].cleanup = CsvCleanup::Complete;
        assert_eq!(
            next_cleanup([(id, owned.abandoned())], &rows),
            Some(CleanupAction::Release(id))
        );
    }
}
