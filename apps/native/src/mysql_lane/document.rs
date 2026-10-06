//! MySQL documents: query, table data, structure and object definition.
//! Each borrows the lane's session; with no session it keeps its content and
//! refuses new requests until the lane connects again. Queries and pages
//! carry a request id, so Stop and closing the tab cancel only this
//! document's request.
use super::model::{Load, ObjectRef};
use super::statements;
use crate::{
    accessible_editor::AccessibleEditor,
    controller::Host,
    grid::ResultGrid,
    style, ui,
    workbench::{RunScript, RunStatement, StopQuery},
};
use dbunk_lib::backend::QueryEvent;
use dbunk_lib::backend::StatementClassSummary;
use dbunk_lib::backend::mysql_sessions::{
    MYSQL_MAX_PAGE_ROWS, MySqlCancel, MySqlRequestId, MySqlResult, MySqlSession, MySqlSessionError,
    MySqlStructure,
};
use editor::Editor;
use gpui::{
    ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, Role, SharedString, Task,
    Window, div, prelude::*, px,
};
use language::Buffer;
use multi_buffer::MultiBufferOffset;
use std::sync::Arc;

/// Rows per data page.
const PAGE_ROWS: u32 = 200;
const _: () = assert!(PAGE_ROWS <= MYSQL_MAX_PAGE_ROWS);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocKind {
    Query,
    Data(ObjectRef),
    Structure(ObjectRef),
    Definition(ObjectRef),
}

impl DocKind {
    pub fn icon(&self) -> &'static str {
        match self {
            DocKind::Query => "icons/terminal.svg",
            DocKind::Data(_) => "icons/table.svg",
            DocKind::Structure(_) => "icons/list_tree.svg",
            DocKind::Definition(_) => "icons/code.svg",
        }
    }
}

pub enum DocEvent {
    Latency(u64),
    Structure(ObjectRef),
    Data(ObjectRef),
}

/// Data page navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Reload,
    Next,
    Previous,
}

struct Query {
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    database: Option<String>,
    databases: Vec<String>,
    picking: bool,
    /// Script waiting for an explicit policy confirmation.
    confirm: Option<(String, Vec<StatementClassSummary>)>,
}

pub struct MySqlDocument {
    host: Arc<Host>,
    kind: DocKind,
    session: Option<MySqlSession>,
    focus: FocusHandle,
    query: Option<Query>,
    grid: Entity<ResultGrid>,
    has_result: bool,
    /// First row of the shown page.
    offset: u64,
    /// Where the next page starts (after the rows actually kept), if any.
    next_offset: Option<u64>,
    /// Start offsets of the pages before this one, for Previous.
    history: Vec<u64>,
    structure: Load<MySqlStructure>,
    definition: Load<String>,
    status: String,
    error: Option<String>,
    running: Option<Task<()>>,
    /// The query or page request behind `running`.
    request: Option<MySqlRequestId>,
    cancel: Option<Task<()>>,
    loads: Vec<Task<()>>,
}

impl EventEmitter<DocEvent> for MySqlDocument {}

impl MySqlDocument {
    pub fn new(
        host: Arc<Host>,
        kind: DocKind,
        session: Option<MySqlSession>,
        database: Option<String>,
        databases: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = (kind == DocKind::Query).then(|| {
            let buffer = cx.new(|cx| {
                let mut buffer = Buffer::local("", cx);
                if let Ok(language) = crate::sql::language(cx) {
                    buffer.set_language(Some(language), cx);
                }
                buffer
            });
            let editor = cx.new(|cx| Editor::for_buffer(buffer, None, window, cx));
            let accessible =
                cx.new(|cx| AccessibleEditor::new(editor.clone(), "MySQL query editor", cx));
            Query {
                editor,
                accessible,
                database,
                databases,
                picking: false,
                confirm: None,
            }
        });
        let grid = cx.new(ResultGrid::new);
        grid.update(cx, |grid, _| grid.set_export_host(host.clone()));
        let mut document = Self {
            host,
            kind,
            session: None,
            focus: cx.focus_handle(),
            query,
            grid,
            has_result: false,
            offset: 0,
            next_offset: None,
            history: Vec::new(),
            structure: Load::Idle,
            definition: Load::Idle,
            status: String::new(),
            error: None,
            running: None,
            request: None,
            cancel: None,
            loads: Vec::new(),
        };
        document.set_session(session, true, cx);
        document
    }

