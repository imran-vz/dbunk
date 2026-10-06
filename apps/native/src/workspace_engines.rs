//! Plan 031 step 4: engine surfaces. A non-PostgreSQL connection with a
//! native session gets one surface per connection, owned here. While such a
//! connection is selected, the surface supplies the sidebar tree, the tabs
//! and the document area in place of the PostgreSQL documents; the status
//! bar and connection row read its session phase and latency.
//!
//! Engine-neutral seam: add a variant to [`EngineSurface`] and a match arm in
//! each method below. Nothing else in the workspace names an engine.
//!
//! Lifecycle, the same for every engine: selecting a connection creates its
//! surface and connects (the explicit connect and retry; nothing retries on
//! its own). A disconnect, a stale record and a removed connection detach the
//! session and close it within [`CLOSE_DEADLINE`]; quit joins every close
//! before the backend shuts down.
use super::*;
use crate::{
    clickhouse::workspace::{ClickHouseEvent, ClickHouseWorkspace},
    document_view::TabInfo,
    mysql_lane::{MySqlEvent, MySqlLane},
    open_anything::{Item, ItemKind, Target},
    redis_view::{RedisEvent, RedisWorkspace},
    sqlite_workspace::{SqliteEvent, SqliteWorkspace},
};
use dbunk_lib::backend::DevelopmentEngineConnection;
use futures_util::future::BoxFuture;
use gpui::{AnyView, App, EventEmitter};

/// How long one session may take to close. SQLite aborts its worker at this
/// deadline; any close still running a second later is abandoned (at quit
/// the backend shutdown that follows retires what is left).
const CLOSE_DEADLINE: Duration = Duration::from_secs(5);

/// A detached session's close, run on the Tokio runtime.
type Closing = BoxFuture<'static, Result<(), String>>;

#[derive(Clone)]
pub(super) enum EngineSurface {
    Redis(Entity<RedisWorkspace>),
    Sqlite(Entity<SqliteWorkspace>),
    ClickHouse(Entity<ClickHouseWorkspace>),
    MySql(Entity<MySqlLane>),
}

