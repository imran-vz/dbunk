//! Native table page. The workspace drains replies fairly and owns persistence;
//! the table runtime owns sockets, cancellation and worker termination.
use crate::{
    browse_controls::{BrowseControls, BrowseEvent},
    browse_preferences::{BrowsePreferences, BrowseState, PAGE_SIZES, PreferencePatch},
    controller::{Host, TableCommand, TableControls, TableMessage, TableReceiver},
    data_model::{PageAction, RequestTicket, TableDocument, TableQuery},
    fk_navigation::{Navigation, Selection},
    grid::{GridEvent, ResultGrid},
    grid_columns::ColumnAction,
    table_changes::{ChangesEvent, TableChanges},
};
use dbunk_lib::backend::{WorkspaceDocument, WorkspaceTableState, data::*};
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, SharedString,
    Subscription, Window, div, prelude::*, px,
};
use std::{cell::Cell, collections::HashMap, rc::Rc, sync::Arc};

mod relationship_detail;
mod whole_config;
mod whole_export;

use relationship_detail::{Back, Current, DEPTH, Detail, Origin, ROW_LIMIT, State};

const BACK: &str = "Back to previous row";
const DETAIL_BUTTONS: [&str; 7] = [
    "Next related row",
    "Previous detail column",
    "Next detail column",
    "Follow detail foreign key",
    "Open related table",
    BACK,
    "Close details",
];

fn current<'a>(
    connection: Option<&'a str>,
    state: &'a WorkspaceTableState,
    model: Option<&'a TableDocument>,
) -> Option<Current<'a>> {
    let model = model?;
    Some(Current {
        connection,
        schema: &state.schema,
        table: &state.table,
        query: model.query(),
        page_number: model.page(),
        page: model
            .page_is_current()
            .then(|| model.shared_result())
            .flatten(),
    })
}

const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;

pub enum TableEvent {
    OpenWholeTableCsv {
        connection: String,
        target: dbunk_lib::backend::csv_transfers::CsvTarget,
        null_token: String,
    },
    OpenStructure {
        connection: String,
        schema: String,
        table: String,
    },
    OpenTableSeed {
        connection: String,
        schema: String,
        table: String,
    },
    OpenTableCopy {
        connection: String,
        schema: String,
        table: String,
    },
    OpenCsvTransfer {
        connection: String,
        direction: dbunk_lib::backend::csv_transfers::CsvDirection,
        target: dbunk_lib::backend::csv_transfers::CsvTarget,
    },
    OpenPgTools {
        connection: String,
        operation: crate::pg_tool_jobs::Operation,
        context: Option<(String, String)>,
    },
    OpenReference {
        connection: String,
        schema: String,
        table: String,
        filters: Vec<BrowseFilter>,
    },
    Changed,
    PersistApply(u64),
}

#[derive(Clone, Copy)]
enum Action {
    WholeExport,
    Structure,
    TableCopy,
    TableSeed,
    FileJob(crate::pg_tool_jobs::Operation),
    CsvTransfer(dbunk_lib::backend::csv_transfers::CsvDirection),
    ForeignKeys,
    NextReference,
    OpenReference,
    CloseReference,
    ShowRelated,
    DetailRow,
    DetailColumn(bool),
    DetailForeignKeys,
    OpenRelated,
    DetailBack,
    CloseDetail,
    Column(ColumnAction),
    AutoFit(bool),
    Preferences,
    Connect,
    Edit,
    Insert,
    Duplicate,
    Bulk,
    Delete,
    First,
    Previous,
    Next,
    Last,
    Refresh,
    Count,
    Cancel,
    Sort,
    PageSize(u32),
}

