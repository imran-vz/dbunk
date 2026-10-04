//! One ClickHouse document: a query tab, a table's data, or its structure.
//! Each document owns at most one request at a time; replacing, cancelling or
//! closing aborts it. Results are bounded by the backend and shown in the
//! shared read-only grid. Documents are not persisted across launches.
use super::{
    document_model::{self, PAGE_ROWS, Paging},
    sessions::{ClickHouseSessions, SessionsChanged},
    tree_model::ObjectKind,
};
use crate::{
    accessible_editor::AccessibleEditor,
    controller::Host,
    document_view::ConnectionPhase,
    grid::{GridEvent, ResultGrid},
    sql, style, ui,
    workbench::{RunStatement, StopQuery},
};
use dbunk_lib::backend::{
    clickhouse::{
        ClickHouseError, ClickHouseErrorKind, ClickHouseQueryOutcome, ClickHouseRows,
        ClickHouseSession, ClickHouseStructure, StatementClassSummary,
    },
    select_sql_range,
};
use editor::Editor;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, Role, SharedString, Subscription, Task,
    Window, div, prelude::*, px,
};
use language::Buffer;
use multi_buffer::MultiBufferOffset;
use std::{cell::Cell, rc::Rc, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Query,
    Data {
        database: String,
        name: String,
        kind: ObjectKind,
    },
    Structure {
        database: String,
        name: String,
        kind: ObjectKind,
    },
}

impl Mode {
    /// The object a data or structure document shows.
    pub fn object(&self) -> Option<(&str, &str, ObjectKind)> {
        match self {
            Self::Query => None,
            Self::Data {
                database,
                name,
                kind,
            }
            | Self::Structure {
                database,
                name,
                kind,
            } => Some((database, name, *kind)),
        }
    }
}

pub enum DocumentEvent {
    /// Open the other view (data ↔ structure) of this document's object.
    Open { structure: bool },
    /// A statement completed in this many milliseconds.
    Latency(u64),
}

struct Run {
    query_id: Option<String>,
    session: ClickHouseSession,
    abort: tokio::task::AbortHandle,
    _task: Task<()>,
}

/// A statement waiting for the user's explicit confirmation.
struct Confirmation {
    sql: String,
    statements: Vec<StatementClassSummary>,
}

pub struct ClickHouseDocument {
    host: Arc<Host>,
    sessions: Entity<ClickHouseSessions>,
    connection: String,
    mode: Mode,
    editor: Option<(Entity<Editor>, Entity<AccessibleEditor>)>,
    grid: Entity<ResultGrid>,
    paging: Paging,
    has_more: bool,
    structure: Option<Rc<ClickHouseStructure>>,
    /// The session the last data/structure load used. A different session
    /// (a reconnect) loads again; the same one never reloads on its own.
    attempted: Option<ClickHouseSession>,
    run: Option<Run>,
    next_request: u64,
    confirmation: Option<Confirmation>,
    status: String,
    error: Option<String>,
    error_seq: u64,
    focus: FocusHandle,
    _grid_events: Subscription,
    _sessions: Subscription,
}
impl EventEmitter<DocumentEvent> for ClickHouseDocument {}

impl Drop for ClickHouseDocument {
    fn drop(&mut self) {
        if let Some(run) = self.run.take() {
            run.abort.abort();
        }
    }
}