pub(super) struct Surface {
    /// The connection record the session was opened for; see [`fate`].
    opened: DevelopmentConnection,
    surface: EngineSurface,
    /// Sidebar tree view, placed by the shell.
    tree: AnyView,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
pub(super) struct EngineSurfaces(Vec<Surface>);

/// A saved, readable connection of an engine with a native surface.
pub(super) fn surface_connection(connection: &DevelopmentConnection) -> bool {
    connection.unsupported_reason.is_none()
        && matches!(
            connection.settings,
            Some(
                DevelopmentEngineConnection::Redis(_)
                    | DevelopmentEngineConnection::SQLite(_)
                    | DevelopmentEngineConnection::ClickHouse(_)
                    | DevelopmentEngineConnection::MySQL(_)
            )
        )
}

/// The session inputs of a record. Organization (project, folder) is not one.
fn record_of(connection: &DevelopmentConnection) -> String {
    connection
        .settings
        .as_ref()
        .and_then(|settings| serde_json::to_string(settings).ok())
        .unwrap_or_default()
}

/// Whether a session survives a settled form: an edit or delete (`scope`)
/// ends that connection's session, a credential change (`all`) ends every
/// session, and a new connection (neither) ends none.
fn survives_change(scope: Option<&str>, all: bool, id: &str) -> bool {
    !all && scope != Some(id)
}

#[derive(Debug, PartialEq, Eq)]
enum Fate {
    Keep,
    /// Close the session; the surface and its tabs stay for a reconnect.
    Close,
    /// Close the session and drop the surface.
    Drop,
}

/// What a reload (`form == None`) or a settled form (`Some((scope, all))`)
/// does to the surface opened for `opened`, given the connection's current
/// record (`None` once deleted).
fn fate(
    opened: &DevelopmentConnection,
    current: Option<&DevelopmentConnection>,
    form: Option<(Option<&str>, bool)>,
) -> Fate {
    let Some(current) = current.filter(|current| surface_connection(current)) else {
        return Fate::Drop;
    };
    let engine = |connection: &DevelopmentConnection| {
        connection
            .settings
            .as_ref()
            .map(DevelopmentEngineConnection::engine)
    };
    if engine(current) != engine(opened) {
        return Fate::Drop;
    }
    let edited = form.is_some_and(|(scope, all)| !survives_change(scope, all, &current.id));
    match current.settings {
        // The backend retires MySQL sessions on every edit, delete and
        // credential change (`retire_data`); the lane sees its session close.
        Some(DevelopmentEngineConnection::MySQL(_)) => Fate::Keep,
        _ if record_of(current) != record_of(opened) => Fate::Close,
        // A SQLite session reads only its record: no secret, no route.
        Some(DevelopmentEngineConnection::SQLite(_)) => Fate::Keep,
        // Redis and ClickHouse resolve a secret and route at open.
        _ if edited => Fate::Close,
        _ => Fate::Keep,
    }
}

/// Closes detached sessions concurrently, each within the deadline, and
/// reports the first failure.
pub(super) async fn close_within(closing: Vec<Closing>) -> Result<(), String> {
    close_by(closing, CLOSE_DEADLINE + Duration::from_secs(1)).await
}

async fn close_by(closing: Vec<Closing>, limit: Duration) -> Result<(), String> {
    let bounded = closing.into_iter().map(|close| async move {
        tokio::time::timeout(limit, close)
            .await
            .unwrap_or_else(|_| Err("A session did not close in time; it was abandoned".into()))
    });
    futures_util::future::join_all(bounded)
        .await
        .into_iter()
        .collect()
}

/// Palette entries for one surface's tabs. Keys carry the connection, since
/// tab ids are only unique within a surface.
fn tab_items<C>(connection: &str, connection_name: &str, tabs: &[TabInfo]) -> Vec<Item<C>> {
    tabs.iter()
        .map(|tab| {
            Item::new(
                format!("engine-tab:{connection}:{}", tab.id),
                ItemKind::Tab,
                tab.title.clone(),
                format!("Open tab · {connection_name}"),
                &tab.status,
                Target::Tab(tab.id.clone()),
            )
        })
        .collect()
}

/// Repaints the workspace when `view` changes and records its latency.
fn watch<V: EventEmitter<E>, E: 'static>(
    view: &Entity<V>,
    connection: &str,
    latency: fn(&E) -> Option<u64>,
    cx: &mut Context<Workspace>,
) -> Vec<Subscription> {
    let connection = connection.to_owned();
    vec![
        cx.observe(view, |_, _, cx| cx.notify()),
        cx.subscribe(view, move |this, _, event: &E, cx| {
            if let Some(ms) = latency(event) {
                this.shell.last_latency.insert(connection.clone(), ms);
            }
            cx.notify();
        }),
    ]
}

impl EngineSurface {
    fn phase(&self, cx: &App) -> ConnectionPhase {
        match self {
            Self::Redis(view) => view.read(cx).phase(),
            Self::Sqlite(view) => view.read(cx).phase(),
            Self::ClickHouse(view) => view.read(cx).phase(cx),
            Self::MySql(view) => view.read(cx).phase(),
        }
    }

