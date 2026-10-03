//! Persistent Navigator schema/object tree for the selected connection. Each
//! explicit Load opens a transient owned data lane, reads one bounded catalog
//! and closes it, so the tree never holds a socket or a document slot at rest.
use crate::{
    accessible_editor::AccessibleEditor,
    catalog::Catalog,
    controller::{Host, TableCommand, TableControls, TableMessage, TableReceiver},
    navigator_model::{self, Key, Move, Row, RowKind, Tree},
};
use dbunk_lib::backend::objects::PgObjectRef;
use editor::{Editor, EditorEvent};
use gpui::{
    Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role,
    SharedString, Subscription, UniformListScrollHandle, Window, div, prelude::*, px, rgb,
    uniform_list,
};
use std::{
    cell::Cell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

const TYPE_AHEAD_RESET: Duration = Duration::from_millis(700);
const FILTER_BYTES: usize = 8192;

pub enum NavigatorEvent {
    OpenTable {
        connection: String,
        schema: String,
        table: String,
    },
    Describe {
        connection: String,
        reference: PgObjectRef,
    },
}
#[derive(Clone, Copy, PartialEq)]
enum Action {
    Load,
    Cancel,
}
#[derive(PartialEq)]
enum Phase {
    Idle,
    Connecting,
    Reading,
    /// The reply settled or was cancelled; the lane is joining.
    Closing,
}
pub struct NavigatorView {
    host: Arc<Host>,
    wake: async_channel::Sender<()>,
    budget: Rc<Cell<usize>>,
    connection: Option<String>,
    lanes: u64,
    controls: Option<TableControls>,
    receiver: Option<TableReceiver>,
    phase: Phase,
    next: u64,
    pending: Option<u64>,
    cancelled: bool,
    catalog: Option<Catalog>,
    stale: bool,
    tree: Tree,
    rows: Vec<Row>,
    selected: usize,
    applied_filter: String,
    filter: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    _filter_events: Subscription,
    root: FocusHandle,
    list: FocusHandle,
    buttons: [FocusHandle; 2],
    scroll: UniformListScrollHandle,
    type_ahead: (String, Instant),
    editable: bool,
    status: String,
}
impl EventEmitter<NavigatorEvent> for NavigatorView {}
impl NavigatorView {
    pub fn new(
        host: Arc<Host>,
        wake: async_channel::Sender<()>,
        budget: Rc<Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| Editor::single_line(window, cx));
        let accessible = cx.new(|cx| {
            AccessibleEditor::field(filter.clone(), "Filter schemas and objects", false, cx)
        });
        let filter_events = cx.subscribe_in(
            &filter,
            window,
            |this, editor, event: &EditorEvent, window, cx| {
                // Composition stays local until committed, like other fields.
                if matches!(event, EditorEvent::BufferEdited)
                    && editor
                        .update(cx, |editor, cx| editor.marked_text_range(window, cx))
                        .is_none()
                {
                    this.apply_filter(cx);
                }
            },
        );
        Self {
            host,
            wake,
            budget,
            connection: None,
            lanes: 0,
            controls: None,
            receiver: None,
            phase: Phase::Idle,
            next: 0,
            pending: None,
            cancelled: false,
            catalog: None,
            stale: false,
            tree: Tree::default(),
            rows: Vec::new(),
            selected: 0,
            applied_filter: String::new(),
            filter,
            accessible,
            _filter_events: filter_events,
            root: cx.focus_handle(),
            list: cx.focus_handle(),
            buttons: [cx.focus_handle(), cx.focus_handle()],
            scroll: UniformListScrollHandle::new(),
            type_ahead: (String::new(), Instant::now()),
            editable: true,
            status: "Select a connection, then Load objects".into(),
        }
    }
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.filter.focus_handle(cx), cx);
    }
    /// The retained capture and its connection, for Open Anything.
    pub fn catalog(&self) -> Option<(&str, &Catalog)> {
        Some((self.connection.as_deref()?, self.catalog.as_ref()?))
    }
    pub fn reveal_schema(&mut self, schema: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.tree.expand_schema(schema);
        self.filter
            .update(cx, |editor, cx| editor.set_text("", window, cx));
        self.applied_filter.clear();
        self.rebuild();
        self.restore_selection(Some(id));
        window.focus(&self.list, cx);
        cx.notify();
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editable = editable;
        self.filter
            .update(cx, |editor, _| editor.set_read_only(!editable));
        cx.notify();
    }
    /// The tree follows the workspace selection. A different connection drops
    /// the retained capture; an in-flight read is cancelled and its lane joins.
    pub fn set_connection(&mut self, connection: Option<String>, cx: &mut Context<Self>) {
        if self.connection == connection {
            return;
        }
        self.stop_lane("Connection changed; catalog read cancelled");
        self.connection = connection;
        self.catalog = None;
        self.tree = Tree::default();
        self.stale = false;
        self.rebuild();
        if self.controls.is_none() {
            self.status = if self.connection.is_some() {
                "Load objects to browse schemas".into()
            } else {
                "Select a connection, then Load objects".into()
            };
        }
        cx.notify();
    }
    /// A write may have changed this connection's catalog; keep the retained
    /// tree visible but disclose it until an explicit reload.
    pub fn mark_stale(&mut self, connection: Option<&str>, cx: &mut Context<Self>) {
        if self.catalog.is_some()
            && connection.is_none_or(|connection| self.connection.as_deref() == Some(connection))
        {
            self.stale = true;
            self.status = "Database may have changed; Load objects to refresh".into();
            cx.notify();
        }
    }
    /// The worker reports Closed through `drain_one`; until then the lane is
    /// owned here and shutdown joins it through the host.
    fn stop_lane(&mut self, status: &str) {
        if let Some(controls) = &self.controls
            && self.phase != Phase::Closing
        {
            controls.cancel();
            controls.stop();
            self.cancelled = true;
            self.phase = Phase::Closing;
            self.status = status.into();
        }
    }
    fn load(&mut self, cx: &mut Context<Self>) {
        if !self.editable || self.controls.is_some() {
            return;
        }
        let Some(connection) = self.connection.clone() else {
            self.status = "Select a connection first".into();
            cx.notify();
            return;
        };
        self.lanes = self.lanes.wrapping_add(1);
        match self.host.open_table_document(
            format!("navigator:{}", self.lanes),
            connection,
            self.wake.clone(),
        ) {
            Ok((controls, receiver)) => {
                self.controls = Some(controls);
                self.receiver = Some(receiver);
                self.phase = Phase::Connecting;
                self.cancelled = false;
                self.status = "Connecting to read objects".into();
            }
            Err(error) => self.status = format!("{error}; retained tree unchanged"),
        }
        cx.notify();
    }
    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.stop_lane("Catalog read cancelled; retained tree unchanged");
        cx.notify();
    }
    pub fn drain_one(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(message) = self
            .receiver
            .as_ref()
            .and_then(|receiver| receiver.try_recv())
        else {
            return false;
        };
        match message.into_message() {
            TableMessage::Opened if self.phase == Phase::Connecting && !self.cancelled => {
                self.next = self.next.wrapping_add(1);
                match self
                    .controls
                    .as_ref()
                    .map(|controls| controls.send(TableCommand::Catalog(self.next)))
                {
                    Some(Ok(())) => {
                        self.pending = Some(self.next);
                        self.phase = Phase::Reading;
                        self.status = "Reading objects".into();
                    }
                    Some(Err(error)) => {
                        self.stop_lane(error);
                    }
                    None => {}
                }
            }
            TableMessage::Catalog(id, result) if self.pending == Some(id) => {
                self.pending = None;
                if !self.cancelled {
                    self.settle(result);
                }
                // One read per lane: release the socket and document slot.
                if let Some(controls) = &self.controls {
                    controls.stop();
                }
                self.phase = Phase::Closing;
            }
            TableMessage::Error(error) => {
                self.status = format!("Catalog read failed: {error}; retained tree unchanged");
                self.stop_lane(&self.status.clone());
            }
            TableMessage::Closed(result) => {
                self.controls = None;
                self.receiver = None;
                self.pending = None;
                self.phase = Phase::Idle;
                if let Err(error) = result {
                    self.status = format!("Catalog lane cleanup failed: {error}");
                }
            }
            // Late or foreign replies never replace the retained capture.
            _ => {}
        }
        cx.notify();
        true
    }
    fn settle<E: std::fmt::Debug>(
        &mut self,
        result: Result<dbunk_lib::backend::objects::PgObjectCatalog, E>,
    ) {
        match result {
            Ok(catalog) => {
                // Release the previous reservation before admitting the next.
                self.catalog = None;
                match Catalog::new(catalog, self.budget.clone()) {
                    Ok(catalog) => {
                        let partial = catalog.truncated.len();
                        self.catalog = Some(catalog);
                        self.stale = false;
                        self.rebuild();
                        self.status = format!(
                            "{} rows shown{}",
                            self.rows.len(),
                            if partial > 0 {
                                format!("; {partial} groups cut at 2,000 on the server")
                            } else {
                                String::new()
                            }
                        );
                    }
                    Err(error) => {
                        self.rebuild();
                        self.status = format!("{error}; tree cleared");
                    }
                }
            }
            Err(error) => {
                self.status = format!("Catalog read failed: {error:?}; retained tree unchanged")
            }
        }
    }
    fn apply_filter(&mut self, cx: &mut Context<Self>) {
        let buffer = self.filter.read(cx).buffer().read(cx);
        if buffer.len(cx).0 > FILTER_BYTES {
            self.status = "Filter exceeds 8 KiB; previous filter preserved".into();
            cx.notify();
            return;
        }
        let selected = self.rows.get(self.selected).map(|row| row.id.clone());
        self.applied_filter = self.filter.read(cx).text(cx);
        self.rebuild();
        self.restore_selection(selected);
        cx.notify();
    }
    fn rebuild(&mut self) {
        self.rows = self
            .catalog
            .as_ref()
            .map(|catalog| navigator_model::rows(catalog, &self.applied_filter, &self.tree))
            .unwrap_or_default();
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }
    fn restore_selection(&mut self, id: Option<String>) {
        self.selected = id
            .and_then(|id| self.rows.iter().position(|row| row.id == id))
            .unwrap_or(0);
        self.scroll
            .scroll_to_item(self.selected, gpui::ScrollStrategy::Center);
    }
    fn toggle(&mut self, index: usize) {
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        self.tree.toggle(&row);
        self.rebuild();
        self.restore_selection(Some(row.id));
    }
    fn activate_row(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        match row.kind {
            RowKind::Schema { .. } | RowKind::Group { .. } => self.toggle(index),
            RowKind::ShowMore { .. } => {
                self.tree.show_more(&row);
                self.rebuild();
                self.restore_selection(Some(row.id));
            }
            RowKind::Object(object) => {
                let (Some(connection), Some(catalog)) = (&self.connection, &self.catalog) else {
                    return;
                };
                let entry = &catalog.rows[object];
                if entry.kind.relation()
                    && let Some(schema) = &entry.schema
                {
                    cx.emit(NavigatorEvent::OpenTable {
                        connection: connection.clone(),
                        schema: schema.clone(),
                        table: entry.entry.name.clone(),
                    });
                } else if let Some(reference) = entry.reference() {
                    cx.emit(NavigatorEvent::Describe {
                        connection: connection.clone(),
                        reference,
                    });
                } else {
                    self.status = format!(
                        "{} entries are listed only; no description is available",
                        entry.kind.label()
                    );
                }
            }
            RowKind::Database | RowKind::Truncated => {}
        }
        cx.notify();
    }
    fn enabled(&self, action: Action) -> bool {
        self.editable
            && match action {
                Action::Load => self.connection.is_some() && self.controls.is_none(),
                Action::Cancel => {
                    self.controls.is_some()
                        && matches!(self.phase, Phase::Connecting | Phase::Reading)
                }
            }
    }
    fn activate(&mut self, action: Action, cx: &mut Context<Self>) {
        if !self.enabled(action) {
            return;
        }
        match action {
            Action::Load => self.load(cx),
            Action::Cancel => self.cancel(cx),
        }
    }
    fn button(
        &self,
        index: usize,
        label: &'static str,
        action: Action,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let weak = cx.entity().downgrade();
        let enabled = self.enabled(action);
        // Stable IDs and focus handles; GPUI's key-up click is the only
        // keyboard activation path, so no key-down handler duplicates it.
        div()
            .id(("navigator-action", index))
            .role(Role::Button)
            .aria_label(label)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .track_focus(&self.buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .focus(|style| style.bg(rgb(0x222222)))
            .text_color(if enabled {
                rgb(0xffffff)
            } else {
                rgb(0x888888)
            })
            .px_2()
            .py_1()
            .text_sm()
            .border_1()
            .border_color(rgb(0x444444))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.activate(action, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, _, cx| {
                weak.update(cx, |this, cx| this.activate(action, cx)).ok();
            })
            .into_any_element()
    }
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = &event.keystroke.modifiers;
        if !self.list.is_focused(window) || modifiers.control || modifiers.alt || modifiers.platform
        {
            return;
        }
        let key = match event.keystroke.key.as_str() {
            "up" => Some(Key::Up),
            "down" => Some(Key::Down),
            "left" => Some(Key::Left),
            "right" => Some(Key::Right),
            "home" => Some(Key::Home),
            "end" => Some(Key::End),
            _ => None,
        };
        if let Some(key) = key {
            match navigator_model::navigate(&self.rows, self.selected, key) {
                Some(Move::Select(index)) => self.selected = index,
                Some(Move::Toggle(index)) => self.toggle(index),
                None => {}
            }
        } else if matches!(event.keystroke.key.as_str(), "enter" | "space") && !modifiers.shift {
            // The list itself is not clickable, so this is its only activation.
            self.activate_row(self.selected, cx);
        } else if let Some(text) = event
            .keystroke
            .key_char
            .as_deref()
            .filter(|text| text.chars().count() == 1 && !text.trim().is_empty())
        {
            let restart = self.type_ahead.1.elapsed() > TYPE_AHEAD_RESET;
            if restart {
                self.type_ahead.0.clear();
            }
            self.type_ahead.0.push_str(text);
            self.type_ahead.1 = Instant::now();
            match navigator_model::type_ahead(
                &self.rows,
                self.selected,
                &self.type_ahead.0,
                self.type_ahead.0.chars().count() == 1,
            ) {
                Some(index) => self.selected = index,
                None => return,
            }
        } else {
            return;
        }
        self.scroll
            .scroll_to_item(self.selected, gpui::ScrollStrategy::Center);
        cx.notify();
        cx.stop_propagation();
    }
    fn row_label(&self, row: &Row) -> String {
        let state = match row.kind {
            RowKind::Schema { expanded } | RowKind::Group { expanded } => {
                if expanded {
                    "expanded"
                } else {
                    "collapsed"
                }
            }
            _ => "",
        };
        let kind = match row.kind {
            RowKind::Object(index) => self
                .catalog
                .as_ref()
                .map(|catalog| catalog.rows[index].kind.label())
                .unwrap_or(""),
            RowKind::Schema { .. } => "Schema",
            _ => "",
        };
        [
            kind.to_owned(),
            row.label.clone(),
            row.count
                .map(|count| format!("{count} objects"))
                .unwrap_or_default(),
            state.to_owned(),
            format!("level {}", row.level),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
    }
}
impl Focusable for NavigatorView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.list.clone()
    }
}
impl Render for NavigatorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_label = self
            .rows
            .get(self.selected)
            .map(|row| self.row_label(row))
            .unwrap_or_default();
        let status = if self.stale && self.controls.is_none() {
            format!("{} (may be stale)", self.status)
        } else {
            self.status.clone()
        };
        div()
            .id("object-navigator")
            .role(Role::Group)
            .aria_label("Schemas and objects")
            .track_focus(&self.root)
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .border_t_1()
            .border_color(rgb(0x444444))
            .on_key_down(cx.listener(Self::key_down))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .child(self.button(
                        0,
                        if self.catalog.is_some() {
                            "Refresh objects"
                        } else {
                            "Load objects"
                        },
                        Action::Load,
                        cx,
                    ))
                    .child(self.button(1, "Cancel", Action::Cancel, cx)),
            )
            .child(div().h(px(26.)).child(self.accessible.clone()))
            .child(
                div()
                    .id("navigator-status")
                    .role(Role::Status)
                    .aria_label(status.clone())
                    .text_xs()
                    .px_2()
                    .text_color(rgb(0xbbbbbb))
                    .child(status),
            )
            .child(
                div()
                    .id("navigator-tree")
                    .role(Role::Tree)
                    .aria_label(format!(
                        "{} navigator rows; arrows move, Right expands, Left collapses, Enter opens",
                        self.rows.len()
                    ))
                    .aria_value(selected_label)
                    .track_focus(&self.list)
                    .tab_stop(true)
                    .tab_index(0)
                    .focus(|style| style.border_1().border_color(rgb(0x666666)))
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "navigator-rows",
                            self.rows.len(),
                            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|position| {
                                        let row = &this.rows[position];
                                        let marker = match row.kind {
                                            RowKind::Schema { expanded: true }
                                            | RowKind::Group { expanded: true } => "▾ ",
                                            RowKind::Schema { expanded: false }
                                            | RowKind::Group { expanded: false } => "▸ ",
                                            _ => "",
                                        };
                                        let text = match row.count {
                                            Some(count) => format!("{marker}{}  {count}", row.label),
                                            None => format!("{marker}{}", row.label),
                                        };
                                        let selected = position == this.selected;
                                        div()
                                            .id(("navigator-row", position))
                                            .role(Role::TreeItem)
                                            .aria_label(this.row_label(row))
                                            .aria_selected(selected)
                                            .h(px(22.))
                                            .pl(px(8. + 12. * (row.level.saturating_sub(1)) as f32))
                                            .pr_2()
                                            .text_sm()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_color(match row.kind {
                                                RowKind::Truncated => rgb(0xfbbf24),
                                                RowKind::Group { .. } | RowKind::Database => {
                                                    rgb(0xbbbbbb)
                                                }
                                                _ => rgb(0xffffff),
                                            })
                                            .when(selected, |row| row.bg(rgb(0x252525)))
                                            .child(SharedString::from(text))
                                            .on_click(cx.listener(
                                                move |this, event: &gpui::ClickEvent, window, cx| {
                                                    // Rows may have shrunk since this frame.
                                                    let Some(row) = this.rows.get(position) else {
                                                        return;
                                                    };
                                                    let expandable = matches!(
                                                        row.kind,
                                                        RowKind::Schema { .. }
                                                            | RowKind::Group { .. }
                                                            | RowKind::ShowMore { .. }
                                                    );
                                                    this.selected = position;
                                                    window.focus(&this.list, cx);
                                                    if event.click_count() > 1 || expandable {
                                                        this.activate_row(position, cx);
                                                    }
                                                    cx.notify();
                                                },
                                            ))
                                    })
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.scroll)
                        .h_full(),
                    ),
            )
    }
}
