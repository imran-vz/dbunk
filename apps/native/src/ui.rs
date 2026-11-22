//! Document kit: toolbars, tool buttons, segmented switchers, list rows,
//! fields, page buttons and footer status lines. Helpers only style; callers
//! keep their own focus handles, tab order, `on_click` and `on_a11y_action`
//! handlers. Tool-document scaffolding (`tool_header`, `data_table`,
//! `property_list`, `empty_state`, `info_hint`, `env_tag`) lives in `tool`
//! and is re-exported here.
use crate::style;
use gpui::{
    Animation, AnimationElement, AnimationExt, AnyView, App, BoxShadow, Context, Div, ElementId,
    Render, Role, SharedString, Stateful, Window, div, ease_out_quint, hsla, point, prelude::*, px,
    svg,
};
use std::time::Duration;

pub mod confirm;
pub mod dialog;
pub mod popover;
pub mod tool;

// Re-exported so views write `ui::tool_header`, `ui::data_table`, ...
pub use tool::*;

/// The keyboard focus ring: 2 pt of `accent` at 60 % drawn outside the
/// control, so focus never reads as a primary border.
pub fn ring() -> BoxShadow {
    BoxShadow {
        color: style::focus_ring().into(),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
        inset: false,
    }
}

/// A `TOOLBAR` document toolbar. Wraps onto further rows instead of clipping
/// controls when the window is narrow.
pub fn toolbar() -> Div {
    div()
        .flex_none()
        .min_h(px(style::TOOLBAR))
        .py(px((style::TOOLBAR - style::CONTROL) / 2.))
        .px(px(style::INSET_BAR))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(style::S2))
        .border_b_1()
        .border_color(style::line_soft())
        .text_sm()
        .text_color(style::dim())
}

/// A `CONTROL` toolbar button labelled with `label` (also its AX name).
/// Disabled buttons render faint and expose AX disabled; a caller that
/// installs its own `a11y_synthetic_children` replaces that hook and must set
/// it again. `primary` is kept for existing callers and only raises the face;
/// a toolbar's primary action is `tool_button_accent`.
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
        .h(px(style::CONTROL))
        .px(px(style::S3 + 1.))
        .flex()
        .items_center()
        .gap(px(style::S2 + 1.))
        .rounded(px(style::R_SM))
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
        .focus(|s| s.text_color(style::text()).shadow(vec![ring()]))
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

/// A fixed `TOOLBAR` row that never wraps; controls that do not fit are
/// clipped, so callers move secondary actions into an overflow menu instead.
pub fn toolbar_strip() -> Div {
    div()
        .flex_none()
        .h(px(style::TOOLBAR))
        .px(px(style::INSET_BAR))
        .flex()
        .flex_nowrap()
        .items_center()
        .gap(px(style::S2))
        .overflow_hidden()
        .border_b_1()
        .border_color(style::line_soft())
        .text_sm()
        .whitespace_nowrap()
        .text_color(style::dim())
}

/// A `CONTROL` square icon-only button. `label` is its AX name and hover
/// tooltip.
pub fn icon_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: &'static str,
    enabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    let color = if enabled {
        style::dim()
    } else {
        style::faint()
    };
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .flex_none()
        .size(px(style::CONTROL))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(style::R_SM))
        .border_1()
        .border_color(gpui::transparent_black())
        .text_color(color)
        .when(enabled, |button| {
            press(
                button
                    .cursor_pointer()
                    .hover(|s| s.bg(style::hover()).text_color(style::text())),
            )
        })
        .focus(|s| s.text_color(style::text()).shadow(vec![ring()]))
        .child(
            svg()
                .path(icon)
                .size(px(style::ICON))
                .flex_none()
                .text_color(color),
        )
        .tooltip(tooltip(label))
        .tooltip_show_delay(tooltip_delay())
        .when(!enabled, |button| {
            button.a11y_synthetic_children(|builder| builder.parent_node().set_disabled())
        })
}

