//! Plan 031 step 4: SQLite documents. A query tab, a paged table-data tab and
//! a structure tab, all on the connection's one session. Results reuse the
//! shared result grid; every request is bounded by the backend session and
//! retention here is charged to the workspace allowance like query tabs.
use crate::accessible_editor::AccessibleEditor;
use crate::grid::ResultGrid;
use crate::sqlite_model::{self, Paging};
use crate::sqlite_workspace::SqliteContext;
use crate::workbench::{RunScript, RunStatement, StopQuery};
use crate::{sql, structure_table, style, ui};
use dbunk_lib::backend::QueryEvent;
use dbunk_lib::backend::sqlite_session::{
    SqliteExecution, SqlitePage, SqliteSessionError, SqliteStructure,
};
use editor::{Editor, EditorEvent};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, Role, SharedString, Subscription,
    Task, Window, div, prelude::*, px,
};
use language::Buffer;
use multi_buffer::MultiBufferOffset;

/// The workspace-wide retained-results ceiling shared with query tabs.
const RETAINED_CEILING: usize = 128 * 1024 * 1024;

pub enum SqliteDocEvent {
    /// Tab status changed.
    Changed,
    Latency(u64),
    /// A run may have created, dropped or attached objects.
    ObjectsChanged,
    OpenData {
        schema: String,
        name: String,
    },
    OpenStructure {
        schema: String,
        name: String,
    },
}

/// This document's share of the workspace retention allowance.
#[derive(Default)]
struct Retention {
    bytes: usize,
}

impl Retention {
    fn allowance(&self, context: &SqliteContext) -> usize {
        RETAINED_CEILING
            .saturating_sub(context.retained.get())
            .saturating_add(self.bytes)
    }

    fn account(&mut self, context: &SqliteContext, grid: &Entity<ResultGrid>, cx: &App) {
        let bytes = grid.read(cx).model().retained_bytes;
        context.retained.set(
            context
                .retained
                .get()
                .saturating_sub(self.bytes)
                .saturating_add(bytes),
        );
        self.bytes = bytes;
    }

    /// Clears the grid and gives its bytes back.
    fn release(&mut self, context: &SqliteContext, grid: &Entity<ResultGrid>, cx: &mut App) {
        grid.update(cx, |grid, cx| grid.begin(cx));
        self.account(context, grid, cx);
    }

    fn feed(
        &mut self,
        context: &SqliteContext,
        grid: &Entity<ResultGrid>,
        events: Vec<QueryEvent>,
        cx: &mut App,
    ) {
        grid.update(cx, |grid, cx| grid.begin(cx));
        self.account(context, grid, cx);
        let limit = self.allowance(context);
        grid.update(cx, |grid, cx| {
            for event in events {
                grid.consume_with_limit(event, limit, cx);
            }
        });
        self.account(context, grid, cx);
    }
}

fn new_grid(context: &SqliteContext, cx: &mut App) -> Entity<ResultGrid> {
    let grid = cx.new(ResultGrid::new);
    grid.update(cx, |grid, _| {
        grid.set_inspection_budget(context.retained.clone());
        grid.set_export_host(context.host.clone());
    });
    grid
}

/// Truncation and retention notes from the grid's last completion.
fn diagnostics(grid: &Entity<ResultGrid>, cx: &App) -> Vec<String> {
    let model = grid.read(cx).model();
    let mut notes = Vec::new();
    if let Some(completion) = &model.completion {
        if completion.omitted_result_sets > 0 {
            notes.push(format!(
                "{} result sets not shown",
                completion.omitted_result_sets
            ));
        }
        notes.extend(completion.truncation_reasons.iter().cloned());
    }
    if model.retention_limited {
        notes.push(format!(
            "Workspace retention limit: {} rows not kept",
            model.native_omitted_rows
        ));
    }
    notes
}

fn session_error(error: SqliteSessionError) -> Option<String> {
    match error {
        SqliteSessionError::Cancelled => None,
        other => Some(other.to_string()),
    }
}

fn icon_tool(
    id: &'static str,
    label: impl Into<SharedString>,
    icon: &'static str,
    enabled: bool,
    primary: bool,
) -> gpui::Stateful<gpui::Div> {
    ui::tool_button(id, label, Some(icon), enabled, primary).tab_index(0)
}