    /// One explicit attempt unless a session is open or opening.
    fn connect(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::Redis(view) => view.update(cx, |view, cx| view.connect(cx)),
            Self::Sqlite(view) => view.update(cx, |view, cx| view.connect(window, cx)),
            Self::ClickHouse(view) => view.update(cx, |view, cx| view.connect(cx)),
            Self::MySql(view) => view.update(cx, |view, cx| view.connect(window, cx)),
        }
    }

    /// Detaches the session (the surface returns to Idle and drops late
    /// results by generation) and returns its close for the caller to run.
    fn detach(&self, cx: &mut App) -> Option<Closing> {
        Some(match self {
            Self::Redis(view) => {
                let session = view.update(cx, |view, cx| view.disconnect(cx))?;
                // The last handle owns the sockets and SSH route; dropping
                // them may block.
                Box::pin(async move {
                    tokio::task::spawn_blocking(move || drop(session))
                        .await
                        .map_err(|_| "The Redis session did not close cleanly".to_string())
                })
            }
            Self::Sqlite(view) => {
                let session = view.update(cx, |view, cx| view.take_session(cx))?;
                Box::pin(async move { session.close(CLOSE_DEADLINE).await })
            }
            Self::ClickHouse(view) => {
                let session = view.update(cx, |view, cx| view.disconnect(cx))?;
                Box::pin(async move {
                    session.close().await;
                    Ok(())
                })
            }
            Self::MySql(view) => {
                let session = view.update(cx, |view, cx| view.disconnect(cx))?;
                Box::pin(async move {
                    session.close().await;
                    Ok(())
                })
            }
        })
    }

    /// Releases what outlives the session before the surface is dropped.
    fn release(&self, cx: &mut App) {
        match self {
            // Retained result bytes are shared with the workspace budget.
            Self::Sqlite(view) => view.update(cx, |view, cx| view.release_all(cx)),
            Self::Redis(_) | Self::ClickHouse(_) | Self::MySql(_) => {}
        }
    }

    fn tabs(&self, cx: &App) -> Vec<TabInfo> {
        match self {
            Self::Redis(view) => view.read(cx).tabs(),
            Self::Sqlite(view) => view.read(cx).tabs(cx),
            Self::ClickHouse(view) => view.read(cx).tabs(cx),
            Self::MySql(view) => view.read(cx).tabs(),
        }
    }

    fn owns_tab(&self, id: &str, cx: &App) -> bool {
        match self {
            Self::Redis(view) => view.read(cx).owns_tab(id),
            Self::Sqlite(view) => view.read(cx).owns_tab(id),
            Self::ClickHouse(view) => view.read(cx).owns_tab(id),
            Self::MySql(view) => view.read(cx).has_tab(id),
        }
    }

    fn select_tab(&self, id: &str, window: &mut Window, cx: &mut App) {
        match self {
            Self::Redis(view) => view.update(cx, |view, cx| view.select_tab_id(id, window, cx)),
            Self::Sqlite(view) => view.update(cx, |view, cx| view.select_tab(id, window, cx)),
            Self::ClickHouse(view) => view.update(cx, |view, cx| view.select_tab(id, window, cx)),
            Self::MySql(view) => view.update(cx, |view, cx| view.select_tab(id, window, cx)),
        }
    }

    fn close_tab(&self, id: &str, window: &mut Window, cx: &mut App) {
        match self {
            Self::Redis(view) => view.update(cx, |view, cx| view.close_tab_id(id, window, cx)),
            Self::Sqlite(view) => {
                view.update(cx, |view, cx| view.close_tab(id, window, cx));
            }
            Self::ClickHouse(view) => view.update(cx, |view, cx| view.close_tab(id, window, cx)),
            Self::MySql(view) => view.update(cx, |view, cx| view.close_tab(id, window, cx)),
        }
    }

    /// A new query tab; Redis shows its console.
    fn new_tab(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::Redis(view) => view.update(cx, |view, cx| view.show_console(window, cx)),
            Self::Sqlite(view) => view.update(cx, |view, cx| view.new_query(window, cx)),
            Self::ClickHouse(view) => view.update(cx, |view, cx| view.new_query(window, cx)),
            Self::MySql(view) => view.update(cx, |view, cx| view.new_query(window, cx)),
        }
    }

    fn close_active(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::Redis(view) => view.update(cx, |view, cx| view.close_tab(window, cx)),
            Self::Sqlite(view) => {
                view.update(cx, |view, cx| view.close_active(window, cx));
            }
            Self::ClickHouse(view) => view.update(cx, |view, cx| view.close_active(window, cx)),
            Self::MySql(view) => view.update(cx, |view, cx| view.close_active(window, cx)),
        }
    }

    fn cycle(&self, forward: bool, window: &mut Window, cx: &mut App) {
        match self {
            Self::Redis(view) => view.update(cx, |view, cx| view.cycle_tab(forward, window, cx)),
            Self::Sqlite(view) => {
                view.update(cx, |view, cx| view.select_next(!forward, window, cx))
            }
            Self::ClickHouse(view) => view.update(cx, |view, cx| view.cycle(forward, window, cx)),
            Self::MySql(view) => view.update(cx, |view, cx| view.cycle(forward, window, cx)),
        }
    }

    /// The engine's display name, as in `DevelopmentConnection::engine`.
    fn engine(&self) -> &'static str {
        match self {
            Self::Redis(_) => "Redis",
            Self::Sqlite(_) => "SQLite",
            Self::ClickHouse(_) => "ClickHouse",
            Self::MySql(_) => "MySQL",
        }
    }

    /// Whether [`clear`](Self::clear) does anything: Redis clears its
    /// console, ClickHouse the active tab's results. SQLite and MySQL tabs
    /// expose no clear yet.
    pub(super) fn can_clear(&self) -> bool {
        matches!(self, Self::Redis(_) | Self::ClickHouse(_))
    }

    /// Clears the active tab's results; false when the engine has none to
    /// clear.
    fn clear(&self, window: &mut Window, cx: &mut App) -> bool {
        match self {
            Self::Redis(view) => view.update(cx, |view, cx| view.clear_console(window, cx)),
            Self::ClickHouse(view) => view.update(cx, |view, cx| view.clear_results(cx)),
            Self::Sqlite(_) | Self::MySql(_) => return false,
        }
        true
    }

    /// Open Anything entries for this surface's tabs on `connection`.
    pub(super) fn palette_items<C>(
        &self,
        connection: &str,
        connection_name: &str,
        cx: &App,
    ) -> Vec<Item<C>> {
        tab_items(connection, connection_name, &self.tabs(cx))
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::Redis(view) => view.update(cx, |view, cx| view.focus_active(window, cx)),
            Self::Sqlite(view) => view.update(cx, |view, cx| view.focus_active(window, cx)),
            Self::ClickHouse(view) => view.update(cx, |view, cx| view.focus_active(window, cx)),
            Self::MySql(view) => view.update(cx, |view, cx| view.focus_active(window, cx)),
        }
    }

    /// The document area under the tab bar.
    fn body(&self) -> AnyView {
        match self {
            Self::Redis(view) => view.clone().into(),
            Self::Sqlite(view) => view.clone().into(),
            Self::ClickHouse(view) => view.clone().into(),
            Self::MySql(view) => view.clone().into(),
        }
    }
}

