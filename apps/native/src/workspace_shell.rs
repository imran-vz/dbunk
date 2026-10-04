//! Plan 031 shell: sidebar (traffic-light row, project/environment switcher,
//! connection search and list, object tree), tab bar, production strip,
//! collapsible status bar and the environment frame + tint.
use super::*;
use crate::document_view::ConnectionPhase;
use crate::style;
use dbunk_lib::backend::DevelopmentEnvironment;
use editor::Editor;
use gpui::{
    AnimationExt, AnyElement, MouseButton, SpringAnimation, SpringConfig, WindowControlArea, rgba,
    svg,
};

/// Width of the tab bar's left inset when the sidebar is hidden: room for
/// the traffic lights plus the show-sidebar button.
const REVEAL: f32 = style::TRAFFIC_LIGHTS + 28.;

#[derive(Clone, PartialEq)]
pub(super) enum ShellMenu {
    Projects,
    Tools,
    Connection(String),
}

pub(super) struct ShellState {
    pub sidebar_collapsed: bool,
    /// Set by the first toggle; until then the sidebar renders without motion.
    pub sidebar_toggled: bool,
    /// True while the open/close spring runs, so both ends stay rendered.
    pub sidebar_settling: bool,
    pub sidebar_epoch: u64,
    pub status_collapsed: bool,
    /// `None` until connections load; then the first project.
    pub project: Option<String>,
    pub env_filter: Option<DevelopmentEnvironment>,
    pub menu: Option<ShellMenu>,
    pub search: Entity<Editor>,
    pub search_field: Entity<crate::accessible_editor::AccessibleEditor>,
    pub last_latency: std::collections::HashMap<String, u64>,
    /// Armed by mouse-down on an empty titlebar region; the next move drags.
    should_move: std::rc::Rc<std::cell::Cell<bool>>,
    _search_events: Subscription,
}
impl ShellState {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let search = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Search connections", window, cx);
            editor
        });
        let search_field = cx.new(|cx| {
            crate::accessible_editor::AccessibleEditor::field(
                search.clone(),
                "Search connections",
                false,
                cx,
            )
        });
        let events = cx.subscribe(&search, |_, _, event: &editor::EditorEvent, cx| {
            if matches!(event, editor::EditorEvent::BufferEdited) {
                cx.notify();
            }
        });
        Self {
            sidebar_collapsed: false,
            sidebar_toggled: false,
            sidebar_settling: false,
            sidebar_epoch: 0,
            status_collapsed: false,
            project: None,
            env_filter: None,
            menu: None,
            search,
            search_field,
            last_latency: Default::default(),
            should_move: Default::default(),
            _search_events: events,
        }
    }
}

fn project_of(connection: &DevelopmentConnection) -> &str {
    if connection.organization.project.is_empty() {
        "Ungrouped"
    } else {
        &connection.organization.project
    }
}
/// Projects in first-seen order of a name-sorted list; stable across renders.
pub(super) fn projects(connections: &[DevelopmentConnection]) -> Vec<String> {
    let mut projects: Vec<String> = Vec::new();
    for connection in connections {
        let project = project_of(connection);
        if !projects.iter().any(|known| known == project) {
            projects.push(project.to_owned());
        }
    }
    projects.sort_by_key(|project| (project == "Ungrouped", project.to_lowercase()));
    projects
}

/// Groups visible connections. Without a search, one project is shown and
/// grouped by environment; a search spans projects as "Project · Env".
pub(super) fn groups<'a>(
    connections: &'a [DevelopmentConnection],
    project: Option<&str>,
    environment: Option<DevelopmentEnvironment>,
    search: &str,
) -> Vec<(
    String,
    DevelopmentEnvironment,
    Vec<&'a DevelopmentConnection>,
)> {
    let needle = search.trim().to_lowercase();
    let mut groups: Vec<(String, DevelopmentEnvironment, Vec<&DevelopmentConnection>)> = Vec::new();
    for connection in connections {
        if environment.is_some_and(|wanted| connection.environment != wanted) {
            continue;
        }
        if needle.is_empty() {
            if project.is_some_and(|project| project != project_of(connection)) {
                continue;
            }
        } else {
            let endpoint = connection
                .endpoint()
                .map(|endpoint| endpoint.label())
                .unwrap_or_default();
            let haystack = format!(
                "{} {} {} {}",
                connection.name,
                project_of(connection),
                connection.organization.folder,
                endpoint
            )
            .to_lowercase();
            if !haystack.contains(&needle) {
                continue;
            }
        }
        let label = if needle.is_empty() {
            style::env_label(connection.environment).to_owned()
        } else {
            format!(
                "{} · {}",
                project_of(connection),
                style::env_label(connection.environment)
            )
        };
        match groups.iter_mut().find(|(known, _, _)| known == &label) {
            Some((_, _, items)) => items.push(connection),
            None => groups.push((label, connection.environment, vec![connection])),
        }
    }
    groups.sort_by_key(|(label, environment, _)| {
        (
            style::ENVIRONMENTS
                .iter()
                .position(|known| known == environment)
                .unwrap_or(9),
            label.clone(),
        )
    });
    groups
}

