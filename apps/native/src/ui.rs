//! Plan 031 document kit: toolbars, tool buttons, segmented switchers and
//! footer status lines. Helpers only style; callers keep their own focus
//! handles, tab order, `on_click` and `on_a11y_action` handlers.
use crate::style;
use gpui::{Div, ElementId, Role, SharedString, Stateful, div, prelude::*, px, svg};

/// A 28 px document toolbar. Wraps onto further rows instead of clipping
/// controls when the window is narrow.
pub fn toolbar() -> Div {
    div()
        .flex_none()
        .min_h(px(style::TOOLBAR))
        .py(px(4.))
        .px(px(8.))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(4.))
        .border_b_1()
        .border_color(style::line_soft())
        .text_sm()
        .text_color(style::dim())
}

/// A 20 px toolbar button labelled with `label` (also its AX name). Disabled
/// buttons render faint and expose AX disabled; a caller that installs its own
/// `a11y_synthetic_children` replaces that hook and must set it again.
pub fn tool_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<&'static str>,
    enabled: bool,
    primary: bool,
) -> Stateful<Div> {
    let label = label.into();
    let color = if !enabled {
        style::faint()
    } else if primary {
        style::text()
    } else {
        style::dim()
    };
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .flex_none()
        .h(px(style::TOOL))
        .px(px(6.))
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded(px(4.))
        .border_1()
        .border_color(if primary {
            style::line().into()
        } else {
            gpui::transparent_black()
        })
        .when(primary, |button| button.bg(style::raised()))
        .text_sm()
        .whitespace_nowrap()
        .text_color(color)
        .when(enabled, |button| {
            button
                .cursor_pointer()
                .hover(|s| s.bg(style::hover()).text_color(style::text()))
        })
        .focus(|s| {
            s.bg(style::hover())
                .text_color(style::text())
                .border_color(style::accent())
        })
        .when_some(icon, |button, path| {
            button.child(
                svg()
                    .path(path)
                    .size(px(style::ICON))
                    .flex_none()
                    .text_color(color),
            )
        })
        .child(label)
        .when(!enabled, |button| {
            button.a11y_synthetic_children(|builder| builder.parent_node().set_disabled())
        })
}

/// Raised background and text colour for a pressed toggle or selected tab.
pub fn pressed(button: Stateful<Div>, selected: bool) -> Stateful<Div> {
    button.when(selected, |button| {
        button
            .bg(style::raised())
            .border_color(style::line())
            .text_color(style::text())
    })
}

/// A 24 px row of small tabs (results, inspector views). Children are
/// `segment`s, optionally followed by `grow()` and trailing metadata.
pub fn segmented() -> Div {
    div()
        .flex_none()
        .min_h(px(style::FOOTER))
        .px(px(6.))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(2.))
        .border_b_1()
        .border_color(style::line_soft())
        .text_sm()
        .text_color(style::dim())
}

/// One 18 px tab of a `segmented` row.
pub fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    enabled: bool,
) -> Stateful<Div> {
    pressed(tool_button(id, label, None, enabled, false), selected)
        .h(px(18.))
        .rounded(px(3.))
}

pub fn separator() -> Div {
    div()
        .flex_none()
        .w(px(1.))
        .h(px(14.))
        .mx(px(4.))
        .bg(style::line())
}

pub fn grow() -> Div {
    div().flex_1()
}

/// Section heading inside a document (`Columns 8`, `Indexes 3`).
pub fn section_label(label: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_sm()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(style::dim())
        .child(label.into())
}

/// Small monospace count or tag.
pub fn badge(text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .px(px(4.))
        .rounded(px(3.))
        .border_1()
        .border_color(style::line())
        .font_family(style::MONO)
        .text_size(px(style::FONT_SMALL))
        .text_color(style::dim())
        .child(text.into())
}

/// `schema.` faint, `table` bright, both monospace.
pub fn crumbs(prefix: impl Into<SharedString>, leaf: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .font_family(style::MONO)
        .text_color(style::faint())
        .whitespace_nowrap()
        .child(prefix.into())
        .child(div().text_color(style::text()).child(leaf.into()))
}

/// The 24 px document footer: monospace 10 px, faint, bordered on top.
pub fn status_line() -> Div {
    div()
        .flex_none()
        .min_h(px(style::FOOTER))
        .px(px(8.))
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(12.))
        .border_t_1()
        .border_color(style::line())
        .font_family(style::MONO)
        .text_size(px(style::FONT_SMALL))
        .text_color(style::faint())
}

/// Keyboard hint inside a tool button (`⌘↵`); pair with `aria_keyshortcuts`.
pub fn shortcut(keys: &'static str) -> Div {
    div()
        .flex_none()
        .font_family(style::MONO)
        .text_size(px(style::FONT_SMALL))
        .text_color(style::faint())
        .child(keys)
}

/// A bordered inline text field frame for an `AccessibleEditor`.
pub fn field() -> Div {
    div()
        .min_h(px(22.))
        .px(px(6.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .border_1()
        .border_color(style::line())
        .bg(style::bg())
}