impl Workspace {
    fn engine_connection(&self, id: &str) -> Option<&DevelopmentConnection> {
        self.connections
            .iter()
            .find(|connection| connection.id == id && surface_connection(connection))
    }

    fn surface(&self, id: &str) -> Option<&Surface> {
        self.engines
            .0
            .iter()
            .find(|surface| surface.opened.id == id)
    }

    fn active_surface(&self) -> Option<&Surface> {
        let id = self.selected_connection.as_deref()?;
        self.engine_connection(id)?;
        self.surface(id)
    }

    /// The surface of the selected connection, when it has one.
    pub(super) fn active_engine(&self) -> Option<&EngineSurface> {
        self.active_surface().map(|surface| &surface.surface)
    }

    pub(super) fn active_engine_connection(&self) -> Option<&DevelopmentConnection> {
        self.active_surface()?;
        self.engine_connection(self.selected_connection.as_deref()?)
    }

    /// An engine connection's phase: its surface's, else Idle. `None` for
    /// PostgreSQL connections.
    pub(super) fn engine_phase(&self, id: &str, cx: &App) -> Option<ConnectionPhase> {
        match self.surface(id) {
            Some(surface) => Some(surface.surface.phase(cx)),
            None => self.engine_connection(id).map(|_| ConnectionPhase::Idle),
        }
    }

    /// Sidebar tree for the active surface.
    pub(super) fn engine_tree(&self) -> Option<AnyView> {
        self.active_surface().map(|surface| surface.tree.clone())
    }

    /// Tabs of the active surface.
    pub(super) fn engine_tabs(&self, cx: &App) -> Option<Vec<TabInfo>> {
        self.active_engine().map(|surface| surface.tabs(cx))
    }

    /// Document area for the active surface.
    pub(super) fn engine_body(&self) -> Option<AnyView> {
        self.active_engine().map(EngineSurface::body)
    }

    pub(super) fn focus_engine(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(surface) = self.active_engine() {
            surface.focus(window, cx);
        }
    }

    /// `Some(can_clear)` while an engine surface is selected.
    pub(super) fn active_engine_can_clear(&self) -> Option<bool> {
        self.active_engine().map(EngineSurface::can_clear)
    }

    /// Open Anything entries for every surface's tabs, each with the
    /// connection and tab it opens.
    pub(super) fn engine_palette_items<C>(&self, cx: &App) -> Vec<(String, String, Item<C>)> {
        let mut items = Vec::new();
        for surface in &self.engines.0 {
            let connection = &surface.opened;
            for item in surface
                .surface
                .palette_items(&connection.id, &connection.name, cx)
            {
                if let Target::Tab(tab) = &item.target {
                    items.push((connection.id.clone(), tab.clone(), item));
                }
            }
        }
        items
    }