/// One connection's state from its documents' query sessions and the host's
/// open sessions (`live`). An attempt in flight wins; then any open session;
/// then the first failure, which stays until a retry or a disconnect.
pub(super) fn connection_phase(
    live: bool,
    documents: impl IntoIterator<Item = ConnectionPhase>,
) -> ConnectionPhase {
    let mut failed = None;
    let mut connected = live;
    for phase in documents {
        match phase {
            ConnectionPhase::Connecting => return ConnectionPhase::Connecting,
            ConnectionPhase::Connected => connected = true,
            ConnectionPhase::Failed(error) => {
                failed.get_or_insert(error);
            }
            ConnectionPhase::Idle => {}
        }
    }
    if connected {
        ConnectionPhase::Connected
    } else if let Some(error) = failed {
        ConnectionPhase::Failed(error)
    } else {
        ConnectionPhase::Idle
    }
}

/// Whether the native host can open a session for this connection.
fn connectable(connection: &DevelopmentConnection) -> bool {
    (connection.postgres.is_some() && connection.unsupported_reason.is_none())
        || crate::sqlite_workspace::is_sqlite(connection)
        || super::engines::surface_connection(connection)
        || crate::clickhouse::is_clickhouse(connection)
        || (crate::engine_lane::EngineLane::supports(connection)
            && connection.unsupported_reason.is_none())
}

/// One tab in the tab bar, from a workspace document or an engine tab.
struct ShellTab {
    id: String,
    name: String,
    kind: &'static str,
    status: String,
    active: bool,
    pinned: bool,
}

fn sidebar_spring() -> SpringConfig {
    let (stiffness, damping, mass) = style::SIDEBAR_SPRING;
    SpringConfig::new(stiffness, damping, mass)
}

fn icon(path: &'static str, color: gpui::Rgba) -> impl IntoElement {
    svg()
        .path(path)
        .size(px(style::ICON))
        .flex_none()
        .text_color(color)
}

