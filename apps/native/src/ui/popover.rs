//! Plan 032 popovers, selects and menus. Panels paint in a deferred layer
//! anchored to a trigger's recorded bounds; callers close them from
//! `on_mouse_down_out` (the outside click still goes through) and drive
//! keyboard navigation with `MenuNav`.
use crate::style;
use gpui::{
    Anchor, Bounds, Deferred, Div, ElementId, Pixels, Role, SharedString, Stateful, anchored,
    canvas, deferred, div, point, prelude::*, px, svg,
};
use std::{cell::Cell, rc::Rc};

/// Margin kept between a panel and the window edges.
const MARGIN: f32 = style::S4;
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

/// The popover surface: `panel` fill, `R_LG` radius, line border, large
/// shadow and `S2` vertical padding. Occludes, so clicks inside never reach
/// the document behind. Every menu, select list and palette uses it.
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
        .py(px(style::S2))
        .rounded(px(style::R_LG))
        .border_1()
        .border_color(style::line())
        .bg(style::panel())
        .shadow_lg()
        .text_size(px(style::FONT))
        .text_color(style::text())
}

/// Shared `ROW` menu row, inset `S2` from the panel edges so the highlight
/// is a rounded pill; `leading` is the icon or check column.
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
        .min_w(px(180.))
        .h(px(style::ROW))
        .mx(px(style::S2))
        .px(px(style::S3 + 1.))
        .flex()
        .items_center()
        .gap(px(style::S4))
        .rounded(px(style::R_SM))
        .whitespace_nowrap()
        .text_color(if enabled {
            style::text()
        } else {
            style::faint()
        })
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

/// A menu row: optional icon, label and a trailing hint (a shortcut or an
/// SQL badge) in sans `dim`. `highlighted` marks the keyboard row from
/// `MenuNav`. A menu gives every row an icon or none (`icons_consistent`).
pub fn item(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<&'static str>,
    hint: Option<SharedString>,
    highlighted: bool,
    enabled: bool,
) -> Stateful<Div> {
    styled_item(id, label.into(), icon, hint, highlighted, enabled, false)
}

/// A destructive menu row ("Delete…"): label and icon in `bad_text`. Put it
/// in its own group at the bottom of the menu, after a `divider`.
pub fn danger_item(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<&'static str>,
    highlighted: bool,
    enabled: bool,
) -> Stateful<Div> {
    styled_item(id, label.into(), icon, None, highlighted, enabled, true)
}

fn styled_item(
    id: impl Into<ElementId>,
    label: SharedString,
    icon: Option<&'static str>,
    hint: Option<SharedString>,
    highlighted: bool,
    enabled: bool,
    danger: bool,
) -> Stateful<Div> {
    let color = match (enabled, danger) {
        (false, _) => style::faint(),
        (true, true) => style::bad_text(),
        (true, false) => style::dim(),
    };
    row(id, label.clone(), highlighted, enabled)
        .role(Role::MenuItem)
        .when(highlighted, |row| row.aria_active_descendant())
        .when(danger && enabled, |row| row.text_color(style::bad_text()))
        .child(leading(icon, color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(label),
        )
        .when_some(hint, |row, hint| {
            row.child(
                div()
                    .flex_none()
                    .pl(px(style::S5))
                    .text_size(px(style::FONT_SMALL))
                    .text_color(style::dim())
                    .child(hint),
            )
        })
}

/// True when every row of a menu has an icon or none does. Menus that mix
/// them get a ragged label column.
#[cfg(test)]
pub fn icons_consistent(icons: impl IntoIterator<Item = bool>) -> bool {
    let mut icons = icons.into_iter();
    match icons.next() {
        None => true,
        Some(first) => icons.all(|has| has == first),
    }
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

/// A rule between groups of a menu.
pub fn divider() -> Div {
    div()
        .flex_none()
        .h(px(1.))
        .my(px(style::S2))
        .bg(style::line_soft())
}

/// A small sentence-case group heading inside a menu, aligned with the row
/// labels' leading edge.
pub fn heading(label: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .h(px(style::ROW - 2.))
        .px(px(style::S2 + style::S3 + 1.))
        .flex()
        .items_center()
        .text_size(px(style::FONT_SMALL))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(style::dim())
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
        .h(px(style::CONTROL))
        .pl(px(style::S4))
        .pr(px(style::S3))
        .flex()
        .items_center()
        .gap(px(style::S3))
        .rounded(px(style::R_SM))
        .border_1()
        .border_color(if open { style::accent() } else { style::line() })
        .bg(style::bg())
        .text_sm()
        .whitespace_nowrap()
        .text_color(if enabled {
            style::text()
        } else {
            style::faint()
        })
        .when(enabled, |trigger| {
            trigger
                .cursor_pointer()
                .hover(|s| s.border_color(style::faint()))
        })
        .focus(|s| s.shadow(vec![crate::ui::ring()]))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(value),
        )
        .child(
            svg()
                .path("icons/chevron_down.svg")
                .size(px(10.))
                .flex_none()
                .text_color(style::dim()),
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
    fn menus_give_every_row_an_icon_or_none() {
        assert!(icons_consistent([]));
        assert!(icons_consistent([true, true, true]));
        assert!(icons_consistent([false, false]));
        assert!(!icons_consistent([true, false, true]));
        assert!(!icons_consistent([false, true]));
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
}
