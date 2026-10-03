//! App-owned job observation. Setup tabs borrow this state; closing them cannot
//! drop an admitted preparation, start acknowledgement, or restore invalidation.
use crate::{
    controller::{Host, ToolCommand, ToolControls, ToolDelivery, ToolReply},
    pg_tool_jobs::{Capture, ObservationOrder},
};
use dbunk_lib::backend::pg_tools::{
    PgToolAttemptId, PgToolConfirmation, PgToolIntent, PgToolReview, PgToolSubmission,
};
use gpui::{Context, EventEmitter, Task};
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

pub struct CaptureChanged;
pub struct ToolStore {
    controls: Option<ToolControls>,
    replies: Option<async_channel::Receiver<ToolDelivery>>,
    order: ObservationOrder,
    pending: Option<u64>,
    pending_list: bool,
    capture: Option<Capture>,
    observation_current: bool,
    observation_error: bool,
    review: Option<PgToolReview>,
    confirmation: Option<PgToolConfirmation>,
    budget: Rc<Cell<usize>>,
    message: Option<String>,
    _lease: StoreLease,
    _observe: Task<()>,
}
struct StoreLease(Option<Rc<Cell<usize>>>);
const STORE_BYTES: usize = 128 * 1024;
impl Drop for StoreLease {
    fn drop(&mut self) {
        if let Some(budget) = &self.0 {
            budget.set(budget.get().saturating_sub(STORE_BYTES));
        }
    }
}
impl EventEmitter<CaptureChanged> for ToolStore {}
impl ToolStore {
    pub fn new(host: Arc<Host>, budget: Rc<Cell<usize>>, cx: &mut Context<Self>) -> Self {
        let admitted = STORE_BYTES <= (128 * 1024 * 1024usize).saturating_sub(budget.get());
        let lease = StoreLease(admitted.then(|| {
            budget.set(budget.get() + STORE_BYTES);
            budget.clone()
        }));
        let (wake, awakened) = async_channel::bounded(1);
        let opened = if admitted {
            host.open_pg_tools(wake)
        } else {
            Err("File job observer needs 128 KiB of shared allowance")
        };
        let (controls, replies, message) = match opened {
            Ok((controls, replies)) => (Some(controls), Some(replies), None),
            Err(error) => (None, None, Some(error.into())),
        };
        let observe = cx.spawn(async move |this, cx| {
            let mut poll = true;
            loop {
                let delay = this.update(cx, |this, cx| {
                    let changed = this.drain(cx);
                    if poll || changed {
                        this.refresh(cx);
                    }
                    if this.capture.as_ref().is_some_and(Capture::has_active) {
                        1
                    } else {
                        15
                    }
                });
                let Ok(delay) = delay else {
                    break;
                };
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
            capture: None,
            observation_current: false,
            observation_error: false,
            review: None,
            confirmation: None,
            budget,
            message,
            _lease: lease,
            _observe: observe,
        }
    }
    pub fn observation_current(&self) -> bool {
        self.observation_current && self.controls.is_some()
    }
    pub fn capture(&self) -> Option<&Capture> {
        self.capture.as_ref()
    }
    pub fn review(&self) -> Option<&PgToolReview> {
        self.review.as_ref()
    }
    pub fn confirmation(&self) -> Option<&PgToolConfirmation> {
        self.confirmation.as_ref()
    }
    pub fn busy(&self) -> bool {
        (self.pending.is_some() && !self.pending_list) || self.controls.is_none()
    }
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
    fn send(&mut self, command: impl FnOnce(u64) -> ToolCommand, cx: &mut Context<Self>) -> bool {
        let result = (|| {
            if self.pending.is_some() {
                return Err("A file job request is pending");
            }
            let id = self.order.issue()?;
            self.controls
                .as_ref()
                .ok_or("File job observer unavailable")?;
            let command = command(id);
            let list = matches!(command, ToolCommand::List(_));
            self.controls.as_ref().unwrap().send(command)?;
            self.pending_list = list;
            if !list {
                self.observation_error = false;
            }
            self.pending = Some(id);
            Ok(())
        })();
        match result {
            Ok(()) => {
                if !self.pending_list {
                    cx.notify();
                }
                true
            }
            Err(error) => {
                self.message = Some(error.into());
                cx.notify();
                false
            }
        }
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_none() && self.controls.is_some() {
            self.send(ToolCommand::List, cx);
        }
    }
    pub fn begin(
        &mut self,
        connection: String,
        intent: PgToolIntent,
        cx: &mut Context<Self>,
    ) -> Option<PgToolAttemptId> {
        if !self.observation_current() {
            self.message = Some("Refresh job observations before preparing another job".into());
            cx.notify();
            return None;
        }
        if self.capture.as_ref().is_some_and(|capture| {
            capture.rows().iter().any(|job| {
                job.connection_id == connection
                    && job.kind == dbunk_lib::backend::pg_tools::PgToolKind::Restore
                    && job.effect == dbunk_lib::backend::pg_tools::PgToolEffect::Unknown
            })
        }) {
            self.message = Some("Inspect the target and explicitly reconcile its unknown restore before preparing another job".into());
            cx.notify();
            return None;
        }
        let attempt = PgToolAttemptId::new();
        self.send(|id| ToolCommand::Begin(id, attempt, connection, intent), cx)
            .then_some(attempt)
    }
    pub fn review_job(&mut self, attempt: PgToolAttemptId, cx: &mut Context<Self>) {
        if self.send(|id| ToolCommand::Review(id, attempt), cx) {
            self.review = None;
            self.confirmation = None;
        }
    }
    pub fn start(&mut self, attempt: PgToolAttemptId, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            self.message =
                Some("Job observation or command is pending; retry after it finishes".into());
            cx.notify();
            return;
        }
        if self.busy()
            || self
                .review
                .as_ref()
                .is_none_or(|review| review.attempt_id() != attempt)
        {
            return;
        }
        let review = self.review.take().unwrap();
        self.send(|id| ToolCommand::Start(id, review), cx);
    }
    pub fn confirm(&mut self, attempt: PgToolAttemptId, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            self.message =
                Some("Job observation or command is pending; retry after it finishes".into());
            cx.notify();
            return;
        }
        if self.busy()
            || self
                .confirmation
                .as_ref()
                .is_none_or(|confirmation| confirmation.review().attempt_id() != attempt)
        {
            return;
        }
        let confirmation = self.confirmation.take().unwrap();
        self.send(|id| ToolCommand::Confirm(id, confirmation), cx);
    }
    pub fn cancel(&mut self, attempt: PgToolAttemptId, cx: &mut Context<Self>) {
        self.send(|id| ToolCommand::Cancel(id, attempt), cx);
    }
    pub fn release(&mut self, attempt: PgToolAttemptId, cx: &mut Context<Self>) {
        self.send(|id| ToolCommand::Release(id, attempt), cx);
    }
    fn drain(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(replies) = &self.replies else {
            return false;
        };
        let delivery = match replies.try_recv() {
            Ok(delivery) => delivery,
            Err(async_channel::TryRecvError::Closed) => {
                self.controls = None;
                self.pending = None;
                self.message = Some("File job observer closed. A dispatched operation may still be owned by the backend; no automatic retry was made.".into());
                cx.notify();
                return false;
            }
            Err(async_channel::TryRecvError::Empty) => return false,
        };
        if self.pending != Some(delivery.request) || !self.order.accept(delivery.request) {
            return false;
        }
        self.pending = None;
        let refresh = !self.pending_list;
        // Consume the delivery only after retained admission. The worker does
        // not dispatch another command while this payload owns its queue lease.
        match delivery.result {
            Ok(ToolReply::List(data)) => {
                if self.observation_error {
                    self.observation_error = false;
                    self.message = None;
                    cx.notify();
                }
                if self
                    .capture
                    .as_ref()
                    .is_some_and(|capture| capture.matches(&data))
                {
                    if !self.observation_current {
                        self.observation_current = true;
                        cx.notify();
                    }
                    return false;
                }
                match Capture::new(data, self.budget.clone()) {
                    Ok(capture) => {
                        self.observation_current = true;
                        self.capture = Some(capture);
                        cx.emit(CaptureChanged);
                    }
                    Err(error) => {
                        self.observation_current = false;
                        self.observation_error = true;
                        self.message = Some(error.into());
                    }
                }
            }
            Ok(ToolReply::Review(review)) => {
                self.review = Some(review);
                self.message = None;
            }
            Ok(ToolReply::Submission(PgToolSubmission::NeedsConfirmation(confirmation))) => {
                self.confirmation = Some(*confirmation);
                self.message = None;
            }
            Ok(ToolReply::Submission(PgToolSubmission::Accepted(observation)))
            | Ok(ToolReply::Observation(observation)) => {
                self.message = Some(format!(
                    "{}: {}",
                    observation.attempt_id,
                    crate::pg_tool_jobs::phase_label(observation.phase)
                ));
            }
            Ok(ToolReply::Released(attempt)) => {
                if self
                    .review
                    .as_ref()
                    .is_some_and(|review| review.attempt_id() == attempt)
                {
                    self.review = None;
                }
                if self
                    .confirmation
                    .as_ref()
                    .is_some_and(|review| review.review().attempt_id() == attempt)
                {
                    self.confirmation = None;
                }
                // Commands are serialized behind a consumed full-list reply, so
                // no older list can reintroduce this released attempt.
                if let Err(error) = self.order.fence() {
                    self.message = Some(error.into());
                } else {
                    self.message = Some("Job released".into());
                }
                self.capture = None;
            }
            Err(error) => {
                self.observation_error = self.pending_list;
                if self.pending_list {
                    self.observation_current = false;
                }
                self.message = Some(format!(
                    "{error}. Refresh jobs and review the current attempt before trying again"
                ));
            }
        }
        cx.notify();
        refresh
    }
}