pub struct TableView {
    host: Arc<Host>,
    whole_export: whole_export::WholeExport,
    id: String,
    connection: Option<String>,
    state: WorkspaceTableState,
    model: Option<TableDocument>,
    grid: Entity<ResultGrid>,
    changes: Entity<TableChanges>,
    _change_events: Subscription,
    _change_status: Subscription,
    browse_controls: Entity<BrowseControls>,
    _browse_events: Subscription,
    _browse_status: Subscription,
    restore_query: bool,
    next_reference: u64,
    pending_reference: Option<(u64, Selection, bool)>,
    references: Option<Navigation>,
    detail: Option<Detail>,
    pending_detail: Option<(RequestTicket, u64, u64)>,
    _reference_status: Subscription,
    next_preference: u64,
    pending_preferences: Option<(u64, PreferencePatch)>,
    after_analysis: Option<PreferencePatch>,
    record_next: Option<(BrowseState, bool)>,
    pending_browse_preferences: Option<(crate::data_model::RequestTicket, BrowseState, bool)>,
    control_bytes: usize,
    controls: Option<TableControls>,
    receiver: Option<TableReceiver>,
    wake: async_channel::Sender<()>,
    retained: Rc<Cell<usize>>,
    retained_bytes: usize,
    editable: bool,
    busy: bool,
    status: String,
    delivery_failure: Option<String>,
    preferences_ready: bool,
    preferences_status: Option<String>,
    buttons: HashMap<String, FocusHandle>,
    tab_order: Vec<FocusHandle>,
    previous_focus: Option<FocusHandle>,
    _grid_events: Subscription,
}
impl EventEmitter<TableEvent> for TableView {}
impl TableView {
    pub fn new(
        host: Arc<Host>,
        document: &mut WorkspaceDocument,
        restored: bool,
        wake: async_channel::Sender<()>,
        retained: Rc<Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Transfer durable intent before cloning view preferences. Recovery must
        // retain one owned payload while it waits for working-budget admission.
        let saved_draft = document
            .table
            .as_mut()
            .expect("table document")
            .draft
            .take();
        let state = document.table.clone().expect("table document");
        let grid =
            cx.new(|cx| ResultGrid::new_table(state.schema.clone(), state.table.clone(), cx));
        grid.update(cx, |grid, _| grid.set_inspection_budget(retained.clone()));
        grid.update(cx, |grid, _| grid.set_export_host(host.clone()));
        let changes = cx.new(|_| TableChanges::new(&state, saved_draft, retained.clone()));
        let change_events = cx.subscribe_in(&changes, window, |this, _, event, window, cx| {
            match event {
                ChangesEvent::Changed => {
                    cx.emit(TableEvent::Changed);
                }
                ChangesEvent::PersistApply(id) => {
                    cx.emit(TableEvent::PersistApply(*id));
                }
                ChangesEvent::Applied => this.browse(PageAction::First, false, cx),
                ChangesEvent::KeyChanged => {
                    this.browse(PageAction::First, true, cx);
                    if !this.busy {
                        this.changes
                            .update(cx, |changes, cx| changes.key_refresh_failed(cx));
                    }
                }
                ChangesEvent::FocusGrid(handles) => {
                    if handles
                        .iter()
                        .any(|focus| focus.contains_focused(window, cx))
                    {
                        window.focus(&this.grid.focus_handle(cx), cx);
                    }
                }
            }
            cx.notify();
        });
        let change_status = cx.observe(&changes, |_, _, cx| cx.notify());
        let events = cx.subscribe(&grid, |this, _, event, cx| {
            if !this.editable || this.busy || this.changes.read(cx).navigation_blocked() {
                return;
            }
            match event {
                GridEvent::Preferences(patch) => {
                    if this.preferences_ready {
                        this.save_preferences(patch.clone(), cx);
                    } else {
                        this.preferences_status =
                            Some("Load table preferences before changing columns".into());
                    }
                }
                GridEvent::Sort { column, append } => {
                    if let Some(model) = &mut this.model {
                        match model.cycle_sort(column, *append) {
                            Ok(()) => {
                                let sort = model.query().sort.clone();
                                let mut state = this.browse_controls.read(cx).state().clone();
                                state.sort = sort;
                                this.apply_browse(state, true, cx);
                            }
                            Err(error) => this.status = format!("Sort refused: {error:?}"),
                        }
                    }
                }
            }
            cx.notify();
        });
        let browse_controls =
            cx.new(|cx| BrowseControls::new(BrowseState::workspace(&state), window, cx));
        let browse_events = cx.subscribe(&browse_controls, |this, _, event, cx| {
            if !this.editable
                || this.busy
                || this.changes.read(cx).navigation_blocked()
                || !this.preferences_ready
            {
                return;
            }
            match event {
                BrowseEvent::Apply(state, history) => {
                    this.apply_browse(state.clone(), *history, cx)
                }
                BrowseEvent::Mode(mode) => this.save_preferences(PreferencePatch::Mode(*mode), cx),
                BrowseEvent::SavePreset(preset) => {
                    this.save_preferences(PreferencePatch::Preset(preset.clone()), cx)
                }
            }
        });
        let browse_status = cx.observe(&browse_controls, |_, _, cx| cx.notify());
        // Bounds the preference/control snapshots and prepared inspection text
        // separately from the shared page, which still has one retained lease.
        let control_bytes = if retained.get() <= WORKSPACE_BYTES - 2 * 1024 * 1024 {
            retained.set(retained.get() + 2 * 1024 * 1024);
            2 * 1024 * 1024
        } else {
            0
        };
        let restore_query = restored || !state.filters.is_empty() || !state.sort.is_empty();
        let mut selected_cell = None;
        let reference_status = cx.observe(&grid, move |_, grid, cx| {
            let current = grid.read(cx).selected_cell();
            if current != selected_cell {
                selected_cell = current;
                cx.notify();
            }
        });
        Self {
            whole_export: whole_export::WholeExport::default(),
            host,
            id: document.id.clone(),
            connection: document.connection_id.clone(),
            state,
            model: None,
            grid,
            changes,
            _change_events: change_events,
            _change_status: change_status,
            browse_controls,
            _browse_events: browse_events,
            _browse_status: browse_status,
            restore_query,
            next_reference: 0,
            pending_reference: None,
            references: None,
            detail: None,
            pending_detail: None,
            _reference_status: reference_status,
            next_preference: 0,
            pending_preferences: None,
            after_analysis: None,
            record_next: None,
            pending_browse_preferences: None,
            control_bytes,
            controls: None,
            receiver: None,
            wake,
            retained,
            retained_bytes: 0,
            editable: true,
            busy: false,
            status: "Disconnected".into(),
            delivery_failure: None,
            preferences_ready: false,
            preferences_status: None,
            buttons: HashMap::new(),
            tab_order: Vec::new(),
            previous_focus: None,
            _grid_events: events,
        }
    }
    pub fn apply_saved(&mut self, id: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        self.changes
            .update(cx, |changes, cx| changes.apply_saved(id, result, cx));
    }
    pub fn snapshot(&self, cx: &gpui::App) -> WorkspaceTableState {
        let mut state = self.state.clone();
        state.draft = self.changes.read(cx).snapshot();
        state
    }
    pub fn snapshot_bytes(&self, cx: &gpui::App) -> usize {
        let state = crate::results::encoded_size(&self.state);
        let draft = self.changes.read(cx).snapshot_bytes();
        if draft == 0 {
            state
        } else {
            state.saturating_sub(4).saturating_add(draft)
        }
    }
    pub fn document_status(&self) -> &str {
        &self.status
    }
    pub fn focus_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = &self.whole_export.view {
            window.focus(&view.read(cx).focus(), cx);
            return;
        }
        let mut valid = self.changes.read(cx).focus_handles(cx);
        valid.push(self.grid.focus_handle(cx));
        valid.extend(self.grid.read(cx).export_focus());
        valid.extend(self.browse_controls.read(cx).focus_handles(cx));
        valid.extend(self.reference_handles());
        valid.extend(self.detail_handles());
        let focus = self
            .previous_focus
            .as_ref()
            .filter(|focus| valid.contains(focus))
            .cloned()
            .unwrap_or_else(|| self.grid.focus_handle(cx));
        window.focus(&focus, cx);
    }
    pub fn remember_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .whole_export
            .view
            .as_ref()
            .is_some_and(|view| view.read(cx).contains_focus(window, cx))
        {
            self.previous_focus = window.focused(cx);
            return;
        }
        let focus = self
            .changes
            .read(cx)
            .focus_handles(cx)
            .into_iter()
            .chain(self.browse_controls.read(cx).focus_handles(cx))
            .chain(self.reference_handles())
            .chain(self.detail_handles())
            .chain(self.grid.read(cx).export_focus())
            .find(|focus| focus.contains_focused(window, cx))
            .unwrap_or_else(|| self.grid.focus_handle(cx));
        self.previous_focus = Some(focus);
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editable = editable;
        self.changes.update(cx, |changes, cx| {
            changes.set_enabled(editable && !self.busy, cx)
        });
        self.update_controls(cx);
        cx.notify();
    }
    pub fn bind_connection(&mut self, id: String, cx: &mut Context<Self>) {
        self.close_whole_export(cx);
        self.references = None;
        self.pending_reference = None;
        self.detail = None;
        self.pending_detail = None;
        self.connection = Some(id);
        cx.emit(TableEvent::Changed);
    }
    pub fn begin_connect(&mut self, cx: &mut Context<Self>) {
        if self.control_bytes == 0 && self.retained.get() <= WORKSPACE_BYTES - 2 * 1024 * 1024 {
            self.control_bytes = 2 * 1024 * 1024;
            self.retained.set(self.retained.get() + self.control_bytes);
        }
        if self.control_bytes == 0 {
            self.status = "Workspace memory budget is full; close another tab".into();
            cx.notify();
            return;
        }
        if !self.editable || self.controls.is_some() {
            return;
        }
        let Some(connection) = &self.connection else {
            self.status = "Select a connection".into();
            cx.notify();
            return;
        };
        let mut model = match TableDocument::new(
            connection.clone(),
            self.id.clone(),
            MutationTable {
                schema: self.state.schema.clone(),
                table: self.state.table.clone(),
            },
        ) {
            Ok(model) => model,
            Err(error) => {
                self.status = format!("Table unavailable: {error:?}");
                cx.notify();
                return;
            }
        };
        if let Err(error) = model.set_query(
            TableQuery {
                filters: self.state.filters.clone(),
                sort: self.state.sort.clone(),
                page_size: self.state.page_size,
            },
            "",
        ) {
            self.status = format!("Filter unavailable: {error:?}");
            cx.notify();
            return;
        }
        match self
            .host
            .open_table_document(self.id.clone(), connection.clone(), self.wake.clone())
        {
            Ok((controls, receiver)) => {
                self.delivery_failure = None;
                self.model = Some(model);
                self.controls = Some(controls);
                self.receiver = Some(receiver);
                self.busy = true;
                self.status = "Connecting".into();
            }
            Err(error) => self.status = error.to_string(),
        }
        cx.notify();
    }
    /// A restore changed this connection. Stop the metadata/data lane and
    /// preserve staged intent; query sessions are owned by a different lane.
    pub fn invalidate_after_restore(&mut self, cx: &mut Context<Self>) {
        if let Some(controls) = &self.controls {
            controls.stop();
        }
        self.mark_disconnected(cx);
        self.status =
            "Database may have changed; reconnect and refresh before using retained data".into();
        cx.notify();
    }
    pub fn mark_disconnected(&mut self, cx: &mut Context<Self>) {
        self.whole_export.read.clear();
        if let Some(view) = &self.whole_export.view {
            view.update(cx, |view, cx| view.fail("Disconnected; retained export capture is historical. Reconnect to capture again".into(), cx));
        }
        self.references = None;
        self.pending_reference = None;
        self.detail = None;
        self.pending_detail = None;
        self.changes
            .update(cx, |changes, cx| changes.disconnected(cx));
        self.controls.take();
        self.receiver.take();
        self.pending_preferences = None;
        self.after_analysis = None;
        self.pending_browse_preferences = None;
        self.record_next = None;
        self.restore_query = true;
        self.preferences_ready = false;
        if let Some(model) = &mut self.model {
            model.invalidate();
        }
        self.busy = false;
        self.status = "Disconnected".into();
        cx.notify();
    }
    pub fn clear_results(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.changes.read(cx).navigation_blocked() {
            return;
        }
        self.references = None;
        self.changes.update(cx, |changes, cx| {
            changes.page(None, self.controls.clone(), cx)
        });
        self.grid.update(cx, |grid, cx| grid.begin(cx));
        self.browse_controls
            .update(cx, |controls, cx| controls.page(None, cx));
        // All three views release their shared page before its reservation.
        if let Some(model) = &mut self.model {
            model.clear_page();
        }
        self.account(0);
        cx.notify();
    }
    fn account(&mut self, bytes: usize) {
        self.retained.set(
            self.retained
                .get()
                .saturating_sub(self.retained_bytes)
                .saturating_add(bytes),
        );
        self.retained_bytes = bytes;
    }
    fn browse(&mut self, action: PageAction, structure: bool, cx: &mut Context<Self>) {
        self.references = None;
        let preference_intent = self.record_next.take();
        self.pending_browse_preferences = None;
        let (Some(model), Some(controls)) = (&mut self.model, &self.controls) else {
            return;
        };
        match model.browse(action, structure) {
            Ok((ticket, payload)) => match controls.send(TableCommand::Browse(ticket, payload)) {
                Ok(()) => {
                    self.busy = true;
                    self.status = "Loading table".into();
                    self.pending_browse_preferences =
                        preference_intent.map(|(state, history)| (ticket, state, history));
                }
                Err(error) => {
                    model.failed(ticket);
                    self.status = error.into();
                }
            },
            Err(error) => self.status = format!("Page unavailable: {error:?}"),
        }
        self.changes.update(cx, |changes, cx| {
            changes.set_enabled(self.editable && !self.busy, cx)
        });
        cx.notify();
    }
    fn update_controls(&mut self, cx: &mut Context<Self>) {
        let enabled = self.editable
            && !self.busy
            && self.preferences_ready
            && self.controls.is_some()
            && !self.changes.read(cx).navigation_blocked();
        self.browse_controls
            .update(cx, |controls, cx| controls.set_enabled(enabled, cx));
    }
    fn apply_browse(&mut self, state: BrowseState, history: bool, cx: &mut Context<Self>) {
        let result = state.validate().map_err(str::to_owned).and_then(|_| {
            self.model
                .as_mut()
                .ok_or("Connect this table first".to_owned())?
                .set_query(state.query(), &state.raw_filter_text)
                .map_err(|error| format!("Browse change refused: {error:?}"))
        });
        if let Err(error) = result {
            self.status = error;
            cx.notify();
            return;
        }
        let model = self.model.as_ref().unwrap();
        self.state.filters = model.query().filters.clone();
        self.state.sort = model.query().sort.clone();
        self.state.page_size = model.query().page_size;
        self.browse_controls
            .update(cx, |controls, cx| controls.set_state(state.clone(), cx));
        self.record_next = Some((state, history));
        self.restore_query = true;
        cx.emit(TableEvent::Changed);
        self.browse(PageAction::First, false, cx);
    }
    fn save_preferences(&mut self, patch: PreferencePatch, cx: &mut Context<Self>) {
        let Some(controls) = &self.controls else {
            return;
        };
        let id = self.next_preference.wrapping_add(1);
        self.next_preference = id;
        match controls.send(TableCommand::SavePreferences(
            id,
            self.state.schema.clone(),
            self.state.table.clone(),
            patch.clone(),
        )) {
            Ok(()) => {
                self.pending_preferences = Some((id, patch));
                self.busy = true;
                self.preferences_status = Some("Saving table preferences".into());
            }
            Err(error) => self.preferences_status = Some(error.into()),
        }
        self.update_controls(cx);
        cx.notify();
    }
    fn accept_preferences(
        &mut self,
        prefs: Option<TableGridPrefs>,
        cx: &mut Context<Self>,
    ) -> Result<BrowsePreferences, String> {
        let prefs = prefs.unwrap_or_else(|| TableGridPrefs(serde_json::json!({"version":1})));
        let parsed = BrowsePreferences::parse(&prefs).map_err(str::to_owned)?;
        self.grid
            .update(cx, |grid, cx| grid.load_columns(Some(prefs), cx))
            .map_err(str::to_owned)?;
        self.browse_controls
            .update(cx, |controls, cx| controls.preferences(parsed.clone(), cx));
        Ok(parsed)
    }
    fn load_preferences(&mut self, cx: &mut Context<Self>) {
        let Some(controls) = &self.controls else {
            return;
        };
        match controls.send(TableCommand::LoadPreferences(
            self.state.schema.clone(),
            self.state.table.clone(),
        )) {
            Ok(()) => {
                self.busy = true;
                self.preferences_status = Some("Loading table preferences".into());
            }
            Err(error) => {
                self.busy = false;
                self.preferences_status = Some(error.into());
            }
        }
        cx.notify();
    }
    fn reference_handles(&self) -> Vec<FocusHandle> {
        if self.references.is_none() {
            return vec![];
        }
        [
            "Next constraint",
            "Show related row",
            "Open referenced table",
            "Close reference",
        ]
        .into_iter()
        .filter_map(|label| self.buttons.get(label).cloned())
        .collect()
    }
    fn detail_handles(&self) -> Vec<FocusHandle> {
        if self.detail.is_none() {
            return vec![];
        }
        DETAIL_BUTTONS
            .into_iter()
            .filter_map(|label| self.buttons.get(label).cloned())
            .collect()
    }
    fn focus_button(&mut self, label: &str, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self
            .buttons
            .entry(label.to_owned())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        window.focus(&focus, cx);
    }
    /// A reference source is either the exact grid cell or the top detail cell.
    fn reference_current(&self, selection: &Selection, cx: &gpui::App) -> bool {
        self.selection_matches(selection, cx)
            || (self.connection.as_deref() == Some(selection.connection.as_str())
                && self
                    .detail
                    .as_ref()
                    .is_some_and(|detail| detail.owns(selection)))
    }
    fn selection_matches(&self, selection: &Selection, cx: &gpui::App) -> bool {
        self.model.as_ref().is_some_and(|model| {
            model.page_is_current()
                && model.shared_result().is_some_and(|page| {
                    selection.matches(
                        &page,
                        self.connection.as_deref(),
                        self.grid.read(cx).selected_cell(),
                    )
                })
        })
    }
    fn request_references(&mut self, from_detail: bool, cx: &mut Context<Self>) {
        if self.after_analysis.is_some() || self.pending_preferences.is_some() {
            return;
        }
        let source = (|| -> Result<_, &'static str> {
            let connection = self
                .connection
                .clone()
                .ok_or("The table connection is unavailable")?;
            if from_detail {
                let detail = self.detail.as_ref().ok_or("Open related details first")?;
                let frame = detail.top();
                return Ok((
                    detail.selection(&connection)?,
                    frame.schema.clone(),
                    frame.table.clone(),
                ));
            }
            let model = self
                .model
                .as_ref()
                .filter(|model| model.page_is_current())
                .ok_or("Refresh the table before following a reference")?;
            let page = model.shared_result().ok_or("Load a table page first")?;
            let (row, column) = self
                .grid
                .read(cx)
                .selected_cell()
                .ok_or("Select a source cell")?;
            Ok((
                Selection::new(page, connection, row, column)?,
                self.state.schema.clone(),
                self.state.table.clone(),
            ))
        })();
        let (selection, schema, table) = match source {
            Ok(source) => source,
            Err(error) => {
                self.status = error.into();
                return;
            }
        };
        let id = self.next_reference.wrapping_add(1);
        self.next_reference = id;
        match self
            .controls
            .as_ref()
            .ok_or("Connect this table first")
            .and_then(|controls| controls.send(TableCommand::ForeignKeys(id, schema, table)))
        {
            Ok(()) => {
                self.references = None;
                self.pending_reference = Some((id, selection, false));
                self.busy = true;
                self.status = if from_detail {
                    "Loading foreign keys for the selected related row".into()
                } else {
                    "Loading foreign keys for the original selected row".into()
                };
            }
            Err(error) => self.status = error.into(),
        }
    }
    fn open_reference(&mut self, cx: &mut Context<Self>) {
        let Some(review) = &self.references else {
            return;
        };
        if !self.reference_current(&review.selection, cx) {
            self.status = "The selected cell or page changed; load references again".into();
            return;
        }
        match &review.choice().target {
            Ok(target) => cx.emit(TableEvent::OpenReference {
                connection: review.selection.connection.clone(),
                schema: target.schema.clone(),
                table: target.table.clone(),
                filters: target.filters.clone(),
            }),
            Err(error) => self.status = error.message().into(),
        }
    }
    /// Inline read of the chosen reference. A grid source starts a new history;
    /// a related-row source pushes one bounded step onto the current history.
    fn show_related(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = &self.references else {
            return;
        };
        if !self.reference_current(&review.selection, cx) {
            self.status = "The selected cell or page changed; load references again".into();
            return;
        }
        let choice = review.choice();
        let (key, target) = (choice.key.clone(), choice.target.clone());
        let opened = if self
            .detail
            .as_ref()
            .is_some_and(|detail| detail.owns(&review.selection))
        {
            self.detail.as_mut().unwrap().push(&key, target)
        } else {
            let cell = self.grid.read(cx).selected_cell();
            current(self.connection.as_deref(), &self.state, self.model.as_ref())
                .ok_or("Refresh the table first")
                .and_then(|current| Origin::capture(&current, cell.ok_or("Select a source cell")?))
                .and_then(|origin| Detail::open(origin, &key, target, self.retained.clone()))
                .map(|(detail, generation)| {
                    self.detail = Some(detail);
                    generation
                })
        };
        match opened {
            Ok(generation) => {
                self.references = None;
                self.status = "Related row details opened; Back returns to the source".into();
                if let Some(generation) = generation {
                    self.fetch_related(generation);
                }
                self.focus_button(BACK, window, cx);
            }
            Err(error) => self.status = error.into(),
        }
    }
    fn fetch_related(&mut self, generation: u64) {
        let Some(detail) = &mut self.detail else {
            return;
        };
        let frame = detail.top();
        let request = self
            .model
            .as_mut()
            .ok_or("Connect this table first".to_owned())
            .and_then(|model| {
                model
                    .related(
                        &frame.schema,
                        &frame.table,
                        frame.filters.clone(),
                        ROW_LIMIT,
                    )
                    .map_err(|error| format!("Related row unavailable: {error:?}"))
            })
            .and_then(|(ticket, payload)| {
                let request_id = payload.request_id;
                self.controls
                    .as_ref()
                    .ok_or("Connect this table first")
                    .and_then(|controls| controls.send(TableCommand::Browse(ticket, payload)))
                    .map(|()| (ticket, request_id))
                    .map_err(str::to_owned)
            });
        match request {
            Ok((ticket, request_id)) => {
                self.pending_detail = Some((ticket, generation, request_id));
                self.busy = true;
                self.status = "Loading related row".into();
            }
            Err(error) => {
                detail.settle(generation, 0, Err(error.clone()));
                self.status = error;
            }
        }
    }
    fn detail_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = current(self.connection.as_deref(), &self.state, self.model.as_ref());
        let Some(detail) = &mut self.detail else {
            return;
        };
        let result = match &current {
            Some(current) => detail.back(current),
            None => Err(relationship_detail::Stale::Connection),
        };
        match result {
            Ok(Back::Frame) => {
                self.references = None;
                self.status = format!("Returned to related row via {}", detail.top().constraint);
            }
            Ok(Back::Origin((row, column))) => {
                if self
                    .grid
                    .update(cx, |grid, cx| grid.select_source_cell(row, column, cx))
                {
                    self.close_detail(window, cx);
                    self.status = "Returned to the original row and cell".into();
                } else {
                    self.status = "The original column is hidden; show it or close details".into();
                }
            }
            Err(stale) => self.status = stale.message().into(),
        }
    }
    fn close_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .references
            .as_ref()
            .zip(self.detail.as_ref())
            .is_some_and(|(review, detail)| detail.owns(&review.selection))
        {
            self.references = None;
        }
        self.detail = None;
        window.focus(&self.grid.focus_handle(cx), cx);
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable
            || ((self.busy || self.changes.read(cx).navigation_blocked())
                && !matches!(action, Action::Cancel))
        {
            return;
        }
        match action {
            Action::WholeExport => self.open_whole_export(window, cx),
            Action::TableCopy => {
                if let Some(connection) = &self.connection {
                    cx.emit(TableEvent::OpenTableCopy {
                        connection: connection.clone(),
                        schema: self.state.schema.clone(),
                        table: self.state.table.clone(),
                    });
                }
            }
            Action::Structure => {
                if let Some(connection) = &self.connection {
                    cx.emit(TableEvent::OpenStructure {
                        connection: connection.clone(),
                        schema: self.state.schema.clone(),
                        table: self.state.table.clone(),
                    });
                }
            }
            Action::TableSeed => {
                if let Some(connection) = &self.connection {
                    cx.emit(TableEvent::OpenTableSeed {
                        connection: connection.clone(),
                        schema: self.state.schema.clone(),
                        table: self.state.table.clone(),
                    });
                }
            }
            Action::CsvTransfer(direction) => {
                if let Some(connection) = &self.connection {
                    cx.emit(TableEvent::OpenCsvTransfer {
                        connection: connection.clone(),
                        direction,
                        target: dbunk_lib::backend::csv_transfers::CsvTarget {
                            schema: self.state.schema.clone(),
                            table: self.state.table.clone(),
                        },
                    });
                }
            }
            Action::FileJob(operation) => {
                if let Some(connection) = &self.connection {
                    cx.emit(TableEvent::OpenPgTools {
                        connection: connection.clone(),
                        operation,
                        context: Some((self.state.schema.clone(), self.state.table.clone())),
                    });
                }
            }
            Action::ForeignKeys => self.request_references(false, cx),
            Action::DetailForeignKeys => self.request_references(true, cx),
            Action::ShowRelated => self.show_related(window, cx),
            Action::DetailRow => {
                if let Some(detail) = &mut self.detail {
                    detail.next_row();
                }
            }
            Action::DetailColumn(forward) => {
                if let Some(detail) = &mut self.detail {
                    detail.move_column(forward);
                }
            }
            Action::OpenRelated => {
                if let (Some(detail), Some(connection)) = (&self.detail, &self.connection) {
                    let frame = detail.top();
                    if !frame.filters.is_empty() {
                        cx.emit(TableEvent::OpenReference {
                            connection: connection.clone(),
                            schema: frame.schema.clone(),
                            table: frame.table.clone(),
                            filters: frame.filters.clone(),
                        });
                    }
                }
            }
            Action::DetailBack => self.detail_back(window, cx),
            Action::CloseDetail => {
                self.close_detail(window, cx);
                self.status = "Related row details closed; nothing changed".into();
            }
            Action::NextReference => {
                if let Some(review) = &mut self.references {
                    review.next();
                }
            }
            Action::OpenReference => self.open_reference(cx),
            Action::CloseReference => {
                self.references = None;
                window.focus(&self.grid.focus_handle(cx), cx);
            }
            Action::Preferences => self.load_preferences(cx),
            Action::Column(action) => {
                if !self.preferences_ready {
                    return;
                }
                match self.grid.read(cx).change_columns(action) {
                    Ok(patch) => self.save_preferences(patch, cx),
                    Err(error) => self.preferences_status = Some(error.into()),
                }
            }
            Action::AutoFit(all) => {
                if self.preferences_ready {
                    self.grid.update(cx, |grid, cx| grid.auto_fit(all, cx));
                }
            }
            Action::Edit => {
                if let Some((row, column)) = self.grid.read(cx).selected_cell() {
                    self.changes
                        .update(cx, |changes, cx| changes.edit_cell(row, column, window, cx));
                }
            }
            Action::Insert => self
                .changes
                .update(cx, |changes, cx| changes.insert(window, cx)),
            Action::Duplicate => {
                if let Some((row, _)) = self.grid.read(cx).selected_cell() {
                    self.changes
                        .update(cx, |changes, cx| changes.duplicate(row, window, cx));
                }
            }
            Action::Bulk => {
                if let (Some(rows), Some((_, column))) = (
                    self.grid.read(cx).selected_rows(),
                    self.grid.read(cx).selected_cell(),
                ) {
                    if rows.is_empty() || rows.len() > 128 {
                        self.status = "Bulk edit requires 1 to 128 selected retained rows".into();
                    } else {
                        self.changes.update(cx, |changes, cx| {
                            changes.bulk_edit(rows.collect(), column, window, cx)
                        });
                    }
                }
            }
            Action::Delete => {
                if let Some((row, _)) = self.grid.read(cx).selected_cell() {
                    self.changes
                        .update(cx, |changes, cx| changes.delete(row, cx));
                }
            }
            Action::Sort => {
                if let (Some(column), Some(model)) =
                    (self.grid.read(cx).selected_column(), &mut self.model)
                {
                    match model.cycle_sort(&column, false) {
                        Ok(()) => {
                            let sort = model.query().sort.clone();
                            let mut state = self.browse_controls.read(cx).state().clone();
                            state.sort = sort;
                            self.apply_browse(state, true, cx);
                        }
                        Err(error) => self.status = format!("Sort refused: {error:?}"),
                    }
                }
            }
            Action::Connect => self.begin_connect(cx),
            Action::Cancel => {
                if self.whole_export.read.busy() {
                    self.whole_export.read.cancel();
                    if let Some(controls) = &self.controls {
                        controls.cancel();
                    }
                }
                if let Some((_, _, cancelled)) = &mut self.pending_reference {
                    *cancelled = true;
                }
                let deferred_save = self.after_analysis.take().is_some();
                if self.pending_browse_preferences.take().is_some() || deferred_save {
                    self.preferences_status =
                        Some("Cancelled; browse preferences were not saved".into());
                }
                if self.changes.update(cx, |changes, cx| changes.cancel(cx)) {
                    if let Some(controls) = &self.controls {
                        controls.cancel();
                        self.status = "Cancelling".into();
                    }
                } else {
                    self.status = "Cancelled; staged changes retained".into();
                }
                if self.pending_detail.is_some()
                    && self.detail.as_mut().is_some_and(Detail::cancel)
                    && let Some(controls) = &self.controls
                {
                    controls.cancel();
                    self.status = "Cancelling related row; nothing changed".into();
                }
            }
            Action::First => self.browse(PageAction::First, false, cx),
            Action::Previous => self.browse(PageAction::Previous, false, cx),
            Action::Next => self.browse(PageAction::Next, false, cx),
            Action::Last => self.browse(PageAction::Last, false, cx),
            Action::Refresh => self.browse(PageAction::First, true, cx),
            Action::Count => {
                if let (Some(model), Some(controls)) = (&mut self.model, &self.controls) {
                    match model.count() {
                        Ok((ticket, payload)) => {
                            match controls.send(TableCommand::Count(ticket, payload)) {
                                Ok(()) => {
                                    self.busy = true;
                                    self.status = "Counting rows".into();
                                }
                                Err(error) => {
                                    model.failed(ticket);
                                    self.status = error.into();
                                }
                            }
                        }
                        Err(error) => self.status = format!("Count unavailable: {error:?}"),
                    }
                }
            }
            Action::PageSize(page_size) => {
                let mut state = self.browse_controls.read(cx).state().clone();
                state.page_size = page_size;
                self.apply_browse(state, false, cx);
            }
        }
        self.changes.update(cx, |changes, cx| {
            changes.set_enabled(self.editable && !self.busy, cx)
        });
        cx.notify();
    }
    pub fn has_pending(&self) -> bool {
        self.whole_export.config.has_pending()
            || self
                .receiver
                .as_ref()
                .is_some_and(TableReceiver::has_pending)
    }
    pub fn drain_one(&mut self, cx: &mut Context<Self>) -> bool {
        if self.drain_whole_config(cx) {
            return true;
        }
        let Some(message) = self.receiver.as_ref().and_then(TableReceiver::try_recv) else {
            return false;
        };
        match message.into_message() {
            TableMessage::WholeTableExport(id, result) => self.settle_whole_export(id, result, cx),
            TableMessage::ForeignKeys(id, result)
                if self
                    .pending_reference
                    .as_ref()
                    .is_some_and(|(pending, _, _)| *pending == id) =>
            {
                let (_, selection, cancelled) = self.pending_reference.take().unwrap();
                self.busy = false;
                if cancelled {
                    self.status = "Foreign-key lookup cancelled".into();
                } else if !self.reference_current(&selection, cx) {
                    self.status = "Selected cell changed; foreign-key reply discarded".into();
                } else {
                    match result
                        .map_err(|error| format!("Foreign-key lookup failed: {error:?}"))
                        .and_then(|keys| {
                            Navigation::new(selection, keys, self.retained.clone())
                                .map_err(str::to_owned)
                        }) {
                        Ok(review) => {
                            self.status =
                                "Choose a constraint, then open the referenced table".into();
                            self.references = Some(review);
                        }
                        Err(error) => self.status = error,
                    }
                }
            }
            TableMessage::ForeignKeys(..) => {}
            TableMessage::TableDdlObserved(..)
            | TableMessage::TableDdlReviewed(..)
            | TableMessage::TableDdlApplied(..)
            | TableMessage::CompletionColumns(..)
            | TableMessage::MaintenanceReviewed(..)
            | TableMessage::MaintenanceApplied(..)
            | TableMessage::Sequence(..)
            | TableMessage::AdminApplied(..)
            | TableMessage::Admin(..)
            | TableMessage::SchemaReviewed(..)
            | TableMessage::SchemaApplied(..)
            | TableMessage::ServerDetails(..)
            | TableMessage::Overview(..)
            | TableMessage::DdlExport(..)
            | TableMessage::SchemaMap(..)
            | TableMessage::MapPreferencesLoad(..)
            | TableMessage::MapPreferencesSave(..)
            | TableMessage::MapPreferencesReset(..)
            | TableMessage::Catalog(..)
            | TableMessage::Structure(..)
            | TableMessage::Description(..)
            | TableMessage::DropImpact(..) => {
                self.status = "Unexpected catalog reply in table document".into()
            }
            message @ (TableMessage::VirtualKeyLoaded(..)
            | TableMessage::VirtualKeySaved(..)
            | TableMessage::Analysis(..)
            | TableMessage::Reviewed(..)
            | TableMessage::Applied(..)) => self
                .changes
                .update(cx, |changes, cx| changes.consume(message, cx)),
            TableMessage::Opened => self.load_preferences(cx),
            TableMessage::Preferences(result) => {
                self.preferences_ready = false;
                match result
                    .map_err(|error| format!("Table preferences unavailable: {error:?}"))
                    .and_then(|prefs| self.accept_preferences(prefs, cx))
                {
                    Ok(parsed) => {
                        self.preferences_ready = true;
                        self.preferences_status = None;
                        let mut state = if self.restore_query {
                            self.browse_controls.read(cx).state().clone()
                        } else {
                            parsed.state.clone()
                        };
                        state.filter_mode = parsed.state.filter_mode;
                        if let Some(model) = &mut self.model {
                            if let Err(error) =
                                model.set_query(state.query(), &state.raw_filter_text)
                            {
                                self.preferences_ready = false;
                                self.preferences_status =
                                    Some(format!("Stored browse settings refused: {error:?}"));
                            } else {
                                self.state.filters = model.query().filters.clone();
                                self.state.sort = model.query().sort.clone();
                                self.state.page_size = model.query().page_size;
                                self.browse_controls
                                    .update(cx, |controls, cx| controls.set_state(state, cx));
                                cx.emit(TableEvent::Changed);
                            }
                        }
                        self.restore_query = true;
                    }
                    Err(error) => self.preferences_status = Some(error),
                }
                self.busy = false;
                self.browse(PageAction::First, false, cx);
            }
            TableMessage::PreferencesSaved(id, result) => {
                if self
                    .pending_preferences
                    .as_ref()
                    .is_some_and(|(pending, _)| *pending == id)
                {
                    let (_, patch) = self.pending_preferences.take().unwrap();
                    self.busy = false;
                    match result
                        .map_err(|error| format!("Table preferences not saved: {error:?}"))
                        .and_then(|prefs| self.accept_preferences(Some(prefs), cx))
                    {
                        Ok(_) => {
                            if let PreferencePatch::Mode(mode) = patch {
                                self.browse_controls
                                    .update(cx, |controls, cx| controls.set_mode(mode, cx));
                            }
                            self.preferences_status = Some("Table preferences saved".into());
                        }
                        Err(error) => self.preferences_status = Some(error),
                    }
                }
            }
            TableMessage::Page(ticket, result)
                if self
                    .pending_detail
                    .as_ref()
                    .is_some_and(|(pending, _, _)| *pending == ticket) =>
            {
                let (_, generation, request_id) = self.pending_detail.take().unwrap();
                self.busy = false;
                let result = result.map_err(|error| format!("Related row failed: {error:?}"));
                let settled = self
                    .detail
                    .as_mut()
                    .is_some_and(|detail| detail.settle(generation, request_id, result));
                self.status = match self.detail.as_ref().map(|detail| &detail.top().state) {
                    Some(State::Failed(error)) if settled => error.clone(),
                    Some(State::NotFound) if settled => "No related row matches".into(),
                    Some(_) if settled => "Related row loaded; read-only".into(),
                    Some(State::Cancelled) => {
                        "Related-row lookup cancelled; nothing changed".into()
                    }
                    _ => "Related-row reply discarded".into(),
                };
            }
            TableMessage::Page(ticket, result) => match result {
                Ok(page) => {
                    let allowed = WORKSPACE_BYTES
                        .saturating_sub(self.retained.get())
                        .saturating_add(self.retained_bytes);
                    if let Some(model) = &mut self.model {
                        match model.receive_page_with_limit(ticket, page, allowed) {
                            Ok(true) => {
                                self.busy = false;
                                let page = model.shared_result().unwrap();
                                let bytes = model.retained_bytes();
                                self.grid
                                    .update(cx, |grid, cx| grid.table_page(page.clone(), cx));
                                self.status = format!(
                                    "Page {} · {} rows · {} ms{}",
                                    model.page(),
                                    page.rows.len(),
                                    page.runtime_ms,
                                    if page.truncated_cells > 0 || page.omitted_rows > 0 {
                                        " · incomplete values"
                                    } else {
                                        ""
                                    }
                                );
                                self.account(bytes);
                                self.browse_controls.update(cx, |controls, cx| {
                                    controls.page(Some(page.clone()), cx)
                                });
                                self.changes.update(cx, |changes, cx| {
                                    changes.page(Some(page), self.controls.clone(), cx);
                                });
                                if self
                                    .pending_browse_preferences
                                    .as_ref()
                                    .is_some_and(|(pending, _, _)| *pending == ticket)
                                {
                                    let (_, state, history) =
                                        self.pending_browse_preferences.take().unwrap();
                                    // Analysis already owns the single request lane. Keep the
                                    // successful browse intent until its acknowledgement, then save.
                                    self.after_analysis =
                                        Some(PreferencePatch::Browse { state, history });
                                }
                            }
                            Ok(false) => {}
                            Err(error) => {
                                self.busy = false;
                                self.status = format!("Page rejected: {error:?}");
                                self.changes
                                    .update(cx, |changes, cx| changes.key_refresh_failed(cx));
                                self.pending_browse_preferences = None;
                            }
                        }
                    }
                }
                Err(error) => {
                    if let Some(model) = &mut self.model
                        && model.failed(ticket)
                    {
                        self.busy = false;
                        self.status = format!("Browse failed: {error:?}");
                        self.changes
                            .update(cx, |changes, cx| changes.key_refresh_failed(cx));
                        self.pending_browse_preferences = None;
                    }
                }
            },
            TableMessage::Count(ticket, result) => match result {
                Ok(count) => {
                    let value = count.value;
                    if let Some(model) = &mut self.model {
                        match model.receive_count(ticket, count) {
                            Ok(true) => {
                                self.busy = false;
                                self.status = format!("{value} matching rows");
                            }
                            Ok(false) => {}
                            Err(error) => {
                                self.busy = false;
                                self.status = format!("Count rejected: {error:?}");
                            }
                        }
                    }
                }
                Err(error) => {
                    if let Some(model) = &mut self.model
                        && model.failed(ticket)
                    {
                        self.busy = false;
                        self.status = format!("Count failed: {error:?}");
                    }
                }
            },
            TableMessage::Error(error) => {
                // Cancellation failure does not settle the original request.
                // Delivery/open failures are followed by an owned close result.
                self.status = format!("Table failed: {error:?}");
                self.delivery_failure = Some(self.status.clone());
            }
            TableMessage::Closed(result) => {
                self.mark_disconnected(cx);
                match result {
                    Ok(DataCloseOutcome::Closed) => {
                        if let Some(error) = &self.delivery_failure {
                            self.status = error.clone();
                        }
                    }
                    Ok(DataCloseOutcome::ConnectionDataClosed) => {
                        self.status =
                            "All table documents on this connection were closed during cleanup"
                                .into()
                    }
                    Err(error) => self.status = format!("Table cleanup failed: {error:?}"),
                }
            }
        }
        if !self.busy
            && !self.changes.read(cx).pending()
            && let Some(patch) = self.after_analysis.take()
        {
            self.save_preferences(patch, cx);
        }
        self.changes.update(cx, |changes, cx| {
            changes.set_enabled(self.editable && !self.busy, cx)
        });
        self.update_controls(cx);
        cx.notify();
        true
    }
    fn render_detail(&mut self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let detail = self.detail.as_ref()?;
        let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
        let value = |value: &Option<String>| match value {
            None => "NULL".to_owned(),
            Some(text) => {
                let mut shown = text.chars().take(256).collect::<String>();
                if shown.len() < text.len() {
                    shown.push('…');
                }
                serde_json::to_string(&shown).unwrap_or_default()
            }
        };
        let frame = detail.top();
        let (schema, table) = detail.origin_table();
        let filters = frame
            .filters
            .iter()
            .filter_map(|filter| match filter {
                BrowseFilter::Comparison { column, value, .. } => Some(format!(
                    "{} = {}",
                    quote(column),
                    serde_json::to_string(value).unwrap_or_default()
                )),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(", ");
        let header = format!(
            "Related step {}/{DEPTH} from {}.{} loaded row {}: {} → {}.{}{}{}",
            detail.depth(),
            quote(schema),
            quote(table),
            detail.origin().cell.0 + 1,
            quote(&frame.constraint),
            quote(&frame.schema),
            quote(&frame.table),
            if filters.is_empty() { "" } else { " where " },
            filters
        );
        let message = match &frame.state {
            State::Loading => "Loading related row…".to_owned(),
            State::Loaded(page) if frame.multiple() => format!(
                "{} rows match{}; the referenced columns do not identify one row",
                page.rows.len(),
                if page.page_info.has_more {
                    " (more exist)"
                } else {
                    ""
                }
            ),
            State::Loaded(page) if page.truncated_cells > 0 => {
                "One related row; some values were truncated. Read-only".to_owned()
            }
            State::Loaded(_) => "One related row. Read-only; staged edits are not used".to_owned(),
            State::NotFound => {
                "No related row matches; it may have been deleted or is not visible".to_owned()
            }
            State::Refused(refusal) => refusal.message().to_owned(),
            State::Failed(error) => error.clone(),
            State::Cancelled => "Related-row lookup cancelled; nothing changed".to_owned(),
        };
        let mut rows = String::new();
        let mut selected = String::new();
        if let Some(page) = frame.page() {
            for (index, row) in page.rows.iter().enumerate() {
                let cells = page
                    .columns
                    .iter()
                    .zip(row)
                    .map(|(column, cell)| format!("{} = {}", quote(&column.name), value(cell)))
                    .collect::<Vec<_>>()
                    .join(", ");
                rows.push_str(&format!("Row {}: {cells}\n", index + 1));
            }
            let (row, column) = frame.cell;
            if let (Some(name), Some(cell)) = (
                page.columns.get(column),
                page.rows.get(row).and_then(|values| values.get(column)),
            ) {
                selected = format!(
                    "Selected related cell: row {}/{}, {} = {}",
                    row + 1,
                    page.rows.len(),
                    quote(&name.name),
                    value(cell)
                );
            }
        }
        let summary = format!("{header}\n{message}");
        let mut panel = div()
            .id("relationship-detail")
            .role(Role::Group)
            .aria_label("Related row details")
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(crate::style::line())
            .child(
                div()
                    .id("relationship-detail-summary")
                    .role(Role::Label)
                    .aria_label(summary.clone())
                    .px_2()
                    .py_1()
                    .child(summary),
            );
        if !rows.is_empty() {
            panel = panel.child(
                div()
                    .id("relationship-detail-rows")
                    .role(Role::Label)
                    .aria_label(rows.clone())
                    .px_2()
                    .font_family(crate::style::MONO)
                    .text_color(crate::style::dim())
                    .max_h(px(160.))
                    .overflow_y_scroll()
                    .child(rows),
            );
        }
        if !selected.is_empty() {
            panel = panel.child(
                div()
                    .id("relationship-detail-cell")
                    .role(Role::Label)
                    .aria_label(selected.clone())
                    .px_2()
                    .font_family(crate::style::MONO)
                    .child(selected),
            );
        }
        let mut buttons = crate::ui::toolbar().border_b_0();
        for (label, action) in DETAIL_BUTTONS.into_iter().zip([
            Action::DetailRow,
            Action::DetailColumn(false),
            Action::DetailColumn(true),
            Action::DetailForeignKeys,
            Action::OpenRelated,
            Action::DetailBack,
            Action::CloseDetail,
        ]) {
            buttons = buttons.child(self.button(label, action, cx));
        }
        Some(panel.child(buttons).into_any_element())
    }
    fn button(&mut self, label: &str, action: Action, cx: &Context<Self>) -> gpui::AnyElement {
        let label = SharedString::from(label.to_owned());
        let focus = self
            .buttons
            .entry(label.to_string())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let enabled = self.editable
            && if matches!(action, Action::Cancel) {
                self.busy || self.changes.read(cx).navigation_blocked()
            } else if matches!(
                action,
                Action::WholeExport
                    | Action::FileJob(_)
                    | Action::CsvTransfer(_)
                    | Action::TableCopy
                    | Action::TableSeed
                    | Action::Structure
            ) {
                self.connection.is_some()
                    && !self.busy
                    && !self.changes.read(cx).navigation_blocked()
            } else if matches!(action, Action::Connect) {
                self.controls.is_none()
            } else {
                !self.busy
                    && !self.changes.read(cx).navigation_blocked()
                    && self.controls.is_some()
                    && (!matches!(action, Action::Column(_) | Action::AutoFit(_))
                        || self.preferences_ready)
                    && match action {
                        Action::Duplicate | Action::Bulk => {
                            self.grid.read(cx).selected_cell().is_some()
                                && self
                                    .model
                                    .as_ref()
                                    .is_some_and(TableDocument::page_is_current)
                        }
                        Action::ForeignKeys => {
                            self.grid.read(cx).selected_cell().is_some()
                                && self
                                    .model
                                    .as_ref()
                                    .is_some_and(TableDocument::page_is_current)
                                && self.after_analysis.is_none()
                                && self.pending_preferences.is_none()
                        }
                        Action::NextReference => self
                            .references
                            .as_ref()
                            .is_some_and(|review| review.choices.len() > 1),
                        Action::OpenReference => self.references.as_ref().is_some_and(|review| {
                            self.selection_matches(&review.selection, cx)
                                && review.choice().target.is_ok()
                        }),
                        Action::CloseReference => self.references.is_some(),
                        Action::ShowRelated => self.references.as_ref().is_some_and(|review| {
                            self.reference_current(&review.selection, cx)
                                && self.detail.as_ref().is_none_or(|detail| {
                                    !detail.owns(&review.selection) || detail.depth() < DEPTH
                                })
                        }),
                        Action::DetailRow => self.detail.as_ref().is_some_and(|detail| {
                            detail.top().page().is_some_and(|page| page.rows.len() > 1)
                        }),
                        Action::DetailColumn(_) => self.detail.as_ref().is_some_and(|detail| {
                            detail
                                .top()
                                .page()
                                .is_some_and(|page| page.columns.len() > 1)
                        }),
                        Action::DetailForeignKeys => {
                            self.detail
                                .as_ref()
                                .is_some_and(|detail| detail.top().page().is_some())
                                && self.after_analysis.is_none()
                                && self.pending_preferences.is_none()
                        }
                        Action::OpenRelated => self
                            .detail
                            .as_ref()
                            .is_some_and(|detail| !detail.top().filters.is_empty()),
                        Action::DetailBack | Action::CloseDetail => self.detail.is_some(),
                        _ => true,
                    }
            };
        let weak = cx.weak_entity();
        if enabled {
            self.tab_order.push(focus.clone());
        }
        let icon = match action {
            Action::Refresh => Some("icons/rotate_cw.svg"),
            Action::Insert => Some("icons/plus.svg"),
            Action::Delete => Some("icons/trash.svg"),
            Action::Count => Some("icons/hash.svg"),
            Action::ForeignKeys => Some("icons/link.svg"),
            Action::WholeExport => Some("icons/download.svg"),
            Action::Structure => Some("icons/list_tree.svg"),
            Action::Connect => Some("icons/power.svg"),
            _ => None,
        };
        let selected = match action {
            Action::PageSize(size) => self
                .model
                .as_ref()
                .is_some_and(|model| model.query().page_size == size),
            _ => false,
        };
        crate::ui::pressed(
            crate::ui::tool_button(label.clone(), label.clone(), icon, enabled, false),
            selected,
        )
        .track_focus(&focus)
        .tab_index(0)
        .tab_stop(enabled)
        .on_click(cx.listener(move |this, _, window, cx| {
            if enabled {
                this.activate(action, window, cx);
            }
        }))
        .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
            if enabled {
                let _ = weak.update(cx, |this, cx| this.activate(action, window, cx));
            }
        })
        .into_any_element()
    }
}
impl Drop for TableView {
    fn drop(&mut self) {
        self.references = None;
        self.pending_reference = None;
        self.detail = None;
        self.pending_detail = None;
        if let Some(controls) = &self.controls {
            controls.stop();
        }
        self.account(0);
        self.retained
            .set(self.retained.get().saturating_sub(self.control_bytes));
    }
}
impl Render for TableView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Disconnect, rebind or restore can remove the detail panel while one
        // of its buttons holds focus; return focus to the grid, never nowhere.
        if self.detail.is_none()
            && DETAIL_BUTTONS.into_iter().any(|label| {
                self.buttons
                    .get(label)
                    .is_some_and(|focus| focus.is_focused(window))
            })
        {
            window.focus(&self.grid.focus_handle(cx), cx);
        }
        if let Some(view) = &self.whole_export.view {
            view.update(cx, |view, cx| {
                view.sync(
                    self.controls.is_some(),
                    self.busy,
                    self.whole_export.read.busy(),
                    self.editable,
                    cx,
                );
                view.sync_config_pending(self.whole_export.config.read.busy(), cx);
            });
            return div().size_full().child(view.clone());
        }
        self.tab_order.clear();
        let mut toolbar = crate::ui::toolbar()
            .child(crate::ui::crumbs(
                format!("{}.", self.state.schema),
                self.state.table.clone(),
            ))
            .child(crate::ui::separator());
        for (label, action) in [
            ("Connect", Action::Connect),
            (
                "Backup",
                Action::FileJob(crate::pg_tool_jobs::Operation::Backup),
            ),
            (
                "Restore",
                Action::FileJob(crate::pg_tool_jobs::Operation::Restore),
            ),
            (
                "Import CSV",
                Action::CsvTransfer(dbunk_lib::backend::csv_transfers::CsvDirection::Import),
            ),
            (
                "Export CSV",
                Action::CsvTransfer(dbunk_lib::backend::csv_transfers::CsvDirection::Export),
            ),
            ("Export table", Action::WholeExport),
            ("Copy table", Action::TableCopy),
            ("Seed table", Action::TableSeed),
            ("Structure", Action::Structure),
            ("Refresh", Action::Refresh),
            ("Edit cell", Action::Edit),
            ("Insert row", Action::Insert),
            ("Duplicate row", Action::Duplicate),
            ("Bulk edit", Action::Bulk),
            ("Delete row", Action::Delete),
            ("Follow foreign key", Action::ForeignKeys),
            ("Count", Action::Count),
            ("Sort selected column", Action::Sort),
            ("Cancel", Action::Cancel),
        ] {
            // Groups follow the existing keyboard order: connection, table
            // tools, then row editing and page actions.
            if matches!(
                action,
                Action::FileJob(_) | Action::Refresh | Action::Cancel
            ) && !matches!(
                action,
                Action::FileJob(crate::pg_tool_jobs::Operation::Restore)
            ) {
                toolbar = toolbar.child(crate::ui::separator());
            }
            toolbar = toolbar.child(self.button(label, action, cx));
        }
        let mut columns = crate::ui::toolbar().child(
            div()
                .pr_1()
                .text_color(crate::style::faint())
                .child("Columns"),
        );
        for (label, action) in [
            ("Narrow column", Action::Column(ColumnAction::Narrow)),
            ("Widen column", Action::Column(ColumnAction::Widen)),
            ("Auto-fit column", Action::AutoFit(false)),
            ("Auto-fit visible columns", Action::AutoFit(true)),
            ("Move column left", Action::Column(ColumnAction::Left)),
            ("Move column right", Action::Column(ColumnAction::Right)),
            (
                "Pin / unpin column",
                Action::Column(ColumnAction::TogglePin),
            ),
            ("Hide column", Action::Column(ColumnAction::Hide)),
            ("Show all columns", Action::Column(ColumnAction::ShowAll)),
            ("Reload preferences", Action::Preferences),
        ] {
            columns = columns.child(self.button(label, action, cx));
        }
        self.update_controls(cx);
        self.tab_order.push(self.grid.focus_handle(cx));
        let mut reference = div().flex().flex_col().flex_shrink_0().text_sm();
        if let Some(review) = &self.references {
            let current = self.reference_current(&review.selection, cx);
            let source = if self
                .detail
                .as_ref()
                .is_some_and(|detail| detail.owns(&review.selection))
            {
                "Related"
            } else {
                "Loaded"
            };
            let choice = review.choice();
            let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
            let mapping = choice
                .key
                .columns
                .iter()
                .zip(&choice.key.referenced_columns)
                .map(|(source, target)| format!("{} → {}", quote(source), quote(target)))
                .collect::<Vec<_>>()
                .join(", ");
            let title = format!(
                "Constraint {}/{}: {} → {}.{} ({mapping})",
                review.selected + 1,
                review.choices.len(),
                quote(&choice.key.name),
                quote(&choice.key.referenced_schema),
                quote(&choice.key.referenced_table)
            );
            let message = if !current {
                "Selected cell changed; load references again"
            } else {
                choice.target.as_ref().err().map_or(
                    "Uses original loaded row values; staged edits are not used",
                    |error| error.message(),
                )
            };
            let values = choice
                .target
                .as_ref()
                .ok()
                .map(|target| {
                    target
                        .filters
                        .iter()
                        .filter_map(|filter| match filter {
                            BrowseFilter::Comparison { column, value, .. } => Some(format!(
                                "{} = {}",
                                quote(column),
                                serde_json::to_string(value).unwrap_or_default()
                            )),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let details = format!(
                "{source} row {}. {title}\n{message}\n{values}",
                review.selection.row + 1
            );
            reference = reference.child(
                div()
                    .id("foreign-key-review")
                    .role(Role::Group)
                    .aria_label("Foreign-key reference from original loaded row")
                    .max_h(px(120.))
                    .overflow_y_scroll()
                    .px_2()
                    .py_1()
                    .border_t_1()
                    .border_color(crate::style::line())
                    .font_family(crate::style::MONO)
                    .text_color(crate::style::dim())
                    .child(
                        div()
                            .id("foreign-key-details")
                            .role(Role::Label)
                            .aria_label(details.clone())
                            .child(details),
                    ),
            );
            let buttons = crate::ui::toolbar()
                .child(self.button("Next constraint", Action::NextReference, cx))
                .child(self.button("Show related row", Action::ShowRelated, cx))
                .child(self.button("Open referenced table", Action::OpenReference, cx))
                .child(self.button("Close reference", Action::CloseReference, cx));
            reference = reference.child(buttons);
        }
        if let Some(panel) = self.render_detail(cx) {
            reference = reference.child(panel);
        }
        let mut paging = div().flex().flex_wrap().items_center().gap(px(2.));
        for (label, action) in [
            ("First", Action::First),
            ("Previous", Action::Previous),
            ("Next", Action::Next),
            ("Last", Action::Last),
        ] {
            paging = paging.child(self.button(label, action, cx));
        }
        paging = paging.child(crate::ui::separator());
        for size in PAGE_SIZES {
            paging = paging.child(self.button(&size.to_string(), Action::PageSize(size), cx));
        }
        let summary = self.model.as_ref().and_then(|model| {
            let result = model.result()?;
            let total = model
                .exact_count()
                .map(|count| (count.value, false))
                .or_else(|| match result.count.kind {
                    BrowseCountKind::Unknown => None,
                    kind => result
                        .count
                        .value
                        .map(|value| (value, kind == BrowseCountKind::Estimated)),
                });
            Some((
                page_summary(
                    model.page(),
                    model.query().page_size,
                    result.rows.len(),
                    total,
                ),
                format!("{} ms", result.runtime_ms),
            ))
        });
        let staged = self.changes.read(cx).staged_len();
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.grid.read(cx).inspector_has_focus(window, cx)
                    || this
                        .changes
                        .update(cx, |changes, cx| changes.composition_active(window, cx))
                    || this
                        .browse_controls
                        .update(cx, |controls, cx| controls.composition_active(window, cx))
                {
                    return;
                }
                let modifiers = event.keystroke.modifiers;
                if event.keystroke.key == "escape"
                    && !modifiers.modified()
                    && this
                        .detail_handles()
                        .iter()
                        .any(|focus| focus.contains_focused(window, cx))
                {
                    // While a related read is in flight Escape cancels it;
                    // closing would be refused by the busy gate.
                    let action = if this.pending_detail.is_some() {
                        Action::Cancel
                    } else {
                        Action::CloseDetail
                    };
                    this.activate(action, window, cx);
                    cx.stop_propagation();
                    return;
                }
                let mut order = this.tab_order.clone();
                if let Some(index) = order
                    .iter()
                    .position(|focus| *focus == this.grid.focus_handle(cx))
                {
                    order.splice(
                        index..index,
                        this.browse_controls.read(cx).focus_handles(cx),
                    );
                }
                if let Some(index) = order
                    .iter()
                    .position(|focus| *focus == this.grid.focus_handle(cx))
                    && let Some(focus) = this.grid.read(cx).export_focus()
                {
                    order.insert(index, focus);
                }
                order.extend(this.changes.read(cx).focus_handles(cx));
                if event.keystroke.key == "tab"
                    && !modifiers.control
                    && !modifiers.alt
                    && !modifiers.platform
                    && !order.is_empty()
                {
                    let current = order
                        .iter()
                        .position(|focus| focus.contains_focused(window, cx));
                    match (current, modifiers.shift) {
                        (Some(0), true) => window.focus_prev(cx),
                        (Some(index), false) if index + 1 == order.len() => window.focus_next(cx),
                        (Some(index), true) => window.focus(&order[index - 1], cx),
                        (Some(index), false) => window.focus(&order[index + 1], cx),
                        (None, true) => window.focus(&order[order.len() - 1], cx),
                        (None, false) => window.focus(&order[0], cx),
                    }
                    cx.stop_propagation();
                }
            }))
            .child(toolbar)
            .child(columns)
            .children(self.preferences_status.as_ref().map(|message| {
                div()
                    .id("column-preferences-status")
                    .role(Role::Status)
                    .aria_label(message.clone())
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(crate::style::dim())
                    .border_b_1()
                    .border_color(crate::style::line_soft())
                    .child(message.clone())
            }))
            .child(self.browse_controls.clone())
            .child(div().flex_1().min_h_0().child(self.grid.clone()))
            .child(reference)
            .child(self.changes.clone())
            .child(
                crate::ui::status_line()
                    .child(paging)
                    .when_some(summary.as_ref(), |footer, (rows, _)| {
                        footer.child(rows.clone())
                    })
                    .when(staged > 0, |footer| {
                        footer.child(div().text_color(crate::style::warn()).child(format!(
                            "{staged} staged change{}",
                            if staged == 1 { "" } else { "s" }
                        )))
                    })
                    .child(
                        div()
                            .id("table-status")
                            .role(Role::Status)
                            .aria_label(self.status.clone())
                            .a11y_synthetic_children(|builder| {
                                builder
                                    .parent_node()
                                    .set_live(gpui::accesskit::Live::Polite)
                            })
                            .flex_1()
                            .min_w_0()
                            .text_color(crate::style::dim())
                            .child(self.status.clone()),
                    )
                    .when_some(summary, |footer, (_, latency)| footer.child(latency)),
            )
    }
}