    /// `None` stops requests. A new session loads anything not yet loaded
    /// only when `load` is set (the visible document); the others load when
    /// they are selected, so a reconnect does not flood the session queue.
    pub fn set_session(
        &mut self,
        session: Option<MySqlSession>,
        load: bool,
        cx: &mut Context<Self>,
    ) {
        let connected = session.is_some();
        self.session = session;
        if !connected {
            // Replies from the old session are dropped with their tasks.
            if self.running.take().is_some() {
                self.status = "Interrupted: disconnected".into();
            }
            self.request = None;
            self.cancel = None;
            self.loads.clear();
            if matches!(self.structure, Load::Loading) {
                self.structure = Load::Idle;
            }
            if matches!(self.definition, Load::Loading) {
                self.definition = Load::Idle;
            }
        } else if load {
            self.load_pending(cx);
        }
        cx.notify();
    }

    /// Loads what this document still lacks (after a reconnect, on select).
    pub fn load_pending(&mut self, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        match &self.kind {
            DocKind::Query => {}
            DocKind::Data(_) if !self.has_result && self.running.is_none() => {
                self.load_page(Step::Reload, cx)
            }
            DocKind::Data(_) => {}
            DocKind::Structure(_) | DocKind::Definition(_) => self.load_details(false, cx),
        }
    }

    /// The tab is closing: its queued or running request is cancelled, so
    /// it neither keeps the shared session busy nor runs after the tab is
    /// gone. Replies are dropped with the tasks.
    pub fn abandon(&mut self) {
        self.cancel = None;
        self.loads.clear();
        let request = self.request.take();
        if self.running.take().is_none() {
            return;
        }
        if let (Some(session), Some(id)) = (self.session.clone(), request) {
            let host = self.host.clone();
            drop(self.host.runtime.spawn(async move {
                let _ = host.backend.cancel_mysql_query(&session, id).await;
            }));
        }
    }

