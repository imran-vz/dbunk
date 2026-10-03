//! Overview uses the Administration document's owned read lane and reply fences.
use super::{audit::LocalRead, *};
use crate::{
    controller::{LibraryCommand, LibraryReply},
    overview_model::recent::{Recent, Refusal, Step},
    overview_view::{OverviewEvent, OverviewRuntime, OverviewView},
    query_library::Rows,
    query_library_view::OpenQuery,
};
use dbunk_lib::backend::{
    WorkspaceTool,
    data::DataError,
    overview::{OverviewSnapshot, RelationStatsRequest},
};

#[derive(Default)]
pub(super) struct Overview {
    pub view: Option<Entity<OverviewView>>,
    pub incoming: Option<Incoming>,
    pub current: bool,
    pub show: bool,
    /// Finished recent-queries list awaiting view admission; already charged.
    recent: Option<Recent>,
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
                            OverviewEvent::RefreshRecent => this.load_recent(cx),
                            OverviewEvent::OpenRecent { sql, connection } => {
                                this.open_recent(sql, connection, cx)
                            }
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

    pub(super) fn recent_enabled(&self) -> bool {
        self.editable
            && self.connection.is_some()
            && !self.read.busy()
            && !self.control.pending()
            && self.audit.pending.is_none()
            && !self.recent.loading()
    }

    /// Explicit refresh of this connection's recent history from the profile.
    /// It never needs or touches the PostgreSQL reader.
    fn load_recent(&mut self, cx: &mut Context<Self>) {
        if !self.recent_enabled() {
            return;
        }
        let Some(connection) = self.connection.clone() else {
            return;
        };
        if !self.open_local_lane(cx) {
            return;
        }
        match self.recent.begin(&connection, self.budget.clone()) {
            Ok((generation, request)) => self.send_recent(generation, request, cx),
            Err(error) => {
                self.status = error.into();
                cx.notify();
            }
        }
    }

    fn send_recent(
        &mut self,
        generation: u64,
        request: dbunk_lib::backend::query_library::LibraryRequest,
        cx: &mut Context<Self>,
    ) {
        let id = match self.read.begin() {
            Ok(id) => id,
            Err(error) => {
                self.recent.reset();
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        match self.send_local(LibraryCommand::Load(WorkspaceTool::History, request)) {
            Ok(()) => {
                self.audit.pending = Some(id);
                self.audit.purpose = LocalRead::Recent(generation);
                self.status = "Reading recent queries for this connection from this profile".into();
            }
            Err(error) => {
                self.read.settle(id);
                self.recent.reset();
                self.status = format!("Recent queries not read: {error}; previous list retained");
            }
        }
        cx.notify();
    }

    pub(super) fn recent_lane_closed(&mut self, id: u64, generation: u64, cx: &mut Context<Self>) {
        if self.recent.generation() == Some(generation) {
            self.recent.reset();
        }
        if self.read.settle(id) != Reply::Stale {
            self.status =
                "Local history worker closed; previous recent list retained. Refresh to retry"
                    .into();
        }
        cx.notify();
    }

    pub(super) fn settle_recent(
        &mut self,
        id: u64,
        generation: u64,
        result: Result<LibraryReply, String>,
        cx: &mut Context<Self>,
    ) {
        let reply = self.read.settle(id);
        if reply != Reply::Current {
            // Stale (disconnect fence or older request) or cancelled: discard
            // and end this generation; the previous list stays.
            if self.recent.generation() == Some(generation) {
                self.recent.reset();
                if reply == Reply::Cancelled {
                    self.status = "Recent queries read discarded; previous list retained".into();
                }
            }
            cx.notify();
            return;
        }
        let page = match result {
            Ok(LibraryReply::Page(Rows::History(page))) => page,
            Ok(_) => {
                self.recent.reset();
                self.status = "Unexpected reply while reading recent queries".into();
                cx.notify();
                return;
            }
            Err(error) => {
                self.recent.reset();
                self.status =
                    format!("Recent queries read failed: {error}; previous list retained");
                cx.notify();
                return;
            }
        };
        match self
            .recent
            .receive(generation, self.connection.as_deref(), page)
        {
            Ok(Step::Continue(request)) => self.send_recent(generation, request, cx),
            Ok(Step::Done(recent)) => {
                self.status = recent.summary();
                self.overview.recent = Some(recent);
            }
            Err(Refusal::Stale) => {}
            Err(Refusal::Invalid(error)) => self.status = error.into(),
        }
        cx.notify();
    }

    fn open_recent(&mut self, sql: &str, connection: &str, cx: &mut Context<Self>) {
        if !self.editable || self.connection.as_deref() != Some(connection) {
            self.status = "Recent query belongs to another connection; refresh the list".into();
            return;
        }
        cx.emit(AdminEvent::OpenQuery(OpenQuery {
            sql: sql.to_owned(),
            name: "Recent query".into(),
            connection: Some(connection.to_owned()),
            saved_id: None,
        }));
        self.status = "Opened recent SQL as a disconnected draft; it was not executed".into();
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
        let state = if self.ready {
            "Connected"
        } else if self.opening {
            "Connecting"
        } else {
            "Disconnected"
        };
        let header = match (&self.connection, &self.identity) {
            (None, _) => "No connection bound".to_owned(),
            (Some(_), Some(identity)) => format!("{identity} · {state}"),
            (Some(id), None) => format!("Connection {id} · {state}"),
        };
        let header = match &self.health {
            Some(health) if self.connection.is_some() => format!("{header} · {health}"),
            _ => header,
        };
        let recent_enabled = self.recent_enabled();
        view.update(cx, |view, cx| {
            view.set_runtime(
                OverviewRuntime {
                    ready: self.ready && !self.control.pending(),
                    busy: self.read.busy() || self.opening || self.audit.pending.is_some(),
                    can_cancel: self.enabled(Action::Cancel),
                    editable: self.editable,
                    capture_current: self.overview.current,
                    status: &self.status,
                    header: &header,
                    connection: self.connection.as_deref(),
                    recent_enabled,
                    recent_loading: self.recent.loading(),
                },
                cx,
            )
        });
        // After the runtime carries the current connection binding.
        if let Some(recent) = self.overview.recent.take() {
            view.update(cx, |view, cx| view.receive_recent(recent, cx));
        }
    }
}
