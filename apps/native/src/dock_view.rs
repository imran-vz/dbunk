//! Global console Dock: hidden by default, toggled by Ctrl-` or the status-bar
//! badge, never opened by new events. Severity filter, follow and clear match
//! the baseline dock.
use crate::console_model::{Console, Entry, Severity};
use crate::{style, ui};
use gpui::{
    Context, EventEmitter, FocusHandle, Focusable, Role, SharedString, UniformListScrollHandle,
    Window, div, prelude::*, px, uniform_list,
};

/// Height of the open console.
const HEIGHT: f32 = 200.;
/// Column widths of a console row.
const CLOCK_WIDTH: f32 = 56.;
const SEVERITY_WIDTH: f32 = 64.;
const SOURCE_WIDTH: f32 = 72.;

/// `HH:MM:SS` (UTC) of a Unix time in seconds.
fn clock(seconds: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// One rendered console line.
#[derive(Clone)]
struct ConsoleRow {
    clock: SharedString,
    source: SharedString,
    message: SharedString,
    /// AX name: the full line plus its detail.
    label: SharedString,
    severity: Severity,
}

pub struct DockClosed;
#[derive(Clone, Copy, PartialEq)]
enum Action {
    Filter(Option<Severity>),
    Follow,
    Clear,
    Close,
}
const ACTIONS: [(&str, Action); 7] = [
    ("All", Action::Filter(None)),
    ("Info", Action::Filter(Some(Severity::Info))),
    ("Warnings", Action::Filter(Some(Severity::Warning))),
    ("Errors", Action::Filter(Some(Severity::Error))),
    ("Follow", Action::Follow),
    ("Clear", Action::Clear),
    ("Close", Action::Close),
];

pub struct DockView {
    console: Console,
    filter: Option<Severity>,
    follow: bool,
    /// Stable per-control handles; labels never key focus.
    buttons: Vec<FocusHandle>,
    list: FocusHandle,
    scroll: UniformListScrollHandle,
}
impl EventEmitter<DockClosed> for DockView {}
impl DockView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            console: Console::default(),
            filter: None,
            follow: true,
            buttons: ACTIONS.iter().map(|_| cx.focus_handle()).collect(),
            list: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
        }
    }
    pub fn is_open(&self) -> bool {
        self.console.open
    }
    pub fn unread(&self) -> usize {
        self.console.unread
    }
    pub fn append(&mut self, entry: Entry, cx: &mut Context<Self>) {
        self.console.append(entry);
        if self.console.open && self.follow {
            let count = self.console.visible(self.filter).len();
            self.scroll
                .scroll_to_item(count.saturating_sub(1), gpui::ScrollStrategy::Bottom);
        }
        cx.notify();
    }
    pub fn set_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.console.set_open(open);
        if open {
            window.focus(&self.list, cx);
        }
        cx.notify();
    }
    fn activate(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::Filter(filter) => self.filter = filter,
            Action::Follow => self.follow = !self.follow,
            Action::Clear => self.console.clear(),
            Action::Close => {
                self.console.set_open(false);
                cx.emit(DockClosed);
            }
        }
        cx.notify();
    }
}
impl Focusable for DockView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.list.clone()
    }
}
impl Render for DockView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<ConsoleRow> = self
            .console
            .visible(self.filter)
            .into_iter()
            .map(|event| {
                let seconds = event
                    .at
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |at| at.as_secs());
                let clock = clock(seconds);
                let entry = &event.entry;
                let text = format!(
                    "{clock} UTC  {}  {}  {}",
                    entry.severity.label(),
                    entry.source.label(),
                    entry.message
                );
                let label = match &entry.detail {
                    Some(detail) => format!("{text}. {detail}"),
                    None => text,
                };
                ConsoleRow {
                    clock: clock.into(),
                    source: entry.source.label().into(),
                    message: entry.message.clone().into(),
                    label: label.into(),
                    severity: entry.severity,
                }
            })
            .collect();
        let count = rows.len();
        let control = |index: usize, element: gpui::Stateful<gpui::Div>, cx: &mut Context<Self>| {
            let action = ACTIONS[index].1;
            let weak = cx.entity().downgrade();
            element
                .track_focus(&self.buttons[index])
                .tab_index(0)
                .on_click(cx.listener(move |this, _, _, cx| this.activate(action, cx)))
                .on_a11y_action(gpui::accesskit::Action::Click, move |_, _, cx| {
                    weak.update(cx, |this, cx| this.activate(action, cx)).ok();
                })
        };
        // Severity is one exclusive choice: a segment group (D6).
        let mut filters = ui::segment_group();
        for (index, (label, action)) in ACTIONS.iter().enumerate() {
            let Action::Filter(filter) = *action else {
                continue;
            };
            let selected = self.filter == filter;
            filters = filters.child(control(
                index,
                ui::segment(("dock-action", index), *label, selected, true).aria_selected(selected),
                cx,
            ));
        }
        let index_of = |wanted: Action| {
            ACTIONS
                .iter()
                .position(|(_, action)| *action == wanted)
                .unwrap_or_default()
        };
        // Follow is on/off: a checkbox row, not a pressed chip (D6).
        let follow = index_of(Action::Follow);
        let follow = control(
            follow,
            div()
                .id(("dock-action", follow))
                .role(Role::CheckBox)
                .aria_label("Follow")
                .aria_toggled(self.follow.into())
                .h(px(style::CONTROL))
                .px(px(style::S3))
                .flex()
                .items_center()
                .gap(px(style::S3))
                .rounded(px(style::R_SM))
                .text_color(style::dim())
                .cursor_pointer()
                .hover(|s| s.bg(style::hover()).text_color(style::text()))
                .focus(|s| s.shadow(vec![ui::ring()]))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(ui::check_box(self.follow)),
                )
                .child("Follow"),
            cx,
        );
        let clear = index_of(Action::Clear);
        let clear = control(
            clear,
            ui::tool_button(
                ("dock-action", clear),
                "Clear",
                Some("icons/eraser.svg"),
                true,
                false,
            ),
            cx,
        );
        let close = index_of(Action::Close);
        let close = control(
            close,
            ui::icon_button(("dock-action", close), "Close", "icons/close.svg", true),
            cx,
        );
        div()
            .id("console-dock")
            .role(Role::Group)
            .aria_label("Console")
            .h(px(HEIGHT))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(style::line())
            .bg(style::bg())
            .child(
                div()
                    .flex_none()
                    .h(px(style::TOOLBAR))
                    .px(px(style::INSET_BAR))
                    .flex()
                    .items_center()
                    .gap(px(style::S2))
                    .bg(style::panel())
                    .border_b_1()
                    .border_color(style::line_soft())
                    .text_sm()
                    .whitespace_nowrap()
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(style::S3))
                            .pr(px(style::S4))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(style::text())
                            .child(
                                gpui::svg()
                                    .path("icons/terminal.svg")
                                    .size(px(style::ICON))
                                    .text_color(style::dim()),
                            )
                            .child("Console")
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::NORMAL)
                                    .text_color(style::dim())
                                    .child(count.to_string()),
                            ),
                    )
                    .child(filters)
                    .child(follow)
                    .child(div().flex_1())
                    .child(clear)
                    .child(close),
            )
            .child(
                div()
                    .id("console-events")
                    .role(Role::List)
                    .aria_label(format!("{count} console events"))
                    .track_focus(&self.list)
                    .tab_index(0)
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "console-rows",
                            count,
                            cx.processor(move |_, range: std::ops::Range<usize>, _, _| {
                                range
                                    .map(|index| {
                                        let row = rows[index].clone();
                                        let tone = match row.severity {
                                            Severity::Info => style::dim(),
                                            Severity::Warning => style::warn(),
                                            Severity::Error => style::bad_text(),
                                        };
                                        // Columns: time, severity, source,
                                        // message (S0-6).
                                        div()
                                            .id(("console-row", index))
                                            .role(Role::ListItem)
                                            .aria_label(row.label)
                                            .w_full()
                                            .h(px(style::ROW))
                                            .px(px(style::INSET_BAR))
                                            .flex()
                                            .items_center()
                                            .gap(px(style::S5))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_size(px(style::FONT_SMALL))
                                            .border_b_1()
                                            .border_color(style::line_soft())
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .w(px(CLOCK_WIDTH))
                                                    .font_family(style::MONO)
                                                    .text_color(style::dim())
                                                    .child(row.clock),
                                            )
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .w(px(SEVERITY_WIDTH))
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(style::S2))
                                                    .text_color(tone)
                                                    .child(
                                                        div()
                                                            .flex_none()
                                                            .size(px(6.))
                                                            .rounded_full()
                                                            .bg(tone),
                                                    )
                                                    .child(row.severity.label()),
                                            )
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .w(px(SOURCE_WIDTH))
                                                    .text_color(style::dim())
                                                    .child(row.source),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .text_color(match row.severity {
                                                        Severity::Info => style::text(),
                                                        _ => tone,
                                                    })
                                                    .child(row.message),
                                            )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_reads_hours_minutes_seconds_of_the_day() {
        assert_eq!(clock(0), "00:00:00");
        assert_eq!(clock(3_661), "01:01:01");
        // Wraps at midnight.
        assert_eq!(clock(86_400 + 59), "00:00:59");
    }

    #[test]
    fn every_dock_action_has_a_stable_index() {
        for wanted in [Action::Follow, Action::Clear, Action::Close] {
            assert_eq!(
                ACTIONS
                    .iter()
                    .filter(|(_, action)| *action == wanted)
                    .count(),
                1
            );
        }
    }
}