    /// Selects `connection` (without connecting: choosing a tab is not the
    /// connect gesture) and shows its tab `tab`.
    pub(super) fn open_engine_tab(
        &mut self,
        connection: String,
        tab: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(surface) = self
            .surface(&connection)
            .map(|surface| surface.surface.clone())
        else {
            self.refuse("That connection's tabs were closed", cx);
            return;
        };
        if self.engine_connection(&connection).is_none() || !surface.owns_tab(&tab, cx) {
            self.refuse("That tab was closed", cx);
            return;
        }
        if self.selected_connection.as_deref() != Some(connection.as_str()) {
            self.remember_focus(window, cx);
            self.follow_connection_project(&connection);
            self.selected_connection = Some(connection);
        }
        surface.select_tab(&tab, window, cx);
        cx.notify();
    }

    fn ensure_surface(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.surface(id).is_some() {
            return;
        }
        let Some(connection) = self.engine_connection(id).cloned() else {
            return;
        };
        let (host, retained) = (self.host.clone(), self.retained.clone());
        let (surface, tree, subscriptions) = match &connection.settings {
            Some(DevelopmentEngineConnection::Redis(_)) => {
                let view = cx.new(|cx| RedisWorkspace::new(id.to_owned(), host, window, cx));
                let tree = view.update(cx, |view, cx| view.tree_view(cx)).into();
                let watch = watch(&view, id, |RedisEvent::Latency(ms)| Some(*ms), cx);
                (EngineSurface::Redis(view), tree, watch)
            }
            Some(DevelopmentEngineConnection::SQLite(_)) => {
                let view =
                    cx.new(|cx| SqliteWorkspace::new(id.to_owned(), host, retained, window, cx));
                let tree = view.read(cx).tree().into();
                let watch = watch(
                    &view,
                    id,
                    |event| match event {
                        SqliteEvent::Latency(ms) => Some(*ms),
                        SqliteEvent::Changed => None,
                    },
                    cx,
                );
                (EngineSurface::Sqlite(view), tree, watch)
            }
            Some(DevelopmentEngineConnection::ClickHouse(_)) => {
                let view = cx
                    .new(|cx| ClickHouseWorkspace::new(host, id.to_owned(), retained, window, cx));
                let tree = view.read(cx).tree().into();
                let watch = watch(&view, id, |ClickHouseEvent::Latency(ms)| Some(*ms), cx);
                (EngineSurface::ClickHouse(view), tree, watch)
            }
            Some(DevelopmentEngineConnection::MySQL(_)) => {
                let view = cx.new(|cx| MySqlLane::new(host, id.to_owned(), window, cx));
                let tree = view.read(cx).tree_view().into();
                let watch = watch(&view, id, |MySqlEvent::Latency(ms)| Some(*ms), cx);
                (EngineSurface::MySql(view), tree, watch)
            }
            _ => return,
        };
        self.engines.0.push(Surface {
            opened: connection,
            surface,
            tree,
            _subscriptions: subscriptions,
        });
    }