/// The Data | Structure switch; the other side opens its own tab.
fn table_switch<V: EventEmitter<SqliteDocEvent>>(
    schema: &str,
    name: &str,
    structure: bool,
    cx: &mut Context<V>,
) -> gpui::Div {
    let target = (schema.to_owned(), name.to_owned());
    ui::view_switch(
        "sqlite-view",
        structure,
        cx.listener(move |_, _, _, cx| {
            let (schema, name) = target.clone();
            cx.emit(if structure {
                SqliteDocEvent::OpenData { schema, name }
            } else {
                SqliteDocEvent::OpenStructure { schema, name }
            })
        }),
    )
}

pub struct SqliteQueryView {
    context: SqliteContext,
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    grid: Entity<ResultGrid>,
    running: Option<(u64, Task<()>)>,
    /// SQL awaiting the policy confirmation, exactly as it was reviewed.
    confirmation: Option<(String, String)>,
    error: Option<String>,
    summary: String,
    retention: Retention,
    /// Bumped per run; keys the error shake so a repeated error moves.
    attempt: u64,
    _editor_events: Subscription,
}

impl EventEmitter<SqliteDocEvent> for SqliteQueryView {}

impl SqliteQueryView {
    pub fn new(context: SqliteContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let language = sql::language(cx).expect("SQL grammar");
        let buffer = cx.new(|cx| {
            let mut buffer = Buffer::local("SELECT * FROM sqlite_schema LIMIT 100;", cx);
            buffer.set_language(Some(language), cx);
            buffer
        });
        let editor = cx.new(|cx| Editor::for_buffer(buffer, None, window, cx));
        let accessible = cx.new(|cx| AccessibleEditor::new(editor.clone(), "SQL editor", cx));
        let editor_events = cx.subscribe(&editor, |this, _, event, cx| {
            // A pending confirmation covers the reviewed text only.
            if matches!(event, EditorEvent::BufferEdited) && this.confirmation.take().is_some() {
                this.summary = "Edited; run again to review".into();
                cx.notify();
            }
        });
        let grid = new_grid(&context, cx);
        Self {
            context,
            editor,
            accessible,
            grid,
            running: None,
            confirmation: None,
            error: None,
            summary: "Ready".into(),
            retention: Retention::default(),
            attempt: 0,
            _editor_events: editor_events,
        }
    }

    pub fn focus(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }

    pub fn status(&self) -> String {
        if self.running.is_some() {
            "running".into()
        } else if self.error.is_some() {
            "failed".into()
        } else if self.confirmation.is_some() {
            "needs confirmation".into()
        } else {
            "ready".into()
        }
    }

