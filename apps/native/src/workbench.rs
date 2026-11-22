//! One editor and result model, with three interchangeable pane arrangements.
use crate::{
    accessible_editor::AccessibleEditor,
    controller::{Command, Controls, Host, transaction_allowed},
    diagnostics::{self, ExecutedSql},
    grid::ResultGrid,
    mailbox::{self, Message},
    results::TerminalStatus,
    sql,
};
use dbunk_lib::backend::{
    self, AckPayload, ConnectionPooling, ExecutePayload, ExecutionPayload, Layout, QueryEvent,
    QuerySessionError, QueryTransactionIsolation, QueryTransactionMode, QueryTransactionSnapshot,
    QueryTransactionStatus, TransactionControl,
};
use editor::{Editor, EditorEvent};
use gpui::accesskit::{Action, Live};
use gpui::{
    Context, Entity, FocusHandle, Focusable, KeyDownEvent, Role, SharedString, Subscription, Task,
    Window, actions, div, prelude::*, px,
};
use language::Buffer;
use multi_buffer::MultiBufferOffset;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[path = "query_parameters.rs"]
mod query_parameters;
use query_parameters::{ParametersEvent, QueryParameters};
mod completion;
mod formatting;
mod query_changes;

actions!(
    native,
    [
        RunStatement,
        RunScript,
        InsertTopRows,
        InsertGroupedCount,
        InsertRecentRows,
        RefreshCompletionMetadata,
        FormatSql,
        FindInSql,
        FindNext,
        FindPrevious,
        StopQuery,
        Reconnect,
        SwitchPane,
        FocusToolbar,
        LeaveToolbar,
        NextControl,
        PreviousControl,
        CycleLayout,
        Quit
    ]
);

const INITIAL_SQL: &str = "SELECT id, label, padded, bucket, note\nFROM plan024.fixture_many\nWHERE id <= 100\nORDER BY id;\n";

#[derive(Clone, Copy, Debug)]
enum Control {
    Run,
    Script,
    Stop,
    Reconnect,
    Layout(Layout),
    Expand,
    Result(usize),
    Notices,
    ReturnToSql,
    Transaction(TransactionControl),
    ConfirmQuery,
    CancelReview,
    Bindings,
    Explain(bool),
    Plan,
    EditCell,
}
/// The two transaction selects at the right of the query toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransactionMenu {
    Mode,
    Isolation,
}
impl TransactionMenu {
    fn index(self) -> usize {
        match self {
            Self::Mode => 0,
            Self::Isolation => 1,
        }
    }
    /// The trigger's `toolbar_focus` slot (the old first radio of its group).
    fn focus_index(self) -> usize {
        match self {
            Self::Mode => 10,
            Self::Isolation => 12,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Mode => "Transaction mode",
            Self::Isolation => "Isolation level",
        }
    }
    fn owns(self, control: TransactionControl) -> bool {
        matches!(
            (self, control),
            (Self::Mode, TransactionControl::Mode(_))
                | (Self::Isolation, TransactionControl::Isolation(_))
        )
    }
}

/// Commit, Rollback and Recheck belong to manual transactions; they also
/// stay reachable whenever the session state needs attention (active,
/// failed or unknown), so recovery never hides behind a mode switch.
fn transaction_actions_visible(
    connected: bool,
    snapshot: Option<&QueryTransactionSnapshot>,
) -> bool {
    match snapshot {
        Some(snapshot) => {
            snapshot.mode == QueryTransactionMode::Manual
                || snapshot.status != QueryTransactionStatus::Idle
        }
        None => connected,
    }
}

/// Short visible transaction state beside the selects. `None` when the
/// selects already say everything (an idle, direct session).
fn transaction_note(
    connected: bool,
    pending: bool,
    snapshot: Option<&QueryTransactionSnapshot>,
) -> Option<String> {
    if !connected {
        return None;
    }
    if pending {
        return Some("Updating…".into());
    }
    let Some(snapshot) = snapshot else {
        return Some("State unknown".into());
    };
    let status = match snapshot.status {
        QueryTransactionStatus::Idle => None,
        QueryTransactionStatus::Active => Some("Active"),
        QueryTransactionStatus::Failed => Some("Failed; roll back"),
        QueryTransactionStatus::Unknown => Some("Unknown; recheck"),
    };
    let pooler = match snapshot.pooling {
        ConnectionPooling::TransactionPooler => Some("Transaction pooler"),
        ConnectionPooling::Pooler => Some("Connection pooler"),
        ConnectionPooling::Direct | ConnectionPooling::SessionPooler => None,
    };
    match (status, pooler) {
        (Some(status), Some(pooler)) => Some(format!("{status} · {pooler}")),
        (Some(text), None) | (None, Some(text)) => Some(text.into()),
        (None, None) => None,
    }
}

/// The `?` toolbar icon: keyboard hints as a tooltip, also its AX label.
fn keyboard_hint(text: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id("query-keyboard-hints")
        .role(Role::Image)
        .aria_label(text)
        .flex_none()
        .size(px(crate::style::CONTROL))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(crate::style::R_SM))
        .hover(|s| s.bg(crate::style::hover()))
        .child(
            gpui::svg()
                .path("icons/circle_help.svg")
                .size(px(crate::style::ICON))
                .text_color(crate::style::dim()),
        )
        .tooltip(crate::ui::tooltip(text))
        .tooltip_show_delay(crate::ui::tooltip_delay())
}

const KEYBOARD_HINTS: &str =
    "F8 controls · Tab next · Enter activate · Esc return · F6 SQL / results";

struct Execution {
    id: String,
    started: Instant,
    terminal: Option<u64>,
    source: Option<ExecutedSql>,
    review_revision: u64,
}
struct QueryReview {
    sql: String,
    classification: String,
    bindings: String,
}

struct QueryFailure {
    announcement_id: String,
    message: String,
    code: Option<String>,
    location: Option<String>,
}
impl QueryFailure {
    fn announcement(&self) -> String {
        format!(
            "Query failed. {}{}",
            self.code
                .as_ref()
                .map_or(String::new(), |code| format!("PostgreSQL {code}. ")),
            self.message
        )
    }
}

pub enum WorkbenchEvent {
    DraftChanged,
    PersistApply(u64),
    LayoutChanged(Layout),
    Quit,
    OpenQuery(crate::query_library_view::OpenQuery),
    Console(crate::console_model::Entry),
}
impl gpui::EventEmitter<WorkbenchEvent> for Workbench {}

struct DocumentBinding {
    id: String,
    connection_id: Option<String>,
    wake: async_channel::Sender<()>,
}

pub struct Workbench {
    completion: Option<crate::sql_completion::CompletionHandle>,
    completion_inflight: Option<u64>,
    completion_reset: bool,
    completion_status: String,
    completion_composing: bool,
    query_changes: query_changes::QueryChanges,
    /// Write policy of the bound connection, applied to every result-change
    /// view so review asks for the same confirmation a table tab would.
    query_policy: crate::data_model::TablePolicy,
    query_changes_return_focus: bool,
    document: Option<DocumentBinding>,
    receiver: Option<mailbox::Receiver>,
    retained_budget: Option<std::rc::Rc<std::cell::Cell<usize>>>,
    retained_bytes: usize,
    host: Arc<Host>,
    editor: Entity<Editor>,
    buffer: Entity<Buffer>,
    failure: Option<QueryFailure>,
    accessible: Entity<AccessibleEditor>,
    find: Option<Entity<crate::sql_find::FindView>>,
    _find_events: Option<gpui::Subscription>,
    grid: Entity<ResultGrid>,
    plan: Option<Entity<crate::explain_view::ExplainView>>,
    show_plan: bool,
    plan_return_focus: bool,
    plan_events: Option<Subscription>,
    layout: Layout,
    expanded: bool,
    owner: String,
    session: String,
    stream: crate::stream::Stream,
    controls: Option<Controls>,
    events: Option<Task<()>>,
    execution: Option<Execution>,
    review: Option<QueryReview>,
    parameters: Entity<QueryParameters>,
    show_parameters: bool,
    parameters_return_focus: bool,
    _parameters_events: Subscription,
    editor_revision: u64,
    transaction: Option<QueryTransactionSnapshot>,
    transaction_pending: bool,
    transaction_error: Option<String>,
    /// The open transaction select and its keyboard row.
    transaction_menu: Option<(TransactionMenu, crate::ui::popover::MenuNav)>,
    transaction_anchors: [crate::ui::popover::AnchorSlot; 2],
    transaction_menu_focus: FocusHandle,
    connected: bool,
    connecting: bool,
    /// Why the last session failed; cleared by a new attempt or a normal close.
    connect_error: Option<String>,
    closing: bool,
    status: String,
    toolbar_focus: Vec<FocusHandle>,
    previous_focus: Option<FocusHandle>,
    result_focus: Vec<FocusHandle>,
    show_notices: bool,
    _activation: Subscription,
    _editor_events: Subscription,
}
impl Workbench {
    pub fn new(
        host: Arc<Host>,
        layout: Layout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(host, layout, INITIAL_SQL, window, cx)
    }

