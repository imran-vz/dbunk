//! Plan 031 shell: sidebar (traffic-light row, project/environment switcher,
//! connection search and list, object tree), tab bar, production strip,
//! collapsible status bar and the environment frame + tint.
use super::*;
use crate::document_view::{ConnectionPhase, TabInfo};
use crate::style;
use dbunk_lib::backend::DevelopmentEnvironment;
use editor::Editor;
use gpui::{
    AnimationExt, AnyElement, KeyDownEvent, MouseButton, Pixels, Point, SpringAnimation,
    SpringConfig, WindowControlArea, anchored, svg,
};

/// Width of the tab bar's left inset when the sidebar is hidden: room for
/// the traffic lights plus the show-sidebar button.
const REVEAL: f32 = style::TRAFFIC_LIGHTS + 28.;
/// Engine badge label size (DESIGN §2: 14 px badge, 8 px mono label).
const BADGE_FONT: f32 = 8.;

#[derive(Clone, PartialEq)]
pub(super) enum ShellMenu {
    Projects,
    Tools,
    Connection(String),
}

/// One row of an open shell menu; rendering and keys share the list.
struct MenuEntry {
    id: SharedString,
    label: String,
    operation: Operation,
}

/// What a key does in an open menu.
#[derive(Debug, PartialEq)]
enum MenuKey {
    Close,
    Move(usize),
    Activate,
    Ignore,
}

/// Menu keys: Escape closes, Up/Down (and Tab/Shift-Tab, so focus stays in
/// the menu) move with wrap, Home/End jump, Return/Space activate.
fn menu_key(key: &str, shift: bool, selected: usize, len: usize) -> MenuKey {
    if key == "escape" {
        return MenuKey::Close;
    }
    if len == 0 {
        return MenuKey::Ignore;
    }
    let last = len - 1;
    let selected = selected.min(last);
    let down = MenuKey::Move(if selected >= last { 0 } else { selected + 1 });
    let up = MenuKey::Move(if selected == 0 { last } else { selected - 1 });
    match key {
        "enter" | "space" => MenuKey::Activate,
        "down" => down,
        "up" => up,
        "tab" if shift => up,
        "tab" => down,
        "home" | "pageup" => MenuKey::Move(0),
        "end" | "pagedown" => MenuKey::Move(last),
        _ => MenuKey::Ignore,
    }
}

/// Tools menu entries. `engine` is `Some(can_clear)` while an engine
/// surface is selected: its PostgreSQL-only tools are left out, the
/// app-wide ones stay. `general` is the general PostgreSQL profile, the
/// only one with bastion and managed servers.
fn tool_entries(engine: Option<bool>, general: bool) -> Vec<(&'static str, Operation)> {
    let mut entries = match engine {
        None => vec![
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
        ],
        Some(can_clear) => {
            let mut entries = vec![
                ("Connect", Operation::Connect),
                ("Disconnect", Operation::Disconnect),
            ];
            if can_clear {
                entries.push(("Clear results", Operation::Clear));
            }
            entries
        }
    };
    if general {
        entries.push(("Bastion servers", Operation::Bastions));
        entries.push(("Managed servers", Operation::ManagedServers));
    }
    entries.push(("Credentials", Operation::Credentials));
    entries
}

/// A connection's stored colour as `0xRRGGBB`. The form takes free text;
/// `#rgb`, `#rrggbb` (with or without `#`) and a few basic names render,
/// anything else shows no dot.
pub(super) fn connection_color(stored: &str) -> Option<u32> {
    let value = stored.trim().to_ascii_lowercase();
    let named = match value.as_str() {
        "" => return None,
        "red" => Some(0xf85149),
        "orange" => Some(0xdb6d28),
        "yellow" => Some(0xd29922),
        "green" => Some(0x3fb950),
        "teal" | "cyan" => Some(0x39c5cf),
        "blue" => Some(0x6aa6ff),
        "purple" | "violet" => Some(0xa371f7),
        "pink" => Some(0xdb61a2),
        "gray" | "grey" => Some(0x8a929c),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    let hex = value.strip_prefix('#').unwrap_or(&value);
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => u32::from_str_radix(hex, 16).ok(),
        3 => {
            let short = u32::from_str_radix(hex, 16).ok()?;
            let (r, g, b) = ((short >> 8) & 0xf, (short >> 4) & 0xf, short & 0xf);
            Some(((r * 0x11) << 16) | ((g * 0x11) << 8) | (b * 0x11))
        }
        _ => None,
    }
}

