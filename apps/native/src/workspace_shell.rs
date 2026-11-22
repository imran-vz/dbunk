//! Plan 031 shell: sidebar (traffic-light row, project switcher, connection
//! search, environment filter and list, object tree), tab bar, production
//! strip, collapsible status bar and the environment frame. The environment
//! wash tints chrome only (sidebar, tab bar, status bar); documents stay
//! neutral.
use super::*;
use crate::document_view::{ConnectionPhase, TabInfo};
use crate::style;
use crate::ui::{self, popover};
use dbunk_lib::backend::DevelopmentEnvironment;
use editor::Editor;
use gpui::{
    Anchor, AnimationExt, AnyElement, App, Bounds, Div, KeyDownEvent, MouseButton, Pixels, Point,
    Rgba, ScrollHandle, SpringAnimation, SpringConfig, Stateful, WindowControlArea, anchored,
    point, svg,
};

/// Width of the tab bar's left inset when the sidebar is hidden: room for
/// the traffic lights plus the show-sidebar button.
const REVEAL: f32 = style::TRAFFIC_LIGHTS + style::CONTROL + style::S2;
/// Left edge of sidebar text: the `list_row` margin plus its padding, so
/// headers, group labels and row labels share one column.
const TEXT_INSET: f32 = style::S2 + style::S3;
/// Tab width bounds (D9): labels ellipsize inside them.
const TAB_MIN: f32 = 88.;
const TAB_MAX: f32 = 180.;
/// A tab's close button, reserved on every tab so widths never jump.
const TAB_CLOSE: f32 = 16.;
/// The trailing slot of a connection row: two 16 pt row buttons.
const ROW_ACTIONS: f32 = 16. * 2. + style::S1 + style::S1;
/// Height of the production strip under the tab bar.
const STRIP: f32 = 22.;
/// Narrowest shell menu.
const MENU_MIN: f32 = 200.;
/// Corner radius of the macOS window, so the environment frame follows it.
const WINDOW_RADIUS: f32 = 10.;
/// Gap between a menu trigger and its panel.
const MENU_GAP: f32 = 2.;

#[derive(Clone, PartialEq)]
pub(super) enum ShellMenu {
    Projects,
    Tools,
    Connection(String),
    /// The sidebar's environment filter.
    Environments,
    /// Every open tab, from the tab bar's overflow button.
    Tabs,
}

/// How a menu row is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
enum EntryStyle {
    Plain,
    /// A check column; `true` shows the mark.
    Check(bool),
    /// Destructive: `bad_text`, in its own group at the bottom.
    Danger,
}

/// One row of an open shell menu; rendering and keys share the list.
struct MenuEntry {
    id: SharedString,
    label: String,
    /// The AX name when it differs from the visible label, so scripts and
    /// assistive users keep the names they know.
    ax_label: Option<String>,
    icon: Option<&'static str>,
    /// Visible shortcut hint and its AX form.
    hint: Option<(&'static str, &'static str)>,
    style: EntryStyle,
    /// A divider separates rows whose groups differ.
    group: u8,
    operation: Operation,
}

impl MenuEntry {
    fn new(id: impl Into<SharedString>, label: impl Into<String>, operation: Operation) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            ax_label: None,
            icon: None,
            hint: None,
            style: EntryStyle::Plain,
            group: 0,
            operation,
        }
    }
    fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }
    fn group(mut self, group: u8) -> Self {
        self.group = group;
        self
    }
    fn check(mut self, checked: bool) -> Self {
        self.style = EntryStyle::Check(checked);
        self
    }
    fn danger(mut self) -> Self {
        self.style = EntryStyle::Danger;
        self
    }
    fn ax(mut self, label: impl Into<String>) -> Self {
        self.ax_label = Some(label.into());
        self
    }
    fn hint(mut self, hint: Option<(&'static str, &'static str)>) -> Self {
        self.hint = hint;
        self
    }
}

/// True where a divider goes before each entry: wherever the group changes.
fn group_breaks(groups: &[u8]) -> Vec<bool> {
    groups
        .iter()
        .enumerate()
        .map(|(index, group)| index > 0 && groups[index - 1] != *group)
        .collect()
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

/// Tools menu groups, in order (D11).
const GROUP_QUERY: u8 = 0;
const GROUP_DATABASE: u8 = 1;
const GROUP_DATA: u8 = 2;
const GROUP_TAB: u8 = 3;
const GROUP_SETTINGS: u8 = 4;

/// One Tools menu row: its group, label (also the AX name), icon, optional
/// shortcut hint (visible, AX) and operation.
struct ToolEntry {
    group: u8,
    label: &'static str,
    icon: &'static str,
    hint: Option<(&'static str, &'static str)>,
    operation: Operation,
}

fn tool(
    group: u8,
    label: &'static str,
    icon: &'static str,
    hint: Option<(&'static str, &'static str)>,
    operation: Operation,
) -> ToolEntry {
    ToolEntry {
        group,
        label,
        icon,
        hint,
        operation,
    }
}

/// The icon of a workspace tool, shared by its tab and its Tools menu row.
fn tool_icon(kind: WorkspaceTool) -> &'static str {
    match kind {
        WorkspaceTool::History => "icons/clock.svg",
        WorkspaceTool::SavedQueries => "icons/book.svg",
        WorkspaceTool::Objects => "icons/database_zap.svg",
        WorkspaceTool::Administration => "icons/signal_high.svg",
        WorkspaceTool::SchemaMap => "icons/git_graph.svg",
        WorkspaceTool::SchemaCompare => "icons/diff.svg",
        WorkspaceTool::CsvTransfer => "icons/arrow_right_left.svg",
        WorkspaceTool::BackupRestore => "icons/archive.svg",
        WorkspaceTool::TableCopy => "icons/copy.svg",
        WorkspaceTool::TableSeed => "icons/sparkle.svg",
    }
}

/// A document tab's icon: its tool's, a table, or a query file (D9).
fn document_icon(document: &WorkspaceDocument) -> &'static str {
    match (document.tool, &document.table) {
        (Some(kind), _) => tool_icon(kind),
        (None, Some(_)) => "icons/table.svg",
        (None, None) => "icons/file_code.svg",
    }
}

