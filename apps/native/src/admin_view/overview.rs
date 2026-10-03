//! Overview uses the Administration document's owned read lane and reply fences.
use super::*;
use crate::overview_view::{OverviewEvent, OverviewRuntime, OverviewView};
use dbunk_lib::backend::{
    data::DataError,
    overview::{OverviewSnapshot, RelationStatsRequest},
};

#[derive(Default)]
pub(super) struct Overview {
    pub view: Option<Entity<OverviewView>>,
    pub incoming: Option<Incoming>,
    pub current: bool,
    pub show: bool,
    events: Option<gpui::Subscription>,
}

// Inactive documents may not render immediately. Charge the reply from dequeue
// through view admission, including overlap with the retained previous page.
pub(super) struct Incoming {
    snapshot: Option<OverviewSnapshot>,
    budget: Rc<Cell<usize>>,
    retained: usize,
}
impl Incoming {
    fn new(snapshot: OverviewSnapshot, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        let retained = snapshot.checked_heap_bytes().ok_or(
            "Overview response is invalid or exceeds its bounds; previous capture retained",
        )?;
        if retained > (128 * 1024 * 1024usize).saturating_sub(budget.get()) {
            return Err("Overview needs shared memory; clear a capture and refresh");
        }
        budget.set(budget.get() + retained);
        Ok(Self {
            snapshot: Some(snapshot),
            budget,
            retained,
        })
    }
}
impl Drop for Incoming {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.retained));
    }
}

impl AdminView {
    pub(super) fn show_overview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overview.view.is_none() {
            let view = cx.new(|cx| OverviewView::new(self.budget.clone(), window, cx));
            self.overview.events =
                Some(
                    cx.subscribe_in(&view, window, |this, _, event, window, cx| {
                        match event {
                            OverviewEvent::Back => {
                                this.overview.show = false;
                                this.previous_focus = None;
                                window.focus(&this.buttons[21], cx);
                            }
                            OverviewEvent::Connect => this.begin_connect(cx),
                            OverviewEvent::Refresh(request) | OverviewEvent::Next(request) => {
                                this.load_overview(request.clone(), cx);
                            }
                            OverviewEvent::Cancel => this.activate(Action::Cancel, window, cx),
                        }
                        cx.notify();
                    }),
                );
            self.overview.view = Some(view);
        }
        self.overview.show = true;
        if let Some(view) = &self.overview.view {
            window.focus(&view.read(cx).focus(cx), cx);
        }
    }

    fn load_overview(&mut self, request: RelationStatsRequest, cx: &mut Context<Self>) {
        if !self.ready || !self.enabled(Action::Refresh) {
            return;
        }
        let id = match self.read.begin() {
            Ok(id) => id,
            Err(error) => {
                self.status = error.into();
                return;
            }
        };
        self.overview.current = false;
        self.overview.incoming = None;
        match self
            .controls
            .as_ref()
            .ok_or("Connect first")
            .and_then(|controls| controls.send(TableCommand::Overview(id, request)))
        {
            Ok(()) => {
                self.failure = None;
                self.status =
                    "Collecting overview statistics; previous capture may be stale".into();
            }
            Err(error) => {
                self.read.settle(id);
                self.status = error.into();
            }
        }
        cx.notify();
    }

    pub(super) fn settle_overview(
        &mut self,
        id: u64,
        result: Result<OverviewSnapshot, Arc<DataError>>,
        cx: &mut Context<Self>,
    ) {
        match self.read.settle(id) {
            Reply::Stale => return,
            Reply::Cancelled => {
                self.failure = None;
                self.status =
                    "Overview read cancelled; late reply discarded and previous capture retained"
                        .into();
            }
            Reply::Current => match result {
                Ok(snapshot) => match Incoming::new(snapshot, self.budget.clone()) {
                    Ok(incoming) => self.overview.incoming = Some(incoming),
                    Err(error) => self.status = error.into(),
                },
                Err(error) => {
                    let error = match error.as_ref() {
                        DataError::Catalog(error) => error.to_string(),
                        error => format!("{error:?}"),
                    };
                    self.status =
                        format!("Overview read refused: {error}; previous capture retained")
                }
            },
        }
        cx.notify();
    }

    pub(super) fn sync_overview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.overview.view else {
            return;
        };
        if let Some(mut incoming) = self.overview.incoming.take() {
            let snapshot = incoming.snapshot.take().expect("admitted overview reply");
            match view.update(cx, |view, cx| view.receive(snapshot, window, cx)) {
                Ok(()) => {
                    self.overview.current = true;
                    self.status = "Overview readings collected; row counts are estimates".into();
                }
                Err(error) => self.status = error.into(),
            }
        }
        view.update(cx, |view, cx| {
            view.set_runtime(
                OverviewRuntime {
                    ready: self.ready && !self.control.pending(),
                    busy: self.read.busy() || self.opening || self.audit.pending.is_some(),
                    can_cancel: self.enabled(Action::Cancel),
                    editable: self.editable,
                    capture_current: self.overview.current,
                    status: &self.status,
                },
                cx,
            )
        });
    }
}
