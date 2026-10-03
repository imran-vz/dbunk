use super::*;
pub const MAX_EXPORT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EXPORT_PIXELS: u64 = 8_000_000;
/// UI owner retains this lease until a file job using take_bytes() has joined.
/// Dropping an export waiter must not drop this owner while its worker runs.
pub struct PreparedSvg {
    bytes: Option<Vec<u8>>,
    pub viewport: Viewport,
    pub key: SceneKey,
    _lease: Lease,
}
impl PreparedSvg {
    pub fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }
    pub fn take_bytes(&mut self) -> Result<Vec<u8>, &'static str> {
        self.bytes
            .take()
            .ok_or("SVG bytes already transferred to the owned file worker")
    }
}
struct Output {
    count: usize,
    text: Option<String>,
}
impl Write for Output {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.count = self
            .count
            .checked_add(text.len())
            .filter(|n| *n <= MAX_EXPORT_BYTES)
            .ok_or(fmt::Error)?;
        if let Some(out) = &mut self.text {
            out.push_str(text);
        }
        Ok(())
    }
}
fn xml(w: &mut impl Write, text: &str) -> fmt::Result {
    for ch in text.chars() {
        match ch {
            '&' => w.write_str("&amp;")?,
            '<' => w.write_str("&lt;")?,
            '>' => w.write_str("&gt;")?,
            '"' => w.write_str("&quot;")?,
            '\'' => w.write_str("&apos;")?,
            '\n' => w.write_str("&#10;")?,
            '\r' => w.write_str("&#13;")?,
            '\t' => w.write_str("&#9;")?,
            c if c.is_control() => write!(w, "[U+{:04X}]", u32::from(c))?,
            c => w.write_char(c)?,
        }
    }
    Ok(())
}
impl Scene {
    /// Baseline export is the current viewport/camera on white, not fit-all and
    /// not a screen capture. The caller can explicitly supply a fitted camera.
    /// PNG must rasterize these same bytes at viewport.raster_size(2), after
    /// separately reserving parser/glyph/raster/encoder working memory.
    pub fn svg(&self, viewport: Viewport) -> Result<PreparedSvg, &'static str> {
        viewport.validate()?;
        let mut measure = Output {
            count: 0,
            text: None,
        };
        self.write_svg(viewport, &mut measure)
            .map_err(|_| "SVG exceeds 8 MiB or contains inconsistent scene geometry")?;
        let retained = measure
            .count
            .checked_add(size_of::<PreparedSvg>() + 4096)
            .ok_or("SVG allowance overflow")?;
        let lease = Lease::new(self.lease.budget.clone(), retained)?;
        let mut output = Output {
            count: 0,
            text: Some(String::with_capacity(measure.count)),
        };
        self.write_svg(viewport, &mut output)
            .map_err(|_| "SVG formatting failed")?;
        let bytes = output.text.unwrap().into_bytes();
        if bytes.capacity() > retained {
            return Err("SVG allocation exceeds admitted allowance");
        }
        Ok(PreparedSvg {
            bytes: Some(bytes),
            viewport,
            key: self.key,
            _lease: lease,
        })
    }
    fn write_svg(&self, v: Viewport, w: &mut impl Write) -> fmt::Result {
        writeln!(
            w,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" overflow=\"hidden\">",
            v.width, v.height, v.width, v.height
        )?;
        w.write_str("<title>PostgreSQL schema map: ")?;
        xml(w, &self.snapshot.database)?;
        w.write_str("</title><desc>Captured ")?;
        xml(w, &self.snapshot.captured_at)?;
        writeln!(
            w,
            ". Current viewport; read-only catalog relationships. Database OID {}.</desc>",
            self.snapshot.database_oid
        )?;
        writeln!(
            w,
            "<rect width=\"100%\" height=\"100%\" fill=\"white\"/><g transform=\"translate({} {}) scale({})\" font-family=\"monospace\" font-size=\"12\" fill=\"black\">",
            v.camera.pan.x, v.camera.pan.y, v.camera.zoom
        )?;
        let world = Rect {
            x: -v.camera.pan.x / v.camera.zoom,
            y: -v.camera.pan.y / v.camera.zoom,
            width: f64::from(v.width) / v.camera.zoom,
            height: f64::from(v.height) / v.camera.zoom,
        };
        for edge in &self.edges {
            let bound = edge.path.bounds();
            let expanded = Rect {
                x: bound.x - 32.,
                y: bound.y - 32.,
                width: bound.width + 64.,
                height: bound.height + 64.,
            };
            let label_bounds = edge.label_bounds();
            if !expanded.union(label_bounds).intersects(world) {
                continue;
            }
            write!(
                w,
                "<g data-constraint-oid=\"{}\"><title>",
                edge.identity.constraint_oid
            )?;
            let fk = &self.snapshot.foreign_keys[edge.foreign_key_index];
            xml(w, &fk.name)?;
            w.write_str("</title><path d=\"")?;
            path(w, &edge.path)?;
            w.write_str("\" fill=\"none\" stroke=\"#444\" stroke-width=\"1.5\"/>")?;
            marker(w, &edge.source_geometry)?;
            marker(w, &edge.target_geometry)?;
            write!(
                w,
                "<clipPath id=\"edge-label-{}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath><g clip-path=\"url(#edge-label-{})\">",
                edge.identity.constraint_oid,
                label_bounds.x,
                label_bounds.y,
                label_bounds.width,
                label_bounds.height,
                edge.identity.constraint_oid
            )?;
            write!(
                w,
                "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-size=\"{}\" paint-order=\"stroke\" stroke=\"white\" stroke-width=\"3\" stroke-linejoin=\"round\">",
                edge.label_position.x, edge.label_position.y, EDGE_LABEL_FONT_SIZE
            )?;
            xml(w, &edge.label)?;
            w.write_str("</text></g></g>\n")?;
        }
        for node in &self.nodes {
            if !node.bounds.intersects(world) {
                continue;
            }
            let r = node.bounds;
            let id = node.identity.relation_oid;
            write!(w, "<g data-relation-oid=\"{id}\"><title>")?;
            let t = &self.snapshot.tables[node.table_index];
            xml(w, &t.schema)?;
            w.write_str(".")?;
            xml(w, &t.name)?;
            writeln!(
                w,
                "</title><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"white\" stroke=\"black\"/><clipPath id=\"node-{id}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath><g clip-path=\"url(#node-{id})\">",
                r.x,
                r.y,
                r.width,
                r.height,
                r.x + 8.,
                r.y + 1.,
                r.width - 16.,
                r.height - 2.
            )?;
            write!(
                w,
                "<text x=\"{}\" y=\"{}\" font-weight=\"bold\">",
                r.x + 8.,
                r.y + 18.
            )?;
            xml(w, &node.title)?;
            w.write_str("</text>")?;
            write!(
                w,
                "<text x=\"{}\" y=\"{}\" font-size=\"10\">{} columns{}{}",
                r.x + 8.,
                r.y + 35.,
                t.columns.len(),
                if node.junction { " | junction" } else { "" },
                if node.external { " | external" } else { "" }
            )?;
            if node.trigger_count > 0 {
                write!(w, " | {} triggers", node.trigger_count)?;
            }
            w.write_str("</text>")?;
            for row in &node.rows {
                write!(
                    w,
                    "<text x=\"{}\" y=\"{}\">",
                    r.x + 8.,
                    r.y + row.center_y + 4.
                )?;
                xml(w, &row.label)?;
                if row.foreign_key {
                    w.write_str("  FK")?;
                }
                if row.trigger_count > 0 {
                    write!(w, "  T{}", row.trigger_count)?;
                }
                w.write_str("</text>")?;
            }
            writeln!(
                w,
                "</g><path d=\"M {} {} H {}\" stroke=\"#777\"/></g>",
                r.x,
                r.y + HEADER_HEIGHT,
                r.x + r.width
            )?;
        }
        w.write_str("</g></svg>\n")
    }
}
fn path(w: &mut impl Write, path: &Path) -> fmt::Result {
    match path {
        Path::Curve(p) => write!(
            w,
            "M {} {} C {} {}, {} {}, {} {}",
            p[0].x, p[0].y, p[1].x, p[1].y, p[2].x, p[2].y, p[3].x, p[3].y
        ),
        Path::Step(p) => {
            let Some(first) = p.first() else {
                return Err(fmt::Error);
            };
            write!(w, "M {} {}", first.x, first.y)?;
            for point in &p[1..] {
                write!(w, " L {} {}", point.x, point.y)?;
            }
            Ok(())
        }
    }
}
fn marker(w: &mut impl Write, geometry: &MarkerGeometry) -> fmt::Result {
    for line in geometry.segments.iter().flatten() {
        write!(
            w,
            "<path d=\"M {} {} L {} {}\" fill=\"none\" stroke=\"#444\" stroke-width=\"1.5\"/>",
            line[0].x, line[0].y, line[1].x, line[1].y
        )?;
    }
    if let Some((point, radius)) = geometry.circle {
        write!(
            w,
            "<circle cx=\"{}\" cy=\"{}\" r=\"{radius}\" fill=\"white\" stroke=\"#444\"/>",
            point.x, point.y
        )?;
    }
    Ok(())
}