/// Footer row range, e.g. `rows 101–200 of 18,204 (estimate) · page 2`.
fn page_summary(page: u32, page_size: u32, rows: usize, total: Option<(u64, bool)>) -> String {
    let start = u64::from(page.max(1) - 1) * u64::from(page_size) + 1;
    let range = if rows == 0 {
        "no rows".to_owned()
    } else {
        format!("rows {start}–{}", start + rows as u64 - 1)
    };
    let total = total.map_or(String::new(), |(value, estimate)| {
        format!(
            " of {}{}",
            grouped(value),
            if estimate { " (estimate)" } else { "" }
        )
    });
    format!("{range}{total} · page {}", page.max(1))
}

fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod footer_tests {
    use super::*;

    #[test]
    fn page_summary_reports_one_based_ranges_and_marks_estimates() {
        assert_eq!(
            page_summary(1, 50, 50, Some((18_204, true))),
            "rows 1–50 of 18,204 (estimate) · page 1"
        );
        assert_eq!(
            page_summary(3, 100, 7, Some((207, false))),
            "rows 201–207 of 207 · page 3"
        );
        assert_eq!(page_summary(1, 100, 0, None), "no rows · page 1");
        assert_eq!(grouped(1_000_000), "1,000,000");
        assert_eq!(grouped(999), "999");
    }
}