    /// Picker choices once the tree has loaded; an unset database takes the
    /// connection default when it is listed.
    pub fn set_databases(
        &mut self,
        databases: Vec<String>,
        default: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(query) = &mut self.query {
            if query.database.is_none() {
                query.database = default.filter(|default| databases.contains(default));
            }
            query.databases = databases;
            cx.notify();
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.query {
            Some(query) => window.focus(&query.editor.focus_handle(cx), cx),
            None => window.focus(&self.focus, cx),
        }
    }

    fn session(&mut self) -> Option<MySqlSession> {
        if self.session.is_none() {
            self.error = Some("Not connected. Connect from the sidebar first.".into());
        }
        self.session.clone()
    }

    /// Runs the selection or the statement at the cursor, or with `script`
    /// the whole editor. `confirmed` re-runs only the text the safety policy
    /// asked about; with nothing pending it does nothing.
    fn run(&mut self, confirmed: bool, script: bool, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        let Some(query) = &mut self.query else {
            return;
        };
        let pending = query.confirm.take();
        let editor = query.editor.clone();
        let database = query.database.clone();
        let sql = match (confirmed, pending) {
            (true, Some((sql, _))) => sql,
            // Never run the editor text as confirmed.
            (true, None) => return,
            (false, _) => match selected_sql(&editor, script, cx) {
                Ok(sql) => sql,
                Err(message) => {
                    self.error = Some(message);
                    cx.notify();
                    return;
                }
            },
        };
        let Some(session) = self.session() else {
            cx.notify();
            return;
        };
        self.error = None;
        self.status = "Running…".into();
        let pending = sql.clone();
        let id = session.request_id();
        self.request = Some(id);
        let task = self
            .host
            .runtime
            .spawn(async move { session.query(sql, database, confirmed, id).await });
        self.running = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.finished(pending, result, cx))
                .ok();
        }));
        cx.notify();
    }

    fn finished(
        &mut self,
        sql: String,
        result: Result<Result<MySqlResult, MySqlSessionError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        self.running = None;
        self.request = None;
        self.cancel = None;
        match result {
            Ok(Ok(result)) => {
                cx.emit(DocEvent::Latency(result.runtime_ms));
                self.status = result_summary(&result);
                // Follow a `USE` in the script, so the next run is pinned to
                // the database this tab is actually in.
                if let (Some(query), Some(database)) = (&mut self.query, &result.database) {
                    query.database = Some(database.clone());
                }
                self.show(&result, cx);
            }
            Ok(Err(MySqlSessionError::NeedsConfirmation(statements))) => {
                self.status = "Waiting for confirmation".into();
                if let Some(query) = &mut self.query {
                    query.confirm = Some((sql, statements));
                }
            }
            Ok(Err(MySqlSessionError::Cancelled)) => self.status = "Cancelled".into(),
            Ok(Err(error)) => {
                self.status = "Failed".into();
                self.error = Some(error.to_string());
            }
            Err(_) => {
                self.status = "Failed".into();
                self.error = Some("Request task ended unexpectedly".into());
            }
        }
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if self.running.is_none() || self.cancel.is_some() {
            return;
        }
        let (Some(session), Some(id)) = (self.session.clone(), self.request) else {
            return;
        };
        self.status = "Cancelling…".into();
        let host = self.host.clone();
        let task = self
            .host
            .runtime
            .spawn(async move { host.backend.cancel_mysql_query(&session, id).await });
        self.cancel = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.cancel = None;
                match result {
                    // It never ran and never will: stop waiting for it.
                    Ok(Ok(MySqlCancel::Withdrawn)) if this.request == Some(id) => {
                        this.running = None;
                        this.request = None;
                        this.status = "Cancelled".into();
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => this.error = Some(format!("Cancel failed: {error}")),
                    Err(_) => {
                        this.error = Some("Cancel failed: cancel task ended unexpectedly".into())
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Clears a query tab's result, status and error. A running query is
    /// left alone, since its result would refill the grid; data, structure
    /// and definition tabs have no results to clear.
    pub fn clear_results(&mut self, cx: &mut Context<Self>) {
        if self.query.is_none() || self.running.is_some() {
            return;
        }
        self.grid.update(cx, |grid, cx| grid.begin(cx));
        self.has_result = false;
        self.error = None;
        self.status.clear();
        cx.notify();
    }

    fn show(&mut self, result: &MySqlResult, cx: &mut Context<Self>) {
        self.has_result = true;
        self.grid.update(cx, |grid, cx| {
            grid.begin(cx);
            for event in grid_events(result) {
                grid.consume(event, cx);
            }
        });
    }

    fn load_page(&mut self, step: Step, cx: &mut Context<Self>) {
        let DocKind::Data(object) = &self.kind else {
            return;
        };
        if self.running.is_some() {
            return;
        }
        let offset = match step {
            Step::Reload => self.offset,
            Step::Next => match self.next_offset {
                Some(offset) => offset,
                None => return,
            },
            Step::Previous => self.history.last().copied().unwrap_or(0),
        };
        let (database, table) = (object.database.clone(), object.name.clone());
        let Some(session) = self.session() else {
            cx.notify();
            return;
        };
        self.error = None;
        self.status = "Loading…".into();
        let id = session.request_id();
        self.request = Some(id);
        let task = self
            .host
            .runtime
            .spawn(async move { session.browse(database, table, offset, PAGE_ROWS, id).await });
        self.running = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.running = None;
                this.request = None;
                match result {
                    Ok(Ok(page)) => {
                        cx.emit(DocEvent::Latency(page.runtime_ms));
                        match step {
                            Step::Next => this.history.push(this.offset),
                            Step::Previous => {
                                this.history.pop();
                            }
                            Step::Reload => {}
                        }
                        this.offset = offset;
                        this.next_offset = next_offset(offset, &page);
                        this.status = page_label(offset, &page);
                        this.show(&page, cx);
                    }
                    Ok(Err(MySqlSessionError::Cancelled)) => this.status = "Cancelled".into(),
                    Ok(Err(error)) => {
                        this.status = "Failed".into();
                        this.error = Some(error.to_string());
                    }
                    Err(_) => this.error = Some("Request task ended unexpectedly".into()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Structure (tables and views) and the `SHOW CREATE` text. `force`
    /// reloads values that already loaded.
    fn load_details(&mut self, force: bool, cx: &mut Context<Self>) {
        let (object, with_structure) = match &self.kind {
            DocKind::Structure(object) => (object.clone(), true),
            DocKind::Definition(object) => (object.clone(), false),
            _ => return,
        };
        fn pending<T>(force: bool, load: &Load<T>) -> bool {
            force || matches!(load, Load::Idle | Load::Failed(_))
        }
        let want_structure = with_structure && pending(force, &self.structure);
        let want_definition = pending(force, &self.definition);
        if !want_structure && !want_definition {
            return;
        }
        let Some(session) = self.session() else {
            cx.notify();
            return;
        };
        self.error = None;
        // Finished loads are dropped so the list stays bounded.
        self.loads.retain(|task| !task.is_ready());
        if want_structure {
            self.structure = Load::Loading;
            let session = session.clone();
            let (database, table) = (object.database.clone(), object.name.clone());
            let task = self
                .host
                .runtime
                .spawn(async move { session.structure(database, table).await });
            self.loads.push(cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    this.structure = match result {
                        Ok(Ok(structure)) => Load::Ready(structure),
                        Ok(Err(error)) => Load::Failed(error.to_string()),
                        Err(_) => Load::Failed("Request task ended unexpectedly".into()),
                    };
                    cx.notify();
                })
                .ok();
            }));
        }
        if want_definition {
            self.definition = Load::Loading;
            let task = self.host.runtime.spawn(async move {
                session
                    .definition(object.database, object.kind, object.name)
                    .await
            });
            self.loads.push(cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    this.definition = match result {
                        Ok(Ok(text)) => Load::Ready(text),
                        Ok(Err(error)) => Load::Failed(error.to_string()),
                        Err(_) => Load::Failed("Request task ended unexpectedly".into()),
                    };
                    cx.notify();
                })
                .ok();
            }));
        }
        cx.notify();
    }

    fn button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        icon: Option<&'static str>,
        enabled: bool,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        ui::tool_button(id, label, icon, enabled, false).when(enabled, |button| {
            button.on_click(cx.listener(move |this, _, window, cx| action(this, window, cx)))
        })
    }

    fn crumbs(&self) -> Option<impl IntoElement + use<>> {
        let object = match &self.kind {
            DocKind::Query => return None,
            DocKind::Data(object) | DocKind::Structure(object) | DocKind::Definition(object) => {
                object
            }
        };
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(ui::crumbs(
                    format!("{}.", object.database),
                    object.name.clone(),
                ))
                .child(ui::badge(super::kind_label(object.kind))),
        )
    }

    fn toolbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let connected = self.session.is_some();
        let running = self.running.is_some();
        let mut bar = ui::toolbar();
        match self.kind.clone() {
            DocKind::Query => {
                bar = bar
                    .child(
                        self.button(
                            "mysql-run",
                            "Run",
                            Some("icons/play_filled.svg"),
                            connected && !running,
                            cx,
                            |this, _, cx| this.run(false, false, cx),
                        )
                        .map(|run| ui::pressed(run, true)),
                    )
                    .child(ui::shortcut("⌘↵"))
                    .child(self.button(
                        "mysql-stop",
                        "Stop",
                        Some("icons/stop.svg"),
                        running && self.cancel.is_none(),
                        cx,
                        |this, _, cx| this.stop(cx),
                    ))
                    .child(ui::separator());
                let database = self
                    .query
                    .as_ref()
                    .and_then(|query| query.database.clone())
                    .unwrap_or_else(|| "no database".into());
                bar = bar.child(self.button(
                    "mysql-database",
                    format!("Database: {database}"),
                    Some("icons/database_zap.svg"),
                    !running,
                    cx,
                    |this, _, cx| {
                        if let Some(query) = &mut this.query {
                            query.picking = !query.picking;
                        }
                        cx.notify();
                    },
                ));
            }
            DocKind::Data(object) => {
                bar = bar
                    .children(self.crumbs())
                    .child(ui::separator())
                    .child(self.button(
                        "mysql-refresh",
                        "Refresh",
                        Some("icons/rotate_cw.svg"),
                        connected && !running,
                        cx,
                        |this, _, cx| this.load_page(Step::Reload, cx),
                    ))
                    .child(self.button(
                        "mysql-previous",
                        "Previous",
                        None,
                        connected && !running && !self.history.is_empty(),
                        cx,
                        |this, _, cx| this.load_page(Step::Previous, cx),
                    ))
                    .child(self.button(
                        "mysql-next",
                        "Next",
                        None,
                        connected && !running && self.next_offset.is_some(),
                        cx,
                        |this, _, cx| this.load_page(Step::Next, cx),
                    ))
                    .child(ui::grow())
                    .child(self.button(
                        "mysql-open-structure",
                        "Structure",
                        Some("icons/list_tree.svg"),
                        true,
                        cx,
                        move |_, _, cx| cx.emit(DocEvent::Structure(object.clone())),
                    ));
            }
            DocKind::Structure(object) => {
                bar = bar
                    .children(self.crumbs())
                    .child(ui::separator())
                    .child(self.button(
                        "mysql-refresh",
                        "Refresh",
                        Some("icons/rotate_cw.svg"),
                        connected,
                        cx,
                        |this, _, cx| this.load_details(true, cx),
                    ))
                    .child(ui::grow())
                    .child(self.button(
                        "mysql-open-data",
                        "Data",
                        Some("icons/table.svg"),
                        true,
                        cx,
                        move |_, _, cx| cx.emit(DocEvent::Data(object.clone())),
                    ));
            }
            DocKind::Definition(_) => {
                let text = match &self.definition {
                    Load::Ready(text) => Some(text.clone()),
                    _ => None,
                };
                bar = bar
                    .children(self.crumbs())
                    .child(ui::separator())
                    .child(self.button(
                        "mysql-refresh",
                        "Refresh",
                        Some("icons/rotate_cw.svg"),
                        connected,
                        cx,
                        |this, _, cx| this.load_details(true, cx),
                    ))
                    .child(self.button(
                        "mysql-copy",
                        "Copy",
                        Some("icons/copy.svg"),
                        text.is_some(),
                        cx,
                        move |_, _, cx| {
                            if let Some(text) = &text {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                            }
                        },
                    ));
            }
        }
        bar
    }

    fn picker(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let query = self.query.as_ref().filter(|query| query.picking)?;
        let current = query.database.clone();
        Some(
            div()
                .id("mysql-database-picker")
                .role(Role::List)
                .aria_label("Databases")
                .flex_none()
                .max_h(px(180.))
                .overflow_y_scroll()
                .mx(px(8.))
                .my(px(4.))
                .p(px(2.))
                .rounded(px(6.))
                .border_1()
                .border_color(style::line())
                .bg(style::panel())
                .when(query.databases.is_empty(), |list| {
                    list.child(
                        div()
                            .px(px(8.))
                            .h(px(style::ROW))
                            .text_color(style::faint())
                            .child("Databases appear after the tree loads"),
                    )
                })
                .children(query.databases.iter().enumerate().map(|(index, name)| {
                    let selected = current.as_ref() == Some(name);
                    let pick = name.clone();
                    div()
                        .id(("mysql-database-option", index))
                        .role(Role::ListItem)
                        .aria_label(name.clone())
                        .aria_selected(selected)
                        .h(px(style::ROW))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(px(4.))
                        .font_family(style::MONO)
                        .cursor_pointer()
                        .hover(|s| s.bg(style::hover()))
                        .when(selected, |row| row.bg(style::select()))
                        .child(name.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(query) = &mut this.query {
                                query.database = Some(pick.clone());
                                query.picking = false;
                            }
                            cx.notify();
                        }))
                })),
        )
    }

    fn confirmation(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (_, statements) = self.query.as_ref()?.confirm.as_ref()?;
        Some(
            div()
                .id("mysql-confirm")
                .role(Role::Alert)
                .aria_label(confirmation_text(statements))
                .flex_none()
                .mx(px(8.))
                .my(px(4.))
                .px(px(8.))
                .py(px(5.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(5.))
                .border_1()
                .border_color(style::warn())
                .bg(style::warn_fill())
                .text_color(style::text())
                .child(div().flex_1().child(confirmation_text(statements)))
                .child(
                    ui::button("mysql-confirm-run", "Run anyway", ui::Variant::Danger, true)
                        .on_click(cx.listener(|this, _, _, cx| this.run(true, false, cx))),
                )
                .child(
                    ui::button("mysql-confirm-cancel", "Cancel", ui::Variant::Ghost, true)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(query) = &mut this.query {
                                query.confirm = None;
                            }
                            this.status = "Not run".into();
                            cx.notify();
                        })),
                ),
        )
    }

    fn details(&self) -> impl IntoElement + use<> {
        let mut body = div()
            .id("mysql-details")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(10.));
        if matches!(self.kind, DocKind::Structure(_)) {
            body = match &self.structure {
                Load::Ready(structure) => body.children(structure_sections(structure)),
                Load::Failed(error) => {
                    body.child(ui::error_banner("mysql-structure-error", error.clone()))
                }
                Load::Loading => body.child(note("Loading structure…")),
                Load::Idle => body.child(note("Connect to load the structure")),
            };
        }
        let definition = match &self.definition {
            Load::Ready(text) => div()
                .p(px(8.))
                .rounded(px(5.))
                .border_1()
                .border_color(style::line_soft())
                .font_family(style::MONO)
                .text_color(style::text())
                .whitespace_normal()
                .child(text.clone())
                .into_any_element(),
            Load::Failed(error) => {
                ui::error_banner("mysql-definition-error", error.clone()).into_any_element()
            }
            Load::Loading => note("Loading definition…").into_any_element(),
            Load::Idle => note("Connect to load the definition").into_any_element(),
        };
        body.child(ui::section_label("Definition"))
            .child(definition)
    }
}