    /// The whole buffer for a script; otherwise the selection, or the
    /// statement under the cursor when nothing is selected.
    fn sql_to_run(&self, script: bool, cx: &mut Context<Self>) -> Result<String, &'static str> {
        self.editor.update(cx, |editor, cx| {
            let text = editor.text(cx);
            if script {
                return Ok(text);
            }
            let selection = editor
                .selections
                .newest::<MultiBufferOffset>(&editor.display_snapshot(cx));
            match sqlite_model::sql_to_run(&text, selection.start.0..selection.end.0) {
                Some(range) => Ok(text[range].to_owned()),
                None => Err("No SQL statement at the cursor"),
            }
        })
    }

    /// Runs the statement at the cursor (or the selection), the whole
    /// buffer when `script`, or the reviewed SQL when `confirmed`.
    fn run(&mut self, confirmed: bool, script: bool, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        let Some(session) = self.context.session() else {
            self.error = Some("Connect to the database to run queries".into());
            cx.notify();
            return;
        };
        let sql = if confirmed {
            match self.confirmation.take() {
                Some((sql, _)) => sql,
                None => return,
            }
        } else {
            match self.sql_to_run(script, cx) {
                Ok(sql) => sql,
                Err(message) => {
                    self.summary = message.into();
                    cx.notify();
                    return;
                }
            }
        };
        if sql.trim().is_empty() {
            self.summary = "Nothing to run".into();
            cx.notify();
            return;
        }
        self.confirmation = None;
        self.error = None;
        self.attempt += 1;
        self.summary = "Running…".into();
        self.retention.release(&self.context, &self.grid, cx);
        let ticket = session.ticket();
        let statement = sql.clone();
        let task = self
            .context
            .host
            .runtime
            .spawn(async move { session.execute(ticket, statement, confirmed).await });
        self.running = Some((
            ticket,
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| this.finish(sql, result, cx))
                    .ok();
            }),
        ));
        cx.emit(SqliteDocEvent::Changed);
        cx.notify();
    }

    fn finish(
        &mut self,
        sql: String,
        result: Result<Result<SqliteExecution, SqliteSessionError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        self.running = None;
        match result {
            Ok(Ok(execution)) => {
                self.summary = sqlite_model::execution_summary(&execution);
                match execution {
                    SqliteExecution::Completed {
                        sets,
                        omitted_sets,
                        byte_limited,
                        elapsed_ms,
                        ..
                    } => {
                        let events =
                            sqlite_model::execution_events(sets, omitted_sets, byte_limited);
                        self.retention.feed(&self.context, &self.grid, events, cx);
                        cx.emit(SqliteDocEvent::Latency(elapsed_ms));
                        if sqlite_model::changes_objects(&sql) {
                            cx.emit(SqliteDocEvent::ObjectsChanged);
                        }
                    }
                    SqliteExecution::NeedsConfirmation { statements } => {
                        self.confirmation =
                            Some((sql, sqlite_model::confirmation_text(&statements)));
                    }
                    SqliteExecution::Blocked { reason } => self.error = Some(reason),
                }
            }
            Ok(Err(error)) => {
                self.summary = match &error {
                    SqliteSessionError::Cancelled => "Cancelled".into(),
                    _ => "Failed".into(),
                };
                self.error = session_error(error);
            }
            Err(_) => {
                self.summary = "Failed".into();
                self.error = Some("Query task failed".into());
            }
        }
        cx.emit(SqliteDocEvent::Changed);
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if let (Some((ticket, _)), Some(session)) = (&self.running, self.context.session()) {
            session.cancel(*ticket);
            self.summary = "Cancelling…".into();
            cx.notify();
        }
    }

    /// The session went away (its owner closes it, which interrupts the
    /// run). Forget the run so a later Stop cannot aim its ticket at a new
    /// session and Run is available after a reconnect.
    pub fn detached(&mut self, cx: &mut Context<Self>) {
        self.confirmation = None;
        if self.running.take().is_some() {
            self.summary = "Disconnected".into();
        }
        cx.notify();
    }

    pub fn release(&mut self, cx: &mut Context<Self>) {
        self.stop(cx);
        self.running = None;
        self.retention.release(&self.context, &self.grid, cx);
    }

    /// Clears the results (giving their bytes back) and the last error. A
    /// running query is left alone: its results would refill the grid, and
    /// the grid is already empty while it runs. A pending confirmation stays.
    pub fn clear_results(&mut self, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        self.retention.release(&self.context, &self.grid, cx);
        self.error = None;
        if self.confirmation.is_none() {
            self.summary = "Ready".into();
        }
        cx.emit(SqliteDocEvent::Changed);
        cx.notify();
    }
}