impl Workspace {
    /// Empty titlebar space: drag moves the window, double-click zooms.
    fn drag_region(&self, id: &'static str) -> gpui::Stateful<gpui::Div> {
        let down = self.shell.should_move.clone();
        let up = down.clone();
        let moving = down.clone();
        div()
            .id(id)
            .flex_1()
            .h_full()
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, move |_, _, _| down.set(true))
            .on_mouse_up(MouseButton::Left, move |_, _, _| up.set(false))
            .on_mouse_move(move |_, window, _| {
                if moving.replace(false) {
                    window.start_window_move();
                }
            })
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
    }
    fn shell_button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        operation: Operation,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let label = label.into();
        let click = operation.clone();
        let ax = operation.clone();
        let weak = cx.weak_entity();
        div()
            .id(id.into())
            .role(Role::Button)
            .aria_label(label.clone())
            .tab_index(0)
            .h(px(20.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(4.))
            .text_color(style::dim())
            .hover(|s| s.bg(style::hover()).text_color(style::text()))
            .focus(|s| s.bg(style::hover()).text_color(style::text()))
            .cursor_pointer()
            .active(|s| s.bg(style::pressed()))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(ax.clone(), window, cx))
                    .ok();
            })
            .on_click(
                cx.listener(move |this, _, window, cx| this.activate(click.clone(), window, cx)),
            )
    }
    fn icon_button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        path: &'static str,
        operation: Operation,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let label = label.into();
        crate::ui::press(self.shell_button(id, label.clone(), operation, cx))
            .w(px(20.))
            .px_0()
            .justify_center()
            .tooltip(crate::ui::tooltip(label))
            .tooltip_show_delay(crate::ui::tooltip_delay())
            .child(icon(path, style::dim()))
    }
    /// A button inside a connection row; it must not also select the row.
    fn row_button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        path: &'static str,
        color: gpui::Rgba,
        operation: Operation,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let label = label.into();
        self.shell_button(id, label.clone(), operation, cx)
            .on_click(|_, _, cx| cx.stop_propagation())
            .tooltip(crate::ui::tooltip(label))
            .tooltip_show_delay(crate::ui::tooltip_delay())
            .size(px(16.))
            .px_0()
            .justify_center()
            .child(icon(path, color))
    }
    fn phase_of(
        &self,
        id: &str,
        connected: &std::collections::BTreeSet<String>,
        cx: &gpui::App,
    ) -> ConnectionPhase {
        if let Some(phase) = self.clickhouse_phase(id, cx) {
            return phase;
        }
        if let Some(phase) = self.sqlite_phase(id, cx) {
            return phase;
        }
        connection_phase(
            connected.contains(id),
            self.documents
                .iter()
                .filter(|document| document.metadata.connection_id.as_deref() == Some(id))
                .map(|document| document.view.read(cx).connection_phase(cx))
                .chain(self.engine_phase(id, cx))
                .chain(self.engine_lane_of(id).map(|lane| lane.phase(cx))),
        )
    }
    fn current_connection(&self) -> Option<&DevelopmentConnection> {
        if let Some(connection) = self.active_engine_connection() {
            return Some(connection);
        }
        // A selected SQLite connection or engine lane owns the whole
        // workspace view.
        let owned = (self.sqlite_active().is_some() || self.engine_lane().is_some())
            .then_some(self.selected_connection.as_ref())
            .flatten();
        let id = owned
            .or_else(|| {
                self.active_index()
                    .and_then(|index| self.documents[index].metadata.connection_id.as_ref())
            })
            .or(self.selected_connection.as_ref())?;
        self.connections
            .iter()
            .find(|connection| &connection.id == id)
    }

    fn title_row(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("sidebar-title")
            .h(px(style::BAR))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .pl(px(style::TRAFFIC_LIGHTS))
            .pr(px(10.))
            .child(self.icon_button(
                "collapse-sidebar",
                "Hide sidebar  ⌘\\",
                "icons/threads_sidebar_left_open.svg",
                Operation::ToggleSidebar,
                cx,
            ))
            .child(self.drag_region("title-drag"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(style::text())
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(
                        div()
                            .size(px(12.))
                            .rounded(px(3.))
                            .bg(rgba((self.env_color() << 8) | 0xff)),
                    )
                    .child("dbunk"),
            )
    }
    pub(super) fn env_color(&self) -> u32 {
        style::env(
            self.current_connection()
                .map(|connection| connection.environment),
        )
    }

    fn switcher(&self, cx: &Context<Self>) -> impl IntoElement {
        let project = self
            .shell
            .project
            .clone()
            .unwrap_or_else(|| "No connections".into());
        let mut chips = vec![
            self.shell_button(
                "env-all",
                "All environments",
                Operation::EnvFilter(None),
                cx,
            )
            .when(self.shell.env_filter.is_none(), |chip| {
                chip.bg(style::raised()).text_color(style::text())
            })
            .aria_selected(self.shell.env_filter.is_none())
            .child("All")
            .into_any_element(),
        ];
        for environment in style::ENVIRONMENTS {
            let selected = self.shell.env_filter == Some(environment);
            chips.push(
                self.shell_button(
                    SharedString::from(format!("env-{}", style::env_label(environment))),
                    format!("{} only", style::env_label(environment)),
                    Operation::EnvFilter(Some(environment)),
                    cx,
                )
                .aria_selected(selected)
                .when(selected, |chip| {
                    chip.bg(style::raised()).text_color(style::text())
                })
                .px(px(4.))
                .child(
                    div()
                        .size(px(6.))
                        .rounded_full()
                        .bg(rgba((style::env(Some(environment)) << 8) | 0xff)),
                )
                .child(&style::env_label(environment)[..1])
                .into_any_element(),
            );
        }
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(8.))
            .pb(px(6.))
            .child(
                self.shell_button(
                    "project-switcher",
                    format!("Project {project}"),
                    Operation::ShellMenu(ShellMenu::Projects),
                    cx,
                )
                .flex_1()
                .min_w_0()
                .h(px(22.))
                .border_1()
                .border_color(style::line())
                .bg(style::raised())
                .text_color(style::text())
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(icon("icons/folder.svg", style::dim()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(project),
                )
                .child(icon("icons/chevron_down.svg", style::faint())),
            )
            .children(chips)
    }

    fn connection_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let search = self.shell.search.read(cx).text(cx);
        let connected = self.host.connected();
        let groups = groups(
            &self.connections,
            self.shell.project.as_deref(),
            self.shell.env_filter,
            &search,
        );
        let mut rows: Vec<AnyElement> = Vec::new();
        if groups.is_empty() {
            rows.push(
                div()
                    .px(px(16.))
                    .py(px(8.))
                    .text_color(style::faint())
                    .child(if self.connections.is_empty() {
                        "No connections yet"
                    } else {
                        "No connections match"
                    })
                    .into_any_element(),
            );
        }
        for (label, environment, items) in groups {
            rows.push(
                div()
                    .h(px(style::ROW))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .px(px(10.))
                    .text_size(px(style::FONT_SMALL))
                    .text_color(style::faint())
                    .child(
                        div()
                            .size(px(6.))
                            .rounded_full()
                            .bg(rgba((style::env(Some(environment)) << 8) | 0xff)),
                    )
                    .child(label.to_uppercase())
                    .into_any_element(),
            );
            for connection in items {
                let id = connection.id.clone();
                let selected = self.selected_connection.as_ref() == Some(&id);
                let phase = self.phase_of(&id, &connected, cx);
                let health = self.health.label(&id);
                let unhealthy = health
                    .as_ref()
                    .is_some_and(|label| !label.starts_with("Healthy"));
                let (badge, badge_color) = style::engine_badge(&connection.engine);
                let database = connection
                    .endpoint()
                    .map(|endpoint| endpoint.database)
                    .unwrap_or_default();
                let mut row = self
                    .shell_button(
                        SharedString::from(format!("connection-{id}")),
                        format!(
                            "{}, {}, {}{}{}",
                            connection.name,
                            connection.engine,
                            match &phase {
                                ConnectionPhase::Idle => "not connected".to_owned(),
                                ConnectionPhase::Connecting => "connecting".to_owned(),
                                ConnectionPhase::Connected => "connected".to_owned(),
                                ConnectionPhase::Failed(error) =>
                                    format!("connection failed: {error}"),
                            },
                            health.map(|label| format!(", {label}")).unwrap_or_default(),
                            connection
                                .unsupported_reason
                                .as_ref()
                                .map(|reason| format!(", {reason}"))
                                .unwrap_or_default()
                        ),
                        Operation::SelectConnection(id.clone()),
                        cx,
                    )
                    .role(Role::ListBoxOption)
                    .aria_selected(selected)
                    .rounded_none()
                    .pl(px(16.))
                    .pr(px(8.))
                    .gap(px(6.))
                    .text_color(style::text())
                    .when(selected, |row| row.bg(style::select()))
                    .child(
                        div()
                            .size(px(14.))
                            .rounded(px(3.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(8.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(style::bg())
                            .bg(rgba((badge_color << 8) | 0xff))
                            .child(badge),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(format!(
                                "{}{}",
                                if connection.organization.is_favorite {
                                    "★ "
                                } else {
                                    ""
                                },
                                connection.name
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(style::FONT_SMALL))
                            .text_color(match phase {
                                ConnectionPhase::Failed(_) => style::bad(),
                                ConnectionPhase::Connecting => style::accent(),
                                _ => style::faint(),
                            })
                            .child(match phase {
                                ConnectionPhase::Connecting => "connecting…".into(),
                                ConnectionPhase::Failed(_) => "failed".into(),
                                _ => database,
                            }),
                    );
                let dot = div().size(px(6.)).rounded_full();
                row = row.child(match phase {
                    ConnectionPhase::Connected => dot.bg(if unhealthy {
                        style::warn()
                    } else {
                        style::ok()
                    }),
                    ConnectionPhase::Connecting => dot.border_1().border_color(style::accent()),
                    ConnectionPhase::Failed(_) => dot.bg(style::bad()),
                    ConnectionPhase::Idle => dot.border_1().border_color(style::faint()),
                });
                if connectable(connection) && (selected || phase != ConnectionPhase::Idle) {
                    row = row.child(match phase {
                        ConnectionPhase::Connected | ConnectionPhase::Connecting => self
                            .row_button(
                                SharedString::from(format!("connection-toggle-{id}")),
                                format!("Disconnect {}", connection.name),
                                "icons/power.svg",
                                style::ok(),
                                Operation::DisconnectConnection(id.clone()),
                                cx,
                            ),
                        ConnectionPhase::Failed(_) => self.row_button(
                            SharedString::from(format!("connection-toggle-{id}")),
                            format!("Retry connecting {}", connection.name),
                            "icons/rotate_cw.svg",
                            style::bad(),
                            Operation::SelectConnection(id.clone()),
                            cx,
                        ),
                        ConnectionPhase::Idle => self.row_button(
                            SharedString::from(format!("connection-toggle-{id}")),
                            format!("Connect {}", connection.name),
                            "icons/power.svg",
                            style::dim(),
                            Operation::SelectConnection(id.clone()),
                            cx,
                        ),
                    });
                }
                if selected {
                    row = row.child(self.row_button(
                        SharedString::from(format!("connection-menu-{id}")),
                        format!("Actions for {}", connection.name),
                        "icons/ellipsis.svg",
                        style::dim(),
                        Operation::ShellMenu(ShellMenu::Connection(id.clone())),
                        cx,
                    ));
                }
                rows.push(row.into_any_element());
            }
        }
        div()
            .id("connection-list")
            .role(Role::ListBox)
            .aria_label("Connections")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pb(px(6.))
            .children(rows)
    }

    fn sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("sidebar")
            .role(Role::Group)
            .aria_label("Sidebar")
            .w(px(self.navigator_width.max(style::SIDEBAR)))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(style::panel())
            .border_r_1()
            .border_color(style::line())
            .child(
                div()
                    .h(gpui::relative(0.38))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .border_b_1()
                    .border_color(style::line())
                    .child(self.title_row(cx))
                    .child(self.switcher(cx))
                    .child(
                        div()
                            .mx(px(8.))
                            .mb(px(6.))
                            .h(px(22.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(7.))
                            .rounded(px(4.))
                            .bg(style::bg())
                            .border_1()
                            .border_color(style::line())
                            .child(icon("icons/magnifying_glass.svg", style::faint()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(18.))
                                    .child(self.shell.search_field.clone()),
                            )
                            .child(self.icon_button(
                                "new-connection",
                                "New connection",
                                "icons/plus.svg",
                                Operation::NewConnection,
                                cx,
                            )),
                    )
                    .child(self.connection_list(cx)),
            )
            .child(div().flex_1().min_h_0().flex().flex_col().map(|tree| {
                match (self.sqlite_active(), self.engine_tree(), self.engine_lane()) {
                    (Some(sqlite), _, _) => tree.child(sqlite.read(cx).tree()),
                    (None, Some(engine), _) => tree.child(engine),
                    (None, None, Some(lane)) => tree.child(lane.tree(cx)),
                    (None, None, None) => tree.child(self.object_tree()),
                }
            }))
    }

    fn tab_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let env = self.env_color();
        // A selected SQLite connection or engine lane shows its own tabs.
        let lane_tabs = self.engine_lane().map(|lane| {
            lane.tabs(cx)
                .into_iter()
                .map(|tab| ShellTab {
                    active: tab.active,
                    id: tab.id,
                    name: tab.title,
                    kind: tab.icon,
                    status: String::new(),
                    pinned: false,
                })
                .collect::<Vec<_>>()
        });
        let items = match self.sqlite_tabs(cx) {
            Some(tabs) => tabs
                .into_iter()
                .map(|tab| ShellTab {
                    active: tab.active,
                    id: tab.id,
                    name: tab.title,
                    kind: tab.icon,
                    status: tab.status,
                    pinned: false,
                })
                .collect::<Vec<_>>(),
            None if lane_tabs.is_some() => lane_tabs.unwrap_or_default(),
            None => self
                .documents
                .iter()
                .map(|document| ShellTab {
                    id: document.metadata.id.clone(),
                    name: document.metadata.name.clone(),
                    kind: if let Some(icon) = self.clickhouse_tab_icon(document, cx) {
                        icon
                    } else if document.metadata.tool.is_some() {
                        "icons/list_tree.svg"
                    } else if document.metadata.table.is_some() {
                        "icons/table.svg"
                    } else {
                        "icons/terminal.svg"
                    },
                    status: document.view.read(cx).document_status(cx).to_owned(),
                    active: Some(&document.metadata.id) == self.active.as_ref(),
                    pinned: document.metadata.pinned,
                })
                .collect(),
        };
        let tabs = items.into_iter().map(|tab| {
            let ShellTab {
                id,
                name,
                kind,
                status,
                active,
                pinned,
            } = tab;
            self.shell_button(
                SharedString::from(format!("tab-{id}")),
                if status.is_empty() {
                    name.clone()
                } else {
                    format!("{name}, {status}")
                },
                Operation::SelectDocument(id.clone()),
                cx,
            )
            .role(Role::Tab)
            .aria_selected(active)
            .h_full()
            .rounded_none()
            .min_w(px(110.))
            .max_w(px(200.))
            .px(px(10.))
            .gap(px(6.))
            .border_r_1()
            .border_color(style::line_soft())
            .relative()
            .when(active, |tab| {
                tab.bg(style::bg()).text_color(style::text()).child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(2.))
                        .bg(rgba((env << 8) | 0xff)),
                )
            })
            .child(icon(
                kind,
                if active { style::text() } else { style::dim() },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(format!("{}{}", if pinned { "● " } else { "" }, name)),
            )
            .group(SharedString::from(format!("tab-group-{id}")))
            .child({
                let group = SharedString::from(format!("tab-group-{id}"));
                let close = self
                    .row_button(
                        SharedString::from(format!("close-tab-{id}")),
                        format!("Close {name}"),
                        "icons/close.svg",
                        style::dim(),
                        Operation::CloseDocument(id.clone()),
                        cx,
                    )
                    .hover(|s| s.bg(style::hover()));
                // Hidden until the tab is hovered, always shown when active.
                if active {
                    close.opacity(0.8)
                } else {
                    close.opacity(0.).group_hover(group, |s| s.opacity(0.8))
                }
            })
        });
        div()
            .id("tab-bar")
            .h(px(style::BAR))
            .flex_none()
            .flex()
            .items_stretch()
            .bg(style::panel())
            .border_b_1()
            .border_color(style::line())
            .when(
                self.shell.sidebar_collapsed || self.shell.sidebar_settling,
                |bar| {
                    let collapsed = self.shell.sidebar_collapsed;
                    let mut spring = SpringAnimation::new(sidebar_spring()).to(px(if collapsed {
                        REVEAL
                    } else {
                        0.
                    }));
                    if self.shell.sidebar_toggled {
                        spring = spring.from(px(if collapsed { 0. } else { REVEAL }));
                    }
                    bar.child(
                        div()
                            .flex_none()
                            .h_full()
                            .overflow_hidden()
                            .child(
                                div()
                                    .w(px(REVEAL))
                                    .h_full()
                                    .pl(px(style::TRAFFIC_LIGHTS))
                                    .flex()
                                    .items_center()
                                    .child(self.icon_button(
                                        "expand-sidebar",
                                        "Show sidebar  ⌘\\",
                                        "icons/threads_sidebar_left_closed.svg",
                                        Operation::ToggleSidebar,
                                        cx,
                                    )),
                            )
                            .with_spring("tab-bar-reveal", spring, |reveal, width| reveal.w(width)),
                    )
                },
            )
            .child(
                div()
                    .id("document-tabs")
                    .role(Role::TabList)
                    .aria_label("Tabs")
                    .flex()
                    .min_w_0()
                    .overflow_x_scroll()
                    .map(|list| match self.engine_tabs() {
                        Some(engine) => list.child(engine),
                        None => list.children(tabs),
                    }),
            )
            .child(self.drag_region("tab-bar-drag"))
            .child(
                div()
                    .id("tab-actions")
                    .flex()
                    .when(self.active_engine().is_some(), |actions| actions.hidden())
                    .items_center()
                    .gap(px(2.))
                    .px(px(6.))
                    .border_l_1()
                    .border_color(style::line_soft())
                    .child(self.icon_button(
                        "new-query",
                        "New query",
                        "icons/plus.svg",
                        Operation::New,
                        cx,
                    ))
                    .child(
                        self.shell_button(
                            "tools-menu",
                            "Tools",
                            Operation::ShellMenu(ShellMenu::Tools),
                            cx,
                        )
                        .child("Tools")
                        .child(icon("icons/chevron_down.svg", style::faint())),
                    ),
            )
    }

    fn status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let env = self.env_color();
        if self.shell.status_collapsed {
            return div()
                .id("status-bar")
                .role(Role::Button)
                .aria_label("Show status bar")
                .tab_index(0)
                .h(px(4.))
                .flex_none()
                .bg(rgba((env << 8) | 0xff))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.activate(Operation::ToggleStatusBar, window, cx)
                }))
                .into_any_element();
        }
        let connection = self.current_connection();
        let phase = connection.map(|c| self.phase_of(&c.id, &self.host.connected(), cx));
        let save = match &self.save_status {
            SaveStatus::Pending => "Saving".to_string(),
            SaveStatus::Saved => "Saved".to_string(),
            SaveStatus::Failed(error) => format!("Not saved: {error}"),
        };
        let summary = connection
            .map(|c| {
                let database = c
                    .endpoint()
                    .map(|endpoint| format!(" · {}", endpoint.database))
                    .unwrap_or_default();
                format!("{}{database} · {}", c.name, c.engine)
            })
            .unwrap_or_else(|| "No connection".into());
        let latency = connection
            .and_then(|c| {
                self.shell.last_latency.get(&c.id).copied().or_else(|| {
                    self.engine_lane_of(&c.id)
                        .and_then(|lane| lane.last_latency(cx))
                })
            })
            .map(|ms| format!("last query {ms} ms"))
            .unwrap_or_else(|| "last query —".into());
        let state = match (&phase, connection) {
            (None, _) => "",
            (Some(ConnectionPhase::Connecting), _) => "Connecting",
            (Some(ConnectionPhase::Connected), _) => "Connected",
            (Some(ConnectionPhase::Failed(_)), _) => "Connection failed",
            (Some(ConnectionPhase::Idle), Some(c)) if !connectable(c) => "Sessions unavailable",
            (Some(ConnectionPhase::Idle), _) => "Disconnected",
        };
        let failure = match &phase {
            Some(ConnectionPhase::Failed(error)) => Some(error.clone()),
            _ => None,
        };
        let announcement = format!(
            "{state}{} {summary}, {latency}, {save}",
            failure
                .as_ref()
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        );
        div()
            .id("status-bar")
            .role(Role::Status)
            .aria_label(announcement)
            .h(px(style::STATUS))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(8.))
            .bg(style::panel())
            .border_t_1()
            .border_color(style::line())
            .text_size(px(style::FONT_SMALL))
            .text_color(style::dim())
            .whitespace_nowrap()
            .when_some(connection, |bar, c| {
                bar.child(
                    div()
                        .px(px(5.))
                        .rounded(px(3.))
                        .bg(rgba((env << 8) | 0xff))
                        .text_color(style::bg())
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(style::env_label(c.environment).to_uppercase()),
                )
            })
            .when_some(phase.clone(), |bar, phase| {
                let dot = div().size(px(6.)).rounded_full();
                bar.child(match phase {
                    ConnectionPhase::Connected => dot.bg(style::ok()),
                    ConnectionPhase::Connecting => dot.border_1().border_color(style::accent()),
                    ConnectionPhase::Failed(_) => dot.bg(style::bad()),
                    ConnectionPhase::Idle => dot.border_1().border_color(style::faint()),
                })
                .child(
                    div()
                        .when(failure.is_some(), |s| s.text_color(style::bad()))
                        .child(state),
                )
            })
            .child(summary)
            .child(latency)
            .child(match failure {
                Some(error) => div()
                    .id("connection-failure")
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(style::bad())
                    .child(error)
                    .into_any_element(),
                None => div().flex_1().into_any_element(),
            })
            .child(
                self.shell_button("console-badge", "Toggle console", Operation::Console, cx)
                    .h(px(18.))
                    .child(match self.dock.read(cx).unread() {
                        0 => "Console".to_string(),
                        unread => format!("Console · {unread}"),
                    }),
            )
            .child(
                div()
                    .id("workspace-save-status")
                    .text_color(if matches!(self.save_status, SaveStatus::Failed(_)) {
                        style::warn()
                    } else {
                        style::faint()
                    })
                    .child(if self.closing { "Closing".into() } else { save }),
            )
            .child(
                self.icon_button(
                    "collapse-status",
                    "Collapse status bar",
                    "icons/chevron_down.svg",
                    Operation::ToggleStatusBar,
                    cx,
                )
                .size(px(16.)),
            )
            .into_any_element()
    }

    fn menu_item(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        operation: Operation,
        cx: &Context<Self>,
    ) -> AnyElement {
        let label = label.into();
        self.shell_button(id, label.clone(), operation, cx)
            .role(Role::MenuItem)
            .rounded_none()
            .px(px(10.))
            .text_color(style::text())
            .child(label)
            .into_any_element()
    }
    fn menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.shell.menu.clone()?;
        let mut items: Vec<AnyElement> = Vec::new();
        let (left, top) = match &menu {
            ShellMenu::Projects => {
                for (index, project) in projects(&self.connections).into_iter().enumerate() {
                    items.push(self.menu_item(
                        SharedString::from(format!("project-{index}")),
                        project.clone(),
                        Operation::SelectProject(project),
                        cx,
                    ));
                }
                (px(8.), px(style::BAR + 24.))
            }
            ShellMenu::Tools => {
                for (index, (label, operation)) in [
                    ("Query history", Operation::Library(WorkspaceTool::History)),
                    (
                        "Saved queries",
                        Operation::Library(WorkspaceTool::SavedQueries),
                    ),
                    ("Objects", Operation::Library(WorkspaceTool::Objects)),
                    (
                        "Administration",
                        Operation::Library(WorkspaceTool::Administration),
                    ),
                    ("Schema map", Operation::Library(WorkspaceTool::SchemaMap)),
                    (
                        "Compare schemas",
                        Operation::Library(WorkspaceTool::SchemaCompare),
                    ),
                    (
                        "Backup / Restore",
                        Operation::Library(WorkspaceTool::BackupRestore),
                    ),
                    (
                        "CSV transfer",
                        Operation::Library(WorkspaceTool::CsvTransfer),
                    ),
                    ("Copy table", Operation::Library(WorkspaceTool::TableCopy)),
                    ("Open table by name", Operation::OpenTable),
                    ("Save query", Operation::SaveQuery),
                    ("Rename tab", Operation::Rename),
                    ("Pin tab", Operation::Pin),
                    ("Connect tab", Operation::Connect),
                    ("Disconnect tab", Operation::Disconnect),
                    ("Clear results", Operation::Clear),
                    ("Bastion servers", Operation::Bastions),
                    ("Managed servers", Operation::ManagedServers),
                    ("Credentials", Operation::Credentials),
                ]
                .into_iter()
                .enumerate()
                {
                    let general = self.host.backend.native_profile_kind()
                        == Some(dbunk_lib::backend::NativeProfileKind::GeneralPostgres);
                    if matches!(operation, Operation::Bastions | Operation::ManagedServers)
                        && !general
                    {
                        continue;
                    }
                    items.push(self.menu_item(
                        SharedString::from(format!("tool-{index}")),
                        label,
                        operation,
                        cx,
                    ));
                }
                (px(0.), px(style::BAR))
            }
            ShellMenu::Connection(id) => {
                let postgres = self
                    .connections
                    .iter()
                    .any(|c| &c.id == id && c.postgres.is_some() && c.unsupported_reason.is_none());
                items.push(self.menu_item(
                    "conn-edit",
                    "Edit…",
                    Operation::EditConnection(id.clone()),
                    cx,
                ));
                items.push(self.menu_item(
                    "conn-duplicate",
                    "Duplicate",
                    Operation::DuplicateConnection(id.clone()),
                    cx,
                ));
                if postgres {
                    items.push(self.menu_item(
                        "conn-uri",
                        "Copy URI",
                        Operation::CopyConnectionUri(id.clone()),
                        cx,
                    ));
                }
                items.push(self.menu_item(
                    "conn-favorite",
                    "Toggle favorite",
                    Operation::Favorite(id.clone()),
                    cx,
                ));
                items.push(self.menu_item(
                    "conn-delete",
                    "Delete…",
                    Operation::DeleteConnection(id.clone()),
                    cx,
                ));
                (px(style::SIDEBAR - 150.), px(style::BAR + 60.))
            }
        };
        let panel = div()
            .id("shell-menu")
            .role(Role::Menu)
            .aria_label("Menu")
            .absolute()
            .top(top)
            .min_w(px(170.))
            .py(px(4.))
            .flex()
            .flex_col()
            .bg(style::raised())
            .border_1()
            .border_color(style::line())
            .rounded(px(6.))
            .shadow_lg()
            .occlude()
            .children(items);
        let panel = match menu {
            ShellMenu::Tools => panel.right(px(8.)),
            _ => panel.left(left),
        };
        Some(
            div()
                .id("shell-menu-backdrop")
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.shell.menu = None;
                        cx.notify();
                    }),
                )
                .child(crate::ui::appear("shell-menu-panel", panel))
                .into_any_element(),
        )
    }

    pub(super) fn render_shell(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        window.set_rem_size(px(style::REM));
        let env = self.env_color();
        let production = self
            .current_connection()
            .is_some_and(|c| c.environment == DevelopmentEnvironment::Production);
        let workspace_area = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.tab_bar(cx))
            .when(production, |area| {
                area.child(
                    div()
                        .id("production-strip")
                        .role(Role::Alert)
                        .aria_label("Production: writes require review and confirmation")
                        .h(px(22.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(10.))
                        .bg(style::with_alpha(0xf85149, 0x24))
                        .text_color(gpui::rgb(0xffb3ad))
                        .border_b_1()
                        .border_color(style::with_alpha(0xf85149, 0x66))
                        .child(icon("icons/warning.svg", gpui::rgb(0xffb3ad)))
                        .child("Production · writes require review and confirmation"),
                )
            })
            .child(
                match (
                    self.sqlite_active().cloned(),
                    self.engine_body(),
                    self.active_index(),
                ) {
                    // A selected SQLite connection shows its own active document.
                    (Some(sqlite), _, _) => {
                        div().flex_1().min_h_0().child(sqlite).into_any_element()
                    }
                    (None, Some(body), _) => crate::ui::appear(
                        SharedString::from(format!(
                            "engine-{}",
                            self.selected_connection.as_deref().unwrap_or_default()
                        )),
                        div().flex_1().min_h_0().bg(style::bg()).child(body),
                    )
                    .into_any_element(),
                    // An engine lane renders its own documents.
                    (None, None, _) if self.engine_lane().is_some() => div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .bg(style::bg())
                        .children(self.engine_lane().map(|lane| lane.content()))
                        .into_any_element(),
                    // Each document fades and settles in when it becomes active.
                    (None, None, Some(index)) => crate::ui::appear(
                        SharedString::from(format!(
                            "document-{}",
                            self.documents[index].metadata.id
                        )),
                        div()
                            .flex_1()
                            .min_h_0()
                            .bg(style::bg())
                            .child(self.documents[index].view.clone()),
                    )
                    .into_any_element(),
                    (None, None, None) => div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(style::bg())
                        .text_color(style::faint())
                        .child(if self.loading {
                            "Loading workspace"
                        } else if self.restored {
                            "Select a connection, then open a table or ⌘T for a query"
                        } else {
                            "Workspace recovery required"
                        })
                        .into_any_element(),
                },
            )
            .when_some(self.message.clone(), |area, message| {
                // Keyed by text and by the action that set it, so a repeated
                // error shakes again.
                let key = {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    message.hash(&mut hasher);
                    self.message_seq.hash(&mut hasher);
                    hasher.finish()
                };
                area.child(crate::ui::shake(
                    ("workspace-error-shake", key),
                    div()
                        .id("workspace-error")
                        .role(Role::Alert)
                        .aria_label(message.clone())
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(10.))
                        .py(px(4.))
                        .bg(style::bad_fill())
                        .border_t_1()
                        .border_color(style::bad_line())
                        .text_color(style::bad_text())
                        .child(icon("icons/warning.svg", style::bad_text()))
                        .child(div().flex_1().child(message))
                        .when(
                            matches!(self.save_status, SaveStatus::Failed(_))
                                || self.load_error.is_some()
                                || self.cleanup_failed,
                            |strip| {
                                strip
                                    .child(
                                        self.shell_button(
                                            "retry-save",
                                            "Retry",
                                            Operation::Retry,
                                            cx,
                                        )
                                        .child("Retry"),
                                    )
                                    .child(
                                        self.shell_button(
                                            "export-sql",
                                            "Export",
                                            Operation::Export,
                                            cx,
                                        )
                                        .child(
                                            if self.load_error.is_some() {
                                                "Export saved JSON"
                                            } else {
                                                "Export SQL"
                                            },
                                        ),
                                    )
                                    .child(
                                        self.shell_button(
                                            "discard-drafts",
                                            "Discard",
                                            Operation::Discard,
                                            cx,
                                        )
                                        .child(
                                            if self.load_error.is_some() {
                                                "Reset saved workspace"
                                            } else {
                                                "Discard and quit"
                                            },
                                        ),
                                    )
                            },
                        ),
                ))
            })
            .when(self.dock.read(cx).is_open(), |area| {
                area.child(self.dock.clone())
            });
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(style::bg())
            .text_color(style::text())
            .text_size(px(style::FONT))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(
                        !self.shell.sidebar_collapsed || self.shell.sidebar_settling,
                        |row| {
                            let width = self.navigator_width.max(style::SIDEBAR);
                            let collapsed = self.shell.sidebar_collapsed;
                            let mut spring = SpringAnimation::new(sidebar_spring())
                                .to(px(if collapsed { 0. } else { width }));
                            if self.shell.sidebar_toggled {
                                spring = spring.from(px(if collapsed { width } else { 0. }));
                            }
                            // The sidebar keeps its width inside a clipping slot,
                            // so content slides out instead of reflowing.
                            row.child(
                                div()
                                    .flex_none()
                                    .h_full()
                                    .overflow_hidden()
                                    .child(self.sidebar(cx))
                                    .with_spring("sidebar-slot", spring, |slot, width| {
                                        slot.w(width)
                                    }),
                            )
                        },
                    )
                    .child(workspace_area),
            )
            .child(self.status_bar(cx))
            // Environment frame + tint: a 2 px border and a faint wash that
            // never intercepts input.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .border_2()
                    .border_color(rgba((env << 8) | 0xff))
                    .bg(style::with_alpha(env, 0x0a)),
            )
            .when_some(self.menu(cx), |root, menu| root.child(menu))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::DevelopmentConnectionOrganization;

    fn connection(
        name: &str,
        project: &str,
        environment: DevelopmentEnvironment,
    ) -> DevelopmentConnection {
        DevelopmentConnection {
            id: name.into(),
            name: name.into(),
            engine: "PostgreSQL".into(),
            organization: DevelopmentConnectionOrganization {
                project: project.into(),
                ..Default::default()
            },
            unsupported_reason: None,
            postgres: None,
            settings: None,
            environment,
        }
    }

    #[test]
    fn sqlite_connections_are_connectable_without_postgres_settings() {
        let mut local = connection("local", "", DevelopmentEnvironment::Development);
        assert!(!connectable(&local));
        local.engine = "SQLite".into();
        local.settings = Some(dbunk_lib::backend::DevelopmentEngineConnection::SQLite(
            dbunk_lib::backend::DevelopmentSqliteConnection {
                name: "local".into(),
                path: "/tmp/local.db".into(),
                environment: DevelopmentEnvironment::Development,
                safe_mode: dbunk_lib::backend::DevelopmentSafeMode::Inherit,
                read_only: false,
            },
        ));
        assert!(connectable(&local));
        local.unsupported_reason = Some("unsupported".into());
        assert!(!connectable(&local));
    }

    #[test]
    fn groups_follow_project_then_environment_order() {
        use DevelopmentEnvironment::*;
        let all = vec![
            connection("billing-prod", "Billing", Production),
            connection("billing-dev", "Billing", Development),
            connection("events", "Analytics", Development),
            connection("scratch", "", Test),
        ];
        assert_eq!(projects(&all), ["Analytics", "Billing", "Ungrouped"]);
        let billing = groups(&all, Some("Billing"), None, "");
        let labels: Vec<_> = billing
            .iter()
            .map(|(label, _, items)| (label.as_str(), items.len()))
            .collect();
        assert_eq!(labels, [("Dev", 1), ("Prod", 1)]);
        let prod_only = groups(&all, Some("Billing"), Some(Production), "");
        assert_eq!(prod_only.len(), 1);
        // Search spans projects and labels groups with their project.
        let search = groups(&all, Some("Billing"), None, "EVENTS");
        assert_eq!(search[0].0, "Analytics · Dev");
        assert!(groups(&all, Some("Billing"), None, "nothing").is_empty());
    }

    #[test]
    fn connection_phase_prefers_attempts_then_sessions_then_failures() {
        use ConnectionPhase::*;
        let failed = || Failed("password authentication failed".into());
        assert_eq!(connection_phase(false, []), Idle);
        assert_eq!(connection_phase(false, [Idle, failed()]), failed());
        // A retry in flight replaces the failure on the row.
        assert_eq!(connection_phase(true, [failed(), Connecting]), Connecting);
        // A table-lane session counts even when a query document failed.
        assert_eq!(connection_phase(true, [failed()]), Connected);
        assert_eq!(connection_phase(false, [Connected, failed()]), Connected);
        assert_eq!(
            connection_phase(false, [Failed("first".into()), Failed("second".into())]),
            Failed("first".into())
        );
    }
}