/// The single primary action of a toolbar (`Run`, `Review 3 ⌘S`): a
/// `CONTROL` tool button in the primary accent tint, with an optional
/// trailing shortcut. The only way to mark a toolbar action as primary.
pub fn tool_button_accent(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    shortcut_keys: Option<&'static str>,
    enabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .flex_none()
        .h(px(style::CONTROL))
        .px(px(style::S4))
        .flex()
        .items_center()
        .gap(px(style::S3))
        .rounded(px(style::R_SM))
        .border_1()
        .border_color(if enabled {
            style::primary_line()
        } else {
            style::line()
        })
        .bg(if enabled {
            style::primary_fill()
        } else {
            gpui::transparent_black().into()
        })
        .text_sm()
        .whitespace_nowrap()
        .text_color(if enabled {
            style::primary_text()
        } else {
            style::faint()
        })
        .when(enabled, |button| {
            press(
                button
                    .cursor_pointer()
                    .hover(|s| s.border_color(style::accent())),
            )
        })
        .when(!enabled, |button| button.opacity(0.5))
        .focus(|s| s.shadow(vec![ring()]))
        .child(label)
        .when_some(shortcut_keys, |button, keys| button.child(shortcut(keys)))
        .when(!enabled, |button| {
            button.a11y_synthetic_children(|builder| builder.parent_node().set_disabled())
        })
}

/// A 16 pt pill counting active filters, sort keys or hidden columns next
/// to a toolbar button.
pub fn count_badge(text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .h(px(16.))
        .min_w(px(16.))
        .px(px(style::S2 + 1.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(style::primary_fill())
        .text_size(px(style::FONT_SMALL))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(style::accent())
        .child(text.into())
}

/// The Data | Structure switch of a table document. Only the unselected
/// side is clickable; it opens the other view through `on_other`.
pub fn view_switch(
    id: &'static str,
    structure: bool,
    on_other: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let side = |index: usize, label: &'static str, selected: bool| {
        segment((id, index), label, selected, true)
            .aria_selected(selected)
            .tab_index(0)
    };
    let (data, view) = (side(0, "Data", !structure), side(1, "Structure", structure));
    if structure {
        segment_group().child(data.on_click(on_other)).child(view)
    } else {
        segment_group().child(data).child(view.on_click(on_other))
    }
}

/// Thousands separators: `18204` → `18,204`.
pub fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The range of `rows` rows starting after `offset` rows: `401–600`, or
/// `No rows`.
pub fn offset_range_label(offset: u64, rows: usize) -> String {
    if rows == 0 {
        return "No rows".to_owned();
    }
    format!(
        "{}–{}",
        grouped(offset.saturating_add(1)),
        grouped(offset.saturating_add(rows as u64))
    )
}

/// The `401–600` status between an offset pager's arrows: `(offset, rows)`
/// of the shown page, or `None` before the first page.
pub fn range_status(id: impl Into<ElementId>, page: Option<(u64, usize)>) -> Stateful<Div> {
    let range = page.map_or_else(
        || "–".to_owned(),
        |(offset, rows)| offset_range_label(offset, rows),
    );
    div()
        .id(id)
        .role(Role::Status)
        .aria_label(SharedString::from(format!("Rows {range}")))
        .flex_none()
        .px(px(style::S2))
        .text_color(style::dim())
        .child(range)
}

/// A `CONTROL` bordered row of `segment`s (Data | Structure). Every exclusive
/// choice uses one; the selected segment is `raised` with `text`.
pub fn segment_group() -> Div {
    div()
        .flex_none()
        .h(px(style::CONTROL))
        .p(px(1.))
        .flex()
        .items_center()
        .gap(px(1.))
        .rounded(px(style::R_SM + 1.))
        .border_1()
        .border_color(style::line())
        .bg(style::bg())
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

/// A `FOOTER` row of small tabs (results, inspector views). Children are
/// `segment`s, optionally followed by `grow()` and trailing metadata. New
/// exclusive choices put their segments in a `segment_group` instead.
pub fn segmented() -> Div {
    div()
        .flex_none()
        .min_h(px(style::FOOTER))
        .px(px(style::INSET_BAR))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(style::S1))
        .border_b_1()
        .border_color(style::line_soft())
        .text_sm()
        .text_color(style::dim())
}

/// One segment of a `segment_group` (or a `segmented` row): fits inside
/// the group's border at `CONTROL - 4`.
pub fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    enabled: bool,
) -> Stateful<Div> {
    pressed(tool_button(id, label, None, enabled, false), selected)
        .h(px(style::CONTROL - 4.))
        .rounded(px(style::R_SM - 1.))
}

