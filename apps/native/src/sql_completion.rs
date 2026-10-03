//! Connection-scoped, cache-only SQL completions. Metadata requests are handed
//! to the Workbench's existing data lane; the provider owns no database task.
mod context;
mod provider;
#[cfg(test)]
mod tests;

use dbunk_lib::backend::{completion::CompletionColumns, objects::PgObjectCatalog};
use gpui::{App, Context, Window};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
const CACHE_BYTES: usize = 4 * 1024 * 1024;
const CACHE_TEXT_BYTES: usize = 1024 * 1024;
const MAX_NODES: usize = 10_000;
const MENU_BYTES: usize = 2 * 1024 * 1024;
const MENU_TEXT_BYTES: usize = 32 * 1024;
const MENU_ITEMS: usize = 128;

struct Lease {
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Lease {
    fn admit(budget: Rc<Cell<usize>>, bytes: usize) -> Result<Self, &'static str> {
        if bytes > WORKSPACE_BYTES.saturating_sub(budget.get()) {
            return Err("SQL completion needs shared memory; clear a retained result or tool");
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetadataRequest {
    Catalog {
        id: u64,
        generation: u64,
    },
    Columns {
        id: u64,
        generation: u64,
        schema: String,
        relation: String,
    },
}
impl MetadataRequest {
    pub fn id(&self) -> u64 {
        match self {
            Self::Catalog { id, .. } | Self::Columns { id, .. } => *id,
        }
    }
}
struct Request {
    value: MetadataRequest,
    dispatched: bool,
}
struct Relation {
    name: String,
    view: bool,
}
struct Schema {
    name: String,
    relations: Vec<Relation>,
}
struct Cache {
    schemas: Vec<Schema>,
    truncated: bool,
    columns: Option<CompletionColumns>,
    _lease: Lease,
}
struct State {
    budget: Rc<Cell<usize>>,
    wake: async_channel::Sender<()>,
    connection: Option<String>,
    connected: bool,
    composing: bool,
    generation: u64,
    sequence: u64,
    request: Option<Request>,
    cache: Option<Cache>,
    failed: Option<(Option<(String, String)>, String)>,
    status: String,
    refresh: bool,
    // Incremented by the root's editor observation, including selection changes.
    editor_revision: u64,
    requested_revision: u64,
}

#[derive(Clone)]
pub struct CompletionHandle(Rc<RefCell<State>>);
impl CompletionHandle {
    pub fn new(budget: Rc<Cell<usize>>, wake: async_channel::Sender<()>) -> Self {
        Self(Rc::new(RefCell::new(State {
            budget,
            wake,
            connection: None,
            connected: false,
            composing: false,
            generation: 0,
            sequence: 0,
            request: None,
            cache: None,
            failed: None,
            status: "SQL completion: connect to load catalog names".into(),
            refresh: false,
            editor_revision: 0,
            requested_revision: 0,
        })))
    }
    pub fn provider(&self) -> Rc<dyn editor::CompletionProvider> {
        Rc::new(provider::Provider(self.clone()))
    }
    /// Call before changing the tab binding, then dismiss the editor's menu/tasks.
    pub fn bind(&self, connection: Option<String>) {
        let mut s = self.0.borrow_mut();
        s.generation = s.generation.saturating_add(1);
        s.connection = connection;
        s.connected = false;
        s.request = None;
        s.cache = None;
        s.failed = None;
        s.refresh = false;
        s.status = "SQL completion: connect to load catalog names".into();
    }
    pub fn set_connected(&self, connected: bool) {
        let mut s = self.0.borrow_mut();
        if s.connected == connected {
            return;
        }
        s.connected = connected;
        s.generation = s.generation.saturating_add(1);
        s.request = None;
        s.refresh = false;
        s.failed = None;
        if !connected {
            s.cache = None;
        }
        s.status = if connected {
            "SQL completion ready; invoke in the editor"
        } else {
            "SQL completion disconnected"
        }
        .into();
    }
    pub fn set_composing(&self, composing: bool) {
        let mut s = self.0.borrow_mut();
        s.composing = composing;
        if composing {
            s.refresh = false;
        }
    }
    /// Record buffer/selection changes so metadata arrival never reopens a menu
    /// for a context the user has left. No network cancellation occurs here.
    pub fn editor_changed(&self) {
        let mut s = self.0.borrow_mut();
        s.editor_revision = s.editor_revision.saturating_add(1);
        s.refresh = false;
    }
    pub fn pending_request(&self) -> Option<MetadataRequest> {
        self.0
            .borrow()
            .request
            .as_ref()
            .filter(|r| !r.dispatched)
            .map(|r| r.value.clone())
    }
    pub fn mark_dispatched(&self, id: u64) {
        if let Some(r) = &mut self.0.borrow_mut().request
            && r.value.id() == id
        {
            r.dispatched = true;
        }
    }
    pub fn fail(&self, id: u64, error: impl Into<String>) {
        let mut s = self.0.borrow_mut();
        let Some(r) = s.request.take() else {
            return;
        };
        if r.value.id() != id {
            s.request = Some(r);
            return;
        }
        let key = match r.value {
            MetadataRequest::Catalog { .. } => None,
            MetadataRequest::Columns {
                schema, relation, ..
            } => Some((schema, relation)),
        };
        let message: String = error.into().chars().take(512).collect();
        s.status = format!("SQL completion metadata unavailable: {message}");
        s.failed = Some((key, message));
        s.refresh = false;
    }
    /// Explicit retry/refresh; it does not discard a good catalog until a new
    /// bounded capture has been admitted. Root exposes this as a manual action.
    pub fn refresh(&self) {
        let mut s = self.0.borrow_mut();
        if s.connected && !s.composing && s.request.is_none() {
            s.failed = None;
            s.queue(None);
        }
    }
    pub fn status(&self) -> String {
        self.0.borrow().status.clone()
    }
    pub fn take_refresh(&self) -> bool {
        let mut s = self.0.borrow_mut();
        let refresh =
            s.refresh && s.connected && !s.composing && s.editor_revision == s.requested_revision;
        s.refresh = false;
        refresh
    }
    pub fn accept_catalog(&self, id: u64, result: Result<PgObjectCatalog, String>) {
        if !self.0.borrow().request.as_ref().is_some_and(
            |r| matches!(r.value,MetadataRequest::Catalog{id:request,..} if request == id),
        ) {
            return;
        }
        let result = result.and_then(|catalog| {
            Cache::new(catalog, self.0.borrow().budget.clone()).map_err(str::to_owned)
        });
        match result {
            Err(error) => self.fail(id, error),
            Ok(cache) => {
                let mut s = self.0.borrow_mut();
                s.cache = Some(cache);
                s.loaded();
            }
        }
    }
    pub fn accept_columns(&self, id: u64, result: Result<CompletionColumns, String>) {
        let s = self.0.borrow();
        let Some(Request {
            value:
                MetadataRequest::Columns {
                    id: request,
                    schema,
                    relation,
                    ..
                },
            ..
        }) = &s.request
        else {
            return;
        };
        if *request != id {
            return;
        }
        let result = result.and_then(|columns| {
            if columns.schema != *schema || columns.relation != *relation {
                return Err("Column metadata identity changed".into());
            }
            validate_columns(&columns).map_err(str::to_owned)?;
            Ok(columns)
        });
        drop(s);
        match result {
            Err(error) => self.fail(id, error),
            Ok(columns) => {
                let mut s = self.0.borrow_mut();
                if let Some(cache) = &mut s.cache {
                    cache.columns = Some(columns);
                }
                s.loaded();
            }
        }
    }
}
impl State {
    fn loaded(&mut self) {
        self.request = None;
        self.failed = None;
        self.refresh = self.editor_revision == self.requested_revision && !self.composing;
        self.status = if self.cache.as_ref().is_some_and(|c| c.truncated) {
            "SQL completion catalog is partial; omitted objects are unavailable"
        } else {
            "SQL completion metadata loaded"
        }
        .into();
        let _ = self.wake.try_send(());
    }
    fn queue(&mut self, key: Option<(String, String)>) {
        if !self.connected
            || self.connection.is_none()
            || self.composing
            || self.request.is_some()
            || self
                .failed
                .as_ref()
                .is_some_and(|(failed, _)| *failed == key)
        {
            return;
        }
        let Some(id) = self.sequence.checked_add(1) else {
            self.status = "SQL completion request sequence exhausted".into();
            return;
        };
        self.sequence = id;
        let value = match key {
            Some((schema, relation)) => MetadataRequest::Columns {
                id,
                generation: self.generation,
                schema,
                relation,
            },
            None => MetadataRequest::Catalog {
                id,
                generation: self.generation,
            },
        };
        self.request = Some(Request {
            value,
            dispatched: false,
        });
        self.requested_revision = self.editor_revision;
        self.status = "SQL completion metadata queued".into();
        let _ = self.wake.try_send(());
    }
}
impl Cache {
    fn new(catalog: PgObjectCatalog, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        let lease = Lease::admit(budget, CACHE_BYTES)?;
        let mut schemas = Vec::new();
        let mut bytes = 0usize;
        let mut nodes = 0usize;
        for schema in catalog.schemas {
            nodes += 1;
            bytes = bytes.saturating_add(schema.name.capacity());
            if nodes > MAX_NODES || bytes > CACHE_TEXT_BYTES || schema.name.len() > 8192 {
                return Err("SQL completion catalog exceeds native cache limits");
            }
            let mut relations = Vec::new();
            for (items, view) in [
                (schema.tables, false),
                (schema.views, true),
                (schema.materialized_views, true),
                (schema.foreign_tables, false),
            ] {
                for entry in items {
                    nodes += 1;
                    bytes = bytes.saturating_add(entry.name.capacity());
                    if nodes > MAX_NODES || bytes > CACHE_TEXT_BYTES || entry.name.len() > 8192 {
                        return Err("SQL completion catalog exceeds native cache limits");
                    }
                    relations.push(Relation {
                        name: entry.name,
                        view,
                    });
                }
            }
            schemas.push(Schema {
                name: schema.name,
                relations,
            });
        }
        Ok(Self {
            schemas,
            truncated: !catalog.truncated.is_empty(),
            columns: None,
            _lease: lease,
        })
    }
}
fn validate_columns(c: &CompletionColumns) -> Result<(), &'static str> {
    let mut bytes = c
        .columns
        .capacity()
        .saturating_mul(std::mem::size_of::<
            dbunk_lib::backend::completion::CompletionColumn,
        >())
        .saturating_add(c.schema.capacity())
        .saturating_add(c.relation.capacity());
    if c.columns.len() > 4096 || c.schema.len() > 63 || c.relation.len() > 63 || c.relation_oid == 0
    {
        return Err("Invalid or oversized completion column metadata");
    }
    let mut ordinal = 0;
    for column in &c.columns {
        bytes = bytes
            .saturating_add(column.name.capacity())
            .saturating_add(column.data_type.capacity());
        if column.name.len() > 63
            || column.data_type.len() > 8192
            || column.ordinal_position <= ordinal
        {
            return Err("Invalid or oversized completion column metadata");
        }
        ordinal = column.ordinal_position;
    }
    if bytes > CACHE_TEXT_BYTES {
        return Err("Completion columns exceed the 1 MiB native cache limit");
    }
    Ok(())
}

/// With a Window, this synchronously drops the menu AND Zed's pending completion
/// tasks. Call before retarget/disconnect and when marked composition starts.
pub fn dismiss(editor: &mut editor::Editor, window: &mut Window, cx: &mut Context<editor::Editor>) {
    editor.dismiss_menus_and_popups(false, window, cx);
}
/// Immediate no-Window fence. Root must also call `dismiss` at its next window
/// boundary to cancel an internal asynchronous menu build.
pub fn clear_menu(editor: &mut editor::Editor, cx: &mut Context<editor::Editor>) {
    editor.context_menu().borrow_mut().take();
    cx.notify();
}
/// Refresh only while the query editor owns focus, after take_refresh succeeds.
pub fn show(editor: &mut editor::Editor, window: &mut Window, cx: &mut Context<editor::Editor>) {
    use gpui::{EntityInputHandler, Focusable};
    if editor.focus_handle(cx).is_focused(window) && editor.marked_text_range(window, cx).is_none()
    {
        editor.show_completions(&editor::actions::ShowCompletions, window, cx);
    }
}
