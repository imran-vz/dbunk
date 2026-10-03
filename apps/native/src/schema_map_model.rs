//! Pure bounded relationship scene. Geometry is in map coordinates and is shared
//! by the GPUI canvas and SVG/PNG exports; no renderer performs another layout.
pub use dbunk_lib::backend::schema_map::preferences::{
    MAX_MAP_POSITION_COORDINATE, MapAttributes, MapPoint, MapPrefs, MapRouting, SavedPosition,
};
use dbunk_lib::backend::schema_map::*;
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    fmt::{self, Write},
    mem::size_of,
    rc::Rc,
    sync::Arc,
};
mod camera;
mod detail_pages;
mod details;
mod drag;
mod hit;
mod layout;
mod svg;
#[cfg(test)]
mod tests;
pub use camera::{Camera, Rect, Viewport};
pub use svg::{MAX_EXPORT_PIXELS, PreparedSvg};
const SHARED_BYTES: usize = 128 * 1024 * 1024;
const SCRATCH_BYTES: usize = 2 * 1024 * 1024;
const PRESENTATION_FIXED: usize = 128 * 1024;
const DETAIL_ALLOWANCE: usize = 64 * 1024 * 64 + 128 * 4096;
pub const NODE_WIDTH: f64 = 320.;
pub const HEADER_HEIGHT: f64 = 46.;
pub const ROW_HEIGHT: f64 = 22.;
pub const EDGE_LABEL_FONT_SIZE: f64 = 10.;
pub const EDGE_LABEL_BASELINE: f64 = 14.;
pub const EDGE_LABEL_HEIGHT: f64 = 20.;
const EDGE_LABEL_PADDING: f64 = 16.;
fn label_advance(ch: char) -> usize {
    if ch.is_ascii() { 7 } else { 12 }
}
fn edge_label_width(text: &str) -> f64 {
    text.chars().map(label_advance).sum::<usize>() as f64 + EDGE_LABEL_PADDING
}
fn caption_bounds(position: MapPoint, width: f64) -> Rect {
    Rect {
        x: position.x - width / 2.,
        y: position.y - EDGE_LABEL_BASELINE,
        width,
        height: EDGE_LABEL_HEIGHT,
    }
}
impl Edge {
    /// The shared caption box clips fallback glyphs in both canvas and SVG.
    /// Width is a bounded layout measure, not a font-allocation bound.
    pub fn label_bounds(&self) -> Rect {
        caption_bounds(self.label_position, edge_label_width(&self.label))
    }
}
pub const MAX_DETAIL_BYTES: usize = 32 * 1024;
pub const MAX_DETAIL_LINES: usize = 128;
const LABEL_CHARS: usize = 96;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneKey {
    pub document_generation: u64,
    pub capture_generation: u64,
    pub layout_revision: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeIdentity {
    pub database_oid: u32,
    pub constraint_oid: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Node {
        key: SceneKey,
        identity: SchemaMapIdentity,
    },
    Edge {
        key: SceneKey,
        identity: EdgeIdentity,
    },
}
#[derive(Clone, Debug)]
pub struct ColumnRow {
    pub attnum: i16,
    pub label: String,
    pub foreign_key: bool,
    pub trigger_count: usize,
    pub center_y: f64,
}
#[derive(Clone, Debug)]
pub struct Node {
    pub identity: SchemaMapIdentity,
    pub table_index: usize,
    pub bounds: Rect,
    pub title: String,
    pub rows: Vec<ColumnRow>,
    pub junction: bool,
    pub external: bool,
    pub trigger_count: usize,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Path {
    Curve([MapPoint; 4]),
    Step(Vec<MapPoint>),
}
impl Path {
    pub fn points(&self) -> &[MapPoint] {
        match self {
            Self::Curve(p) => p,
            Self::Step(p) => p,
        }
    }
    pub fn bounds(&self) -> Rect {
        Rect::from_points(self.points()).expect("admitted finite path")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    One,
    ZeroOrOne,
    Many,
}
#[derive(Clone, Debug)]
pub struct MarkerGeometry {
    pub segments: [Option<[MapPoint; 2]>; 3],
    pub circle: Option<(MapPoint, f64)>,
}
#[derive(Clone, Debug)]
pub struct Edge {
    pub identity: EdgeIdentity,
    pub foreign_key_index: usize,
    pub source: usize,
    pub target: usize,
    pub path: Path,
    pub label: String,
    pub label_position: MapPoint,
    pub source_marker: Marker,
    pub target_marker: Marker,
    pub source_geometry: MarkerGeometry,
    pub target_geometry: MarkerGeometry,
}
struct Lease {
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Lease {
    fn new(budget: Rc<Cell<usize>>, bytes: usize) -> Result<Self, &'static str> {
        if bytes > SHARED_BYTES.saturating_sub(budget.get()) {
            return Err(
                "Schema map needs more shared allowance; clear a capture or show fewer attributes",
            );
        }
        budget.set(budget.get() + bytes);
        Ok(Self { budget, bytes })
    }
    fn shrink(&mut self, bytes: usize) {
        assert!(bytes <= self.bytes);
        self.budget.set(self.budget.get() - (self.bytes - bytes));
        self.bytes = bytes;
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}
pub struct Scene {
    snapshot: Arc<SchemaMapSnapshot>,
    key: SceneKey,
    prefs: MapPrefs,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    bounds: Rect,
    lease: Lease,
}
impl Scene {
    /// Caller retains its old scene until this succeeds. Snapshot ownership is
    /// shared without deep cloning, but every retained scene charges its complete
    /// reachable capture conservatively. Incoming delivery needs its own lease.
    pub fn new(
        snapshot: Arc<SchemaMapSnapshot>,
        key: SceneKey,
        prefs: MapPrefs,
        positions: &[SavedPosition],
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        let capture_bytes = snapshot
            .checked_heap_bytes()
            .ok_or("Invalid or oversized schema-map capture")?;
        if positions.len() > MAX_SCHEMA_MAP_TABLES {
            return Err("Too many saved map positions");
        }
        let mut lease = Lease::new(
            budget,
            capture_bytes
                .checked_add(SCRATCH_BYTES)
                .ok_or("Map allowance overflow")?,
        )?;
        let mut identities = BTreeSet::new();
        for saved in positions {
            if saved.identity.database_oid != snapshot.database_oid
                || !camera::valid_point(saved.position)
                || !identities.insert(saved.identity)
            {
                return Err("Saved map position has stale identity or invalid coordinates");
            }
        }
        let keys = foreign_columns(&snapshot);
        let mut order = (0..snapshot.tables.len()).collect::<Vec<_>>();
        order.sort_by_key(|i| {
            let t = &snapshot.tables[*i];
            (&t.schema, &t.name, t.identity)
        });
        let mut label_bytes = 0usize;
        let mut rows = 0usize;
        for &i in &order {
            let t = &snapshot.tables[i];
            label_bytes = label_bytes
                .checked_add(measure_label(|w| write_title(w, t))?)
                .ok_or("Map label allowance overflow")?;
            for c in &t.columns {
                if displayed(t.identity, c, prefs, &keys) {
                    rows += 1;
                    label_bytes = label_bytes
                        .checked_add(measure_label(|w| write_column(w, c, prefs))?)
                        .ok_or("Map label allowance overflow")?;
                }
            }
        }
        for fk in &snapshot.foreign_keys {
            label_bytes = label_bytes
                .checked_add(measure_label(|w| write_edge_label(w, &snapshot, fk))?)
                .ok_or("Map label allowance overflow")?;
        }
        let labels = snapshot.tables.len() + snapshot.foreign_keys.len() + rows;
        // Covers bounded UTF-8 labels, element/text/AX presentation overlap,
        // geometry capacities, exact selected detail current/replacement, and
        // construction scratch. Refusal precedes any derived string allocation.
        let geometry = order
            .len()
            .checked_mul(size_of::<Node>() * 2)
            .and_then(|n| n.checked_add(rows.checked_mul(size_of::<ColumnRow>() * 2)?))
            .and_then(|n| {
                n.checked_add(
                    snapshot
                        .foreign_keys
                        .len()
                        .checked_mul(size_of::<Edge>() * 2 + 8 * size_of::<MapPoint>())?,
                )
            })
            .ok_or("Map geometry allowance overflow")?;
        let presentation = label_bytes
            .checked_mul(68)
            .and_then(|n| n.checked_add(labels.checked_mul(512)?))
            .and_then(|n| n.checked_add(PRESENTATION_FIXED + DETAIL_ALLOWANCE))
            .ok_or("Map presentation allowance overflow")?;
        let retained = capture_bytes
            .checked_add(geometry)
            .and_then(|n| n.checked_add(presentation))
            .and_then(|n| n.checked_add(size_of::<Self>()))
            .ok_or("Map allowance overflow")?;
        let extra = retained.saturating_sub(capture_bytes);
        let mut added = Lease::new(lease.budget.clone(), extra)?;
        // Both initial capture+scratch and final scene reservations remain held
        // while constructing. The temporary reservation is released afterward.
        let mut nodes = Vec::with_capacity(order.len());
        for i in order {
            let table = &snapshot.tables[i];
            let mut displayed_rows = Vec::new();
            for column in &table.columns {
                if displayed(table.identity, column, prefs, &keys) {
                    let center_y = HEADER_HEIGHT + ROW_HEIGHT * (displayed_rows.len() as f64 + 0.5);
                    displayed_rows.push(ColumnRow {
                        attnum: column.attnum,
                        label: make_label(|w| write_column(w, column, prefs))?,
                        foreign_key: keys.contains(&(table.identity, column.attnum)),
                        trigger_count: table
                            .triggers
                            .iter()
                            .filter(|t| t.columns.contains(&column.attnum))
                            .count(),
                        center_y,
                    });
                }
            }
            let height = HEADER_HEIGHT + ROW_HEIGHT * (displayed_rows.len().max(1) as f64);
            nodes.push(Node {
                identity: table.identity,
                table_index: i,
                bounds: Rect {
                    x: 0.,
                    y: 0.,
                    width: NODE_WIDTH,
                    height,
                },
                title: make_label(|w| write_title(w, table))?,
                rows: displayed_rows,
                junction: table.junction,
                external: table.external,
                trigger_count: table.triggers.len(),
            });
        }
        layout::place(&mut nodes, &snapshot)?;
        for saved in positions {
            if let Some(node) = nodes.iter_mut().find(|n| n.identity == saved.identity) {
                node.bounds.x = saved.position.x;
                node.bounds.y = saved.position.y;
            }
        }
        let edges = layout::edges(&nodes, &snapshot, prefs.routing)?;
        let bounds = layout::bounds(&nodes, &edges)?;
        // Replace temporary two-lease accounting with one exact conservative
        // reservation, without an uncharged interval.
        let total = lease.bytes + added.bytes;
        let budget = lease.budget.clone();
        added.bytes = 0;
        drop(added);
        lease.bytes = total;
        lease.shrink(retained);
        let scene = Self {
            snapshot,
            key,
            prefs,
            nodes,
            edges,
            bounds,
            lease,
        };
        if scene.actual_heap_bytes().ok_or("Map heap overflow")? > retained {
            return Err("Map capacity exceeds admitted allowance");
        }
        debug_assert!(budget.get() >= retained);
        Ok(scene)
    }
    pub fn snapshot(&self) -> &SchemaMapSnapshot {
        &self.snapshot
    }
    pub fn key(&self) -> SceneKey {
        self.key
    }
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }
    pub fn bounds(&self) -> Rect {
        self.bounds
    }
    #[cfg(test)]
    pub fn retained_bytes(&self) -> usize {
        self.lease.bytes
    }
    pub fn rebuild(
        &self,
        key: SceneKey,
        prefs: MapPrefs,
        positions: &[SavedPosition],
    ) -> Result<Self, &'static str> {
        Self::new(
            self.snapshot.clone(),
            key,
            prefs,
            positions,
            self.lease.budget.clone(),
        )
    }
    pub fn node_selection(&self, identity: SchemaMapIdentity) -> Option<Selection> {
        self.nodes
            .iter()
            .any(|n| n.identity == identity)
            .then_some(Selection::Node {
                key: self.key,
                identity,
            })
    }
    pub fn edge_selection(&self, identity: EdgeIdentity) -> Option<Selection> {
        self.edges
            .iter()
            .any(|e| e.identity == identity)
            .then_some(Selection::Edge {
                key: self.key,
                identity,
            })
    }
    pub fn accepts(&self, selection: Selection) -> bool {
        match selection {
            Selection::Node { key, identity } => {
                key == self.key && self.nodes.iter().any(|n| n.identity == identity)
            }
            Selection::Edge { key, identity } => {
                key == self.key && self.edges.iter().any(|e| e.identity == identity)
            }
        }
    }
    #[cfg(test)]
    pub fn anchor(
        &self,
        identity: SchemaMapIdentity,
        attnum: i16,
        right: bool,
    ) -> Option<MapPoint> {
        let n = self.nodes.iter().find(|n| n.identity == identity)?;
        self.snapshot.tables[n.table_index]
            .columns
            .iter()
            .any(|c| c.attnum == attnum)
            .then(|| layout::anchor(n, attnum, right))
    }
    pub fn hit_node(&self, key: SceneKey, point: MapPoint) -> Option<Selection> {
        if key != self.key || !camera::valid_point(point) {
            return None;
        }
        self.nodes
            .iter()
            .rev()
            .find(|n| n.bounds.contains(point))
            .map(|n| Selection::Node {
                key,
                identity: n.identity,
            })
    }
    fn actual_heap_bytes(&self) -> Option<usize> {
        let mut bytes = self
            .snapshot
            .checked_heap_bytes()?
            .checked_add(size_of::<Self>())?
            .checked_add(self.nodes.capacity().checked_mul(size_of::<Node>())?)?
            .checked_add(self.edges.capacity().checked_mul(size_of::<Edge>())?)?;
        for n in &self.nodes {
            bytes = bytes
                .checked_add(n.title.capacity())?
                .checked_add(n.rows.capacity().checked_mul(size_of::<ColumnRow>())?)?;
            for r in &n.rows {
                bytes = bytes.checked_add(r.label.capacity())?;
            }
        }
        for e in &self.edges {
            bytes = bytes.checked_add(e.label.capacity())?;
            if let Path::Step(p) = &e.path {
                bytes = bytes.checked_add(p.capacity().checked_mul(size_of::<MapPoint>())?)?;
            }
        }
        Some(bytes)
    }
}
fn foreign_columns(snapshot: &SchemaMapSnapshot) -> BTreeSet<(SchemaMapIdentity, i16)> {
    snapshot
        .foreign_keys
        .iter()
        .flat_map(|fk| {
            fk.columns
                .iter()
                .flat_map(move |p| [(fk.source, p.source), (fk.target, p.target)])
        })
        .collect()
}
fn displayed(
    identity: SchemaMapIdentity,
    c: &SchemaMapColumn,
    prefs: MapPrefs,
    keys: &BTreeSet<(SchemaMapIdentity, i16)>,
) -> bool {
    match prefs.attributes {
        MapAttributes::All => true,
        MapAttributes::KeysOnly => c.primary_key || keys.contains(&(identity, c.attnum)),
        MapAttributes::None => false,
    }
}
fn write_title(w: &mut impl Write, t: &SchemaMapTable) -> fmt::Result {
    details::identifier(w, &t.schema)?;
    w.write_char('.')?;
    details::identifier(w, &t.name)
}
fn write_column(w: &mut impl Write, c: &SchemaMapColumn, p: MapPrefs) -> fmt::Result {
    write!(w, "{}{}", if c.primary_key { "PK " } else { "" }, c.name)?;
    if p.show_types {
        write!(w, "  {}", c.data_type)?;
    }
    if p.show_nulls {
        w.write_str(if c.nullable { "  NULL" } else { "  NOT NULL" })?;
    }
    if p.show_comments
        && let Some(comment) = &c.comment
    {
        write!(w, "  {comment}")?;
    }
    Ok(())
}
fn write_edge_label(
    w: &mut impl Write,
    s: &SchemaMapSnapshot,
    fk: &SchemaMapForeignKey,
) -> fmt::Result {
    let source = s
        .tables
        .iter()
        .find(|t| t.identity == fk.source)
        .ok_or(fmt::Error)?;
    let target = s
        .tables
        .iter()
        .find(|t| t.identity == fk.target)
        .ok_or(fmt::Error)?;
    for (i, p) in fk.columns.iter().enumerate() {
        if i > 0 {
            w.write_str(", ")?;
        }
        w.write_str(
            &source
                .columns
                .iter()
                .find(|c| c.attnum == p.source)
                .ok_or(fmt::Error)?
                .name,
        )?;
    }
    w.write_str(" → ")?;
    for (i, p) in fk.columns.iter().enumerate() {
        if i > 0 {
            w.write_str(", ")?;
        }
        w.write_str(
            &target
                .columns
                .iter()
                .find(|c| c.attnum == p.target)
                .ok_or(fmt::Error)?
                .name,
        )?;
    }
    Ok(())
}
struct Label {
    text: Option<String>,
    bytes: usize,
    chars: usize,
    advance: usize,
    truncated: bool,
}
impl Write for Label {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        for ch in value.chars() {
            if self.chars == LABEL_CHARS {
                self.truncated = true;
                break;
            }
            let ch = match ch {
                '\n' | '\r' => '↵',
                '\t' => '⇥',
                c if c.is_control() => '�',
                c => c,
            };
            self.bytes += ch.len_utf8();
            self.chars += 1;
            self.advance += label_advance(ch);
            if let Some(text) = &mut self.text {
                text.push(ch);
            }
        }
        Ok(())
    }
}
fn label(
    mut f: impl FnMut(&mut Label) -> fmt::Result,
    allocate: bool,
) -> Result<Label, &'static str> {
    let mut value = Label {
        text: allocate.then(String::new),
        bytes: 0,
        chars: 0,
        advance: 0,
        truncated: false,
    };
    f(&mut value).map_err(|_| "Map label metadata is inconsistent")?;
    if value.truncated {
        value.bytes += 3;
        value.advance += label_advance('…');
        if let Some(text) = &mut value.text {
            text.push('…');
        }
    }
    Ok(value)
}
fn measure_label(f: impl FnMut(&mut Label) -> fmt::Result) -> Result<usize, &'static str> {
    Ok(label(f, false)?.bytes)
}
fn make_label(f: impl FnMut(&mut Label) -> fmt::Result) -> Result<String, &'static str> {
    Ok(label(f, true)?.text.unwrap())
}
