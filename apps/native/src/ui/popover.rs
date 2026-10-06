//! Plan 032 popovers, selects and menus. Panels paint in a deferred layer
//! anchored to a trigger's recorded bounds; callers close them from
//! `on_mouse_down_out` (the outside click still goes through) and drive
//! keyboard navigation with `MenuNav`.
use crate::style;
use gpui::{
    Anchor, Bounds, Deferred, Div, ElementId, Pixels, Point, Role, SharedString, Size, Stateful,
    anchored, canvas, deferred, div, point, prelude::*, px, svg,
};
use std::{cell::Cell, rc::Rc};

/// Margin kept between a panel and the window edges.
const MARGIN: f32 = 8.;
/// Gap between a trigger and the panel it opens.
const GAP: f32 = 2.;

/// The window bounds of a trigger, written by `probe` during prepaint and
/// read when its popover opens.
pub type AnchorSlot = Rc<Cell<Option<Bounds<Pixels>>>>;

pub fn anchor_slot() -> AnchorSlot {
    Rc::new(Cell::new(None))
}

/// Absolute `size_full` canvas that records its parent's bounds each
/// prepaint. Place it as the last child of a `relative()` trigger.
pub fn probe(slot: AnchorSlot) -> impl IntoElement {
    canvas(move |bounds, _, _| slot.set(Some(bounds)), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Under the anchor, left edges aligned.
    Below,
    /// Under the anchor, right edges aligned (toolbar trailing buttons).
    BelowEnd,
    /// At the anchor's origin (context menus at the pointer).
    AtPoint,
}

/// Top-left corner for a panel of `size` opened from `anchor` inside a
/// window of `window`. Flips above the anchor when there is no room below,
/// then clamps so the panel keeps an 8 px margin to every window edge.
pub fn place(
    anchor: Bounds<Pixels>,
    size: Size<Pixels>,
    window: Size<Pixels>,
    placement: Placement,
) -> Point<Pixels> {
    let (ax, ay) = (anchor.origin.x.as_f32(), anchor.origin.y.as_f32());
    let (aw, ah) = (anchor.size.width.as_f32(), anchor.size.height.as_f32());
    let (w, h) = (size.width.as_f32(), size.height.as_f32());
    let (ww, wh) = (window.width.as_f32(), window.height.as_f32());
    let (x, below, above) = match placement {
        Placement::Below => (ax, ay + ah + GAP, ay - GAP - h),
        Placement::BelowEnd => (ax + aw - w, ay + ah + GAP, ay - GAP - h),
        Placement::AtPoint => (ax, ay, ay - h),
    };
    let y = if below + h <= wh - MARGIN || above < MARGIN {
        below
    } else {
        above
    };
    point(px(clamp_axis(x, w, ww)), px(clamp_axis(y, h, wh)))
}

/// Keeps `[start, start + len]` inside `[MARGIN, limit - MARGIN]`; a panel
/// larger than the window aligns to the leading margin.
fn clamp_axis(start: f32, len: f32, limit: f32) -> f32 {
    start.min(limit - MARGIN - len).max(MARGIN)
}

/// Paints `panel` above the document, anchored to `anchor` (window
/// coordinates) and kept inside the window with an 8 px margin.
pub fn layer(anchor: Bounds<Pixels>, placement: Placement, panel: impl IntoElement) -> Deferred {
    let (corner, position) = match placement {
        Placement::Below => (
            Anchor::TopLeft,
            anchor.bottom_left() + point(px(0.), px(GAP)),
        ),
        Placement::BelowEnd => (
            Anchor::TopRight,
            anchor.bottom_right() + point(px(0.), px(GAP)),
        ),
        Placement::AtPoint => (Anchor::TopLeft, anchor.origin),
    };
    deferred(
        anchored()
            .anchor(corner)
            .position(position)
            .snap_to_window_with_margin(px(MARGIN))
            .child(panel),
    )
    .with_priority(1)
}

/// The popover surface: `panel` fill, 8 px radius, line border, large shadow
/// and 4 px vertical padding. Occludes, so clicks inside never reach the
/// document behind.
pub fn panel(
    id: impl Into<ElementId>,
    role: Role,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(role)
        .aria_label(label.into())
        .occlude()
        .flex()
        .flex_col()
        .py(px(4.))
        .rounded(px(8.))
        .border_1()
        .border_color(style::line())
        .bg(style::panel())
        .shadow_lg()
        .text_size(px(style::FONT))
        .text_color(style::text())
}

/// Shared 22 px menu row; `leading` is the icon or check column.
fn row(
    id: impl Into<ElementId>,
    label: SharedString,
    highlighted: bool,
    enabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .aria_label(label.clone())
        .flex_none()
        .min_w(px(170.))
        .h(px(22.))
        .mx(px(4.))
        .px(px(6.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(4.))
        .whitespace_nowrap()
        .text_color(if enabled { style::text() } else { style::faint() })
        .when(highlighted, |row| row.bg(style::hover()))
        .when(enabled, |row| {
            row.cursor_pointer()
                .hover(|s| s.bg(style::hover()))
                .active(|s| s.bg(style::pressed()))
        })
        .when(!enabled, |row| {
            row.a11y_synthetic_children(|builder| builder.parent_node().set_disabled())
        })
}

fn leading(icon: Option<&'static str>, color: gpui::Rgba) -> Div {
    div()
        .flex_none()
        .size(px(style::ICON))
        .flex()
        .items_center()
        .justify_center()
        .when_some(icon, |slot, path| {
            slot.child(svg().path(path).size(px(style::ICON)).text_color(color))
        })
}

/// A menu row: optional icon, label and a trailing hint (an SQL badge or a
/// shortcut). `highlighted` marks the keyboard row from `MenuNav`.
pub fn item(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<&'static str>,
    hint: Option<SharedString>,
    highlighted: bool,
    enabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    row(id, label.clone(), highlighted, enabled)
        .role(Role::MenuItem)
        .when(highlighted, |row| row.aria_active_descendant())
        .child(leading(
            icon,
            if enabled { style::dim() } else { style::faint() },
        ))
        .child(div().flex_1().min_w_0().overflow_hidden().child(label))
        .when_some(hint, |row, hint| {
            row.child(
                div()
                    .flex_none()
                    .font_family(style::MONO)
                    .text_size(px(style::FONT_SMALL))
                    .text_color(style::faint())
                    .child(hint),
            )
        })
}

/// A menu row with a check column; AX toggled follows `checked`.
pub fn check_item(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    highlighted: bool,
    enabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    row(id, label.clone(), highlighted, enabled)
        .role(Role::MenuItemCheckBox)
        .aria_toggled(gpui::accesskit::Toggled::from(checked))
        .when(highlighted, |row| row.aria_active_descendant())
        .child(leading(
            checked.then_some("icons/check.svg"),
            style::accent(),
        ))
        .child(div().flex_1().min_w_0().overflow_hidden().child(label))
}

pub fn divider() -> Div {
    div()
        .flex_none()
        .h(px(1.))
        .my(px(4.))
        .bg(style::line_soft())
}

/// A small group heading inside a menu.
pub fn heading(label: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .h(px(20.))
        .px(px(10.))
        .flex()
        .items_center()
        .text_size(px(style::FONT_SMALL))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(style::faint())
        .child(label.into())
}

/// The closed face of a select: a field-styled button showing `value` with a
/// chevron. `label` names the choice for assistive clients.
pub fn select_trigger(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    open: bool,
    enabled: bool,
) -> Stateful<Div> {
    let value = value.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.into())
        .aria_value(value.clone())
        .aria_expanded(open)
        .flex_none()
        .h(px(style::TOOL))
        .pl(px(6.))
        .pr(px(4.))
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded(px(4.))
        .border_1()
        .border_color(if open { style::accent() } else { style::line() })
        .bg(style::bg())
        .text_sm()
        .whitespace_nowrap()
        .text_color(if enabled { style::text() } else { style::faint() })
        .when(enabled, |trigger| {
            trigger
                .cursor_pointer()
                .hover(|s| s.border_color(style::faint()))
        })
        .focus(|s| s.border_color(style::accent()))
        .child(div().flex_1().min_w_0().overflow_hidden().child(value))
        .child(
            svg()
                .path("icons/chevron_down.svg")
                .size(px(9.))
                .flex_none()
                .text_color(style::faint()),
        )
        .when(!enabled, |trigger| {
            trigger.a11y_synthetic_children(|builder| builder.parent_node().set_disabled())
        })
}

/// Keyboard state of an open menu or select list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MenuNav {
    pub len: usize,
    pub highlighted: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    /// The highlight moved; repaint.
    Moved,
    /// Run the item at this index.
    Activate(usize),
    /// Close the menu.
    Dismiss,
    /// Not a menu key; let it propagate.
    Ignored,
}

