//! Staged table writes. Service tokens never survive editing or disconnection;
//! apply cannot reach the worker until the workspace acknowledges its journal.
//! Plan 032: edits open in the grid (inline) or in an anchored popover, and
//! every save goes through an environment-aware review dialog. The backend
//! NeedsConfirmation token remains the enforcement boundary (ADR-0024).
use crate::{
    accessible_editor::AccessibleEditor,
    cell_value::{self, Kind},
    controller::{TableCommand, TableControls, TableMessage},
    data_model::{
        Advance, ApplyResolution, ApplyTicket, CellRef, ConfirmationStep, DraftOverlay, EditSeed,
        ModelError, MutationDraft, OverlayKey, Preconfirmation, ReviewPlan, RowMark, TablePolicy,
        on_needs_confirmation,
    },
};
use dbunk_lib::backend::{WorkspaceMutationDraft, WorkspaceTableState, data::*};
use editor::Editor;
use gpui::{
    AnyView, App, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent,
    Pixels, Role, SharedString, Subscription, Window, div, prelude::*, px,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};
use uuid::Uuid;

pub enum ChangesEvent {
    Changed,
    PersistApply(u64),
    Applied,
    KeyChanged,
    FocusGrid(Vec<FocusHandle>),
    /// An editor opened for `cell`. With `popover`, the host supplies the
    /// cell's window bounds through `set_popover_anchor`.
    EditOpened {
        cell: CellRef,
        source: usize,
        popover: bool,
    },
    /// The editor closed (staged or cancelled); the host moves the grid
    /// selection by `advance` and focuses the grid.
    EditClosed {
        advance: Advance,
    },
    /// Staged tints, inserted rows or the failed-change outline changed.
    OverlayChanged,
}
/// Host-triggered actions (toolbar, overflow menu, notice strip).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChangesCommand {
    Review,
    Discard,
    RetryRecovery,
    Reconcile,
    CancelPending,
    OpenVirtualKey,
    OpenChangeList(Bounds<Pixels>),
}
/// The single notice the table shows above the grid, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangesNotice {
    OutcomeUnknown,
    Unrestored,
    ReadOnlyWithStaged,
    Unavailable(SharedString),
}
/// Cheap, render-ready state for the table toolbar and footer.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangesSummary {
    pub staged: usize,
    pub included: usize,
    pub updates: usize,
    pub inserts: usize,
    pub deletes: usize,
    pub pending: bool,
    pub editing: bool,
    /// A modal dialog (review, discard, virtual key) is open. The change-list
    /// popover is not modal and does not count.
    pub dialog_open: bool,
    pub can_review: Result<(), SharedString>,
    pub notice: Option<ChangesNotice>,
    pub message: SharedString,
}
#[derive(Debug, Clone, Copy)]
enum Action {
    KeyEdit,
    KeyColumn,
    KeyAdd,
    KeyRemove(usize),
    KeySave,
    KeyClear,
    KeyCancel,
    KeyReload,
    RetryRecovery,
    Review,
    Apply,
    Confirm,
    CancelReview,
    CancelPending,
    ReviewAgain,
    CloseDialog,
    Stage,
    BulkColumn,
    Null,
    FormatValue,
    CopyLiteral,
    RawValue,
    CancelEdit,
    Discard,
    ConfirmDiscard,
    CancelDiscard,
    Include(Uuid, bool),
    Remove(Uuid),
    Reconcile,
}
/// Where an open editor renders: in the grid cell, or in an anchored popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presentation {
    Inline,
    Popover,
}
struct Edit {
    row: Option<usize>,
    table_index: usize,
    column: Option<String>,
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    null: bool,
    kind: Option<Kind>,
    raw: bool,
    array: Option<Entity<array_view::ArrayView>>,
    _array_events: Option<gpui::Subscription>,
    context: batch_edit::EditContext,
    history: literal_guard::History,
    _literal_events: Option<gpui::Subscription>,
    /// The grid cell this editor belongs to (table mode).
    cell: Option<(CellRef, usize)>,
    /// Staged insert whose column value this editor sets.
    insert: Option<Uuid>,
    /// The insert cell was DEFAULT and its text has not been edited.
    default: bool,
    presentation: Presentation,
    /// Validation or staging refusal shown inside the popover editor.
    error: Option<String>,
}
impl Edit {
    fn multiline(&self) -> bool {
        self.presentation == Presentation::Popover
    }
}
enum Token {
    Review(MutationReview),
    Confirmation(MutationConfirmation),
    /// Lets cx-free tests build a pending apply; never constructed otherwise.
    #[cfg(test)]
    Test,
}
mod array_view;
mod batch_edit;
mod cell_editor;
mod change_list;
mod key_dialog;
mod review;
mod source;
use source::ChangeSource;
mod literal_guard;
mod retention;
use crate::apply_flow::ApplyFlow;
struct PendingApply {
    ticket: ApplyTicket,
    flow: ApplyFlow<Token>,
    /// What the user's click acknowledged, recorded before dispatch.
    preconfirmed: Preconfirmation,
    /// Backend confirmations sent automatically for this apply (at most one).
    auto_confirms: u32,
}
/// Overlay-layer content. At most one is open. The review dialog stays open
/// from Review until the apply succeeds, fails or is cancelled.
enum Dialog {
    Review(review::ReviewDialog),
    Discard,
    VirtualKey,
    ChangeList(Bounds<Pixels>),
}
impl Dialog {
    fn modal(&self) -> bool {
        !matches!(self, Self::ChangeList(_))
    }
}

const NO_IDENTITY: &str = "No row identity. Choose ⋯ › Virtual key… to edit";
const CHECKING: &str = "Checking editable columns…";

/// Short, value-free text for a refused draft operation.
fn model_error_text(error: ModelError) -> &'static str {
    match error {
        ModelError::Budget => "the 128-change or 4 MiB draft limit was reached",
        ModelError::InvalidInput => "the row or column is no longer valid",
        ModelError::InvalidReply => "the database reply was inconsistent",
        ModelError::Unavailable => "this row or column cannot be changed",
        ModelError::Stale => "the table changed; wait for the editable columns check",
        ModelError::Applying => "an apply is in progress",
        ModelError::AmbiguousIdentity => {
            "the row identity is ambiguous; refresh the page before editing"
        }
        ModelError::OutcomeUnknown => "the previous apply outcome is unknown",
    }
}
fn refusal(action: &str, error: ModelError) -> SharedString {
    format!("{action} refused: {}", model_error_text(error)).into()
}