pub fn separator() -> Div {
    div()
        .flex_none()
        .w(px(1.))
        .h(px(16.))
        .mx(px(style::S2))
        .bg(style::line())
}

/// Element group name set on every `list_row`; trailing actions use
/// `row_actions` (or `.group_hover(LIST_ROW_GROUP, ...)`) to appear on hover.
pub const LIST_ROW_GROUP: &str = "list-row";

/// Fill of a list row: `select` when selected in the focused pane, `raised`
/// when selected in an unfocused pane, none otherwise.
pub fn list_row_fill(selected: bool, focused: bool) -> Option<gpui::Rgba> {
    match (selected, focused) {
        (true, true) => Some(style::select()),
        (true, false) => Some(style::raised()),
        (false, _) => None,
    }
}

/// The one list row: connections, navigator tree, palette results, catalog
/// sub-nav, history and other lists. `ROW` high, `R_SM` corners, `S3`
/// padding, `hover` fill under the pointer, `list_row_fill` when selected.
///
/// The row stretches to its parent with an `S2` margin on both sides, so
/// the highlight always spans the full row: put it in a flex column (or
/// block) parent that is as wide as the list, never in a content-sized one.
/// It sets `.hover(...)` and `.group(LIST_ROW_GROUP)`; callers must not set
/// `hover` again (GPUI allows one hover style).
pub fn list_row(id: impl Into<ElementId>, selected: bool, focused: bool) -> Stateful<Div> {
    let fill = list_row_fill(selected, focused);
    div()
        .id(id)
        .group(LIST_ROW_GROUP)
        .flex_none()
        .h(px(style::ROW))
        .mx(px(style::S2))
        .px(px(style::S3))
        .flex()
        .items_center()
        .gap(px(style::S3))
        .overflow_hidden()
        .rounded(px(style::R_SM))
        .text_sm()
        .whitespace_nowrap()
        .text_color(style::text())
        .cursor_pointer()
        .when_some(fill, |row, fill| row.bg(fill))
        .when(fill.is_none(), |row| row.hover(|s| s.bg(style::hover())))
        .focus(|s| s.shadow(vec![ring()]))
}

/// A row's primary label: takes the remaining width and ellipsizes.
pub fn row_label(text: impl Into<SharedString>) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text.into())
}

/// Right-aligned `dim` metadata of a row (a database name, a count).
pub fn row_meta(text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .max_w(px(140.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(style::FONT_SMALL))
        .text_color(style::dim())
        .child(text.into())
}

/// The fixed-width trailing slot of a `list_row`. Its actions are hidden
/// until the row is hovered, or always shown when `shown` (the row is
/// selected); the slot keeps its width either way, so nothing shifts.
pub fn row_actions(width: f32, shown: bool) -> Div {
    div()
        .flex_none()
        .w(px(width))
        .h_full()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(style::S1))
        .when(!shown, |slot| {
            slot.invisible()
                .group_hover(LIST_ROW_GROUP, |s| s.visible())
        })
}

pub fn grow() -> Div {
    div().flex_1()
}

/// Inline section heading inside a document toolbar or card
/// (`Columns 8`, `Indexes 3`): semibold `text`; put counts in a `dim` child.
pub fn section_label(label: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(style::S3))
        .text_sm()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(style::text())
        .child(label.into())
}

/// A sentence-case `FONT_HEADING` semibold heading for a group inside a tool
/// document or page, with no rule. `section` adds the soft rule for pages.
pub fn heading(title: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(style::S3))
        .text_size(px(style::FONT_HEADING))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(style::text())
        .child(title.into())
}

/// Small sans tag (`Table`, `read-only`, `unique`): `R_SM`, line border,
/// `FONT_SMALL` `dim`. Wrap identifier values in `mono` yourself.
pub fn badge(text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .h(px(18.))
        .px(px(style::S3 - 1.))
        .flex()
        .items_center()
        .rounded(px(style::R_SM))
        .border_1()
        .border_color(style::line())
        .text_size(px(style::FONT_SMALL))
        .text_color(style::dim())
        .whitespace_nowrap()
        .child(text.into())
}

