//! Global console Dock: hidden by default, toggled by Ctrl-` or the status-bar
//! badge, never opened by new events. Severity filter, follow and clear match
//! the baseline dock.
use crate::console_model::{Console, Entry, Severity};
use gpui::{
    Context, EventEmitter, FocusHandle, Focusable, Role, SharedString, UniformListScrollHandle,
    Window, div, prelude::*, px, uniform_list,
};

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
        let rows: Vec<(SharedString, SharedString, Severity)> = self
            .console
            .visible(self.filter)
            .into_iter()
            .map(|event| {
                let seconds = event
                    .at
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |at| at.as_secs());
                let clock = format!(
                    "{:02}:{:02}:{:02} UTC",
                    seconds / 3600 % 24,
                    seconds / 60 % 60,
                    seconds % 60
                );
                let entry = &event.entry;
                let text = format!(
                    "{clock}  {}  {}  {}",
                    entry.severity.label(),
                    entry.source.label(),
                    entry.message
                );
                let label = match &entry.detail {
                    Some(detail) => format!("{text}. {detail}"),
                    None => text.clone(),
                };
                (text.into(), label.into(), entry.severity)
            })
            .collect();
        let count = rows.len();
        let buttons = ACTIONS
            .iter()
            .enumerate()
            .map(|(index, (label, action))| {
                let action = *action;
                let selected = match action {
                    Action::Filter(filter) => self.filter == filter,
                    Action::Follow => self.follow,
                    _ => false,
                };
                let weak = cx.entity().downgrade();
                crate::ui::segment(("dock-action", index), *label, selected, true)
                    .when(
                        matches!(action, Action::Filter(_) | Action::Follow),
                        |button| button.aria_selected(selected),
                    )
                    .track_focus(&self.buttons[index])
                    .tab_index(0)
                    .on_click(cx.listener(move |this, _, _, cx| this.activate(action, cx)))
                    .on_a11y_action(gpui::accesskit::Action::Click, move |_, _, cx| {
                        weak.update(cx, |this, cx| this.activate(action, cx)).ok();
                    })
            })
            .collect::<Vec<_>>();
        div()
            .id("console-dock")
            .role(Role::Group)
            .aria_label("Console")
            .h(px(200.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(crate::style::line())
            .bg(crate::style::bg())
            .child(
                crate::ui::segmented()
                    .bg(crate::style::panel())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .px_1()
                            .text_color(crate::style::text())
                            .child(
                                gpui::svg()
                                    .path("icons/terminal.svg")
                                    .size(px(crate::style::ICON))
                                    .text_color(crate::style::dim()),
                            )
                            .child("Console")
                            .child(crate::ui::badge(count.to_string())),
                    )
                    .child(crate::ui::separator())
                    .children(buttons),
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
                                        let (text, label, severity) = rows[index].clone();
                                        div()
                                            .id(("console-row", index))
                                            .role(Role::ListItem)
                                            .aria_label(label)
                                            .h(px(crate::style::ROW))
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .font_family(crate::style::MONO)
                                            .text_size(px(crate::style::FONT_SMALL))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_color(match severity {
                                                Severity::Info => crate::style::dim(),
                                                Severity::Warning => crate::style::warn(),
                                                Severity::Error => crate::style::bad(),
                                            })
                                            .child(text)
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
