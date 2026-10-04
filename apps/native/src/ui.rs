//! Plan 031 document kit: toolbars, tool buttons, segmented switchers and
//! footer status lines. Helpers only style; callers keep their own focus
//! handles, tab order, `on_click` and `on_a11y_action` handlers.
use crate::style;
use gpui::{
    Animation, AnimationElement, AnimationExt, AnyView, App, BoxShadow, Context, Div, ElementId,
    Render, Role, SharedString, Stateful, Window, div, ease_out_quint, hsla, point, prelude::*, px,
    svg,
};
use std::time::Duration;

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
            press(
                button
                    .cursor_pointer()
                    .hover(|s| s.bg(style::hover()).text_color(style::text())),
            )
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

/// Pressed depth for any clickable control: the face darkens and sinks under
/// an inset shadow while the pointer is held, so a click is felt before its
/// result arrives. Layout never moves.
pub fn press(control: Stateful<Div>) -> Stateful<Div> {
    control.active(|s| s.bg(style::pressed()).shadow(vec![sunken()]))
}

fn sunken() -> BoxShadow {
    BoxShadow {
        color: hsla(0., 0., 0., 0.55),
        offset: point(px(0.), px(1.)),
        blur_radius: px(2.),
        spread_radius: px(0.),
        inset: true,
    }
}

/// The resting lift of a raised button: one hairline of shadow underneath.
fn lifted() -> BoxShadow {
    BoxShadow {
        color: hsla(0., 0., 0., 0.45),
        offset: point(px(0.), px(1.)),
        blur_radius: px(0.),
        spread_radius: px(0.),
        inset: false,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    /// The one action a page exists for (Save, Unlock, Continue).
    Primary,
    /// Other actions: raised and bordered.
    Secondary,
    /// Quiet actions (Cancel, Back): no fill until hovered.
    Ghost,
    /// Destructive actions.
    Danger,
}

/// A 24 px form/page button. Like `tool_button`, it only styles: callers own
/// focus, `on_click` and `on_a11y_action`.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    variant: Variant,
    enabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    let (fill, line, text) = match variant {
        Variant::Primary => (
            style::primary_fill(),
            style::primary_line(),
            style::primary_text(),
        ),
        Variant::Secondary => (style::raised(), style::line(), style::text()),
        Variant::Ghost => (
            gpui::transparent_black().into(),
            gpui::transparent_black().into(),
            style::dim(),
        ),
        Variant::Danger => (style::bad_fill(), style::bad_line(), style::bad_text()),
    };
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .flex_none()
        .h(px(24.))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(5.))
        .rounded(px(5.))
        .border_1()
        .border_color(line)
        .bg(fill)
        .text_sm()
        .whitespace_nowrap()
        .text_color(if enabled { text } else { style::faint() })
        .when(variant != Variant::Ghost && enabled, |b| {
            b.shadow(vec![lifted()])
        })
        .when(!enabled, |b| b.opacity(0.55))
        .when(enabled, |b| {
            press(b.cursor_pointer().hover(move |s| match variant {
                Variant::Ghost => s.bg(style::hover()).text_color(style::text()),
                _ => s.border_color(style::faint()),
            }))
        })
        .focus(|s| s.border_color(style::accent()))
        .child(label)
        .when(!enabled, |b| {
            b.a11y_synthetic_children(|builder| builder.parent_node().set_disabled())
        })
}

/// Fades and lifts a page, dialog or panel into place once per `id`. Give
/// each page its own id so switching pages replays the entrance.
pub fn appear<E: Styled + IntoElement + 'static>(
    id: impl Into<ElementId>,
    element: E,
) -> AnimationElement<E> {
    element.with_animation(
        id,
        Animation::new(Duration::from_millis(style::APPEAR_MS)).with_easing(ease_out_quint()),
        |element, t| {
            element
                .opacity(t)
                .relative()
                .top(px((1. - t) * style::APPEAR_RISE))
        },
    )
}

/// Horizontal offset of an error shake at normalized time `t`: three damped
/// swings that start and end at rest.
pub fn shake_offset(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    (t * std::f32::consts::PI * 6.).sin() * style::SHAKE_PX * (1. - t)
}

/// Shakes an error once per `id`. Key the id by an error sequence number so
/// a repeated identical error still draws the eye.
pub fn shake<E: Styled + IntoElement + 'static>(
    id: impl Into<ElementId>,
    element: E,
) -> AnimationElement<E> {
    element.with_animation(
        id,
        Animation::new(Duration::from_millis(style::SHAKE_MS)),
        |element, t| element.relative().left(px(shake_offset(t))),
    )
}

/// A dismissable inline error: red wash, warning icon, AX alert. Wrap with
/// `shake` at the call site.
pub fn error_banner(id: impl Into<ElementId>, message: impl Into<SharedString>) -> Stateful<Div> {
    let message = message.into();
    div()
        .id(id)
        .role(Role::Alert)
        .aria_label(message.clone())
        .flex()
        .items_start()
        .gap(px(6.))
        .px(px(8.))
        .py(px(5.))
        .rounded(px(5.))
        .border_1()
        .border_color(style::bad_line())
        .bg(style::bad_fill())
        .text_sm()
        .text_color(style::bad_text())
        .child(
            svg()
                .path("icons/warning.svg")
                .mt(px(1.))
                .size(px(style::ICON))
                .flex_none()
                .text_color(style::bad_text()),
        )
        .child(div().flex_1().min_w_0().child(message))
}

