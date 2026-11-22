//! Administration Tool tab with deliberately collected reads and exact-target
//! signal review. Read cancellation and backend signals are separate actions.
use crate::server_details_model::{Capture as ServerCapture, ServerSection};
use crate::{
    admin_model::{ReadState, Reply, Section as ActivitySection, Snapshot},
    controller::{Host, TableCommand, TableControls, TableMessage, TableReceiver},
};
use dbunk_lib::backend::{WorkspaceDocument, data::DataCloseOutcome};
use gpui::{
    ClipboardItem, Context, Entity, FocusHandle, Focusable, KeyDownEvent, Role, ScrollHandle,
    SharedString, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use std::{cell::Cell, rc::Rc, sync::Arc};
mod control;
use control::{Control, ControlAction};
pub enum AdminEvent {
    Changed,
    PersistApply(u64),
    EditConnection(String),
    /// Exact recorded SQL to open as a disconnected, unexecuted draft.
    OpenQuery(crate::query_library_view::OpenQuery),
}
impl gpui::EventEmitter<AdminEvent> for AdminView {}
mod audit;
mod connection_settings;
mod filter;
mod overview;
mod server;
use audit::Audit;
use filter::FilterInput;
use server::Section;

#[derive(Clone, Copy)]
enum Action {
    Connect,
    Refresh,
    Cancel,
    Copy,
    Clear,
    Section(Section),
    Search,
    NonDefault,
    AuditNext,
    Control(ControlAction),
    Overview,
    ConnectionSettings,
}
/// Most height the selected row's details take under the table.
const DETAILS_HEIGHT: f32 = 240.;
const ACTIONS: [(Action, &str); 23] = [
    (Action::Connect, "Connect"),
    (Action::Refresh, "Refresh"),
    (Action::Cancel, "Cancel read"),
    (Action::Copy, "Copy selected details"),
    (Action::Clear, "Clear captures"),
    (
        Action::Section(Section::Activity(ActivitySection::Sessions)),
        "Sessions",
    ),
    (
        Action::Section(Section::Activity(ActivitySection::Locks)),
        "Locks",
    ),
    (
        Action::Section(Section::Activity(ActivitySection::Pending)),
        "Pending transactions",
    ),
    (
        Action::Section(Section::Server(ServerSection::Facts)),
        "Server facts",
    ),
    (
        Action::Section(Section::Server(ServerSection::Settings)),
        "Settings",
    ),
    (
        Action::Section(Section::Server(ServerSection::Extensions)),
        "Extensions",
    ),
    (Action::Section(Section::Audit), "Safety overrides"),
    (Action::Search, "Search captured settings"),
    (Action::NonDefault, "Non-default source"),
    (Action::AuditNext, "Older overrides"),
    (
        Action::Control(ControlAction::ReviewCancel),
        "Review cancel query",
    ),
    (
        Action::Control(ControlAction::ReviewTerminate),
        "Review terminate session",
    ),
    (
        Action::Control(ControlAction::Apply),
        "Send reviewed signal",
    ),
    (
        Action::Control(ControlAction::Confirm),
        "Confirm reviewed signal",
    ),
    (
        Action::Control(ControlAction::Cancel),
        "Cancel signal request",
    ),
    (
        Action::Control(ControlAction::Clear),
        "Clear signal recovery",
    ),
    (Action::Overview, "Overview statistics"),
    (Action::ConnectionSettings, "Connection settings"),
];

pub struct AdminView {
    host: Arc<Host>,
    id: String,
    connection: Option<String>,
    wake: async_channel::Sender<()>,
    controls: Option<TableControls>,
    receiver: Option<TableReceiver>,
    ready: bool,
    opening: bool,
    editable: bool,
    read: ReadState,
    control: Control,
    control_details: FocusHandle,
    control_scroll: ScrollHandle,
    budget: Rc<Cell<usize>>,
    snapshot: Option<Snapshot>,
    server: Option<ServerCapture>,
    overview: overview::Overview,
    /// Survives Clear so request generations stay monotonic for this tab.
    recent: crate::overview_model::recent::Requests,
    /// Local connection metadata: "name · engine · database".
    identity: Option<String>,
    /// Display name, engine and environment for the tool header.
    tool_connection: Option<crate::overview_view::chrome::ToolConnection>,
    /// Latest workspace health tick for the bound connection, if any.
    health: Option<String>,
    connection_settings: connection_settings::Settings,
    audit: Audit,
    server_selected: [Option<String>; 3],
    server_stale: bool,
    filter: Entity<FilterInput>,
    filter_query: String,
    non_default: bool,
    section: Section,
    selected: [usize; 3],
    stale: bool,
    status: String,
    failure: Option<String>,
    root: FocusHandle,
    list: FocusHandle,
    details: FocusHandle,
    buttons: Vec<FocusHandle>,
    previous_focus: Option<FocusHandle>,
    scroll: UniformListScrollHandle,
    detail_scroll: ScrollHandle,
}
impl AdminView {
    pub fn new(
        host: Arc<Host>,
        document: &WorkspaceDocument,
        wake: async_channel::Sender<()>,
        budget: Rc<Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            host,
            id: document.id.clone(),
            connection: document.connection_id.clone(),
            wake,
            controls: None,
            receiver: None,
            ready: false,
            opening: false,
            editable: true,
            read: ReadState::default(),
            control: Control::new(document.admin_control.clone(), budget.clone()),
            control_details: cx.focus_handle(),
            control_scroll: ScrollHandle::new(),
            budget: budget.clone(),
            snapshot: None,
            server: None,
            overview: overview::Overview::default(),
            recent: Default::default(),
            identity: None,
            tool_connection: None,
            health: None,
            connection_settings: connection_settings::Settings::new(budget.clone(), cx),
            audit: Audit::default(),
            server_selected: Default::default(),
            server_stale: false,
            filter: cx.new(|cx| FilterInput::new(budget.clone(), window, cx)),
            filter_query: String::new(),
            non_default: false,
            section: Section::Activity(ActivitySection::Sessions),
            selected: [0; 3],
            stale: false,
            status: "Disconnected. Connect, then Refresh to collect readings".into(),
            failure: None,
            root: cx.focus_handle(),
            list: cx.focus_handle(),
            details: cx.focus_handle(),
            buttons: (0..ACTIONS.len()).map(|_| cx.focus_handle()).collect(),
            previous_focus: None,
            scroll: UniformListScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
        }
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn has_pending(&self) -> bool {
        self.control.pending()
            || self.audit.has_pending()
            || self
                .receiver
                .as_ref()
                .is_some_and(TableReceiver::has_pending)
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editable = editable;
        self.connection_settings
            .view
            .update(cx, |view, cx| view.set_editable(editable, cx));
        cx.notify();
    }
    pub fn focus_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection_settings.show {
            window.focus(&self.connection_settings.view.read(cx).focus(), cx);
            return;
        }
        if self.overview.show
            && let Some(view) = &self.overview.view
        {
            window.focus(&view.read(cx).focus(cx), cx);
            return;
        }
        self.control.try_admit();
        let order = self.focus_order(cx);
        let focus = self
            .previous_focus
            .as_ref()
            .filter(|focus| order.contains(focus))
            .unwrap_or(if self.has_capture() {
                &self.list
            } else if self.ready || self.section == Section::Audit {
                &self.buttons[1]
            } else {
                &self.buttons[0]
            });
        window.focus(focus, cx);
    }
    pub fn remember_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.root.contains_focused(window, cx)
            || self
                .connection_settings
                .view
                .read(cx)
                .contains_focus(window, cx)
            || self
                .overview
                .view
                .as_ref()
                .is_some_and(|view| view.read(cx).contains_focus(window, cx))
        {
            self.previous_focus = window.focused(cx);
        }
    }
    pub fn bind_connection(&mut self, id: String, cx: &mut Context<Self>) {
        if self.controls.is_none() && !self.read.busy() && !self.has_control_recovery() {
            self.connection_settings
                .view
                .update(cx, |view, cx| view.receive(None, None, cx));
            self.connection = Some(id);
            self.identity = None;
            self.tool_connection = None;
            self.health = None;
            self.recent.reset();
            self.audit.clear();
            self.clear_results(cx);
        }
    }
    pub fn begin_connect(&mut self, cx: &mut Context<Self>) {
        if !self.editable || self.controls.is_some() || self.opening {
            return;
        }
        let Some(connection) = &self.connection else {
            self.status = "Select a connection first".into();
            cx.notify();
            return;
        };
        match self
            .host
            .open_table_document(self.id.clone(), connection.clone(), self.wake.clone())
        {
            Ok((controls, receiver)) => {
                self.controls = Some(controls);
                self.receiver = Some(receiver);
                self.opening = true;
                self.ready = false;
                self.failure = None;
                self.status = "Connecting administration reader".into();
            }
            Err(error) => self.status = error.into(),
        }
        cx.notify();
    }
    /// A restore changed this connection. Stop the metadata/data lane and
    /// preserve staged intent; query sessions are owned by a different lane.
    pub fn invalidate_after_restore(&mut self, cx: &mut Context<Self>) {
        if let Some(controls) = &self.controls {
            controls.stop();
        }
        self.mark_disconnected(cx);
        self.status =
            "Database may have changed; reconnect and refresh before using retained data".into();
        cx.notify();
    }
    pub fn mark_disconnected(&mut self, cx: &mut Context<Self>) {
        if let Some(controls) = self.controls.take() {
            controls.stop();
        }
        self.receiver = None;
        self.control.disconnected();
        cx.emit(AdminEvent::Changed);
        self.ready = false;
        self.opening = false;
        self.read.disconnected();
        self.audit.stale = self.audit.capture.is_some();
        self.stale = self.snapshot.is_some();
        self.server_stale = self.server.is_some();
        self.overview.current = false;
        self.overview.incoming = None;
        self.status = "Disconnected; retained readings may be stale".into();
        cx.notify();
    }
    pub fn clear_results(&mut self, cx: &mut Context<Self>) {
        if self.control.pending()
            || self.read.busy()
            || self.opening
            || self.audit.pending.is_some()
        {
            return;
        }
        self.snapshot = None;
        self.server = None;
        self.overview = overview::Overview::default();
        self.control.try_admit();
        self.audit.clear();
        self.server_selected = Default::default();
        self.server_stale = false;
        self.filter.update(cx, |field, cx| {
            field.try_admit(cx);
        });
        self.selected = [0; 3];
        self.stale = false;
        self.previous_focus = None;
        self.status = "Capture cleared".into();
        cx.notify();
    }
    fn load(&mut self, cx: &mut Context<Self>) {
        if self.section == Section::Audit {
            self.load_audit(false, cx);
            return;
        }
        if !self.enabled(Action::Refresh) {
            return;
        }
        let id = match self.read.begin() {
            Ok(id) => id,
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };
        match self
            .controls
            .as_ref()
            .ok_or("Connect first")
            .and_then(|controls| {
                controls.send(match self.section {
                    Section::Activity(_) => TableCommand::Admin(id),
                    Section::Server(_) => TableCommand::ServerDetails(id),
                    Section::Audit => unreachable!("local audit loads separately"),
                })
            }) {
            Ok(()) => {
                self.failure = None;
                match self.section {
                    Section::Activity(_) => self.stale = self.snapshot.is_some(),
                    Section::Server(_) => self.server_stale = self.server.is_some(),
                    Section::Audit => unreachable!("local audit loads separately"),
                }
                self.status =
                    "Collecting administration readings; previous capture retained".into();
            }
            Err(error) => {
                self.read.settle(id);
                self.status = error.into();
            }
        }
        cx.notify();
    }
    pub fn drain_one(&mut self, cx: &mut Context<Self>) -> bool {
        if self.drain_audit(cx) {
            return true;
        }
        let Some(envelope) = self.receiver.as_ref().and_then(TableReceiver::try_recv) else {
            return false;
        };
        match envelope.into_message() {
            TableMessage::Opened => {
                self.opening = false;
                self.ready = true;
                self.status = "Connected. Refresh to collect administration readings".into();
            }
            TableMessage::Admin(id, result) => match self.read.settle(id) {
                Reply::Stale => return true,
                Reply::Cancelled => {
                    self.failure = None;
                    self.status =
                        "Read cancelled; late reply discarded and previous capture retained".into();
                }
                Reply::Current => match result {
                    Ok(data) => match Snapshot::from_capture(data, self.budget.clone()) {
                        Ok(snapshot) => {
                            self.failure = None;
                            self.snapshot = Some(snapshot);
                            self.selected = [0; 3];
                            self.stale = false;
                            if matches!(self.section, Section::Activity(_)) {
                                self.detail_scroll.set_offset(gpui::point(px(0.), px(0.)));
                                self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
                            }
                            self.status =
                                "Activity readings collected; this is not an atomic historical snapshot"
                                    .into();
                        }
                        Err(error) => self.status = error.into(),
                    },
                    Err(error) => {
                        self.status = format!(
                            "Administration read failed: {error:?}; previous capture retained"
                        );
                    }
                },
            },
            TableMessage::AdminApplied(id, result) => self.control_received(id, result, cx),
            TableMessage::ServerDetails(id, result) => self.settle_server(id, result, cx),
            TableMessage::Overview(id, result) => self.settle_overview(id, result, cx),
            TableMessage::Error(error) => {
                // Cancel failure does not settle the original request. Fatal
                // worker errors are followed by its owned Closed result.
                self.status = format!("Administration read failed: {error}");
                self.failure = Some(self.status.clone());
            }
            TableMessage::Closed(result) => {
                self.mark_disconnected(cx);
                self.status = match result {
                    Ok(DataCloseOutcome::Closed) => self.failure.take().unwrap_or_else(|| {
                        "Administration disconnected; retained readings may be stale".into()
                    }),
                    Ok(DataCloseOutcome::ConnectionDataClosed) => {
                        "All data documents on this connection closed during cleanup".into()
                    }
                    Err(error) => format!("Administration cleanup failed: {error}"),
                };
            }
            _ => self.status = "Unexpected reply in administration document".into(),
        }
        cx.notify();
        true
    }
    fn enabled(&self, action: Action) -> bool {
        if !self.editable {
            return false;
        }
        match action {
            Action::Connect => {
                self.section != Section::Audit && self.controls.is_none() && !self.opening
            }
            Action::Refresh => {
                (self.ready || self.section == Section::Audit && self.connection.is_some())
                    && !self.control.pending()
                    && !self.read.busy()
                    && self.audit.pending.is_none()
            }
            Action::AuditNext => {
                self.section == Section::Audit
                    && self.enabled(Action::Refresh)
                    && self
                        .audit
                        .capture
                        .as_ref()
                        .is_some_and(|capture| capture.can_continue())
            }
            Action::Cancel => self.read.busy() && !self.read.cancelling(),
            Action::Copy => self.selected_index().is_some(),
            Action::Clear => {
                (self.snapshot.is_some()
                    || self.server.is_some()
                    || self.audit.capture.is_some()
                    || self.overview.view.is_some())
                    && !self.read.busy()
                    && !self.opening
                    && self.audit.pending.is_none()
            }
            Action::Control(action) => self.control_enabled(action),
            Action::Search | Action::NonDefault => {
                self.section == Section::Server(ServerSection::Settings)
            }
            Action::Section(_) => true,
            Action::ConnectionSettings => {
                !self.read.busy()
                    && !self.control.pending()
                    && !self.opening
                    && self.audit.pending.is_none()
            }
            Action::Overview => {
                !self.read.busy() && !self.control.pending() && self.audit.pending.is_none()
            }
        }
    }
    fn focus_order(&self, cx: &gpui::App) -> Vec<FocusHandle> {
        // Visual order: header actions, capture row, section tabs, section
        // tools, the table, the selected row's details and actions, then
        // the signal review.
        let mut order = [0, 1, 2, 21, 22, 4]
            .into_iter()
            .chain(5..12)
            .filter(|index| self.enabled(ACTIONS[*index].0))
            .map(|index| self.buttons[index].clone())
            .collect::<Vec<_>>();
        if self.section == Section::Server(ServerSection::Settings) {
            order.push(self.filter.focus_handle(cx));
            order.extend([self.buttons[12].clone(), self.buttons[13].clone()]);
        }
        if self.enabled(Action::AuditNext) {
            order.push(self.buttons[14].clone());
        }
        order.push(self.list.clone());
        order.extend(
            [15, 16, 3]
                .into_iter()
                .filter(|index| self.enabled(ACTIONS[*index].0))
                .map(|index| self.buttons[index].clone()),
        );
        if self.selected_index().is_some() {
            order.push(self.details.clone());
        }
        order.extend(
            (17..21)
                .filter(|index| self.enabled(ACTIONS[*index].0))
                .map(|index| self.buttons[index].clone()),
        );
        if self.control.can_display() {
            order.push(self.control_details.clone());
        }
        order
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action) {
            return;
        }
        match action {
            Action::Overview => self.show_overview(window, cx),
            Action::ConnectionSettings => self.show_connection_settings(window, cx),
            Action::Control(action) => self.control_action(action, window, cx),
            Action::Connect => self.begin_connect(cx),
            Action::Refresh => self.load(cx),
            Action::AuditNext => self.load_audit(true, cx),
            Action::Cancel => {
                if self.audit.pending.is_some() {
                    self.read.cancel();
                    self.status =
                        "Discard requested; waiting for the owned local read to settle".into();
                } else if let Some(controls) = &self.controls {
                    controls.cancel();
                    self.read.cancel();
                    self.status =
                        "Read cancellation requested; waiting for the owned operation".into();
                }
            }
            Action::Copy => {
                if let Some(details) = self.selected_details() {
                    cx.write_to_clipboard(ClipboardItem::new_string(details.clone()));
                    let copied = cx.read_from_clipboard().is_some_and(|item| {
                        matches!(item.entries(), [gpui::ClipboardEntry::String(text)] if text.text() == &details)
                    });
                    self.status = if copied {
                        "Captured row details copied"
                    } else {
                        "Could not verify details clipboard copy"
                    }
                    .into();
                }
            }
            Action::Clear => {
                self.clear_results(cx);
                window.focus(&self.list, cx);
            }
            Action::Search | Action::NonDefault => self.apply_filter(action, window, cx),
            Action::Section(section) => {
                self.section = section;
                self.detail_scroll.set_offset(gpui::point(px(0.), px(0.)));
                self.scroll.scroll_to_item(
                    self.selected_index().unwrap_or(0),
                    gpui::ScrollStrategy::Top,
                );
            }
        }
        cx.notify();
    }
    fn button(&self, index: usize, cx: &Context<Self>) -> gpui::AnyElement {
        self.button_as(index, Look::Tool, cx)
    }
    fn button_as(&self, index: usize, look: Look, cx: &Context<Self>) -> gpui::AnyElement {
        let (action, default_label) = ACTIONS[index];
        let label = if matches!(action, Action::Control(ControlAction::Clear)) {
            self.control.clear_label()
        } else {
            default_label
        };
        let enabled = self.enabled(action);
        let selected = matches!(action, Action::Section(section) if self.section == section)
            || matches!(action, Action::NonDefault) && self.non_default;
        let tab = matches!(action, Action::Section(_));
        let weak = cx.weak_entity();
        let id = ("admin-control", index);
        let button = if tab {
            crate::ui::segment(id, label, selected, enabled)
        } else if matches!(action, Action::NonDefault) {
            crate::overview_view::chrome::check_row(id, label, selected, enabled)
        } else {
            match look {
                Look::Accent => crate::ui::tool_button_accent(id, label, None, enabled),
                Look::Page => crate::ui::button(id, label, crate::ui::Variant::Primary, enabled),
                Look::Tool => crate::ui::tool_button(id, label, None, enabled, false),
            }
        };
        button
            .role(if tab {
                Role::Tab
            } else if matches!(action, Action::NonDefault) {
                Role::CheckBox
            } else {
                Role::Button
            })
            .when(tab, |button| button.aria_selected(selected))
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
                if matches!(action, Action::NonDefault) {
                    builder
                        .parent_node()
                        .set_toggled(gpui::accesskit::Toggled::from(selected));
                }
            })
            .track_focus(&self.buttons[index])
            .tab_index(0)
            .tab_stop(enabled)
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if tab && matches!(event.keystroke.key.as_str(), "left" | "right") {
                    let next = if event.keystroke.key == "left" {
                        (index - 5 + Section::ALL.len() - 1) % Section::ALL.len()
                    } else {
                        (index - 5 + 1) % Section::ALL.len()
                    };
                    this.activate(Action::Section(Section::ALL[next]), window, cx);
                    window.focus(&this.buttons[next + 5], cx);
                    cx.stop_propagation();
                }
            }))
            .into_any_element()
    }
    /// The header's next step: Connect until the reader is open, then
    /// Refresh (Safety overrides read locally and always offer Refresh).
    fn primary_index(&self) -> usize {
        if self.section != Section::Audit && (self.controls.is_none() || self.opening) {
            0
        } else {
            1
        }
    }
    /// Empty table: what is missing and the next step, hosting the primary
    /// action while there is nothing captured.
    fn empty_table(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let section = self.section.label().to_lowercase();
        if self.has_capture() {
            return crate::ui::empty_state(
                "icons/database_zap.svg",
                "Nothing to show",
                Some(self.capture_empty_label().into()),
                None,
            )
            .into_any_element();
        }
        let (title, body): (&'static str, String) = if self.section == Section::Audit {
            (
                "No overrides loaded",
                "Refresh to read the safety overrides this app recorded.".into(),
            )
        } else if self.read.busy() || self.audit.pending.is_some() {
            ("Reading…", format!("Collecting {section} from the server."))
        } else if self.opening {
            (
                "Connecting…",
                "Opening a read-only administration session.".into(),
            )
        } else if self.controls.is_none() {
            (
                "Not connected",
                "Connect to read sessions, locks and server settings.".into(),
            )
        } else {
            ("No readings yet", format!("Refresh to read {section}."))
        };
        crate::ui::empty_state(
            "icons/database_zap.svg",
            title,
            Some(body.into()),
            Some(self.button_as(self.primary_index(), Look::Page, cx)),
        )
        .into_any_element()
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Tool,
    Accent,
    Page,
}
/// Fixed column width for a captured section heading; the last column flexes.
fn track(label: &str) -> f32 {
    match label {
        "PID" => 72.,
        "Granted" => 80.,
        "User" | "Blockers" | "Transaction age" => 120.,
        "State" | "Time" => 140.,
        "Mode" | "Schema" => 160.,
        "Relation" | "Wait" | "Fact" => 200.,
        "Command" => 220.,
        "Name" => 260.,
        "Category" => 300.,
        _ => 160.,
    }
}
/// `data_table` columns for a section, from its sentence-case headings.
fn admin_columns(section: Section) -> Vec<crate::ui::TableColumn> {
    let labels: Vec<&str> = section.headings().split(" · ").collect();
    let last = labels.len().saturating_sub(1);
    labels
        .into_iter()
        .enumerate()
        .map(|(index, label)| {
            let column = if index == last {
                crate::ui::TableColumn::flex(label)
            } else {
                crate::ui::TableColumn::fixed(label, track(label))
            };
            let column = if matches!(label, "PID" | "Transaction age") {
                column.end()
            } else {
                column
            };
            if matches!(label, "PID" | "Query" | "Relation" | "Name" | "Schema") {
                column.mono()
            } else {
                column
            }
        })
        .collect()
}
/// Cells of a captured row summary: SQL on one line, server NULLs absent.
fn admin_cells(section: Section, label: &str, columns: usize) -> Vec<Option<SharedString>> {
    use crate::overview_view::chrome::{compact_timestamp, single_line, split_cells, today};
    let today = today();
    split_cells(label, columns)
        .into_iter()
        .enumerate()
        .map(|(index, cell)| {
            let cell = cell.trim();
            if cell.is_empty() || cell == "NULL" || cell == "NULL / NULL" {
                return None;
            }
            if index + 1 == columns {
                return Some(single_line(cell, 240).into());
            }
            if section == Section::Audit && index == 0 {
                return Some(
                    compact_timestamp(cell, today)
                        .unwrap_or_else(|| cell.to_owned())
                        .into(),
                );
            }
            Some(SharedString::from(cell.to_owned()))
        })
        .collect()
}
/// Activity statistics as label/value pairs, sizes in readable units.
fn metric_pairs(metrics: &str) -> Vec<(String, String)> {
    metrics
        .split(" · ")
        .filter_map(|part| part.split_once(": "))
        .map(|(label, value)| {
            let value = value
                .strip_suffix(" bytes")
                .and_then(|bytes| bytes.parse::<u64>().ok())
                .map_or_else(|| value.to_owned(), readable_bytes);
            (label.replace(" (cluster)", ""), value)
        })
        .collect()
}
/// `9694355` → `9.2 MB` (1024-based, one decimal from KB up).
fn readable_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.;
    let mut unit = 0;
    while value >= 1024. && unit + 1 < UNITS.len() {
        value /= 1024.;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
/// The end of a capture interval (`… collected A to B. …`): its last
/// timestamp, for "Updated 19:02:37".
fn updated_at(interval: &str) -> Option<String> {
    let today = crate::overview_view::chrome::today();
    interval
        .split_whitespace()
        .map(|word| word.trim_end_matches(['.', ',']))
        .rfind(|word| crate::overview_view::chrome::compact_timestamp(word, today).is_some())
        .map(str::to_owned)
}
impl Drop for AdminView {
    fn drop(&mut self) {
        if let Some(controls) = &self.controls {
            controls.stop();
        }
    }
}
impl Render for AdminView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::overview_view::chrome;
        use crate::style;
        if self.connection_settings.show {
            return div()
                .id("administration-connection-settings")
                .size_full()
                .child(self.connection_settings.view.clone());
        }
        self.sync_overview(window, cx);
        if self.overview.show
            && let Some(view) = &self.overview.view
        {
            return div()
                .id("administration-overview")
                .size_full()
                .child(view.clone());
        }
        let count = self.count();
        let selected = self.selected_index().unwrap_or(usize::MAX);
        let details = self.selected_details().unwrap_or_default();
        let connection = self.connection.as_deref().unwrap_or("unbound connection");
        let captured = self.capture_interval();
        let metrics = self.capture_metrics();
        let limits = self.capture_limits();
        let has_capture = self.has_capture();
        let activity = matches!(self.section, Section::Activity(_));
        let columns = admin_columns(self.section);
        let stats = if activity && self.snapshot.is_some() {
            metric_pairs(&metrics)
        } else {
            Vec::new()
        };
        let updated = if has_capture {
            updated_at(&captured)
        } else {
            None
        };
        // Caveats and scope notes live behind one info icon, not above the work.
        let mut caveats = Vec::new();
        if has_capture || self.section == Section::Audit {
            caveats.push(captured.clone());
        }
        if !activity {
            caveats.push(metrics.clone());
        }
        if !limits.is_empty() {
            caveats.push(limits.clone());
        }
        let caveat = caveats.join("\n");
        let parsed = chrome::parse_details(&details);
        let primary = self.primary_index();
        let primary_in_header = count > 0 || has_capture;
        let header_actions = primary_in_header
            .then(|| self.button_as(primary, Look::Accent, cx))
            .into_iter()
            .chain(self.enabled(Action::Cancel).then(|| self.button(2, cx)))
            .chain([self.button(21, cx), self.button(22, cx)])
            .collect::<Vec<_>>();
        let capture_row = has_capture || self.enabled(Action::Clear);
        let row_actions = [15, 16, 3]
            .into_iter()
            .filter(|index| self.enabled(ACTIONS[*index].0))
            .map(|index| self.button(index, cx))
            .collect::<Vec<_>>();
        let rows = uniform_list(
            "admin-rows",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                let focused = this.list.is_focused(window);
                let columns = admin_columns(this.section);
                range
                    .map(|index| {
                        let label = this.row_label(index).unwrap_or_default();
                        let selected = Some(index) == this.selected_index();
                        let cells = admin_cells(this.section, &label, columns.len())
                            .into_iter()
                            .zip(columns.iter())
                            .map(|(value, column)| crate::ui::data_cell(column, value))
                            .collect::<Vec<_>>();
                        crate::ui::data_table_row(("admin-row", index), cells, selected, focused)
                            .role(Role::ListBoxOption)
                            .aria_label(label)
                            .aria_selected(selected)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select(index);
                                this.detail_scroll.set_offset(gpui::point(px(0.), px(0.)));
                                window.focus(&this.list, cx);
                                cx.notify();
                            }))
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll);
        let rows = if count == 0 {
            rows.h(px(0.))
        } else {
            rows.flex_1()
        };
        div()
            .id("administration-tool")
            .role(Role::Group)
            .aria_label(format!("PostgreSQL administration for {connection}"))
            .track_focus(&self.root)
            .size_full()
            .flex()
            .flex_col()
            .bg(style::bg())
            .text_color(style::text())
            .text_size(px(style::FONT))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.filter.focus_handle(cx).is_focused(window)
                    && this
                        .filter
                        .update(cx, |field, cx| field.composing(window, cx))
                {
                    return;
                }
                let modifiers = event.keystroke.modifiers;
                if modifiers.control || modifiers.alt || modifiers.platform {
                    return;
                }
                if event.keystroke.key == "tab" {
                    let order = this.focus_order(cx);
                    let current = order
                        .iter()
                        .position(|focus| focus.contains_focused(window, cx));
                    let next = if modifiers.shift {
                        current.map_or(order.len() - 1, |i| (i + order.len() - 1) % order.len())
                    } else {
                        current.map_or(0, |i| (i + 1) % order.len())
                    };
                    window.focus(&order[next], cx);
                    cx.stop_propagation();
                    return;
                }
                if (this.details.is_focused(window) || this.control_details.is_focused(window))
                    && event.keystroke.key == "escape"
                {
                    window.focus(&this.list, cx);
                    cx.stop_propagation();
                    return;
                }
                if this.details.is_focused(window) || this.control_details.is_focused(window) {
                    let scroll = if this.control_details.is_focused(window) {
                        &this.control_scroll
                    } else {
                        &this.detail_scroll
                    };
                    let mut offset = scroll.offset();
                    let bottom = -scroll.max_offset().y;
                    offset.y = match event.keystroke.key.as_str() {
                        "up" => offset.y + px(28.),
                        "down" => offset.y - px(28.),
                        "pageup" => offset.y + px(160.),
                        "pagedown" => offset.y - px(160.),
                        "home" => px(0.),
                        "end" => bottom,
                        _ => return,
                    }
                    .max(bottom)
                    .min(px(0.));
                    scroll.set_offset(offset);
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                if !this.list.is_focused(window) {
                    return;
                }
                let count = this.count();
                let mut selected = this.selected_index().unwrap_or(0);
                match event.keystroke.key.as_str() {
                    "up" => selected = selected.saturating_sub(1),
                    "down" => {
                        selected = this
                            .selected_index()
                            .map_or(0, |i| (i + 1).min(count.saturating_sub(1)))
                    }
                    "home" => selected = 0,
                    "end" => selected = count.saturating_sub(1),
                    "pageup" => selected = selected.saturating_sub(20),
                    "pagedown" => selected = (selected + 20).min(count.saturating_sub(1)),
                    "enter" if this.selected_index().is_some() => {
                        window.focus(&this.details, cx);
                        cx.stop_propagation();
                        return;
                    }
                    _ => return,
                }
                this.select(selected);
                this.scroll
                    .scroll_to_item(selected, gpui::ScrollStrategy::Top);
                this.detail_scroll.set_offset(gpui::point(px(0.), px(0.)));
                cx.notify();
                cx.stop_propagation();
            }))
            .child(crate::ui::tool_header(
                "Administration",
                chrome::connection_context(
                    self.tool_connection.as_ref(),
                    self.connection.is_some(),
                ),
                header_actions,
            ))
            .when(capture_row, |view| {
                view.child(
                    div()
                        .id("admin-capture")
                        .flex_none()
                        .min_h(px(style::TOOLBAR))
                        .px(px(style::INSET_CONTENT))
                        .py(px(style::S2))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(px(style::S6))
                        .border_b_1()
                        .border_color(style::line_soft())
                        .text_sm()
                        .children(stats.into_iter().map(|(label, value)| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(style::S2))
                                .whitespace_nowrap()
                                .child(div().text_color(style::dim()).child(label))
                                .child(div().text_color(style::text()).child(value))
                        }))
                        .child(crate::ui::grow())
                        .when(self.capture_stale(), |row| {
                            row.child(
                                div()
                                    .id("admin-stale")
                                    .role(Role::Status)
                                    .aria_label("Retained readings may be stale")
                                    .text_color(style::warn())
                                    .child("May be stale"),
                            )
                        })
                        .when_some(updated, |row, raw| {
                            row.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(style::S2))
                                    .text_color(style::dim())
                                    .child("Updated")
                                    .child(chrome::timestamp("admin-updated", &raw)),
                            )
                        })
                        .when(!caveat.is_empty(), |row| {
                            row.child(crate::ui::info_hint("admin-capture-notes", caveat.clone()))
                        })
                        .when(self.enabled(Action::Clear), |row| {
                            row.child(self.button(4, cx))
                        }),
                )
            })
            .child(
                div()
                    .flex_none()
                    .px(px(style::INSET_CONTENT))
                    .py(px(style::S3))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(style::S4))
                    .child(
                        crate::ui::segment_group()
                            .id("admin-sections")
                            .role(Role::TabList)
                            .aria_label("Administration sections")
                            .children((5..12).map(|index| self.button(index, cx))),
                    )
                    .when(
                        self.section == Section::Server(ServerSection::Settings),
                        |bar| {
                            let focused = self.filter.focus_handle(cx).contains_focused(window, cx);
                            bar.child(
                                crate::ui::input_well(false, focused)
                                    .flex_1()
                                    .min_w(px(160.))
                                    .max_w(px(360.))
                                    .child(div().flex_1().min_w_0().child(self.filter.clone())),
                            )
                            .child(self.button(12, cx))
                            .child(self.button(13, cx))
                        },
                    )
                    .when(self.section == Section::Audit, |bar| {
                        bar.child(crate::ui::grow()).child(self.button(14, cx))
                    }),
            )
            .child(
                div()
                    .id("admin-list")
                    .role(Role::ListBox)
                    .aria_label(format!(
                        "{} captured {}; arrows select, Enter inspects",
                        count,
                        self.section.label()
                    ))
                    .aria_value(details.clone())
                    .track_focus(&self.list)
                    .tab_stop(true)
                    .tab_index(0)
                    .flex_1()
                    .min_h_0()
                    .px(px(style::S3))
                    .flex()
                    .flex_col()
                    .when(count == 0, |list| list.child(self.empty_table(cx)))
                    .when(count > 0, |list| {
                        list.child(crate::ui::data_table_header(&columns))
                    })
                    .child(rows),
            )
            .when(selected != usize::MAX, |view| {
                view.child(
                    div()
                        .id("admin-selected-details")
                        .role(Role::Group)
                        .aria_label(format!(
                            "Selected {} row {}; captured details",
                            self.section.label(),
                            selected + 1
                        ))
                        .aria_value(details.clone())
                        .track_focus(&self.details)
                        .tab_index(0)
                        .tab_stop(true)
                        .flex_none()
                        .max_h(px(DETAILS_HEIGHT))
                        .overflow_y_scroll()
                        .track_scroll(&self.detail_scroll)
                        .px(px(style::INSET_CONTENT))
                        .pb(px(style::S4))
                        .border_t_1()
                        .border_color(style::line())
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .h(px(style::TOOLBAR))
                                .flex()
                                .items_center()
                                .gap(px(style::S2))
                                .child(crate::ui::heading("Details"))
                                .child(crate::ui::grow())
                                .children(row_actions),
                        )
                        .children(chrome::detail_rows("admin-detail", &parsed)),
                )
            })
            .child(self.control_panel(cx))
            .child(
                crate::ui::status_line()
                    .id("admin-status")
                    .role(Role::Status)
                    .aria_label(self.status.clone())
                    .child(self.status.clone()),
            )
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn sessions_table_has_bounded_tracks_and_a_flexible_query() {
        let columns = admin_columns(Section::Activity(ActivitySection::Sessions));
        let labels = columns
            .iter()
            .map(|column| column.label.to_string())
            .collect::<Vec<_>>();
        assert_eq!(labels, ["PID", "User", "State", "Wait", "Query"]);
        assert_eq!(columns[0].track, crate::ui::Track::Fixed(72.));
        assert!(columns[0].end && columns[0].mono);
        assert!(
            columns[1..4]
                .iter()
                .all(|column| matches!(column.track, crate::ui::Track::Fixed(_)))
        );
        assert_eq!(columns[4].track, crate::ui::Track::Flex);
        assert!(columns[4].mono);
    }

    #[test]
    fn row_cells_keep_positions_and_put_sql_on_one_line() {
        let section = Section::Activity(ActivitySection::Sessions);
        let cells = admin_cells(
            section,
            "12207 · dbunk · idle · Client / ClientRead ·           SELECT c.reltuples           FROM pg_class c",
            5,
        );
        assert_eq!(cells[0].as_deref(), Some("12207"));
        assert_eq!(cells[3].as_deref(), Some("Client / ClientRead"));
        assert_eq!(
            cells[4].as_deref(),
            Some("SELECT c.reltuples FROM pg_class c")
        );
        let background = admin_cells(section, "61 · NULL · NULL · NULL / NULL · ", 5);
        assert_eq!(background[0].as_deref(), Some("61"));
        assert!(background[1..].iter().all(Option::is_none));
    }

    #[test]
    fn metrics_read_as_labelled_values_with_readable_sizes() {
        let pairs = metric_pairs(
            "Active sessions: 1 · Blocked locks (cluster): 0 · Cache hit: 99.9% · Database size: 9694355 bytes",
        );
        assert_eq!(
            pairs,
            vec![
                ("Active sessions".to_owned(), "1".to_owned()),
                ("Blocked locks".to_owned(), "0".to_owned()),
                ("Cache hit".to_owned(), "99.9%".to_owned()),
                ("Database size".to_owned(), "9.2 MB".to_owned()),
            ]
        );
        assert_eq!(readable_bytes(512), "512 B");
        assert_eq!(readable_bytes(2048), "2.0 KB");
    }

    #[test]
    fn updated_time_is_the_end_of_the_collection_interval() {
        assert_eq!(
            updated_at(
                "Database d · reader PID 1 · collected 2026-11-21T19:02:37.289433+00:00 to 2026-11-21T19:02:37.306748+00:00. Readings may change during collection"
            )
            .as_deref(),
            Some("2026-11-21T19:02:37.306748+00:00")
        );
        assert_eq!(updated_at("No readings collected for this section"), None);
    }
}
