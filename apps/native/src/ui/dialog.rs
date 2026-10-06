//! Plan 032 in-document modals: an occluding backdrop, a titled panel with a
//! scrollable body and an action footer, and the environment notice that
//! frames a write review. Callers own focus, Tab cycling and listeners.
use crate::style;
use dbunk_lib::backend::DevelopmentEnvironment;
use gpui::{AnyElement, Div, ElementId, Role, SharedString, Stateful, div, prelude::*, px};

/// Covers the document, swallows pointer input behind it and centres its
/// child. A dismiss-on-click listener, if any, belongs to the caller.
pub fn backdrop(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .occlude()
        .bg(gpui::rgba(0x00000073))
        .flex()
        .items_center()
        .justify_center()
        .p(px(16.))
}

/// The dialog panel: AX modal dialog named `title`, fixed `width`, at most
/// 80 % of the backdrop's height. Wrap with `ui::appear` at the call site; a
/// caller that installs its own `a11y_synthetic_children` must set modal again.
pub fn modal(id: impl Into<ElementId>, title: impl Into<SharedString>, width: f32) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Dialog)
        .aria_label(title.into())
        .a11y_synthetic_children(|builder| builder.parent_node().set_modal())
        .occlude()
        .w(px(width))
        .max_w_full()
        .max_h(gpui::relative(0.8))
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded(px(8.))
        .border_1()
        .border_color(style::line())
        .bg(style::panel())
        .shadow_lg()
        .text_size(px(style::FONT))
        .text_color(style::text())
}

/// 34 px title row with optional trailing content (an environment chip).
pub fn header(title: impl Into<SharedString>, trailing: Option<AnyElement>) -> Div {
    div()
        .flex_none()
        .h(px(34.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(style::line())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .children(trailing)
}

/// The scrolling middle of a dialog.
pub fn body(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(8.))
        .px(px(12.))
        .py(px(10.))
}

/// 40 px action row, right-aligned.
pub fn footer() -> Div {
    div()
        .flex_none()
        .h(px(40.))
        .px(px(12.))
        .flex()
        .items_center()
        .justify_end()
        .gap(px(6.))
        .border_t_1()
        .border_color(style::line())
}

/// Names the target of a write: a 2 px bar in the environment colour beside
/// `text`. `loud` (typed confirmation) adds the caution wash.
pub fn env_notice(
    environment: Option<DevelopmentEnvironment>,
    text: impl Into<SharedString>,
    loud: bool,
) -> Div {
    let color = style::env(environment);
    div()
        .flex_none()
        .flex()
        .items_stretch()
        .gap(px(8.))
        .rounded(px(5.))
        .overflow_hidden()
        .border_1()
        .border_color(if loud { style::with_alpha(color, 0x73) } else { style::line() })
        .when(loud, |notice| notice.bg(style::warn_fill()))
        .child(div().flex_none().w(px(2.)).bg(style::with_alpha(color, 0xff)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .py(px(5.))
                .pr(px(8.))
                .text_sm()
                .text_color(if loud { style::text() } else { style::dim() })
                .child(text.into()),
        )
}