impl ClickHouseDocument {
    pub fn new(
        host: Arc<Host>,
        sessions: Entity<ClickHouseSessions>,
        connection: String,
        mode: Mode,
        budget: Rc<Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = (mode == Mode::Query).then(|| {
            let language = sql::language(cx).expect("SQL grammar");
            let buffer = cx.new(|cx| {
                let mut buffer = Buffer::local("", cx);
                buffer.set_language(Some(language), cx);
                buffer
            });
            let editor = cx.new(|cx| {
                let mut editor = Editor::for_buffer(buffer, None, window, cx);
                editor.set_placeholder_text("SELECT … (one statement per run)", window, cx);
                editor
            });
            let accessible =
                cx.new(|cx| AccessibleEditor::new(editor.clone(), "ClickHouse SQL editor", cx));
            (editor, accessible)
        });
        let grid = cx.new(|cx| match mode.object() {
            Some((database, name, _)) => ResultGrid::new_table(database.into(), name.into(), cx),
            None => ResultGrid::new(cx),
        });
        grid.update(cx, |grid, _| grid.set_inspection_budget(budget));
        let grid_events = cx.subscribe(&grid, |this, _, event: &GridEvent, cx| {
            if let GridEvent::Sort { column, .. } = event
                && matches!(this.mode, Mode::Data { .. })
            {
                this.paging.sort_by(column);
                this.load(cx);
            }
        });
        let session_events = cx.subscribe(&sessions, |this, _, _: &SessionsChanged, cx| {
            this.session_changed(cx);
        });
        let mut document = Self {
            host,
            sessions,
            connection,
            mode,
            editor,
            grid,
            paging: Paging::default(),
            has_more: false,
            structure: None,
            attempted: None,
            run: None,
            next_request: 0,
            confirmation: None,
            status: String::new(),
            error: None,
            error_seq: 0,
            focus: cx.focus_handle(),
            _grid_events: grid_events,
            _sessions: session_events,
        };
        document.status = match document.mode {
            Mode::Query => "Cmd-Enter runs the statement at the cursor".into(),
            _ => String::new(),
        };
        document.load(cx);
        document
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }
    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn title(&self) -> String {
        match &self.mode {
            Mode::Query => "ClickHouse query".into(),
            Mode::Data { name, .. } => name.clone(),
            Mode::Structure { name, .. } => format!("{name} structure"),
        }
    }

    pub fn connection_phase(&self, cx: &gpui::App) -> ConnectionPhase {
        self.sessions.read(cx).phase(&self.connection)
    }