impl MenuNav {
    pub fn new(len: usize) -> Self {
        Self {
            len,
            highlighted: 0,
        }
    }

    /// Keeps the highlight on a valid row when the item list changes.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        self.highlighted = self.highlighted.min(len.saturating_sub(1));
    }

    /// Maps a GPUI `Keystroke::key`: up/down wrap, home/end jump, enter and
    /// space activate, escape dismisses.
    pub fn key(&mut self, key: &str) -> MenuKey {
        match key {
            "escape" => MenuKey::Dismiss,
            _ if self.len == 0 => MenuKey::Ignored,
            "up" => {
                self.highlighted = self.highlighted.checked_sub(1).unwrap_or(self.len - 1);
                MenuKey::Moved
            }
            "down" => {
                self.highlighted = (self.highlighted + 1) % self.len;
                MenuKey::Moved
            }
            "home" => {
                self.highlighted = 0;
                MenuKey::Moved
            }
            "end" => {
                self.highlighted = self.len - 1;
                MenuKey::Moved
            }
            "enter" | "space" => MenuKey::Activate(self.highlighted.min(self.len - 1)),
            _ => MenuKey::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn menu_nav_wraps_jumps_and_maps_keys() {
        let mut nav = MenuNav::new(3);
        assert_eq!(nav.key("up"), MenuKey::Moved);
        assert_eq!(nav.highlighted, 2);
        assert_eq!(nav.key("down"), MenuKey::Moved);
        assert_eq!(nav.highlighted, 0);
        nav.key("end");
        assert_eq!(nav.highlighted, 2);
        nav.key("home");
        assert_eq!(nav.highlighted, 0);
        nav.key("down");
        assert_eq!(nav.key("enter"), MenuKey::Activate(1));
        assert_eq!(nav.key("space"), MenuKey::Activate(1));
        assert_eq!(nav.key("escape"), MenuKey::Dismiss);
        assert_eq!(nav.key("a"), MenuKey::Ignored);
        assert_eq!(nav.key("tab"), MenuKey::Ignored);
    }

    #[test]
    fn menu_nav_clamps_after_shrinking_and_ignores_an_empty_list() {
        let mut nav = MenuNav::new(5);
        nav.key("end");
        nav.set_len(2);
        assert_eq!(nav.highlighted, 1);
        assert_eq!(nav.key("down"), MenuKey::Moved);
        assert_eq!(nav.highlighted, 0);
        nav.set_len(0);
        assert_eq!(nav.highlighted, 0);
        assert_eq!(nav.key("down"), MenuKey::Ignored);
        assert_eq!(nav.key("enter"), MenuKey::Ignored);
        assert_eq!(nav.key("escape"), MenuKey::Dismiss);
        nav.set_len(4);
        assert_eq!(nav.key("up"), MenuKey::Moved);
        assert_eq!(nav.highlighted, 3);
    }

    #[test]
    fn place_opens_below_and_aligns_end_edges() {
        let window = size(px(1000.), px(800.));
        let anchor = bounds(100., 50., 80., 20.);
        let panel = size(px(200.), px(100.));
        assert_eq!(
            place(anchor, panel, window, Placement::Below),
            point(px(100.), px(72.))
        );
        assert_eq!(
            place(anchor, panel, window, Placement::BelowEnd),
            point(px(MARGIN), px(72.))
        );
        let trailing = bounds(700., 50., 80., 20.);
        assert_eq!(
            place(trailing, panel, window, Placement::BelowEnd),
            point(px(580.), px(72.))
        );
        assert_eq!(
            place(bounds(300., 400., 0., 0.), panel, window, Placement::AtPoint),
            point(px(300.), px(400.))
        );
    }

    #[test]
    fn place_flips_above_without_room_and_keeps_window_margins() {
        let window = size(px(1000.), px(800.));
        let panel = size(px(300.), px(200.));
        let low = bounds(100., 700., 80., 20.);
        assert_eq!(
            place(low, panel, window, Placement::Below),
            point(px(100.), px(498.))
        );
        assert_eq!(
            place(bounds(900., 760., 0., 0.), panel, window, Placement::AtPoint),
            point(px(692.), px(560.))
        );
        // No room either way: stay below, clamped inside the margin.
        let tall = size(px(300.), px(790.));
        let p = place(bounds(100., 300., 80., 20.), tall, window, Placement::Below);
        assert_eq!(p.y, px(MARGIN));
        for anchor in [bounds(-50., -50., 10., 10.), bounds(990., 790., 10., 10.)] {
            for placement in [Placement::Below, Placement::BelowEnd, Placement::AtPoint] {
                let p = place(anchor, panel, window, placement);
                assert!(p.x >= px(MARGIN) && p.x + panel.width <= px(1000. - MARGIN));
                assert!(p.y >= px(MARGIN) && p.y + panel.height <= px(800. - MARGIN));
            }
        }
    }
}