    /// Detaches the session of `id`'s surface and closes it; a failed close
    /// is reported.
    fn disconnect_engine(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(closing) = self
            .surface(id)
            .and_then(|surface| surface.surface.detach(cx))
        else {
            return;
        };
        let task = self.host.runtime.spawn(close_within(vec![closing]));
        cx.spawn(async move |this, cx| {
            let result = task
                .await
                .unwrap_or_else(|_| Err("Disconnect task failed".into()));
            if let Err(error) = result {
                this.update(cx, |this, cx| {
                    this.message = Some(error);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Handles operations aimed at an engine connection or at the active
    /// surface. Returns false to fall through to the document workspace.
    pub(super) fn engine_operation(
        &mut self,
        operation: &Operation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match operation {
            Operation::SelectConnection(id) if self.engine_connection(id).is_some() => {
                self.remember_focus(window, cx);
                self.selected_connection = Some(id.clone());
                self.ensure_surface(id, window, cx);
                if let Some(surface) = self.surface(id) {
                    // Selecting is the explicit connect (and retry) gesture.
                    surface.surface.connect(window, cx);
                }
                self.focus_engine(window, cx);
                cx.notify();
                return true;
            }
            // Leaving a surface for a PostgreSQL connection: hand focus back
            // to the documents before the normal selection runs.
            Operation::SelectConnection(id) if self.active_engine().is_some() => {
                self.selected_connection = Some(id.clone());
                self.focus_active(window, cx);
                return false;
            }
            Operation::DisconnectConnection(id)
                if self.surface(id).is_some() || self.engine_connection(id).is_some() =>
            {
                self.disconnect_engine(id, cx);
                return true;
            }
            _ => {}
        }
        let Some(surface) = self.active_engine().cloned() else {
            return false;
        };
        match operation {
            Operation::SelectDocument(id) if surface.owns_tab(id, cx) => {
                surface.select_tab(id, window, cx)
            }
            Operation::CloseDocument(id) if surface.owns_tab(id, cx) => {
                surface.close_tab(id, window, cx)
            }
            // A workspace document chosen elsewhere (palette) leaves the
            // surface for that document's connection.
            Operation::SelectDocument(id) => {
                if let Some(document) = self
                    .documents
                    .iter()
                    .find(|document| &document.metadata.id == id)
                {
                    self.selected_connection = document.metadata.connection_id.clone();
                }
                return false;
            }
            Operation::New => surface.new_tab(window, cx),
            Operation::Close => surface.close_active(window, cx),
            Operation::Next => surface.cycle(true, window, cx),
            Operation::Previous => surface.cycle(false, window, cx),
            Operation::Connect => surface.connect(window, cx),
            Operation::Disconnect => {
                if let Some(id) = self.selected_connection.clone() {
                    self.disconnect_engine(&id, cx);
                }
            }
            Operation::Clear => {
                if !surface.clear(window, cx) {
                    self.message = Some(format!(
                        "Clear results is not available for {} tabs yet",
                        surface.engine()
                    ));
                }
            }
            Operation::OpenTable => {
                self.message = Some("Open tables from the sidebar tree".into());
            }
            Operation::Rename
            | Operation::Pin
            | Operation::Move(_)
            | Operation::Library(_)
            | Operation::SaveQuery => {
                self.message = Some("Available for PostgreSQL connections only".into());
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    /// After connections reload (`form == None`) or a settled connection or
    /// credential form (`Some((scope, all))`): closes stale sessions and
    /// drops the surfaces of deleted or unsupported connections. Nothing
    /// reconnects by itself.
    pub(super) fn reconcile_engines(
        &mut self,
        form: Option<(Option<&str>, bool)>,
        cx: &mut Context<Self>,
    ) {
        let mut closing = Vec::new();
        let connections = &self.connections;
        self.engines.0.retain_mut(|surface| {
            let current = connections
                .iter()
                .find(|connection| connection.id == surface.opened.id);
            match fate(&surface.opened, current, form) {
                Fate::Keep => true,
                Fate::Close => {
                    if let Some(current) = current {
                        surface.opened = current.clone();
                    }
                    closing.extend(surface.surface.detach(cx));
                    true
                }
                Fate::Drop => {
                    closing.extend(surface.surface.detach(cx));
                    surface.surface.release(cx);
                    false
                }
            }
        });
        if !closing.is_empty() {
            self.host.runtime.spawn(async move {
                if let Err(error) = close_within(closing).await {
                    log::warn!("{error}");
                }
            });
        }
    }

    /// Detaches every session for a joined close at quit.
    pub(super) fn detach_engines(&mut self, cx: &mut Context<Self>) -> Vec<Closing> {
        self.engines
            .0
            .iter()
            .filter_map(|surface| surface.surface.detach(cx))
            .collect()
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use dbunk_lib::backend::{
        DevelopmentClickHouseConnection, DevelopmentEnvironment, DevelopmentMySqlConnection,
        DevelopmentRedisConnection, DevelopmentSafeMode, DevelopmentSqliteConnection,
    };

    const ENV: DevelopmentEnvironment = DevelopmentEnvironment::Development;
    const SAFE: DevelopmentSafeMode = DevelopmentSafeMode::Inherit;

    pub(crate) fn record(
        engine: &str,
        settings: Option<DevelopmentEngineConnection>,
    ) -> DevelopmentConnection {
        DevelopmentConnection {
            id: "c".into(),
            name: "c".into(),
            engine: engine.into(),
            organization: Default::default(),
            unsupported_reason: None,
            postgres: None,
            environment: ENV,
            settings,
        }
    }

    fn sqlite(path: &str, read_only: bool) -> DevelopmentConnection {
        record(
            "SQLite",
            Some(DevelopmentEngineConnection::SQLite(
                DevelopmentSqliteConnection {
                    name: "local".into(),
                    path: path.into(),
                    environment: ENV,
                    safe_mode: SAFE,
                    read_only,
                },
            )),
        )
    }

    fn redis(host: &str) -> DevelopmentConnection {
        record(
            "Redis",
            Some(DevelopmentEngineConnection::Redis(
                DevelopmentRedisConnection {
                    name: "cache".into(),
                    host: host.into(),
                    port: 6379,
                    db_number: 0,
                    user: String::new(),
                    environment: ENV,
                    safe_mode: SAFE,
                    read_only: false,
                    use_tls: false,
                    verify_tls_cert: true,
                    ssh_tunnel: None,
                },
            )),
        )
    }

    fn clickhouse(host: &str) -> DevelopmentConnection {
        record(
            "ClickHouse",
            Some(DevelopmentEngineConnection::ClickHouse(
                DevelopmentClickHouseConnection {
                    name: "events".into(),
                    host: host.into(),
                    port: 8123,
                    database: "default".into(),
                    user: "default".into(),
                    environment: ENV,
                    safe_mode: SAFE,
                    read_only: false,
                    use_https: false,
                    url_path: String::new(),
                    ssh_tunnel: None,
                },
            )),
        )
    }

    fn mysql(host: &str) -> DevelopmentConnection {
        record(
            "MySQL",
            Some(DevelopmentEngineConnection::MySQL(
                DevelopmentMySqlConnection {
                    name: "shop".into(),
                    host: host.into(),
                    port: 3306,
                    database: "shop".into(),
                    user: "root".into(),
                    environment: ENV,
                    safe_mode: SAFE,
                    read_only: false,
                    ssl: false,
                    ssh_tunnel: None,
                },
            )),
        )
    }

    /// One readable record per engine with a surface.
    pub(crate) fn engine_records() -> [DevelopmentConnection; 4] {
        [
            redis("127.0.0.1"),
            sqlite("/tmp/a.db", false),
            clickhouse("127.0.0.1"),
            mysql("127.0.0.1"),
        ]
    }

    fn unsupported(mut connection: DevelopmentConnection) -> DevelopmentConnection {
        connection.unsupported_reason = Some("unreadable".into());
        connection
    }

    #[test]
    fn palette_tab_entries_are_keyed_by_connection_and_open_their_tab() {
        let tab = |id: &str, title: &str| TabInfo {
            id: id.into(),
            title: title.into(),
            icon: "icons/terminal.svg",
            status: "12 rows".into(),
            active: false,
            pinned: false,
            closable: true,
        };
        let tabs = [tab("q1", "Query 1"), tab("t2", "users")];
        let items: Vec<Item<()>> = tab_items("conn-a", "shop", &tabs);
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].key, "engine-tab:conn-a:t2");
        assert_eq!(items[1].label, "users");
        assert_eq!(items[1].description, "Open tab · shop");
        assert_eq!(items[1].target, Target::Tab("t2".into()));
        // The same tab id on another connection is a different entry.
        let other: Vec<Item<()>> = tab_items("conn-b", "cache", &tabs[..1]);
        assert_ne!(other[0].key, items[0].key);
    }

    #[test]
    fn only_readable_supported_engine_records_get_a_surface() {
        for connection in engine_records() {
            assert!(surface_connection(&connection), "{}", connection.engine);
            // Unreadable or unsupported records stay visible but never connect.
            let mut unreadable = connection.clone();
            unreadable.settings = None;
            assert!(!surface_connection(&unreadable), "{}", connection.engine);
            assert!(
                !surface_connection(&unsupported(connection.clone())),
                "{}",
                connection.engine
            );
        }
        // PostgreSQL keeps the document workspace.
        let postgres = record("PostgreSQL", None);
        assert!(!surface_connection(&postgres));
    }

    #[test]
    fn a_changed_path_or_policy_changes_the_session_record() {
        let base = record_of(&sqlite("/tmp/a.db", false));
        assert_eq!(base, record_of(&sqlite("/tmp/a.db", false)));
        assert_ne!(base, record_of(&sqlite("/tmp/b.db", false)));
        assert_ne!(base, record_of(&sqlite("/tmp/a.db", true)));
        let mut renamed = sqlite("/tmp/a.db", false);
        renamed.organization.folder = "elsewhere".into();
        assert_eq!(
            base,
            record_of(&renamed),
            "organization is not a session input"
        );
    }

    #[test]
    fn only_edited_connections_or_credential_changes_end_sessions() {
        assert!(survives_change(None, false, "a"), "new connection");
        assert!(survives_change(Some("b"), false, "a"));
        assert!(!survives_change(Some("a"), false, "a"));
        assert!(!survives_change(None, true, "a"));
    }

    #[test]
    fn deleted_unsupported_or_retyped_connections_drop_their_surface() {
        for opened in engine_records() {
            assert_eq!(fate(&opened, None, None), Fate::Drop, "{}", opened.engine);
            let current = unsupported(opened.clone());
            assert_eq!(fate(&opened, Some(&current), None), Fate::Drop);
            let mut unreadable = opened.clone();
            unreadable.settings = None;
            assert_eq!(fate(&opened, Some(&unreadable), None), Fate::Drop);
            let postgres = record("PostgreSQL", None);
            assert_eq!(fate(&opened, Some(&postgres), None), Fate::Drop);
        }
        // Same id, different engine: the surface cannot serve it.
        assert_eq!(fate(&redis("h"), Some(&clickhouse("h")), None), Fate::Drop);
    }

    #[test]
    fn unchanged_records_keep_their_session_on_reload() {
        for opened in engine_records() {
            assert_eq!(fate(&opened, Some(&opened), None), Fate::Keep);
            let mut moved = opened.clone();
            moved.organization.folder = "elsewhere".into();
            assert_eq!(fate(&opened, Some(&moved), None), Fate::Keep);
        }
    }

    #[test]
    fn sqlite_closes_only_when_its_record_changes() {
        let opened = sqlite("/tmp/a.db", false);
        assert_eq!(
            fate(&opened, Some(&sqlite("/tmp/b.db", false)), None),
            Fate::Close
        );
        assert_eq!(
            fate(&opened, Some(&sqlite("/tmp/a.db", true)), None),
            Fate::Close
        );
        // No secret or route: credential and scoped forms leave it alone.
        assert_eq!(fate(&opened, Some(&opened), Some((None, true))), Fate::Keep);
        assert_eq!(
            fate(&opened, Some(&opened), Some((Some("c"), false))),
            Fate::Keep
        );
    }

    #[test]
    fn redis_and_clickhouse_close_on_record_scoped_or_credential_changes() {
        for (opened, edited) in [(redis("a"), redis("b")), (clickhouse("a"), clickhouse("b"))] {
            assert_eq!(fate(&opened, Some(&edited), None), Fate::Close);
            assert_eq!(
                fate(&opened, Some(&opened), Some((Some("c"), false))),
                Fate::Close
            );
            assert_eq!(
                fate(&opened, Some(&opened), Some((None, true))),
                Fate::Close
            );
            // Another connection's edit or a new connection: untouched.
            assert_eq!(
                fate(&opened, Some(&opened), Some((Some("other"), false))),
                Fate::Keep
            );
            assert_eq!(
                fate(&opened, Some(&opened), Some((None, false))),
                Fate::Keep
            );
        }
    }

    #[test]
    fn mysql_leaves_edit_retirement_to_the_backend() {
        let opened = mysql("a");
        assert_eq!(fate(&opened, Some(&mysql("b")), None), Fate::Keep);
        assert_eq!(fate(&opened, Some(&opened), Some((None, true))), Fate::Keep);
        assert_eq!(
            fate(&opened, Some(&opened), Some((Some("c"), false))),
            Fate::Keep
        );
        // Deletion still drops the lane (and closes its session).
        assert_eq!(fate(&opened, None, None), Fate::Drop);
    }

    #[test]
    fn closes_run_concurrently_and_report_the_first_failure() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let step = Duration::from_millis(100);
            let slow = |result: Result<(), String>| -> Closing {
                Box::pin(async move {
                    tokio::time::sleep(step).await;
                    result
                })
            };
            let started = tokio::time::Instant::now();
            let closes = vec![slow(Ok(())), slow(Ok(())), slow(Ok(()))];
            assert_eq!(close_by(closes, step * 5).await, Ok(()));
            assert!(started.elapsed() < step * 3, "joined, not sequential");
            let closes = vec![slow(Ok(())), slow(Err("first".into()))];
            assert_eq!(close_by(closes, step * 5).await, Err("first".into()));
            // A close that never finishes is abandoned at the limit.
            let started = tokio::time::Instant::now();
            let hung: Closing = Box::pin(std::future::pending());
            assert!(close_by(vec![hung, slow(Ok(()))], step * 2).await.is_err());
            assert!(started.elapsed() < step * 4);
        });
    }
}
