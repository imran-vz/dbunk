//! PNG from the exact prepared viewport SVG, at 2x on its white canvas.
//! Run only on the owned FileRuntime worker after the caller reserves the plan's
//! working ceiling and retains PreparedSvg's separate source lease until join.
//!
//! Source/raster/output buffers and parsed layer dimensions have checked bounds.
//! The parser/font working reservation is conservative accounting, not a proof
//! of every usvg allocation or total process RSS. Only bundled fonts are loaded.
use crate::schema_map_model::PreparedSvg;
use dbunk_lib::backend::result_files::Cancellation;
use resvg::{tiny_skia, usvg};
use std::{
    fmt,
    io::{self, Write},
};
#[cfg(test)]
mod tests;
mod tree;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 8_000_000;
pub const MAX_DIMENSION: u32 = 8192;
pub const MAX_WORKING_BYTES: usize = 128 * 1024 * 1024;
const FIXED_WORKING: usize = 8 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PngError {
    Source,
    Dimensions,
    WorkingLimit,
    Unsupported,
    MissingGlyph,
    Fonts,
    Parse,
    Render,
    Encode,
    OutputLimit,
    Cancelled,
}
impl fmt::Display for PngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Source=>"Map SVG is missing or exceeds 8 MiB",
            Self::Dimensions=>"Map PNG exceeds 8 million pixels or 8192 pixels per axis at 2x; use SVG or a smaller viewport",
            Self::WorkingLimit=>"Map PNG needs more renderer working allowance; use SVG or a smaller viewport",
            Self::Unsupported=>"Map SVG contains unsupported rendering features or exceeds text/path complexity limits; use SVG",
            Self::MissingGlyph=>"Bundled PNG fonts cannot render every map character; export SVG for system-font fallback",
            Self::Fonts=>"Bundled map fonts are unavailable or exceed their bounds",
            Self::Parse=>"Map SVG could not be parsed for PNG",
            Self::Render=>"Map PNG raster dimensions or pixel buffer did not match the prepared viewport",
            Self::Encode=>"Map PNG encoding failed",
            Self::OutputLimit=>"Encoded map PNG exceeds 8 MiB; use SVG or a smaller viewport",
            Self::Cancelled=>"Map PNG export cancelled before publication",
        })
    }
}
impl std::error::Error for PngError {}
#[derive(Clone, Copy, Debug)]
pub struct PngPlan {
    logical: (u32, u32),
    dimensions: (u32, u32),
    svg_len: usize,
    rgba_bytes: usize,
    base_bytes: usize,
    working_limit: usize,
}
impl PngPlan {
    pub fn new(svg: &PreparedSvg) -> Result<Self, PngError> {
        svg.viewport
            .raster_size(2)
            .map_err(|_| PngError::Dimensions)?;
        Self::for_size(
            svg.viewport.width,
            svg.viewport.height,
            svg.bytes().ok_or(PngError::Source)?.len(),
        )
    }
    fn for_size(width: u32, height: u32, svg_len: usize) -> Result<Self, PngError> {
        if svg_len == 0 || svg_len > MAX_BYTES {
            return Err(PngError::Source);
        }
        let width2 = width.checked_mul(2).ok_or(PngError::Dimensions)?;
        let height2 = height.checked_mul(2).ok_or(PngError::Dimensions)?;
        let pixels = u64::from(width2)
            .checked_mul(u64::from(height2))
            .ok_or(PngError::Dimensions)?;
        if width2 == 0
            || height2 == 0
            || width2 > MAX_DIMENSION
            || height2 > MAX_DIMENSION
            || pixels > MAX_PIXELS
        {
            return Err(PngError::Dimensions);
        }
        let rgba_bytes = usize::try_from(pixels)
            .ok()
            .and_then(|n| n.checked_mul(4))
            .ok_or(PngError::Dimensions)?;
        let base_bytes = rgba_bytes
            .checked_mul(2)
            .and_then(|n| n.checked_add(MAX_BYTES + FIXED_WORKING))
            .and_then(|n| n.checked_add(svg_len.checked_mul(64)?))
            .filter(|n| *n <= MAX_WORKING_BYTES)
            .ok_or(PngError::WorkingLimit)?;
        Ok(Self {
            logical: (width, height),
            dimensions: (width2, height2),
            svg_len,
            rgba_bytes,
            base_bytes,
            working_limit: base_bytes,
        })
    }
    pub fn dimensions(&self) -> (u32, u32) {
        self.dimensions
    }
    /// Minimum allowance, before glyph paths and isolated layers are measured.
    pub fn required_bytes(&self) -> usize {
        self.base_bytes
    }
    /// The caller must admit this entire ceiling before worker dispatch. More
    /// than the minimum permits complex clipped labels, without changing SVG.
    pub fn with_working_limit(mut self, bytes: usize) -> Result<Self, PngError> {
        if bytes < self.base_bytes || bytes > MAX_WORKING_BYTES {
            return Err(PngError::WorkingLimit);
        }
        self.working_limit = bytes;
        Ok(self)
    }
    pub fn encode(self, svg: Vec<u8>, cancel: &Cancellation) -> Result<Vec<u8>, PngError> {
        check(cancel)?;
        if svg.len() != self.svg_len || svg.capacity() > MAX_BYTES {
            return Err(PngError::Source);
        }
        let parsed = tree::parse(&svg, self.logical, cancel)?;
        let extra = tree::raster_working(&parsed, self.dimensions)?;
        if self
            .base_bytes
            .checked_add(extra)
            .is_none_or(|n| n > self.working_limit)
        {
            return Err(PngError::WorkingLimit);
        }
        check(cancel)?;
        let (width, height) = self.dimensions;
        let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(PngError::Render)?;
        resvg::render(
            &parsed,
            tiny_skia::Transform::from_scale(2., 2.),
            &mut pixmap.as_mut(),
        );
        check(cancel)?;
        if pixmap.width() != width
            || pixmap.height() != height
            || pixmap.data().len() != self.rgba_bytes
        {
            return Err(PngError::Render);
        }
        drop(parsed);
        drop(svg);
        // Direct resvg is premultiplied RGBA. GPUI would swap R/B and
        // unpremultiply; bypassing GPUI means NO BGRA swap here. Composite any
        // residual transparent edge onto white using premultiplied channels.
        let mut rgba = Vec::with_capacity(self.rgba_bytes);
        for part in pixmap.data().chunks(CHUNK) {
            check(cancel)?;
            for pixel in part.as_chunks::<4>().0 {
                let white = 255 - pixel[3];
                rgba.extend_from_slice(&[
                    pixel[0].saturating_add(white),
                    pixel[1].saturating_add(white),
                    pixel[2].saturating_add(white),
                    255,
                ]);
            }
        }
        drop(pixmap);
        encode_rgba(&rgba, self.dimensions, cancel)
    }
}
fn check(cancel: &Cancellation) -> Result<(), PngError> {
    if cancel.is_cancelled() {
        Err(PngError::Cancelled)
    } else {
        Ok(())
    }
}
struct Output<'a> {
    bytes: Vec<u8>,
    cancel: &'a Cancellation,
    limited: bool,
}
impl Write for Output<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("cancelled"));
        }
        if data.len() > MAX_BYTES.saturating_sub(self.bytes.len()) {
            self.limited = true;
            return Err(io::Error::other("PNG bound"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encode_rgba(
    rgba: &[u8],
    dimensions: (u32, u32),
    cancel: &Cancellation,
) -> Result<Vec<u8>, PngError> {
    check(cancel)?;
    let mut output = Output {
        bytes: Vec::with_capacity(MAX_BYTES),
        cancel,
        limited: false,
    };
    let result = (|| {
        let mut encoder = png::Encoder::new(&mut output, dimensions.0, dimensions.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Balanced);
        let mut writer = encoder.write_header()?;
        {
            let mut stream = writer.stream_writer_with_size(CHUNK)?;
            for bytes in rgba.chunks(CHUNK) {
                stream.write_all(bytes)?;
            }
            stream.finish()?;
        }
        writer.finish()
    })();
    check(cancel)?;
    if output.limited {
        return Err(PngError::OutputLimit);
    }
    result.map_err(|_| PngError::Encode)?;
    Ok(output.bytes)
}
