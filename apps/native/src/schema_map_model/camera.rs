use super::*;
pub const MIN_ZOOM: f64 = 0.0001;
pub const MAX_ZOOM: f64 = 4.;
pub(super) fn valid_point(p: MapPoint) -> bool {
    p.x.is_finite()
        && p.y.is_finite()
        && p.x.abs() <= MAX_MAP_POSITION_COORDINATE
        && p.y.abs() <= MAX_MAP_POSITION_COORDINATE
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub fn valid(self) -> bool {
        valid_point(MapPoint {
            x: self.x,
            y: self.y,
        }) && self.width.is_finite()
            && self.height.is_finite()
            && self.width >= 0.
            && self.height >= 0.
            && valid_point(MapPoint {
                x: self.x + self.width,
                y: self.y + self.height,
            })
    }
    pub fn contains(self, p: MapPoint) -> bool {
        p.x >= self.x && p.y >= self.y && p.x <= self.x + self.width && p.y <= self.y + self.height
    }
    pub fn intersects(self, other: Self) -> bool {
        self.x <= other.x + other.width
            && self.x + self.width >= other.x
            && self.y <= other.y + other.height
            && self.y + self.height >= other.y
    }
    pub fn from_points(points: &[MapPoint]) -> Option<Self> {
        let first = points.first()?;
        if !points.iter().copied().all(valid_point) {
            return None;
        }
        let (mut x, mut y, mut right, mut bottom) = (first.x, first.y, first.x, first.y);
        for p in points {
            x = x.min(p.x);
            y = y.min(p.y);
            right = right.max(p.x);
            bottom = bottom.max(p.y);
        }
        Some(Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            width: (self.x + self.width).max(other.x + other.width) - x,
            height: (self.y + self.height).max(other.y + other.height) - y,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub pan: MapPoint,
    pub zoom: f64,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            pan: MapPoint { x: 0., y: 0. },
            zoom: 1.,
        }
    }
}
impl Camera {
    pub fn valid(self) -> bool {
        self.zoom.is_finite()
            && (MIN_ZOOM..=MAX_ZOOM).contains(&self.zoom)
            && self.pan.x.is_finite()
            && self.pan.y.is_finite()
            && self.pan.x.abs() <= 8. * MAX_MAP_POSITION_COORDINATE
            && self.pan.y.abs() <= 8. * MAX_MAP_POSITION_COORDINATE
    }
    pub fn to_screen(self, world: MapPoint) -> Option<MapPoint> {
        if !self.valid() || !valid_point(world) {
            return None;
        }
        Some(MapPoint {
            x: world.x * self.zoom + self.pan.x,
            y: world.y * self.zoom + self.pan.y,
        })
    }
    pub fn to_world(self, screen: MapPoint) -> Option<MapPoint> {
        if !self.valid() || !screen.x.is_finite() || !screen.y.is_finite() {
            return None;
        }
        let p = MapPoint {
            x: (screen.x - self.pan.x) / self.zoom,
            y: (screen.y - self.pan.y) / self.zoom,
        };
        valid_point(p).then_some(p)
    }
    /// Local viewport coordinates, not window/screen coordinates. Keeps the
    /// map position under the pointer fixed across a clamped zoom change.
    pub fn zoom_at(self, pointer: MapPoint, zoom: f64) -> Result<Self, &'static str> {
        if !zoom.is_finite() || zoom <= 0. {
            return Err("Invalid map zoom");
        }
        let world = self
            .to_world(pointer)
            .ok_or("Invalid map pointer geometry")?;
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let next = Self {
            zoom,
            pan: MapPoint {
                x: pointer.x - world.x * zoom,
                y: pointer.y - world.y * zoom,
            },
        };
        if next.valid() {
            Ok(next)
        } else {
            Err("Map camera exceeds finite bounds")
        }
    }
    pub fn pan_by(self, delta: MapPoint) -> Result<Self, &'static str> {
        let next = Self {
            pan: MapPoint {
                x: self.pan.x + delta.x,
                y: self.pan.y + delta.y,
            },
            ..self
        };
        if next.valid() {
            Ok(next)
        } else {
            Err("Map camera exceeds finite bounds")
        }
    }
    pub fn fit(bounds: Rect, width: f64, height: f64) -> Result<Self, &'static str> {
        if !bounds.valid()
            || !width.is_finite()
            || !height.is_finite()
            || width <= 0.
            || height <= 0.
        {
            return Err("Invalid fit geometry");
        }
        let zoom = (width / (bounds.width.max(1.) * 1.36))
            .min(height / (bounds.height.max(1.) * 1.36))
            .min(1.);
        if zoom < MIN_ZOOM {
            return Err("Map extent exceeds minimum fit zoom");
        }
        let next = Self {
            zoom,
            pan: MapPoint {
                x: width / 2. - (bounds.x + bounds.width / 2.) * zoom,
                y: height / 2. - (bounds.y + bounds.height / 2.) * zoom,
            },
        };
        if next.valid() {
            Ok(next)
        } else {
            Err("Map camera exceeds finite bounds")
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub camera: Camera,
}
impl Viewport {
    pub fn validate(self) -> Result<(), &'static str> {
        if self.width == 0
            || self.height == 0
            || self.width > 16_384
            || self.height > 16_384
            || !self.camera.valid()
        {
            Err("Invalid map viewport dimensions or camera")
        } else {
            Ok(())
        }
    }
    pub fn raster_size(self, scale: u32) -> Result<(u32, u32), &'static str> {
        self.validate()?;
        if scale == 0 {
            return Err("Invalid map raster scale");
        }
        let width = self
            .width
            .checked_mul(scale)
            .ok_or("Map raster dimensions overflow")?;
        let height = self
            .height
            .checked_mul(scale)
            .ok_or("Map raster dimensions overflow")?;
        if width > 8192 || height > 8192 || u64::from(width) * u64::from(height) > MAX_EXPORT_PIXELS
        {
            return Err("Map PNG exceeds 8 million pixels; use SVG or a smaller viewport");
        }
        Ok((width, height))
    }
}
