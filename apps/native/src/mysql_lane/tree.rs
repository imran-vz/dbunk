//! The MySQL sidebar tree: a virtualized list over the lane's flattened rows.
//! It only renders and reports intent; the lane owns loading.
use super::model::{Group, ObjectRef, Row, RowKind};
use crate::style;
use dbunk_lib::backend::mysql_sessions::MySqlObjectKind;
use gpui::{
    Context, EventEmitter, FocusHandle, KeyDownEvent, Role, SharedString, UniformListScrollHandle,
    Window, div, prelude::*, px, svg, uniform_list,
};

#[derive(Clone, Debug)]
pub enum TreeEvent {
    ToggleDatabase(String),
    ToggleGroup(String, Group),
    /// Rows for tables and views, the definition for other objects.
    Open(ObjectRef),
    Structure(ObjectRef),
    NewQuery(Option<String>),
    Refresh,
}

pub struct MySqlTreeView {
    rows: Vec<Row>,
    connected: bool,
    selected: usize,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
}

impl EventEmitter<TreeEvent> for MySqlTreeView {}

impl MySqlTreeView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            rows: Vec::new(),
            connected: false,
            selected: 0,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
        }
    }

    pub fn set_rows(&mut self, rows: Vec<Row>, connected: bool, cx: &mut Context<Self>) {
        self.rows = rows;
        self.connected = connected;
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        cx.notify();
    }

    fn activate(&mut self, position: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(position) else {
            return;
        };
        let event = match &row.kind {
            RowKind::Database { .. } => TreeEvent::ToggleDatabase(row.label.clone()),
            RowKind::Group {
                database, group, ..
            } => TreeEvent::ToggleGroup(database.clone(), *group),
            RowKind::Object(object) => TreeEvent::Open(object.clone()),
            RowKind::Note { .. } => return,
        };
        cx.emit(event);
    }

    fn database_of(&self, position: usize) -> Option<String> {
        match &self.rows.get(position)?.kind {
            RowKind::Database { .. } => Some(self.rows[position].label.clone()),
            RowKind::Group { database, .. } => Some(database.clone()),
            RowKind::Object(object) => Some(object.database.clone()),
            RowKind::Note { .. } => None,
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let last = self.rows.len().saturating_sub(1);
        let expanded = |row: &Row| match row.kind {
            RowKind::Database { expanded } | RowKind::Group { expanded, .. } => Some(expanded),
            _ => None,
        };
        match event.keystroke.key.as_str() {
            "down" => self.selected = (self.selected + 1).min(last),
            "up" => self.selected = self.selected.saturating_sub(1),
            "enter" => self.activate(self.selected, cx),
            "right" if self.rows.get(self.selected).and_then(expanded) == Some(false) => {
                self.activate(self.selected, cx)
            }
            "left" if self.rows.get(self.selected).and_then(expanded) == Some(true) => {
                self.activate(self.selected, cx)
            }
            _ => return,
        }
        self.scroll
            .scroll_to_item(self.selected, gpui::ScrollStrategy::Center);
        window.focus(&self.focus, cx);
        cx.stop_propagation();
        cx.notify();
    }

    fn header_button(
        &self,
        id: &'static str,
        label: &'static str,
        path: &'static str,
        event: TreeEvent,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .tooltip(crate::ui::tooltip(label))
            .size(px(style::TOOL))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| s.bg(style::hover()))
            .child(
                svg()
                    .path(path)
                    .size(px(style::ICON))
                    .text_color(style::dim()),
            )
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(event.clone())))
    }
}

fn icon(row: &Row) -> (&'static str, gpui::Rgba) {
    use crate::style::TreeKind as T;
    style::kind_icon(match &row.kind {
        RowKind::Database { .. } => T::Database,
        RowKind::Group { .. } => T::Folder,
        RowKind::Note { error: true, .. } => T::Warning,
        RowKind::Note { .. } => T::Info,
        RowKind::Object(object) => match object.kind {
            MySqlObjectKind::Table => T::Table,
            MySqlObjectKind::View => T::View,
            MySqlObjectKind::Procedure | MySqlObjectKind::Function => T::Routine,
            MySqlObjectKind::Event => T::Event,
            MySqlObjectKind::Trigger => T::Trigger,
        },
    })
}

fn row_label(row: &Row) -> String {
    match &row.kind {
        RowKind::Database { expanded } => format!(
            "database {}, {}",
            row.label,
            if *expanded { "expanded" } else { "collapsed" }
        ),
        RowKind::Group { expanded, .. } => format!(
            "{}, {} items, {}",
            row.label,
            row.count.unwrap_or(0),
            if *expanded { "expanded" } else { "collapsed" }
        ),
        RowKind::Object(object) => format!(
            "{} {}{}",
            super::kind_label(object.kind),
            object.name,
            row.detail
                .as_ref()
                .map(|detail| format!(", {detail}"))
                .unwrap_or_default()
        ),
        RowKind::Note { text, .. } => text.clone(),
    }
}