/// The one key chip (PK, FK, unique, identity, primary): `primary_fill`,
/// sans `FONT_SMALL` semibold in `accent`.
pub fn key_chip(text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .h(px(16.))
        .px(px(style::S2 + 1.))
        .flex()
        .items_center()
        .rounded(px(style::R_SM))
        .bg(style::primary_fill())
        .text_size(px(style::FONT_SMALL - 1.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(style::accent())
        .whitespace_nowrap()
        .child(text.into())
}

/// A monospace span for data inside sans chrome: cell values, SQL,
/// identifiers, hosts and in-row durations (DESIGN.md mono rule).
pub fn mono(text: impl Into<SharedString>) -> Div {
    div()
        .font_family(style::MONO)
        .text_size(px(style::FONT_DATA))
        .child(text.into())
}

/// `schema.` dim, `table` bright, both monospace (identifiers as data).
pub fn crumbs(prefix: impl Into<SharedString>, leaf: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .font_family(style::MONO)
        .text_color(style::dim())
        .whitespace_nowrap()
        .child(prefix.into())
        .child(div().text_color(style::text()).child(leaf.into()))
}

/// The `FOOTER` document footer: sans `FONT_SMALL`, `dim`, bordered on top.
/// Left: counts; right (after `grow()`): timing.
pub fn status_line() -> Div {
    div()
        .flex_none()
        .min_h(px(style::FOOTER))
        .px(px(style::INSET_BAR))
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(style::S5))
        .border_t_1()
        .border_color(style::line_soft())
        .text_size(px(style::FONT_SMALL))
        .text_color(style::dim())
}

/// Keyboard hint inside a tool button or menu row (`⌘↵`); pair with
/// `aria_keyshortcuts`.
pub fn shortcut(keys: &'static str) -> Div {
    div()
        .flex_none()
        .text_size(px(style::FONT_SMALL))
        .text_color(style::dim())
        .child(keys)
}

/// A bordered inline text field frame for an `AccessibleEditor` in a
/// toolbar: `CONTROL` high.
pub fn field() -> Div {
    div()
        .min_h(px(style::CONTROL))
        .px(px(style::S3))
        .flex()
        .items_center()
        .rounded(px(style::R_SM))
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

/// Fill, border and text of a page button variant.
fn button_colors(variant: Variant) -> (gpui::Rgba, gpui::Rgba, gpui::Rgba) {
    let clear: gpui::Rgba = gpui::transparent_black().into();
    match variant {
        Variant::Primary => (style::accent(), clear, style::on_accent()),
        Variant::Secondary => (style::raised(), style::line(), style::text()),
        Variant::Ghost => (clear, clear, style::dim()),
        Variant::Danger => (style::bad(), clear, style::on_accent()),
    }
}

/// A `FIELD` (28 pt) page and dialog button. Primary is solid `accent` with
/// dark text and at most one per view; Danger is solid `bad`. Focus is the
/// outside `ring`, never a border change, so a focused secondary never
/// looks primary. Disabled: 50 % opacity, `faint` text, AX disabled. Like
/// `tool_button`, it only styles: callers own focus, `on_click` and
/// `on_a11y_action`.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    variant: Variant,
    enabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    let (fill, line, text) = button_colors(variant);
    let solid = matches!(variant, Variant::Primary | Variant::Danger);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .flex_none()
        .h(px(style::FIELD))
        .px(px(style::S5))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(style::S3))
        .rounded(px(style::R_MD))
        .border_1()
        .border_color(line)
        .bg(fill)
        .text_sm()
        .when(solid, |b| b.font_weight(gpui::FontWeight::SEMIBOLD))
        .whitespace_nowrap()
        .text_color(if enabled { text } else { style::faint() })
        .when(variant == Variant::Secondary && enabled, |b| {
            b.shadow(vec![lifted()])
        })
        .when(!enabled, |b| b.opacity(0.5))
        .when(enabled, |b| {
            let b = b.cursor_pointer().hover(move |s| match variant {
                Variant::Ghost => s.bg(style::hover()).text_color(style::text()),
                Variant::Primary => s.bg(style::accent_hover()),
                Variant::Danger => s.opacity(0.9),
                Variant::Secondary => s.bg(style::hover()).border_color(style::faint()),
            });
            if solid {
                // A solid face keeps its colour while held, so its dark
                // label stays readable; it only sinks.
                b.active(|s| s.opacity(0.85).shadow(vec![sunken()]))
            } else {
                press(b)
            }
        })
        .focus(move |s| {
            if variant == Variant::Secondary {
                s.shadow(vec![lifted(), ring()])
            } else {
                s.shadow(vec![ring()])
            }
        })
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
        .gap(px(style::S3))
        .px(px(style::S4))
        .py(px(style::S3))
        .rounded(px(style::R_MD))
        .border_1()
        .border_color(style::bad_line())
        .bg(style::bad_fill())
        .text_sm()
        .text_color(style::bad_text())
        .child(
            svg()
                .path("icons/warning.svg")
                .mt(px(2.))
                .size(px(style::ICON))
                .flex_none()
                .text_color(style::bad_text()),
        )
        .child(div().flex_1().min_w_0().child(message))
}

