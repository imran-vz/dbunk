//! Live column resize. The header handle starts a typed drag; the grid root
//! follows it, applies widths locally and persists once on release. Releasing
//! outside the grid, or losing the release entirely, still finishes exactly once.
use super::*;
use crate::grid_columns::{RESIZE_MAX, RESIZE_MIN};
use gpui::{Div, DragMoveEvent, EntityId, MouseUpEvent, Stateful};

/// Width of the grab area at a header cell's right edge.
pub(super) const HANDLE_WIDTH: f32 = 6.;

/// The drag payload. `grid` filters drags that started in another grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ColumnResizeDrag {
    pub grid: EntityId,
    pub display: usize,
}

/// Recorded on the first move of a drag. `source` stays valid if the display
/// order changes mid-drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ResizeState {
    pub source: usize,
    pub start_x: f32,
    pub start_width: f32,
    pub width: f32,
}

/// Pure width for a pointer at `x` after starting at `start_x`. Rounded so
/// sub-pixel motion never repaints, clamped to the stored-width bounds.
pub(super) fn resize_width(start_width: f32, start_x: f32, x: f32) -> f32 {
    let width = start_width + (x - start_x);
    if !width.is_finite() {
        return start_width.clamp(RESIZE_MIN, RESIZE_MAX);
    }
    width.round().clamp(RESIZE_MIN, RESIZE_MAX)
}

impl ResultGrid {
    /// Absolute 6 px handle at the right edge of header cell `display`. A
    /// 1 px accent line shows on hover and while this column is being dragged.
    pub(super) fn resize_handle(&self, display: usize, cx: &Context<Self>) -> Stateful<Div> {
        let source = self.source_column(display);
        let dragging = source.is_some()
            && self
                .resize
                .is_some_and(|resize| Some(resize.source) == source);
        div()
            .id(("column-resize", display))
            .absolute()
            .top_0()
            .right_0()
            .w(px(HANDLE_WIDTH))
            .h_full()
            .cursor_col_resize()
            .group("column-resize")
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .w(px(1.))
                    .h_full()
                    .bg(crate::style::accent())
                    .opacity(if dragging { 1. } else { 0. })
                    .group_hover("column-resize", |style| style.opacity(1.)),
            )
            // The header opens its menu on click; keep the press here so a
            // drag or double-click never reaches it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                cx.stop_propagation();
                if event.click_count() >= 2
                    && let Some(source) = this.source_column(display)
                {
                    this.auto_fit_source(source, cx);
                }
            }))
            .on_drag(
                ColumnResizeDrag {
                    grid: cx.entity_id(),
                    display,
                },
                |_, _, _, cx| cx.new(|_| gpui::Empty),
            )
    }

    pub(super) fn drag_resize(
        &mut self,
        event: &DragMoveEvent<ColumnResizeDrag>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let drag = *event.drag(cx);
        if drag.grid != cx.entity_id() {
            return;
        }
        let x = event.event.position.x / px(1.);
        let state = match self.resize {
            Some(state) => state,
            None => {
                let Some(source) = self.source_column(drag.display) else {
                    return;
                };
                let start_width = self.column_width(drag.display) / px(1.);
                let state = ResizeState {
                    source,
                    start_x: x,
                    start_width,
                    width: start_width,
                };
                self.resize = Some(state);
                cx.notify();
                state
            }
        };
        let width = resize_width(state.start_width, state.start_x, x);
        if (width - state.width).abs() < 1. {
            return;
        }
        if self.apply_width(state.source, width) {
            self.resize = Some(ResizeState { width, ..state });
            self.clamp_horizontal_scroll();
            cx.notify();
        }
    }

    /// Applies a width to one source column of the current layout without
    /// persisting it.
    fn apply_width(&mut self, source: usize, width: f32) -> bool {
        if self.table.is_some() {
            match self.columns.display(source) {
                Some(display) => self.columns.set_live_width(display, width),
                None => false,
            }
        } else {
            self.model
                .sets
                .get_mut(self.model.active)
                .is_some_and(|set| set.widths.set_resized(source, width))
        }
    }

    /// Idempotent: the drop, a release outside the grid and the next press
    /// inside it may all call this for one drag.
    pub(super) fn finish_resize(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.resize.take() else {
            return;
        };
        cx.notify();
        if self.table.is_none() || state.width == state.start_width {
            // Query grids keep dragged widths locally only.
            return;
        }
        let Some(display) = self.columns.display(state.source) else {
            return;
        };
        match self.columns.width_patch(display) {
            Ok(patch) => cx.emit(GridEvent::Preferences(patch)),
            Err(error) => {
                // Without a savable name the local width would silently
                // diverge from storage; return to the stored width.
                self.columns.clear_overrides();
                self.clamp_horizontal_scroll();
                self.set_status(error, cx);
            }
        }
    }

    pub(super) fn drop_resize(
        &mut self,
        drag: &ColumnResizeDrag,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if drag.grid == cx.entity_id() {
            self.finish_resize(cx);
        }
    }

    pub(super) fn release_outside(
        &mut self,
        _: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_resize(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_width_clamps_and_is_monotonic_in_pointer_position() {
        assert_eq!(resize_width(160., 500., 500.), 160.);
        assert_eq!(resize_width(160., 500., 540.4), 200.);
        assert_eq!(resize_width(160., 500., 0.), RESIZE_MIN);
        assert_eq!(resize_width(160., 500., 5_000.), RESIZE_MAX);
        assert_eq!(resize_width(160., 500., f32::NAN), 160.);
        let mut previous = f32::MIN;
        for step in 0..400 {
            let width = resize_width(300., 200., step as f32 * 7.5);
            assert!(width >= previous, "{step}");
            assert!((RESIZE_MIN..=RESIZE_MAX).contains(&width));
            previous = width;
        }
    }
}