fn note(text: &'static str) -> impl IntoElement {
    div().text_color(style::faint()).child(text)
}

fn structure_sections(structure: &MySqlStructure) -> Vec<gpui::AnyElement> {
    let mut sections = Vec::new();
    let table = |rows: Vec<Vec<String>>| {
        div()
            .flex()
            .flex_col()
            .rounded(px(5.))
            .border_1()
            .border_color(style::line_soft())
            .children(rows.into_iter().enumerate().map(|(index, cells)| {
                div()
                    .h(px(style::ROW))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .font_family(style::MONO)
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(style::line_soft())
                    })
                    .when(index == 0, |row| row.text_color(style::faint()))
                    .children(cells.into_iter().enumerate().map(|(column, cell)| {
                        div()
                            .when(column == 0, |cell| {
                                cell.w(px(180.)).text_color(style::text())
                            })
                            .when(column > 0, |cell| cell.flex_1())
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(cell)
                    }))
            }))
    };
    sections
        .push(ui::section_label(format!("Columns {}", structure.columns.len())).into_any_element());
    let mut rows = vec![vec![
        "name".into(),
        "type".into(),
        "null".into(),
        "default".into(),
        "key".into(),
    ]];
    rows.extend(structure.columns.iter().map(|column| {
        vec![
            column.name.clone(),
            column.data_type.clone(),
            if column.nullable { "yes" } else { "no" }.into(),
            column.default_value.clone().unwrap_or_default(),
            [
                column.primary_key.then_some("primary"),
                column.generated.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" "),
        ]
    }));
    sections.push(table(rows).into_any_element());
    if !structure.indexes.is_empty() {
        sections.push(
            ui::section_label(format!("Indexes {}", structure.indexes.len())).into_any_element(),
        );
        let mut rows = vec![vec!["name".into(), "columns".into(), "kind".into()]];
        rows.extend(structure.indexes.iter().map(|index| {
            vec![
                index.name.clone(),
                index.columns.join(", "),
                [
                    if index.primary {
                        "primary"
                    } else if index.unique {
                        "unique"
                    } else {
                        "index"
                    },
                    index.method.as_deref().unwrap_or_default(),
                ]
                .join(" "),
            ]
        }));
        sections.push(table(rows).into_any_element());
    }
    if !structure.foreign_keys.is_empty() {
        sections.push(
            ui::section_label(format!("Foreign keys {}", structure.foreign_keys.len()))
                .into_any_element(),
        );
        let mut rows = vec![vec!["name".into(), "columns".into(), "references".into()]];
        rows.extend(structure.foreign_keys.iter().map(|key| {
            vec![
                key.name.clone(),
                key.columns.join(", "),
                format!(
                    "{}.{} ({}){}{}",
                    key.referenced_schema,
                    key.referenced_table,
                    key.referenced_columns.join(", "),
                    key.on_update
                        .as_ref()
                        .map(|rule| format!(" on update {rule}"))
                        .unwrap_or_default(),
                    key.on_delete
                        .as_ref()
                        .map(|rule| format!(" on delete {rule}"))
                        .unwrap_or_default()
                ),
            ]
        }));
        sections.push(table(rows).into_any_element());
    }
    if !structure.constraints.is_empty() {
        sections.push(
            ui::section_label(format!("Checks {}", structure.constraints.len())).into_any_element(),
        );
        let mut rows = vec![vec!["name".into(), "definition".into()]];
        rows.extend(
            structure
                .constraints
                .iter()
                .map(|check| vec![check.name.clone(), check.definition.clone()]),
        );
        sections.push(table(rows).into_any_element());
    }
    sections
}