/// Seconds since the Unix epoch for a stored timestamp: RFC 3339
/// (`2026-08-24T10:00:00Z`, `…T10:00:00.123+02:00`) or SQLite's
/// `2026-08-24 10:00:00` (UTC when no zone is given). `None` otherwise.
fn timestamp_secs(stored: &str) -> Option<i64> {
    let text = stored.trim();
    let bytes = text.as_bytes();
    if bytes.len() < 19 || !text.is_char_boundary(19) {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = text.get(range)?;
        if !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse::<i64>().ok()
    };
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        zone => {
            let sign = match zone.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let zone = &zone[1..];
            let (hours, minutes) = match zone.len() {
                5 if zone.as_bytes()[2] == b':' => (zone.get(0..2)?, zone.get(3..5)?),
                4 => (zone.get(0..2)?, zone.get(2..4)?),
                _ => return None,
            };
            if !hours
                .bytes()
                .chain(minutes.bytes())
                .all(|b| b.is_ascii_digit())
            {
                return None;
            }
            let hours: i64 = hours.parse().ok()?;
            let minutes: i64 = minutes.parse().ok()?;
            sign * (hours * 3600 + minutes * 60)
        }
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let year_of_era = shifted - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// Sidebar tooltip for a connection's last activity, relative to `now`
/// (Unix seconds) for the last week, then the stored date. Text that does
/// not parse is shown as stored rather than hidden.
fn last_activity_label(stored: &str, now: i64) -> String {
    let Some(at) = timestamp_secs(stored) else {
        return format!("Last activity {}", stored.trim());
    };
    let age = now - at;
    // Older than a week, or a clock that moved backwards: the stored date.
    let when = if !(0..604_800).contains(&age) {
        let stored = stored.trim();
        stored.get(..10).unwrap_or(stored).to_owned()
    } else if age < 60 {
        "just now".to_owned()
    } else if age < 3600 {
        format!("{} min ago", age / 60)
    } else if age < 86_400 {
        format!("{} h ago", age / 3600)
    } else if age < 2 * 86_400 {
        "yesterday".to_owned()
    } else {
        format!("{} days ago", age / 86_400)
    };
    format!("Last activity {when}")
}

/// The error strip's Retry button. After a latched cleanup failure with the
/// drafts already durable, Retry quits (see `Workspace::finish_close`), so it
/// says so.
fn retry_label(cleanup_failed: bool, drafts_durable: bool) -> &'static str {
    if cleanup_failed && drafts_durable {
        "Quit anyway"
    } else {
        "Retry"
    }
}