impl Render for SqliteQueryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = self.context.session().is_some();
        let read_only = self
            .context
            .session()
            .is_some_and(|session| session.info().read_only);
        let running = self.running.is_some();
        let model = self.grid.read(cx).model();
        let sets: Vec<(usize, String, bool)> = model
            .sets
            .iter()
            .enumerate()
            .map(|(index, set)| {
                (
                    index,
                    format!(
                        "Result {} · {} rows",
                        index + 1,
                        set.row_count.unwrap_or(set.rows.len() as u64)
                    ),
                    index == model.active,
                )
            })
            .collect();
        let notes = diagnostics(&self.grid, cx);
        div()
            .id("sqlite-query")
            .key_context("SqliteQuery")
            .on_action(cx.listener(|this, _: &RunStatement, _, cx| this.run(false, false, cx)))
            .on_action(cx.listener(|this, _: &RunScript, _, cx| this.run(false, true, cx)))
            .on_action(cx.listener(|this, _: &StopQuery, _, cx| this.stop(cx)))
            .size_full()
            .flex()
            .flex_col()
            .child(
                ui::toolbar()
                    .child(
                        icon_tool(
                            "sqlite-run",
                            "Run",
                            "icons/play_filled.svg",
                            connected && !running,
                            true,
                        )
                        .aria_keyshortcuts("Meta+Enter")
                        .child(ui::shortcut("⌘↵"))
                        .tooltip(ui::tooltip(
                            "Run statement at cursor or selection (Cmd-Enter); \
                             run all (Cmd-Shift-Enter)",
                        ))
                        .tooltip_show_delay(ui::tooltip_delay())
                        .when(connected && !running, |button| {
                            button
                                .on_click(cx.listener(|this, _, _, cx| this.run(false, false, cx)))
                        }),
                    )
                    .child(
                        icon_tool("sqlite-stop", "Stop", "icons/stop.svg", running, false)
                            .aria_keyshortcuts("Meta+Period")
                            .when(running, |button| {
                                button.on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
                            }),
                    )
                    .when(read_only, |bar| bar.child(ui::badge("read-only")))
                    .child(ui::grow())
                    .child(
                        div()
                            .id("sqlite-query-summary")
                            .role(Role::Status)
                            .aria_label(self.summary.clone())
                            .font_family(style::MONO)
                            .text_color(style::faint())
                            .child(self.summary.clone()),
                    ),
            )
            .child(
                div()
                    .h(gpui::relative(0.4))
                    .flex_none()
                    .min_h_0()
                    .border_b_1()
                    .border_color(style::line())
                    .child(self.accessible.clone()),
            )
            .when_some(self.confirmation.clone(), |root, (_, text)| {
                root.child(
                    div()
                        .id("sqlite-confirmation")
                        .role(Role::Alert)
                        .aria_label(text.clone())
                        .flex_none()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .px(px(10.))
                        .py(px(6.))
                        .bg(style::warn_fill())
                        .border_b_1()
                        .border_color(style::line())
                        .text_color(style::text())
                        .child(div().flex_1().min_w(px(200.)).child(text))
                        .child(
                            ui::button("sqlite-confirm", "Run anyway", ui::Variant::Danger, true)
                                .tab_index(0)
                                .on_click(cx.listener(|this, _, _, cx| this.run(true, false, cx))),
                        )
                        .child(
                            ui::button("sqlite-confirm-cancel", "Cancel", ui::Variant::Ghost, true)
                                .tab_index(0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirmation = None;
                                    this.summary = "Not run".into();
                                    cx.emit(SqliteDocEvent::Changed);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when_some(self.error.clone(), |root, error| {
                root.child(div().flex_none().p(px(6.)).child(ui::shake(
                    ("sqlite-query-error", self.attempt),
                    ui::error_banner("sqlite-query-error-banner", error),
                )))
            })
            .when(sets.len() > 1, |root| {
                root.child(ui::segmented().children(sets.into_iter().map(
                    |(index, label, active)| {
                        ui::segment(("sqlite-result-set", index), label, active, true)
                            .tab_index(0)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.grid.update(cx, |grid, cx| grid.set_active(index, cx))
                            }))
                    },
                )))
            })
            .child(div().flex_1().min_h_0().child(self.grid.clone()))
            .when(!notes.is_empty(), |root| {
                root.child(ui::status_line().children(notes))
            })
    }
}

pub struct SqliteDataView {
    context: SqliteContext,
    schema: String,
    name: String,
    paging: Paging,
    page: Option<SqlitePage>,
    grid: Entity<ResultGrid>,
    loading: Option<(u64, Task<()>)>,
    error: Option<String>,
    retention: Retention,
    attempt: u64,
}

impl EventEmitter<SqliteDocEvent> for SqliteDataView {}

impl SqliteDataView {
    pub fn new(
        context: SqliteContext,
        schema: String,
        name: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let grid = new_grid(&context, cx);
        let mut view = Self {
            context,
            schema,
            name,
            paging: Paging::default(),
            page: None,
            grid,
            loading: None,
            error: None,
            retention: Retention::default(),
            attempt: 0,
        };
        view.reload(cx);
        view
    }

    pub fn focus(&self, cx: &App) -> FocusHandle {
        self.grid.focus_handle(cx)
    }