/// The grid's event stream for one finished result.
pub fn grid_events(result: &MySqlResult) -> Vec<QueryEvent> {
    if result.columns.is_empty() {
        return Vec::new();
    }
    vec![
        QueryEvent::ResultSetStarted {
            result_set_index: 0,
            columns: result.columns.iter().cloned().map(Some).collect(),
        },
        QueryEvent::RowBatch {
            result_set_index: 0,
            rows: result.rows.clone(),
        },
        QueryEvent::ResultSetCompleted {
            result_set_index: 0,
            row_count: result.total_rows,
            partial: result.truncated,
            limit: None,
        },
    ]
}

pub fn result_summary(result: &MySqlResult) -> String {
    let mut parts = Vec::new();
    if result.columns.is_empty() {
        parts.push(format!(
            "{} row{} affected",
            result.rows_affected,
            if result.rows_affected == 1 { "" } else { "s" }
        ));
    } else if result.truncated {
        parts.push(format!(
            "showing {} of {} rows",
            result.rows.len(),
            result.total_rows
        ));
    } else {
        parts.push(format!(
            "{} row{}",
            result.total_rows,
            if result.total_rows == 1 { "" } else { "s" }
        ));
    }
    if result.truncated_cells > 0 {
        parts.push(format!("{} long values shortened", result.truncated_cells));
    }
    if result.result_sets > 1 {
        parts.push(format!(
            "{} statements, last result shown",
            result.result_sets
        ));
    }
    parts.push(format!("{} ms", result.runtime_ms));
    parts.join(" · ")
}