/// Marks which status message of a document is an error, so it renders as a
/// shaking `error_strip`. The strip shows only while the document's message
/// still equals the recorded error; any later status quietly replaces it.
#[derive(Default)]
pub struct Failure {
    seq: u64,
    text: String,
}

impl Failure {
    /// Records `message` as the current error; each call replays the shake.
    pub fn record(&mut self, message: &str) {
        self.seq = self.seq.wrapping_add(1);
        self.text.clear();
        self.text.push_str(message);
    }

    /// The shake key while `message` is still the recorded error.
    pub fn shown(&self, message: &str) -> Option<u64> {
        (self.seq != 0 && !message.is_empty() && self.text == message).then_some(self.seq)
    }
}

/// A document's inline error: `error_banner` that shakes once per `seq`.
pub fn error_strip(id: &'static str, seq: u64, message: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .px(px(style::S4))
        .py(px(style::S3))
        .child(shake((id, seq), error_banner(id, message)))
}

/// A hover tooltip that fades in; no scale, no slide.
pub struct Tooltip {
    text: SharedString,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(360.))
            .px(px(style::S4))
            .py(px(style::S2))
            .rounded(px(style::R_MD))
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

/// Border of a text-field well: `bad_line` on error, `accent` while the
/// field has focus, `line` at rest.
pub fn input_border(error: bool, focused: bool) -> gpui::Rgba {
    if error {
        style::bad_line()
    } else if focused {
        style::accent()
    } else {
        style::line()
    }
}

/// Bordered `FIELD` text-field well with an `S4` inset; `error` swaps the
/// border to the danger colour. Use `input_well` when the caller knows
/// whether the field has focus.
pub fn input_frame(error: bool) -> Div {
    input_well(error, false)
}

/// `input_frame` with the 1 pt `accent` focus edge while `focused`, so a
/// focused empty field is visible without its caret.
pub fn input_well(error: bool, focused: bool) -> Div {
    div()
        .min_h(px(style::FIELD))
        .px(px(style::S4))
        .flex()
        .items_center()
        .gap(px(style::S2))
        .rounded(px(style::R_MD))
        .border_1()
        .border_color(input_border(error, focused))
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
        .gap(px(style::S3))
        .min_w_0()
        .child(div().text_sm().text_color(style::dim()).child(label.into()))
        .child(input)
        .when_some(note, |field, (text, error)| {
            field.child(
                div()
                    .text_size(px(style::FONT_SMALL))
                    .text_color(if error {
                        style::bad_text()
                    } else {
                        style::dim()
                    })
                    .child(text),
            )
        })
}

/// A titled group of fields on a full-window page: sentence-case
/// `FONT_HEADING` semibold heading over a soft rule. Tool documents use
/// `heading` (no rule).
pub fn section(title: impl Into<SharedString>) -> Div {
    div().flex().flex_col().gap(px(style::S5)).child(
        div()
            .pb(px(style::S3))
            .border_b_1()
            .border_color(style::line_soft())
            .text_size(px(style::FONT_HEADING))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(style::text())
            .child(title.into()),
    )
}

/// 12 pt checkbox glyph for toggle rows.
pub fn check_box(checked: bool) -> Div {
    div()
        .flex_none()
        .size(px(12.))
        .mt(px(2.))
        .rounded(px(style::R_SM - 1.))
        .border_1()
        .border_color(if checked {
            style::accent()
        } else {
            style::dim()
        })
        .bg(if checked {
            style::accent()
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
                    .text_color(style::on_accent()),
            )
        })
}

/// State of a tri-state checkbox: a select-all header is `Mixed` while only
/// some rows are checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Off,
    Mixed,
    On,
}

