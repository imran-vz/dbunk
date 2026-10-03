//! A frozen prefix keeps its own horizontal viewport. When pins overflow,
//! horizontal input over that prefix and keyboard reveal pan it independently.
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
