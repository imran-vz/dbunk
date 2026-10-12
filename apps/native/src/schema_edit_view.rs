//! Plan 033 Structure draft mode, hosted by the PostgreSQL, MySQL and SQLite
//! structure tabs. One table: observe a baseline, edit a draft with live
//! diff tints, then review and apply through `schema_edit_review`. Engine
//! neutral: the host only supplies how backend calls travel ([`Channel`]),
//! whether the draft is journaled with the workspace, and the connection.
use crate::{
    bounded_field::{Changed, Field as TextField},
    controller::{Host, TableCommand, TableControls},
    data_model::TablePolicy,
    schema_edit_model::{
        Attr, Choice, DraftSave, INPUT_BYTES, Input, OutcomeText, Recovery, SchemaEditModel,
        Settlement, Target, policy_for,
    },
    schema_edit_review::{
        ReviewState,
        runtime::{self, SchemaEditCall, SchemaEditReply},
    },
    ui,
};
use dbunk_lib::backend::schema_edit::{
    ChangeGroup, SchemaEditRequest, SchemaEditSource, SchemaEditTarget, WorkspaceSchemaEdit,
};
use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Subscription, Task,
    UniformListScrollHandle, Window,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

mod render;

/// PostgreSQL journals the draft at most once per this interval while it is
/// edited (every journal clones and encodes the whole table), and always
/// before review, close and disconnect.
const JOURNAL_IDLE: Duration = Duration::from_millis(400);

pub enum SchemaEditEvent {
    /// The journaled draft or attempt changed (PostgreSQL persists it).
    Changed,
    /// Save the uncertain attempt durably, then call `apply_saved`.
    PersistApply(u64),
    /// An observation or apply started or settled.
    Activity(bool),
    /// Leave draft mode; the host shows the read-only structure again.
    Close,
    /// The table may have changed; the host refreshes the structure.
    StructureChanged,
}

/// How this editor's backend calls travel.
pub enum Channel {
    /// MySQL and SQLite: the facade is called on the host runtime with the
    /// lane's session. `None` while disconnected.
    Direct(Option<SchemaEditSource>),
    /// PostgreSQL: through the Objects document worker, which owns the
    /// `DataDocument`. `None` while disconnected.
    Worker(Option<TableControls>),
}

pub(crate) struct InputField {
    pub(crate) input: Input,
    pub(crate) field: Entity<TextField>,
    _subscription: Subscription,
}

pub struct SchemaEditView {
    pub(crate) host: Arc<Host>,
    pub(crate) channel: Channel,
    pub(crate) connection: String,
    pub(crate) request: SchemaEditRequest,
    /// PostgreSQL journals the draft and the attempt with the workspace.
    pub(crate) persist: bool,
    /// Request identities shared with the host's other apply flows.
    pub(crate) ids: Rc<Cell<u64>>,
    pub(crate) recovery: Recovery,
    pub(crate) model: Option<SchemaEditModel>,
    /// The captured baseline's authority, held while editing. Review consumes
    /// it; a closed review gives it back.
    pub(crate) target: Option<SchemaEditTarget>,
    pub(crate) observing: Option<u64>,
    pub(crate) policy: TablePolicy,
    /// A read-only session (SQLite opened read-only) overrides the policy.
    pub(crate) session_read_only: bool,
    pub(crate) selected: Target,
    pub(crate) inputs: Vec<InputField>,
    pub(crate) inputs_stale: bool,
    pub(crate) input_error: Option<(Input, String)>,
    pub(crate) review: Option<ReviewState>,
    /// The reconcile Discard was clicked once.
    pub(crate) armed: bool,
    /// Draft too large for the workspace journal: kept in memory only.
    pub(crate) memory_only: bool,
    pub(crate) message: String,
    pub(crate) failure: ui::Failure,
    pub(crate) focus: FocusHandle,
    pub(crate) focus_request: bool,
    /// The virtualized draft table's scroll position.
    pub(crate) scroll: UniformListScrollHandle,
    /// An edit is not journaled yet (PostgreSQL).
    journal_dirty: bool,
    /// The record `pending_draft` builds and its encoded size, kept until
    /// the next edit: workspace saves read it several times per snapshot.
    pending: RefCell<Option<Option<(WorkspaceSchemaEdit, usize)>>>,
    journal_scheduled: bool,
    _journal_timer: Option<Task<()>>,
    tasks: Vec<Task<()>>,
}

