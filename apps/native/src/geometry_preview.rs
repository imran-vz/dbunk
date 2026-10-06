//! Baseline geometry preview: POINT, LINESTRING and POLYGON WKT scaled into a
//! 320x180 view with bounds. A drawing aid only, not WKT, SRID or topology
//! validation; the database still validates the staged literal.
use crate::style;
use gpui::{IntoElement, PathBuilder, Styled, canvas, point, px};

/// Larger drafts are not parsed on every frame.
pub const PREVIEW_BYTES: usize = 64 * 1024;
const POINTS: usize = 4096;
const WIDTH: f64 = 320.;
const HEIGHT: f64 = 180.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Point,
    LineString,
    Polygon,
}
#[derive(Debug, PartialEq)]
pub struct Preview {
    pub shape: Shape,
    /// View coordinates within 320x180, y down, 16-unit margins.
    pub points: Vec<(f64, f64)>,
    pub bounds: String,
}

/// Mirrors baseline `parseWkt`, with explicit size limits.
pub fn parse(value: &str) -> Result<Preview, &'static str> {
    if value.len() > PREVIEW_BYTES {
        return Err("Geometry text is too large to preview");
    }
    let trimmed = value.trim();
    let upper = trimmed.to_ascii_uppercase();
    let (shape, rest) = [
        ("POINT", Shape::Point),
        ("LINESTRING", Shape::LineString),
        ("POLYGON", Shape::Polygon),
    ]
    .into_iter()
    .find_map(|(prefix, shape)| {
        upper
            .starts_with(prefix)
            .then(|| (shape, trimmed[prefix.len()..].trim_start()))
    })
    .ok_or("Supports POINT, LINESTRING, and POLYGON WKT.")?;
    let body = rest
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or("Supports POINT, LINESTRING, and POLYGON WKT.")?;
    let body = if shape == Shape::Polygon {
        let body = body.strip_prefix('(').unwrap_or(body);
        body.strip_suffix(')').unwrap_or(body)
    } else {
        body
    };
    let mut raw = Vec::new();
    for pair in body.split(',') {
        if raw.len() == POINTS {
            return Err("Geometry preview is limited to 4,096 points");
        }
        let mut values = pair.split_whitespace().map(str::parse::<f64>);
        match (values.next(), values.next()) {
            (Some(Ok(x)), Some(Ok(y))) if x.is_finite() && y.is_finite() => raw.push((x, y)),
            _ => return Err("Coordinates must be numeric x y pairs."),
        }
    }
    let fold = |pick: fn(&(f64, f64)) -> f64, min: bool| {
        raw.iter().map(pick).fold(
            if min {
                f64::INFINITY
            } else {
                f64::NEG_INFINITY
            },
            if min { f64::min } else { f64::max },
        )
    };
    let (min_x, max_x) = (fold(|p| p.0, true), fold(|p| p.0, false));
    let (min_y, max_y) = (fold(|p| p.1, true), fold(|p| p.1, false));
    let width = if max_x - min_x == 0. {
        1.
    } else {
        max_x - min_x
    };
    let height = if max_y - min_y == 0. {
        1.
    } else {
        max_y - min_y
    };
    Ok(Preview {
        shape,
        points: raw
            .iter()
            .map(|(x, y)| {
                (
                    16. + (x - min_x) / width * (WIDTH - 32.),
                    HEIGHT - 16. - (y - min_y) / height * (HEIGHT - 32.),
                )
            })
            .collect(),
        bounds: format!("{min_x},{min_y} → {max_x},{max_y}"),
    })
}

/// Draws the preview into a fixed 320x180 box.
pub fn render(preview: &Preview) -> impl IntoElement + use<> {
    let shape = preview.shape;
    let points = preview.points.clone();
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let at = |(x, y): (f64, f64)| bounds.origin + point(px(x as f32), px(y as f32));
            let mut path = PathBuilder::stroke(px(2.));
            match shape {
                Shape::Point => {
                    let (cx, cy) = points.first().copied().unwrap_or((WIDTH / 2., HEIGHT / 2.));
                    for i in 0..=16 {
                        let a = f64::from(i) * std::f64::consts::TAU / 16.;
                        let p = at((cx + 5. * a.cos(), cy + 5. * a.sin()));
                        if i == 0 {
                            path.move_to(p);
                        } else {
                            path.line_to(p);
                        }
                    }
                }
                Shape::LineString | Shape::Polygon => {
                    let Some(first) = points.first() else {
                        return;
                    };
                    path.move_to(at(*first));
                    for p in &points[1..] {
                        path.line_to(at(*p));
                    }
                    if shape == Shape::Polygon {
                        path.line_to(at(*first));
                    }
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, style::accent());
            }
        },
    )
    .w(px(WIDTH as f32))
    .h(px(HEIGHT as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_shapes_scale_into_the_view_with_bounds() {
        let point = parse("point (10 20)").unwrap();
        assert_eq!(point.shape, Shape::Point);
        assert_eq!(point.points, vec![(16., 164.)]);
        assert_eq!(point.bounds, "10,20 → 10,20");
        let line = parse("LINESTRING(0 0, 10 5, 20 10)").unwrap();
        assert_eq!(line.points.first(), Some(&(16., 164.)));
        assert_eq!(line.points.last(), Some(&(304., 16.)));
        let polygon = parse("  POLYGON((0 0, 4 0, 4 4, 0 4, 0 0))  ").unwrap();
        assert_eq!((polygon.shape, polygon.points.len()), (Shape::Polygon, 5));
        assert_eq!(polygon.bounds, "0,0 → 4,4");
    }

    #[test]
    fn unsupported_or_malformed_text_is_refused_with_baseline_messages() {
        for text in ["MULTIPOINT((1 2))", "SRID=4326;POINT(1 2)", "POINT 1 2", ""] {
            assert_eq!(
                parse(text),
                Err("Supports POINT, LINESTRING, and POLYGON WKT."),
                "{text}"
            );
        }
        for text in [
            "POINT(1)",
            "LINESTRING(0 0, x 1)",
            "POINT(NaN 1)",
            "POINT(inf 1)",
        ] {
            assert_eq!(
                parse(text),
                Err("Coordinates must be numeric x y pairs."),
                "{text}"
            );
        }
        let many = format!(
            "LINESTRING({})",
            (0..=POINTS)
                .map(|i| format!("{i} {i}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(parse(&many).is_err());
        assert!(parse(&format!("POINT({})", " ".repeat(PREVIEW_BYTES))).is_err());
    }
}