    pub fn focus_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.editor {
            Some((editor, _)) => window.focus(&editor.focus_handle(cx), cx),
            None if matches!(self.mode, Mode::Data { .. }) => {
                window.focus(&self.grid.read(cx).pane_focus(cx), cx)
            }
            None => window.focus(&self.focus, cx),
        }
    }

    pub fn clear_results(&mut self, cx: &mut Context<Self>) {
        if self.mode == Mode::Query {
            self.grid.update(cx, |grid, cx| grid.begin(cx));
            self.error = None;
            self.status.clear();
            cx.notify();
        }
    }

    /// Data and structure documents load once their session is open; a new
    /// session after a reconnect reloads them. Query documents never rerun.
    fn session_changed(&mut self, cx: &mut Context<Self>) {
        let phase = self.connection_phase(cx);
        if let Some(run) = &self.run
            && self
                .sessions
                .read(cx)
                .session(&self.connection)
                .is_none_or(|current| !current.same(&run.session))
        {
            self.stop_run(false);
            self.status = "Stopped: the session ended".into();
        }
        let current = self.sessions.read(cx).session(&self.connection);
        let fresh = match (&current, &self.attempted) {
            (Some(current), Some(attempted)) => !current.same(attempted),
            (Some(_), None) => true,
            (None, _) => false,
        };
        if self.mode != Mode::Query
            && phase == ConnectionPhase::Connected
            && fresh
            && self.run.is_none()
        {
            self.load(cx);
        }
        cx.notify();
    }

    fn session(&mut self, cx: &mut Context<Self>) -> Option<ClickHouseSession> {
        let session = self.sessions.read(cx).session(&self.connection);
        if session.is_none() {
            self.status = match self.connection_phase(cx) {
                ConnectionPhase::Connecting => "Connecting…".into(),
                _ => "Not connected. Connect from the sidebar to load.".into(),
            };
            cx.notify();
        }
        session
    }

    fn stop_run(&mut self, kill: bool) {
        if let Some(run) = self.run.take() {
            run.abort.abort();
            if kill && let Some(query_id) = run.query_id {
                let session = run.session;
                self.host
                    .runtime
                    .spawn(async move { session.cancel(&query_id).await });
            }
        }
    }

    fn fail(
        &mut self,
        session: &ClickHouseSession,
        error: ClickHouseError,
        cx: &mut Context<Self>,
    ) {
        if error.kind == ClickHouseErrorKind::Lost {
            let message = error.message.clone();
            let connection = self.connection.clone();
            self.sessions.update(cx, |sessions, cx| {
                sessions.lost(&connection, session, message, cx)
            });
        }
        self.error = Some(document_model::error_text(&error));
        self.error_seq += 1;
        self.status = "Failed".into();
    }

    /// (Re)loads a data page or the structure. Query documents run on demand.
    fn load(&mut self, cx: &mut Context<Self>) {
        let Some((database, name, _)) = self.mode.object() else {
            return;
        };
        let (database, name) = (database.to_owned(), name.to_owned());
        let Some(session) = self.session(cx) else {
            return;
        };
        self.stop_run(false);
        self.error = None;
        self.attempted = Some(session.clone());
        self.next_request += 1;
        let request = self.next_request;
        let reader = session.clone();
        match self.mode {
            Mode::Data { .. } => {
                let paging = self.paging.clone();
                self.status = "Loading…".into();
                let read = self.host.runtime.spawn(async move {
                    let order = paging
                        .sort
                        .as_ref()
                        .map(|(column, desc)| (column.as_str(), *desc));
                    reader
                        .browse(&database, &name, order, paging.offset, PAGE_ROWS + 1)
                        .await
                });
                let abort = read.abort_handle();
                let task = cx.spawn(async move |this, cx| {
                    let result = read.await;
                    this.update(cx, |this, cx| {
                        if this.next_request != request {
                            return;
                        }
                        let session = this.run.take().map(|run| run.session);
                        match (result, session) {
                            (Ok(Ok(rows)), _) => this.show_page(rows, request, cx),
                            (Ok(Err(error)), Some(session)) => this.fail(&session, error, cx),
                            _ => this.status = "Stopped".into(),
                        }
                        cx.notify();
                    })
                    .ok();
                });
                self.run = Some(Run {
                    query_id: None,
                    session,
                    abort,
                    _task: task,
                });
            }
            Mode::Structure { .. } => {
                self.status = "Reading structure…".into();
                let read = self
                    .host
                    .runtime
                    .spawn(async move { reader.structure(&database, &name).await });
                let abort = read.abort_handle();
                let task = cx.spawn(async move |this, cx| {
                    let result = read.await;
                    this.update(cx, |this, cx| {
                        if this.next_request != request {
                            return;
                        }
                        let session = this.run.take().map(|run| run.session);
                        match (result, session) {
                            (Ok(Ok(structure)), _) => {
                                this.status = format!(
                                    "{} columns · {}",
                                    structure.columns.len(),
                                    structure.engine
                                );
                                this.structure = Some(Rc::new(structure));
                            }
                            (Ok(Err(error)), Some(session)) => this.fail(&session, error, cx),
                            _ => this.status = "Stopped".into(),
                        }
                        cx.notify();
                    })
                    .ok();
                });
                self.run = Some(Run {
                    query_id: None,
                    session,
                    abort,
                    _task: task,
                });
            }
            Mode::Query => {}
        }
        cx.notify();
    }

    fn show_page(&mut self, rows: ClickHouseRows, request: u64, cx: &mut Context<Self>) {
        let (rows, has_more) = document_model::take_page(rows);
        self.has_more = has_more;
        self.status =
            document_model::page_summary(&self.paging, rows.rows.len(), has_more, rows.runtime_ms);
        let page = document_model::grid_page(
            rows,
            request,
            u32::try_from(self.paging.page()).ok(),
            has_more,
            String::new(),
        );
        self.grid
            .update(cx, |grid, cx| grid.table_page(Rc::new(page), cx));
    }

    fn statement(&self, cx: &mut Context<Self>) -> Result<String, String> {
        let Some((editor, _)) = &self.editor else {
            return Err("Not a query document".into());
        };
        editor.update(cx, |editor, cx| {
            let selection = editor
                .selections
                .newest::<MultiBufferOffset>(&editor.display_snapshot(cx));
            let text = editor.text(cx);
            match select_sql_range(&text, &(selection.start.0..selection.end.0), false) {
                Ok(Some(range)) => Ok(text[range].to_owned()),
                Ok(None) => Err("No SQL statement at the cursor".into()),
                Err(_) => Err("SQL cannot be parsed; select the statement to run".into()),
            }
        })
    }

    fn run_statement(&mut self, cx: &mut Context<Self>) {
        if self.mode != Mode::Query || self.run.is_some() {
            return;
        }
        match self.statement(cx) {
            Ok(sql) => self.execute(sql, false, cx),
            Err(message) => {
                self.status = message;
                cx.notify();
            }
        }
    }

    fn execute(&mut self, sql: String, confirmed: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session(cx) else {
            return;
        };
        self.confirmation = None;
        self.error = None;
        self.next_request += 1;
        let request = self.next_request;
        let query_id = uuid::Uuid::new_v4().to_string();
        let reader = session.clone();
        let id = query_id.clone();
        let statement = sql.clone();
        let read = self
            .host
            .runtime
            .spawn(async move { reader.query(&statement, confirmed, &id).await });
        let abort = read.abort_handle();
        let task = cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                if this.next_request != request {
                    return;
                }
                let session = this.run.take().map(|run| run.session);
                match (result, session) {
                    (Ok(Ok(ClickHouseQueryOutcome::Rows(rows))), _) => {
                        this.status = document_model::query_summary(&rows);
                        cx.emit(DocumentEvent::Latency(rows.runtime_ms));
                        let page = document_model::grid_page(rows, request, None, false, sql);
                        this.grid
                            .update(cx, |grid, cx| grid.table_page(Rc::new(page), cx));
                    }
                    (Ok(Ok(ClickHouseQueryOutcome::NeedsConfirmation(statements))), _) => {
                        this.status = "Confirmation required".into();
                        this.confirmation = Some(Confirmation { sql, statements });
                    }
                    (Ok(Err(error)), Some(session)) => this.fail(&session, error, cx),
                    _ => this.status = "Stopped".into(),
                }
                cx.notify();
            })
            .ok();
        });
        self.status = "Running…".into();
        self.run = Some(Run {
            query_id: Some(query_id),
            session,
            abort,
            _task: task,
        });
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if self.run.is_some() {
            self.next_request += 1;
            self.stop_run(true);
            self.status = "Stopped".into();
            cx.notify();
        }
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        let connection = self.connection.clone();
        self.sessions
            .update(cx, |sessions, cx| sessions.connect(&connection, cx));
    }

    fn page(&mut self, next: bool, cx: &mut Context<Self>) {
        let moved = if next {
            self.paging.next(self.has_more)
        } else {
            self.paging.previous()
        };
        if moved {
            self.load(cx);
        }
    }

    fn toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let running = self.run.is_some();
        let phase = self.connection_phase(cx);
        let connected = phase == ConnectionPhase::Connected;
        let mut bar = ui::toolbar();
        match &self.mode {
            Mode::Query => {
                bar = bar
                    .child(
                        ui::tool_button(
                            "clickhouse-run",
                            "Run",
                            Some("icons/play_filled.svg"),
                            connected && !running,
                            true,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.run_statement(cx)))
                        .tooltip(ui::tooltip("Run statement at cursor (Cmd-Enter)"))
                        .tooltip_show_delay(ui::tooltip_delay()),
                    )
                    .child(
                        ui::tool_button(
                            "clickhouse-stop",
                            "Stop",
                            Some("icons/stop.svg"),
                            running,
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
                        .tooltip(ui::tooltip("Stop (Cmd-.)"))
                        .tooltip_show_delay(ui::tooltip_delay()),
                    );
            }
            Mode::Data { .. } => {
                bar = bar
                    .child(
                        ui::tool_button(
                            "clickhouse-refresh",
                            "Refresh",
                            Some("icons/rotate_cw.svg"),
                            connected && !running,
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                    )
                    .child(
                        ui::tool_button(
                            "clickhouse-previous",
                            "Previous page",
                            None,
                            connected && !running && self.paging.offset > 0,
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.page(false, cx))),
                    )
                    .child(
                        ui::tool_button(
                            "clickhouse-next",
                            "Next page",
                            None,
                            connected && !running && self.has_more,
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.page(true, cx))),
                    )
                    .when_some(self.paging.sort.as_ref(), |bar, (column, desc)| {
                        bar.child(ui::badge(format!(
                            "{column} {}",
                            if *desc { "desc" } else { "asc" }
                        )))
                    })
                    .child(ui::separator())
                    .child(
                        ui::tool_button(
                            "clickhouse-open-structure",
                            "Structure",
                            Some("icons/list_tree.svg"),
                            true,
                            false,
                        )
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(DocumentEvent::Open { structure: true })
                        })),
                    );
            }
            Mode::Structure { .. } => {
                bar = bar
                    .child(
                        ui::tool_button(
                            "clickhouse-refresh",
                            "Refresh",
                            Some("icons/rotate_cw.svg"),
                            connected && !running,
                            false,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                    )
                    .child(
                        ui::tool_button(
                            "clickhouse-open-data",
                            "Data",
                            Some("icons/table.svg"),
                            true,
                            false,
                        )
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(DocumentEvent::Open { structure: false })
                        })),
                    );
            }
        }
        if !connected {
            bar = bar.child(ui::separator()).child(
                ui::tool_button(
                    "clickhouse-connect",
                    if phase == ConnectionPhase::Connecting {
                        "Connecting…"
                    } else {
                        "Connect"
                    },
                    Some("icons/power.svg"),
                    matches!(phase, ConnectionPhase::Idle | ConnectionPhase::Failed(_)),
                    false,
                )
                .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
            );
        }
        let crumbs = match self.mode.object() {
            Some((database, name, kind)) => ui::crumbs(
                format!("{database} ·"),
                format!("{name} ({})", kind.label()),
            ),
            None => ui::crumbs("ClickHouse", "query"),
        };
        bar.child(ui::grow()).child(crumbs)
    }

    fn structure_view(structure: &ClickHouseStructure) -> gpui::AnyElement {
        let mono = |text: String| {
            div()
                .font_family(style::MONO)
                .text_color(style::text())
                .child(text)
        };
        let field = |label: &'static str, value: Option<String>| {
            value.map(|value| {
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .text_color(style::dim())
                            .child(label),
                    )
                    .child(mono(value))
            })
        };
        let mut summary = vec![structure.engine.clone()];
        if let Some(rows) = structure.total_rows {
            summary.push(format!("{rows} rows"));
        }
        if let Some(bytes) = structure.total_bytes {
            summary.push(document_model::bytes(bytes));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(ui::section_label("Table"))
                    .children(field("Engine", Some(summary.join(" · "))))
                    .children(field(
                        "Sorting key",
                        (!structure.sorting_key.is_empty())
                            .then(|| structure.sorting_key.join(", ")),
                    ))
                    .children(field("Partition by", structure.partition_by.clone()))
                    .children(field("Sample by", structure.sample_by.clone())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(ui::section_label(format!(
                        "Columns {}",
                        structure.columns.len()
                    )))
                    .children(structure.columns.iter().map(|column| {
                        div()
                            .h(px(style::ROW))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .border_b_1()
                            .border_color(style::line_soft())
                            .child(
                                div()
                                    .w(px(12.))
                                    .flex_none()
                                    .text_color(style::warn())
                                    .child(if column.in_sorting_key { "K" } else { "" }),
                            )
                            .child(
                                div()
                                    .w(px(200.))
                                    .flex_none()
                                    .overflow_hidden()
                                    .child(column.name.clone()),
                            )
                            .child(
                                div()
                                    .w(px(220.))
                                    .flex_none()
                                    .overflow_hidden()
                                    .font_family(style::MONO)
                                    .text_color(style::number())
                                    .child(column.type_name.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .font_family(style::MONO)
                                    .text_color(style::faint())
                                    .child(column.default.clone().unwrap_or_default()),
                            )
                    })),
            )
            .when(!structure.skip_indexes.is_empty(), |root| {
                root.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(ui::section_label(format!(
                            "Skip indexes {}",
                            structure.skip_indexes.len()
                        )))
                        .children(structure.skip_indexes.iter().map(|index| {
                            mono(format!(
                                "{}  {}  ({})",
                                index.name, index.expression, index.kind
                            ))
                        })),
                )
            })
            .when(!structure.constraints.is_empty(), |root| {
                root.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(ui::section_label("Constraints"))
                        .children(
                            structure
                                .constraints
                                .iter()
                                .map(|(name, check)| mono(format!("{name}  CHECK {check}"))),
                        ),
                )
            })
            .when(!structure.ddl.is_empty(), |root| {
                root.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(ui::section_label("DDL"))
                        .child(
                            div()
                                .p(px(8.))
                                .rounded(px(5.))
                                .bg(style::panel())
                                .border_1()
                                .border_color(style::line_soft())
                                .child(mono(structure.ddl.clone())),
                        ),
                )
            })
            .into_any_element()
    }
}