    fn build(
        host: Arc<Host>,
        layout: Layout,
        initial_sql: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let language = sql::language(cx).expect("SQL grammar");
        let buffer = cx.new(|cx| {
            let mut buffer = Buffer::local(initial_sql, cx);
            buffer.set_language(Some(language), cx);
            buffer
        });
        let editor = cx.new(|cx| Editor::for_buffer(buffer.clone(), None, window, cx));
        diagnostics::install(&editor, cx);
        let editor_events = cx.subscribe(&editor, |this, _, event, cx| {
            if matches!(
                event,
                EditorEvent::BufferEdited | EditorEvent::SelectionsChanged { local: true }
            ) {
                this.invalidate_review(cx);
                if let Some(completion) = &this.completion {
                    completion.editor_changed();
                }
            }
            if matches!(event, EditorEvent::BufferEdited) {
                // Even editing and undoing invalidates an in-flight source position.
                if let Some(execution) = &mut this.execution {
                    execution.source = None;
                }
                if let Some(failure) = &mut this.failure {
                    failure.location = None;
                }
                diagnostics::clear(&this.editor, &this.buffer, cx);
                cx.emit(WorkbenchEvent::DraftChanged);
                cx.notify();
            }
            // Caret-only moves never rebuild the workspace snapshot. Every
            // snapshot reads the live selection, so the caret is persisted
            // with the next text save and by the immediate close/quit save.
        });
        let accessible = cx.new(|cx| AccessibleEditor::new(editor.clone(), "SQL editor", cx));
        let parameters = cx.new(|cx| QueryParameters::new(window, cx));
        let parameters_events = cx.subscribe(&parameters, |this, _, event, cx| {
            match event {
                ParametersEvent::Changed => this.invalidate_review(cx),
                ParametersEvent::Close => {
                    this.show_parameters = false;
                    this.parameters_return_focus = true;
                }
            }
            cx.notify();
        });
        let grid = cx.new(ResultGrid::new);
        grid.update(cx, |grid, _| {
            grid.set_export_host(host.clone());
            // The results header carries the row summary and Export.
            grid.set_embedded_status(true);
        });
        let activation = cx.observe_window_activation(window, |this, window, _| {
            if let Some(controls) = &this.controls {
                controls.focus(window.is_window_active());
            }
        });
        Self {
            completion: None,
            completion_inflight: None,
            completion_reset: false,
            completion_status: String::new(),
            completion_composing: false,
            query_changes: query_changes::QueryChanges::default(),
            query_policy: crate::data_model::TablePolicy::UNKNOWN,
            query_changes_return_focus: false,
            document: None,
            receiver: None,
            retained_budget: None,
            retained_bytes: 0,
            host,
            editor,
            buffer,
            failure: None,
            accessible,
            find: None,
            _find_events: None,
            grid,
            plan: None,
            show_plan: false,
            plan_return_focus: false,
            plan_events: None,
            layout,
            expanded: false,
            owner: String::new(),
            session: String::new(),
            stream: crate::stream::Stream::new(String::new(), String::new()),
            controls: None,
            events: None,
            execution: None,
            review: None,
            parameters,
            show_parameters: false,
            parameters_return_focus: false,
            _parameters_events: parameters_events,
            editor_revision: 0,
            transaction: None,
            transaction_pending: false,
            transaction_error: None,
            transaction_menu: None,
            transaction_anchors: [
                crate::ui::popover::anchor_slot(),
                crate::ui::popover::anchor_slot(),
            ],
            transaction_menu_focus: cx.focus_handle(),
            connected: false,
            connecting: false,
            connect_error: None,
            closing: false,
            status: "Connecting".into(),
            toolbar_focus: (0..25).map(|_| cx.focus_handle()).collect(),
            previous_focus: None,
            result_focus: Vec::new(),
            show_notices: false,
            _activation: activation,
            _editor_events: editor_events,
        }
    }
    pub fn new_document(
        host: Arc<Host>,
        document: &mut backend::WorkspaceDocument,
        layout: Layout,
        wake: async_channel::Sender<()>,
        retained_budget: std::rc::Rc<std::cell::Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::build(host, layout, &document.sql, window, cx);
        view.owner = view.host.owner().unwrap_or_default().into();
        view.document = Some(DocumentBinding {
            id: document.id.clone(),
            connection_id: document.connection_id.clone(),
            wake,
        });
        view.grid.update(cx, |grid, _| {
            grid.set_inspection_budget(retained_budget.clone())
        });
        view.retained_budget = Some(retained_budget);
        view.install_completion(cx);
        if let Some(saved) = document.query_changes.take() {
            view.restore_query_changes(saved, cx);
        }
        view.status = "Disconnected".into();
        let selection = document.selection;
        view.editor.update(cx, |editor, cx| {
            editor.change_selections(Default::default(), window, cx, |selections| {
                selections.select_ranges([
                    MultiBufferOffset(selection.anchor)..MultiBufferOffset(selection.head)
                ]);
            });
        });
        view
    }

    pub fn draft(&self, cx: &mut gpui::App) -> (String, backend::WorkspaceSelection) {
        self.editor.update(cx, |editor, cx| {
            let selection = editor
                .selections
                .newest::<MultiBufferOffset>(&editor.display_snapshot(cx));
            let (anchor, head) = if selection.reversed {
                (selection.end.0, selection.start.0)
            } else {
                (selection.start.0, selection.end.0)
            };
            (
                editor.text(cx),
                backend::WorkspaceSelection { anchor, head },
            )
        })
    }

