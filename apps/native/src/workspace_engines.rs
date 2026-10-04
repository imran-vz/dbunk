//! Plan 031 step 4: engine surfaces. A non-PostgreSQL connection with a
//! native session gets one surface per connection, owned here. While such a
//! connection is selected, the surface supplies the sidebar tree, the tab
//! strip and the document area in place of the PostgreSQL documents; the
//! status bar and connection row read its session phase.
//!
//! Engine-neutral seam: add a variant to [`EngineSurface`] and a match arm in
//! each method below. Nothing else in the workspace names an engine.
use super::*;
use crate::redis_view::{RedisEvent, RedisWorkspace};
use gpui::{AnyElement, AnyView, App};

pub(super) enum EngineSurface {
    Redis(Entity<RedisWorkspace>),
}

pub(super) struct Surface {
    connection: String,
    surface: EngineSurface,
    /// Sidebar tree and tab strip views, placed by the shell.
    tree: AnyView,
    tabs: AnyView,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
pub(super) struct EngineSurfaces(Vec<Surface>);

/// Engines whose connections open a native surface instead of documents.
fn has_surface(engine: &str) -> bool {
    matches!(engine, "Redis")
}

/// A saved, readable connection of an engine with a native surface.
pub(super) fn surface_connection(connection: &DevelopmentConnection) -> bool {
    has_surface(&connection.engine)
        && connection.settings.is_some()
        && connection.unsupported_reason.is_none()
}

impl EngineSurface {
    fn phase(&self, cx: &App) -> ConnectionPhase {
        match self {
            Self::Redis(view) => view.read(cx).phase(),
        }
    }
    fn connect(&self, cx: &mut App) {
        match self {
            Self::Redis(view) => view.update(cx, |view, cx| view.connect(cx)),
        }
    }
    fn disconnect(&self, cx: &mut App) {
        match self {
            Self::Redis(view) => view.update(cx, |view, cx| view.disconnect(cx)),
        }
    }
}

impl Workspace {
    fn engine_connection(&self, id: &str) -> Option<&DevelopmentConnection> {
        self.connections
            .iter()
            .find(|connection| connection.id == id && surface_connection(connection))
    }

    fn surface(&self, id: &str) -> Option<&EngineSurface> {
        self.engines
            .0
            .iter()
            .find(|surface| surface.connection == id)
            .map(|surface| &surface.surface)
    }

    /// The surface of the selected connection, when it has one.
    pub(super) fn active_engine(&self) -> Option<&EngineSurface> {
        let id = self.selected_connection.as_deref()?;
        self.engine_connection(id)?;
        self.surface(id)
    }

    pub(super) fn active_engine_connection(&self) -> Option<&DevelopmentConnection> {
        self.active_engine()?;
        self.engine_connection(self.selected_connection.as_deref()?)
    }

    pub(super) fn engine_phase(&self, id: &str, cx: &App) -> Option<ConnectionPhase> {
        self.surface(id).map(|surface| surface.phase(cx))
    }

    fn ensure_surface(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.surface(id).is_some() {
            return;
        }
        let Some(connection) = self.engine_connection(id) else {
            return;
        };
        let surface = match connection.engine.as_str() {
            "Redis" => {
                let view =
                    cx.new(|cx| RedisWorkspace::new(id.to_owned(), self.host.clone(), window, cx));
                let connection = id.to_owned();
                let (tree, tabs) = view.update(cx, |view, cx| {
                    (view.tree_view(cx).into(), view.tabs_view(cx).into())
                });
                Surface {
                    connection: id.to_owned(),
                    tree,
                    tabs,
                    _subscriptions: vec![
                        cx.observe(&view, |_, _, cx| cx.notify()),
                        cx.subscribe(&view, move |this, _, event: &RedisEvent, cx| match event {
                            RedisEvent::Latency(ms) => {
                                this.shell.last_latency.insert(connection.clone(), *ms);
                                cx.notify();
                            }
                        }),
                    ],
                    surface: EngineSurface::Redis(view),
                }
            }
            _ => return,
        };
        self.engines.0.push(surface);
    }

