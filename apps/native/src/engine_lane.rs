//! Plan 031 step 4: the seam between the workspace shell and engines other
//! than PostgreSQL. An engine lane owns one connection's session, sidebar
//! tree and documents; the shell only asks it for those surfaces and routes
//! tab and connection commands to it. PostgreSQL keeps its existing document
//! model. Each engine adds one variant here.
use crate::{controller::Host, document_view::ConnectionPhase};
use dbunk_lib::backend::DevelopmentConnection;
use gpui::{AnyView, App, AppContext as _, Entity, Window};
use std::sync::Arc;

#[derive(Clone)]
pub enum EngineLane {
    MySql(Entity<crate::mysql_lane::MySqlLane>),
}

/// One tab in the shell's tab bar for a lane document.
pub struct EngineTab {
    pub id: String,
    pub title: String,
    pub icon: &'static str,
    pub active: bool,
}

impl EngineLane {
    /// Whether a lane can serve this saved connection.
    pub fn supports(connection: &DevelopmentConnection) -> bool {
        connection.unsupported_reason.is_none()
            && connection.settings.is_some()
            && connection.engine == "MySQL"
    }

    pub fn new(
        connection: &DevelopmentConnection,
        host: Arc<Host>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Self> {
        if !Self::supports(connection) {
            return None;
        }
        let id = connection.id.clone();
        Some(Self::MySql(cx.new(|cx| {
            crate::mysql_lane::MySqlLane::new(host, id, window, cx)
        })))
    }

    /// Repaints `V` (the workspace) whenever the lane changes.
    pub fn observe<V: 'static>(&self, cx: &mut gpui::Context<V>) -> gpui::Subscription {
        match self {
            Self::MySql(lane) => cx.observe(lane, |_, _, cx| cx.notify()),
        }
    }

    pub fn phase(&self, cx: &App) -> ConnectionPhase {
        match self {
            Self::MySql(lane) => lane.read(cx).phase(),
        }
    }

    pub fn last_latency(&self, cx: &App) -> Option<u64> {
        match self {
            Self::MySql(lane) => lane.read(cx).last_latency(),
        }
    }

    pub fn tabs(&self, cx: &App) -> Vec<EngineTab> {
        match self {
            Self::MySql(lane) => lane.read(cx).tabs(),
        }
    }

    pub fn has_tab(&self, id: &str, cx: &App) -> bool {
        match self {
            Self::MySql(lane) => lane.read(cx).has_tab(id),
        }
    }

    /// The sidebar tree below the connection list.
    pub fn tree(&self, cx: &App) -> AnyView {
        match self {
            Self::MySql(lane) => lane.read(cx).tree_view().into(),
        }
    }

    /// The workspace area under the tab bar.
    pub fn content(&self) -> AnyView {
        match self {
            Self::MySql(lane) => lane.clone().into(),
        }
    }

    pub fn connect(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::MySql(lane) => lane.update(cx, |lane, cx| lane.connect(window, cx)),
        }
    }

    pub fn disconnect(&self, cx: &mut App) {
        match self {
            Self::MySql(lane) => lane.update(cx, |lane, cx| lane.disconnect(cx)),
        }
    }

    pub fn new_tab(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::MySql(lane) => lane.update(cx, |lane, cx| lane.new_query(window, cx)),
        }
    }

    pub fn select_tab(&self, id: &str, window: &mut Window, cx: &mut App) {
        match self {
            Self::MySql(lane) => lane.update(cx, |lane, cx| lane.select_tab(id, window, cx)),
        }
    }

    pub fn cycle_tabs(&self, back: bool, window: &mut Window, cx: &mut App) {
        match self {
            Self::MySql(lane) => lane.update(cx, |lane, cx| lane.cycle(back, window, cx)),
        }
    }

    pub fn close_tab(&self, id: &str, cx: &mut App) {
        match self {
            Self::MySql(lane) => lane.update(cx, |lane, cx| lane.close_tab(id, cx)),
        }
    }

    pub fn close_active_tab(&self, cx: &mut App) {
        match self {
            Self::MySql(lane) => lane.update(cx, |lane, cx| lane.close_active(cx)),
        }
    }
}
