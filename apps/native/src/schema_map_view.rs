//! Owned schema-map tab. Setup is transient; captures, preference revisions and
//! pointer actions each retain their exact document/capture identity.
use crate::{
    bounded_field::{Changed, Field},
    controller::{Host, TableCommand, TableControls, TableMessage, TableReceiver},
    schema_map_model::{
        Camera, MapAttributes, MapPoint, MapRouting, Scene, SceneKey, Selection, Viewport,
    },
};
use dbunk_lib::backend::{
    WorkspaceDocument, result_files as files,
    schema_map::{preferences::*, *},
};
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, ScrollHandle,
    Subscription, UniformListScrollHandle, Window, div, prelude::*, px, rgb,
};
use std::{cell::Cell, rc::Rc, sync::Arc};
mod actions;
mod canvas;
mod movement;
mod render;
mod runtime;
mod save;
#[cfg(test)]
mod tests;
gpui::actions!(schema_map, [NextControl, PreviousControl]);

pub enum SchemaMapEvent {
    OpenTable {
        connection: String,
        schema: String,
        table: String,
    },
}
impl EventEmitter<SchemaMapEvent> for SchemaMapView {}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Database,
    Schema,
    Relation,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Connect,
    Refresh,
    Cancel,
    Clear,
    Fit,
    ZoomIn,
    ZoomOut,
    Routing,
    Attributes,
    Types,
    Nulls,
    Comments,
    Save,
    ResetLayout,
    Defaults,
    Open,
    Copy,
    Previous,
    Next,
    Svg,
    Png,
    Glossary,
    CancelFile,
    Scope(Scope),
}
const ACTIONS: [(Action, &str); 23] = [
    (Action::Connect, "Connect"),
    (Action::Refresh, "Refresh map"),
    (Action::Cancel, "Cancel read"),
    (Action::Clear, "Clear map / reset identity"),
    (Action::Fit, "Fit map"),
    (Action::ZoomIn, "Zoom in"),
    (Action::ZoomOut, "Zoom out"),
    (Action::Routing, "Routing"),
    (Action::Attributes, "Attributes"),
    (Action::Types, "Types"),
    (Action::Nulls, "NULL"),
    (Action::Comments, "Comments"),
    (Action::Save, "Save map settings"),
    (Action::ResetLayout, "Reset layout"),
    (Action::Defaults, "Reset scope defaults"),
    (Action::Open, "Open selected table"),
    (Action::Copy, "Copy detail page"),
    (Action::Previous, "Previous detail page"),
    (Action::Next, "Next detail page"),
    (Action::Svg, "Save viewport SVG"),
    (Action::CancelFile, "Cancel file save"),
    (Action::Png, "Save viewport PNG 2x"),
    (Action::Glossary, "Relationship glossary"),
];
struct Lease {
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Lease {
    fn new(budget: Rc<Cell<usize>>, bytes: usize) -> Result<Self, &'static str> {
        if bytes > (128 * 1024 * 1024usize).saturating_sub(budget.get()) {
            return Err("Map shared memory allowance exhausted; clear another capture and retry");
        }
        budget.set(budget.get() + bytes);
        Ok(Self { budget, bytes })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}
enum Purpose {
    Refresh,
    Open {
        identity: SchemaMapIdentity,
        scene: SceneKey,
    },
}
enum Pending {
    Graph {
        id: u64,
        request: SchemaMapRequest,
        purpose: Purpose,
        cancelled: bool,
    },
    Load {
        id: u64,
        scope: SchemaMapPreferenceScope,
    },
    Save {
        id: u64,
        scope: SchemaMapPreferenceScope,
        value: SchemaMapPreferences,
    },
    Reset {
        id: u64,
        scope: SchemaMapPreferenceScope,
        scene: Box<Scene>,
        value: SchemaMapPreferences,
    },
}
impl Pending {
    fn id(&self) -> u64 {
        match self {
            Self::Graph { id, .. }
            | Self::Load { id, .. }
            | Self::Save { id, .. }
            | Self::Reset { id, .. } => *id,
        }
    }
}
struct Incoming {
    snapshot: Arc<SchemaMapSnapshot>,
    _lease: Lease,
}
#[derive(Clone, Copy)]
enum Drag {
    Pan {
        last: MapPoint,
    },
    Node {
        identity: SchemaMapIdentity,
        key: SceneKey,
        last: MapPoint,
        moved: bool,
    },
}
pub struct SchemaMapView {
    host: Arc<Host>,
    id: String,
    connection: Option<String>,
    wake: async_channel::Sender<()>,
    budget: Rc<Cell<usize>>,
    controls: Option<TableControls>,
    receiver: Option<TableReceiver>,
    ready: bool,
    opening: bool,
    editable: bool,
    generation: u64,
    sequence: u64,
    pending: Option<Pending>,
    incoming: Option<Incoming>,
    scene: Option<Scene>,
    current: bool,
    preference_scope: Option<SchemaMapPreferenceScope>,
    revision: Option<SchemaMapPreferencesRevision>,
    working: Option<SchemaMapPreferences>,
    dirty: bool,
    scope: Scope,
    fields: Vec<Entity<Field>>,
    field_events: Vec<Subscription>,
    // 4 MiB covers two 128 KiB field histories, editor overlap, bounded
    // preference current/pending/ACK copies and detail/status/AX strings.
    // Scene separately charges shaped detail and graph presentation overlap.
    _ui_lease: Option<Lease>,
    camera: Camera,
    viewport: Viewport,
    canvas_origin: MapPoint,
    drag: Option<Drag>,
    fit_pending: bool,
    selection: Option<Selection>,
    detail_page: usize,
    detail_text: String,
    detail_next: bool,
    glossary: bool,
    status: String,
    root: FocusHandle,
    list: FocusHandle,
    canvas_focus: FocusHandle,
    details: FocusHandle,
    buttons: Vec<FocusHandle>,
    scope_buttons: Vec<FocusHandle>,
    previous_focus: Option<FocusHandle>,
    scroll: UniformListScrollHandle,
    detail_scroll: ScrollHandle,
    file_busy: bool,
    file_cancel: Option<files::Cancellation>,
}
impl SchemaMapView {
    pub fn new(
        host: Arc<Host>,
        document: &WorkspaceDocument,
        wake: async_channel::Sender<()>,
        budget: Rc<Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            host,
            id: document.id.clone(),
            connection: document.connection_id.clone(),
            wake,
            budget,
            controls: None,
            receiver: None,
            ready: false,
            opening: false,
            editable: true,
            generation: 0,
            sequence: 0,
            pending: None,
            incoming: None,
            scene: None,
            current: false,
            preference_scope: None,
            revision: None,
            working: None,
            dirty: false,
            scope: Scope::Database,
            fields: Vec::new(),
            field_events: Vec::new(),
            _ui_lease: None,
            camera: Camera::default(),
            viewport: Viewport {
                width: 1,
                height: 1,
                camera: Camera::default(),
            },
            canvas_origin: MapPoint { x: 0., y: 0. },
            drag: None,
            fit_pending: false,
            selection: None,
            detail_page: 0,
            detail_text: String::new(),
            detail_next: false,
            glossary: false,
            status: "Disconnected. Connect, then Refresh to read the map".into(),
            root: cx.focus_handle(),
            list: cx.focus_handle(),
            canvas_focus: cx.focus_handle(),
            details: cx.focus_handle(),
            buttons: ACTIONS.iter().map(|_| cx.focus_handle()).collect(),
            scope_buttons: (0..3).map(|_| cx.focus_handle()).collect(),
            previous_focus: None,
            scroll: UniformListScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
            file_busy: false,
            file_cancel: None,
        };
        view.ensure_fields(window, cx);
        view
    }
    fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.fields.is_empty() {
            return;
        }
        match Lease::new(self.budget.clone(), 4 * 1024 * 1024) {
            Ok(lease) => self._ui_lease = Some(lease),
            Err(error) => {
                self.status = error.into();
                return;
            }
        }
        for label in [
            "Map schema, exact quoted name",
            "Map table, exact quoted name",
        ] {
            let field = cx.new(|cx| Field::new(label, 63, false, String::new(), window, cx));
            self.field_events
                .push(cx.subscribe(&field, |_, _, _: &Changed, cx| cx.notify()));
            self.fields.push(field);
        }
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn has_pending(&self) -> bool {
        self.receiver
            .as_ref()
            .is_some_and(TableReceiver::has_pending)
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editable = editable;
        self.sync_fields(cx);
        cx.notify();
    }
    fn sync_fields(&self, cx: &mut Context<Self>) {
        for field in &self.fields {
            field.update(cx, |field, cx| {
                field.set_readonly(!self.editable || self.pending.is_some() || self.opening, cx)
            });
        }
    }
    fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.fields.iter().any(|field| {
            field.focus_handle(cx).is_focused(window)
                && field.update(cx, |field, cx| field.composing(window, cx))
        })
    }
    pub fn focus_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_fields(window, cx);
        let order = self.focus_order(cx);
        let focus = self
            .previous_focus
            .as_ref()
            .filter(|f| order.contains(f))
            .unwrap_or(if self.scene.is_some() {
                &self.list
            } else {
                &self.buttons[0]
            });
        window.focus(focus, cx);
    }
    pub fn remember_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.root.contains_focused(window, cx) {
            self.previous_focus = window.focused(cx);
        }
    }
    pub fn bind_connection(&mut self, connection: String, cx: &mut Context<Self>) {
        if self.dirty {
            self.status =
                "Unsaved map settings retained; clear them explicitly before changing connections"
                    .into();
            cx.notify();
            return;
        }
        if self.controls.is_none() && !self.opening && self.pending.is_none() {
            self.clear_results(cx);
            self.connection = Some(connection);
        }
    }
    pub fn invalidate_after_restore(&mut self, cx: &mut Context<Self>) {
        self.mark_disconnected(cx);
        self.status="Database may have changed; reconnect and Refresh before opening tables or saving positions".into();
    }
    pub fn mark_disconnected(&mut self, cx: &mut Context<Self>) {
        if let Some(controls) = self.controls.take() {
            controls.stop();
        }
        self.receiver = None;
        self.ready = false;
        self.opening = false;
        if matches!(self.pending, Some(Pending::Reset { .. })) {
            self.dirty = true;
        }
        self.pending = None;
        self.incoming = None;
        self.current = false;
        self.revision = None;
        self.drag = None;
        self.generation = self.generation.saturating_add(1);
        self.sync_fields(cx);
        self.status = if self.dirty {
            "Disconnected. Unsaved map settings retained; reload before another save"
        } else {
            "Disconnected; retained map may be stale"
        }
        .into();
        cx.notify();
    }
    pub fn clear_results(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_some() || self.opening {
            return;
        }
        self.scene = None;
        self.incoming = None;
        self.working = None;
        self.revision = None;
        self.preference_scope = None;
        self.selection = None;
        self.detail_text.clear();
        self.detail_next = false;
        self.glossary = false;
        self.dirty = false;
        self.current = false;
        self.drag = None;
        self.status = "Map cleared; next Refresh resolves fresh identities".into();
        cx.notify();
    }
    fn next_id(&mut self) -> Result<u64, &'static str> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("Map request identity exhausted; reopen this tab")?;
        Ok(self.sequence)
    }
    fn report(&mut self, text: impl AsRef<str>) {
        self.status = text.as_ref().chars().take(1024).collect();
    }
}
impl Drop for SchemaMapView {
    fn drop(&mut self) {
        if let Some(controls) = &self.controls {
            controls.stop();
        }
        if let Some(token) = &self.file_cancel {
            token.cancel();
        }
    }
}
impl Focusable for SchemaMapView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.root.clone()
    }
}
fn preference_scope(scope: &SchemaMapScope) -> SchemaMapPreferenceScope {
    match scope {
        SchemaMapScope::Database => SchemaMapPreferenceScope::Database,
        SchemaMapScope::Schema { name, .. } => {
            SchemaMapPreferenceScope::Schema { name: name.clone() }
        }
        SchemaMapScope::Relation { schema, table, .. } => SchemaMapPreferenceScope::Relation {
            schema: schema.clone(),
            table: table.clone(),
        },
    }
}