impl EventEmitter<SchemaEditEvent> for SchemaEditView {}

impl Focusable for SchemaEditView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SchemaEditView {
    /// Starts observing at once. `restored` is the workspace journal record
    /// (PostgreSQL); it restores the draft only for an unchanged table.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        host: Arc<Host>,
        channel: Channel,
        connection: String,
        request: SchemaEditRequest,
        persist: bool,
        ids: Rc<Cell<u64>>,
        restored: Option<WorkspaceSchemaEdit>,
        session_read_only: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let recovery = Recovery::new(restored, persist);
        let message = if recovery.unknown() {
            "A previous structure change has an unknown outcome. Inspect the table, then reconcile before editing again.".into()
        } else {
            String::new()
        };
        let mut policy = TablePolicy::UNKNOWN;
        policy.read_only = session_read_only;
        let mut view = Self {
            host,
            channel,
            connection,
            request,
            persist,
            ids,
            recovery,
            model: None,
            target: None,
            observing: None,
            policy,
            session_read_only,
            selected: Target::Table,
            inputs: Vec::new(),
            inputs_stale: true,
            input_error: None,
            review: None,
            armed: false,
            memory_only: false,
            message,
            failure: ui::Failure::default(),
            focus: cx.focus_handle(),
            focus_request: true,
            scroll: UniformListScrollHandle::new(),
            journal_dirty: false,
            pending: RefCell::new(None),
            journal_scheduled: false,
            _journal_timer: None,
            tasks: Vec::new(),
        };
        view.load_policy(cx);
        view.observe(cx);
        view
    }

    pub fn request(&self) -> &SchemaEditRequest {
        &self.request
    }

    /// The editor is shown again: re-capture when authority was given up
    /// (after an apply, a reconnect or a cancelled review).
    pub fn resume(&mut self, cx: &mut Context<Self>) {
        if self.review.is_none()
            && self.target.is_none()
            && self.observing.is_none()
            && self.connected()
        {
            self.observe(cx);
        }
        self.focus_request = true;
        cx.notify();
    }

    /// The journal record for the workspace snapshot (PostgreSQL). An edit
    /// still waiting for the journal timer is included, so a save taken in
    /// between (quit, close) never loses it.
    pub fn snapshot(&self) -> Option<WorkspaceSchemaEdit> {
        self.pending_draft(|pending| pending.map(|(record, _)| record.clone()))
            .unwrap_or_else(|| self.recovery.record().cloned())
    }

    pub fn snapshot_bytes(&self) -> usize {
        self.pending_draft(|pending| pending.map_or(0, |(_, bytes)| *bytes))
            .unwrap_or_else(|| {
                self.recovery
                    .record()
                    .map_or(0, crate::results::encoded_size)
            })
    }

    /// Reads what `journal` would store for an edit not journaled yet:
    /// `Some(read(None))` clears the record (unchanged or too large), `None`
    /// keeps the stored one. Built once per edit (see `pending`).
    fn pending_draft<R>(
        &self,
        read: impl FnOnce(Option<&(WorkspaceSchemaEdit, usize)>) -> R,
    ) -> Option<R> {
        if !self.journal_dirty || !self.persist || self.recovery.unknown() {
            return None;
        }
        let model = self.model.as_ref().filter(|model| model.editable())?;
        let mut cache = self.pending.borrow_mut();
        let pending = cache.get_or_insert_with(|| {
            if model.draft().table == model.baseline().table {
                return None;
            }
            WorkspaceSchemaEdit::draft(
                model.request().clone(),
                model.baseline().clone(),
                model.draft().clone(),
            )
            .ok()
            .map(|record| {
                let bytes = crate::results::encoded_size(&record);
                (record, bytes)
            })
        });
        Some(read(pending.as_ref()))
    }

    /// A draft, an attempt or a request is outstanding.
    pub fn has_changes(&self) -> bool {
        self.recovery.record().is_some()
            || self
                .model
                .as_ref()
                .is_some_and(SchemaEditModel::has_changes)
            || self.has_pending()
    }

    /// Something is in flight that only the backend can settle, or a save
    /// fence is pending.
    pub fn has_pending(&self) -> bool {
        self.observing.is_some()
            || self
                .review
                .as_ref()
                .and_then(|review| review.flow.as_ref())
                .is_some_and(|flow| !flow.confirming())
    }

    /// PostgreSQL: the Objects worker's controls and readiness.
    pub fn set_runtime(&mut self, controls: Option<TableControls>, cx: &mut Context<Self>) {
        if let Channel::Worker(current) = &mut self.channel {
            let changed = current.is_some() != controls.is_some();
            *current = controls;
            if changed {
                cx.notify();
            }
        }
    }

    /// MySQL and SQLite: the lane's session, or `None` when disconnected.
    pub fn set_source(&mut self, source: Option<SchemaEditSource>, cx: &mut Context<Self>) {
        let connected = source.is_some();
        if let Channel::Direct(current) = &mut self.channel {
            *current = source;
        }
        if !connected {
            self.disconnected(cx);
        }
        cx.notify();
    }

    pub(crate) fn connected(&self) -> bool {
        match &self.channel {
            Channel::Direct(source) => source.is_some(),
            Channel::Worker(controls) => controls.is_some(),
        }
    }

    /// The lane disconnected: authority is gone; a save fence not yet
    /// released is revoked. A dispatched PostgreSQL apply settles as unknown:
    /// its worker dies with the document, so the reply never comes. A
    /// MySQL or SQLite call runs on the host runtime with its own session and
    /// always replies (`SchemaEditCall::lost` when its task ends), so that
    /// flow stays open until the real receipt settles it.
    pub fn disconnected(&mut self, cx: &mut Context<Self>) {
        self.flush_journal(cx);
        let worker = matches!(self.channel, Channel::Worker(_));
        // A retained worker handle would keep the closed document's worker
        // alive (`ControlHandle` stops it on drop).
        if let Channel::Worker(controls) = &mut self.channel {
            *controls = None;
        }
        self.target = None;
        self.observing = None;
        let dispatched = self
            .review
            .as_ref()
            .and_then(|review| review.flow.as_ref())
            .is_some_and(|flow| flow.dispatched());
        let lost = dispatched && worker;
        self.revoke_before_dispatch();
        // A settled outcome ("Structure change applied") stays on screen: the
        // refresh after a successful apply disconnects the document too.
        let settled = self
            .review
            .as_ref()
            .is_some_and(|review| review.settlement.is_some());
        if let Some(review) = &mut self.review {
            if lost {
                // The reply can no longer arrive; the journal stays unknown.
                review.flow = None;
                review.settlement = Some(Settlement::Unknown);
                review.outcome = Some(OutcomeText {
                    text: "Disconnected while applying, so the outcome is unknown. Inspect the table and reload before another attempt.".into(),
                    error: true,
                });
            } else if review.flow.is_none() && review.review.is_some() {
                review.review = None;
                review.failure = Some("Disconnected. Reconnect and review again.".into());
            }
        }
        if dispatched && !lost {
            self.message =
                "Disconnected while applying; waiting for the outcome of the change already sent."
                    .into();
        } else if self.model.is_some() && !settled {
            self.message =
                "Disconnected. The draft is kept; reconnect and reload to review.".into();
        }
        self.publish(cx);
        if lost {
            cx.emit(SchemaEditEvent::StructureChanged);
        }
    }

    pub(crate) fn next_id(&self) -> Result<u64, &'static str> {
        let id = self
            .ids
            .get()
            .checked_add(1)
            .ok_or("Request identity exhausted; reopen the workspace")?;
        self.ids.set(id);
        Ok(id)
    }

    /// Records an error status that renders as a shaking banner.
    pub(crate) fn fail(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.failure.record(&self.message);
    }

    pub(crate) fn publish(&mut self, cx: &mut Context<Self>) {
        cx.emit(SchemaEditEvent::Changed);
        cx.emit(SchemaEditEvent::Activity(self.has_pending()));
        cx.notify();
    }

    fn load_policy(&mut self, cx: &mut Context<Self>) {
        let backend = self.host.backend.clone();
        let connection = self.connection.clone();
        let task = self
            .host
            .runtime
            .spawn(async move { backend.development_connections().await });
        self.tasks.push(cx.spawn(async move |this, cx| {
            if let Ok(Ok(records)) = task.await {
                this.update(cx, |this, cx| {
                    let mut policy = policy_for(&records, &connection);
                    policy.read_only = policy.read_only || this.session_read_only;
                    if this.policy != policy {
                        this.policy = policy;
                        this.inputs_stale = true;
                        cx.notify();
                    }
                })
                .ok();
            }
        }));
    }

    /// Sends one backend call; the reply arrives in `receive` with `id`.
    pub(crate) fn send(
        &mut self,
        id: u64,
        call: SchemaEditCall,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        match &self.channel {
            Channel::Worker(controls) => controls
                .as_ref()
                .ok_or("Connect the Objects document first")?
                .send(TableCommand::SchemaEdit(id, call)),
            Channel::Direct(source) => {
                let source = source
                    .clone()
                    .ok_or("Not connected. Connect from the sidebar first.")?;
                let backend = self.host.backend.clone();
                let writes = call.writes();
                let task = self
                    .host
                    .runtime
                    .spawn(async move { runtime::call(&backend, source, call).await });
                self.tasks.retain(|task| !task.is_ready());
                self.tasks.push(cx.spawn(async move |this, cx| {
                    let reply = task.await.unwrap_or_else(|_| SchemaEditCall::lost(writes));
                    this.update(cx, |this, cx| this.receive(id, reply, cx)).ok();
                }));
                Ok(())
            }
        }
    }

    /// Every backend reply, from the worker (PostgreSQL) or a runtime task.
    pub fn receive(&mut self, id: u64, reply: SchemaEditReply, cx: &mut Context<Self>) {
        match reply {
            SchemaEditReply::Observed(result) => {
                if self.observing != Some(id) {
                    return;
                }
                self.observing = None;
                match result {
                    Ok(target) => self.observed(*target),
                    Err(error) => self.fail(format!("Structure unavailable: {error}")),
                }
                self.publish(cx);
            }
            SchemaEditReply::Applied(result) => self.applied(id, result, cx),
        }
    }

    /// Captures a fresh baseline. The draft carries over when the table is
    /// unchanged on the server.
    pub fn observe(&mut self, cx: &mut Context<Self>) {
        if self.observing.is_some() || self.review.as_ref().is_some_and(|r| r.flow.is_some()) {
            return;
        }
        let id = match self.next_id() {
            Ok(id) => id,
            Err(error) => {
                self.fail(error);
                cx.notify();
                return;
            }
        };
        match self.send(id, SchemaEditCall::Observe(self.request.clone()), cx) {
            Ok(()) => {
                self.observing = Some(id);
                self.target = None;
                if !self.recovery.unknown() {
                    self.message = "Reading the table structure…".into();
                }
            }
            Err(error) => self.fail(error),
        }
        self.publish(cx);
    }

    fn observed(&mut self, target: SchemaEditTarget) {
        let baseline = target.baseline().clone();
        if baseline.table.schema != self.request.schema || baseline.table.name != self.request.table
        {
            self.fail("The captured table does not match this editor; nothing was changed");
            return;
        }
        let editable = baseline.editable.is_editable();
        let mut note = None;
        if let Some(model) = self.model.as_mut() {
            let had_changes = model.has_changes();
            if !model.rebase(baseline) && had_changes {
                note = Some(if model.conflicts().is_empty() {
                    "The table changed on the server; the draft was re-based onto the new structure"
                } else {
                    "The table changed on the server; the draft was re-based. Check the conflicts"
                });
            }
        } else {
            let request = self.request.clone();
            let model = match self.recovery.restorable(&self.request, baseline.engine) {
                Some((old, draft)) if old.table == baseline.table => {
                    note = Some("Restored the saved draft");
                    SchemaEditModel::restore(request, baseline, draft.clone())
                }
                Some((old, draft)) => {
                    note = Some(
                        "The table changed since the draft was saved; the draft was re-based onto it",
                    );
                    SchemaEditModel::rebased(request, old, baseline, draft)
                }
                None => SchemaEditModel::new(request, baseline),
            };
            self.model = Some(model);
        }
        self.target = editable.then_some(target);
        if !self
            .model
            .as_ref()
            .is_some_and(|m| m.in_draft(self.selected))
        {
            self.selected = Target::Table;
        }
        self.inputs_stale = true;
        self.journal();
        self.message = if self.recovery.unknown() {
            "Outcome unknown for the last attempt. Inspect the table, then reconcile.".into()
        } else if !editable {
            "This table cannot be edited here".into()
        } else {
            note.unwrap_or("Edit the draft, then review the exact SQL")
                .into()
        };
    }

    /// Journals the draft in progress (kept in memory when it does not fit).
    /// MySQL and SQLite journal nothing: the model holds the draft.
    pub(crate) fn journal(&mut self) {
        self.journal_dirty = false;
        *self.pending.get_mut() = None;
        let Some(model) = &self.model else {
            return;
        };
        if !model.editable() {
            return;
        }
        if !self.persist {
            self.recovery.forget_draft();
            self.memory_only = false;
            return;
        }
        match self
            .recovery
            .save_draft(model.request(), model.baseline(), model.draft())
        {
            DraftSave::MemoryOnly => self.memory_only = true,
            DraftSave::Saved | DraftSave::Cleared => self.memory_only = false,
            DraftSave::Blocked => {}
        }
    }

    /// Editing is open: an editable baseline, a writable connection, no
    /// unknown outcome and no open review.
    pub(crate) fn can_edit(&self) -> bool {
        self.model.as_ref().is_some_and(SchemaEditModel::editable)
            && !self.policy.read_only
            && !self.recovery.unknown()
            && self.review.is_none()
    }

    /// Why Review is unavailable, for the button tooltip.
    pub(crate) fn review_block(&self) -> Option<String> {
        if let Some(reason) = self.policy.read_only_reason() {
            return Some(reason.into());
        }
        if self.recovery.unknown() {
            return Some("Reconcile the unknown outcome first".into());
        }
        let model = self.model.as_ref()?;
        if let Some(reason) = model.review_block() {
            return Some(reason);
        }
        if self.observing.is_some() {
            return Some("Reading the table structure".into());
        }
        if self.target.is_none() {
            return Some("Reload the structure before reviewing".into());
        }
        None
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        if self.message.is_empty() || self.failure.shown(&self.message).is_some() {
            self.message = String::new();
        }
        // Every draft change lands here; the pending record is stale.
        *self.pending.get_mut() = None;
        if self.persist {
            // Journaling clones and encodes the whole table: coalesce it.
            self.journal_dirty = true;
            self.schedule_journal(cx);
            cx.notify();
        } else {
            self.journal();
            self.publish(cx);
        }
    }

    fn schedule_journal(&mut self, cx: &mut Context<Self>) {
        if self.journal_scheduled {
            return;
        }
        self.journal_scheduled = true;
        self._journal_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(JOURNAL_IDLE).await;
            this.update(cx, |this, cx| {
                this.journal_scheduled = false;
                this.flush_journal(cx);
            })
            .ok();
        }));
    }

    /// Journals an edit still waiting for the timer, and tells the host.
    pub(crate) fn flush_journal(&mut self, cx: &mut Context<Self>) {
        if self.journal_dirty {
            self.journal();
            cx.emit(SchemaEditEvent::Changed);
        }
    }

    // -- Editing actions (from render listeners) ------------------------------

    pub(crate) fn select(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected != target {
            self.selected = target;
            self.rebuild_inputs(window, cx);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(crate) fn rebuild_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inputs_stale = false;
        self.input_error = None;
        self.inputs.clear();
        let editable = self.can_edit();
        let Some(model) = &self.model else {
            return;
        };
        let target = self.selected;
        let mut inputs = Vec::new();
        for input in model.inputs(target) {
            let text = model.input_value(target, input);
            let enabled = editable && model.input_enabled(target, input);
            let field = cx.new(|cx| {
                let mut field = TextField::new(
                    input.label(),
                    INPUT_BYTES,
                    input.multiline(),
                    text,
                    window,
                    cx,
                );
                field.set_readonly(!enabled, cx);
                field
            });
            let subscription = cx.subscribe(&field, move |this, field, _: &Changed, cx| {
                this.input_changed(input, field, cx)
            });
            inputs.push(InputField {
                input,
                field,
                _subscription: subscription,
            });
        }
        self.inputs = inputs;
    }

    fn input_changed(&mut self, input: Input, field: Entity<TextField>, cx: &mut Context<Self>) {
        if !self.can_edit() {
            return;
        }
        let text = match field.read(cx).value(cx) {
            Ok(text) => text,
            Err(error) => {
                self.input_error = Some((input, error.into()));
                cx.notify();
                return;
            }
        };
        let target = self.selected;
        let Some(model) = &mut self.model else {
            return;
        };
        match model.set_input(target, input, &text) {
            Ok(changed) => {
                if self.input_error.as_ref().is_some_and(|(i, _)| *i == input) {
                    self.input_error = None;
                }
                if changed {
                    self.edited(cx);
                } else {
                    cx.notify();
                }
            }
            Err(error) => {
                self.input_error = Some((input, error));
                cx.notify();
            }
        }
    }

    pub(crate) fn choose(&mut self, choice: Choice, cx: &mut Context<Self>) {
        if !self.can_edit() {
            return;
        }
        let target = self.selected;
        if let Some(model) = &mut self.model
            && model.set_choice(target, choice)
        {
            // Defaults and generation enable or clear their text inputs.
            self.inputs_stale = true;
            self.edited(cx);
        }
    }

    pub(crate) fn undo(&mut self, target: Target, attr: Attr, cx: &mut Context<Self>) {
        if !self.can_edit() {
            return;
        }
        if let Some(model) = &mut self.model
            && model.revert(target, attr)
        {
            self.inputs_stale = true;
            self.edited(cx);
        }
    }

    pub(crate) fn undo_row(&mut self, target: Target, cx: &mut Context<Self>) {
        if !self.can_edit() {
            return;
        }
        let Some(model) = &mut self.model else {
            return;
        };
        match model.revert_object(target) {
            Ok(true) => {
                self.inputs_stale = true;
                self.edited(cx);
            }
            Ok(false) => {}
            Err(error) => {
                self.fail(error);
                cx.notify();
            }
        }
    }

    pub(crate) fn remove(&mut self, target: Target, cx: &mut Context<Self>) {
        if !self.can_edit() {
            return;
        }
        if let Some(model) = &mut self.model
            && model.remove(target)
        {
            self.inputs_stale = true;
            self.edited(cx);
        }
    }

    pub(crate) fn restore(&mut self, target: Target, cx: &mut Context<Self>) {
        if !self.can_edit() {
            return;
        }
        let Some(model) = &mut self.model else {
            return;
        };
        match model.restore_object(target) {
            Ok(()) => {
                self.inputs_stale = true;
                self.edited(cx);
            }
            Err(error) => {
                self.fail(error);
                cx.notify();
            }
        }
    }

    pub(crate) fn add(&mut self, group: ChangeGroup, primary: bool, cx: &mut Context<Self>) {
        if !self.can_edit() {
            return;
        }
        let Some(model) = &mut self.model else {
            return;
        };
        let added = match group {
            ChangeGroup::Columns => Ok(model.add_column()),
            ChangeGroup::Keys if primary => model.add_primary_key(),
            ChangeGroup::Keys => Ok(model.add_unique()),
            ChangeGroup::ForeignKeys => Ok(model.add_foreign_key()),
            ChangeGroup::Checks => Ok(model.add_check()),
            ChangeGroup::Indexes => Ok(model.add_index()),
            ChangeGroup::Table => return,
        };
        match added {
            Ok(target) => {
                if let Some(line) = model.line_of(target) {
                    self.scroll
                        .scroll_to_item(line, gpui::ScrollStrategy::Nearest);
                }
                self.selected = target;
                self.inputs_stale = true;
                self.edited(cx);
            }
            Err(error) => {
                self.fail(error);
                cx.notify();
            }
        }
    }

    pub(crate) fn move_column(&mut self, target: Target, up: bool, cx: &mut Context<Self>) {
        if !self.can_edit() {
            return;
        }
        let Target::Column(origin) = target else {
            return;
        };
        if let Some(model) = &mut self.model
            && model.move_column(origin, up)
        {
            self.edited(cx);
        }
    }

    pub(crate) fn discard(&mut self, cx: &mut Context<Self>) {
        if !self.can_edit() {
            return;
        }
        if let Some(model) = &mut self.model {
            model.discard();
            self.selected = Target::Table;
            self.inputs_stale = true;
            self.message = "Draft discarded".into();
            self.edited(cx);
        }
    }

    /// Two clicks: the first explains, the second removes the local record.
    /// It neither retries nor undoes SQL.
    pub(crate) fn reconcile(&mut self, cx: &mut Context<Self>) {
        if !self.recovery.unknown() || self.has_pending() {
            return;
        }
        if !self.armed {
            self.armed = true;
            self.message = "Inspect the table first. Click Discard record again to remove this local recovery record; it neither retries nor undoes SQL.".into();
        } else if self.recovery.discard(true) {
            self.armed = false;
            self.message = "Recovery record discarded".into();
            self.inputs_stale = true;
            self.observe(cx);
        }
        self.publish(cx);
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if self.review.is_some() {
            return;
        }
        self.flush_journal(cx);
        cx.emit(SchemaEditEvent::Close);
    }
}

impl Drop for SchemaEditView {
    fn drop(&mut self) {
        self.inputs.clear();
        self.review = None;
        self.target = None;
    }
}
