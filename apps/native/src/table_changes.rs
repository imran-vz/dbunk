//! Staged table writes. Service tokens never survive editing or disconnection;
//! apply cannot reach the worker until the workspace acknowledges its journal.
use crate::{
    accessible_editor::AccessibleEditor,
    cell_value::{self, Kind},
    controller::{TableCommand, TableControls, TableMessage},
    data_model::{ApplyResolution, ApplyTicket, MutationDraft, ReviewPlan},
};
use dbunk_lib::backend::{WorkspaceMutationDraft, WorkspaceTableState, data::*};
use editor::Editor;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, SharedString,
    Window, div, prelude::*, px,
};
use std::{cell::Cell, collections::HashMap, rc::Rc};

pub enum ChangesEvent {
    Changed,
    PersistApply(u64),
    Applied,
    KeyChanged,
    FocusGrid(Vec<FocusHandle>),
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
    Include(uuid::Uuid, bool),
    Remove(uuid::Uuid),
    Reconcile,
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
}
enum Token {
    Review(MutationReview),
    Confirmation(MutationConfirmation),
}
mod array_view;
mod batch_edit;
mod source;
use source::ChangeSource;
mod literal_guard;
mod retention;
use crate::apply_flow::ApplyFlow;
struct PendingApply {
    ticket: ApplyTicket,
    flow: ApplyFlow<Token>,
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
    discard: bool,
    message: String,
    buttons: HashMap<String, FocusHandle>,
    visible_buttons: Vec<FocusHandle>,
    rendered_buttons: Vec<FocusHandle>,
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
            discard: false,
            message: String::new(),
            buttons: HashMap::new(),
            visible_buttons: Vec::new(),
            rendered_buttons: Vec::new(),
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
        self.pending() || self.edit.is_some() || self.key.editing
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
        // Captured bulk/duplicate pages must drop before their source lease.
        self.edit = None;
        self.finish_work();
        self.page = page;
        self.controls = controls;
        self.reset_key();
        self.discard_review();
        if let Some(draft) = &mut self.draft {
            draft.invalidate();
        }
        self.reserve(false, 0);
        self.analysis = None;
        if self.page.is_some() && self.controls.is_some() {
            self.analyze(cx);
        }
        cx.notify();
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
    fn discard_review(&mut self) {
        self.review = None;
        self.reviewing = None;
        if self.applying.is_none() {
            self.reserve(true, 0);
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
        let id = self.next();
        let payload = AnalyzeResultSetPayload {
            connection_id: String::new(),
            tab_id: String::new(),
            request_id: id,
            source: match self.source.analysis_source() {
                Ok(source) => source,
                Err(error) => {
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
        self.controls = None;
        self.reset_key();
        self.analysis = None;
        self.analyzing = None;
        self.reviewing = None;
        self.review = None;
        self.edit = None;
        if let Some(pending) = self.applying.take()
            && let Some(draft) = &mut self.draft
        {
            let error = if pending.flow.dispatched() {
                ResultMutationError::ConnectionLost
            } else {
                ResultMutationError::Cancelled
            };
            let _ = draft.finish_apply(pending.ticket, Err(error));
            cx.emit(ChangesEvent::Changed);
        }
        if let Some(draft) = &mut self.draft {
            draft.invalidate();
        }
        self.reserve(true, 0);
        self.reserve(false, 0);
        self.finish_work();
        cx.notify();
    }
    pub fn edit_cell(
        &mut self,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.open_edit((Some(row), Some(name), table_index), text, null, window, cx);
    }
    pub fn insert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_query() || !self.can_edit() || !self.admit_work() {
            cx.notify();
            return;
        }
        self.open_edit((None, None, 0), "{}".into(), false, window, cx);
    }
    fn open_edit(
        &mut self,
        (row, column, table_index): (Option<usize>, Option<String>, usize),
        mut text: String,
        null: bool,
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
        let multiline = kind.is_some() || row.is_none();
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
        });
        self.install_literal_guard(window, cx);
        if kind == Some(Kind::Array) && !raw {
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
        self.enabled
            && self.unrestored.is_none()
            && !self.pending()
            && !self.key.editing
            && self.controls.is_some()
            && self.analysis.is_some()
    }
    fn changed(
        &mut self,
        result: Result<(), crate::data_model::ModelError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(()) => {
                self.discard_review();
                self.message = "Changes staged".into();
                cx.emit(ChangesEvent::Changed);
            }
            Err(error) => self.message = format!("Change refused: {error:?}"),
        };
        self.finish_work();
        cx.notify();
    }
    fn stage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composition_active(window, cx) {
            self.message = "Finish composing the value before staging".into();
            cx.notify();
            return;
        }
        if !self.edit.as_ref().is_none_or(|edit| {
            edit.context
                .current(self.page.as_ref(), self.analysis.as_ref())
        }) {
            self.message = "Source changed; reopen this edit on the current page".into();
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
        let text = if !edit.null
            && let Some(array) = edit.array.clone()
        {
            match array.update(cx, |array, cx| array.replacement(window, cx)) {
                Ok(Some(text)) => text,
                Ok(None) => edit.editor.read(cx).text(cx),
                Err(error) => {
                    self.message = error;
                    cx.notify();
                    return;
                }
            }
        } else {
            edit.editor.read(cx).text(cx)
        };
        if !edit.null && !edit.raw {
            let validation = match edit.kind {
                Some(Kind::Json) => cell_value::validate_json(&text),
                Some(Kind::Array) => cell_value::parse_array(&text).map(|_| ()),
                Some(Kind::Geometry) => cell_value::check_wkt_prefix(&text),
                None => Ok(()),
            };
            if let Err(error) = validation {
                self.message = error.to_string();
                cx.notify();
                return;
            }
        }
        if matches!(edit.context, batch_edit::EditContext::Bulk(_)) {
            self.stage_bulk(text, cx);
            return;
        }
        let rows = self.captured_rows();
        let result =
            if let (Some(row), Some(column), Some(rows)) = (edit.row, edit.column.as_ref(), rows) {
                let Some(values) = rows.row(row) else {
                    self.message = "Source row is unavailable".into();
                    cx.notify();
                    return;
                };
                self.draft
                    .as_mut()
                    .ok_or(crate::data_model::ModelError::Unavailable)
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
                    self.message = "Source result is unavailable; no changes staged".into();
                    cx.notify();
                    return;
                }
                let values = insert_values(&text);
                match values {
                    Ok(values) => self
                        .draft
                        .as_mut()
                        .ok_or(crate::data_model::ModelError::Unavailable)
                        .and_then(|draft| draft.stage_insert(0, values).map(|_| ())),
                    Err(error) => {
                        self.message = error;
                        cx.notify();
                        return;
                    }
                }
            };
        if result.is_ok() {
            self.return_focus(cx);
            self.edit = None;
        }
        self.changed(result, cx);
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
    fn prepare_apply(&mut self, cx: &mut Context<Self>) {
        if self.unrestored.is_some() {
            return;
        }
        let Some((plan, token)) = self.review.take() else {
            return;
        };
        let result = self
            .draft
            .as_mut()
            .ok_or(crate::data_model::ModelError::Unavailable)
            .and_then(|draft| draft.begin_apply(&plan));
        match result {
            Ok(ticket) => {
                let id = self.next();
                self.applying = Some(PendingApply {
                    ticket,
                    flow: ApplyFlow::new(id, Token::Review(token)),
                });
                self.message = "Saving change recovery record".into();
                cx.emit(ChangesEvent::Changed);
                cx.emit(ChangesEvent::PersistApply(id));
                self.return_focus(cx);
            }
            Err(error) => {
                self.reserve(true, 0);
                self.message = format!("Apply refused: {error:?}");
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
            self.return_focus(cx);
            self.edit = None;
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
                            self.message = match reason {
                                NotAnalyzableReason::SessionDependentTypes => "This query includes types that need execution rendering context. Edit the row in a table tab".into(),
                                NotAnalyzableReason::PossibleTempShadowing => "Temporary tables may change this query's target. Result editing is unavailable".into(),
                                NotAnalyzableReason::NoTableOrigins | NotAnalyzableReason::NoProjectedColumns => "This result has no editable base-table columns".into(),
                                NotAnalyzableReason::MultiStatement => "Result editing requires one SQL statement".into(),
                                NotAnalyzableReason::Database { message, .. } => format!("Result editing unavailable: {message}"),
                            };
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
                            self.message =
                                "Analysis retention budget reached; clear another tab".into();
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
                            self.message = "Recovered query guards need execution encoding metadata; saved changes are retained for export and reconciliation".into();
                            cx.notify();
                            return;
                        }
                        if !compatible {
                            self.reserve(false, 0);
                            self.message = "Table structure changed; refresh before editing".into();
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
                                self.message = String::new();
                            }
                            Err(error) => {
                                self.reserve(false, 0);
                                self.message = format!("Editing unavailable: {error:?}");
                            }
                        }
                    }
                    Err(error) => self.message = format!("Analysis failed: {error:?}"),
                }
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
                            self.message="Review exceeds its retention budget; select fewer changes or clear another tab".into();
                            cx.notify();
                            return;
                        }
                        self.review = Some((plan, token));
                        self.message = "Review the SQL and bound values before applying".into();
                    }
                    Err(error) => {
                        self.reserve(true, 0);
                        self.message = format!("Review failed: {error:?}");
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
                        if !self.reserve(true, token.retained_bytes()) {
                            self.cancel_pending(
                                "Confirmation exceeds its retention budget; changes retained"
                                    .into(),
                                cx,
                            );
                            return;
                        }
                        let pending = self.applying.as_mut().unwrap();
                        pending.flow.needs_confirmation(Token::Confirmation(*token));
                        self.message =
                            "Safe Mode requires confirmation of these exact changes".into();
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
                                    cx.emit(ChangesEvent::Applied);
                                }
                                Ok(ApplyResolution::Failed { change, error }) => {
                                    self.message = format!(
                                        "Apply failed{}: {error:?}",
                                        change.map_or(String::new(), |id| format!(
                                            " for change {id}"
                                        ))
                                    )
                                }
                                Err(error) => {
                                    self.message =
                                        format!("Apply outcome requires inspection: {error:?}")
                                }
                            }
                        }
                        cx.emit(ChangesEvent::Changed);
                    }
                }
            }
            _ => {}
        }
        self.finish_work();
        cx.notify();
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
            Action::Review if !self.pending() => self.request_review(cx),
            Action::Apply if !self.pending() => self.prepare_apply(cx),
            Action::Confirm => {
                if self
                    .applying
                    .as_ref()
                    .is_some_and(|pending| pending.flow.confirming())
                {
                    let id = self.next();
                    if self.applying.as_mut().unwrap().flow.confirm(id) {
                        self.return_focus(cx);
                        cx.emit(ChangesEvent::PersistApply(id));
                    }
                }
            }
            Action::CancelReview => {
                if self
                    .applying
                    .as_ref()
                    .is_some_and(|pending| !pending.flow.dispatched())
                {
                    self.cancel_pending("Confirmation cancelled; changes retained".into(), cx);
                } else if self.applying.is_none() {
                    self.discard_review();
                    self.return_focus(cx);
                }
            }
            Action::CancelPending if self.pending() => {
                if self.cancel(cx)
                    && let Some(controls) = &self.controls
                {
                    controls.cancel();
                    self.message = "Stopping change operation; waiting for its outcome".into();
                }
            }
            Action::Stage if !self.pending() => self.stage(window, cx),
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
                }
            }
            Action::CancelEdit => {
                self.return_focus(cx);
                self.edit = None;
            }
            Action::Discard if self.applying.is_none() => self.discard = true,
            Action::CancelDiscard => {
                self.return_focus(cx);
                self.discard = false;
            }
            Action::Include(id, included)
                if !self.pending() && self.edit.is_none() && self.admit_work() =>
            {
                let result = self
                    .draft
                    .as_mut()
                    .ok_or(crate::data_model::ModelError::Unavailable)
                    .and_then(|draft| draft.include(id, included));
                self.changed(result, cx);
            }
            Action::Remove(id) if !self.pending() && self.edit.is_none() && self.admit_work() => {
                let result = self
                    .draft
                    .as_mut()
                    .ok_or(crate::data_model::ModelError::Unavailable)
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
                self.discard = false;
                self.return_focus(cx);
                self.edit = None;
                self.analyze(cx);
                cx.emit(ChangesEvent::Changed);
            }
            Action::Reconcile if !self.pending() => {
                if let Some(draft) = &mut self.draft {
                    match draft.mark_outcome_reconciled() {
                        Ok(()) => {
                            self.return_focus(cx);
                            cx.emit(ChangesEvent::Changed);
                            self.analyze(cx);
                        }
                        Err(error) => self.message = format!("Recovery refused: {error:?}"),
                    }
                }
            }
            _ => {}
        }
        self.finish_work();
        cx.notify();
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
}