/// The status bar's host: `host:port`, or `None` for file engines.
fn status_host(host: &str, port: Option<u16>) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    Some(match port {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
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
    /// Keyboard focus while a menu is open; Escape returns it to `menu_return`.
    menu_focus: FocusHandle,
    menu_return: Option<FocusHandle>,
    /// Highlighted entry; Return activates it.
    menu_selected: usize,
    /// Window position of the click that opened a connection menu.
    menu_anchor: Option<Point<Pixels>>,
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
            menu_focus: cx.focus_handle(),
            menu_return: None,
            menu_selected: 0,
            menu_anchor: None,
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
        || super::engines::surface_connection(connection)
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
        if let Some(phase) = self.engine_phase(id, cx) {
            return phase;
        }
        connection_phase(
            connected.contains(id),
            self.documents
                .iter()
                .filter(|document| document.metadata.connection_id.as_deref() == Some(id))
                .map(|document| document.view.read(cx).connection_phase(cx)),
        )
    }
    fn current_connection(&self) -> Option<&DevelopmentConnection> {
        // A selected engine connection owns the whole workspace view.
        if let Some(connection) = self.active_engine_connection() {
            return Some(connection);
        }
        let id = self
            .active_index()
            .and_then(|index| self.documents[index].metadata.connection_id.as_ref())
            .or(self.selected_connection.as_ref())?;
        self.connections
            .iter()
            .find(|connection| &connection.id == id)
    }

    /// The project a saved connection belongs to.
    pub(super) fn connection_project(&self, id: &str) -> Option<String> {
        self.connections
            .iter()
            .find(|connection| connection.id == id)
            .map(|connection| project_of(connection).to_owned())
    }

    /// Selecting a connection shows its project in the switcher and list.
    pub(super) fn follow_connection_project(&mut self, id: &str) {
        if let Some(project) = self.connection_project(id) {
            self.shell.project = Some(project);
        }
    }

    /// The general PostgreSQL profile, the only one with bastion and
    /// managed servers.
    pub(super) fn general_profile(&self) -> bool {
        self.host.backend.native_profile_kind()
            == Some(dbunk_lib::backend::NativeProfileKind::GeneralPostgres)
    }

    /// A blocked action says why in the error strip (DESIGN §6); a repeat
    /// shakes again.
    pub(super) fn refuse(&mut self, reason: impl Into<String>, cx: &mut Context<Self>) {
        self.message = Some(reason.into());
        self.message_seq = self.message_seq.wrapping_add(1);
        cx.notify();
    }

    /// Opens `menu` with keyboard focus on it, or closes it when it is the
    /// open one.
    pub(super) fn toggle_shell_menu(
        &mut self,
        menu: ShellMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shell.menu.as_ref() == Some(&menu) {
            self.close_shell_menu(window, cx);
            return;
        }
        if self.shell.menu.is_none() {
            self.shell.menu_return = window.focused(cx);
        }
        self.shell.menu_selected = match &menu {
            ShellMenu::Projects => self
                .shell
                .project
                .as_ref()
                .and_then(|current| {
                    projects(&self.connections)
                        .iter()
                        .position(|project| project == current)
                })
                .unwrap_or(0),
            _ => 0,
        };
        // A click that opened a connection menu records its position after
        // this runs; keyboard and accessibility opens fall back to a fixed spot.
        self.shell.menu_anchor = None;
        self.shell.menu = Some(menu);
        window.focus(&self.shell.menu_focus, cx);
        cx.notify();
    }

    /// Closes the open menu and returns focus to where it was.
    pub(super) fn close_shell_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shell.menu.take().is_none() {
            return;
        }
        self.shell.menu_anchor = None;
        match self.shell.menu_return.take() {
            Some(focus) => window.focus(&focus, cx),
            None => self.focus_active(window, cx),
        }
        cx.notify();
    }

    fn menu_entries(&self, menu: &ShellMenu) -> Vec<MenuEntry> {
        match menu {
            ShellMenu::Projects => projects(&self.connections)
                .into_iter()
                .enumerate()
                .map(|(index, project)| MenuEntry {
                    id: SharedString::from(format!("project-{index}")),
                    label: project.clone(),
                    operation: Operation::SelectProject(project),
                })
                .collect(),
            ShellMenu::Tools => {
                tool_entries(self.active_engine_can_clear(), self.general_profile())
                    .into_iter()
                    .enumerate()
                    .map(|(index, (label, operation))| MenuEntry {
                        id: SharedString::from(format!("tool-{index}")),
                        label: label.to_owned(),
                        operation,
                    })
                    .collect()
            }
            ShellMenu::Connection(id) => {
                let postgres = self
                    .connections
                    .iter()
                    .any(|c| &c.id == id && c.postgres.is_some() && c.unsupported_reason.is_none());
                let mut entries = vec![
                    ("conn-edit", "Edit…", Operation::EditConnection(id.clone())),
                    (
                        "conn-duplicate",
                        "Duplicate",
                        Operation::DuplicateConnection(id.clone()),
                    ),
                ];
                if postgres {
                    entries.push((
                        "conn-uri",
                        "Copy URI",
                        Operation::CopyConnectionUri(id.clone()),
                    ));
                }
                entries.push((
                    "conn-favorite",
                    "Toggle favorite",
                    Operation::Favorite(id.clone()),
                ));
                entries.push((
                    "conn-delete",
                    "Delete…",
                    Operation::DeleteConnection(id.clone()),
                ));
                entries
                    .into_iter()
                    .map(|(id, label, operation)| MenuEntry {
                        id: id.into(),
                        label: label.to_owned(),
                        operation,
                    })
                    .collect()
            }
        }
    }

    fn menu_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.shell.menu.clone() else {
            return;
        };
        let modifiers = &event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        let entries = self.menu_entries(&menu);
        // Entries can change under an open menu (a reload); stay in range.
        let selected = self
            .shell
            .menu_selected
            .min(entries.len().saturating_sub(1));
        match menu_key(
            event.keystroke.key.as_str(),
            modifiers.shift,
            selected,
            entries.len(),
        ) {
            MenuKey::Close => self.close_shell_menu(window, cx),
            MenuKey::Move(index) => {
                self.shell.menu_selected = index;
                cx.notify();
            }
            MenuKey::Activate => {
                if let Some(entry) = entries.into_iter().nth(selected) {
                    self.activate(entry.operation, window, cx);
                }
            }
            MenuKey::Ignore => return,
        }
        cx.stop_propagation();
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
                            .bg(style::with_alpha(self.env_color(), 0xff)),
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
            .aria_keyshortcuts("Meta+0")
            .tooltip(crate::ui::tooltip("All environments  ⌘0"))
            .tooltip_show_delay(crate::ui::tooltip_delay())
            .child("All")
            .into_any_element(),
        ];
        // ⌘1–⌘4 follow `style::ENVIRONMENTS` order.
        for (index, environment) in style::ENVIRONMENTS.into_iter().enumerate() {
            let selected = self.shell.env_filter == Some(environment);
            let label = format!("{} only", style::env_label(environment));
            chips.push(
                self.shell_button(
                    SharedString::from(format!("env-{}", style::env_label(environment))),
                    label.clone(),
                    Operation::EnvFilter(Some(environment)),
                    cx,
                )
                .aria_selected(selected)
                .aria_keyshortcuts(format!("Meta+{}", index + 1))
                .tooltip(crate::ui::tooltip(format!("{label}  ⌘{}", index + 1)))
                .tooltip_show_delay(crate::ui::tooltip_delay())
                .when(selected, |chip| {
                    chip.bg(style::raised()).text_color(style::text())
                })
                .px(px(4.))
                .child(
                    div()
                        .size(px(6.))
                        .rounded_full()
                        .bg(style::with_alpha(style::env(Some(environment)), 0xff)),
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
                .aria_expanded(self.shell.menu == Some(ShellMenu::Projects))
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
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
            });
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
                            .bg(style::with_alpha(style::env(Some(environment)), 0xff)),
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
                let activity = connection
                    .last_activity_at
                    .as_deref()
                    .filter(|stored| !stored.trim().is_empty())
                    .map(|stored| last_activity_label(stored, now));
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
                    .when_some(activity, |row, activity| {
                        row.tooltip(crate::ui::tooltip(activity))
                            .tooltip_show_delay(crate::ui::tooltip_delay())
                    })
                    .rounded_none()
                    .relative()
                    .pl(px(16.))
                    .pr(px(8.))
                    .gap(px(6.))
                    .text_color(style::text())
                    .when(selected, |row| row.bg(style::select()))
                    // The stored connection colour sits in the left gutter,
                    // so rows with and without one stay aligned.
                    .when_some(
                        connection_color(&connection.organization.color),
                        |row, color| {
                            row.child(
                                div()
                                    .absolute()
                                    .left(px(6.))
                                    .top(px((style::ROW - 6.) / 2.))
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(style::with_alpha(color, 0xff)),
                            )
                        },
                    )
                    .child(
                        div()
                            .size(px(14.))
                            .flex_none()
                            .rounded(px(3.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .font_family(style::MONO)
                            .text_size(px(BADGE_FONT))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(style::bg())
                            .bg(style::with_alpha(badge_color, 0xff))
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
                    let menu = ShellMenu::Connection(id.clone());
                    let open = self.shell.menu.as_ref() == Some(&menu);
                    row = row.child(
                        self.row_button(
                            SharedString::from(format!("connection-menu-{id}")),
                            format!("Actions for {}", connection.name),
                            "icons/ellipsis.svg",
                            style::dim(),
                            Operation::ShellMenu(menu),
                            cx,
                        )
                        .aria_expanded(open)
                        // Runs after the open above: the menu opens at the
                        // click (a keyboard click reports the button's corner).
                        .on_click(cx.listener(
                            |this, event: &gpui::ClickEvent, _, cx| {
                                if matches!(this.shell.menu, Some(ShellMenu::Connection(_))) {
                                    this.shell.menu_anchor = Some(event.position());
                                    cx.notify();
                                }
                            },
                        )),
                    );
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
                            .child(
                                self.icon_button(
                                    "new-connection",
                                    "New connection  ⌘N",
                                    "icons/plus.svg",
                                    Operation::NewConnection,
                                    cx,
                                )
                                .aria_keyshortcuts("Meta+N"),
                            ),
                    )
                    .child(self.connection_list(cx)),
            )
            .child(div().flex_1().min_h_0().flex().flex_col().map(
                |tree| match self.engine_tree() {
                    Some(engine) => tree.child(engine),
                    None => tree.child(self.navigator.clone()),
                },
            ))
    }

    fn tab_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let env = self.env_color();
        // A selected engine connection shows its own tabs.
        let items = self.engine_tabs(cx).unwrap_or_else(|| {
            self.documents
                .iter()
                .map(|document| TabInfo {
                    id: document.metadata.id.clone(),
                    title: document.metadata.name.clone(),
                    icon: if document.metadata.tool.is_some() {
                        "icons/list_tree.svg"
                    } else if document.metadata.table.is_some() {
                        "icons/table.svg"
                    } else {
                        "icons/terminal.svg"
                    },
                    status: document.view.read(cx).document_status(cx).to_owned(),
                    active: Some(&document.metadata.id) == self.active.as_ref(),
                    pinned: document.metadata.pinned,
                    closable: true,
                })
                .collect()
        });
        let tabs = items.into_iter().map(|tab| {
            let TabInfo {
                id,
                title: name,
                icon: kind,
                status,
                active,
                pinned,
                closable,
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
                        .bg(style::with_alpha(env, 0xff)),
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
            .when(closable, |tab| {
                tab.child({
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
                    .children(tabs),
            )
            .child(self.drag_region("tab-bar-drag"))
            .child(
                div()
                    .id("tab-actions")
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .px(px(6.))
                    .border_l_1()
                    .border_color(style::line_soft())
                    .child(
                        self.icon_button(
                            "new-query",
                            "New query  ⌘T",
                            "icons/plus.svg",
                            Operation::New,
                            cx,
                        )
                        .aria_keyshortcuts("Meta+T"),
                    )
                    // An engine surface lists only the tools that apply to it
                    // plus the app-wide ones (see `tool_entries`).
                    .child(
                        self.shell_button(
                            "tools-menu",
                            "Tools",
                            Operation::ShellMenu(ShellMenu::Tools),
                            cx,
                        )
                        .aria_expanded(self.shell.menu == Some(ShellMenu::Tools))
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
                .bg(style::with_alpha(env, 0xff))
                .cursor_pointer()
                .aria_keyshortcuts("Meta+J")
                .tooltip(crate::ui::tooltip("Show status bar  ⌘J"))
                .tooltip_show_delay(crate::ui::tooltip_delay())
                .on_a11y_action(gpui::accesskit::Action::Click, {
                    let weak = cx.weak_entity();
                    move |_, window, cx| {
                        weak.update(cx, |this, cx| {
                            this.activate(Operation::ToggleStatusBar, window, cx)
                        })
                        .ok();
                    }
                })
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
        let latency_value = connection
            .and_then(|c| self.shell.last_latency.get(&c.id).copied())
            .map(|ms| format!("{ms} ms"))
            .unwrap_or_else(|| "—".into());
        let latency = format!("last query {latency_value}");
        let host = connection
            .and_then(|c| c.endpoint())
            .and_then(|endpoint| status_host(&endpoint.host, endpoint.port));
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
            "{state}{} {summary}, {latency}{}, {save}",
            failure
                .as_ref()
                .map(|error| format!(": {error}"))
                .unwrap_or_default(),
            host.as_ref()
                .map(|host| format!(", host {host}"))
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
                        .bg(style::with_alpha(env, 0xff))
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
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child("last query")
                    .child(div().font_family(style::MONO).child(latency_value)),
            )
            .when_some(host, |bar, host| {
                bar.child(
                    div()
                        .font_family(style::MONO)
                        .text_color(style::faint())
                        .child(host),
                )
            })
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
                    "Collapse status bar  ⌘J",
                    "icons/chevron_down.svg",
                    Operation::ToggleStatusBar,
                    cx,
                )
                .aria_keyshortcuts("Meta+J")
                .size(px(16.)),
            )
            .into_any_element()
    }

    fn menu_item(&self, entry: MenuEntry, highlighted: bool, cx: &Context<Self>) -> AnyElement {
        let MenuEntry {
            id,
            label,
            operation,
        } = entry;
        self.shell_button(id, label.clone(), operation, cx)
            .role(Role::MenuItem)
            // Focus stays on the menu; the highlighted row is what assistive
            // technology reads as focused.
            .when(highlighted, |item| {
                item.bg(style::hover()).aria_active_descendant()
            })
            .rounded_none()
            .px(px(10.))
            .text_color(style::text())
            .child(label)
            .into_any_element()
    }
    fn menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.shell.menu.clone()?;
        let entries = self.menu_entries(&menu);
        let highlighted = self
            .shell
            .menu_selected
            .min(entries.len().saturating_sub(1));
        let items: Vec<AnyElement> = entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| self.menu_item(entry, index == highlighted, cx))
            .collect();
        let panel = div()
            .id("shell-menu")
            .role(Role::Menu)
            .aria_label(match &menu {
                ShellMenu::Projects => "Projects",
                ShellMenu::Tools => "Tools",
                ShellMenu::Connection(_) => "Connection actions",
            })
            .key_context("ShellMenu")
            .track_focus(&self.shell.menu_focus)
            .capture_key_down(cx.listener(Self::menu_key_down))
            .min_w(px(170.))
            .py(px(4.))
            .flex()
            .flex_col()
            .bg(style::panel())
            .border_1()
            .border_color(style::line())
            .rounded(px(8.))
            .shadow_lg()
            .occlude()
            .children(items);
        // `appear` makes its element relative, so placement lives on a wrapper.
        let panel = crate::ui::appear("shell-menu-panel", panel);
        let placed = match (&menu, self.shell.menu_anchor) {
            (ShellMenu::Connection(_), Some(position)) => anchored()
                .position(position)
                .snap_to_window_with_margin(px(8.))
                .child(panel)
                .into_any_element(),
            (ShellMenu::Tools, _) => div()
                .absolute()
                .top(px(style::BAR))
                .right(px(8.))
                .child(panel)
                .into_any_element(),
            (ShellMenu::Projects, _) => div()
                .absolute()
                .top(px(style::BAR + 24.))
                .left(px(8.))
                .child(panel)
                .into_any_element(),
            // Opened without a click position (accessibility action).
            (ShellMenu::Connection(_), None) => div()
                .absolute()
                .top(px(style::BAR + 60.))
                .left(px(style::SIDEBAR - 150.))
                .child(panel)
                .into_any_element(),
        };
        // The backdrop takes every click outside the menu: it only closes
        // the menu, nothing underneath reacts.
        Some(
            div()
                .id("shell-menu-backdrop")
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_shell_menu(window, cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_shell_menu(window, cx);
                    }),
                )
                .child(placed)
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
                        .bg(style::bad_fill())
                        .text_color(style::bad_text())
                        .border_b_1()
                        .border_color(style::bad_line())
                        .child(icon("icons/warning.svg", style::bad_text()))
                        .child("Production · writes require review and confirmation"),
                )
            })
            .child(match (self.engine_body(), self.active_index()) {
                // A selected engine connection renders its own documents.
                (Some(body), _) => crate::ui::appear(
                    SharedString::from(format!(
                        "engine-{}",
                        self.selected_connection.as_deref().unwrap_or_default()
                    )),
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .bg(style::bg())
                        .child(body),
                )
                .into_any_element(),
                // Each document fades and settles in when it becomes active.
                (None, Some(index)) => crate::ui::appear(
                    SharedString::from(format!("document-{}", self.documents[index].metadata.id)),
                    div()
                        .flex_1()
                        .min_h_0()
                        .bg(style::bg())
                        .child(self.documents[index].view.clone()),
                )
                .into_any_element(),
                (None, None) => div()
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
            })
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
                                let retry = retry_label(self.cleanup_failed, self.drafts_durable);
                                strip
                                    .child(
                                        self.shell_button(
                                            "retry-save",
                                            retry,
                                            Operation::Retry,
                                            cx,
                                        )
                                        .child(retry),
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
                    .border_color(style::with_alpha(env, 0xff))
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
            last_activity_at: None,
        }
    }

    #[test]
    fn readable_records_of_every_engine_are_connectable() {
        // PostgreSQL connects through its endpoint, not engine settings.
        let mut postgres = connection("pg", "", DevelopmentEnvironment::Development);
        assert!(!connectable(&postgres), "no endpoint");
        postgres.postgres = Some(dbunk_lib::backend::DevelopmentPostgresConnection {
            name: "pg".into(),
            host: "127.0.0.1".into(),
            port: 5432,
            database: "postgres".into(),
            user: "postgres".into(),
            environment: DevelopmentEnvironment::Development,
            safe_mode: dbunk_lib::backend::DevelopmentSafeMode::Inherit,
            read_only: false,
            tls: Default::default(),
            driver_options: Default::default(),
            ssh_tunnel: None,
        });
        assert!(connectable(&postgres));
        postgres.unsupported_reason = Some("unsupported".into());
        assert!(!connectable(&postgres));
        for engine in super::super::engines::tests::engine_records() {
            assert!(connectable(&engine), "{}", engine.engine);
            let mut unreadable = engine.clone();
            unreadable.settings = None;
            assert!(!connectable(&unreadable), "{}", engine.engine);
            let mut unsupported = engine.clone();
            unsupported.unsupported_reason = Some("unsupported".into());
            assert!(!connectable(&unsupported), "{}", engine.engine);
        }
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

    #[test]
    fn menu_keys_wrap_jump_activate_and_close() {
        assert_eq!(menu_key("down", false, 0, 3), MenuKey::Move(1));
        assert_eq!(menu_key("down", false, 2, 3), MenuKey::Move(0), "wraps");
        assert_eq!(menu_key("up", false, 0, 3), MenuKey::Move(2), "wraps");
        // Tab keeps focus in the menu by moving like the arrows.
        assert_eq!(menu_key("tab", false, 1, 3), MenuKey::Move(2));
        assert_eq!(menu_key("tab", true, 1, 3), MenuKey::Move(0));
        assert_eq!(menu_key("home", false, 2, 3), MenuKey::Move(0));
        assert_eq!(menu_key("end", false, 0, 3), MenuKey::Move(2));
        assert_eq!(menu_key("enter", false, 1, 3), MenuKey::Activate);
        assert_eq!(menu_key("space", false, 1, 3), MenuKey::Activate);
        assert_eq!(menu_key("escape", false, 1, 3), MenuKey::Close);
        assert_eq!(menu_key("a", false, 1, 3), MenuKey::Ignore);
        // A stale selection past a shrunken list stays in range.
        assert_eq!(menu_key("down", false, 9, 3), MenuKey::Move(0));
        assert_eq!(menu_key("up", false, 9, 3), MenuKey::Move(1));
        // An empty menu still closes.
        assert_eq!(menu_key("escape", false, 0, 0), MenuKey::Close);
        assert_eq!(menu_key("down", false, 0, 0), MenuKey::Ignore);
    }

    #[test]
    fn engine_tools_keep_app_wide_entries_and_drop_postgres_ones() {
        let labels = |engine, general| -> Vec<&'static str> {
            tool_entries(engine, general)
                .into_iter()
                .map(|(label, _)| label)
                .collect()
        };
        let postgres = labels(None, true);
        assert!(postgres.contains(&"Query history"));
        assert!(postgres.contains(&"Bastion servers"));
        assert_eq!(postgres.last(), Some(&"Credentials"));
        assert_eq!(
            labels(Some(true), true),
            [
                "Connect",
                "Disconnect",
                "Clear results",
                "Bastion servers",
                "Managed servers",
                "Credentials"
            ]
        );
        // No clear for the engine: no dead entry.
        assert!(!labels(Some(false), true).contains(&"Clear results"));
        // Bastion and managed servers belong to the general profile only.
        assert_eq!(
            labels(Some(false), false),
            ["Connect", "Disconnect", "Credentials"]
        );
        assert!(!labels(None, false).contains(&"Managed servers"));
    }

    #[test]
    fn retry_reads_quit_anyway_only_when_a_latched_cleanup_left_drafts_safe() {
        assert_eq!(retry_label(true, true), "Quit anyway");
        // Drafts still at risk: Retry runs cleanup again.
        assert_eq!(retry_label(true, false), "Retry");
        // A failed save or load keeps its Retry, durable or not.
        assert_eq!(retry_label(false, true), "Retry");
        assert_eq!(retry_label(false, false), "Retry");
    }

    #[test]
    fn stored_activity_timestamps_parse_rfc3339_and_sqlite_text() {
        // 2026-08-24T00:00:00Z
        let midnight = 1_787_529_600;
        assert_eq!(timestamp_secs("2026-08-24T00:00:00Z"), Some(midnight));
        assert_eq!(timestamp_secs("2026-08-24T00:00:00.123Z"), Some(midnight));
        assert_eq!(timestamp_secs("2026-08-24 00:00:00"), Some(midnight));
        assert_eq!(timestamp_secs("2026-08-24T02:00:00+02:00"), Some(midnight));
        assert_eq!(timestamp_secs("2026-08-23T22:30:00-0130"), Some(midnight));
        assert_eq!(timestamp_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(timestamp_secs("2024-03-01T00:00:00Z"), Some(1_709_251_200));
        for bad in [
            "",
            "2026-08-24",
            "2026-13-24T00:00:00Z",
            "2026-08-24T00:00:00+2",
            "2026-08-24T00:00:00.Z",
            "2026-08-24T00:00:00 UTC",
            "yesterday afternoon",
            "2026-08-24T00:00:0é",
        ] {
            assert_eq!(timestamp_secs(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn last_activity_reads_relative_for_a_week_then_as_a_date() {
        let at = "2026-08-24T00:00:00Z";
        let midnight = 1_787_529_600;
        let label = |age: i64| last_activity_label(at, midnight + age);
        assert_eq!(label(5), "Last activity just now");
        assert_eq!(label(125), "Last activity 2 min ago");
        assert_eq!(label(3 * 3600 + 10), "Last activity 3 h ago");
        assert_eq!(label(86_400 + 5), "Last activity yesterday");
        assert_eq!(label(4 * 86_400), "Last activity 4 days ago");
        assert_eq!(label(30 * 86_400), "Last activity 2026-08-24");
        // Clock behind the stored time: the date, never "in the future".
        assert_eq!(label(-3600), "Last activity 2026-08-24");
        // Unparseable text is shown as stored rather than hidden.
        assert_eq!(
            last_activity_label(" last tuesday ", midnight),
            "Last activity last tuesday"
        );
    }

    #[test]
    fn stored_connection_colours_parse_hex_and_basic_names() {
        assert_eq!(connection_color("#3fb950"), Some(0x3fb950));
        assert_eq!(connection_color(" 3FB950 "), Some(0x3fb950));
        assert_eq!(connection_color("#f0a"), Some(0xff00aa));
        assert_eq!(connection_color("Blue"), Some(0x6aa6ff));
        assert_eq!(connection_color("grey"), connection_color("gray"));
        for unknown in ["", "  ", "#12345", "#ggg", "chartreuse", "#1234567"] {
            assert_eq!(connection_color(unknown), None, "{unknown:?}");
        }
    }

    #[test]
    fn status_host_shows_network_endpoints_only() {
        assert_eq!(
            status_host("db.internal", Some(5432)).as_deref(),
            Some("db.internal:5432")
        );
        assert_eq!(status_host("db", None).as_deref(), Some("db"));
        // SQLite has a path, not a host.
        assert_eq!(status_host("", None), None);
    }
}