/// The icon a tab shows: engine query tabs report the console glyph, which
/// reads as a terminal; every query tab shows the query file instead (D9).
fn tab_glyph(icon: &'static str) -> &'static str {
    if icon == "icons/terminal.svg" {
        "icons/file_code.svg"
    } else {
        icon
    }
}

/// Tools menu entries, grouped Query · Database · Data · Tab · Settings.
/// `engine` is `Some(can_clear)` while an engine surface is selected: its
/// PostgreSQL-only tools are left out, the app-wide ones stay. `general` is
/// the general PostgreSQL profile, the only one with bastion and managed
/// servers.
fn tool_entries(engine: Option<bool>, general: bool) -> Vec<ToolEntry> {
    use dbunk_lib::backend::WorkspaceTool as T;
    let library = Operation::Library;
    let mut entries = match engine {
        None => vec![
            tool(
                GROUP_QUERY,
                "Query history",
                tool_icon(T::History),
                None,
                library(T::History),
            ),
            tool(
                GROUP_QUERY,
                "Saved queries",
                tool_icon(T::SavedQueries),
                None,
                library(T::SavedQueries),
            ),
            tool(
                GROUP_QUERY,
                "Save query",
                "icons/bookmark.svg",
                None,
                Operation::SaveQuery,
            ),
            tool(
                GROUP_DATABASE,
                "Objects",
                tool_icon(T::Objects),
                None,
                library(T::Objects),
            ),
            tool(
                GROUP_DATABASE,
                "Administration",
                tool_icon(T::Administration),
                None,
                library(T::Administration),
            ),
            tool(
                GROUP_DATABASE,
                "Schema map",
                tool_icon(T::SchemaMap),
                None,
                library(T::SchemaMap),
            ),
            tool(
                GROUP_DATABASE,
                "Compare schemas",
                tool_icon(T::SchemaCompare),
                None,
                library(T::SchemaCompare),
            ),
            tool(
                GROUP_DATA,
                "CSV transfer",
                tool_icon(T::CsvTransfer),
                None,
                library(T::CsvTransfer),
            ),
            tool(
                GROUP_DATA,
                "Copy table",
                tool_icon(T::TableCopy),
                None,
                library(T::TableCopy),
            ),
            tool(
                GROUP_DATA,
                "Backup / Restore",
                tool_icon(T::BackupRestore),
                None,
                library(T::BackupRestore),
            ),
            tool(
                GROUP_DATA,
                "Open table by name",
                "icons/table.svg",
                Some(("⌘O", "Meta+O")),
                Operation::OpenTable,
            ),
            tool(
                GROUP_TAB,
                "Rename tab",
                "icons/pencil.svg",
                Some(("⇧⌘R", "Meta+Shift+R")),
                Operation::Rename,
            ),
            tool(
                GROUP_TAB,
                "Pin tab",
                "icons/pin.svg",
                Some(("⇧⌘P", "Meta+Shift+P")),
                Operation::Pin,
            ),
            tool(
                GROUP_TAB,
                "Connect tab",
                "icons/power.svg",
                None,
                Operation::Connect,
            ),
            tool(
                GROUP_TAB,
                "Disconnect tab",
                "icons/disconnected.svg",
                None,
                Operation::Disconnect,
            ),
            tool(
                GROUP_TAB,
                "Clear results",
                "icons/eraser.svg",
                None,
                Operation::Clear,
            ),
        ],
        Some(can_clear) => {
            let mut entries = vec![
                tool(
                    GROUP_TAB,
                    "Connect",
                    "icons/power.svg",
                    None,
                    Operation::Connect,
                ),
                tool(
                    GROUP_TAB,
                    "Disconnect",
                    "icons/disconnected.svg",
                    None,
                    Operation::Disconnect,
                ),
            ];
            if can_clear {
                entries.push(tool(
                    GROUP_TAB,
                    "Clear results",
                    "icons/eraser.svg",
                    None,
                    Operation::Clear,
                ));
            }
            entries
        }
    };
    if general {
        entries.push(tool(
            GROUP_SETTINGS,
            "Bastion servers",
            "icons/link.svg",
            None,
            Operation::Bastions,
        ));
        entries.push(tool(
            GROUP_SETTINGS,
            "Managed servers",
            "icons/server.svg",
            None,
            Operation::ManagedServers,
        ));
    }
    entries.push(tool(
        GROUP_SETTINGS,
        "Credentials",
        "icons/lock.svg",
        Some(("⌘,", "Meta+Comma")),
        Operation::Credentials,
    ));
    entries
}

/// Blends the chrome `base` with the environment wash (D8). Opaque, so text
/// contrast on chrome is measured against what is painted.
fn washed(base: Rgba, environment: Option<DevelopmentEnvironment>) -> Rgba {
    let wash = style::env_wash(environment);
    let mix = |under: f32, over: f32| under * (1. - wash.a) + over * wash.a;
    Rgba {
        r: mix(base.r, wash.r),
        g: mix(base.g, wash.g),
        b: mix(base.b, wash.b),
        a: base.a,
    }
}

/// The status bar's connection detail after the name: the database (or a
/// file's name) and the host, each only when present.
fn status_detail(database: Option<&str>, host: Option<&str>) -> Vec<String> {
    database
        .filter(|database| !database.is_empty())
        .map(|database| row_detail(database).to_owned())
        .into_iter()
        .chain(host.map(str::to_owned))
        .collect()
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

/// Sidebar row detail: the database name, or only the file name for a
/// file-backed connection, whose full path would crowd out the row's name.
fn row_detail(database: &str) -> &str {
    database
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(database)
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
    /// Set once a key moves the highlight; until then no row is painted as
    /// highlighted (a freshly opened menu shows no pre-selection).
    menu_keyed: bool,
    /// Trigger bounds of the menus that open under a button.
    projects_anchor: popover::AnchorSlot,
    tools_anchor: popover::AnchorSlot,
    filter_anchor: popover::AnchorSlot,
    tabs_anchor: popover::AnchorSlot,
    /// Scroll state of the tab strip; the active tab is kept in view.
    tab_scroll: ScrollHandle,
    /// Active tab, window width and sidebar state the strip last scrolled
    /// for; a change scrolls the active tab into view again.
    tab_scroll_key: Option<(String, i32, bool)>,
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
            menu_keyed: false,
            projects_anchor: popover::anchor_slot(),
            tools_anchor: popover::anchor_slot(),
            filter_anchor: popover::anchor_slot(),
            tabs_anchor: popover::anchor_slot(),
            tab_scroll: ScrollHandle::new(),
            tab_scroll_key: None,
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
        // Sentence-case full names (D8); tags are for the status bar.
        let label = if needle.is_empty() {
            style::env_name(connection.environment).to_owned()
        } else {
            format!(
                "{} · {}",
                project_of(connection),
                style::env_name(connection.environment)
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

fn icon(path: &'static str, color: Rgba) -> impl IntoElement {
    svg()
        .path(path)
        .size(px(style::ICON))
        .flex_none()
        .text_color(color)
}

/// Shortcut hints of the environment filter, in `style::ENVIRONMENTS` order.
const ENV_HINTS: [(&str, &str); 4] = [
    ("⌘1", "Meta+1"),
    ("⌘2", "Meta+2"),
    ("⌘3", "Meta+3"),
    ("⌘4", "Meta+4"),
];

/// The product mark: a monochrome monogram with the environment colour as a
/// small corner dot (F0-18). The frame stays the environment signature.
fn brand_mark(environment: Option<DevelopmentEnvironment>) -> Div {
    div()
        .relative()
        .flex_none()
        .size(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(style::R_SM))
        .bg(style::text())
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(style::panel())
        .child("d")
        .child(
            div()
                .absolute()
                .right(px(-style::S1))
                .bottom(px(-style::S1))
                .size(px(7.))
                .rounded_full()
                .border_1()
                .border_color(style::panel())
                .bg(style::with_alpha(style::env(environment), 0xff)),
        )
}

/// Places a menu panel under its trigger's recorded bounds, start- or
/// end-aligned. Before the trigger has painted once, `fallback` places it.
fn under(
    anchor: Option<Bounds<Pixels>>,
    end: bool,
    panel: impl IntoElement,
    fallback: Div,
) -> AnyElement {
    match anchor {
        Some(bounds) => {
            let (corner, position) = if end {
                (Anchor::TopRight, bounds.bottom_right())
            } else {
                (Anchor::TopLeft, bounds.bottom_left())
            };
            anchored()
                .anchor(corner)
                .position(position + point(px(0.), px(MENU_GAP)))
                .snap_to_window_with_margin(px(style::S4))
                .child(panel)
                .into_any_element()
        }
        None => fallback.child(panel).into_any_element(),
    }
}

impl Workspace {
    /// Empty titlebar space: drag moves the window, double-click zooms.
    fn drag_region(&self, id: &'static str) -> Stateful<Div> {
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
    /// Runs `operation` on click and on the accessibility click action.
    fn activation(
        &self,
        element: Stateful<Div>,
        operation: Operation,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let click = operation.clone();
        let weak = cx.weak_entity();
        element
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(operation.clone(), window, cx))
                    .ok();
            })
            .on_click(
                cx.listener(move |this, _, window, cx| this.activate(click.clone(), window, cx)),
            )
    }
    fn shell_button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        operation: Operation,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let label = label.into();
        self.activation(
            div()
                .id(id.into())
                .role(Role::Button)
                .aria_label(label)
                .tab_index(0)
                .h(px(style::CONTROL))
                .px(px(style::S3))
                .flex()
                .items_center()
                .gap(px(style::S2))
                .rounded(px(style::R_SM))
                .whitespace_nowrap()
                .text_color(style::dim())
                .hover(|s| s.bg(style::hover()).text_color(style::text()))
                .focus(|s| s.text_color(style::text()).shadow(vec![ui::ring()]))
                .cursor_pointer()
                .active(|s| s.bg(style::pressed())),
            operation,
            cx,
        )
    }
    fn icon_button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        path: &'static str,
        operation: Operation,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let label = label.into();
        ui::press(self.shell_button(id, label.clone(), operation, cx))
            .w(px(style::CONTROL))
            .px_0()
            .justify_center()
            .tooltip(ui::tooltip(label))
            .tooltip_show_delay(ui::tooltip_delay())
            .child(icon(path, style::dim()))
    }
    /// A button inside a connection row or tab; it must not also select it.
    fn row_button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        path: &'static str,
        color: Rgba,
        operation: Operation,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let label = label.into();
        self.shell_button(id, label.clone(), operation, cx)
            .on_click(|_, _, cx| cx.stop_propagation())
            .tooltip(ui::tooltip(label))
            .tooltip_show_delay(ui::tooltip_delay())
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
        self.shell.menu_keyed = false;
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
            ShellMenu::Environments => self.shell.env_filter.map_or(0, |current| {
                style::ENVIRONMENTS
                    .iter()
                    .position(|environment| *environment == current)
                    .map_or(0, |index| index + 1)
            }),
            ShellMenu::Tabs => self
                .tab_items(cx)
                .iter()
                .position(|tab| tab.active)
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

    pub(super) fn env_color(&self) -> u32 {
        style::env(self.current_environment())
    }
    fn current_environment(&self) -> Option<DevelopmentEnvironment> {
        self.current_connection()
            .map(|connection| connection.environment)
    }

    /// The tab bar's tabs: a selected engine connection's own, otherwise the
    /// workspace documents.
    fn tab_items(&self, cx: &App) -> Vec<TabInfo> {
        self.engine_tabs(cx).unwrap_or_else(|| {
            self.documents
                .iter()
                .map(|document| TabInfo {
                    id: document.metadata.id.clone(),
                    title: document.metadata.name.clone(),
                    icon: document_icon(&document.metadata),
                    status: document.view.read(cx).document_status(cx).to_owned(),
                    active: Some(&document.metadata.id) == self.active.as_ref(),
                    pinned: document.metadata.pinned,
                    closable: true,
                })
                .collect()
        })
    }

    fn menu_entries(&self, menu: &ShellMenu, cx: &App) -> Vec<MenuEntry> {
        match menu {
            ShellMenu::Projects => projects(&self.connections)
                .into_iter()
                .enumerate()
                .map(|(index, project)| {
                    let current = self.shell.project.as_ref() == Some(&project);
                    MenuEntry::new(
                        format!("project-{index}"),
                        project.clone(),
                        Operation::SelectProject(project),
                    )
                    .check(current)
                })
                .collect(),
            ShellMenu::Tools => {
                tool_entries(self.active_engine_can_clear(), self.general_profile())
                    .into_iter()
                    .enumerate()
                    .map(|(index, entry)| {
                        MenuEntry::new(format!("tool-{index}"), entry.label, entry.operation)
                            .icon(entry.icon)
                            .group(entry.group)
                            .hint(entry.hint)
                    })
                    .collect()
            }
            ShellMenu::Connection(id) => {
                let connection = self.connections.iter().find(|c| &c.id == id);
                let postgres = connection
                    .is_some_and(|c| c.postgres.is_some() && c.unsupported_reason.is_none());
                let favorite = connection.is_some_and(|c| c.organization.is_favorite);
                let mut entries = vec![
                    MenuEntry::new("conn-edit", "Edit…", Operation::EditConnection(id.clone()))
                        .icon("icons/pencil.svg"),
                    MenuEntry::new(
                        "conn-duplicate",
                        "Duplicate",
                        Operation::DuplicateConnection(id.clone()),
                    )
                    .icon("icons/copy.svg"),
                ];
                if postgres {
                    entries.push(
                        MenuEntry::new(
                            "conn-uri",
                            "Copy URI",
                            Operation::CopyConnectionUri(id.clone()),
                        )
                        .icon("icons/link.svg"),
                    );
                }
                // Visible copy names the result; the AX name stays the same.
                entries.push(
                    MenuEntry::new(
                        "conn-favorite",
                        if favorite {
                            "Remove from favorites"
                        } else {
                            "Add to favorites"
                        },
                        Operation::Favorite(id.clone()),
                    )
                    .icon(if favorite {
                        "icons/star_filled.svg"
                    } else {
                        "icons/star.svg"
                    })
                    .ax("Toggle favorite"),
                );
                // Destructive, in its own group at the bottom (D11).
                entries.push(
                    MenuEntry::new(
                        "conn-delete",
                        "Delete…",
                        Operation::DeleteConnection(id.clone()),
                    )
                    .icon("icons/trash.svg")
                    .group(1)
                    .danger(),
                );
                entries
            }
            ShellMenu::Environments => {
                // Same ids and AX names as the former filter chips.
                let mut entries = vec![
                    MenuEntry::new("env-all", "All environments", Operation::EnvFilter(None))
                        .check(self.shell.env_filter.is_none())
                        .hint(Some(("⌘0", "Meta+0"))),
                ];
                for (index, environment) in style::ENVIRONMENTS.into_iter().enumerate() {
                    entries.push(
                        MenuEntry::new(
                            format!("env-{}", style::env_label(environment)),
                            style::env_name(environment),
                            Operation::EnvFilter(Some(environment)),
                        )
                        .check(self.shell.env_filter == Some(environment))
                        .ax(format!("{} only", style::env_label(environment)))
                        .hint(ENV_HINTS.get(index).copied())
                        .group(1),
                    );
                }
                entries
            }
            ShellMenu::Tabs => self
                .tab_items(cx)
                .into_iter()
                .map(|tab| {
                    MenuEntry::new(
                        format!("tab-menu-{}", tab.id),
                        tab.title,
                        Operation::SelectDocument(tab.id),
                    )
                    .check(tab.active)
                })
                .collect(),
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
        let entries = self.menu_entries(&menu, cx);
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
                self.shell.menu_keyed = true;
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

    /// Row 1: traffic lights, sidebar toggle, then the project switcher as a
    /// borderless title that ellipsizes (D10). The rest drags the window.
    fn title_row(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("sidebar-title")
            .h(px(style::BAR))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(style::S1))
            .pl(px(style::TRAFFIC_LIGHTS))
            .pr(px(style::S4))
            .child(self.icon_button(
                "collapse-sidebar",
                "Hide sidebar  ⌘\\",
                "icons/threads_sidebar_left_open.svg",
                Operation::ToggleSidebar,
                cx,
            ))
            .child(self.switcher(cx))
            .child(self.drag_region("title-drag"))
    }

    fn switcher(&self, cx: &Context<Self>) -> impl IntoElement {
        let project = self.shell.project.clone();
        let label = format!("Project {}", project.as_deref().unwrap_or("No connections"));
        let open = self.shell.menu == Some(ShellMenu::Projects);
        self.shell_button(
            "project-switcher",
            label,
            Operation::ShellMenu(ShellMenu::Projects),
            cx,
        )
        .aria_expanded(open)
        .aria_keyshortcuts("Meta+P")
        .relative()
        .min_w_0()
        .px(px(style::S2))
        .gap(px(style::S3))
        .text_color(style::text())
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .when(open, |button| button.bg(style::hover()))
        .child(brand_mark(self.current_environment()))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                // Before the first connection the title is the product.
                .child(project.unwrap_or_else(|| "dbunk".into())),
        )
        .child(icon("icons/chevron_down.svg", style::dim()))
        .child(popover::probe(self.shell.projects_anchor.clone()))
    }

    /// Row 2: connection search, New connection and the environment filter.
    fn search_row(&self, cx: &Context<Self>) -> impl IntoElement {
        let filter = self.shell.env_filter;
        let open = self.shell.menu == Some(ShellMenu::Environments);
        let filter_label = match filter {
            None => "Filter by environment".to_owned(),
            Some(environment) => format!(
                "Filter by environment: {} only",
                style::env_name(environment)
            ),
        };
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(style::S1))
            .mx(px(style::S2))
            .mb(px(style::S3))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(style::CONTROL))
                    .flex()
                    .items_center()
                    .gap(px(style::S3))
                    .px(px(style::S3))
                    .rounded(px(style::R_SM))
                    .bg(style::bg())
                    .border_1()
                    .border_color(style::line())
                    .child(icon("icons/magnifying_glass.svg", style::dim()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(style::CONTROL - style::S3))
                            .child(self.shell.search_field.clone()),
                    ),
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
            )
            // Environment choices wait for a first connection (A0-8).
            .when(!self.connections.is_empty(), |row| {
                row.child(
                    ui::press(self.shell_button(
                        "env-filter",
                        filter_label.clone(),
                        Operation::ShellMenu(ShellMenu::Environments),
                        cx,
                    ))
                    .aria_expanded(open)
                    .relative()
                    .w(px(style::CONTROL))
                    .px_0()
                    .justify_center()
                    .when(open || filter.is_some(), |button| button.bg(style::hover()))
                    .tooltip(ui::tooltip(filter_label))
                    .tooltip_show_delay(ui::tooltip_delay())
                    .child(icon(
                        "icons/filter.svg",
                        match filter {
                            Some(environment) => {
                                style::with_alpha(style::env(Some(environment)), 0xff)
                            }
                            None => style::dim(),
                        },
                    ))
                    .child(popover::probe(self.shell.filter_anchor.clone())),
                )
            })
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
        // With no connections at all the workspace's empty state speaks.
        if groups.is_empty() && !self.connections.is_empty() {
            rows.push(
                div()
                    .h(px(style::ROW))
                    .px(px(TEXT_INSET))
                    .flex()
                    .items_center()
                    .text_color(style::dim())
                    .child("No connections match")
                    .into_any_element(),
            );
        }
        for (label, environment, items) in groups {
            rows.push(
                div()
                    .flex_none()
                    .h(px(style::ROW))
                    .flex()
                    .items_center()
                    .gap(px(style::S3))
                    .px(px(TEXT_INSET))
                    .text_size(px(style::FONT_SMALL))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(style::dim())
                    .child(ui::env_dot(Some(environment)))
                    .child(label)
                    .into_any_element(),
            );
            for connection in items {
                rows.push(self.connection_row(connection, &connected, now, cx));
            }
        }
        div()
            .id("connection-list")
            .role(Role::ListBox)
            .aria_label("Connections")
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .pb(px(style::S3))
            .children(rows)
    }

    /// One connection: engine logo, name, `dim` database at a fixed inset,
    /// then a fixed trailing slot. The status dot sits in the slot at rest;
    /// connect/disconnect and `⋯` replace it on hover and selection, so
    /// nothing shifts (D10).
    fn connection_row(
        &self,
        connection: &DevelopmentConnection,
        connected: &std::collections::BTreeSet<String>,
        now: i64,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = connection.id.clone();
        let selected = self.selected_connection.as_ref() == Some(&id);
        let phase = self.phase_of(&id, connected, cx);
        let health = self.health.label(&id);
        let unhealthy = health
            .as_ref()
            .is_some_and(|label| !label.starts_with("Healthy"));
        let database = connection
            .endpoint()
            .map(|endpoint| row_detail(&endpoint.database).to_owned())
            .unwrap_or_default();
        let activity = connection
            .last_activity_at
            .as_deref()
            .filter(|stored| !stored.trim().is_empty())
            .map(|stored| last_activity_label(stored, now));
        let label = format!(
            "{}, {}, {}{}{}",
            connection.name,
            connection.engine,
            match &phase {
                ConnectionPhase::Idle => "not connected".to_owned(),
                ConnectionPhase::Connecting => "connecting".to_owned(),
                ConnectionPhase::Connected => "connected".to_owned(),
                ConnectionPhase::Failed(error) => format!("connection failed: {error}"),
            },
            health.map(|label| format!(", {label}")).unwrap_or_default(),
            connection
                .unsupported_reason
                .as_ref()
                .map(|reason| format!(", {reason}"))
                .unwrap_or_default()
        );
        let menu = ShellMenu::Connection(id.clone());
        let open = self.shell.menu.as_ref() == Some(&menu);
        let shown = selected || open;
        let mut actions = ui::row_actions(ROW_ACTIONS, shown);
        if connectable(connection) {
            actions = actions.child(match phase {
                ConnectionPhase::Connected | ConnectionPhase::Connecting => self.row_button(
                    SharedString::from(format!("connection-toggle-{id}")),
                    format!("Disconnect {}", connection.name),
                    "icons/power.svg",
                    style::dim(),
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
        actions = actions.child(
            self.row_button(
                SharedString::from(format!("connection-menu-{id}")),
                format!("Actions for {}", connection.name),
                "icons/ellipsis.svg",
                style::dim(),
                Operation::ShellMenu(menu),
                cx,
            )
            .aria_expanded(open)
            // Runs after the open above: the menu opens at the click (a
            // keyboard click reports the button's corner).
            .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                if matches!(this.shell.menu, Some(ShellMenu::Connection(_))) {
                    this.shell.menu_anchor = Some(event.position());
                    cx.notify();
                }
            })),
        );
        // At rest the slot shows the session state; an idle connection
        // shows nothing.
        let dot = div().size(px(6.)).rounded_full();
        let dot = match &phase {
            ConnectionPhase::Connected => Some(dot.bg(if unhealthy {
                style::warn()
            } else {
                style::ok()
            })),
            ConnectionPhase::Connecting => Some(dot.border_1().border_color(style::accent())),
            ConnectionPhase::Failed(_) => Some(dot.bg(style::bad())),
            ConnectionPhase::Idle => None,
        };
        let detail_color = match &phase {
            ConnectionPhase::Failed(_) => style::bad_text(),
            ConnectionPhase::Connecting => style::accent(),
            _ => style::dim(),
        };
        let detail = match &phase {
            ConnectionPhase::Connecting => "connecting…".to_owned(),
            ConnectionPhase::Failed(_) => "failed".to_owned(),
            _ => database,
        };
        self.activation(
            ui::list_row(
                SharedString::from(format!("connection-{id}")),
                selected,
                false,
            )
            .role(Role::ListBoxOption)
            .aria_label(label)
            .aria_selected(selected)
            .tab_index(0)
            .relative()
            .pl(px(style::S3 + style::S2))
            .pr(px(style::S1))
            .gap(px(style::S3)),
            Operation::SelectConnection(id.clone()),
            cx,
        )
        .when(open && !selected, |row| row.bg(style::hover()))
        .when_some(activity, |row, activity| {
            row.tooltip(ui::tooltip(activity))
                .tooltip_show_delay(ui::tooltip_delay())
        })
        // The stored connection colour sits in the left gutter, so rows with
        // and without one stay aligned.
        .when_some(
            connection_color(&connection.organization.color),
            |row, color| {
                row.child(
                    div()
                        .absolute()
                        .left(px(style::S1 + 1.))
                        .top(px((style::ROW - style::S2) / 2.))
                        .size(px(style::S2))
                        .rounded_full()
                        .bg(style::with_alpha(color, 0xff)),
                )
            },
        )
        .child(ui::engine_mark(&connection.engine))
        .when(connection.organization.is_favorite, |row| {
            row.child(
                svg()
                    .path("icons/star_filled.svg")
                    .size(px(10.))
                    .flex_none()
                    .text_color(style::warn()),
            )
        })
        .child(ui::row_label(connection.name.clone()))
        .child(
            div()
                // Bounded so a long detail never hides the name.
                .flex_shrink(1.)
                .min_w_0()
                .max_w(px(96.))
                .truncate()
                .text_size(px(style::FONT_SMALL))
                .text_color(detail_color)
                .child(detail),
        )
        .child(
            div()
                .relative()
                .flex_none()
                .w(px(ROW_ACTIONS))
                .h_full()
                .when_some(dot, |slot, dot| {
                    slot.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(style::S3))
                            .flex()
                            .items_center()
                            .when(shown, |holder| holder.invisible())
                            .when(!shown, |holder| {
                                holder.group_hover(ui::LIST_ROW_GROUP, |s| s.invisible())
                            })
                            .child(dot),
                    )
                })
                .child(actions),
        )
        .into_any_element()
    }

    fn sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let chrome = washed(style::panel(), self.current_environment());
        div()
            .id("sidebar")
            .role(Role::Group)
            .aria_label("Sidebar")
            .w(px(self.navigator_width.max(style::SIDEBAR)))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(chrome)
            .border_r_1()
            .border_color(style::line())
            .child(self.title_row(cx))
            .child(self.search_row(cx))
            // The list sizes to its rows up to 40 % of the sidebar, then
            // scrolls; Objects follows it directly (D10).
            .child(
                div()
                    .flex_none()
                    .max_h(gpui::relative(0.4))
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .when(!self.connections.is_empty(), |list| {
                        list.border_b_1().border_color(style::line_soft())
                    })
                    .child(self.connection_list(cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .pt(px(style::S2))
                    .map(|tree| match self.engine_tree() {
                        Some(engine) => tree.child(engine),
                        None => tree.child(self.navigator.clone()),
                    }),
            )
    }

    fn tab_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let env = self.env_color();
        let chrome = washed(style::panel(), self.current_environment());
        // Measured last frame; the overflow list appears once tabs scroll.
        let overflow = self.shell.tab_scroll.max_offset().x > px(0.);
        let tabs_open = self.shell.menu == Some(ShellMenu::Tabs);
        let tools_open = self.shell.menu == Some(ShellMenu::Tools);
        let tabs: Vec<AnyElement> = self
            .tab_items(cx)
            .into_iter()
            .map(|tab| {
                let TabInfo {
                    id,
                    title: name,
                    icon: kind,
                    status,
                    active,
                    pinned,
                    closable,
                } = tab;
                let group = SharedString::from(format!("tab-group-{id}"));
                // The close slot is reserved on every tab so widths never
                // jump; its glyph shows on hover and on the active tab.
                let close = if closable {
                    let close = self
                        .row_button(
                            SharedString::from(format!("close-tab-{id}")),
                            format!("Close {name}"),
                            "icons/close.svg",
                            style::dim(),
                            Operation::CloseDocument(id.clone()),
                            cx,
                        )
                        .flex_none()
                        .hover(|s| s.bg(style::hover()));
                    if active {
                        close.into_any_element()
                    } else {
                        close
                            .invisible()
                            .group_hover(group.clone(), |s| s.visible())
                            .into_any_element()
                    }
                } else {
                    div().flex_none().w(px(TAB_CLOSE)).into_any_element()
                };
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
                .group(group)
                .h_full()
                .rounded_none()
                .min_w(px(TAB_MIN))
                .max_w(px(TAB_MAX))
                // The active tab keeps its full width; others compress
                // toward the minimum before the strip scrolls.
                .when(active, |tab| tab.flex_shrink_0())
                .pl(px(style::S4))
                .pr(px(style::S2))
                .gap(px(style::S3))
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
                    tab_glyph(kind),
                    if active { style::text() } else { style::dim() },
                ))
                .when(pinned, |tab| tab.child(icon("icons/pin.svg", style::dim())))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(name),
                )
                .child(close)
                .into_any_element()
            })
            .collect();
        div()
            .id("tab-bar")
            .h(px(style::BAR))
            .flex_none()
            .flex()
            .items_stretch()
            .bg(chrome)
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
                    .track_scroll(&self.shell.tab_scroll)
                    .children(tabs),
            )
            .child(self.drag_region("tab-bar-drag"))
            .child(
                div()
                    .id("tab-actions")
                    .flex()
                    .items_center()
                    .gap(px(style::S1))
                    .px(px(style::S3))
                    .border_l_1()
                    .border_color(style::line_soft())
                    // Every open tab, once the strip overflows (D9).
                    .when(overflow, |actions| {
                        actions.child(
                            self.icon_button(
                                "tab-overflow",
                                "All tabs",
                                "icons/chevron_down.svg",
                                Operation::ShellMenu(ShellMenu::Tabs),
                                cx,
                            )
                            .aria_expanded(tabs_open)
                            .relative()
                            .when(tabs_open, |button| button.bg(style::hover()))
                            .child(popover::probe(self.shell.tabs_anchor.clone())),
                        )
                    })
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
                        .aria_expanded(tools_open)
                        .relative()
                        .when(tools_open, |button| button.bg(style::hover()))
                        .child("Tools")
                        .child(icon("icons/chevron_down.svg", style::dim()))
                        .child(popover::probe(self.shell.tools_anchor.clone())),
                    ),
            )
    }

    /// Env tag, session state, connection name, database and host in `dim`,
    /// the last query's time, then Console (F0-15). "Saved" only shows while
    /// something is unsaved.
    fn status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let env = self.env_color();
        if self.shell.status_collapsed {
            return div()
                .id("status-bar")
                .role(Role::Button)
                .aria_label("Show status bar")
                .tab_index(0)
                .h(px(style::S2))
                .flex_none()
                .bg(style::with_alpha(env, 0xff))
                .cursor_pointer()
                .aria_keyshortcuts("Meta+J")
                .tooltip(ui::tooltip("Show status bar  ⌘J"))
                .tooltip_show_delay(ui::tooltip_delay())
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
        let latency_ms = connection.and_then(|c| self.shell.last_latency.get(&c.id).copied());
        let latency_value = latency_ms
            .map(|ms| format!("{ms} ms"))
            .unwrap_or_else(|| "—".into());
        let latency = format!("last query {latency_value}");
        let host = connection
            .and_then(|c| c.endpoint())
            .and_then(|endpoint| status_host(&endpoint.host, endpoint.port));
        let database = connection
            .and_then(|c| c.endpoint())
            .map(|endpoint| endpoint.database);
        let detail = status_detail(database.as_deref(), host.as_deref()).join(" · ");
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
        let unsaved = self.closing || !matches!(self.save_status, SaveStatus::Saved);
        let unread = self.dock.read(cx).unread();
        div()
            .id("status-bar")
            .role(Role::Status)
            .aria_label(announcement)
            .h(px(style::STATUS))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(style::S5))
            .px(px(style::S4))
            .bg(washed(style::panel(), self.current_environment()))
            .border_t_1()
            .border_color(style::line())
            .text_size(px(style::FONT_SMALL))
            .text_color(style::dim())
            .whitespace_nowrap()
            .when_some(connection, |bar, c| bar.child(ui::env_tag(c.environment)))
            .when_some(phase.clone(), |bar, phase| {
                let dot = div().flex_none().size(px(6.)).rounded_full();
                bar.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(style::S3))
                        .child(match phase {
                            ConnectionPhase::Connected => dot.bg(style::ok()),
                            ConnectionPhase::Connecting => {
                                dot.border_1().border_color(style::accent())
                            }
                            ConnectionPhase::Failed(_) => dot.bg(style::bad()),
                            ConnectionPhase::Idle => dot.border_1().border_color(style::faint()),
                        })
                        .child(
                            div()
                                .when(failure.is_some(), |s| s.text_color(style::bad_text()))
                                .child(state),
                        ),
                )
            })
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(style::S3))
                    .overflow_hidden()
                    .child(div().flex_none().text_color(style::text()).child(
                        connection.map_or_else(|| "No connection".to_owned(), |c| c.name.clone()),
                    ))
                    .when(!detail.is_empty(), |line| {
                        line.child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(detail),
                        )
                    }),
            )
            .when_some(latency_ms, |bar, ms| {
                bar.child(div().flex_none().child(format!("Last query {ms} ms")))
            })
            .child(match failure {
                Some(error) => div()
                    .id("connection-failure")
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(style::bad_text())
                    .child(error)
                    .into_any_element(),
                None => div().flex_1().into_any_element(),
            })
            .when(unsaved, |bar| {
                bar.child(
                    div()
                        .id("workspace-save-status")
                        .flex_none()
                        .text_color(if matches!(self.save_status, SaveStatus::Failed(_)) {
                            style::warn()
                        } else {
                            style::dim()
                        })
                        .child(if self.closing { "Closing".into() } else { save }),
                )
            })
            .child(
                self.shell_button("console-badge", "Toggle console", Operation::Console, cx)
                    .h(px(style::ROW - style::S2))
                    .px(px(style::S3))
                    .child(icon("icons/terminal.svg", style::dim()))
                    .child("Console")
                    .when(unread > 0, |button| {
                        button.child(ui::count_badge(unread.to_string()))
                    }),
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
                .size(px(style::ROW - style::S2)),
            )
            .into_any_element()
    }

    fn menu_item(
        &self,
        entry: MenuEntry,
        highlighted: bool,
        selected: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let MenuEntry {
            id,
            label,
            ax_label,
            icon: glyph,
            hint,
            style: look,
            group: _,
            operation,
        } = entry;
        let row = match look {
            EntryStyle::Plain => popover::item(
                id,
                label,
                glyph,
                hint.map(|(keys, _)| SharedString::from(keys)),
                highlighted,
                true,
            ),
            EntryStyle::Check(checked) => {
                popover::check_item(id, label, checked, highlighted, true)
                    .when_some(hint, |row, (keys, _)| row.child(ui::shortcut(keys)))
            }
            EntryStyle::Danger => popover::danger_item(id, label, glyph, highlighted, true),
        };
        let row = row
            .when_some(ax_label, |row, ax| row.aria_label(ax))
            .when_some(hint, |row, (_, keys)| row.aria_keyshortcuts(keys))
            // Focus stays on the menu; the selected row is what assistive
            // technology reads as focused, painted or not.
            .when(selected && !highlighted, |row| row.aria_active_descendant());
        self.activation(row, operation, cx).into_any_element()
    }

    fn menu(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let menu = self.shell.menu.clone()?;
        let entries = self.menu_entries(&menu, cx);
        let selected = self
            .shell
            .menu_selected
            .min(entries.len().saturating_sub(1));
        // No row is painted as highlighted until a key moves (F0-11).
        let keyed = self.shell.menu_keyed;
        let breaks = group_breaks(&entries.iter().map(|entry| entry.group).collect::<Vec<_>>());
        let mut items: Vec<AnyElement> = Vec::with_capacity(entries.len() + 4);
        for (index, (entry, divider)) in entries.into_iter().zip(breaks).enumerate() {
            if divider {
                items.push(popover::divider().into_any_element());
            }
            items.push(self.menu_item(entry, keyed && index == selected, index == selected, cx));
        }
        let panel = popover::panel(
            "shell-menu",
            Role::Menu,
            match &menu {
                ShellMenu::Projects => "Projects",
                ShellMenu::Tools => "Tools",
                ShellMenu::Connection(_) => "Connection actions",
                ShellMenu::Environments => "Environments",
                ShellMenu::Tabs => "Open tabs",
            },
        )
        .key_context("ShellMenu")
        .track_focus(&self.shell.menu_focus)
        .capture_key_down(cx.listener(Self::menu_key_down))
        .min_w(px(MENU_MIN))
        .children(items);
        // `appear` makes its element relative, so placement lives on a wrapper.
        let panel = ui::appear("shell-menu-panel", panel);
        let placed = match &menu {
            ShellMenu::Connection(_) => match self.shell.menu_anchor {
                Some(position) => anchored()
                    .position(position)
                    .snap_to_window_with_margin(px(style::S4))
                    .child(panel)
                    .into_any_element(),
                // Opened without a click position (accessibility action).
                None => div()
                    .absolute()
                    .top(px(style::BAR + 60.))
                    .left(px(style::SIDEBAR - 150.))
                    .child(panel)
                    .into_any_element(),
            },
            ShellMenu::Tools => under(
                self.shell.tools_anchor.get(),
                true,
                panel,
                div().absolute().top(px(style::BAR)).right(px(style::S4)),
            ),
            ShellMenu::Tabs => under(
                self.shell.tabs_anchor.get(),
                true,
                panel,
                div().absolute().top(px(style::BAR)).right(px(style::S4)),
            ),
            ShellMenu::Projects => under(
                self.shell.projects_anchor.get(),
                false,
                panel,
                div().absolute().top(px(style::BAR)).left(px(style::S4)),
            ),
            ShellMenu::Environments => under(
                self.shell.filter_anchor.get(),
                true,
                panel,
                div()
                    .absolute()
                    .top(px(style::BAR + style::CONTROL))
                    .left(px(style::S4)),
            ),
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

    /// The document area with no tab (D16): first run offers New connection;
    /// with connections, a table or a new query.
    fn workspace_empty(&self, cx: &Context<Self>) -> AnyElement {
        let area = div().flex_1().min_h_0().flex().flex_col().bg(style::bg());
        if self.loading || !self.restored {
            return area
                .items_center()
                .justify_center()
                .text_color(style::dim())
                .child(if self.loading {
                    "Loading workspace"
                } else {
                    "Workspace recovery required"
                })
                .into_any_element();
        }
        let (glyph, title, body, action) = if self.connections.is_empty() {
            (
                "icons/database_zap.svg",
                "No connections yet",
                "Add a database to browse its tables and run queries.",
                self.activation(
                    ui::button(
                        "empty-new-connection",
                        "New connection",
                        ui::Variant::Primary,
                        true,
                    )
                    .tab_index(0),
                    Operation::NewConnection,
                    cx,
                )
                .aria_keyshortcuts("Meta+N"),
            )
        } else {
            (
                "icons/table.svg",
                "Open a table or start a query",
                "Pick a table in the sidebar, or press ⌘T for a new query.",
                self.activation(
                    ui::button("empty-new-query", "New query", ui::Variant::Secondary, true)
                        .tab_index(0),
                    Operation::New,
                    cx,
                )
                .aria_keyshortcuts("Meta+T"),
            )
        };
        area.child(ui::empty_state(
            glyph,
            title,
            Some(body.into()),
            Some(action.into_any_element()),
        ))
        .into_any_element()
    }

    pub(super) fn render_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        window.set_rem_size(px(style::REM));
        // Keep the active tab in view when it changes, the window resizes or
        // the sidebar moves (D9). The strip applies it on its next prepaint.
        let active_tab = self
            .tab_items(cx)
            .into_iter()
            .enumerate()
            .find(|(_, tab)| tab.active)
            .map(|(index, tab)| (index, tab.id));
        if let Some((index, id)) = active_tab {
            let key = (
                id,
                f32::from(window.viewport_size().width) as i32,
                self.shell.sidebar_collapsed,
            );
            if self.shell.tab_scroll_key.as_ref() != Some(&key) {
                self.shell.tab_scroll.scroll_to_item(index);
                self.shell.tab_scroll_key = Some(key);
            }
        }
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
                        .h(px(STRIP))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(style::S3))
                        .px(px(style::INSET_BAR + style::S1))
                        .bg(style::bad_fill())
                        .border_b_1()
                        .border_color(style::bad_line())
                        .text_size(px(style::FONT_SMALL))
                        .text_color(style::text())
                        .child(icon("icons/warning.svg", style::bad()))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("Production"),
                        )
                        .child("Writes require review and confirmation"),
                )
            })
            .child(match (self.engine_body(), self.active_index()) {
                // A selected engine connection renders its own documents.
                (Some(body), _) => ui::appear(
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
                (None, Some(index)) => ui::appear(
                    SharedString::from(format!("document-{}", self.documents[index].metadata.id)),
                    div()
                        .flex_1()
                        .min_h_0()
                        .bg(style::bg())
                        .child(self.documents[index].view.clone()),
                )
                .into_any_element(),
                (None, None) => self.workspace_empty(cx),
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
                area.child(ui::shake(
                    ("workspace-error-shake", key),
                    div()
                        .id("workspace-error")
                        .role(Role::Alert)
                        .aria_label(message.clone())
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(style::S4))
                        .px(px(style::INSET_BAR + style::S1))
                        .py(px(style::S2))
                        .bg(style::bad_fill())
                        .border_t_1()
                        .border_color(style::bad_line())
                        .text_color(style::bad_text())
                        .child(icon("icons/warning.svg", style::bad()))
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
                                        .text_color(style::text())
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
            // The environment frame: 2 px in the environment colour, following
            // the window's corners, never intercepting input. The wash lives on
            // the chrome (sidebar, tab bar, status bar) only (D8).
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(px(WINDOW_RADIUS))
                    .border_2()
                    .border_color(style::with_alpha(env, 0xff)),
            )
            .when_some(self.menu(cx), |root, menu| root.child(menu))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::DevelopmentConnectionOrganization;

    #[test]
    fn row_detail_shows_the_file_name_for_file_backed_connections() {
        assert_eq!(
            row_detail("/private/tmp/scratch/demo.sqlite"),
            "demo.sqlite"
        );
        assert_eq!(row_detail("C:\\data\\app.db"), "app.db");
        assert_eq!(row_detail("dbunk_demo"), "dbunk_demo");
        assert_eq!(row_detail("0"), "0");
        assert_eq!(row_detail(""), "");
    }

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
        assert_eq!(labels, [("Development", 1), ("Production", 1)]);
        let prod_only = groups(&all, Some("Billing"), Some(Production), "");
        assert_eq!(prod_only.len(), 1);
        // Search spans projects and labels groups with their project.
        let search = groups(&all, Some("Billing"), None, "EVENTS");
        assert_eq!(search[0].0, "Analytics · Development");
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
                .map(|entry| entry.label)
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
    fn tools_are_grouped_in_order_with_an_icon_on_every_row() {
        for (engine, general) in [(None, true), (None, false), (Some(true), true)] {
            let entries = tool_entries(engine, general);
            let groups: Vec<u8> = entries.iter().map(|entry| entry.group).collect();
            // Groups never interleave: each appears as one run, in order.
            assert!(
                groups.windows(2).all(|pair| pair[0] <= pair[1]),
                "{groups:?}"
            );
            assert!(popover::icons_consistent(
                entries.iter().map(|entry| !entry.icon.is_empty())
            ));
        }
        let entries = tool_entries(None, false);
        let group_of = |label: &str| {
            entries
                .iter()
                .find(|entry| entry.label == label)
                .map(|entry| entry.group)
        };
        assert_eq!(group_of("Save query"), Some(GROUP_QUERY));
        assert_eq!(group_of("Schema map"), Some(GROUP_DATABASE));
        assert_eq!(group_of("Open table by name"), Some(GROUP_DATA));
        assert_eq!(group_of("Pin tab"), Some(GROUP_TAB));
        assert_eq!(group_of("Credentials"), Some(GROUP_SETTINGS));
    }

    #[test]
    fn dividers_go_where_the_group_changes() {
        assert_eq!(group_breaks(&[]), Vec::<bool>::new());
        assert_eq!(
            group_breaks(&[0, 0, 1, 1, 3]),
            [false, false, true, false, true]
        );
        assert_eq!(group_breaks(&[2]), [false]);
    }

    #[test]
    fn every_tool_tab_has_its_own_icon() {
        use dbunk_lib::backend::WorkspaceTool as T;
        let tools = [
            T::History,
            T::SavedQueries,
            T::Objects,
            T::Administration,
            T::SchemaMap,
            T::SchemaCompare,
            T::CsvTransfer,
            T::BackupRestore,
            T::TableCopy,
            T::TableSeed,
        ];
        for (i, a) in tools.iter().enumerate() {
            assert!(
                tools[i + 1..]
                    .iter()
                    .all(|b| tool_icon(*a) != tool_icon(*b))
            );
            assert_ne!(tool_icon(*a), "icons/table.svg");
            assert_ne!(tool_icon(*a), "icons/file_code.svg");
        }
    }

    #[test]
    fn the_wash_tints_chrome_slightly_and_stays_opaque() {
        let base = style::panel();
        let dev = washed(base, Some(DevelopmentEnvironment::Development));
        assert_eq!(dev.a, 1.);
        // Development is green: the green channel rises, a little.
        assert!(dev.g > base.g && dev.g - base.g < 0.05, "{dev:?}");
        // Readable text stays readable on washed chrome.
        for environment in style::ENVIRONMENTS {
            let chrome = washed(base, Some(environment));
            assert!(
                style::contrast(style::dim(), chrome) >= 4.5,
                "{environment:?}"
            );
        }
    }

    #[test]
    fn status_detail_shows_database_and_host_when_present() {
        assert_eq!(
            status_detail(Some("dbunk_demo"), Some("127.0.0.1:5432")),
            ["dbunk_demo", "127.0.0.1:5432"]
        );
        // A file engine shows the file's name and no host.
        assert_eq!(status_detail(Some("/tmp/a.db"), None), ["a.db"]);
        assert!(status_detail(Some(""), None).is_empty());
        assert!(status_detail(None, None).is_empty());
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