impl Render for ClickHouseDocument {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body: gpui::AnyElement = match &self.mode {
            Mode::Query => {
                let editor = self
                    .editor
                    .as_ref()
                    .map(|(_, accessible)| accessible.clone());
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(gpui::relative(0.38))
                            .flex_none()
                            .p(px(6.))
                            .border_b_1()
                            .border_color(style::line())
                            .font_family(style::MONO)
                            .children(editor),
                    )
                    .child(div().flex_1().min_h_0().child(self.grid.clone()))
                    .into_any_element()
            }
            Mode::Data { .. } => div()
                .flex_1()
                .min_h_0()
                .child(self.grid.clone())
                .into_any_element(),
            Mode::Structure { .. } => div()
                .id("clickhouse-structure-scroll")
                .track_focus(&self.focus)
                .tab_stop(true)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .children(self.structure.as_deref().map(Self::structure_view))
                .into_any_element(),
        };
        div()
            .id("clickhouse-document")
            .role(Role::Group)
            .aria_label(SharedString::from(self.title()))
            .key_context("ClickHouseDocument")
            .on_action(cx.listener(|this, _: &RunStatement, _, cx| this.run_statement(cx)))
            .on_action(cx.listener(|this, _: &StopQuery, _, cx| this.stop(cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(style::bg())
            .text_size(px(style::FONT))
            .child(self.toolbar(cx))
            .when_some(self.confirmation.as_ref(), |root, confirmation| {
                let classes = confirmation
                    .statements
                    .iter()
                    .map(|statement| format!("{:?}", statement.class).to_lowercase())
                    .collect::<Vec<_>>()
                    .join(", ");
                root.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(5.))
                        .bg(style::bad_fill())
                        .border_b_1()
                        .border_color(style::bad_line())
                        .text_color(style::bad_text())
                        .child(div().flex_1().child(format!(
                            "This connection's safe mode asks before running a {classes} statement."
                        )))
                        .child(
                            ui::button(
                                "clickhouse-confirm",
                                "Run anyway",
                                ui::Variant::Danger,
                                true,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(confirmation) = this.confirmation.take() {
                                    this.execute(confirmation.sql, true, cx);
                                }
                            })),
                        )
                        .child(
                            ui::button(
                                "clickhouse-cancel-confirm",
                                "Cancel",
                                ui::Variant::Ghost,
                                true,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirmation = None;
                                this.status = "Not run".into();
                                cx.notify();
                            })),
                        ),
                )
            })
            .when_some(self.error.clone(), |root, error| {
                root.child(div().p(px(6.)).child(ui::shake(
                    ("clickhouse-error-shake", self.error_seq as usize),
                    ui::error_banner("clickhouse-error", error),
                )))
            })
            .child(body)
            .child(
                ui::status_line()
                    .id("clickhouse-status")
                    .role(Role::Status)
                    .aria_label(SharedString::from(self.status.clone()))
                    .child(self.status.clone()),
            )
    }
}