    /// Handles operations aimed at an engine connection or at the active
    /// surface. Returns false to fall through to the document workspace.
    pub(super) fn engine_intercepts(
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
                    surface.connect(cx);
                    self.focus_engine(window, cx);
                }
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
            Operation::DisconnectConnection(id) if self.surface(id).is_some() => {
                if let Some(surface) = self.surface(id) {
                    surface.disconnect(cx);
                }
                return true;
            }
            _ => {}
        }
        let Some(EngineSurface::Redis(view)) = self.active_engine() else {
            return false;
        };
        let view = view.clone();
        match operation {
            Operation::New => view.update(cx, |view, cx| view.show_console(window, cx)),
            Operation::Close => view.update(cx, |view, cx| view.close_tab(window, cx)),
            Operation::Next => view.update(cx, |view, cx| view.cycle_tab(true, window, cx)),
            Operation::Previous => view.update(cx, |view, cx| view.cycle_tab(false, window, cx)),
            Operation::Connect => view.update(cx, |view, cx| view.connect(cx)),
            Operation::Disconnect => view.update(cx, |view, cx| view.disconnect(cx)),
            Operation::Clear => view.update(cx, |view, cx| view.clear_console(window, cx)),
            Operation::OpenTable
            | Operation::Library(_)
            | Operation::SaveQuery
            | Operation::Rename
            | Operation::Pin
            | Operation::Move(_) => {
                self.message = Some("Not available for Redis connections".into());
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    pub(super) fn focus_engine(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(EngineSurface::Redis(view)) = self.active_engine() {
            view.update(cx, |view, cx| view.focus_active(window, cx));
        }
    }

    fn active_surface(&self) -> Option<&Surface> {
        self.active_engine()?;
        let id = self.selected_connection.as_deref()?;
        self.engines
            .0
            .iter()
            .find(|surface| surface.connection == id)
    }

    /// Sidebar tree for the active surface.
    pub(super) fn engine_tree(&self) -> Option<AnyView> {
        self.active_surface().map(|surface| surface.tree.clone())
    }

    /// Tab strip for the active surface.
    pub(super) fn engine_tabs(&self) -> Option<AnyView> {
        self.active_surface().map(|surface| surface.tabs.clone())
    }

    /// Document area for the active surface.
    pub(super) fn engine_body(&self) -> Option<AnyElement> {
        match self.active_engine()? {
            EngineSurface::Redis(view) => Some(view.clone().into_any_element()),
        }
    }

    /// Closes the sessions of `scope` (every surface when `None`): after a
    /// connection or credential edit, and when the window closes.
    pub(super) fn retire_engines(&mut self, scope: Option<&str>, cx: &mut Context<Self>) {
        for surface in &self.engines.0 {
            if scope.is_none_or(|id| id == surface.connection) {
                surface.surface.disconnect(cx);
            }
        }
    }

    /// Drops surfaces whose connection was deleted or no longer has one.
    pub(super) fn sync_engines(&mut self, cx: &mut Context<Self>) {
        let keep: Vec<bool> = self
            .engines
            .0
            .iter()
            .map(|surface| self.engine_connection(&surface.connection).is_some())
            .collect();
        let mut keep = keep.into_iter();
        self.engines.0.retain(|surface| {
            let kept = keep.next().unwrap_or(false);
            if !kept {
                surface.surface.disconnect(cx);
            }
            kept
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::{
        DevelopmentEngineConnection, DevelopmentEnvironment, DevelopmentRedisConnection,
        DevelopmentSafeMode,
    };

    fn redis(settings: bool, unsupported: bool) -> DevelopmentConnection {
        DevelopmentConnection {
            id: "r".into(),
            name: "cache".into(),
            engine: "Redis".into(),
            organization: Default::default(),
            unsupported_reason: unsupported.then(|| "unreadable".into()),
            postgres: None,
            environment: DevelopmentEnvironment::Development,
            settings: settings.then(|| {
                DevelopmentEngineConnection::Redis(DevelopmentRedisConnection {
                    name: "cache".into(),
                    host: "127.0.0.1".into(),
                    port: 6379,
                    db_number: 0,
                    user: String::new(),
                    environment: DevelopmentEnvironment::Development,
                    safe_mode: DevelopmentSafeMode::Inherit,
                    read_only: false,
                    use_tls: false,
                    verify_tls_cert: true,
                    ssh_tunnel: None,
                })
            }),
        }
    }

    #[test]
    fn only_readable_redis_records_get_a_surface() {
        assert!(surface_connection(&redis(true, false)));
        // Unreadable or unsupported records stay visible but never connect.
        assert!(!surface_connection(&redis(false, false)));
        assert!(!surface_connection(&redis(true, true)));
        let mut mysql = redis(true, false);
        mysql.engine = "MySQL".into();
        assert!(!surface_connection(&mysql));
    }
}