impl Render for MySqlTreeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_database = self.database_of(self.selected);
        div()
            .id("mysql-tree")
            .role(Role::Group)
            .aria_label("MySQL databases and objects")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .h(px(26.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .pl(px(10.))
                    .pr(px(6.))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(style::text())
                            .child("Objects"),
                    )
                    .when(self.connected, |header| {
                        header
                            .child(self.header_button(
                                "mysql-tree-query",
                                "New query",
                                "icons/terminal.svg",
                                TreeEvent::NewQuery(selected_database.clone()),
                                cx,
                            ))
                            .child(self.header_button(
                                "mysql-tree-refresh",
                                "Refresh objects",
                                "icons/rotate_cw.svg",
                                TreeEvent::Refresh,
                                cx,
                            ))
                    }),
            )
            .when(!self.connected, |root| {
                root.child(
                    div()
                        .px(px(10.))
                        .text_size(px(style::FONT_SMALL))
                        .text_color(style::faint())
                        .child("Connect to browse databases"),
                )
            })
            .child(
                div()
                    .id("mysql-tree-rows")
                    .role(Role::Tree)
                    .aria_label(format!(
                        "{} rows; arrows move, Right expands, Left collapses, Enter opens",
                        self.rows.len()
                    ))
                    .track_focus(&self.focus)
                    .tab_stop(true)
                    .tab_index(0)
                    .flex_1()
                    .min_h_0()
                    .on_key_down(cx.listener(Self::key_down))
                    .child(
                        uniform_list(
                            "mysql-tree-list",
                            self.rows.len(),
                            cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                                let focused = this.focus.is_focused(window);
                                range
                                    .map(|position| {
                                        let row = &this.rows[position];
                                        let expanded = match row.kind {
                                            RowKind::Database { expanded }
                                            | RowKind::Group { expanded, .. } => Some(expanded),
                                            _ => None,
                                        };
                                        let structure = match &row.kind {
                                            RowKind::Object(object) if object.has_rows() => {
                                                Some(object.clone())
                                            }
                                            _ => None,
                                        };
                                        let (path, color) = icon(row);
                                        let selected = position == this.selected;
                                        let group =
                                            SharedString::from(format!("mysql-row-{position}"));
                                        div()
                                            .id(("mysql-tree-row", position))
                                            .group(group.clone())
                                            .role(Role::TreeItem)
                                            .aria_label(row_label(row))
                                            .aria_selected(selected)
                                            .h(px(style::ROW))
                                            .flex()
                                            .items_center()
                                            .gap(px(5.))
                                            .pl(px(8. + 12. * row.depth as f32))
                                            .pr(px(6.))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_color(match row.kind {
                                                RowKind::Note { error: true, .. } => style::warn(),
                                                RowKind::Note { .. } => style::faint(),
                                                _ => style::text(),
                                            })
                                            .hover(|s| s.bg(style::hover()))
                                            .when(selected, |row| {
                                                row.bg(if focused {
                                                    style::select()
                                                } else {
                                                    style::raised()
                                                })
                                            })
                                            .child(match expanded {
                                                Some(open) => svg()
                                                    .path(if open {
                                                        "icons/chevron_down.svg"
                                                    } else {
                                                        "icons/chevron_right.svg"
                                                    })
                                                    .size(px(style::ICON))
                                                    .flex_none()
                                                    .text_color(style::faint())
                                                    .into_any_element(),
                                                None => div()
                                                    .w(px(style::ICON))
                                                    .flex_none()
                                                    .into_any_element(),
                                            })
                                            .child(
                                                svg()
                                                    .path(path)
                                                    .size(px(style::ICON))
                                                    .flex_none()
                                                    .text_color(color),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .child(SharedString::from(row.label.clone())),
                                            )
                                            .when_some(row.detail.clone(), |row, detail| {
                                                row.child(
                                                    div()
                                                        .flex_none()
                                                        .text_size(px(style::FONT_SMALL))
                                                        .text_color(style::faint())
                                                        .child(detail),
                                                )
                                            })
                                            .when_some(row.count, |row, count| {
                                                row.child(
                                                    div()
                                                        .flex_none()
                                                        .text_size(px(style::FONT_SMALL))
                                                        .text_color(style::faint())
                                                        .child(count.to_string()),
                                                )
                                            })
                                            .when_some(structure, |row, object| {
                                                row.child(
                                                    div()
                                                        .id(("mysql-tree-structure", position))
                                                        .role(Role::Button)
                                                        .aria_label(format!(
                                                            "Structure of {}",
                                                            object.name
                                                        ))
                                                        .tooltip(crate::ui::tooltip("Structure"))
                                                        .flex_none()
                                                        .size(px(16.))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .rounded(px(3.))
                                                        .opacity(0.)
                                                        .group_hover(group, |s| s.opacity(1.))
                                                        .hover(|s| s.bg(style::raised()))
                                                        .child(
                                                            svg()
                                                                .path("icons/list_tree.svg")
                                                                .size(px(style::ICON))
                                                                .text_color(style::dim()),
                                                        )
                                                        .on_click(cx.listener(
                                                            move |_, _, _, cx| {
                                                                cx.stop_propagation();
                                                                cx.emit(TreeEvent::Structure(
                                                                    object.clone(),
                                                                ));
                                                            },
                                                        )),
                                                )
                                            })
                                            .on_click(cx.listener(
                                                move |this, event: &gpui::ClickEvent, window, cx| {
                                                    // Rows may have shrunk since this frame.
                                                    let Some(row) = this.rows.get(position) else {
                                                        return;
                                                    };
                                                    let expandable = matches!(
                                                        row.kind,
                                                        RowKind::Database { .. }
                                                            | RowKind::Group { .. }
                                                    );
                                                    this.selected = position;
                                                    window.focus(&this.focus, cx);
                                                    if expandable || event.click_count() > 1 {
                                                        this.activate(position, cx);
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