pub fn page_label(offset: u64, page: &MySqlResult) -> String {
    let shown = page.rows.len() as u64;
    let range = if shown == 0 {
        "no rows".to_owned()
    } else {
        format!("rows {}–{}", offset + 1, offset + shown)
    };
    let mut label = format!(
        "{range}{} · {} ms",
        if page.has_more {
            ", more available"
        } else {
            ""
        },
        page.runtime_ms
    );
    if page.truncated {
        label.push_str(" · page cut by the size limit");
    }
    if page.approximate {
        label.push_str(" · approximate order (no unique key)");
    }
    label
}

/// Where the next page starts: after the rows actually shown, since the
/// size limit can keep fewer than a full page. `None` when there is no next
/// page or no progress is possible.
pub fn next_offset(offset: u64, page: &MySqlResult) -> Option<u64> {
    let shown = page.rows.len() as u64;
    (page.has_more && shown > 0).then_some(offset + shown)
}

/// The editor's selection, else the statement at the cursor; with `script`
/// the whole text.
fn selected_sql(
    editor: &Entity<Editor>,
    script: bool,
    cx: &mut gpui::App,
) -> Result<String, String> {
    editor.update(cx, |editor, cx| {
        let selection = editor
            .selections
            .newest::<MultiBufferOffset>(&editor.display_snapshot(cx));
        let text = editor.text(cx);
        match statements::select(&text, selection.start.0..selection.end.0, script) {
            Some(range) => Ok(text[range].to_owned()),
            None if statements::split(&text).is_empty() => Err("Nothing to run".into()),
            None => Err("No SQL statement at the cursor".into()),
        }
    })
}

