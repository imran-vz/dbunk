use super::*;
impl Path {
    /// Evaluates the same cubic used by the painter/exporter. Step paths use
    /// their exact segments for hit testing below.
    pub fn curve_point(&self, t: f64) -> Option<MapPoint> {
        let Self::Curve(p) = self else {
            return None;
        };
        if !t.is_finite() || !(0.0..=1.0).contains(&t) {
            return None;
        }
        let u = 1. - t;
        Some(MapPoint {
            x: u * u * u * p[0].x
                + 3. * u * u * t * p[1].x
                + 3. * u * t * t * p[2].x
                + t * t * t * p[3].x,
            y: u * u * u * p[0].y
                + 3. * u * u * t * p[1].y
                + 3. * u * t * t * p[2].y
                + t * t * t * p[3].y,
        })
    }
}
fn distance(p: MapPoint, a: MapPoint, b: MapPoint) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length = dx * dx + dy * dy;
    let t = if length == 0. {
        0.
    } else {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / length).clamp(0., 1.)
    };
    (p.x - a.x - t * dx).hypot(p.y - a.y - t * dy)
}
impl Scene {
    /// Nodes win hit testing because they paint above edges. Tolerance is in
    /// map coordinates (divide screen tolerance by camera zoom). Cubics use a
    /// fixed 64-segment approximation; no allocation or unbounded subdivision.
    pub fn hit_test(&self, key: SceneKey, point: MapPoint, tolerance: f64) -> Option<Selection> {
        if key != self.key
            || !camera::valid_point(point)
            || !tolerance.is_finite()
            || !(0.0..=256.).contains(&tolerance)
        {
            return None;
        }
        if let Some(node) = self.hit_node(key, point) {
            return Some(node);
        }
        self.edges
            .iter()
            .rev()
            .find(|edge| {
                edge.label_bounds().contains(point)
                    || match &edge.path {
                        Path::Step(points) => points
                            .windows(2)
                            .any(|p| distance(point, p[0], p[1]) <= tolerance),
                        path @ Path::Curve(points) => {
                            let mut previous = points[0];
                            (1..=64).any(|i| {
                                let next = path.curve_point(f64::from(i) / 64.).unwrap();
                                let hit = distance(point, previous, next) <= tolerance;
                                previous = next;
                                hit
                            })
                        }
                    }
            })
            .map(|e| Selection::Edge {
                key,
                identity: e.identity,
            })
    }
}