const KEY_BYTES: usize = 16 * 1024;
const KEY_RESERVATION: usize = 128 * 1024;
#[derive(Default)]
struct KeyEditor {
    loaded: bool,
    stored: Option<VirtualKey>,
    columns: Vec<String>,
    cursor: usize,
    editing: bool,
    refreshing: bool,
    pending: Option<(u64, bool)>,
    status: String,
}
fn valid_key_columns(columns: &[String]) -> bool {
    #[derive(serde::Serialize)]
    struct Claim<'a> {
        version: u32,
        columns: &'a [String],
    }

    !columns.is_empty()
        && columns.len() <= 64
        && crate::results::encoded_size(&Claim {
            version: 1,
            columns,
        }) <= KEY_BYTES
        && columns
            .iter()
            .enumerate()
            .all(|(index, column)| !column.is_empty() && !columns[..index].contains(column))
}
fn source_key_columns<'a>(
    analysis: &'a AnalyzeResultSetResult,
    relation: &MutationTable,
) -> Vec<&'a str> {
    analysis
        .columns
        .iter()
        .filter_map(|column| match &column.origin {
            ColumnOrigin::Table {
                schema,
                table,
                column,
                attnum,
            } if *attnum > 0 && schema == &relation.schema && table == &relation.table => {
                Some(column.as_str())
            }
            _ => None,
        })
        .collect()
}
pub struct TableChanges {
    source: ChangeSource,
    budget: Rc<Cell<usize>>,
    token_bytes: usize,
    key_bytes: usize,
    key: KeyEditor,
    analysis_bytes: usize,
    draft_bytes: usize,
    work_bytes: usize,
    draft: Option<MutationDraft>,
    unrestored: Option<WorkspaceMutationDraft>,
    analysis: Option<AnalyzeResultSetResult>,
    page: Option<Rc<BrowseTableResult>>,
    controls: Option<TableControls>,
    sequence: u64,
    analyzing: Option<u64>,
    reviewing: Option<(u64, ReviewPlan)>,
    review: Option<(ReviewPlan, MutationReview)>,
    applying: Option<PendingApply>,
    edit: Option<Edit>,
    enabled: bool,
    message: String,
    buttons: HashMap<String, FocusHandle>,
    visible_buttons: Vec<FocusHandle>,
    rendered_buttons: Vec<FocusHandle>,
    policy: TablePolicy,
    cell_editor: Option<(Entity<cell_editor::CellEditor>, Subscription)>,
    dialog: Option<Dialog>,
    overlay_cache: RefCell<Option<(OverlayKey, Option<Uuid>, Rc<DraftOverlay>)>>,
    last_failed: Option<Uuid>,
    popover_anchor: Option<Bounds<Pixels>>,
    /// Why editing is unavailable after the last analysis, if it is.
    unavailable: Option<SharedString>,
    /// Focus the open dialog's first control on the next render.
    focus_request: bool,
}
impl EventEmitter<ChangesEvent> for TableChanges {}
impl TableChanges {
    pub fn new(
        saved: &WorkspaceTableState,
        saved_draft: Option<WorkspaceMutationDraft>,
        budget: Rc<Cell<usize>>,
    ) -> Self {
        Self::create(
            ChangeSource::Table(MutationTable {
                schema: saved.schema.clone(),
                table: saved.table.clone(),
            }),
            saved_draft,
            budget,
        )
    }
    fn create(
        source: ChangeSource,
        saved_draft: Option<WorkspaceMutationDraft>,
        budget: Rc<Cell<usize>>,
    ) -> Self {
        // The caller transfers already-owned durable intent before cloning table
        // preferences. Charge that payload immediately, then admit its decoding.
        let draft_bytes = saved_draft
            .as_ref()
            .map_or(0, MutationDraft::recovery_bytes);
        budget.set(budget.get().saturating_add(draft_bytes));
        let mut view = Self {
            budget,
            token_bytes: 0,
            key_bytes: 0,
            key: KeyEditor::default(),
            analysis_bytes: 0,
            draft_bytes,
            work_bytes: 0,
            source,
            unrestored: saved_draft,
            draft: None,
            analysis: None,
            page: None,
            controls: None,
            sequence: 0,
            analyzing: None,
            reviewing: None,
            review: None,
            applying: None,
            edit: None,
            enabled: true,
            message: String::new(),
            buttons: HashMap::new(),
            visible_buttons: Vec::new(),
            rendered_buttons: Vec::new(),
            policy: TablePolicy::UNKNOWN,
            cell_editor: None,
            dialog: None,
            overlay_cache: RefCell::new(None),
            last_failed: None,
            popover_anchor: None,
            unavailable: None,
            focus_request: false,
        };
        view.restore_intent();
        view
    }
    pub fn snapshot_bytes(&self) -> usize {
        self.unrestored
            .as_ref()
            .filter(|draft| !draft.changes.is_empty())
            .map_or_else(
                || {
                    self.draft
                        .as_ref()
                        .filter(|draft| !draft.is_empty())
                        .map_or(0, MutationDraft::snapshot_bytes)
                },
                crate::results::encoded_size,
            )
    }
    pub fn snapshot(&self) -> Option<WorkspaceMutationDraft> {
        self.unrestored
            .clone()
            .or_else(|| self.draft.as_ref().map(MutationDraft::snapshot))
            .filter(|draft| !draft.changes.is_empty())
    }
    /// Staged operations, for the table footer.
    pub fn staged_len(&self) -> usize {
        self.draft.as_ref().map_or(0, MutationDraft::len)
    }
    pub fn navigation_blocked(&self) -> bool {
        self.pending() || self.edit.is_some() || self.key.editing || self.modal_open()
    }
    fn modal_open(&self) -> bool {
        self.dialog.as_ref().is_some_and(Dialog::modal)
    }
    pub fn pending(&self) -> bool {
        self.applying.is_some()
            || self.reviewing.is_some()
            || self.analyzing.is_some()
            || self.key.pending.is_some()
            || self.key.refreshing
    }
    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.enabled = enabled;
        cx.notify();
    }
    pub fn page(
        &mut self,
        page: Option<Rc<BrowseTableResult>>,
        controls: Option<TableControls>,
        cx: &mut Context<Self>,
    ) {
        // Captured bulk pages must drop before their source lease.
        self.drop_edit();
        self.finish_work();
        self.page = page;
        self.overlay_cache.borrow_mut().take();
        self.controls = controls;
        self.reset_key();
        if matches!(self.dialog, Some(Dialog::VirtualKey | Dialog::ChangeList(_))) {
            self.dialog = None;
        }
        self.discard_review();
        if let Some(draft) = &mut self.draft {
            draft.invalidate();
        }
        self.reserve(false, 0);
        self.analysis = None;
        if self.page.is_some() && self.controls.is_some() {
            self.analyze(cx);
        }
        cx.emit(ChangesEvent::OverlayChanged);
        cx.notify();
    }
    /// Drops an open editor without staging or emitting. Callers that close an
    /// edit the user can see use `close_edit`.
    fn drop_edit(&mut self) {
        self.edit = None;
        self.cell_editor = None;
        self.popover_anchor = None;
    }
    /// Closes the open editor, returns focus to the grid and tells the host
    /// how to move the selection.
    fn close_edit(&mut self, advance: Advance, cx: &mut Context<Self>) {
        if self.edit.is_none() {
            return;
        }
        self.return_focus(cx);
        self.drop_edit();
        cx.emit(ChangesEvent::EditClosed { advance });
    }
    fn reset_key(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.key_bytes));
        self.key_bytes = 0;
        self.key = KeyEditor::default();
    }
    pub fn key_refresh_failed(&mut self, cx: &mut Context<Self>) {
        if self.key.refreshing {
            self.key.refreshing = false;
            self.key.status =
                "Virtual key saved; row identity refresh failed. Refresh before editing".into();
            cx.notify();
        }
    }
    fn key_draft_clear(&self) -> bool {
        self.unrestored.is_none()
            && self
                .draft
                .as_ref()
                .is_none_or(|draft| draft.is_empty() && !draft.outcome_unknown())
    }
    fn key_available(&self) -> bool {
        self.enabled
            && self.source.relation().is_some()
            && self.controls.is_some()
            && self.page.is_some()
            && !self.pending()
            && self.edit.is_none()
            && self.key_draft_clear()
    }
    fn load_key(&mut self, cx: &mut Context<Self>) {
        let Some(relation) = self.source.relation().cloned() else {
            return;
        };
        if self.controls.is_none() || self.key.pending.is_some() || self.key.refreshing {
            return;
        }
        if self.key_bytes == 0 {
            if self.budget.get() > 128 * 1024 * 1024 - KEY_RESERVATION {
                self.key.status =
                    "Virtual key unavailable: workspace retention budget reached".into();
                return;
            }
            self.budget.set(self.budget.get() + KEY_RESERVATION);
            self.key_bytes = KEY_RESERVATION;
        }
        let id = self.next();
        match self
            .controls
            .as_ref()
            .unwrap()
            .send(TableCommand::LoadVirtualKey(
                id,
                relation.schema.clone(),
                relation.table.clone(),
            )) {
            Ok(()) => {
                self.key.pending = Some((id, false));
                self.key.loaded = false;
                self.key.status = "Loading virtual key".into();
            }
            Err(error) => self.key.status = error.into(),
        }
        cx.notify();
    }
    fn write_key(&mut self, clear: bool, cx: &mut Context<Self>) {
        let Some(relation) = self.source.relation().cloned() else {
            return;
        };
        if !self.key_available() || !self.key.loaded {
            return;
        }
        if !clear {
            let Some(analysis) = &self.analysis else {
                return;
            };
            let sources = source_key_columns(
                analysis,
                self.source.relation().expect("table key controls"),
            );
            if !valid_key_columns(&self.key.columns)
                || self
                    .key
                    .columns
                    .iter()
                    .any(|column| !sources.contains(&column.as_str()))
            {
                self.key.status = "Choose 1 to 64 source columns within 16 KiB".into();
                return;
            }
        }
        let id = self.next();
        let columns = if clear {
            None
        } else {
            Some(self.key.columns.clone())
        };
        match self
            .controls
            .as_ref()
            .unwrap()
            .send(TableCommand::WriteVirtualKey(
                id,
                relation.schema.clone(),
                relation.table.clone(),
                columns,
            )) {
            Ok(()) => {
                self.key.pending = Some((id, true));
                self.key.status = if clear {
                    "Clearing virtual key"
                } else {
                    "Saving virtual key"
                }
                .into();
                self.discard_review();
                self.analysis = None;
                self.reserve(false, 0);
                if let Some(draft) = &mut self.draft {
                    draft.invalidate();
                }
            }
            Err(error) => self.key.status = error.into(),
        }
        cx.notify();
    }
    fn reserve(&mut self, token: bool, bytes: usize) -> bool {
        let current = if token {
            self.token_bytes
        } else {
            self.analysis_bytes
        };
        let maximum = if token { 1024 * 1024 } else { 512 * 1024 };
        let available = (128usize * 1024 * 1024)
            .saturating_sub(self.budget.get())
            .saturating_add(current);
        if bytes > maximum || bytes > available {
            return false;
        }
        self.budget.set(
            self.budget
                .get()
                .saturating_sub(current)
                .saturating_add(bytes),
        );
        if token {
            self.token_bytes = bytes;
        } else {
            self.analysis_bytes = bytes;
        }
        true
    }
    /// Drops an unapplied review. A review dialog that is not applying and
    /// shows no failure closes with it, so it never outlives its token.
    fn discard_review(&mut self) {
        self.review = None;
        self.reviewing = None;
        if self.applying.is_none() {
            self.reserve(true, 0);
            if matches!(&self.dialog, Some(Dialog::Review(dialog)) if dialog.failure.is_none()) {
                self.dialog = None;
            }
        }
    }
    fn return_focus(&self, cx: &mut Context<Self>) {
        let handles = self
            .edit_handles(cx, true)
            .into_iter()
            .chain(self.rendered_buttons.iter().cloned())
            .collect();
        cx.emit(ChangesEvent::FocusGrid(handles));
    }
    fn next(&mut self) -> u64 {
        self.sequence = self
            .sequence
            .checked_add(1)
            .expect("table operation sequence exhausted");
        self.sequence
    }
    fn analyze(&mut self, cx: &mut Context<Self>) {
        if self.applying.is_some() || self.unrestored.is_some() {
            return;
        }
        self.analysis = None;
        if let Some(draft) = &mut self.draft {
            draft.invalidate();
        }
        self.reserve(false, 0);
        self.discard_review();
        self.unavailable = None;
        let id = self.next();
        let payload = AnalyzeResultSetPayload {
            connection_id: String::new(),
            tab_id: String::new(),
            request_id: id,
            source: match self.source.analysis_source() {
                Ok(source) => source,
                Err(error) => {
                    self.unavailable = Some(error.clone().into());
                    self.message = error;
                    cx.notify();
                    return;
                }
            },
            refresh_structure: true,
        };
        match self
            .controls
            .as_ref()
            .ok_or("Table is disconnected")
            .and_then(|controls| controls.send(TableCommand::Analyze(id, payload)))
        {
            Ok(()) => {
                self.analyzing = Some(id);
                self.message = "Checking editable columns".into();
            }
            Err(error) => self.message = error.into(),
        }
        cx.notify();
    }
    pub fn disconnected(&mut self, cx: &mut Context<Self>) {
        let editing = self.edit.is_some();
        if self.disconnect_state() {
            cx.emit(ChangesEvent::Changed);
        }
        if editing {
            cx.emit(ChangesEvent::EditClosed {
                advance: Advance::Stay,
            });
        }
        cx.emit(ChangesEvent::OverlayChanged);
        cx.notify();
    }
    /// The cx-free part of `disconnected`. A dispatched apply settles as
    /// ConnectionLost, which leaves its outcome unknown; nothing is retried.
    /// Returns whether a pending apply was settled.
    fn disconnect_state(&mut self) -> bool {
        self.controls = None;
        self.reset_key();
        self.analysis = None;
        self.analyzing = None;
        self.reviewing = None;
        self.review = None;
        self.drop_edit();
        self.dialog = None;
        let mut settled = false;
        if let Some(pending) = self.applying.take()
            && let Some(draft) = &mut self.draft
        {
            let error = if pending.flow.dispatched() {
                ResultMutationError::ConnectionLost
            } else {
                ResultMutationError::Cancelled
            };
            let _ = draft.finish_apply(pending.ticket, Err(error));
            settled = true;
        }
        if let Some(draft) = &mut self.draft {
            draft.invalidate();
        }
        self.reserve(true, 0);
        self.reserve(false, 0);
        self.finish_work();
        settled
    }
    /// Legacy cell entry point. Table tabs route through `begin_edit`; query
    /// results keep their popover editor.
    pub fn edit_cell(
        &mut self,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_query() {
            if let Err(reason) =
                self.begin_edit(CellRef::Page(row), column, EditSeed::Keep, window, cx)
            {
                self.message = reason.to_string();
                cx.notify();
            }
            return;
        }
        if !self.can_edit() || !self.admit_work() {
            cx.notify();
            return;
        }
        let Some(rows) = self.captured_rows() else {
            self.finish_work();
            return;
        };
        let Some(values) = rows.row(row) else {
            self.finish_work();
            return;
        };
        let target = if self.is_query() {
            crate::query_result::editable_target(
                self.analysis.as_ref().unwrap(),
                values,
                column,
                self.query_provenance()
                    .is_some_and(|provenance| provenance.utf8()),
            )
        } else {
            self.page
                .as_ref()
                .and_then(|page| page.columns.get(column))
                .map(|column| (0, column.name.clone()))
                .ok_or("Selected column is unavailable")
        };
        let (table_index, name) = match target {
            Ok(target) => target,
            Err(error) => {
                self.message = error.into();
                self.finish_work();
                cx.notify();
                return;
            }
        };
        let value = match self
            .draft
            .as_ref()
            .ok_or(crate::data_model::ModelError::Unavailable)
            .and_then(|draft| {
                draft.edit_value(
                    table_index,
                    values,
                    rows.hidden(row),
                    rows.truncated(),
                    column,
                )
            }) {
            Ok(value) => value,
            Err(error) => {
                self.message = format!("Cell editing refused: {error:?}");
                self.finish_work();
                cx.notify();
                return;
            }
        };
        if value
            .as_ref()
            .is_some_and(|value| value.len() > cell_value::MAX_VALUE_BYTES)
        {
            self.message =
                "Cell exceeds the 1 MiB editor limit; original value was retained".into();
            self.finish_work();
            cx.notify();
            return;
        }
        let text = value.clone().unwrap_or_default();
        let null = value.is_none();
        self.open_edit(
            (Some(row), Some(name), table_index),
            text,
            null,
            Presentation::Popover,
            window,
            cx,
        );
    }
    /// Legacy JSON insert editor. Table tabs use `add_row` and the insert band.
    pub fn insert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_query() || !self.can_edit() || self.edit.is_some() || !self.admit_work() {
            cx.notify();
            return;
        }
        self.open_edit(
            (None, None, 0),
            "{}".into(),
            false,
            Presentation::Popover,
            window,
            cx,
        );
    }
    fn open_edit(
        &mut self,
        (row, column, table_index): (Option<usize>, Option<String>, usize),
        mut text: String,
        null: bool,
        presentation: Presentation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.discard_review();
        let kind = column.as_deref().and_then(|name| {
                let analysis = self.analysis.as_ref()?;
                let target = analysis.tables.get(table_index)?;
                analysis.columns.iter().find(|column| matches!(&column.origin, ColumnOrigin::Table {schema, table, column, ..} if schema == &target.schema && table == &target.table && column == name))
            })
            .and_then(|column| {
                let kind = cell_value::classify(Some(&column.cast_type));
                // Unknown element delimiters remain raw. Applying the comma
                // formatter to box[] or a custom base type could change data.
                if kind == Some(Kind::Array) && !comma_array(&column.cast_type) {
                    None
                } else {
                    kind
                }
            });
        if null && kind == Some(Kind::Array) {
            text = "{}".into();
        }
        let raw = kind == Some(Kind::Array) && cell_value::parse_array(&text).is_err();
        let multiline = presentation == Presentation::Popover;
        let history = literal_guard::History::new(text.clone());
        let editor = cx.new(|cx| {
            let mut editor = if multiline {
                Editor::for_buffer(
                    cx.new(|cx| language::Buffer::local("", cx)),
                    None,
                    window,
                    cx,
                )
            } else {
                Editor::single_line(window, cx)
            };
            editor.set_text(text, window, cx);
            editor
        });
        let label = column.as_ref().map_or_else(
            || "Insert row JSON, omitted columns use defaults".to_string(),
            |column| format!("Value for {column}"),
        );
        let accessible = cx.new(|cx| {
            if multiline {
                AccessibleEditor::new(editor.clone(), label, cx)
            } else {
                AccessibleEditor::field(editor.clone(), label, false, cx)
            }
        });
        window.focus(&editor.focus_handle(cx), cx);
        self.edit = Some(Edit {
            row,
            table_index,
            column,
            editor,
            accessible,
            null,
            kind,
            raw,
            array: None,
            _array_events: None,
            context: batch_edit::EditContext::Ordinary,
            history,
            _literal_events: None,
            cell: None,
            insert: None,
            default: false,
            presentation,
            error: None,
        });
        self.install_literal_guard(window, cx);
        if kind == Some(Kind::Array) && !raw && multiline {
            self.open_array(window, cx);
        }
        cx.notify();
    }
    fn open_array(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.edit else {
            return;
        };
        if edit.editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
        }) {
            self.message = "Finish composing the literal before switching to elements".into();
            return;
        }
        if edit.editor.read(cx).buffer().read(cx).len(cx).0 > cell_value::MAX_VALUE_BYTES {
            self.message = "Array literal exceeds 1 MiB; original text retained".into();
            return;
        }
        let text = edit.editor.read(cx).text(cx);
        match array_view::create(&text, self.budget.clone(), window, cx) {
            Ok(array) => {
                let events = cx.subscribe_in(&array, window, |this, _, event, window, cx| {
                    this.activate(
                        match event {
                            array_view::ArrayEvent::Stage => Action::Stage,
                            array_view::ArrayEvent::Cancel => Action::CancelEdit,
                        },
                        window,
                        cx,
                    );
                });
                array.update(cx, |array, cx| array.focus(window, cx));
                let edit = self.edit.as_mut().unwrap();
                edit.array = Some(array);
                edit._array_events = Some(events);
                edit.raw = false;
            }
            Err(error) => {
                self.edit.as_mut().unwrap().raw = true;
                self.message = error;
            }
        }
    }
    fn toggle_raw(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.edit else {
            return;
        };
        if edit.kind != Some(Kind::Array) {
            self.edit.as_mut().unwrap().raw = !edit.raw;
            return;
        }
        if let Some(array) = edit.array.clone() {
            match array.update(cx, |array, cx| array.replacement(window, cx)) {
                Ok(replacement) => {
                    let edit = self.edit.as_mut().unwrap();
                    if let Some(text) = replacement {
                        let (editor, accessible) = array_view::literal_editor(
                            text,
                            format!("Value for {}", edit.column.as_deref().unwrap_or("cell")),
                            window,
                            cx,
                        );
                        edit.editor = editor;
                        edit.accessible = accessible;
                        edit.history = literal_guard::History::new(edit.editor.read(cx).text(cx));
                    }
                    edit.array = None;
                    edit._array_events = None;
                    edit.raw = true;
                    window.focus(&edit.editor.focus_handle(cx), cx);
                    self.install_literal_guard(window, cx);
                }
                Err(error) => self.message = error,
            }
        } else {
            self.open_array(window, cx);
        }
    }
    pub fn composition_active(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(edit) = &self.edit else {
            return false;
        };
        if let Some(array) = edit.array.clone() {
            array.update(cx, |array, cx| array.composition_active(window, cx))
        } else {
            edit.editor.focus_handle(cx).contains_focused(window, cx)
                && edit.editor.update(cx, |editor, cx| {
                    gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
                })
        }
    }
    fn edit_handles(&self, cx: &App, disabled: bool) -> Vec<FocusHandle> {
        self.edit.as_ref().map_or_else(Vec::new, |edit| {
            edit.array.as_ref().map_or_else(
                || vec![edit.editor.focus_handle(cx)],
                |array| array.read(cx).handles(cx, disabled),
            )
        })
    }
    pub fn delete(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.is_query() || !self.can_edit() || !self.admit_work() {
            cx.notify();
            return;
        }
        let Some(page) = &self.page else {
            self.finish_work();
            return;
        };
        let Some(values) = page.rows.get(row) else {
            self.finish_work();
            return;
        };
        let hidden = page
            .row_identity
            .as_ref()
            .and_then(|rows| rows.get(row))
            .map(Vec::as_slice);
        let result = self
            .draft
            .as_mut()
            .ok_or(crate::data_model::ModelError::Unavailable)
            .and_then(|draft| draft.stage_delete(0, values, hidden, page.truncated_cells > 0));
        self.changed(result.map(|_| ()), cx);
    }
    fn can_edit(&self) -> bool {
        self.can_edit_now().is_ok()
    }
    /// Why a new edit, insert, delete or duplicate is refused right now. The
    /// first applicable reason wins, so read-only always explains itself.
    pub fn can_edit_now(&self) -> Result<(), SharedString> {
        if let Some(reason) = self.policy.read_only_reason() {
            return Err(reason.into());
        }
        if self.unrestored.is_some() {
            return Err("Saved changes need recovery before editing. Retry or discard them".into());
        }
        if self.applying.is_none()
            && self
                .draft
                .as_ref()
                .is_some_and(MutationDraft::outcome_unknown)
        {
            return Err(
                "The previous apply outcome is unknown. Refresh, then Mark resolved".into(),
            );
        }
        if self.applying.is_some() || self.reviewing.is_some() {
            return Err("Changes are being reviewed or applied".into());
        }
        if self.modal_open() {
            return Err("Close the open dialog first".into());
        }
        if self.key.editing || self.key.pending.is_some() || self.key.refreshing {
            return Err("The virtual key is being updated".into());
        }
        if !self.enabled {
            return Err("The table is busy".into());
        }
        if self.controls.is_none() {
            return Err("The table is disconnected".into());
        }
        if self.analyzing.is_some() {
            return Err(CHECKING.into());
        }
        if self.analysis.is_none() || self.draft.is_none() {
            return Err(self.unavailable.clone().unwrap_or_else(|| CHECKING.into()));
        }
        Ok(())
    }
    /// Gate for removing or excluding staged changes. Unlike editing, this
    /// needs no fresh analysis and stays available on read-only connections.
    fn can_select_now(&self) -> Result<(), SharedString> {
        if self.unrestored.is_some() {
            return Err("Saved changes need recovery first".into());
        }
        if self.applying.is_some() || self.reviewing.is_some() {
            return Err("Changes are being reviewed or applied".into());
        }
        if self.modal_open() {
            return Err("Close the open dialog first".into());
        }
        if self.edit.is_some() {
            return Err("Save or cancel the open edit first".into());
        }
        if !self.enabled {
            return Err("The table is busy".into());
        }
        Ok(())
    }
    fn changed(&mut self, result: Result<(), ModelError>, cx: &mut Context<Self>) {
        match result {
            Ok(()) => {
                self.discard_review();
                self.message = "Changes staged".into();
                cx.emit(ChangesEvent::Changed);
                cx.emit(ChangesEvent::OverlayChanged);
            }
            Err(error) => {
                self.message = format!("Change refused: {}", model_error_text(error));
            }
        };
        self.finish_work();
        cx.notify();
    }
    /// Validates and stages the open editor. On success the editor closes and
    /// the host moves the selection by `advance`; on failure the draft text
    /// stays in the editor with the reason.
    fn stage(&mut self, advance: Advance, window: &mut Window, cx: &mut Context<Self>) {
        if self.composition_active(window, cx) {
            self.message = "Finish composing the value before staging".into();
            cx.notify();
            return;
        }
        if !self.edit.as_ref().is_none_or(|edit| {
            edit.context
                .current(self.page.as_ref(), self.analysis.as_ref())
        }) {
            self.edit_refused("Source changed; reopen this edit on the current page", cx);
            return;
        }
        let Some(edit) = &self.edit else {
            return;
        };
        if edit.editor.read(cx).buffer().read(cx).len(cx).0 > cell_value::MAX_VALUE_BYTES {
            self.edit_refused("Cell input exceeds 1 MiB; text was retained", cx);
            return;
        }
        // An element edit in the array view also replaces a DEFAULT insert cell.
        let mut default = edit.default;
        let text = if !edit.null
            && let Some(array) = edit.array.clone()
        {
            match array.update(cx, |array, cx| array.replacement(window, cx)) {
                Ok(Some(text)) => {
                    default = false;
                    text
                }
                Ok(None) => edit.editor.read(cx).text(cx),
                Err(error) => {
                    self.edit_refused(&error, cx);
                    return;
                }
            }
        } else {
            edit.editor.read(cx).text(cx)
        };
        if !edit.null && !edit.raw && !default {
            let validation = match edit.kind {
                Some(Kind::Json) => cell_value::validate_json(&text),
                Some(Kind::Array) => cell_value::parse_array(&text).map(|_| ()),
                Some(Kind::Geometry) => cell_value::check_wkt_prefix(&text),
                None => Ok(()),
            };
            if let Err(error) = validation {
                self.edit_refused(&error.to_string(), cx);
                return;
            }
        }
        if matches!(edit.context, batch_edit::EditContext::Bulk(_)) {
            self.stage_bulk(text, cx);
            return;
        }
        let rows = self.captured_rows();
        let result = if let (Some(id), Some(column)) = (edit.insert, edit.column.as_ref()) {
            // DEFAULT stays omitted until the text is edited; NULL is explicit.
            let value = if default {
                None
            } else if edit.null {
                Some(None)
            } else {
                Some(Some(text))
            };
            self.draft
                .as_mut()
                .ok_or(ModelError::Unavailable)
                .and_then(|draft| draft.set_insert_value(id, column, value))
        } else if let (Some(row), Some(column), Some(rows)) =
            (edit.row, edit.column.as_ref(), rows)
        {
            let Some(values) = rows.row(row) else {
                self.edit_refused("Source row is unavailable", cx);
                return;
            };
            self.draft
                .as_mut()
                .ok_or(ModelError::Unavailable)
                .and_then(|draft| {
                    draft.stage_update(
                        edit.table_index,
                        values,
                        rows.hidden(row),
                        rows.truncated(),
                        vec![MutationValue {
                            column: column.clone(),
                            value: if edit.null { None } else { Some(text) },
                        }],
                    )
                })
        } else {
            if self.is_query() {
                self.edit_refused("Source result is unavailable; no changes staged", cx);
                return;
            }
            match insert_values(&text) {
                Ok(values) => self
                    .draft
                    .as_mut()
                    .ok_or(ModelError::Unavailable)
                    .and_then(|draft| draft.stage_insert(0, values).map(|_| ())),
                Err(error) => {
                    self.edit_refused(&error, cx);
                    return;
                }
            }
        };
        let staged = result.is_ok();
        if staged {
            self.close_edit(advance, cx);
        }
        self.changed(result, cx);
        if !staged && let Some(edit) = &mut self.edit {
            edit.error = Some(self.message.clone());
        }
    }
    /// Keeps the editor open with its draft text and shows why it was refused.
    fn edit_refused(&mut self, message: &str, cx: &mut Context<Self>) {
        self.message = message.to_owned();
        if let Some(edit) = &mut self.edit {
            edit.error = Some(message.to_owned());
        }
        cx.notify();
    }
    fn format_value(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composition_active(window, cx) {
            self.message = "Finish composing the value before formatting".into();
            cx.notify();
            return;
        }
        let Some(edit) = &self.edit else {
            return;
        };
        if edit.editor.read(cx).buffer().read(cx).len(cx).0 > cell_value::MAX_VALUE_BYTES {
            self.message = "Cell input exceeds 1 MiB; text was retained".into();
            cx.notify();
            return;
        }
        let text = edit.editor.read(cx).text(cx);
        let formatted = match edit.kind {
            Some(Kind::Json) => cell_value::pretty_json(&text),
            Some(Kind::Array) => {
                cell_value::parse_array(&text).and_then(|items| cell_value::format_array(&items))
            }
            _ => return,
        };
        match formatted {
            Ok(formatted) => {
                edit.editor.update(cx, |editor, cx| {
                    editor.transact(window, cx, |editor, window, cx| {
                        editor.select_all(&editor::actions::SelectAll, window, cx);
                        editor.insert(&formatted, window, cx);
                    });
                });
                self.message = "Formatted locally; no change staged".into();
            }
            Err(error) => self.message = error.to_string(),
        }
        cx.notify();
    }

    fn request_review(&mut self, cx: &mut Context<Self>) {
        if self.edit.is_some() || self.unrestored.is_some() || self.pending() {
            return;
        }
        if !self.admit_work() {
            cx.notify();
            return;
        }
        self.discard_review();
        let prepared = self
            .draft
            .as_ref()
            .ok_or(crate::data_model::ModelError::Unavailable)
            .and_then(MutationDraft::review);
        match prepared {
            Ok(plan) => {
                if !self.reserve(
                    true,
                    crate::results::encoded_size(plan.plan()).saturating_mul(2),
                ) {
                    self.message =
                        "Review budget reached; select fewer changes or clear another tab".into();
                    self.finish_work();
                    cx.notify();
                    return;
                }
                let id = self.next();
                let command = TableCommand::Review(id, plan.analysis_id(), plan.plan().clone());
                match self
                    .controls
                    .as_ref()
                    .ok_or("Table is disconnected")
                    .and_then(|controls| controls.send(command))
                {
                    Ok(()) => {
                        self.reviewing = Some((id, plan));
                        self.review = None;
                        self.message = "Preparing SQL review".into();
                    }
                    Err(error) => {
                        self.reserve(true, 0);
                        self.message = error.into();
                    }
                }
            }
            Err(error) => self.message = format!("Review unavailable: {error:?}"),
        }
        self.finish_work();
        cx.notify();
    }
    /// Starts the durable apply. `preconfirmed` records what the user's click
    /// acknowledged; it only ever authorizes one automatic backend
    /// confirmation for this apply.
    fn prepare_apply(&mut self, preconfirmed: Preconfirmation, cx: &mut Context<Self>) {
        if self.unrestored.is_some() || self.policy.read_only {
            return;
        }
        let Some((plan, token)) = self.review.take() else {
            return;
        };
        let result = self
            .draft
            .as_mut()
            .ok_or(ModelError::Unavailable)
            .and_then(|draft| draft.begin_apply(&plan));
        match result {
            Ok(ticket) => {
                let id = self.next();
                self.applying = Some(PendingApply {
                    ticket,
                    flow: ApplyFlow::new(id, Token::Review(token)),
                    preconfirmed,
                    auto_confirms: 0,
                });
                self.last_failed = None;
                self.message = "Saving change recovery record".into();
                cx.emit(ChangesEvent::Changed);
                cx.emit(ChangesEvent::OverlayChanged);
                cx.emit(ChangesEvent::PersistApply(id));
            }
            Err(error) => {
                self.reserve(true, 0);
                self.message = format!("Apply refused: {}", model_error_text(error));
                if let Some(Dialog::Review(dialog)) = &mut self.dialog {
                    dialog.failure = Some(self.message.clone());
                }
            }
        };
        cx.notify();
    }
    pub fn apply_saved(&mut self, id: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        if self
            .applying
            .as_ref()
            .is_none_or(|pending| !pending.flow.waiting_for(id))
        {
            return;
        }
        if let Err(error) = result {
            self.cancel_pending(format!("Apply was not sent: {error}"), cx);
            return;
        }
        let Some(token) = self.applying.as_mut().unwrap().flow.saved(id) else {
            return;
        };
        self.reserve(true, 0);
        let command = match token {
            Token::Review(review) => TableCommand::Apply(id, review),
            Token::Confirmation(confirmation) => TableCommand::Confirm(id, confirmation),
            #[cfg(test)]
            Token::Test => return,
        };
        match self
            .controls
            .as_ref()
            .ok_or("Table is disconnected")
            .and_then(|controls| controls.send(command))
        {
            Ok(()) => self.message = "Applying reviewed changes".into(),
            Err(error) => self.cancel_pending(format!("Apply was not sent: {error}"), cx),
        }
        cx.notify();
    }
    pub fn cancel(&mut self, cx: &mut Context<Self>) -> bool {
        if self.edit.is_some() {
            self.close_edit(Advance::Stay, cx);
            self.finish_work();
            cx.notify();
            return false;
        }
        if let Some(pending) = &mut self.applying
            && pending.flow.cancel_before_dispatch()
        {
            self.cancel_pending(
                "Apply cancelled before dispatch; changes retained".into(),
                cx,
            );
            return false;
        }
        true
    }
    fn cancel_pending(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(pending) = self.applying.take()
            && let Some(draft) = &mut self.draft
        {
            let _ = draft.finish_apply(pending.ticket, Err(ResultMutationError::Cancelled));
        }
        self.reserve(true, 0);
        // The review dialog stays open and explains why nothing was sent.
        if let Some(Dialog::Review(dialog)) = &mut self.dialog {
            dialog.failure = Some(message.clone());
        }
        self.message = message;
        self.return_focus(cx);
        cx.emit(ChangesEvent::Changed);
        cx.notify();
    }
    pub fn consume(&mut self, message: TableMessage, cx: &mut Context<Self>) {
        match message {
            TableMessage::VirtualKeyLoaded(id, result) if self.key.pending == Some((id, false)) => {
                self.key.pending = None;
                match result {
                    Ok(key)
                        if key.as_ref().is_none_or(|key| {
                            key.version == 1 && valid_key_columns(&key.columns)
                        }) =>
                    {
                        self.key.columns = key
                            .as_ref()
                            .map(|key| key.columns.clone())
                            .unwrap_or_default();
                        self.key.stored = key;
                        self.key.loaded = true;
                        self.key.status = String::new();
                    }
                    Ok(_) => {
                        self.key.loaded = false;
                        self.key.status =
                            "Stored virtual key exceeds native limits; key preserved".into();
                    }
                    Err(error) => {
                        self.key.loaded = false;
                        self.key.status = format!("Virtual key unavailable: {error:?}");
                    }
                }
            }
            TableMessage::VirtualKeySaved(id, result) if self.key.pending == Some((id, true)) => {
                self.key.pending = None;
                self.key.editing = false;
                match result {
                    Ok(key) => {
                        self.key.stored = key;
                        self.key.loaded = false;
                        self.key.refreshing = true;
                        self.key.status = "Virtual key saved; refreshing row identity".into();
                        self.return_focus(cx);
                        cx.emit(ChangesEvent::KeyChanged);
                    }
                    Err(error) => {
                        self.key.loaded = false;
                        self.key.status =
                            format!("Virtual key write failed: {error:?}. Refresh before editing.");
                    }
                }
            }
            TableMessage::Analysis(id, result) if self.analyzing == Some(id) => {
                self.analyzing = None;
                match result {
                    Ok(analysis) => {
                        if let AnalysisStatement::NotAnalyzable { reason } = &analysis.statement {
                            self.reserve(false, 0);
                            self.set_unavailable(match reason {
                                NotAnalyzableReason::SessionDependentTypes => "This query includes types that need execution rendering context. Edit the row in a table tab".into(),
                                NotAnalyzableReason::PossibleTempShadowing => "Temporary tables may change this query's target. Result editing is unavailable".into(),
                                NotAnalyzableReason::NoTableOrigins | NotAnalyzableReason::NoProjectedColumns => "This result has no editable base-table columns".into(),
                                NotAnalyzableReason::MultiStatement => "Result editing requires one SQL statement".into(),
                                NotAnalyzableReason::Database { message, .. } => format!("Result editing unavailable: {message}"),
                            });
                            cx.notify();
                            return;
                        }
                        if self.unrestored.is_some() {
                            cx.notify();
                            return;
                        }
                        if !self.reserve(
                            false,
                            crate::results::encoded_size(&analysis).saturating_mul(2),
                        ) {
                            self.set_unavailable(
                                "Analysis retention budget reached; clear another tab".into(),
                            );
                            cx.notify();
                            return;
                        }
                        let compatible = self.source.compatible(self.page.as_deref(), &analysis);
                        let utf8 = self
                            .query_provenance()
                            .is_some_and(|provenance| provenance.utf8());
                        if self.is_query()
                            && self.draft.as_ref().is_some_and(|draft| {
                                draft.changes().any(|(_, _, operation)| {
                                    !crate::query_result::guards_supported(operation, utf8)
                                })
                            })
                        {
                            self.reserve(false, 0);
                            self.set_unavailable("Recovered query guards need execution encoding metadata; saved changes are retained for export and reconciliation".into());
                            cx.notify();
                            return;
                        }
                        if !compatible {
                            self.reserve(false, 0);
                            self.set_unavailable(
                                "Table structure changed; refresh before editing".into(),
                            );
                            cx.notify();
                            return;
                        }
                        let result = if let Some(draft) = &mut self.draft {
                            draft.refresh_analysis(analysis.clone())
                        } else {
                            MutationDraft::new(analysis.clone())
                                .map(|draft| self.draft = Some(draft))
                        };
                        match result {
                            Ok(()) => {
                                self.analysis = Some(analysis);
                                self.unavailable = None;
                                self.message = String::new();
                            }
                            Err(error) => {
                                self.reserve(false, 0);
                                self.set_unavailable(format!(
                                    "Editing unavailable: {}",
                                    model_error_text(error)
                                ));
                            }
                        }
                    }
                    Err(error) => self.set_unavailable(format!("Analysis failed: {error:?}")),
                }
                cx.emit(ChangesEvent::OverlayChanged);
                self.load_key(cx);
            }
            TableMessage::Reviewed(id, result)
                if self
                    .reviewing
                    .as_ref()
                    .is_some_and(|(pending, _)| *pending == id) =>
            {
                let (_, plan) = self.reviewing.take().unwrap();
                match result {
                    Ok(token) => {
                        if !self.reserve(
                            true,
                            token
                                .retained_bytes()
                                .saturating_add(crate::results::encoded_size(plan.plan())),
                        ) {
                            self.reserve(true, 0);
                            self.review_failed("Review exceeds its retention budget; select fewer changes or clear another tab".into());
                            cx.notify();
                            return;
                        }
                        self.reviewed(&plan, token.preview());
                        self.review = Some((plan, token));
                        self.message = "Review the changes and SQL before applying".into();
                    }
                    Err(error) => {
                        self.reserve(true, 0);
                        let message = match &error {
                            DataError::Mutation(error) => {
                                crate::data_model::apply_error_message(error)
                            }
                            error => format!("{error:?}"),
                        };
                        self.review_failed(format!("Review failed: {message}"));
                    }
                }
            }
            TableMessage::Applied(id, result)
                if self.applying.as_ref().is_some_and(|pending| {
                    pending.flow.id() == id && pending.flow.dispatched()
                }) =>
            {
                match result {
                    Ok(MutationSubmission::NeedsConfirmation(token)) => {
                        if self.policy.read_only {
                            self.cancel_pending(
                                "Connection is read-only; nothing was applied and changes are retained"
                                    .into(),
                                cx,
                            );
                            return;
                        }
                        if !self.reserve(true, token.retained_bytes()) {
                            self.cancel_pending(
                                "Confirmation exceeds its retention budget; changes retained"
                                    .into(),
                                cx,
                            );
                            return;
                        }
                        self.needs_confirmation(Token::Confirmation(*token), cx);
                    }
                    result => {
                        let pending = self.applying.take().unwrap();
                        self.reserve(true, 0);
                        let result = match result {
                            Ok(MutationSubmission::Applied(applied)) => Ok(applied),
                            Err(error) => Err(mutation_error(&error)),
                            _ => unreachable!(),
                        };
                        if let Some(draft) = &mut self.draft {
                            match draft.finish_apply(pending.ticket, result) {
                                Ok(ApplyResolution::Applied) => {
                                    self.message = "Changes applied".into();
                                    self.last_failed = None;
                                    if matches!(self.dialog, Some(Dialog::Review(_))) {
                                        self.dialog = None;
                                    }
                                    self.return_focus(cx);
                                    cx.emit(ChangesEvent::Applied);
                                }
                                Ok(ApplyResolution::Failed { change, error }) => {
                                    self.last_failed = change;
                                    self.review_failed(format!(
                                        "Apply failed: {}",
                                        crate::data_model::apply_error_message(&error)
                                    ));
                                }
                                Err(error) => {
                                    self.review_failed(format!(
                                        "Apply outcome requires inspection: {}",
                                        model_error_text(error)
                                    ));
                                }
                            }
                        }
                        cx.emit(ChangesEvent::Changed);
                        cx.emit(ChangesEvent::OverlayChanged);
                    }
                }
            }
            _ => {}
        }
        self.finish_work();
        cx.notify();
    }
    fn set_unavailable(&mut self, message: String) {
        self.unavailable = Some(message.clone().into());
        self.message = message;
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled {
            return;
        }
        match action {
            Action::RetryRecovery if !self.pending() => {
                if self.restore_intent() {
                    self.analyze(cx);
                }
            }
            Action::KeyEdit if self.key_available() && self.key.loaded => {
                self.key.columns = self
                    .key
                    .stored
                    .as_ref()
                    .map(|key| key.columns.clone())
                    .unwrap_or_default();
                self.key.editing = true;
            }
            Action::KeyCancel if !self.pending() => {
                self.return_focus(cx);
                self.key.editing = false;
            }
            Action::KeyReload if self.key_available() => self.load_key(cx),
            Action::KeyColumn if self.key_available() => {
                if let Some(analysis) = &self.analysis {
                    let len = source_key_columns(
                        analysis,
                        self.source.relation().expect("table key controls"),
                    )
                    .len();
                    if len > 0 {
                        self.key.cursor = (self.key.cursor + 1) % len;
                    }
                }
            }
            Action::KeyAdd if self.key_available() => {
                if let Some(analysis) = &self.analysis {
                    let sources = source_key_columns(
                        analysis,
                        self.source.relation().expect("table key controls"),
                    );
                    if let Some(column) = sources.get(self.key.cursor) {
                        let mut columns = self.key.columns.clone();
                        columns.push((*column).to_owned());
                        if valid_key_columns(&columns) {
                            self.key.columns = columns;
                        } else {
                            self.key.status = "Choose distinct columns, up to 64 and 16 KiB".into();
                        }
                    }
                }
            }
            Action::KeyRemove(index) if self.key_available() => {
                if index < self.key.columns.len() {
                    self.key.columns.remove(index);
                }
            }
            Action::KeySave => self.write_key(false, cx),
            Action::KeyClear => self.write_key(true, cx),
            Action::Review if !self.pending() => self.open_review(window, cx),
            Action::ReviewAgain if !self.pending() => {
                self.dialog = None;
                self.open_review(window, cx);
            }
            // The gate is recomputed here from the live policy; a rendered
            // button never authorizes by itself.
            Action::Apply | Action::Confirm => self.apply_clicked(cx),
            Action::CancelReview => self.cancel_review(cx),
            Action::CloseDialog => self.close_dialog(cx),
            Action::CancelPending if self.pending() => {
                if self.cancel(cx)
                    && let Some(controls) = &self.controls
                {
                    controls.cancel();
                    self.message = "Stopping change operation; waiting for its outcome".into();
                }
            }
            Action::Stage if !self.pending() => self.stage(Advance::Stay, window, cx),
            Action::BulkColumn if !self.pending() => self.cycle_bulk_column(window, cx),
            Action::FormatValue if !self.pending() => self.format_value(window, cx),
            Action::CopyLiteral => {
                // Baseline "Copy EWKT": a quoted literal of the trimmed text.
                // Copying never stages, edits or executes anything.
                if let Some(edit) = &self.edit {
                    let text = edit.editor.read(cx).text(cx);
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(format!(
                        "'{}'",
                        text.trim().replace('\'', "''")
                    )));
                    self.message = "Copied geometry as a SQL literal".into();
                }
            }
            Action::RawValue if !self.pending() => self.toggle_raw(window, cx),
            Action::Null => {
                if let Some(edit) = &mut self.edit {
                    edit.null = !edit.null;
                    edit.default = false;
                    edit.error = None;
                }
            }
            Action::CancelEdit => self.close_edit(Advance::Stay, cx),
            Action::Discard if self.applying.is_none() && self.reviewing.is_none() => {
                self.dialog = Some(Dialog::Discard);
                self.focus_request = true;
            }
            Action::CancelDiscard => {
                self.return_focus(cx);
                if matches!(self.dialog, Some(Dialog::Discard)) {
                    self.dialog = None;
                }
            }
            Action::Include(id, included) if self.can_select_now().is_ok() && self.admit_work() => {
                let result = self
                    .draft
                    .as_mut()
                    .ok_or(ModelError::Unavailable)
                    .and_then(|draft| draft.include(id, included));
                self.changed(result, cx);
            }
            Action::Remove(id) if self.can_select_now().is_ok() && self.admit_work() => {
                let result = self
                    .draft
                    .as_mut()
                    .ok_or(ModelError::Unavailable)
                    .and_then(|draft| draft.remove(id));
                if result.is_ok() {
                    self.return_focus(cx);
                }
                self.changed(result, cx);
            }
            Action::ConfirmDiscard if !self.pending() => {
                self.unrestored = None;
                self.draft = self
                    .analysis
                    .clone()
                    .and_then(|analysis| MutationDraft::new(analysis).ok());
                self.discard_review();
                if matches!(self.dialog, Some(Dialog::Discard | Dialog::Review(_))) {
                    self.dialog = None;
                }
                self.last_failed = None;
                self.return_focus(cx);
                self.drop_edit();
                self.analyze(cx);
                cx.emit(ChangesEvent::Changed);
                cx.emit(ChangesEvent::OverlayChanged);
            }
            Action::Reconcile if !self.pending() => {
                if let Some(draft) = &mut self.draft {
                    match draft.mark_outcome_reconciled() {
                        Ok(()) => {
                            self.return_focus(cx);
                            cx.emit(ChangesEvent::Changed);
                            cx.emit(ChangesEvent::OverlayChanged);
                            self.analyze(cx);
                        }
                        Err(error) => {
                            self.message =
                                format!("Recovery refused: {}", model_error_text(error));
                        }
                    }
                }
            }
            _ => {}
        }
        self.finish_work();
        cx.notify();
    }

    pub fn set_policy(&mut self, policy: TablePolicy, cx: &mut Context<Self>) {
        let editing = self.edit.is_some();
        if !self.apply_policy(policy) {
            return;
        }
        if editing && self.edit.is_none() {
            cx.emit(ChangesEvent::EditClosed {
                advance: Advance::Stay,
            });
        }
        cx.notify();
    }
    /// The cx-free part of `set_policy`. Read-only closes any open editor;
    /// an open review keeps its dialog, and its gate follows the new policy
    /// on the next render and on every click. Returns whether it changed.
    fn apply_policy(&mut self, policy: TablePolicy) -> bool {
        if self.policy == policy {
            return false;
        }
        self.policy = policy;
        if policy.read_only && self.edit.is_some() {
            self.drop_edit();
            self.finish_work();
            self.message = policy.read_only_reason().unwrap_or_default().to_owned();
        }
        if policy.read_only && matches!(self.dialog, Some(Dialog::VirtualKey)) {
            self.key.editing = false;
            self.dialog = None;
        }
        true
    }
    pub fn policy(&self) -> TablePolicy {
        self.policy
    }
    /// Staged values for the grid, cached by draft owner and revision, page
    /// identity and the failed change, so repeated syncs share one `Rc`.
    pub fn overlay(&self) -> Rc<DraftOverlay> {
        let key = OverlayKey {
            draft: self
                .draft
                .as_ref()
                .map(|draft| (draft.owner(), draft.revision())),
            page: self
                .page
                .as_ref()
                .map_or(0, |page| Rc::as_ptr(page) as usize),
        };
        if let Some((cached, failed, overlay)) = &*self.overlay_cache.borrow()
            && *cached == key
            && *failed == self.last_failed
        {
            return overlay.clone();
        }
        let overlay = Rc::new(match (&self.draft, &self.page) {
            (Some(draft), Some(page)) => draft.overlay(page, key.page, self.last_failed),
            (_, page) => DraftOverlay::empty(page.as_ref().map_or(0, |page| page.rows.len())),
        });
        *self.overlay_cache.borrow_mut() = Some((key, self.last_failed, overlay.clone()));
        overlay
    }
    /// The inline cell editor the grid hosts, if one is open.
    pub fn inline_editor(&self) -> Option<(CellRef, usize, AnyView)> {
        let (editor, _) = self.cell_editor.as_ref()?;
        let (cell, source) = self.edit.as_ref()?.cell?;
        Some((cell, source, editor.clone().into()))
    }
    /// Window bounds of the edited cell, for the popover editor.
    pub fn set_popover_anchor(&mut self, anchor: Option<Bounds<Pixels>>, cx: &mut Context<Self>) {
        if self.popover_anchor != anchor {
            self.popover_anchor = anchor;
            cx.notify();
        }
    }
    pub fn command(&mut self, command: ChangesCommand, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            ChangesCommand::Review => self.activate(Action::Review, window, cx),
            ChangesCommand::Discard => {
                if self.edit.is_some() {
                    self.close_edit(Advance::Stay, cx);
                }
                self.activate(Action::Discard, window, cx);
            }
            ChangesCommand::RetryRecovery => self.activate(Action::RetryRecovery, window, cx),
            ChangesCommand::Reconcile => self.activate(Action::Reconcile, window, cx),
            ChangesCommand::CancelPending => self.activate(Action::CancelPending, window, cx),
            ChangesCommand::OpenVirtualKey => {
                if self.pending() || self.modal_open() || self.edit.is_some() {
                    self.message = "Finish the current edit or operation first".into();
                } else if self.source.relation().is_none() || self.page.is_none() {
                    self.message = "Virtual keys are available for loaded table pages".into();
                } else {
                    self.dialog = Some(Dialog::VirtualKey);
                    self.focus_request = true;
                    if !self.key.loaded && self.key.pending.is_none() && !self.key.refreshing {
                        self.load_key(cx);
                    }
                }
            }
            ChangesCommand::OpenChangeList(anchor) => {
                if matches!(self.dialog, Some(Dialog::ChangeList(_))) {
                    self.dialog = None;
                } else if !self.modal_open() && self.staged_len() > 0 {
                    self.dialog = Some(Dialog::ChangeList(anchor));
                    self.focus_request = true;
                }
            }
        }
        cx.notify();
    }
    fn close_dialog(&mut self, cx: &mut Context<Self>) {
        let review = matches!(self.dialog, Some(Dialog::Review(_)));
        let key = matches!(self.dialog, Some(Dialog::VirtualKey));
        if review {
            self.cancel_review(cx);
        } else if key {
            // A key write in flight keeps its dialog until it settles.
            if self.key.pending.is_none() {
                self.key.editing = false;
                self.dialog = None;
                self.return_focus(cx);
            }
        } else if self.dialog.is_some() {
            self.dialog = None;
            self.return_focus(cx);
        }
    }
    fn review_gate(&self) -> Result<(), SharedString> {
        if let Some(reason) = self.policy.read_only_reason() {
            return Err(reason.into());
        }
        if self.unrestored.is_some() {
            return Err("Retry or discard the saved changes first".into());
        }
        let Some(draft) = &self.draft else {
            return Err("No staged changes".into());
        };
        if self.applying.is_none() && draft.outcome_unknown() {
            return Err(
                "The previous apply outcome is unknown. Refresh, then Mark resolved".into(),
            );
        }
        if !draft.changes().any(|(_, included, _)| included) {
            return Err("No staged changes are selected".into());
        }
        if self.pending() {
            return Err("Wait for the current operation to finish".into());
        }
        if self
            .edit
            .as_ref()
            .is_some_and(|edit| edit.presentation == Presentation::Popover)
        {
            return Err("Save or cancel the open editor first".into());
        }
        if self.controls.is_none() {
            return Err("The table is disconnected".into());
        }
        if !self.enabled {
            return Err("The table is busy".into());
        }
        if self.analysis.is_none() {
            return Err(self.unavailable.clone().unwrap_or_else(|| CHECKING.into()));
        }
        Ok(())
    }
    pub fn summary(&self) -> ChangesSummary {
        let (mut staged, mut included, mut updates, mut inserts, mut deletes) = (0, 0, 0, 0, 0);
        let mut count = |selected: bool, operation: &MutationOp| {
            staged += 1;
            included += usize::from(selected);
            match operation {
                MutationOp::Update { .. } => updates += 1,
                MutationOp::Insert { .. } => inserts += 1,
                MutationOp::Delete { .. } => deletes += 1,
            }
        };
        if let Some(saved) = &self.unrestored {
            for change in &saved.changes {
                count(change.included, &change.operation);
            }
        } else if let Some(draft) = &self.draft {
            for (_, selected, operation) in draft.changes() {
                count(selected, operation);
            }
        }
        let notice = if self.unrestored.is_some() {
            Some(ChangesNotice::Unrestored)
        } else if self.applying.is_none()
            && self
                .draft
                .as_ref()
                .is_some_and(MutationDraft::outcome_unknown)
        {
            Some(ChangesNotice::OutcomeUnknown)
        } else if self.policy.read_only && staged > 0 {
            Some(ChangesNotice::ReadOnlyWithStaged)
        } else {
            self.unavailable.clone().map(ChangesNotice::Unavailable)
        };
        ChangesSummary {
            staged,
            included,
            updates,
            inserts,
            deletes,
            pending: self.pending(),
            editing: self.edit.is_some(),
            dialog_open: self.modal_open(),
            can_review: self.review_gate(),
            notice,
            message: self.message.clone().into(),
        }
    }
}

