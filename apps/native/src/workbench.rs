//! One editor and result model, with three interchangeable pane arrangements.
use crate::{
    accessible_editor::AccessibleEditor,
    controller::{Command, Controls, Host},
    diagnostics::{self, ExecutedSql},
    grid::ResultGrid,
    mailbox::{self, Message},
    results::TerminalStatus,
    sql,
};
use dbunk_lib::backend::{self, AckPayload, ExecutePayload, ExecutionPayload, Layout, QueryEvent};
use editor::{Editor, EditorEvent};
use gpui::accesskit::{Action, Live};
use gpui::{
    Context, Entity, FocusHandle, Focusable, KeyDownEvent, Role, SharedString, Subscription, Task,
    Window, actions, div, prelude::*, px, rgb,
};
use language::Buffer;
use multi_buffer::MultiBufferOffset;
use std::sync::Arc;
use std::time::{Duration, Instant};

actions!(
    native,
    [
        RunStatement,
        RunScript,
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
}
struct Execution {
    id: String,
    started: Instant,
    terminal: Option<u64>,
    source: Option<ExecutedSql>,
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

pub struct Workbench {
    host: Arc<Host>,
    editor: Entity<Editor>,
    buffer: Entity<Buffer>,
    failure: Option<QueryFailure>,
    accessible: Entity<AccessibleEditor>,
    grid: Entity<ResultGrid>,
    layout: Layout,
    expanded: bool,
    owner: String,
    session: String,
    stream: crate::stream::Stream,
    controls: Option<Controls>,
    events: Option<Task<()>>,
    execution: Option<Execution>,
    connected: bool,
    connecting: bool,
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
        let language = sql::language(cx).expect("SQL grammar");
        let buffer = cx.new(|cx| {
            let mut buffer = Buffer::local(INITIAL_SQL, cx);
            buffer.set_language(Some(language), cx);
            buffer
        });
        let editor = cx.new(|cx| Editor::for_buffer(buffer.clone(), None, window, cx));
        diagnostics::install(&editor, cx);
        let editor_events = cx.subscribe(&editor, |this, _, event, cx| {
            if matches!(event, EditorEvent::BufferEdited) {
                // Even editing and undoing invalidates an in-flight source position.
                if let Some(execution) = &mut this.execution {
                    execution.source = None;
                }
                if let Some(failure) = &mut this.failure {
                    failure.location = None;
                }
                diagnostics::clear(&this.editor, &this.buffer, cx);
                cx.notify();
            }
        });
        let accessible = cx.new(|cx| AccessibleEditor::new(editor.clone(), "SQL editor", cx));
        let grid = cx.new(ResultGrid::new);
        let activation = cx.observe_window_activation(window, |this, window, _| {
            if let Some(controls) = &this.controls {
                controls.focus(window.is_window_active());
            }
        });
        Self {
            host,
            editor,
            buffer,
            failure: None,
            accessible,
            grid,
            layout,
            expanded: false,
            owner: String::new(),
            session: String::new(),
            stream: crate::stream::Stream::new(String::new(), String::new()),
            controls: None,
            events: None,
            execution: None,
            connected: false,
            connecting: false,
            closing: false,
            status: "Connecting".into(),
            toolbar_focus: (0..10).map(|_| cx.focus_handle()).collect(),
            previous_focus: None,
            result_focus: Vec::new(),
            show_notices: false,
            _activation: activation,
            _editor_events: editor_events,
        }
    }
    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.editor.focus_handle(cx), cx);
        self.connect(cx);
    }
    fn connect(&mut self, cx: &mut Context<Self>) {
        if self.closing || self.connecting {
            return;
        }
        self.failure = None;
        diagnostics::clear(&self.editor, &self.buffer, cx);
        self.connected = false;
        self.connecting = true;
        self.execution = None;
        self.owner = uuid::Uuid::new_v4().to_string();
        self.session = uuid::Uuid::new_v4().to_string();
        self.stream =
            crate::stream::Stream::new(self.session.clone(), self.host.backend.fixture().id);
        self.status = "Connecting to dbunk_demo".into();
        self.events.take();
        #[cfg(not(feature = "fixture-verification"))]
        let capacity = mailbox::QUEUE_CAPACITY;
        #[cfg(feature = "fixture-verification")]
        let capacity = crate::verification::capacity();
        let (sender, receiver) = mailbox::channel(capacity, mailbox::QUEUE_BYTES);
        match self
            .host
            .connect(self.owner.clone(), self.session.clone(), sender)
        {
            Ok(controls) => self.controls = Some(controls),
            Err(error) => {
                self.disconnected(error.into(), cx);
                cx.notify();
                return;
            }
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
        diagnostics::clear(&self.editor, &self.buffer, cx);
        self.connected = false;
        self.connecting = false;
        self.execution = None;
        self.stream.retire();
        self.status = format!("Disconnected: {message}");
        if let Some(controls) = &self.controls {
            controls.stop();
        }
    }
    fn consume(&mut self, message: Message, cx: &mut Context<Self>) {
        match message {
            Message::Ready => {
                self.connected = true;
                self.connecting = false;
                self.status = "Ready".into();
            }
            Message::Rejected { execution, message } => {
                if self
                    .execution
                    .as_ref()
                    .is_some_and(|current| current.id == execution)
                {
                    self.execution = None;
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
                    QueryEvent::SessionLost { reason } => {
                        self.disconnected(reason.clone(), cx);
                        return;
                    }
                    QueryEvent::SessionClosed => {
                        self.disconnected("Connection closed".into(), cx);
                        return;
                    }
                    QueryEvent::ExecutionCompleted { .. } => {
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
                let retain_more_rows = self
                    .grid
                    .update(cx, |grid, cx| grid.consume(envelope.event, cx));
                if completed && let Some(completion) = self.grid.read(cx).model().completion.clone()
                {
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
    fn run(&mut self, script: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.connected || self.execution.is_some() || self.closing {
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
        let source = ExecutedSql::new(text, range);
        let id = uuid::Uuid::new_v4().to_string();
        let payload = ExecutePayload {
            session_id: self.session.clone(),
            execution_id: id.clone(),
            sql,
            confirmed: false,
            parameters: None,
            row_limit: None,
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
                    self.grid.update(cx, |grid, cx| grid.begin(cx));
                    self.show_notices = false;
                    self.execution = Some(Execution {
                        id,
                        started: Instant::now(),
                        terminal: None,
                        source,
                    });
                    self.status = "Running".into();
                }
                Err(error) => self.status = error.into(),
            }
        }
        cx.notify();
    }
    fn stop(&mut self, cx: &mut Context<Self>) {
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
        if let Some(controls) = &self.controls
            && let Err(error) = controls.send(Command::Layout(layout))
        {
            self.status = error.into();
        }
        cx.notify();
    }
    fn switch_pane(&mut self, _: &SwitchPane, window: &mut Window, cx: &mut Context<Self>) {
        let focus = if self.grid.focus_handle(cx).contains_focused(window, cx) {
            self.editor.focus_handle(cx)
        } else {
            self.show_notices = false;
            self.grid.read(cx).pane_focus(cx)
        };
        window.focus(&focus, cx);
        cx.notify();
    }
    fn focus_toolbar(&mut self, _: &FocusToolbar, window: &mut Window, cx: &mut Context<Self>) {
        if self.grid.focus_handle(cx).contains_focused(window, cx) {
            self.previous_focus = Some(self.grid.focus_handle(cx));
        } else if self.editor.focus_handle(cx).contains_focused(window, cx) {
            self.previous_focus = Some(self.editor.focus_handle(cx));
        }
        let index = if self.connected && self.execution.is_none() && !self.closing {
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
        let mut controls = vec![4, 5, 6];
        if self.connected && self.execution.is_none() {
            controls.extend([0, 1]);
        }
        if self
            .execution
            .as_ref()
            .is_some_and(|execution| execution.terminal.is_none())
        {
            controls.push(2);
        }
        if !self.connected && !self.connecting {
            controls.push(3);
        }
        if self.layout == Layout::ResultsFirst {
            controls.push(7);
        }
        if self.failure.is_some() {
            controls.push(9);
        }
        let mut focus = controls
            .into_iter()
            .map(|index| self.toolbar_focus[index].clone())
            .collect::<Vec<_>>();
        focus.extend(self.result_focus.iter().cloned());
        focus.push(self.toolbar_focus[8].clone());
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
                self.grid.update(cx, |grid, cx| grid.set_active(index, cx));
                cx.notify();
            }
            Control::Notices => {
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
            Control::Layout(_) => Role::RadioButton,
            Control::Result(_) => Role::Tab,
            _ => Role::Button,
        };
        div()
            .id(SharedString::from(format!("{control:?}")))
            .role(role)
            .aria_label(label.clone())
            .when(
                matches!(control, Control::Layout(_) | Control::Notices),
                |button| button.aria_toggled(selected.into()),
            )
            .when(matches!(control, Control::Result(_)), |button| {
                button.aria_selected(selected)
            })
            .key_context("NativeToolbar")
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(enabled)
            .px_2()
            .py_1()
            .border_1()
            .border_color(if selected {
                rgb(0xffffff)
            } else {
                rgb(0x444444)
            })
            .text_color(if enabled {
                rgb(0xffffff)
            } else {
                rgb(0x888888)
            })
            .text_sm()
            .focus(|style| style.border_color(rgb(0xa9d8c5)))
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
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if enabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.activate(control, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(label)
    }
}
impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_run = self.connected && self.execution.is_none() && !self.closing;
        let can_stop = self
            .execution
            .as_ref()
            .is_some_and(|execution| execution.terminal.is_none())
            && !self.closing;
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
            .border_color(rgb(0x444444))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(28.))
                    .flex_shrink_0()
                    .px_2()
                    .child("Query 1")
                    .when(self.layout == Layout::ResultsFirst, |pane| {
                        pane.child(self.button(
                            if self.expanded {
                                "Compact editor"
                            } else {
                                "Expand editor"
                            },
                            Control::Expand,
                            7,
                            !self.closing,
                            false,
                            cx,
                        ))
                    }),
            )
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
        let results = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .when_some(self.failure.as_ref(), |pane, failure| {
                pane.child(
                    div()
                        .id("query-failure")
                        .role(Role::Group)
                        .aria_label("Query failed")
                        .flex_shrink_0()
                        .max_h(px(180.))
                        .overflow_y_scroll()
                        .border_l_2()
                        .border_b_1()
                        .border_color(rgb(0xf87171))
                        .p_2()
                        .child(div().text_color(rgb(0xf87171)).child(
                            failure.code.as_ref().map_or("Query failed".into(), |code| {
                                format!("Query failed · {code}")
                            }),
                        ))
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
                                    .text_sm()
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
                        )),
                )
            })
            .child(
                div()
                    .id("result-tabs")
                    .overflow_x_scroll()
                    .flex()
                    .gap_2()
                    .p_1()
                    .flex_shrink_0()
                    .children(tabs)
                    .child(self.button(
                        format!("Notices {notices_count}"),
                        Control::Notices,
                        8,
                        true,
                        self.show_notices,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("result-content")
                    .flex_1()
                    .min_h_0()
                    .when(!self.show_notices, |pane| pane.child(self.grid.clone()))
                    .when(self.show_notices, |pane| {
                        pane.overflow_y_scroll()
                            .p_2()
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
                        .px_2()
                        .text_color(rgb(0xefd592))
                        .children(diagnostics.into_iter().enumerate().map(|(index, message)| {
                            div()
                                .id(("query-diagnostic", index))
                                .role(Role::Label)
                                .aria_label(message.clone())
                                .child(message)
                        })),
                )
            });
        let status = self.status.clone();
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
            .bg(rgb(0x000000))
            .text_color(rgb(0xffffff))
            .on_action(cx.listener(|this, _: &RunStatement, window, cx| this.run(false, window, cx)))
            .on_action(cx.listener(|this, _: &RunScript, window, cx| this.run(true, window, cx)))
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
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(rgb(0x333333))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(180.))
                            .text_sm()
                            .child("dbunk_demo · 127.0.0.1:15432"),
                    )
                    .child("Layout")
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
                    ))
                    .child(self.button("Run", Control::Run, 0, can_run, false, cx))
                    .child(self.button("Run script", Control::Script, 1, can_run, false, cx))
                    .child(self.button("Stop", Control::Stop, 2, can_stop, false, cx))
                    .when(!self.connected, |toolbar| {
                        toolbar.child(self.button(
                            "Reconnect",
                            Control::Reconnect,
                            3,
                            !self.connecting && !self.closing,
                            false,
                            cx,
                        ))
                    }),
            )
            .child(
                div().text_sm().px_2().py_1().flex_shrink_0()
                    .child("F8: controls · Tab: next control · Enter: activate · Escape: return · F6: SQL / results")
            )
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
            .child(
                div()
                    .id("query-status")
                    .role(Role::Status)
                    .aria_label("Query status")
                    .a11y_synthetic_children(move |builder| {
                        builder.parent_node().set_value(status.clone())
                    })
                    .px_2()
                    .py_1()
                    .border_t_1()
                    .border_color(rgb(0x333333))
                    .flex_shrink_0()
                    .text_sm()
                    .child(self.status.clone()),
            )
    }
}