impl CheckState {
    /// The AX toggled state callers set on the control that owns the box.
    pub fn toggled(self) -> gpui::accesskit::Toggled {
        match self {
            Self::Off => gpui::accesskit::Toggled::False,
            Self::Mixed => gpui::accesskit::Toggled::Mixed,
            Self::On => gpui::accesskit::Toggled::True,
        }
    }
}

/// 12 px tri-state checkbox glyph: empty, dash (mixed) or check. Styling
/// only; the caller's control carries the role and toggled state.
pub fn tri_check_box(state: CheckState) -> Div {
    let icon = match state {
        CheckState::Off => None,
        CheckState::Mixed => Some("icons/dash.svg"),
        CheckState::On => Some("icons/check.svg"),
    };
    div()
        .flex_none()
        .size(px(12.))
        .rounded(px(style::R_SM - 1.))
        .border_1()
        .border_color(if icon.is_some() {
            style::accent()
        } else {
            style::faint()
        })
        .bg(if icon.is_some() {
            style::primary_fill()
        } else {
            style::bg()
        })
        .flex()
        .items_center()
        .justify_center()
        .when_some(icon, |b, path| {
            b.child(svg().path(path).size(px(9.)).text_color(style::accent()))
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
        .gap(px(style::S5))
        .p(px(style::S5))
        .rounded(px(style::R_MD))
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
        .focus(|s| s.shadow(vec![ring()]))
        .child(
            div()
                .flex_none()
                .size(px(28.))
                .rounded(px(style::R_MD))
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
                .gap(px(style::S1 + 1.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(style::S3))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(style::text())
                        .child(title)
                        .when_some(badge_text, |row, text| {
                            row.child(
                                div()
                                    .font_weight(gpui::FontWeight::NORMAL)
                                    .child(badge(text)),
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
    fn offset_ranges_follow_the_rows_actually_kept() {
        // A byte-bounded page kept 37 of 200 rows after row 400.
        assert_eq!(offset_range_label(400, 37), "401–437");
        assert_eq!(offset_range_label(1_234_000, 0), "No rows");
        assert_eq!(
            offset_range_label(u64::MAX, 3),
            format!("{0}–{0}", grouped(u64::MAX))
        );
    }

    #[test]
    fn grouped_inserts_thousands_separators() {
        assert_eq!(grouped(1_000_000), "1,000,000");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(0), "0");
    }

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

    #[test]
    fn list_rows_fill_by_selection_and_pane_focus() {
        assert_eq!(list_row_fill(false, true), None);
        assert_eq!(list_row_fill(false, false), None);
        assert_eq!(list_row_fill(true, true), Some(style::select()));
        assert_eq!(list_row_fill(true, false), Some(style::raised()));
    }

    #[test]
    fn primary_and_danger_buttons_are_solid_and_readable() {
        for variant in [Variant::Primary, Variant::Danger] {
            let (fill, _, text) = button_colors(variant);
            assert_eq!(fill.a, 1.);
            assert!(style::contrast(text, fill) >= 4.5, "{variant:?}");
        }
        // A secondary never takes the accent; focus is the outside ring.
        let (fill, line, _) = button_colors(Variant::Secondary);
        assert!(fill != style::accent() && line != style::accent());
        assert!(ring().spread_radius > px(0.) && !ring().inset);
    }

    #[test]
    fn input_border_prefers_error_over_focus() {
        assert_eq!(input_border(true, true), style::bad_line());
        assert_eq!(input_border(false, true), style::accent());
        assert_eq!(input_border(false, false), style::line());
    }

    #[test]
    fn check_states_map_to_ax_toggled() {
        use gpui::accesskit::Toggled;
        assert_eq!(CheckState::Off.toggled(), Toggled::False);
        assert_eq!(CheckState::Mixed.toggled(), Toggled::Mixed);
        assert_eq!(CheckState::On.toggled(), Toggled::True);
    }

    #[test]
    fn failure_shows_only_while_its_message_stands_and_replays_on_repeat() {
        let mut failure = Failure::default();
        assert_eq!(failure.shown(""), None);
        assert_eq!(failure.shown("Ready"), None);
        failure.record("Review refused: timeout");
        let first = failure.shown("Review refused: timeout").unwrap();
        assert_eq!(failure.shown("Observing; no change sent"), None);
        failure.record("Review refused: timeout");
        assert!(failure.shown("Review refused: timeout").unwrap() != first);
    }
}