    pub fn status(&self) -> String {
        if self.loading.is_some() {
            "loading".into()
        } else if self.error.is_some() {
            "failed".into()
        } else {
            match &self.page {
                Some(page) => self.paging.label(page),
                None => "not loaded".into(),
            }
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load(self.paging, cx);
    }

    /// Loads one page. A newer page request interrupts the older one.
    fn load(&mut self, paging: Paging, cx: &mut Context<Self>) {
        let Some(session) = self.context.session() else {
            self.error = Some("Not connected".into());
            cx.notify();
            return;
        };
        if let Some((ticket, _)) = self.loading.take() {
            session.cancel(ticket);
        }
        let ticket = session.ticket();
        let (schema, name) = (self.schema.clone(), self.name.clone());
        let task = self.context.host.runtime.spawn(async move {
            session
                .browse(ticket, schema, name, paging.offset, paging.limit)
                .await
        });
        self.error = None;
        self.attempt += 1;
        self.loading = Some((
            ticket,
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    if this
                        .loading
                        .as_ref()
                        .is_none_or(|(current, _)| *current != ticket)
                    {
                        return;
                    }
                    this.loading = None;
                    match result {
                        Ok(Ok(page)) => {
                            this.paging = paging;
                            let events = sqlite_model::page_events(&page, paging.limit);
                            this.retention.feed(&this.context, &this.grid, events, cx);
                            cx.emit(SqliteDocEvent::Latency(page.elapsed_ms));
                            this.page = Some(page);
                        }
                        Ok(Err(error)) => this.error = session_error(error),
                        Err(_) => this.error = Some("Page load failed".into()),
                    }
                    cx.emit(SqliteDocEvent::Changed);
                    cx.notify();
                })
                .ok();
            }),
        ));
        cx.emit(SqliteDocEvent::Changed);
        cx.notify();
    }

    pub fn detached(&mut self, cx: &mut Context<Self>) {
        self.loading = None;
        cx.notify();
    }

    pub fn release(&mut self, cx: &mut Context<Self>) {
        if let (Some((ticket, _)), Some(session)) = (&self.loading, self.context.session()) {
            session.cancel(*ticket);
        }
        self.loading = None;
        self.retention.release(&self.context, &self.grid, cx);
    }
}

impl Render for SqliteDataView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = self.context.session().is_some();
        let idle = connected && self.loading.is_none();
        let has_previous = idle && self.paging.offset > 0;
        let has_next = idle && self.page.as_ref().is_some_and(|page| page.has_more);
        let notes = diagnostics(&self.grid, cx);
        div()
            .id("sqlite-data")
            .size_full()
            .flex()
            .flex_col()
            .child(
                ui::toolbar_strip()
                    .child(table_switch(&self.schema, &self.name, false, cx))
                    .child(ui::separator())
                    .child(ui::crumbs(format!("{}.", self.schema), self.name.clone()))
                    .child(ui::grow())
                    .child(
                        ui::icon_button(
                            "sqlite-data-previous",
                            "Previous page",
                            "icons/chevron_left.svg",
                            has_previous,
                        )
                        .tab_index(0)
                        .when(has_previous, |button| {
                            button.on_click(cx.listener(|this, _, _, cx| {
                                let paging = this.paging.previous();
                                this.load(paging, cx)
                            }))
                        }),
                    )
                    .child(ui::range_status(
                        "sqlite-data-range",
                        self.page
                            .as_ref()
                            .map(|page| (page.offset, page.set.rows.len())),
                    ))
                    .child(
                        ui::icon_button(
                            "sqlite-data-next",
                            "Next page",
                            "icons/chevron_right.svg",
                            has_next,
                        )
                        .tab_index(0)
                        .when(has_next, |button| {
                            button.on_click(cx.listener(|this, _, _, cx| {
                                // Advance by the rows the page kept.
                                let next = this.page.as_ref().map(|page| this.paging.next(page));
                                if let Some(paging) = next {
                                    this.load(paging, cx)
                                }
                            }))
                        }),
                    )
                    .child(
                        ui::icon_button(
                            "sqlite-data-refresh",
                            "Refresh",
                            "icons/rotate_cw.svg",
                            idle,
                        )
                        .tab_index(0)
                        .when(idle, |button| {
                            button.on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                        }),
                    ),
            )
            .when_some(self.error.clone(), |root, error| {
                root.child(div().flex_none().p(px(6.)).child(ui::shake(
                    ("sqlite-data-error", self.attempt),
                    ui::error_banner("sqlite-data-error-banner", error),
                )))
            })
            .child(div().flex_1().min_h_0().child(self.grid.clone()))
            .when_some(self.page.as_ref(), |root, page| {
                root.child(
                    ui::status_line()
                        .child(format!("{} ms", page.elapsed_ms))
                        .child(if page.has_more {
                            "more rows on the next page"
                        } else {
                            "end of table"
                        })
                        .children(notes),
                )
            })
    }
}

/// A loaded structure, kept only as what the page shows.
struct LoadedStructure {
    kind: SharedString,
    columns: usize,
    tables: Vec<structure_table::SectionTable>,
    definition: Option<SharedString>,
}

impl LoadedStructure {
    fn new(structure: SqliteStructure) -> Self {
        Self {
            tables: sqlite_model::structure_tables(&structure),
            columns: structure.columns.len(),
            definition: structure.definition.map(Into::into),
            kind: structure.kind.into(),
        }
    }
}