/// A hover tooltip that fades in; no scale, no slide.
pub struct Tooltip {
    text: SharedString,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(6.))
            .py(px(3.))
            .rounded(px(4.))
            .border_1()
            .border_color(style::line())
            .bg(style::raised())
            .shadow_md()
            .text_size(px(style::FONT))
            .text_color(style::text())
            .child(self.text.clone())
            .with_animation(
                ("tooltip", cx.entity_id().as_u64()),
                Animation::new(Duration::from_millis(style::TOOLTIP_MS)),
                |element, t| element.opacity(t),
            )
    }
}

/// Builder for `.tooltip(...)`; pair with `tooltip_delay()`.
pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();
    move |_, cx| {
        let text = text.clone();
        cx.new(|_| Tooltip { text }).into()
    }
}

pub fn tooltip_delay() -> Duration {
    Duration::from_millis(style::TOOLTIP_DELAY_MS)
}

/// Bordered text-field frame; `error` swaps the border to the danger colour.
pub fn input_frame(error: bool) -> Div {
    div()
        .min_h(px(26.))
        .px(px(7.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .border_1()
        .border_color(if error {
            style::bad_line()
        } else {
            style::line()
        })
        .bg(style::bg())
}

/// Field label over its input, with an optional hint or error underneath.
pub fn labelled(
    label: impl Into<SharedString>,
    input: impl IntoElement,
    note: Option<(SharedString, bool)>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .min_w_0()
        .child(
            div()
                .text_size(px(style::FONT_SMALL))
                .text_color(style::dim())
                .child(label.into()),
        )
        .child(input)
        .when_some(note, |field, (text, error)| {
            field.child(
                div()
                    .text_size(px(style::FONT_SMALL))
                    .text_color(if error {
                        style::bad_text()
                    } else {
                        style::faint()
                    })
                    .child(text),
            )
        })
}

/// A titled group of fields on a page.
pub fn section(title: impl Into<SharedString>) -> Div {
    div().flex().flex_col().gap(px(8.)).child(
        div()
            .pb(px(4.))
            .border_b_1()
            .border_color(style::line_soft())
            .text_size(px(style::FONT_SMALL))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(style::faint())
            .child(title.into().to_uppercase()),
    )
}

/// 12 px checkbox glyph for toggle rows.
pub fn check_box(checked: bool) -> Div {
    div()
        .flex_none()
        .size(px(12.))
        .mt(px(1.))
        .rounded(px(3.))
        .border_1()
        .border_color(if checked {
            style::accent()
        } else {
            style::faint()
        })
        .bg(if checked {
            style::primary_fill()
        } else {
            style::bg()
        })
        .flex()
        .items_center()
        .justify_center()
        .when(checked, |b| {
            b.child(
                svg()
                    .path("icons/check.svg")
                    .size(px(9.))
                    .text_color(style::accent()),
            )
        })
}

/// A selectable card: title, optional badge and body text. Selected cards
/// take the accent border and fill.
pub fn choice_card(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    icon: &'static str,
    badge_text: Option<&'static str>,
    selected: bool,
) -> Stateful<Div> {
    let title = title.into();
    div()
        .id(id)
        .role(Role::RadioButton)
        .aria_label(title.clone())
        .flex()
        .items_start()
        .gap(px(10.))
        .p(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(if selected {
            style::primary_line()
        } else {
            style::line()
        })
        .bg(if selected {
            style::primary_fill()
        } else {
            style::panel()
        })
        .cursor_pointer()
        .hover(|s| s.border_color(style::faint()))
        .focus(|s| s.border_color(style::accent()))
        .child(
            div()
                .flex_none()
                .size(px(26.))
                .rounded(px(5.))
                .border_1()
                .border_color(style::line())
                .bg(style::bg())
                .flex()
                .items_center()
                .justify_center()
                .child(svg().path(icon).size(px(13.)).text_color(if selected {
                    style::accent()
                } else {
                    style::dim()
                })),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(style::text())
                        .child(title)
                        .when_some(badge_text, |row, text| {
                            row.child(
                                div()
                                    .px(px(5.))
                                    .rounded(px(3.))
                                    .bg(style::primary_fill())
                                    .text_size(px(style::FONT_SMALL))
                                    .text_color(style::accent())
                                    .child(text),
                            )
                        }),
                )
                .child(div().text_sm().text_color(style::dim()).child(body.into())),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shake_starts_and_ends_at_rest_within_its_amplitude() {
        assert!(shake_offset(0.).abs() < 1e-4);
        assert!(shake_offset(1.).abs() < 1e-4);
        assert!(shake_offset(-1.).abs() < 1e-4 && shake_offset(2.).abs() < 1e-4);
        let peak = (0..=100)
            .map(|i| shake_offset(i as f32 / 100.).abs())
            .fold(0., f32::max);
        assert!(peak > 1. && peak <= style::SHAKE_PX);
    }
}