pub fn confirmation_text(statements: &[StatementClassSummary]) -> String {
    let writes = statements
        .iter()
        .filter(|statement| {
            !matches!(
                statement.class,
                dbunk_lib::backend::StatementClassKind::Read
            )
        })
        .count();
    format!(
        "This connection's safety policy requires confirmation for {writes} statement{} that may change data or schema.",
        if writes == 1 { "" } else { "s" }
    )
}

impl Render for MySqlDocument {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toolbar = self.toolbar(cx);
        let picker = self.picker(cx);
        let confirmation = self.confirmation(cx);
        let body = match &self.kind {
            DocKind::Query => {
                let editor = self
                    .query
                    .as_ref()
                    .map(|query| query.accessible.clone())
                    .expect("query documents own an editor");
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(gpui::relative(0.4))
                            .min_h(px(80.))
                            .flex_none()
                            .p(px(6.))
                            .border_b_1()
                            .border_color(style::line())
                            .font_family(style::MONO)
                            .child(editor),
                    )
                    .child(self.result_area())
                    .into_any_element()
            }
            DocKind::Data(_) => self.result_area().into_any_element(),
            DocKind::Structure(_) | DocKind::Definition(_) => self.details().into_any_element(),
        };
        div()
            .id("mysql-document")
            .key_context("MySqlDocument")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, _: &RunStatement, _, cx| this.run(false, false, cx)))
            .on_action(cx.listener(|this, _: &RunScript, _, cx| this.run(false, true, cx)))
            .on_action(cx.listener(|this, _: &StopQuery, _, cx| this.stop(cx)))
            .child(toolbar)
            .children(picker)
            .children(confirmation)
            .when_some(self.error.clone(), |root, error| {
                root.child(
                    ui::error_banner("mysql-document-error", error)
                        .flex_none()
                        .mx(px(8.))
                        .my(px(4.)),
                )
            })
            .child(body)
            .child(
                ui::status_line()
                    .id("mysql-document-status")
                    .role(Role::Status)
                    .aria_label(self.status.clone())
                    .child(self.status.clone())
                    .child(ui::grow())
                    .child(if self.session.is_some() {
                        "connected"
                    } else {
                        "not connected"
                    }),
            )
    }
}

