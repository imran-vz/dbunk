//! Immutable overview pages with exact metrics and explicit estimate semantics.
//! Admission covers old/new captures and one selected read-only detail editor.
use dbunk_lib::backend::overview::*;
use std::{
    cell::Cell,
    fmt::{self, Write},
    mem::size_of,
    rc::Rc,
};
const SHARED_BYTES: usize = 128 * 1024 * 1024;
const PRESENTATION_BYTES: usize = 64 * 1024;
const MAX_DETAIL_BYTES: usize = 64 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Database,
    Totals,
    Relations,
}
impl Section {
    pub const ALL: [Self; 3] = [Self::Database, Self::Totals, Self::Relations];
    pub fn label(self) -> &'static str {
        match self {
            Self::Database => "Database metrics",
            Self::Totals => "Scope totals",
            Self::Relations => "Relations",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Database,
    Schema,
    Relation,
}
impl Scope {
    pub const ALL: [Self; 3] = [Self::Database, Self::Schema, Self::Relation];
    pub fn label(self) -> &'static str {
        match self {
            Self::Database => "Database",
            Self::Schema => "Schema",
            Self::Relation => "Relation",
        }
    }
    pub fn request(self, schema: &str, table: &str) -> Result<RelationStatsRequest, &'static str> {
        let name = |value: &str| !value.is_empty() && value.len() <= 63 && !value.contains('\0');
        let scope = match self {
            Self::Database => RelationStatsScope::Database,
            Self::Schema if name(schema) => RelationStatsScope::Schema {
                name: schema.to_owned(),
                expected_oid: None,
            },
            Self::Relation if name(schema) && name(table) => RelationStatsScope::Relation {
                schema: schema.to_owned(),
                name: table.to_owned(),
                expected: None,
            },
            _ => return Err("Scope names must each contain 1–63 UTF-8 bytes without NUL"),
        };
        Ok(RelationStatsRequest {
            scope,
            expected_database_oid: None,
            cursor: None,
        })
    }
}
pub fn same_scope(a: &RelationStatsScope, b: &RelationStatsScope) -> bool {
    match (a, b) {
        (RelationStatsScope::Database, RelationStatsScope::Database) => true,
        (
            RelationStatsScope::Schema { name: a, .. },
            RelationStatsScope::Schema { name: b, .. },
        ) => a == b,
        (
            RelationStatsScope::Relation {
                schema: a,
                name: an,
                ..
            },
            RelationStatsScope::Relation {
                schema: b,
                name: bn,
                ..
            },
        ) => a == b && an == bn,
        _ => false,
    }
}
pub struct Capture {
    database: Option<DatabaseOverviewSnapshot>,
    relations: RelationStatsSnapshot,
    budget: Rc<Cell<usize>>,
    retained: usize,
    largest: Counter,
    metrics_retained: bool,
}
impl Capture {
    /// Retains an older database metric capture only for database-wide paging
    /// on the same observed database. Its own interval is always displayed.
    /// The caller replaces its old Capture only after success. Reservation
    /// includes typed capacities plus 64×largest detail bytes and 4096 bytes per
    /// logical CR/LF boundary for current/replacement editor and accessibility
    /// overlap. Soft wrap must remain disabled. This is not a peak-RSS claim.
    pub fn new(
        data: OverviewSnapshot,
        budget: Rc<Cell<usize>>,
        previous: Option<&Self>,
    ) -> Result<Self, &'static str> {
        let incoming = data.checked_heap_bytes().ok_or(
            "Overview response is invalid or exceeds its bounds; previous capture retained",
        )?;
        let prior = previous
            .filter(|old| {
                matches!(data.relations.scope, RelationStatsScope::Database)
                    && matches!(old.relations.scope, RelationStatsScope::Database)
                    && old.relations.capture.database_oid == data.relations.capture.database_oid
                    && old.relations.capture.database == data.relations.capture.database
            })
            .and_then(|old| old.database.as_ref())
            .filter(|_| data.database.is_none());
        let database = data.database.as_ref().or(prior);
        let mut largest = Counter::default();
        for section in Section::ALL {
            for index in 0..count(database, &data.relations, section) {
                let mut measured = Counter::default();
                render(database, &data.relations, section, index, &mut measured)
                    .map_err(|_| "Overview detail exceeds its formatting bound")?;
                largest.bytes = largest.bytes.max(measured.bytes);
                largest.lines = largest.lines.max(measured.lines);
            }
        }
        let retained = incoming
            .checked_add(
                prior
                    .map_or(Some(0), DatabaseOverviewSnapshot::checked_heap_bytes)
                    .ok_or("Retained database metrics are invalid")?,
            )
            .and_then(|n| n.checked_add(size_of::<Self>()))
            .and_then(|n| n.checked_add(largest.allowance()?))
            .and_then(|n| n.checked_add(PRESENTATION_BYTES))
            .ok_or("Overview retention size overflow")?;
        if retained > SHARED_BYTES.saturating_sub(budget.get()) {
            return Err("Overview exceeds available shared allowance; previous capture retained");
        }
        let metrics_retained = prior.is_some();
        budget.set(budget.get() + retained);
        // This clone is bounded and admitted before copying any retained strings.
        let database = data.database.or_else(|| prior.cloned());
        Ok(Self {
            database,
            relations: data.relations,
            budget,
            retained,
            largest,
            metrics_retained,
        })
    }
    pub fn scope(&self) -> &RelationStatsScope {
        &self.relations.scope
    }
    pub fn count(&self, section: Section) -> usize {
        count(self.database.as_ref(), &self.relations, section)
    }
    pub fn matches(&self, request: &RelationStatsRequest) -> bool {
        same_scope(self.scope(), &request.scope)
    }
    pub fn next_request(&self) -> Option<RelationStatsRequest> {
        Some(RelationStatsRequest {
            scope: self.scope().clone(),
            expected_database_oid: Some(self.relations.capture.database_oid),
            cursor: Some(self.relations.next_cursor.as_ref()?.clone()),
        })
    }
    pub fn has_next(&self) -> bool {
        self.relations.next_cursor.is_some()
    }
    /// Retry matching names against the same observed identity, even when the
    /// page is stale. Only changing scope or dropping this capture can rebind.
    pub fn refresh_for(&self, request: RelationStatsRequest) -> RelationStatsRequest {
        if self.matches(&request) {
            self.refresh_request()
        } else {
            request
        }
    }
    pub fn refresh_request(&self) -> RelationStatsRequest {
        let data = &self.relations;
        let scope = match &data.scope {
            RelationStatsScope::Database => RelationStatsScope::Database,
            RelationStatsScope::Schema { name, .. } => RelationStatsScope::Schema {
                name: name.clone(),
                expected_oid: data.schema_oid,
            },
            RelationStatsScope::Relation { schema, name, .. } => RelationStatsScope::Relation {
                schema: schema.clone(),
                name: name.clone(),
                expected: data
                    .relation_oid
                    .map(|relation_oid| OverviewRelationIdentity {
                        database_oid: data.capture.database_oid,
                        relation_oid,
                    }),
            },
        };
        RelationStatsRequest {
            scope,
            expected_database_oid: Some(data.capture.database_oid),
            cursor: None,
        }
    }
    pub fn status(&self) -> String {
        let r = &self.relations;
        format!(
            "{} | database {} (OID {}) | {} rows on this page of {} captured relations | {} to {}{}",
            ScopeLabel(&r.scope),
            Identifier(&r.capture.database),
            r.capture.database_oid,
            r.rows.len(),
            r.totals.relation_count,
            r.capture.collected_start,
            r.capture.collected_end,
            if self.metrics_retained {
                " | Database metrics retained from their earlier, separately shown interval"
            } else {
                ""
            }
        )
    }
    pub fn empty_label(&self, section: Section) -> &'static str {
        match section {
            Section::Database => "No database metric capture for this scope",
            Section::Relations => "No relations on this captured page",
            Section::Totals => "No scope totals captured",
        }
    }
    pub fn row_label(&self, section: Section, index: usize) -> Option<String> {
        if index >= self.count(section) {
            return None;
        }
        Some(match section {
            Section::Database => {
                let d = self.database.as_ref()?;
                format!(
                    "{}: {}",
                    DATABASE_LABELS[index],
                    metric(database_metric(d, index))
                )
            }
            Section::Totals => format!(
                "{}: {}",
                TOTAL_LABELS[index],
                metric(total_metric(&self.relations.totals, index))
            ),
            Section::Relations => {
                let row = &self.relations.rows[index];
                format!(
                    "{}.{} · {} · rows {} · size {}",
                    Identifier(&row.schema),
                    Identifier(&row.name),
                    kind(row.kind),
                    metric(row.row_count_estimate),
                    metric(row.total_size_bytes)
                )
            }
        })
    }
    pub fn details(&self, section: Section, index: usize) -> Result<Option<String>, &'static str> {
        if index >= self.count(section) {
            return Ok(None);
        }
        let mut measured = Counter::default();
        render(
            self.database.as_ref(),
            &self.relations,
            section,
            index,
            &mut measured,
        )
        .map_err(|_| "Overview detail exceeds formatting bounds")?;
        if measured.bytes > self.largest.bytes || measured.lines > self.largest.lines {
            return Err("Overview detail exceeds admitted editor allowance");
        }
        let mut text = String::with_capacity(measured.bytes);
        render(
            self.database.as_ref(),
            &self.relations,
            section,
            index,
            &mut text,
        )
        .map_err(|_| "Overview detail formatting failed")?;
        Ok(Some(text))
    }
    #[cfg(test)]
    fn retained_bytes(&self) -> usize {
        self.retained
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.retained));
    }
}
const DATABASE_LABELS: [&str; 8] = [
    "Database size (bytes)",
    "Table size (bytes)",
    "Index size (bytes)",
    "Tables",
    "Schemas containing relations",
    "Estimated rows",
    "Indexes",
    "Current-database connections",
];
const TOTAL_LABELS: [&str; 8] = [
    "Relations",
    "Tables",
    "Views",
    "Materialized views",
    "Estimated rows",
    "Known row estimate subtotal",
    "Relations with unknown estimates",
    "Physical size (bytes)",
];
fn count(
    database: Option<&DatabaseOverviewSnapshot>,
    relations: &RelationStatsSnapshot,
    section: Section,
) -> usize {
    match section {
        Section::Database => {
            if database.is_some() {
                8
            } else {
                0
            }
        }
        Section::Totals => 8,
        Section::Relations => relations.rows.len(),
    }
}
fn database_metric(d: &DatabaseOverviewSnapshot, index: usize) -> OverviewMetric {
    match index {
        0 => d.database_size_bytes,
        1 => d.table_size_bytes,
        2 => d.index_size_bytes,
        3 => OverviewMetric::Value(d.table_count),
        4 => OverviewMetric::Value(d.schema_count),
        5 => d.row_count_estimate,
        6 => OverviewMetric::Value(d.index_count),
        _ => d.connection_count,
    }
}
fn total_metric(d: &RelationStatsTotals, index: usize) -> OverviewMetric {
    match index {
        0 => OverviewMetric::Value(d.relation_count),
        1 => OverviewMetric::Value(d.table_count),
        2 => OverviewMetric::Value(d.view_count),
        3 => OverviewMetric::Value(d.materialized_view_count),
        4 => d.row_count_estimate,
        5 => OverviewMetric::Value(d.known_row_count_estimate),
        6 => OverviewMetric::Value(d.unknown_estimate_relations),
        _ => d.total_size_bytes,
    }
}
fn metric(value: OverviewMetric) -> Metric {
    Metric(value)
}
struct Metric(OverviewMetric);
impl fmt::Display for Metric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            OverviewMetric::Value(n) => write!(f, "{n}"),
            OverviewMetric::Unknown => f.write_str("Unknown"),
            OverviewMetric::NotApplicable => f.write_str("Not applicable"),
            OverviewMetric::Restricted => f.write_str("Restricted (permission denied)"),
        }
    }
}
fn kind(value: OverviewRelationKind) -> &'static str {
    match value {
        OverviewRelationKind::Table => "Table",
        OverviewRelationKind::PartitionedTable => "Partitioned table",
        OverviewRelationKind::View => "View",
        OverviewRelationKind::MaterializedView => "Materialized view",
    }
}
struct Identifier<'a>(&'a str);
impl fmt::Display for Identifier<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_char('"')?;
        for c in self.0.chars() {
            if c == '"' {
                f.write_char('"')?;
            }
            f.write_char(c)?;
        }
        f.write_char('"')
    }
}
struct ScopeLabel<'a>(&'a RelationStatsScope);
impl fmt::Display for ScopeLabel<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            RelationStatsScope::Database => f.write_str("Database scope"),
            RelationStatsScope::Schema { name, .. } => write!(f, "Schema {}", Identifier(name)),
            RelationStatsScope::Relation { schema, name, .. } => {
                write!(f, "Relation {}.{}", Identifier(schema), Identifier(name))
            }
        }
    }
}
#[derive(Default)]
struct Counter {
    bytes: usize,
    lines: usize,
}
impl Counter {
    fn allowance(&self) -> Option<usize> {
        self.bytes
            .checked_mul(64)?
            .checked_add(self.lines.checked_add(1)?.checked_mul(4096)?)
    }
}
impl Write for Counter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let bytes = self
            .bytes
            .checked_add(s.len())
            .filter(|n| *n <= MAX_DETAIL_BYTES)
            .ok_or(fmt::Error)?;
        let lines = self
            .lines
            .checked_add(s.bytes().filter(|b| matches!(*b, b'\n' | b'\r')).count())
            .ok_or(fmt::Error)?;
        self.bytes = bytes;
        self.lines = lines;
        Ok(())
    }
}
fn context(w: &mut impl Write, c: &OverviewCapture) -> fmt::Result {
    writeln!(
        w,
        "Database: {}\nDatabase OID: {}\nReader PID: {}\nCollection started: {}\nCollection ended: {}\nStatistics and physical sizes can change during collection; this is not an atomic server-wide measurement.\n",
        Identifier(&c.database),
        c.database_oid,
        c.reader_pid,
        c.collected_start,
        c.collected_end
    )
}
fn render(
    database: Option<&DatabaseOverviewSnapshot>,
    page: &RelationStatsSnapshot,
    section: Section,
    index: usize,
    w: &mut impl Write,
) -> fmt::Result {
    if section == Section::Database {
        let d = database.ok_or(fmt::Error)?;
        context(w, &d.capture)?;
        writeln!(
            w,
            "{}: {}",
            DATABASE_LABELS[index],
            metric(database_metric(d, index))
        )?;
        match index {
            3 => writeln!(
                w,
                "Ordinary and partitioned user tables, including partition children, in the baseline database scope."
            ),
            4 => writeln!(
                w,
                "Counts namespaces containing any pg_class entry, not empty schemas."
            ),
            5 => writeln!(
                w,
                "Planner estimate, not COUNT(*). Known estimate subtotal: {}\nRelations with unknown estimates: {}\nAny unknown estimate makes the total Unknown; zero remains a known zero.",
                d.known_row_count_estimate, d.unknown_estimate_relations
            ),
            6 => writeln!(
                w,
                "Counts physical indexes (relkind i), excluding partitioned index parents (I)."
            ),
            7 => writeln!(
                w,
                "Includes this inspection connection and other current-database sessions."
            ),
            _ => writeln!(
                w,
                "Exact returned byte count. Restricted is distinct from zero or Unknown."
            ),
        }?;
        writeln!(
            w,
            "\nMetric capture interval is shown above. Relation page interval (values may have been collected at different times):\n{} to {}",
            page.capture.collected_start, page.capture.collected_end
        )
    } else {
        context(w, &page.capture)?;
        writeln!(
            w,
            "Requested scope: {}\nSchema OID: {}\nRelation OID: {}\n",
            ScopeLabel(&page.scope),
            OptionalOid(page.schema_oid),
            OptionalOid(page.relation_oid)
        )?;
        if section == Section::Totals {
            writeln!(
                w,
                "{}: {}\nTotals cover the full requested scope, not only the {} rows on this page.\nKnown estimate subtotal: {}\nRelations with unknown estimates: {}",
                TOTAL_LABELS[index],
                metric(total_metric(&page.totals, index)),
                page.rows.len(),
                page.totals.known_row_count_estimate,
                page.totals.unknown_estimate_relations
            )?;
        } else {
            let r = &page.rows[index];
            writeln!(
                w,
                "Relation: {}.{}\nDatabase OID: {}\nSchema OID: {}\nRelation OID: {}\nKind: {}\nIs a partition: {}\nEstimated rows: {}\nPhysical total size (bytes): {}",
                Identifier(&r.schema),
                Identifier(&r.name),
                r.identity.database_oid,
                r.schema_oid,
                r.identity.relation_oid,
                kind(r.kind),
                r.is_partition,
                metric(r.row_count_estimate),
                metric(r.total_size_bytes)
            )?;
        }
        writeln!(
            w,
            "\nDatabase-wide scope excludes system namespaces and partition children. Explicit schema scope includes that namespace but excludes partition children. Explicit relation scope may inspect a child.\nRows are planner estimates, not exact counts. Views are Not applicable. Unknown and Restricted never imply zero. Sizes are physical per relation, not recursive partition totals.\nPages use a fresh read-only capture and stable keyset order; concurrent changes may affect later pages."
        )
    }
}
struct OptionalOid(Option<u32>);
impl fmt::Display for OptionalOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(oid) => write!(f, "{oid}"),
            None => f.write_str("Not scoped to one object"),
        }
    }
}
#[cfg(test)]
mod tests;