mod view;
impl Drop for TableChanges {
    fn drop(&mut self) {
        self.reset_key();
        self.reserve(true, 0);
        self.reserve(false, 0);
        self.budget.set(
            self.budget
                .get()
                .saturating_sub(self.draft_bytes)
                .saturating_sub(self.work_bytes),
        );
    }
}

fn mutation_error(error: &DataError) -> ResultMutationError {
    match error {
        DataError::Mutation(error) => error.clone(),
        _ => ResultMutationError::ConnectionLost,
    }
}
fn insert_values(text: &str) -> Result<Vec<MutationValue>, String> {
    if text.len() > 1024 * 1024 {
        return Err("Insert values exceed 1 MiB".into());
    }
    let object = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(text)
        .map_err(|_| "Enter a JSON object; omit columns to use defaults".to_string())?;
    object.into_iter().map(|(column,value)| Ok(MutationValue { column, value: match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text),
        _ => return Err("Use JSON strings for SQL values, or null for SQL NULL; omitted columns use defaults".into()),
    }})).collect()
}

/// Only known built-in comma-delimited array elements enter the structured path.
/// Schema-qualified/custom and box arrays keep their untouched literal editor.
fn comma_array(data_type: &str) -> bool {
    let Some(element) = data_type.trim().strip_suffix("[]") else {
        return false;
    };
    let element = element
        .split('(')
        .next()
        .unwrap_or(element)
        .trim()
        .to_ascii_lowercase();
    matches!(
        element.as_str(),
        "smallint"
            | "integer"
            | "bigint"
            | "int2"
            | "int4"
            | "int8"
            | "real"
            | "double precision"
            | "numeric"
            | "decimal"
            | "boolean"
            | "bool"
            | "text"
            | "character"
            | "character varying"
            | "varchar"
            | "char"
            | "name"
            | "json"
            | "jsonb"
            | "uuid"
            | "bytea"
            | "date"
            | "time"
            | "time without time zone"
            | "time with time zone"
            | "timestamp"
            | "timestamp without time zone"
            | "timestamp with time zone"
            | "interval"
            | "inet"
            | "cidr"
            | "macaddr"
            | "macaddr8"
            | "bit"
            | "bit varying"
            | "money"
            | "point"
            | "line"
            | "lseg"
            | "path"
            | "polygon"
            | "circle"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn virtual_key_selection_uses_source_identity_and_bounds_encoded_claims() {
        let relation = MutationTable {
            schema: "Mixed".into(),
            table: "rows".into(),
        };
        let mut analysis = AnalyzeResultSetResult {
            request_id: 1,
            analysis_id: 1,
            statement: AnalysisStatement::Analyzed,
            tables: vec![],
            columns: vec![AnalyzedColumn {
                name: "alias".into(),
                origin: ColumnOrigin::Table {
                    schema: relation.schema.clone(),
                    table: relation.table.clone(),
                    column: "Actual.Name".into(),
                    attnum: 1,
                },
                cast_type: "text".into(),
                nullable: false,
                writability: ColumnWritability::Writable,
            }],
        };
        assert_eq!(
            source_key_columns(&analysis, &relation),
            vec!["Actual.Name"]
        );
        analysis.columns[0].origin = ColumnOrigin::Expression;
        assert!(source_key_columns(&analysis, &relation).is_empty());
        assert!(valid_key_columns(&["a".into(), "b".into()]));
        assert!(!valid_key_columns(&["a".into(), "a".into()]));
        assert!(!valid_key_columns(&["\0".repeat(KEY_BYTES / 6 + 1)]));
        assert!(!valid_key_columns(
            &(0..65).map(|index| index.to_string()).collect::<Vec<_>>()
        ));
    }
    #[test]
    fn virtual_key_changes_refuse_staged_excluded_and_unknown_drafts() {
        use dbunk_lib::backend::{WorkspaceApplyState, WorkspaceStagedChange};
        let mut state = WorkspaceTableState {
            schema: "public".into(),
            table: "rows".into(),
            filters: vec![],
            sort: vec![],
            page_size: 100,
            draft: None,
        };
        assert!(
            TableChanges::new(&state, state.draft.clone(), Rc::new(Cell::new(0))).key_draft_clear()
        );
        for apply_state in [
            WorkspaceApplyState::Staged,
            WorkspaceApplyState::OutcomeUnknown,
        ] {
            state.draft = Some(WorkspaceMutationDraft {
                apply_state,
                changes: vec![WorkspaceStagedChange {
                    id: uuid::Uuid::new_v4().to_string(),
                    included: false,
                    identity_kind: None,
                    originals: vec![],
                    operation: MutationOp::Insert {
                        table: MutationTable {
                            schema: state.schema.clone(),
                            table: state.table.clone(),
                        },
                        values: vec![MutationValue {
                            column: "value".into(),
                            value: Some("kept".into()),
                        }],
                    },
                }],
            });
            let changes = TableChanges::new(&state, state.draft.clone(), Rc::new(Cell::new(0)));
            assert!(!changes.key_draft_clear());
            assert!(changes.snapshot() == state.draft);
        }
    }
    #[test]
    fn array_normalization_only_accepts_known_comma_delimiters() {
        for name in [
            "text[]",
            "integer[]",
            "numeric(20,4)[]",
            "character varying(32)[]",
        ] {
            assert!(comma_array(name), "{name}");
        }
        for name in ["box[]", "public.text[]", "custom[]", "text[][]", "text"] {
            assert!(!comma_array(name), "{name}");
        }
    }
    #[test]
    fn review_and_analysis_share_result_allowance_without_evicting_existing_data() {
        let limit = 128 * 1024 * 1024;
        let other_results = limit - 700;
        let budget = Rc::new(Cell::new(other_results));
        let state = WorkspaceTableState {
            schema: "public".into(),
            table: "example".into(),
            filters: vec![],
            sort: vec![],
            page_size: 100,
            draft: None,
        };
        let mut first = TableChanges::new(&state, None, budget.clone());
        let mut second = TableChanges::new(&state, None, budget.clone());
        assert!(first.reserve(false, 200));
        assert!(first.reserve(true, 400));
        assert!(!second.reserve(true, 101));
        assert!(!first.reserve(true, 501));
        assert_eq!(budget.get(), limit - 100);
        assert!(first.reserve(true, 300));
        assert!(second.reserve(true, 200));
        assert_eq!(budget.get(), limit);
        drop(first);
        assert_eq!(budget.get(), other_results + 200);
        drop(second);
        assert_eq!(budget.get(), other_results);
    }
    #[test]
    fn inserted_values_preserve_text_null_and_omission_without_numeric_rounding() {
        let values = insert_values(r#"{"name":"東京", "empty":"", "missing":null}"#).unwrap();
        assert_eq!(values.len(), 3);
        assert_eq!(
            values
                .iter()
                .find(|value| value.column == "name")
                .unwrap()
                .value
                .as_deref(),
            Some("東京")
        );
        assert_eq!(
            values
                .iter()
                .find(|value| value.column == "empty")
                .unwrap()
                .value
                .as_deref(),
            Some("")
        );
        assert_eq!(
            values
                .iter()
                .find(|value| value.column == "missing")
                .unwrap()
                .value,
            None
        );
        assert!(insert_values(r#"{"big":123456789012345678901234567890}"#).is_err());
        assert!(insert_values(r#"{"value":true}"#).is_err());
        assert!(insert_values("{}").unwrap().is_empty());
    }

    fn example_state() -> WorkspaceTableState {
        WorkspaceTableState {
            schema: "public".into(),
            table: "example".into(),
            filters: vec![],
            sort: vec![],
            page_size: 100,
            draft: None,
        }
    }
    fn editable_analysis() -> AnalyzeResultSetResult {
        let allowed = || CapabilityVerdict {
            allowed: true,
            reason: None,
        };
        AnalyzeResultSetResult {
            request_id: 1,
            analysis_id: 1,
            statement: AnalysisStatement::Analyzed,
            columns: ["id", "name"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| AnalyzedColumn {
                    name: name.into(),
                    origin: ColumnOrigin::Table {
                        schema: "public".into(),
                        table: "example".into(),
                        column: name.into(),
                        attnum: index as i16 + 1,
                    },
                    cast_type: if index == 0 { "integer" } else { "text" }.into(),
                    nullable: index != 0,
                    writability: ColumnWritability::Writable,
                })
                .collect(),
            tables: vec![AnalyzedTable {
                schema: "public".into(),
                table: "example".into(),
                identity: MutationIdentity {
                    kind: MutationIdentityKind::PrimaryKey,
                    columns: vec!["id".into()],
                },
                identity_projected: true,
                identity_projection_indexes: vec![0],
                updatable: allowed(),
                deletable: allowed(),
                insertable: allowed(),
            }],
        }
    }
    fn example_page() -> BrowseTableResult {
        BrowseTableResult {
            request_id: 1,
            columns: vec![
                BrowseColumn {
                    name: "id".into(),
                    cast_type: "integer".into(),
                    nullable: false,
                },
                BrowseColumn {
                    name: "name".into(),
                    cast_type: "text".into(),
                    nullable: true,
                },
            ],
            rows: vec![vec![Some("1".into()), Some("Ada".into())]],
            identity: BrowseIdentity {
                kind: BrowseIdentityKind::PrimaryKey,
                columns: vec!["id".into()],
            },
            row_identity: None,
            page_info: BrowsePageInfo {
                mode: BrowsePageMode::Offset,
                page: Some(1),
                has_more: false,
                next_cursor: None,
            },
            count: BrowseCount {
                kind: BrowseCountKind::Unknown,
                value: None,
            },
            inspection: BrowseInspection {
                sql: String::new(),
                params: vec![],
            },
            omitted_rows: 0,
            truncated_cells: 0,
            runtime_ms: 0,
        }
    }

    #[test]
    fn read_only_policy_refuses_review_and_editing_with_its_reason() {
        use dbunk_lib::backend::{DevelopmentEnvironment, DevelopmentSafeMode};
        let state = example_state();
        let mut view = TableChanges::new(&state, None, Rc::new(Cell::new(0)));
        assert_eq!(view.policy(), TablePolicy::UNKNOWN);
        let read_only = TablePolicy::resolve(
            DevelopmentEnvironment::Development,
            DevelopmentSafeMode::Inherit,
            true,
        );
        assert!(view.apply_policy(read_only));
        assert!(!view.apply_policy(read_only), "an unchanged policy is a no-op");
        let reason = SharedString::from(read_only.read_only_reason().unwrap());
        assert_eq!(view.can_edit_now(), Err(reason.clone()));
        assert_eq!(view.summary().can_review, Err(reason.clone()));

        // Staged work stays reviewable only after the connection is writable.
        let mut draft = MutationDraft::new(editable_analysis()).unwrap();
        draft.stage_insert(0, vec![]).unwrap();
        view.draft = Some(draft);
        let summary = view.summary();
        assert_eq!(summary.can_review, Err(reason));
        assert_eq!(summary.notice, Some(ChangesNotice::ReadOnlyWithStaged));
        assert_eq!((summary.staged, summary.inserts), (1, 1));
        // Removing staged changes does not need a writable connection.
        assert_eq!(view.can_select_now(), Ok(()));
    }

    #[test]
    fn overlay_is_cached_until_the_draft_revision_page_or_failure_changes() {
        let state = example_state();
        let mut view = TableChanges::new(&state, None, Rc::new(Cell::new(0)));
        view.page = Some(Rc::new(example_page()));
        let mut draft = MutationDraft::new(editable_analysis()).unwrap();
        let id = draft.stage_insert(0, vec![]).unwrap();
        view.draft = Some(draft);
        let first = view.overlay();
        assert!(Rc::ptr_eq(&first, &view.overlay()));
        view.draft.as_mut().unwrap().include(id, false).unwrap();
        let second = view.overlay();
        assert!(!Rc::ptr_eq(&first, &second));
        assert!(Rc::ptr_eq(&second, &view.overlay()));
        view.last_failed = Some(id);
        let third = view.overlay();
        assert!(!Rc::ptr_eq(&second, &third));
        view.page = Some(Rc::new(example_page()));
        view.overlay_cache.borrow_mut().take();
        assert!(!Rc::ptr_eq(&third, &view.overlay()));
    }

    #[test]
    fn disconnect_during_a_dispatched_apply_leaves_the_outcome_unknown() {
        let state = example_state();
        let mut view = TableChanges::new(&state, None, Rc::new(Cell::new(0)));
        let mut draft = MutationDraft::new(editable_analysis()).unwrap();
        draft.stage_insert(0, vec![]).unwrap();
        let plan = draft.review().unwrap();
        let ticket = draft.begin_apply(&plan).unwrap();
        view.draft = Some(draft);
        let mut flow = ApplyFlow::new(1, Token::Test);
        assert!(flow.saved(1).is_some(), "journal acknowledged and dispatched");
        view.applying = Some(PendingApply {
            ticket,
            flow,
            preconfirmed: Preconfirmation::Granted,
            auto_confirms: 0,
        });
        assert!(view.disconnect_state());
        assert!(view.applying.is_none());
        assert!(view.draft.as_ref().unwrap().outcome_unknown());
        let summary = view.summary();
        assert_eq!(summary.notice, Some(ChangesNotice::OutcomeUnknown));
        assert_eq!(summary.staged, 1, "nothing is dropped or retried");
        assert!(summary.can_review.is_err());
        assert!(view.can_edit_now().is_err());
    }

    #[test]
    fn disconnect_before_dispatch_cancels_without_an_unknown_outcome() {
        let state = example_state();
        let mut view = TableChanges::new(&state, None, Rc::new(Cell::new(0)));
        let mut draft = MutationDraft::new(editable_analysis()).unwrap();
        draft.stage_insert(0, vec![]).unwrap();
        let plan = draft.review().unwrap();
        let ticket = draft.begin_apply(&plan).unwrap();
        view.draft = Some(draft);
        view.applying = Some(PendingApply {
            ticket,
            flow: ApplyFlow::new(1, Token::Test),
            preconfirmed: Preconfirmation::NotGranted,
            auto_confirms: 0,
        });
        assert!(view.disconnect_state());
        assert!(!view.draft.as_ref().unwrap().outcome_unknown());
        assert_eq!(view.summary().notice, None);
    }
}