pub struct SqliteStructureView {
    context: SqliteContext,
    schema: String,
    name: String,
    structure: Option<LoadedStructure>,
    loading: Option<(u64, Task<()>)>,
    error: Option<String>,
    attempt: u64,
    focus: FocusHandle,
}

impl EventEmitter<SqliteDocEvent> for SqliteStructureView {}

impl SqliteStructureView {
    pub fn new(
        context: SqliteContext,
        schema: String,
        name: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            context,
            schema,
            name,
            structure: None,
            loading: None,
            error: None,
            attempt: 0,
            focus: cx.focus_handle(),
        };
        view.reload(cx);
        view
    }

    pub fn focus(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub fn status(&self) -> String {
        if self.loading.is_some() {
            "loading".into()
        } else if self.error.is_some() {
            "failed".into()
        } else {
            self.structure
                .as_ref()
                .map(|structure| format!("{} columns", structure.columns))
                .unwrap_or_else(|| "not loaded".into())
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.context.session() else {
            self.error = Some("Not connected".into());
            cx.notify();
            return;
        };
        if let Some((ticket, _)) = self.loading.take() {
            session.cancel(ticket);
        }
        let ticket = session.ticket();
        let (schema, name) = (self.schema.clone(), self.name.clone());
        let task = self
            .context
            .host
            .runtime
            .spawn(async move { session.structure(ticket, schema, name).await });
        self.error = None;
        self.attempt += 1;
        self.loading = Some((
            ticket,
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    if this
                        .loading
                        .as_ref()
                        .is_none_or(|(current, _)| *current != ticket)
                    {
                        return;
                    }
                    this.loading = None;
                    match result {
                        Ok(Ok(structure)) => this.structure = Some(LoadedStructure::new(structure)),
                        Ok(Err(error)) => this.error = session_error(error),
                        Err(_) => this.error = Some("Structure read failed".into()),
                    }
                    cx.emit(SqliteDocEvent::Changed);
                    cx.notify();
                })
                .ok();
            }),
        ));
        cx.notify();
    }

    pub fn detached(&mut self, cx: &mut Context<Self>) {
        self.loading = None;
        cx.notify();
    }

    pub fn release(&mut self, cx: &mut Context<Self>) {
        if let (Some((ticket, _)), Some(session)) = (&self.loading, self.context.session()) {
            session.cancel(*ticket);
        }
        self.loading = None;
        cx.notify();
    }
}

impl Render for SqliteStructureView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = self.context.session().is_some();
        let idle = connected && self.loading.is_none();
        let body = self.structure.as_ref().map(|structure| {
            div()
                .flex()
                .flex_col()
                .child(structure_table::sections(
                    "sqlite-structure-sections",
                    format!("{} structure, read only", structure.kind),
                    &structure.tables,
                ))
                .when_some(structure.definition.clone(), |body, sql| {
                    body.child(structure_table::definition(
                        "sqlite-definition",
                        "Definition",
                        sql,
                    ))
                })
        });
        div()
            .id("sqlite-structure")
            .track_focus(&self.focus)
            .tab_index(0)
            .size_full()
            .flex()
            .flex_col()
            .child(
                ui::toolbar_strip()
                    .child(table_switch(&self.schema, &self.name, true, cx))
                    .child(ui::separator())
                    .child(ui::crumbs(format!("{}.", self.schema), self.name.clone()))
                    .when_some(self.structure.as_ref(), |bar, structure| {
                        bar.child(ui::badge(structure.kind.clone()))
                    })
                    .child(ui::grow())
                    .child(
                        ui::icon_button(
                            "sqlite-structure-refresh",
                            "Refresh",
                            "icons/rotate_cw.svg",
                            idle,
                        )
                        .tab_index(0)
                        .when(idle, |button| {
                            button.on_click(cx.listener(|this, _, _, cx| this.reload(cx)))
                        }),
                    ),
            )
            .when(self.loading.is_some(), |root| {
                root.child(
                    div()
                        .p(px(10.))
                        .text_color(style::faint())
                        .child("Loading structure…"),
                )
            })
            .when_some(self.error.clone(), |root, error| {
                root.child(div().flex_none().p(px(6.)).child(ui::shake(
                    ("sqlite-structure-error", self.attempt),
                    ui::error_banner("sqlite-structure-error-banner", error),
                )))
            })
            .child(
                div()
                    .id("sqlite-structure-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(body),
            )
    }
}