impl MySqlDocument {
    fn result_area(&self) -> impl IntoElement + use<> {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .when(self.has_result, |area| area.child(self.grid.clone()))
            .when(!self.has_result, |area| {
                area.items_center().justify_center().child(note(
                    if matches!(self.kind, DocKind::Query) {
                        "Run a statement with ⌘↵"
                    } else {
                        "Rows appear here"
                    },
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(columns: &[&str], rows: usize) -> MySqlResult {
        MySqlResult {
            columns: columns.iter().map(|c| (*c).to_owned()).collect(),
            rows: (0..rows).map(|i| vec![Some(i.to_string())]).collect(),
            total_rows: rows as u64,
            runtime_ms: 4,
            result_sets: 1,
            ..Default::default()
        }
    }

    #[test]
    fn summaries_name_rows_writes_cuts_and_scripts() {
        assert_eq!(result_summary(&result(&["a"], 1)), "1 row · 4 ms");
        let mut cut = result(&["a"], 1000);
        cut.total_rows = 25_000;
        cut.truncated = true;
        cut.truncated_cells = 2;
        assert_eq!(
            result_summary(&cut),
            "showing 1000 of 25000 rows · 2 long values shortened · 4 ms"
        );
        let mut write = result(&[], 0);
        write.rows_affected = 3;
        write.result_sets = 2;
        assert_eq!(
            result_summary(&write),
            "3 rows affected · 2 statements, last result shown · 4 ms"
        );
    }

    #[test]
    fn grid_events_carry_nulls_and_partial_results() {
        assert!(grid_events(&result(&[], 0)).is_empty());
        let mut value = result(&["a", "b"], 0);
        value.rows = vec![vec![Some("1".into()), None]];
        value.total_rows = 9;
        value.truncated = true;
        let events = grid_events(&value);
        assert_eq!(events.len(), 3);
        assert!(matches!(
            &events[1],
            QueryEvent::RowBatch { rows, .. } if rows[0][1].is_none()
        ));
        assert!(matches!(
            events[2],
            QueryEvent::ResultSetCompleted {
                row_count: 9,
                partial: true,
                ..
            }
        ));
    }

    #[test]
    fn page_labels_are_one_based_and_flag_more_rows() {
        let mut page = result(&["a"], 200);
        page.has_more = true;
        assert_eq!(
            page_label(400, &page),
            "rows 401–600, more available · 4 ms"
        );
        assert_eq!(page_label(0, &result(&["a"], 0)), "no rows · 4 ms");
        let mut loose = result(&["a"], 2);
        loose.approximate = true;
        assert_eq!(
            page_label(0, &loose),
            "rows 1–2 · 4 ms · approximate order (no unique key)"
        );
    }

    #[test]
    fn the_next_page_starts_after_the_rows_actually_kept() {
        let mut full = result(&["a"], 200);
        full.has_more = true;
        assert_eq!(next_offset(400, &full), Some(600));
        // The 16 MiB budget kept 37 rows: the next page starts at row 38.
        let mut cut = result(&["a"], 37);
        cut.has_more = true;
        cut.truncated = true;
        assert_eq!(next_offset(0, &cut), Some(37));
        assert_eq!(next_offset(0, &result(&["a"], 12)), None);
        // Nothing kept: paging cannot progress, so there is no next page.
        let mut empty = result(&["a"], 0);
        empty.has_more = true;
        assert_eq!(next_offset(0, &empty), None);
    }

    #[test]
    fn confirmation_counts_only_statements_that_may_write() {
        use dbunk_lib::backend::StatementClassKind;
        let summary = |class| StatementClassSummary {
            index: 0,
            class,
            unbounded: false,
            destructive: false,
        };
        assert_eq!(
            confirmation_text(&[
                summary(StatementClassKind::Read),
                summary(StatementClassKind::Dml)
            ]),
            "This connection's safety policy requires confirmation for 1 statement that may change data or schema."
        );
    }
}
