//! A frozen prefix keeps its own horizontal viewport. When pins overflow,
//! horizontal input over that prefix and keyboard reveal pan it independently.
use gpui::{Bounds, Pixels, point, px, size};

#[derive(Clone, Copy, Debug)]
pub(super) struct Panes {
    pub pinned_width: f32,
    pub pinned_viewport: f32,
    pub scrolling_viewport: f32,
    pub content_width: f32,
    pub pinned_max: f32,
    pub scrolling_max: f32,
}
impl Panes {
    pub fn new(total: f32, pinned: f32, viewport: f32, all_pinned: bool) -> Self {
        let viewport = viewport.max(0.);
        let pinned_viewport = pinned.min(if all_pinned { viewport } else { viewport / 2. });
        let scrolling_viewport = viewport - pinned_viewport;
        let scrolling_width = (total - pinned).max(0.);
        Self {
            pinned_width: pinned,
            pinned_viewport,
            scrolling_viewport,
            content_width: (pinned_viewport + scrolling_width).max(viewport),
            pinned_max: (pinned - pinned_viewport).max(0.),
            scrolling_max: (scrolling_width - scrolling_viewport).max(0.),
        }
    }
}

/// Horizontal geometry shared by the header, the insert band, the body and
/// the inline-editor layer. Coordinates are relative to the strip's left edge.
#[derive(Clone, Copy, Debug)]
pub(super) struct CellGeometry {
    pub gutter: f32,
    pub panes: Panes,
    /// Horizontal pan of the frozen pane.
    pub pinned_left: f32,
    /// Horizontal scroll of the data pane.
    pub scroll_left: f32,
    /// `column_left(display)` and `column_width(display)` of the target.
    pub left: f32,
    pub width: f32,
    pub height: f32,
}

/// Unclipped rectangle of one cell, relative to its strip. `pinned` selects
/// the frozen pane; the caller clips with `pane_range`.
pub(super) fn cell_rect(geometry: CellGeometry, row_offset_y: f32, pinned: bool) -> Bounds<Pixels> {
    let x = if pinned {
        geometry.gutter + geometry.left - geometry.pinned_left
    } else {
        geometry.gutter + geometry.panes.pinned_viewport + geometry.left
            - geometry.panes.pinned_width
            - geometry.scroll_left
    };
    Bounds::new(
        point(px(x), px(row_offset_y)),
        size(px(geometry.width), px(geometry.height)),
    )
}

/// Left edge and width of the pane a cell is painted in.
pub(super) fn pane_range(geometry: CellGeometry, pinned: bool) -> (f32, f32) {
    if pinned {
        (geometry.gutter, geometry.panes.pinned_viewport)
    } else {
        (
            geometry.gutter + geometry.panes.pinned_viewport,
            geometry.panes.scrolling_viewport,
        )
    }
}

pub(super) fn reveal(left: f32, width: f32, current: f32, viewport: f32, maximum: f32) -> f32 {
    let next = if left < current || width > viewport {
        left
    } else if left + width > current + viewport {
        left + width - viewport
    } else {
        current
    };
    next.clamp(0., maximum)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_pins_leave_a_scrollable_data_viewport_without_losing_columns() {
        let panes = Panes::new(3000., 1800., 800., false);
        assert_eq!(panes.pinned_viewport, 400.);
        assert_eq!(panes.scrolling_viewport, 400.);
        assert_eq!(panes.content_width, 1600.);
        assert_eq!(panes.scrolling_max, 800.);
        assert_eq!(panes.pinned_max, 1400.);
        assert_eq!(reveal(1500., 300., 0., 400., panes.pinned_max), 1400.);
        assert_eq!(reveal(0., 500., 1400., 400., panes.pinned_max), 0.);
        let all = Panes::new(1800., 1800., 800., true);
        assert_eq!(all.pinned_viewport, 800.);
        assert_eq!(all.scrolling_max, 0.);
        assert_eq!(all.pinned_max, 1000.);
        let none = Panes::new(1800., 0., 800., false);
        assert_eq!(none.pinned_viewport, 0.);
        assert_eq!(none.scrolling_max, 1000.);
    }
    #[test]
    fn cell_rect_follows_pane_scroll_for_pinned_and_scrolling_columns() {
        // Two 100 px pinned columns, a 600 px data pane, 30 px gutter.
        let panes = Panes::new(1400., 200., 800., false);
        let geometry = |left: f32, width: f32| CellGeometry {
            gutter: 30.,
            panes,
            pinned_left: 40.,
            scroll_left: 250.,
            left,
            width,
            height: 20.,
        };
        // Pinned column 1 pans with the frozen pane only.
        let pinned = cell_rect(geometry(100., 100.), 60., true);
        assert_eq!(pinned.origin, point(px(90.), px(60.)));
        assert_eq!(pinned.size, size(px(100.), px(20.)));
        assert_eq!(pane_range(geometry(100., 100.), true), (30., 200.));
        // A data column at 500 px sits after the frozen pane, minus scroll.
        let scrolling = cell_rect(geometry(500., 160.), 0., false);
        assert_eq!(
            scrolling.origin,
            point(px(30. + 200. + 300. - 250.), px(0.))
        );
        assert_eq!(scrolling.size.width, px(160.));
        assert_eq!(pane_range(geometry(500., 160.), false), (230., 600.));
        // Without scroll the first data column starts right after the pins.
        let unscrolled = CellGeometry {
            scroll_left: 0.,
            pinned_left: 0.,
            ..geometry(200., 80.)
        };
        assert_eq!(cell_rect(unscrolled, 20., false).origin.x, px(230.));
        assert_eq!(cell_rect(unscrolled, 20., true).origin.x, px(230.));
    }
    #[test]
    fn keyboard_reveal_respects_the_scrolling_viewport_and_resize() {
        let panes = Panes::new(1400., 200., 800., false);
        assert_eq!(panes.scrolling_viewport, 600.);
        // Coordinates here are relative to the unpinned group, not source order.
        assert_eq!(reveal(900., 200., 0., 600., panes.scrolling_max), 500.);
        assert_eq!(reveal(300., 200., 500., 600., panes.scrolling_max), 300.);
        let wide = Panes::new(1400., 200., 2000., false);
        assert_eq!(
            reveal(
                900.,
                200.,
                500.,
                wide.scrolling_viewport,
                wide.scrolling_max
            ),
            0.
        );
    }
}
