use super::*;
use gpui::AssetSource;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
const MAX_TEXT_CHARS: usize = 65_536;
const MAX_NODES: usize = 65_536;
const MAX_POINTS: usize = 2_000_000;
const MAX_DEPTH: usize = 32;

pub(super) fn parse(
    svg: &[u8],
    logical: (u32, u32),
    cancel: &Cancellation,
) -> Result<usvg::Tree, PngError> {
    let text = std::str::from_utf8(svg).map_err(|_| PngError::Parse)?;
    // This is the app-generated Scene SVG dialect, never arbitrary imported SVG.
    // Reject extra resources/features before usvg can resolve them.
    let xml = usvg::roxmltree::Document::parse_with_options(
        text,
        usvg::roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES as u32,
            ..Default::default()
        },
    )
    .map_err(|_| PngError::Parse)?;
    let mut chars = 0usize;
    for node in xml.descendants().filter(|n| n.is_element()) {
        if node.ancestors().count() > MAX_DEPTH {
            return Err(PngError::Unsupported);
        }
        if !matches!(
            node.tag_name().name(),
            "svg" | "title" | "desc" | "rect" | "g" | "path" | "circle" | "text" | "clipPath"
        ) {
            return Err(PngError::Unsupported);
        }
        if node.tag_name().name() == "text" {
            for child in node.descendants().filter(|n| n.is_text()) {
                chars = chars
                    .checked_add(child.text().unwrap_or("").chars().count())
                    .filter(|n| *n <= MAX_TEXT_CHARS)
                    .ok_or(PngError::Unsupported)?;
            }
        }
    }
    drop(xml);
    check(cancel)?;
    let mut fontdb = usvg::fontdb::Database::new();
    for path in [
        "fonts/lilex/Lilex-Regular.ttf",
        "fonts/lilex/Lilex-Bold.ttf",
        "fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf",
        "fonts/ibm-plex-sans/IBMPlexSans-SemiBold.ttf",
    ] {
        let data = assets::Assets
            .load(path)
            .map_err(|_| PngError::Fonts)?
            .ok_or(PngError::Fonts)?;
        if data.len() > 512 * 1024 {
            return Err(PngError::Fonts);
        }
        fontdb.load_font_data(data.into_owned());
    }
    fontdb.set_monospace_family("Lilex");
    fontdb.set_sans_serif_family("IBM Plex Sans");
    let missing = Arc::new(AtomicBool::new(false));
    let selector = usvg::FontResolver::default_font_selector();
    let fallback = usvg::FontResolver::default_fallback_selector();
    let select_missing = missing.clone();
    let fallback_missing = missing.clone();
    let options = usvg::Options {
        fontdb: Arc::new(fontdb),
        font_resolver: usvg::FontResolver {
            select_font: Box::new(move |font, db| {
                let result = selector(font, db);
                if result.is_none() {
                    select_missing.store(true, Ordering::Relaxed);
                }
                result
            }),
            select_fallback: Box::new(move |ch, fonts, db| {
                let result = fallback(ch, fonts, db);
                if result.is_none() {
                    fallback_missing.store(true, Ordering::Relaxed);
                }
                result
            }),
        },
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(svg, &options).map_err(|_| PngError::Parse)?;
    check(cancel)?;
    if missing.load(Ordering::Relaxed) {
        return Err(PngError::MissingGlyph);
    }
    if tree.size().width() != logical.0 as f32 || tree.size().height() != logical.1 as f32 {
        return Err(PngError::Dimensions);
    }
    Ok(tree)
}
#[derive(Default)]
struct Measure {
    nodes: usize,
    points: usize,
    largest_path: usize,
}
impl Measure {
    fn path(&mut self, path: &usvg::Path) -> Result<(), PngError> {
        let points = path.data().points().len();
        self.points = self
            .points
            .checked_add(points)
            .filter(|n| *n <= MAX_POINTS)
            .ok_or(PngError::Unsupported)?;
        self.largest_path = self.largest_path.max(points);
        Ok(())
    }
    fn group(
        &mut self,
        group: &usvg::Group,
        parent: tiny_skia::Transform,
        size: (u32, u32),
        depth: usize,
    ) -> Result<usize, PngError> {
        self.nodes = self
            .nodes
            .checked_add(1)
            .filter(|n| *n <= MAX_NODES)
            .ok_or(PngError::Unsupported)?;
        if depth > MAX_DEPTH
            || group.mask().is_some()
            || !group.filters().is_empty()
            || group.blend_mode() != usvg::BlendMode::Normal
        {
            return Err(PngError::Unsupported);
        }
        let transform = parent.pre_concat(group.transform());
        let mut child_peak = 0usize;
        for node in group.children() {
            self.nodes = self
                .nodes
                .checked_add(1)
                .filter(|n| *n <= MAX_NODES)
                .ok_or(PngError::Unsupported)?;
            let child = match node {
                usvg::Node::Group(child) => self.group(child, transform, size, depth + 1)?,
                usvg::Node::Path(path) => {
                    self.path(path)?;
                    0
                }
                usvg::Node::Image(_) => return Err(PngError::Unsupported),
                usvg::Node::Text(text) => {
                    if text
                        .layouted()
                        .iter()
                        .flat_map(|span| &span.positioned_glyphs)
                        .any(|glyph| glyph.id.0 == 0)
                    {
                        return Err(PngError::MissingGlyph);
                    }
                    self.group(text.flattened(), transform, size, depth + 1)?
                }
            };
            child_peak = child_peak.max(child);
        }
        if !group.should_isolate() {
            return Ok(child_peak);
        }
        // resvg0.46: ceil(width)+4/ceil(height)+4 and clamp to a 5x
        // root-canvas box. Ignoring its position deliberately overestimates
        // nested layers whose coordinate origin shifts to the parent pixmap.
        let bbox = group
            .layer_bounding_box()
            .transform(transform)
            .ok_or(PngError::Unsupported)?;
        let area = layer_area(bbox.width(), bbox.height(), size)?;
        let own = area.checked_mul(4).ok_or(PngError::WorkingLimit)?;
        let clip = if let Some(clip) = group.clip_path() {
            self.clip(clip, 0)?;
            // clip::apply retains 4B clip pixmap while allocating its 1B mask;
            // the group's own 4B pixmap stays alive throughout.
            area.checked_mul(5).ok_or(PngError::WorkingLimit)?
        } else {
            0
        };
        own.checked_add(child_peak.max(clip))
            .ok_or(PngError::WorkingLimit)
    }
    fn clip(&mut self, clip: &usvg::ClipPath, depth: usize) -> Result<(), PngError> {
        if clip.clip_path().is_some() {
            return Err(PngError::Unsupported);
        }
        self.clip_group(clip.root(), depth)
    }
    fn clip_group(&mut self, group: &usvg::Group, depth: usize) -> Result<(), PngError> {
        if depth > MAX_DEPTH || group.should_isolate() {
            return Err(PngError::Unsupported);
        }
        for node in group.children() {
            match node {
                usvg::Node::Path(path) => self.path(path)?,
                usvg::Node::Group(child) => self.clip_group(child, depth + 1)?,
                _ => return Err(PngError::Unsupported),
            }
        }
        Ok(())
    }
}
fn layer_area(width: f32, height: f32, size: (u32, u32)) -> Result<usize, PngError> {
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
        return Err(PngError::Unsupported);
    }
    let width = (f64::from(width).ceil() + 4.).min(f64::from(size.0) * 5.);
    let height = (f64::from(height).ceil() + 4.).min(f64::from(size.1) * 5.);
    (width as usize)
        .checked_mul(height as usize)
        .ok_or(PngError::WorkingLimit)
}
pub(super) fn raster_working(tree: &usvg::Tree, size: (u32, u32)) -> Result<usize, PngError> {
    let mut measure = Measure::default();
    let layers = measure.group(
        tree.root(),
        tiny_skia::Transform::from_scale(2., 2.),
        size,
        0,
    )?;
    // tiny-skia clones/transforms paths and expands stroked outlines. Charge
    // separate bounded scratch from measured path complexity, not codec space.
    let scratch = measure
        .largest_path
        .checked_mul(256)
        .ok_or(PngError::WorkingLimit)?;
    layers.checked_add(scratch).ok_or(PngError::WorkingLimit)
}