    pub fn draft_bytes(&self, cx: &gpui::App) -> usize {
        self.editor.read(cx).buffer().read(cx).len(cx).0
    }
    pub fn remember_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.previous_focus = Some(
            if let Some(focus) = self.query_changes.view.as_ref().and_then(|view| {
                view.read(cx)
                    .focus_handles(cx)
                    .into_iter()
                    .find(|focus| focus.contains_focused(window, cx))
            }) {
                focus
            } else if let Some(plan) = &self.plan
                && plan.focus_handle(cx).contains_focused(window, cx)
            {
                plan.focus_handle(cx)
            } else if self.grid.read(cx).content_has_focus(window, cx) {
                self.grid.focus_handle(cx)
            } else {
                self.editor.focus_handle(cx)
            },
        );
    }
    pub fn focus_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_content_focus(window, cx);
    }

    pub fn document_status(&self) -> &str {
        &self.status
    }
    pub fn set_document_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.layout = layout;
        cx.notify();
    }
    pub fn bind_connection(&mut self, id: String, cx: &mut Context<Self>) {
        if self.query_changes_blocked(cx) {
            self.status = "Resolve pending result changes before changing connection".into();
            cx.notify();
            return;
        }
        self.retire_query_changes(cx);
        self.invalidate_review(cx);
        if let Some(binding) = &mut self.document {
            binding.connection_id = Some(id);
        }
        // The host sends the new connection's metadata right after binding.
        self.query_policy = crate::data_model::TablePolicy::UNKNOWN;
        self.reset_completion(false, cx);
        cx.emit(WorkbenchEvent::DraftChanged);
    }
    pub fn begin_connect(&mut self, cx: &mut Context<Self>) {
        self.connect(cx);
    }
    pub fn connection_phase(&self) -> crate::document_view::ConnectionPhase {
        use crate::document_view::ConnectionPhase;
        if self.connecting {
            ConnectionPhase::Connecting
        } else if self.connected {
            ConnectionPhase::Connected
        } else if let Some(error) = &self.connect_error {
            ConnectionPhase::Failed(error.clone())
        } else {
            ConnectionPhase::Idle
        }
    }
    pub fn mark_disconnected(&mut self, cx: &mut Context<Self>) {
        self.receiver.take();
        self.events.take();
        self.disconnected("Connection closed".into(), cx);
        self.connect_error = None;
        cx.notify();
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        if !editable {
            self.invalidate_review(cx);
        }
        self.closing = !editable;
        if !editable {
            self.reset_completion(false, cx);
        } else if let Some(handle) = &self.completion {
            handle.set_connected(self.connected);
        }
        if let Some(view) = &self.query_changes.view {
            view.update(cx, |view, cx| view.set_enabled(editable, cx));
        }
        self.editor.update(cx, |editor, _| {
            editor.set_read_only(!editable || self.completion_reset)
        });
        cx.notify();
    }
    pub fn clear_results(&mut self, cx: &mut Context<Self>) {
        if self.execution.is_some() || self.query_changes_blocked(cx) {
            self.status = "Resolve pending result changes before clearing results".into();
            cx.notify();
            return;
        }
        self.retire_query_changes(cx);
        self.plan.take();
        self.plan_events.take();
        self.show_plan = false;
        self.grid.update(cx, |grid, cx| grid.begin(cx));
        self.account_retained(cx);
        cx.notify();
    }
    fn account_retained(&mut self, cx: &gpui::App) {
        let bytes = self.grid.read(cx).model().retained_bytes;
        if let Some(budget) = &self.retained_budget {
            budget.set(
                budget
                    .get()
                    .saturating_sub(self.retained_bytes)
                    .saturating_add(bytes),
            );
        }
        self.retained_bytes = bytes;
    }

    /// The workspace owns the aggregate fair drain. A view consumes at most
    /// one envelope per call, including when it is not the selected tab.
    pub fn drain_one(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(completion) = &self.completion {
            let status = completion.status();
            if status != self.completion_status {
                self.completion_status = status;
                cx.notify();
            }
        }
        if self.drain_completion(cx) {
            return true;
        }
        self.query_changes.turn = !self.query_changes.turn;
        if self.query_changes.turn && self.drain_query_changes(cx) {
            return true;
        }
        let Some(receiver) = &self.receiver else {
            return self.drain_query_changes(cx);
        };
        if let Some(error) = receiver.failure() {
            self.disconnected(error.to_string(), cx);
            self.receiver.take();
            self.events.take();
            cx.notify();
            return true;
        }
        let Some(message) = receiver.receive() else {
            return self.drain_query_changes(cx);
        };
        self.consume(message, cx);
        cx.notify();
        true
    }
    pub fn has_pending(&self, cx: &gpui::App) -> bool {
        self.completion_pending(cx)
            || self.query_changes.pending()
            || self
                .receiver
                .as_ref()
                .is_some_and(|receiver| receiver.pending())
    }

    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.editor.focus_handle(cx), cx);
        self.connect(cx);
    }
    fn connect(&mut self, cx: &mut Context<Self>) {
        if self
            .query_changes
            .view
            .as_ref()
            .is_some_and(|view| view.read(cx).navigation_blocked())
        {
            self.status = "Finish the result edit or pending operation before reconnecting".into();
            cx.notify();
            return;
        }
        if self.closing || self.connecting {
            return;
        }
        self.disconnect_query_changes(cx);
        self.invalidate_review(cx);
        self.review = None;
        self.transaction = None;
        self.transaction_pending = false;
        self.transaction_error = None;
        self.failure = None;
        diagnostics::clear(&self.editor, &self.buffer, cx);
        self.connected = false;
        self.connecting = true;
        self.connect_error = None;
        self.execution = None;
        if self.document.is_none() {
            self.owner = uuid::Uuid::new_v4().to_string();
        }
        self.session = uuid::Uuid::new_v4().to_string();
        let connection_id = match &self.document {
            Some(document) => match &document.connection_id {
                Some(id) => id.clone(),
                None => {
                    self.connecting = false;
                    self.status = "Choose a connection in the Navigator".into();
                    cx.notify();
                    return;
                }
            },
            None => self.host.backend.fixture().id,
        };
        self.stream = match &self.document {
            Some(document) => crate::stream::Stream::for_document(
                self.session.clone(),
                connection_id.clone(),
                document.id.clone(),
            ),
            None => crate::stream::Stream::new(self.session.clone(), connection_id.clone()),
        };
        self.status = "Connecting".into();
        self.events.take();
        #[cfg(not(feature = "fixture-verification"))]
        let capacity = mailbox::QUEUE_CAPACITY;
        #[cfg(feature = "fixture-verification")]
        let capacity = crate::verification::capacity();
        let (sender, receiver) = if self.document.is_some() {
            self.host.mailbox()
        } else {
            mailbox::channel(capacity, mailbox::QUEUE_BYTES)
        };
        let connected = match &self.document {
            Some(document) => self.host.connect_document(
                document.id.clone(),
                connection_id,
                self.session.clone(),
                sender,
            ),
            None => self
                .host
                .connect(self.owner.clone(), self.session.clone(), sender),
        };
        match connected {
            Ok(controls) => self.controls = Some(controls),
            Err(error) => {
                self.disconnected(error.into(), cx);
                cx.notify();
                return;
            }
        }
        if let Some(document) = &self.document {
            let wake = document.wake.clone();
            let awakened = receiver.wake.clone();
            self.receiver = Some(receiver);
            self.events = Some(cx.spawn(async move |_, _| {
                while awakened.recv().await.is_ok() {
                    let _ = wake.try_send(());
                }
            }));
            cx.notify();
            return;
        }
        let owner = self.owner.clone();
        self.events = Some(cx.spawn(async move |this, cx| {
            while receiver.wake.recv().await.is_ok() {
                loop {
                    #[cfg(feature = "fixture-verification")]
                    crate::verification::drain().await;
                    let keep = this
                        .update(cx, |this, cx| {
                            if this.owner != owner || this.closing {
                                return false;
                            }
                            if let Some(failure) = receiver.failure() {
                                this.disconnected(failure.to_string(), cx);
                                cx.notify();
                                return false;
                            }
                            let start = Instant::now();
                            for _ in 0..8 {
                                let Some(message) = receiver.receive() else {
                                    break;
                                };
                                this.consume(message, cx);
                                if !this.connected && !this.connecting {
                                    break;
                                }
                                if start.elapsed() >= Duration::from_millis(2) {
                                    break;
                                }
                            }
                            cx.notify();
                            !this.closing && (this.connected || this.connecting)
                        })
                        .unwrap_or(false);
                    if !keep {
                        eprintln!("Native queue high-water bytes: {}", receiver.high_water());
                        return;
                    }
                    if !receiver.pending() {
                        break;
                    }
                    // Yield only while bounded queued work remains; no idle timer.
                    cx.background_executor()
                        .timer(Duration::from_millis(1))
                        .await;
                }
            }
        }));
        cx.notify();
    }
    fn disconnected(&mut self, message: String, cx: &mut Context<Self>) {
        self.disconnect_query_changes(cx);
        self.invalidate_review(cx);
        self.review = None;
        self.transaction = None;
        self.transaction_pending = false;
        self.transaction_error = None;
        diagnostics::clear(&self.editor, &self.buffer, cx);
        self.connected = false;
        self.connecting = false;
        self.execution = None;
        self.stream.retire();
        self.status = format!("Disconnected: {message}");
        self.connect_error = Some(message);
        if let Some(controls) = &self.controls {
            controls.stop();
        }
    }
    fn console(
        &mut self,
        severity: crate::console_model::Severity,
        source: crate::console_model::Source,
        message: String,
        detail: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.console_with_latency(severity, source, message, detail, None, cx);
    }
    fn console_with_latency(
        &mut self,
        severity: crate::console_model::Severity,
        source: crate::console_model::Source,
        message: String,
        detail: Option<String>,
        latency_ms: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        cx.emit(WorkbenchEvent::Console(crate::console_model::Entry {
            severity,
            source,
            message,
            detail,
            latency_ms,
            connection: self
                .document
                .as_ref()
                .and_then(|document| document.connection_id.clone()),
        }));
    }
    fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = match &self.find {
            Some(view) => view.clone(),
            None => {
                let view =
                    cx.new(|cx| crate::sql_find::FindView::new(self.editor.clone(), window, cx));
                self._find_events = Some(cx.subscribe_in(
                    &view,
                    window,
                    |this, _, _: &crate::sql_find::FindEvent, window, cx| {
                        this.find = None;
                        this._find_events = None;
                        window.focus(&this.editor.focus_handle(cx), cx);
                        cx.notify();
                    },
                ));
                self.find = Some(view.clone());
                view
            }
        };
        view.update(cx, |view, cx| view.focus(window, cx));
        cx.notify();
    }
    fn find_again(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        match self.find.clone() {
            Some(view) => view.update(cx, |view, cx| view.find(forward, false, window, cx)),
            None => self.open_find(window, cx),
        }
    }
    fn consume(&mut self, message: Message, cx: &mut Context<Self>) {
        match message {
            Message::HistoryFailed(error) => {
                self.status = format!("History was not saved: {error}");
            }
            Message::Ready => {
                self.connected = true;
                if let Some(completion) = &self.completion {
                    completion.set_connected(true);
                }
                self.connecting = false;
                self.connect_error = None;
                self.status = "Ready".into();
                self.console(
                    crate::console_model::Severity::Info,
                    crate::console_model::Source::Connection,
                    "Query session connected".into(),
                    None,
                    cx,
                );
                self.connect_query_changes(cx);
            }
            Message::Review {
                execution,
                sql,
                statements,
                parameters,
                row_limit,
            } => {
                let current = self
                    .execution
                    .as_ref()
                    .is_some_and(|value| value.id == execution);
                let unchanged = self
                    .execution
                    .as_ref()
                    .is_some_and(|value| value.review_revision == self.editor_revision);
                if current && unchanged && self.connected && !self.closing {
                    self.review = Some(QueryReview {
                        sql,
                        bindings: review_bindings(parameters.as_deref(), row_limit),
                        classification: statements
                            .iter()
                            .map(|statement| {
                                format!(
                                    "Statement {}: {:?}{}{}",
                                    statement.index + 1,
                                    statement.class,
                                    if statement.unbounded {
                                        " · all rows"
                                    } else {
                                        ""
                                    },
                                    if statement.destructive {
                                        " · destructive"
                                    } else {
                                        ""
                                    }
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(" · "),
                    });
                    self.status = "Review required; query has not run".into();
                } else {
                    if let Some(controls) = &self.controls {
                        controls.discard_confirmation(execution);
                    }
                    if current {
                        self.execution = None;
                        self.status = "Query inputs changed; review discarded".into();
                    }
                }
            }
            Message::Transaction { session, result } => {
                if session != self.session {
                    return;
                }
                self.transaction_pending = false;
                match result {
                    Ok(snapshot) => {
                        self.transaction = Some(snapshot);
                        self.transaction_error = None;
                    }
                    Err(error) => {
                        let status = match &error {
                            QuerySessionError::InvalidTransactionTransition { status, .. } => {
                                *status
                            }
                            _ => QueryTransactionStatus::Unknown,
                        };
                        if let Some(snapshot) = &mut self.transaction {
                            snapshot.status = status;
                        }
                        self.transaction_error = Some(crate::controller::error_message(error));
                    }
                }
            }
            Message::Rejected { execution, message } => {
                if self
                    .execution
                    .as_ref()
                    .is_some_and(|current| current.id == execution)
                {
                    self.execution = None;
                    self.query_changes.source = None;
                    self.review = None;
                    self.status = "Query failed".into();
                    self.failure = Some(QueryFailure {
                        announcement_id: uuid::Uuid::new_v4().to_string(),
                        message,
                        code: None,
                        location: None,
                    });
                }
            }
            Message::CancelFailed { execution, message } => {
                if self
                    .execution
                    .as_ref()
                    .is_some_and(|current| current.id == execution && current.terminal.is_none())
                {
                    self.status = format!("Stop failed: {message}");
                }
            }
            Message::Acked {
                execution,
                sequence,
            } => {
                if self.execution.as_ref().is_some_and(|current| {
                    current.id == execution
                        && current
                            .terminal
                            .is_some_and(|terminal| sequence >= terminal)
                }) {
                    self.execution = None;
                }
            }
            Message::Event(envelope) => {
                match self.stream.admit(
                    &envelope,
                    self.execution
                        .as_ref()
                        .map(|execution| execution.id.as_str()),
                ) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(error) => {
                        self.disconnected(error.into(), cx);
                        return;
                    }
                }
                match &envelope.event {
                    QueryEvent::SessionState { transaction } => {
                        self.transaction = Some(transaction.clone());
                    }
                    QueryEvent::SessionLost { reason } => {
                        self.console(
                            crate::console_model::Severity::Warning,
                            crate::console_model::Source::Connection,
                            "Query session lost".into(),
                            Some(reason.clone()),
                            cx,
                        );
                        self.disconnected(reason.clone(), cx);
                        return;
                    }
                    QueryEvent::SessionClosed => {
                        self.console(
                            crate::console_model::Severity::Info,
                            crate::console_model::Source::Connection,
                            "Query session closed".into(),
                            None,
                            cx,
                        );
                        self.disconnected("Connection closed".into(), cx);
                        self.connect_error = None;
                        return;
                    }
                    QueryEvent::Notice { severity, message } => {
                        self.console(
                            crate::console_model::Severity::for_notice(severity),
                            crate::console_model::Source::Notice,
                            format!("{severity}: {message}"),
                            None,
                            cx,
                        );
                    }
                    QueryEvent::ExecutionCompleted { transaction, .. } => {
                        self.transaction = Some(transaction.clone());
                        eprintln!(
                            "Native retained bytes: {}",
                            self.grid.read(cx).model().retained_bytes
                        );
                        if let Some(execution) = &mut self.execution {
                            execution.terminal = Some(envelope.sequence);
                        }
                    }
                    _ => {}
                }
                let completed = matches!(&envelope.event, QueryEvent::ExecutionCompleted { .. });
                let execution_context = match &envelope.event {
                    QueryEvent::ExecutionCompleted { context, .. } => context.clone(),
                    _ => None,
                };
                let allowance = self.retained_budget.as_ref().map(|budget| {
                    (128 * 1024 * 1024usize)
                        .saturating_sub(budget.get())
                        .saturating_add(self.retained_bytes)
                });
                let retain_more_rows = self.grid.update(cx, |grid, cx| match allowance {
                    Some(limit) => grid.consume_with_limit(envelope.event, limit, cx),
                    None => grid.consume(envelope.event, cx),
                });
                self.account_retained(cx);
                if completed && let Some(completion) = self.grid.read(cx).model().completion.clone()
                {
                    let exact_source = self.query_changes.source.as_ref().is_some_and(|source| {
                        envelope.execution_id.as_deref().is_some_and(|execution| {
                            source.matches(&envelope.connection_id, &envelope.session_id, execution)
                        })
                    });
                    if completion.status != TerminalStatus::Completed || !exact_source {
                        self.query_changes.source = None;
                    } else if let Some(source) = &self.query_changes.source {
                        source.complete(execution_context);
                    }
                    let elapsed = self
                        .execution
                        .as_ref()
                        .map_or(0, |execution| execution.started.elapsed().as_millis());
                    let label = match completion.status {
                        TerminalStatus::Completed => "Completed",
                        TerminalStatus::Cancelled => "Cancelled",
                        TerminalStatus::Failed => "Failed",
                    };
                    self.status = format!("{label} · {elapsed} ms");
                    // Cross-tab query log: one event per terminal run.
                    let retained: usize = self
                        .grid
                        .read(cx)
                        .model()
                        .sets
                        .iter()
                        .map(|set| set.rows.len())
                        .sum();
                    let sql = self
                        .execution
                        .as_ref()
                        .and_then(|execution| execution.source.as_ref())
                        .map(|source| {
                            source
                                .sql()
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" ")
                                .chars()
                                .take(300)
                                .collect::<String>()
                        })
                        .unwrap_or_default();
                    let (severity, message) = match completion.status {
                        TerminalStatus::Failed => (
                            crate::console_model::Severity::Error,
                            "Query failed".to_string(),
                        ),
                        _ => (
                            crate::console_model::Severity::Info,
                            format!(
                                "Query {} · {retained} rows loaded · {elapsed} ms",
                                label.to_lowercase()
                            ),
                        ),
                    };
                    let detail = match &completion.error {
                        Some(error) => format!("{sql}\n{}", error.message),
                        None => sql,
                    };
                    self.console_with_latency(
                        severity,
                        crate::console_model::Source::Query,
                        message,
                        Some(detail),
                        Some(elapsed.min(u128::from(u64::MAX)) as u64),
                        cx,
                    );
                    if let Some(source) = self
                        .execution
                        .as_ref()
                        .and_then(|execution| execution.source.as_ref())
                        && source
                            .sql()
                            .get(..7)
                            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("EXPLAIN"))
                    {
                        let budget = self
                            .retained_budget
                            .clone()
                            .unwrap_or_else(|| std::rc::Rc::new(std::cell::Cell::new(0)));
                        match crate::explain_view::PlanData::from_result(
                            self.grid.read(cx).model(),
                            source.sql(),
                            elapsed.min(u128::from(u64::MAX)) as u64,
                            budget,
                        ) {
                            Ok(data) => {
                                let view =
                                    cx.new(|cx| crate::explain_view::ExplainView::new(data, cx));
                                self.plan_events = Some(cx.subscribe(
                                    &view,
                                    |this, _, _: &crate::explain_view::Close, cx| {
                                        this.show_plan = false;
                                        cx.notify();
                                    },
                                ));
                                self.plan = Some(view);
                                self.show_plan = true;
                            }
                            Err(error) => self.status = format!("{label} · {elapsed} ms · {error}"),
                        }
                    }

                    if completion.status == TerminalStatus::Failed && self.failure.is_none() {
                        let mut location = None;
                        if let Some(error) = &completion.error
                            && let Some(source) = self
                                .execution
                                .as_ref()
                                .and_then(|execution| execution.source.as_ref())
                        {
                            let text = self.editor.read(cx).text(cx);
                            if let Some(range) = source.range_for_position(error.position, &text) {
                                let before = &text[..range.start];
                                let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
                                let column =
                                    before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
                                location = Some(format!("Line {line}, column {column}"));
                                diagnostics::set_error(
                                    &self.buffer,
                                    range,
                                    &error.message,
                                    error.code.as_deref(),
                                    cx,
                                );
                            }
                        }
                        self.failure = Some(QueryFailure {
                            announcement_id: uuid::Uuid::new_v4().to_string(),
                            message: completion
                                .error
                                .as_ref()
                                .map(|error| error.message.clone())
                                .or_else(|| completion.refusal.clone())
                                .unwrap_or_else(|| "Execution failed".into()),
                            code: completion
                                .error
                                .as_ref()
                                .and_then(|error| error.code.clone()),
                            location,
                        });
                    }
                }
                if envelope.requires_ack
                    && let (Some(controls), Some(execution_id)) =
                        (&self.controls, envelope.execution_id)
                {
                    controls.acknowledge(AckPayload {
                        session_id: self.session.clone(),
                        execution_id,
                        ack_through_sequence: envelope.sequence,
                        retain_more_rows,
                    });
                }
            }
        }
    }
    /// Keep the SQL session and manual transaction intact. Retired mutation
    /// sources cannot be reconstructed from an older in-flight result stream.
    pub fn invalidate_after_restore(&mut self, cx: &mut Context<Self>) {
        self.disconnect_query_changes(cx);
        self.invalidate_review(cx);
        self.query_changes.unavailable = Some(
            "Database may have changed; run SQL explicitly to obtain a fresh editing source".into(),
        );
        self.status =
            "Database may have changed; retained results may be stale. SQL session preserved"
                .into();
        cx.notify();
    }
    fn invalidate_review(&mut self, cx: &mut Context<Self>) {
        self.editor_revision = self.editor_revision.wrapping_add(1);
        if let (Some(execution), Some(controls)) = (&self.execution, &self.controls) {
            controls.discard_confirmation(execution.id.clone());
        }
        if self.review.take().is_some() {
            self.execution = None;
            self.status = "Review discarded; query has not run".into();
            cx.notify();
        }
    }

    fn confirm_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.review.is_none() || !self.connected || self.closing || self.transaction_pending {
            return;
        }
        let Some(execution) = &self.execution else {
            return;
        };
        if execution.review_revision != self.editor_revision {
            self.invalidate_review(cx);
            return;
        }
        let Some(controls) = &self.controls else {
            return;
        };
        match controls.send(Command::Confirm(execution.id.clone())) {
            Ok(()) => {
                self.restore_content_focus(window, cx);
                self.review = None;
                if let Some(execution) = &mut self.execution {
                    execution.started = Instant::now();
                }
                self.status = "Running".into();
            }
            Err(error) => self.status = error.into(),
        }
        cx.notify();
    }

    fn transaction_controls() -> [(&'static str, TransactionControl, usize); 8] {
        [
            (
                "Autocommit",
                TransactionControl::Mode(QueryTransactionMode::Autocommit),
                10,
            ),
            (
                "Manual",
                TransactionControl::Mode(QueryTransactionMode::Manual),
                11,
            ),
            (
                "Read committed",
                TransactionControl::Isolation(QueryTransactionIsolation::ReadCommitted),
                12,
            ),
            (
                "Repeatable read",
                TransactionControl::Isolation(QueryTransactionIsolation::RepeatableRead),
                13,
            ),
            (
                "Serializable",
                TransactionControl::Isolation(QueryTransactionIsolation::Serializable),
                14,
            ),
            ("Commit", TransactionControl::Commit, 15),
            ("Rollback", TransactionControl::Rollback, 16),
            ("Recheck", TransactionControl::Recheck, 17),
        ]
    }

    fn can_control_transaction(&self, control: TransactionControl) -> bool {
        self.connected
            && !self.closing
            && !self.transaction_pending
            && self.execution.is_none()
            && transaction_allowed(self.transaction.as_ref(), control)
    }

    fn control_transaction(
        &mut self,
        control: TransactionControl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_control_transaction(control) {
            return;
        }
        let Some(controls) = &self.controls else {
            return;
        };
        match controls.send(Command::Transaction(control)) {
            Ok(()) => {
                self.restore_content_focus(window, cx);
                self.transaction_pending = true;
                self.transaction_error = None;
            }
            Err(error) => self.transaction_error = Some(error.into()),
        }
        cx.notify();
    }

    fn transaction_label(&self) -> String {
        if !self.connected {
            return "Transaction: no session".into();
        }
        if self.transaction_pending {
            return "Transaction update pending".into();
        }
        let Some(snapshot) = &self.transaction else {
            return "Transaction state unknown; recheck".into();
        };
        let mode = match snapshot.mode {
            QueryTransactionMode::Autocommit => "Autocommit",
            QueryTransactionMode::Manual => "Manual",
        };
        let status = match snapshot.status {
            QueryTransactionStatus::Idle => "idle",
            QueryTransactionStatus::Active => "active",
            QueryTransactionStatus::Failed => "failed; rollback required",
            QueryTransactionStatus::Unknown => "unknown; recheck required",
        };
        match snapshot.pooling {
            ConnectionPooling::TransactionPooler => {
                format!("{mode} · {status} · transaction pooler")
            }
            ConnectionPooling::Pooler => format!("{mode} · {status} · connection pooler"),
            ConnectionPooling::Direct | ConnectionPooling::SessionPooler => {
                format!("{mode} · {status}")
            }
        }
    }

    /// The options of one transaction select, in menu order.
    fn transaction_options(menu: TransactionMenu) -> Vec<(&'static str, TransactionControl)> {
        Self::transaction_controls()
            .into_iter()
            .filter(|(_, control, _)| menu.owns(*control))
            .map(|(label, control, _)| (label, control))
            .collect()
    }

    fn transaction_selected(&self, control: TransactionControl) -> bool {
        self.transaction
            .as_ref()
            .is_some_and(|snapshot| match control {
                TransactionControl::Mode(mode) => snapshot.mode == mode,
                TransactionControl::Isolation(isolation) => snapshot.manual_isolation == isolation,
                _ => false,
            })
    }

    /// The select's face: the selected option, or `Unknown` before the first
    /// snapshot.
    fn transaction_value(&self, menu: TransactionMenu) -> &'static str {
        Self::transaction_options(menu)
            .into_iter()
            .find(|(_, control)| self.transaction_selected(*control))
            .map_or("Unknown", |(label, _)| label)
    }

    fn transaction_menu_enabled(&self, menu: TransactionMenu) -> bool {
        Self::transaction_options(menu)
            .into_iter()
            .any(|(_, control)| self.can_control_transaction(control))
    }

    /// Toolbar focus slots of the transaction cluster, in visual order.
    fn transaction_focus_order(&self) -> Vec<usize> {
        let mut order = Vec::new();
        for menu in [TransactionMenu::Mode, TransactionMenu::Isolation] {
            if self.transaction_menu_enabled(menu) {
                order.push(menu.focus_index());
            }
        }
        if transaction_actions_visible(self.connected, self.transaction.as_ref()) {
            order.extend(
                Self::transaction_controls()
                    .into_iter()
                    .filter(|(_, control, _)| {
                        matches!(
                            control,
                            TransactionControl::Commit
                                | TransactionControl::Rollback
                                | TransactionControl::Recheck
                        ) && self.can_control_transaction(*control)
                    })
                    .map(|(_, _, index)| index),
            );
        }
        order
    }

    fn toggle_transaction_menu(
        &mut self,
        menu: TransactionMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.transaction_menu.is_some_and(|(open, _)| open == menu) {
            self.close_transaction_menu(window, cx);
            return;
        }
        if !self.transaction_menu_enabled(menu) {
            return;
        }
        let options = Self::transaction_options(menu);
        let mut nav = crate::ui::popover::MenuNav::new(options.len());
        nav.highlighted = options
            .iter()
            .position(|(_, control)| self.transaction_selected(*control))
            .unwrap_or(0);
        self.transaction_menu = Some((menu, nav));
        window.focus(&self.transaction_menu_focus, cx);
        cx.notify();
    }

    fn close_transaction_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((menu, _)) = self.transaction_menu.take() {
            if self.transaction_menu_focus.contains_focused(window, cx) {
                window.focus(&self.toolbar_focus[menu.focus_index()], cx);
            }
            cx.notify();
        }
    }

    /// Runs one option of a transaction select with the same command the
    /// old radio buttons sent.
    fn choose_transaction(
        &mut self,
        control: TransactionControl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = self.transaction_menu.take().map(|(menu, _)| menu);
        self.control_transaction(control, window, cx);
        if let Some(menu) = menu
            && self.transaction_menu_focus.contains_focused(window, cx)
        {
            window.focus(&self.toolbar_focus[menu.focus_index()], cx);
        }
        cx.notify();
    }

    fn transaction_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((menu, nav)) = self.transaction_menu.as_mut() else {
            return;
        };
        let menu = *menu;
        match nav.key(&event.keystroke.key) {
            crate::ui::popover::MenuKey::Moved => cx.notify(),
            crate::ui::popover::MenuKey::Dismiss => self.close_transaction_menu(window, cx),
            crate::ui::popover::MenuKey::Activate(index) => {
                if let Some((_, control)) = Self::transaction_options(menu).get(index).copied()
                    && self.can_control_transaction(control)
                {
                    self.choose_transaction(control, window, cx);
                }
            }
            crate::ui::popover::MenuKey::Ignored => return,
        }
        cx.stop_propagation();
    }

    /// The closed face of one transaction select.
    fn transaction_select(&self, menu: TransactionMenu, cx: &Context<Self>) -> impl IntoElement {
        let enabled = self.transaction_menu_enabled(menu);
        let open = self.transaction_menu.is_some_and(|(open, _)| open == menu);
        let weak = cx.weak_entity();
        crate::ui::popover::select_trigger(
            ("transaction-select", menu.index()),
            menu.label(),
            self.transaction_value(menu),
            open,
            enabled,
        )
        .w(px(match menu {
            TransactionMenu::Mode => 112.,
            TransactionMenu::Isolation => 136.,
        }))
        .relative()
        .child(crate::ui::popover::probe(
            self.transaction_anchors[menu.index()].clone(),
        ))
        .key_context("NativeToolbar")
        .track_focus(&self.toolbar_focus[menu.focus_index()])
        .tab_index(0)
        .tab_stop(enabled)
        .on_a11y_action(Action::Click, move |_, window, cx| {
            weak.update(cx, |this, cx| {
                this.toggle_transaction_menu(menu, window, cx)
            })
            .ok();
        })
        .on_click(
            cx.listener(move |this, _, window, cx| this.toggle_transaction_menu(menu, window, cx)),
        )
    }

    /// The open transaction select's option list.
    fn render_transaction_menu(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let (menu, nav) = self.transaction_menu?;
        let anchor = self.transaction_anchors[menu.index()].get()?;
        let mut panel = crate::ui::popover::panel("transaction-menu", Role::Menu, menu.label())
            .track_focus(&self.transaction_menu_focus)
            .min_w(px(180.))
            .on_key_down(cx.listener(Self::transaction_menu_key))
            .on_mouse_down_out(
                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                    // The trigger's own click toggles; closing here would reopen it.
                    if !anchor.contains(&event.position) {
                        this.transaction_menu = None;
                        cx.notify();
                    }
                }),
            );
        for (index, (label, control)) in Self::transaction_options(menu).into_iter().enumerate() {
            let enabled = self.can_control_transaction(control);
            let weak = cx.weak_entity();
            panel = panel.child(
                crate::ui::popover::check_item(
                    ("transaction-option", index),
                    label,
                    self.transaction_selected(control),
                    nav.highlighted == index,
                    enabled,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if enabled {
                        this.choose_transaction(control, window, cx);
                    }
                }))
                .on_a11y_action(Action::Click, move |_, window, cx| {
                    if enabled {
                        weak.update(cx, |this, cx| this.choose_transaction(control, window, cx))
                            .ok();
                    }
                }),
            );
        }
        Some(
            crate::ui::popover::layer(anchor, crate::ui::popover::Placement::BelowEnd, panel)
                .into_any_element(),
        )
    }

    fn explain_draft(&mut self, analyze: bool, cx: &mut Context<Self>) {
        if self.closing || self.document.is_none() {
            return;
        }
        // Avoid copying an unbounded editor buffer into a new persisted document.
        if self.editor.read(cx).buffer().read(cx).len(cx).0 > backend::explain::MAX_PLAN_BYTES {
            self.status = "EXPLAIN source exceeds 1 MiB".into();
            cx.notify();
            return;
        }
        let source = self.editor.update(cx, |editor, cx| {
            let text = editor.text(cx);
            let selection = editor
                .selections
                .newest::<MultiBufferOffset>(&editor.display_snapshot(cx));
            backend::select_sql(&text, &(selection.start.0..selection.end.0), false)
        });
        let result = source
            .map_err(|_| "SQL cannot be parsed")
            .and_then(|sql| sql.ok_or("Select one SQL statement"))
            .and_then(|sql| crate::explain_view::draft(&sql, analyze));
        match result {
            Ok(sql) => cx.emit(WorkbenchEvent::OpenQuery(
                crate::query_library_view::OpenQuery {
                    sql,
                    name: if analyze {
                        "EXPLAIN ANALYZE"
                    } else {
                        "EXPLAIN"
                    }
                    .into(),
                    connection: self
                        .document
                        .as_ref()
                        .and_then(|document| document.connection_id.clone()),
                    saved_id: None,
                },
            )),
            Err(error) => self.status = error.into(),
        }
        cx.notify();
    }

    fn insert_snippet(
        &mut self,
        snippet: sql::Snippet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing || self.execution.is_some() {
            return;
        }
        let composed = self.editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
        });
        if composed {
            self.status = "Finish composing before inserting a snippet".into();
            cx.notify();
            return;
        }
        // Match the baseline append behavior, preserving the existing document
        // and applying the insertion as one ordinary undoable editor transaction.
        self.editor.update(cx, |editor, cx| {
            let end = editor.buffer().read(cx).len(cx);
            let text = format!("{}{}", if end.0 == 0 { "" } else { "\n\n" }, snippet.sql());
            let caret = MultiBufferOffset(end.0 + text.len());
            editor.transact(window, cx, |editor, window, cx| {
                editor.edit([(end..end, text)], cx);
                editor.change_selections(Default::default(), window, cx, |selection| {
                    selection.select_ranges([caret..caret]);
                });
            });
        });
        self.previous_focus = Some(self.editor.focus_handle(cx));
        window.focus(&self.editor.focus_handle(cx), cx);
        self.status = "Snippet appended; edit the placeholders before running".into();
        cx.notify();
    }

    fn run(&mut self, script: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.query_changes_blocked(cx) {
            self.status = "Review or discard result changes before running another query".into();
            cx.notify();
            return;
        }
        if !self.connected || self.execution.is_some() || self.transaction_pending || self.closing {
            return;
        }
        let selection = self.editor.update(cx, |editor, cx| {
            let text = editor.text(cx);
            let selection = editor
                .selections
                .newest::<MultiBufferOffset>(&editor.display_snapshot(cx));
            backend::select_sql_range(&text, &(selection.start.0..selection.end.0), script)
                .map(|range| range.map(|range| (text, range)))
        });
        let (text, range) = match selection {
            Ok(Some(selection)) => selection,
            Ok(None) => {
                self.status = "No SQL statement selected".into();
                cx.notify();
                return;
            }
            Err(error) => {
                self.status = format!("Not run: {error:?}");
                cx.notify();
                return;
            }
        };
        let sql = text[range.clone()].to_owned();
        let (parameters, row_limit) = if self.document.is_some() {
            match self
                .parameters
                .update(cx, |parameters, cx| parameters.prepare(&sql, window, cx))
            {
                Ok(values) => values,
                Err(message) => {
                    self.show_parameters = true;
                    self.status = message;
                    cx.notify();
                    return;
                }
            }
        } else {
            (None, None)
        };
        let source = ExecutedSql::new(text, range);
        let id = uuid::Uuid::new_v4().to_string();
        let mutation_source = self.retained_budget.clone().and_then(|budget| {
            let connection = self.document.as_ref()?.connection_id.as_deref()?;
            Some(crate::query_result::ExecutedSource::prepare(
                &sql,
                parameters.is_some(),
                connection,
                &self.session,
                &id,
                budget,
            ))
        });
        let (mutation_source, mutation_refusal) = match mutation_source {
            Some(Ok(source)) => (Some(source), None),
            Some(Err(error)) => (None, Some(error)),
            None => (None, None),
        };
        let payload = ExecutePayload {
            session_id: self.session.clone(),
            execution_id: id.clone(),
            sql,
            confirmed: false,
            parameters,
            row_limit,
        };
        if let Some(controls) = &self.controls {
            #[cfg(feature = "fixture-verification")]
            crate::verification::before_run();
            match controls.send(Command::Run(payload)) {
                Ok(()) => {
                    if self.toolbar_focus[9].is_focused(window)
                        || self
                            .result_focus
                            .iter()
                            .any(|focus| focus.is_focused(window))
                    {
                        window.focus(&self.editor.focus_handle(cx), cx);
                    }
                    self.failure = None;
                    diagnostics::clear(&self.editor, &self.buffer, cx);
                    self.plan.take();
                    self.plan_events.take();
                    self.show_plan = false;
                    self.retire_query_changes(cx);
                    self.query_changes.source = mutation_source;
                    self.query_changes.unavailable = mutation_refusal;
                    self.grid.update(cx, |grid, cx| grid.begin(cx));
                    self.account_retained(cx);
                    self.show_notices = false;
                    self.execution = Some(Execution {
                        id,
                        started: Instant::now(),
                        terminal: None,
                        source,
                        review_revision: self.editor_revision,
                    });
                    self.status = "Running".into();
                }
                Err(error) => self.status = error.into(),
            }
        }
        cx.notify();
    }
    fn stop(&mut self, cx: &mut Context<Self>) {
        if self.review.is_some() {
            self.invalidate_review(cx);
            return;
        }
        if let (Some(execution), Some(controls)) = (&self.execution, &self.controls)
            && execution.terminal.is_none()
        {
            match controls.send(Command::Cancel(ExecutionPayload {
                session_id: self.session.clone(),
                execution_id: execution.id.clone(),
            })) {
                Ok(()) => self.status = "Stopping".into(),
                Err(error) => self.status = error.into(),
            }
        }
        cx.notify();
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.document.is_some() {
            cx.emit(WorkbenchEvent::Quit);
            return;
        }
        if self.closing {
            return;
        }
        self.closing = true;
        #[cfg(feature = "fixture-verification")]
        eprintln!("VERIFY closing");
        diagnostics::clear(&self.editor, &self.buffer, cx);
        self.stream.retire();
        self.status = "Closing connection".into();
        self.events.take();
        let host = self.host.clone();
        let shutdown = self
            .host
            .runtime
            .spawn(async move { host.shutdown().await });
        self.events = Some(cx.spawn(async move |_this, cx| {
            match shutdown.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("Native cleanup failed: {error}"),
                Err(error) => eprintln!("Native cleanup task failed: {error}"),
            }
            cx.update(|cx| cx.quit());
        }));
        cx.notify();
    }
    fn layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.layout = layout;
        if self.document.is_some() {
            cx.emit(WorkbenchEvent::LayoutChanged(layout));
        }
        if self.document.is_none()
            && let Some(controls) = &self.controls
            && let Err(error) = controls.send(Command::Layout(layout))
        {
            self.status = error.into();
        }
        cx.notify();
    }
    fn switch_pane(&mut self, _: &SwitchPane, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .query_changes
            .view
            .as_ref()
            .is_some_and(|view| view.update(cx, |view, cx| view.composition_active(window, cx)))
        {
            return;
        }
        if !self.show_plan && !self.grid.read(cx).inspector_has_focus(window, cx) {
            let mut handles = vec![
                self.editor.focus_handle(cx),
                self.grid.read(cx).pane_focus(cx),
            ];
            if self.grid.read(cx).can_export() {
                handles.extend(self.grid.read(cx).export_focus());
            }
            if let Some(view) = &self.query_changes.view {
                handles.extend(view.read(cx).focus_handles(cx));
            }
            let current = handles.iter().position(|h| h.is_focused(window));
            let next = if window.modifiers().shift {
                current.map_or(handles.len() - 1, |i| {
                    (i + handles.len() - 1) % handles.len()
                })
            } else {
                current.map_or(0, |i| (i + 1) % handles.len())
            };
            self.show_notices = false;
            window.focus(&handles[next], cx);
            cx.notify();
            return;
        }
        let focus = if self.grid.read(cx).content_has_focus(window, cx)
            || self
                .plan
                .as_ref()
                .is_some_and(|plan| plan.focus_handle(cx).contains_focused(window, cx))
        {
            self.editor.focus_handle(cx)
        } else {
            self.show_notices = false;
            if self.show_plan
                && let Some(plan) = &self.plan
            {
                plan.update(cx, |plan, cx| plan.focus(window, cx));
                return;
            }
            self.grid.read(cx).pane_focus(cx)
        };
        window.focus(&focus, cx);
        cx.notify();
    }
    fn focus_toolbar(&mut self, _: &FocusToolbar, window: &mut Window, cx: &mut Context<Self>) {
        if self.grid.read(cx).content_has_focus(window, cx) {
            self.previous_focus = Some(self.grid.focus_handle(cx));
        } else if self.editor.focus_handle(cx).contains_focused(window, cx) {
            self.previous_focus = Some(self.editor.focus_handle(cx));
        }
        let index = if self.review.is_some() && !self.closing {
            18
        } else if self.connected
            && self.execution.is_none()
            && !self.transaction_pending
            && !self.closing
        {
            0
        } else if self
            .execution
            .as_ref()
            .is_some_and(|execution| execution.terminal.is_none())
            && !self.closing
        {
            2
        } else if !self.connected && !self.connecting && !self.closing {
            3
        } else {
            4
        };
        window.focus(&self.toolbar_focus[index], cx);
    }
    fn move_control(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Keep toolbar Tab navigation out of editor/grid focus routes. Include
        // only visible, enabled controls in their rendered order.
        if self.closing {
            return;
        }
        // Visual order: run group, draft tools, cell edit, connect, then the
        // transaction cluster and layout at the right.
        let mut controls = Vec::new();
        if self.connected
            && self.execution.is_none()
            && !self.transaction_pending
            && !self.query_changes_blocked(cx)
        {
            controls.extend([0, 1]);
        }
        if self.review.is_none()
            && self
                .execution
                .as_ref()
                .is_some_and(|execution| execution.terminal.is_none())
        {
            controls.push(2);
        }
        if self.document.is_some() {
            controls.extend([20, 21, 22]);
        }
        if self.can_edit_query_cell(cx) {
            controls.push(24);
        }
        if !self.connected && !self.connecting {
            controls.push(3);
        }
        if self.document.is_some() {
            controls.extend(self.transaction_focus_order());
        }
        controls.extend([4, 5, 6]);
        if self.layout == Layout::ResultsFirst {
            controls.push(7);
        }
        if self.failure.is_some() {
            controls.push(9);
        }
        if self.review.is_some() {
            controls.extend([18, 19]);
        }
        let mut focus = controls
            .into_iter()
            .map(|index| self.toolbar_focus[index].clone())
            .collect::<Vec<_>>();
        if let Some(view) = &self.query_changes.view {
            focus.extend(view.read(cx).focus_handles(cx));
        }
        focus.extend(self.result_focus.iter().cloned());
        focus.push(self.toolbar_focus[8].clone());
        if self.plan.is_some() {
            focus.push(self.toolbar_focus[23].clone());
        }
        let current = focus.iter().position(|handle| handle.is_focused(window));
        let next = match (current, backwards) {
            (Some(index), true) => (index + focus.len() - 1) % focus.len(),
            (Some(index), false) => (index + 1) % focus.len(),
            (None, true) => focus.len() - 1,
            (None, false) => 0,
        };
        window.focus(&focus[next], cx);
    }
    fn restore_content_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self
            .previous_focus
            .clone()
            .unwrap_or_else(|| self.editor.focus_handle(cx));
        if focus == self.grid.focus_handle(cx) {
            self.show_notices = false;
        }
        window.focus(&focus, cx);
        cx.notify();
    }
    fn leave_toolbar(&mut self, _: &LeaveToolbar, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_content_focus(window, cx);
    }
    fn reconnect(&mut self, _: &Reconnect, window: &mut Window, cx: &mut Context<Self>) {
        // Restore content before Ready removes the reconnect control.
        self.restore_content_focus(window, cx);
        self.connect(cx);
    }
    fn cycle_layout(&mut self, _: &CycleLayout, _window: &mut Window, cx: &mut Context<Self>) {
        self.layout(
            match self.layout {
                Layout::Stacked => Layout::SideBySide,
                Layout::SideBySide => Layout::ResultsFirst,
                Layout::ResultsFirst => Layout::Stacked,
            },
            cx,
        );
    }
    fn activate(&mut self, control: Control, window: &mut Window, cx: &mut Context<Self>) {
        match control {
            Control::EditCell => self.edit_query_cell(window, cx),
            Control::Explain(analyze) => self.explain_draft(analyze, cx),
            Control::Plan => {
                self.show_notices = false;
                self.show_plan = true;
                if let Some(view) = &self.plan {
                    view.update(cx, |view, cx| view.focus(window, cx));
                }
                cx.notify();
            }
            Control::Bindings => {
                self.show_parameters = !self.show_parameters;
                if self.show_parameters {
                    self.parameters
                        .update(cx, |parameters, cx| parameters.focus(window, cx));
                } else {
                    self.restore_content_focus(window, cx);
                }
                cx.notify();
            }
            Control::Transaction(action) => self.control_transaction(action, window, cx),
            Control::ConfirmQuery => self.confirm_query(window, cx),
            Control::CancelReview => {
                self.restore_content_focus(window, cx);
                self.invalidate_review(cx);
            }
            Control::ReturnToSql => {
                window.focus(&self.editor.focus_handle(cx), cx);
                cx.notify();
            }
            Control::Run => self.run(false, window, cx),
            Control::Script => self.run(true, window, cx),
            Control::Stop => self.stop(cx),
            Control::Reconnect => self.reconnect(&Reconnect, window, cx),
            Control::Layout(layout) => self.layout(layout, cx),
            Control::Expand => {
                self.expanded = !self.expanded;
                cx.notify();
            }
            Control::Result(index) => {
                self.show_notices = false;
                self.show_plan = false;
                self.grid.update(cx, |grid, cx| grid.set_active(index, cx));
                cx.notify();
            }
            Control::Notices => {
                self.show_plan = false;
                self.show_notices = !self.show_notices;
                cx.notify();
            }
        }
    }
    fn button<L: Into<SharedString>>(
        &self,
        label: L,
        control: Control,
        index: usize,
        enabled: bool,
        selected: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<L> {
        let label = label.into();
        let focus = if let Control::Result(result) = control {
            self.result_focus[result].clone()
        } else {
            self.toolbar_focus[index].clone()
        };
        let weak = cx.weak_entity();
        let role = match control {
            Control::Layout(_)
            | Control::Transaction(
                TransactionControl::Mode(_) | TransactionControl::Isolation(_),
            ) => Role::RadioButton,
            Control::Result(_) => Role::Tab,
            _ => Role::Button,
        };
        let icon = match control {
            Control::Run => Some("icons/play_filled.svg"),
            Control::Script => Some("icons/play_outlined.svg"),
            Control::Stop => Some("icons/stop.svg"),
            Control::Explain(_) => Some("icons/list_tree.svg"),
            Control::Reconnect => Some("icons/rotate_cw.svg"),
            _ => None,
        };
        let shortcut = match control {
            Control::Run => Some(("⌘↵", "Command+Enter")),
            Control::Script => Some(("⇧⌘↵", "Shift+Command+Enter")),
            Control::Stop => Some(("⌘.", "Command+Period")),
            _ => None,
        };
        let id = SharedString::from(format!("{control:?}"));
        let button = if matches!(
            control,
            Control::Result(_) | Control::Notices | Control::Plan | Control::Layout(_)
        ) {
            crate::ui::segment(id, label, selected, enabled)
        } else if matches!(control, Control::Run | Control::ConfirmQuery) {
            // The toolbar's one primary action; the shortcut is added below.
            crate::ui::tool_button_accent(id, label, None, enabled)
        } else {
            crate::ui::pressed(
                crate::ui::tool_button(id, label, icon, enabled, false),
                selected,
            )
        };
        button
            .role(role)
            .when(
                matches!(
                    control,
                    Control::Layout(_)
                        | Control::Notices
                        | Control::Transaction(
                            TransactionControl::Mode(_) | TransactionControl::Isolation(_)
                        )
                ),
                |button| button.aria_toggled(selected.into()),
            )
            .when(matches!(control, Control::Result(_)), |button| {
                button.aria_selected(selected)
            })
            .when_some(shortcut, |button, (keys, spoken)| {
                button
                    .aria_keyshortcuts(spoken)
                    .child(crate::ui::shortcut(keys))
            })
            .key_context("NativeToolbar")
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(enabled)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .on_a11y_action(Action::Click, move |_, window, cx| {
                if enabled {
                    weak.update(cx, |this, cx| this.activate(control, window, cx))
                        .ok();
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if enabled {
                    this.activate(control, window, cx);
                }
            }))
    }
}
impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_completion(window, cx);
        if self.query_changes_return_focus {
            self.query_changes_return_focus = false;
            window.focus(&self.grid.focus_handle(cx), cx);
        }
        if self.plan_return_focus {
            self.plan_return_focus = false;
            window.focus(&self.grid.focus_handle(cx), cx);
        }
        if self.parameters_return_focus {
            self.parameters_return_focus = false;
            window.focus(&self.toolbar_focus[20], cx);
        }
        let editable_bindings =
            !self.closing && (self.execution.is_none() || self.review.is_some());
        self.parameters.update(cx, |parameters, cx| {
            parameters.set_editable(editable_bindings, cx)
        });
        if self.review.is_none()
            && (self.toolbar_focus[18].is_focused(window)
                || self.toolbar_focus[19].is_focused(window))
        {
            self.restore_content_focus(window, cx);
        }
        let can_run = !self.query_changes_blocked(cx)
            && self.connected
            && self.execution.is_none()
            && !self.transaction_pending
            && !self.closing;
        let can_stop = self
            .execution
            .as_ref()
            .is_some_and(|execution| execution.terminal.is_none())
            && !self.closing
            && self.review.is_none();
        let show_edit_cell = self.can_edit_query_cell(cx) || self.query_changes.view.is_some();
        let wide = window.viewport_size().width > px(860.);
        let side = self.layout == Layout::SideBySide && wide;
        let compact = self.layout == Layout::ResultsFirst && !self.expanded;
        let editor = div()
            .min_h_0()
            .min_w_0()
            .when(side, |pane| {
                pane.w(gpui::relative(0.4)).h_full().border_r_1()
            })
            .when(!side, |pane| {
                pane.w_full()
                    .h(gpui::relative(if compact { 0.20 } else { 0.42 }))
                    .border_b_1()
            })
            .border_color(crate::style::line())
            .flex()
            .flex_col()
            .when_some(self.find.clone(), |editor, find| editor.child(find))
            .child(div().flex_1().min_h_0().child(self.accessible.clone()));
        let result_count = self.grid.read(cx).model().sets.len();
        self.result_focus
            .resize_with(result_count, || cx.focus_handle());
        let model = self.grid.read(cx).model();
        let tabs = model
            .sets
            .iter()
            .enumerate()
            .map(|(index, set)| {
                self.button(
                    format!(
                        "Result {} · {} rows{}",
                        set.index + 1,
                        set.row_count.unwrap_or(set.rows.len() as u64),
                        if set.partial { " · partial" } else { "" }
                    ),
                    Control::Result(index),
                    8,
                    !self.closing,
                    !self.show_notices && index == model.active,
                    cx,
                )
            })
            .collect::<Vec<_>>();
        let notices = model
            .notices
            .iter()
            .map(|notice| format!("{}: {}", notice.severity, notice.message))
            .collect::<Vec<_>>();
        let notices_count = notices.len();
        let mut diagnostics = Vec::new();
        if let Some(completion) = &model.completion {
            if completion.omitted_rows > 0 {
                diagnostics.push(format!("{} rows omitted", completion.omitted_rows));
            }
            if completion.omitted_result_sets > 0 {
                diagnostics.push(format!(
                    "{} result sets omitted",
                    completion.omitted_result_sets
                ));
            }
            if completion.omitted_notices > 0 {
                diagnostics.push(format!("{} notices omitted", completion.omitted_notices));
            }
            if completion.omitted_metadata_bytes > 0 {
                diagnostics.push(format!(
                    "{} metadata bytes omitted",
                    completion.omitted_metadata_bytes
                ));
            }
            diagnostics.extend(completion.truncation_reasons.clone());
        }
        if model.retention_limited {
            diagnostics.push(format!(
                "Native retention limit: at least {} rows, {} result sets, {} metadata bytes omitted",
                model.native_omitted_rows,
                model.native_omitted_result_sets,
                model.native_omitted_metadata
            ));
        }
        let no_results = model.sets.is_empty();
        let summary = self.grid.read(cx).summary();
        let can_export = self.grid.read(cx).can_export() && !self.show_notices && !self.show_plan;
        let export_focus = self.grid.read(cx).export_focus();
        let status = self.status.clone();
        let results = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .when_some(self.failure.as_ref(), |pane, failure| {
                let heading = failure.code.as_ref().map_or("Query failed".into(), |code| {
                    format!("Query failed · {code}")
                });
                // The banner's first line is its icon and heading; the
                // full-width details wrap onto the lines below inside the
                // same red wash.
                let details = div()
                    .w_full()
                    .pl(px(crate::style::ICON + crate::style::S3))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(crate::style::S2))
                    .child(
                        div()
                            .id("query-failure-message")
                            .role(Role::Label)
                            .aria_label(failure.message.clone())
                            .child(failure.message.clone()),
                    )
                    .when_some(failure.location.clone(), |surface, location| {
                        surface.child(
                            div()
                                .id("query-failure-location")
                                .role(Role::Label)
                                .aria_label(location.clone())
                                .font_family(crate::style::MONO)
                                .text_color(crate::style::dim())
                                .child(location),
                        )
                    })
                    .child(self.button(
                        "Return to SQL",
                        Control::ReturnToSql,
                        9,
                        !self.closing,
                        false,
                        cx,
                    ));
                pane.child(
                    div()
                        .id("query-failure")
                        .role(Role::Group)
                        .aria_label("Query failed")
                        .flex_shrink_0()
                        .max_h(px(180.))
                        .overflow_y_scroll()
                        .px(px(crate::style::INSET_BAR))
                        .py(px(crate::style::S3))
                        // A new identity per failure replays the shake, even
                        // for an identical repeated error.
                        .child(crate::ui::shake(
                            format!("query-failure-shake-{}", failure.announcement_id),
                            crate::ui::error_banner("query-failure-banner", heading)
                                .flex_wrap()
                                .child(details),
                        )),
                )
            })
            .child(
                // One header row: result tabs at the left; row summary, query
                // status and Export at the right.
                div()
                    .id("results-header")
                    .flex_none()
                    .h(px(crate::style::CONTROL + crate::style::S1))
                    .pl(px(crate::style::INSET_BAR))
                    .pr(px(crate::style::S2))
                    .flex()
                    .items_center()
                    .gap(px(crate::style::S4))
                    .border_b_1()
                    .border_color(crate::style::line_soft())
                    .text_sm()
                    .text_color(crate::style::dim())
                    .child(
                        div()
                            .id("result-tabs")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_nowrap()
                            .items_center()
                            .gap(px(crate::style::S1))
                            .overflow_x_scroll()
                            .when(no_results, |row| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .pr(px(crate::style::S2))
                                        .text_color(crate::style::dim())
                                        .child("No results"),
                                )
                            })
                            .children(tabs)
                            .when(self.plan.is_some(), |pane| {
                                pane.child(self.button(
                                    "Plan",
                                    Control::Plan,
                                    23,
                                    !self.closing,
                                    self.show_plan,
                                    cx,
                                ))
                            })
                            .child(self.button(
                                format!("Notices {notices_count}"),
                                Control::Notices,
                                8,
                                true,
                                self.show_notices,
                                cx,
                            )),
                    )
                    .when(!no_results, |row| {
                        row.child(
                            div()
                                .id("result-summary")
                                .role(Role::Label)
                                .aria_label(summary.clone())
                                .flex_none()
                                .max_w(px(320.))
                                .truncate()
                                .text_size(px(crate::style::FONT_SMALL))
                                .child(summary),
                        )
                    })
                    .child(
                        div()
                            .id("query-status")
                            .role(Role::Status)
                            .aria_label("Query status")
                            .a11y_synthetic_children(move |builder| {
                                builder.parent_node().set_value(status.clone())
                            })
                            .flex_shrink_1()
                            .min_w_0()
                            .max_w(px(360.))
                            .truncate()
                            .text_size(px(crate::style::FONT_SMALL))
                            .text_color(crate::style::dim())
                            .tooltip(crate::ui::tooltip(self.status.clone()))
                            .tooltip_show_delay(crate::ui::tooltip_delay())
                            .child(self.status.clone()),
                    )
                    .when(can_export, |row| {
                        let grid = self.grid.clone();
                        row.child(
                            crate::ui::icon_button(
                                "open-result-export",
                                "Export results",
                                "icons/download.svg",
                                true,
                            )
                            .aria_label(
                                "Export selected cells, or all retained rows when no selection",
                            )
                            .when_some(export_focus.clone(), |button, focus| {
                                button.track_focus(&focus).tab_stop(true).tab_index(0)
                            })
                            .on_a11y_action(Action::Click, move |_, window, cx| {
                                grid.update(cx, |grid, cx| grid.open_export(window, cx));
                            })
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.grid
                                        .update(cx, |grid, cx| grid.open_export(window, cx));
                                },
                            )),
                        )
                    }),
            )
            .child(
                div()
                    .id("result-content")
                    .flex_1()
                    .min_h_0()
                    .when(!self.show_notices && !self.show_plan, |pane| {
                        pane.child(self.grid.clone())
                    })
                    .when(self.show_plan, |pane| pane.children(self.plan.clone()))
                    .when(self.show_notices, |pane| {
                        pane.overflow_y_scroll()
                            .px(px(crate::style::INSET_BAR))
                            .py(px(crate::style::S2))
                            .text_size(px(crate::style::FONT_DATA))
                            .font_family(crate::style::MONO)
                            .role(Role::Group)
                            .aria_label("Query notices")
                            .children(notices.into_iter().enumerate().map(|(index, notice)| {
                                div()
                                    .id(("query-notice", index))
                                    .role(Role::Label)
                                    .aria_label(notice.clone())
                                    .child(notice)
                            }))
                    }),
            )
            .when(!diagnostics.is_empty(), |pane| {
                pane.child(
                    div()
                        .id("diagnostics")
                        .role(Role::Group)
                        .aria_label("Query diagnostics")
                        .max_h(px(90.))
                        .overflow_y_scroll()
                        .text_sm()
                        .px(px(crate::style::INSET_BAR))
                        .py(px(crate::style::S2))
                        .border_t_1()
                        .border_color(crate::style::line_soft())
                        .text_color(crate::style::warn())
                        .children(diagnostics.into_iter().enumerate().map(|(index, message)| {
                            div()
                                .id(("query-diagnostic", index))
                                .role(Role::Label)
                                .aria_label(message.clone())
                                .child(message)
                        })),
                )
            });
        let announcement = self
            .failure
            .as_ref()
            .map(QueryFailure::announcement)
            .unwrap_or_default();
        div()
            .key_context("Workbench")
            .size_full()
            .flex()
            .flex_col()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.sync_completion_input(window, cx);
                let Some(view) = this.query_changes.view.clone() else { return; };
                if view.update(cx, |view, cx| view.composition_active(window, cx)) { return; }
                let handles = view.read(cx).focus_handles(cx);
                let Some(index) = handles.iter().position(|focus| focus.contains_focused(window, cx)) else { return; };
                let modifiers = event.keystroke.modifiers;
                if event.keystroke.key == "tab" && !modifiers.control && !modifiers.alt && !modifiers.platform {
                    let next = if modifiers.shift { index.checked_sub(1) } else { (index + 1 < handles.len()).then_some(index + 1) };
                    window.focus(next.map_or(&this.toolbar_focus[24], |next| &handles[next]), cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &RunStatement, window, cx| this.run(false, window, cx)))
            .on_action(cx.listener(|this, _: &RunScript, window, cx| this.run(true, window, cx)))
            .on_action(cx.listener(|this, _: &InsertTopRows, window, cx| this.insert_snippet(sql::Snippet::TopRows, window, cx)))
            .on_action(cx.listener(|this, _: &InsertGroupedCount, window, cx| this.insert_snippet(sql::Snippet::GroupedCount, window, cx)))
            .on_action(cx.listener(|this, _: &InsertRecentRows, window, cx| this.insert_snippet(sql::Snippet::RecentRows, window, cx)))
            .on_action(cx.listener(|this, _: &RefreshCompletionMetadata, window, cx| this.refresh_completion(window, cx)))
            .on_action(cx.listener(|this, _: &FormatSql, window, cx| this.format_sql(window, cx)))
            .on_action(cx.listener(|this, _: &FindInSql, window, cx| this.open_find(window, cx)))
            .on_action(cx.listener(|this, _: &FindNext, window, cx| this.find_again(true, window, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, window, cx| this.find_again(false, window, cx)))
            .on_action(cx.listener(|this, _: &StopQuery, _, cx| this.stop(cx)))
            .on_action(cx.listener(Self::reconnect))
            .on_action(cx.listener(Self::switch_pane))
            .on_action(cx.listener(Self::focus_toolbar))
            .on_action(cx.listener(Self::leave_toolbar))
            .on_action(cx.listener(|this, _: &NextControl, window, cx| this.move_control(false, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousControl, window, cx| this.move_control(true, window, cx)))
            .on_action(cx.listener(Self::cycle_layout))
            .on_action(cx.listener(|this, _: &Quit, _, cx| this.close(cx)))
            .map(|element| {
                #[cfg(feature = "fixture-verification")]
                let element = element
                    .on_action(cx.listener(|_, _: &crate::verification::ResumeDrain, _, _| crate::verification::release()))
                    .on_action(cx.listener(|this, _: &crate::verification::Reconnect, _, cx| {
                        if crate::verification::enabled() {
                            this.disconnected("fixture reconnect barrier".into(), cx);
                            this.connect(cx);
                        }
                    }))
                    .on_action(cx.listener(|this, _: &crate::verification::ReplaceView, window, cx| {
                        if !crate::verification::enabled() { return; }
                        this.events.take();
                        this.stream.retire();
                        crate::verification::release();
                        let host = this.host.clone();
                        let layout = this.layout;
                        let view = window.replace_root(cx, |window, cx| Workbench::new(host, layout, window, cx));
                        view.update(cx, |view, cx| view.start(window, cx));
                        eprintln!("VERIFY view-replaced");
                    }));
                element
            })
            .child(
                crate::ui::toolbar()
                    .child(self.button("Run", Control::Run, 0, can_run, false, cx))
                    .child(self.button("Run script", Control::Script, 1, can_run, false, cx))
                    // Stop belongs to a running query; it appears with one.
                    .when(can_stop, |toolbar| {
                        toolbar.child(self.button("Stop", Control::Stop, 2, can_stop, false, cx))
                    })
                    .when(self.document.is_some(), |toolbar| toolbar
                        .child(crate::ui::separator())
                        .child(self.button("Explain draft", Control::Explain(false), 21, !self.closing, false, cx))
                        .child(self.button("Analyze draft", Control::Explain(true), 22, !self.closing, false, cx))
                        .child(self.button("Bindings", Control::Bindings, 20, !self.closing, self.show_parameters, cx)))
                    // Result-cell editing is a later step: shown once a cell
                    // can be edited or an edit is open.
                    .when(show_edit_cell, |toolbar| toolbar
                        .child(crate::ui::separator())
                        .child(self.button("Edit selected cell", Control::EditCell, 24, self.can_edit_query_cell(cx), false, cx)))
                    .when(!self.connected, |toolbar| {
                        toolbar.child(self.button(
                            if self.document.is_some() { "Connect" } else { "Reconnect" },
                            Control::Reconnect,
                            3,
                            !self.connecting && !self.closing,
                            false,
                            cx,
                        ))
                    })
                    .child(crate::ui::grow())
                    .when(self.document.is_some(), |toolbar| {
                        let transaction_label = self.transaction_label();
                        let note = transaction_note(self.connected, self.transaction_pending, self.transaction.as_ref());
                        let actions = transaction_actions_visible(self.connected, self.transaction.as_ref());
                        toolbar.child(
                            div().id("transaction-controls").role(Role::Group).aria_label("Transaction controls")
                                .flex().items_center().gap(px(crate::style::S2))
                                .child(div().id("transaction-state").role(Role::Status).aria_label("Transaction state")
                                    .a11y_synthetic_children(move |builder| { builder.parent_node().set_value(transaction_label.clone()); })
                                    .when_some(note, |state, note| state.pr(px(crate::style::S2)).text_size(px(crate::style::FONT_SMALL)).text_color(crate::style::dim()).child(note)))
                                .child(self.transaction_select(TransactionMenu::Mode, cx))
                                .child(self.transaction_select(TransactionMenu::Isolation, cx))
                                .when(actions, |group| {
                                    group.children(Self::transaction_controls().into_iter().filter(|(_, control, _)| matches!(
                                        control,
                                        TransactionControl::Commit | TransactionControl::Rollback | TransactionControl::Recheck
                                    )).map(|(label, control, index)| {
                                        self.button(label, Control::Transaction(control), index, self.can_control_transaction(control), false, cx)
                                    }))
                                }),
                        )
                        .child(crate::ui::separator())
                    })
                    .child(
                        crate::ui::segment_group()
                            .id("layout-choices")
                            .role(Role::RadioGroup)
                            .aria_label("Layout")
                            .child(self.button(
                                "Stacked",
                                Control::Layout(Layout::Stacked),
                                4,
                                !self.closing,
                                self.layout == Layout::Stacked,
                                cx,
                            ))
                            .child(self.button(
                                "Side by side",
                                Control::Layout(Layout::SideBySide),
                                5,
                                !self.closing,
                                self.layout == Layout::SideBySide,
                                cx,
                            ))
                            .child(self.button(
                                "Results first",
                                Control::Layout(Layout::ResultsFirst),
                                6,
                                !self.closing,
                                self.layout == Layout::ResultsFirst,
                                cx,
                            )),
                    )
                    .when(self.layout == Layout::ResultsFirst, |toolbar| {
                        toolbar.child(self.button(
                            if self.expanded { "Compact editor" } else { "Expand editor" },
                            Control::Expand,
                            7,
                            !self.closing,
                            false,
                            cx,
                        ))
                    })
                    .child(keyboard_hint(KEYBOARD_HINTS))
                    .when(self.document.is_none(), |toolbar| {
                        toolbar
                            .child(crate::ui::separator())
                            .child(crate::ui::crumbs("", "dbunk_demo · 127.0.0.1:15432"))
                    }),
            )
            .when(self.document.is_some() && self.show_parameters, |pane| pane.child(self.parameters.clone()))
            .when_some(self.review.as_ref(), |pane, review| {
                let reviewed_sql = review.sql.clone();
                pane.child(
                    div().id("query-confirmation").role(Role::Group).aria_label("Safe Mode query review")
                        .flex().flex_col().flex_shrink_0().px(px(crate::style::INSET_BAR)).py(px(crate::style::S3)).gap(px(crate::style::S2)).text_sm()
                        .bg(crate::style::panel()).border_b_1().border_l_2().border_color(crate::style::warn())
                        .child(div().id("query-confirmation-announcement").role(Role::Alert)
                            .aria_label("Safe Mode requires confirmation. Review the exact SQL, then confirm or cancel. Query has not run.")
                            .flex().items_center().gap(px(crate::style::S3)).text_color(crate::style::text()).font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(gpui::svg().path("icons/warning.svg").size(px(crate::style::ICON)).flex_none().text_color(crate::style::warn()))
                            .child("Safe mode requires confirmation"))
                        .child(div().text_color(crate::style::dim()).child(review.classification.clone()))
                        .child(div().id("query-review-bindings").role(Role::Document).aria_label("Bound parameters and row limit")
                            .a11y_synthetic_children({ let value = review.bindings.clone(); move |builder| { builder.parent_node().set_value(value.clone()); } })
                            .max_h(px(120.)).overflow_y_scroll().font_family(crate::style::MONO).text_color(crate::style::dim()).child(review.bindings.clone()))
                        .child(div().id("query-review-sql").role(Role::Document).aria_label("SQL requiring confirmation")
                            .a11y_synthetic_children(move |builder| { builder.parent_node().set_value(reviewed_sql.clone()); })
                            .max_h(px(160.)).overflow_y_scroll().p(px(crate::style::S3)).rounded(px(crate::style::R_MD)).bg(crate::style::bg())
                            .border_1().border_color(crate::style::line_soft()).font_family(crate::style::MONO).text_size(px(crate::style::FONT_DATA)).child(review.sql.clone()))
                        .child(div().flex().gap(px(crate::style::S2))
                            .child(self.button("Cancel review", Control::CancelReview, 18, !self.closing, false, cx))
                            .child(self.button("Confirm and run", Control::ConfirmQuery, 19, !self.closing, false, cx)))
                )
            })
            // A new identity per failure also announces identical errors when
            // two executions finish before a cleared frame can be painted.
            .when_some(self.failure.as_ref(), |pane, failure| {
                pane.child(div().id(SharedString::from(format!("query-error-{}", failure.announcement_id))).role(Role::Alert)
                    .aria_label("Query error announcement")
                    .a11y_synthetic_children(move |builder| {
                        builder.parent_node().set_live(Live::Assertive);
                        builder.parent_node().set_value(announcement.clone());
                    }))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .when(side, |area| area.flex_row())
                    .when(!side, |area| area.flex_col())
                    .child(editor)
                    .child(results),
            )
            .when_some(self.transaction_error.as_ref(), |pane, message| {
                pane.child(div().flex_shrink_0().px(px(crate::style::INSET_BAR)).py(px(crate::style::S2)).border_t_1().border_color(crate::style::line_soft())
                    .child(crate::ui::shake(format!("transaction-error-shake-{message}"), crate::ui::error_banner("transaction-error", message.clone()))))
            })
            .when_some(self.query_changes.unavailable.clone(), |content, reason| content.child(div().id("query-editing-unavailable").role(Role::Status).aria_label(reason.clone()).px(px(crate::style::INSET_BAR)).py(px(crate::style::S2)).text_size(px(crate::style::FONT_SMALL)).text_color(crate::style::dim()).border_t_1().border_color(crate::style::line_soft()).child(reason)))
            .when_some(self.query_changes.view.clone(), |content, changes| content.child(div().id("query-change-scroll").max_h(px(260.)).overflow_y_scroll().child(changes)))
            .children(self.render_transaction_menu(cx))
    }
}

impl Drop for Workbench {
    fn drop(&mut self) {
        if let Some(budget) = &self.retained_budget {
            budget.set(budget.get().saturating_sub(self.retained_bytes));
        }
    }
}

fn review_bindings(
    parameters: Option<&[mailbox::ReviewParameter]>,
    row_limit: Option<i64>,
) -> String {
    let mut lines = vec![row_limit.map_or_else(
        || "Row limit: none".into(),
        |limit| format!("Row limit: {limit}"),
    )];
    match parameters {
        None => lines.push("Named parameters: off".into()),
        Some([]) => lines.push("Named parameters: on; no bindings".into()),
        Some(values) => lines.extend(values.iter().map(|value| {
            format!(
                ":{} = {}",
                value.name,
                value.value.as_ref().map_or_else(
                    || "NULL".into(),
                    |text| serde_json::to_string(text).expect("string serialization")
                )
            )
        })),
    }
    lines.join("\n")
}

#[cfg(test)]
mod binding_review_tests {
    use super::*;

    #[test]
    fn review_preserves_disabled_empty_null_text_and_bound_limit() {
        assert_eq!(
            review_bindings(None, None),
            "Row limit: none\nNamed parameters: off"
        );
        assert_eq!(
            review_bindings(Some(&[]), Some(7)),
            "Row limit: 7\nNamed parameters: on; no bindings"
        );
        let values = [
            mailbox::ReviewParameter {
                name: "null".into(),
                value: None,
            },
            mailbox::ReviewParameter {
                name: "empty".into(),
                value: Some(String::new()),
            },
            mailbox::ReviewParameter {
                name: "text".into(),
                value: Some("NULL\n'🧪".into()),
            },
        ];
        assert_eq!(
            review_bindings(Some(&values), Some(1)),
            "Row limit: 1\n:null = NULL\n:empty = \"\"\n:text = \"NULL\\n'🧪\""
        );
        assert!(!format!("{:?}", values).contains("🧪"));
    }
}

#[cfg(test)]
mod transaction_chrome_tests {
    use super::*;

    #[test]
    fn transaction_actions_show_in_manual_mode_or_when_state_needs_attention() {
        let idle = QueryTransactionSnapshot::default();
        assert!(!transaction_actions_visible(true, Some(&idle)));
        let manual = QueryTransactionSnapshot {
            mode: QueryTransactionMode::Manual,
            ..Default::default()
        };
        assert!(transaction_actions_visible(true, Some(&manual)));
        for status in [
            QueryTransactionStatus::Active,
            QueryTransactionStatus::Failed,
            QueryTransactionStatus::Unknown,
        ] {
            let snapshot = QueryTransactionSnapshot {
                status,
                ..Default::default()
            };
            assert!(transaction_actions_visible(true, Some(&snapshot)));
        }
        // Recheck stays reachable before the first snapshot arrives.
        assert!(transaction_actions_visible(true, None));
        assert!(!transaction_actions_visible(false, None));
    }

    #[test]
    fn transaction_note_is_silent_for_an_idle_direct_session() {
        let idle = QueryTransactionSnapshot::default();
        assert_eq!(transaction_note(true, false, Some(&idle)), None);
        assert_eq!(transaction_note(false, false, Some(&idle)), None);
        assert_eq!(
            transaction_note(true, true, Some(&idle)).as_deref(),
            Some("Updating…")
        );
        assert_eq!(
            transaction_note(true, false, None).as_deref(),
            Some("State unknown")
        );
        let failed = QueryTransactionSnapshot {
            status: QueryTransactionStatus::Failed,
            pooling: ConnectionPooling::TransactionPooler,
            ..Default::default()
        };
        assert_eq!(
            transaction_note(true, false, Some(&failed)).as_deref(),
            Some("Failed; roll back · Transaction pooler")
        );
        let pooled = QueryTransactionSnapshot {
            pooling: ConnectionPooling::Pooler,
            ..Default::default()
        };
        assert_eq!(
            transaction_note(true, false, Some(&pooled)).as_deref(),
            Some("Connection pooler")
        );
    }

    #[test]
    fn each_transaction_select_lists_only_its_own_options() {
        let modes = Workbench::transaction_options(TransactionMenu::Mode)
            .into_iter()
            .map(|(label, _)| label)
            .collect::<Vec<_>>();
        assert_eq!(modes, ["Autocommit", "Manual"]);
        let isolation = Workbench::transaction_options(TransactionMenu::Isolation)
            .into_iter()
            .map(|(label, _)| label)
            .collect::<Vec<_>>();
        assert_eq!(
            isolation,
            ["Read committed", "Repeatable read", "Serializable"]
        );
    }
}
