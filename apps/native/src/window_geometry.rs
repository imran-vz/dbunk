//! Restores the last normal-quit window frame only where its title bar is
//! reachable on the same connected display; otherwise the size is recentred
//! on the primary display. GPUI frames are display-local with a top-left
//! origin, and window creation sizes the content area, so callers save the
//! frame origin with the content size.
use dbunk_lib::backend::WindowGeometry;

const MIN_WIDTH: f32 = 640.;
const MIN_HEIGHT: f32 = 480.;
const MAX_EXTENT: f32 = 16_384.;
/// Title-bar strip that must remain on screen to drag the window back.
const TITLE_BAR: f32 = 28.;
const VISIBLE_TITLE: f32 = 100.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
/// A connected display: stable UUID when available and its size in points.
pub struct Display {
    pub uuid: Option<String>,
    pub width: f32,
    pub height: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Index into the displays passed to `restore`.
    pub display: usize,
    pub frame: Rect,
}

fn overlap(start: f32, length: f32, limit: f32) -> f32 {
    ((start + length).min(limit) - start.max(0.)).max(0.)
}

/// `displays[0]` is the primary display. `None` means use the default frame.
pub fn restore(saved: Option<&WindowGeometry>, displays: &[Display]) -> Option<Placement> {
    let saved = saved?;
    let primary = displays.first()?;
    if ![saved.x, saved.y, saved.width, saved.height]
        .iter()
        .all(|value| value.is_finite())
        || !(MIN_WIDTH..=MAX_EXTENT).contains(&saved.width)
        || !(MIN_HEIGHT..=MAX_EXTENT).contains(&saved.height)
    {
        return None;
    }
    let frame = Rect {
        x: saved.x,
        y: saved.y,
        width: saved.width,
        height: saved.height,
    };
    let same = saved.display.as_ref().and_then(|uuid| {
        displays
            .iter()
            .position(|display| display.uuid.as_ref() == Some(uuid))
    });
    if let Some(index) = same {
        let display = &displays[index];
        if overlap(frame.x, frame.width, display.width) >= VISIBLE_TITLE
            && overlap(frame.y, TITLE_BAR, display.height) >= TITLE_BAR
        {
            return Some(Placement {
                display: index,
                frame,
            });
        }
    }
    // Removed or rearranged display: keep the size, never place offscreen.
    let width = frame.width.min(primary.width);
    let height = frame.height.min(primary.height);
    Some(Placement {
        display: 0,
        frame: Rect {
            x: (primary.width - width) / 2.,
            y: (primary.height - height) / 2.,
            width,
            height,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn displays() -> Vec<Display> {
        vec![
            Display {
                uuid: Some("primary".into()),
                width: 1512.,
                height: 982.,
            },
            Display {
                uuid: Some("external".into()),
                width: 2560.,
                height: 1440.,
            },
        ]
    }
    fn saved(display: Option<&str>, x: f32, y: f32, width: f32, height: f32) -> WindowGeometry {
        WindowGeometry {
            x,
            y,
            width,
            height,
            maximized: false,
            display: display.map(Into::into),
        }
    }

    #[test]
    fn visible_frames_restore_exactly_on_their_own_display() {
        let frame = saved(Some("external"), 1800., 50., 700., 800.);
        assert_eq!(
            restore(Some(&frame), &displays()),
            Some(Placement {
                display: 1,
                frame: Rect {
                    x: 1800.,
                    y: 50.,
                    width: 700.,
                    height: 800.
                }
            })
        );
    }

    #[test]
    fn missing_display_or_unreachable_title_recentres_on_primary() {
        // Display-local x=2000 is only valid on the external display.
        let detached = saved(Some("gone"), 2000., 50., 900., 800.);
        let placement = restore(Some(&detached), &displays()).unwrap();
        assert_eq!(placement.display, 0);
        assert_eq!(
            placement.frame,
            Rect {
                x: 306.,
                y: 91.,
                width: 900.,
                height: 800.
            }
        );
        let unknown = saved(None, 10., 10., 3000., 2000.);
        let placement = restore(Some(&unknown), &displays()).unwrap();
        assert_eq!((placement.display, placement.frame.width), (0, 1512.));
        // A title bar above the display is unreachable even if the body shows.
        let above = saved(Some("primary"), 100., -500., 1000., 800.);
        assert_eq!(restore(Some(&above), &displays()).unwrap().frame.y, 91.);
        let sliver = saved(Some("primary"), 1450., 100., 900., 700.);
        assert_ne!(restore(Some(&sliver), &displays()).unwrap().frame.x, 1450.);
    }

    #[test]
    fn invalid_or_absent_records_use_the_default_frame() {
        assert_eq!(restore(None, &displays()), None);
        let valid = saved(Some("primary"), 0., 0., 800., 600.);
        assert_eq!(restore(Some(&valid), &[]), None);
        for frame in [
            saved(None, f32::NAN, 0., 800., 600.),
            saved(None, 0., 0., f32::INFINITY, 600.),
            saved(None, 0., 0., 100., 600.),
            saved(None, 0., 0., 800., 20_000.),
        ] {
            assert_eq!(restore(Some(&frame), &displays()), None);
        }
    }
}
