use super::*;
use serde::Serialize;
use std::io::{self, Write};

pub(super) fn name(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_OVERVIEW_NAME_BYTES && !s.contains('\0')
}
pub(super) fn scope_valid(scope: &RelationStatsScope) -> bool {
    match scope {
        RelationStatsScope::Database => true,
        RelationStatsScope::Schema {
            name: value,
            expected_oid,
        } => name(value) && *expected_oid != Some(0),
        RelationStatsScope::Relation {
            schema,
            name: value,
            expected,
        } => {
            name(schema)
                && name(value)
                && expected.is_none_or(|id| id.database_oid != 0 && id.relation_oid != 0)
        }
    }
}
pub(super) fn cursor_valid(c: &RelationStatsCursor) -> bool {
    !c.document.is_empty()
        && c.document.len() <= 256
        && !c.document.contains('\0')
        && name(&c.database)
        && !c.connection.is_empty()
        && c.connection.len() <= 256
        && !c.connection.contains('\0')
        && c.database_oid != 0
        && c.oid != 0
        && name(&c.schema)
        && name(&c.name)
        && scope_valid(&c.scope)
        && c.schema_oid != Some(0)
        && c.relation_oid != Some(0)
}
fn metric(value: OverviewMetric) -> bool {
    !matches!(value, OverviewMetric::Value(n) if n < 0)
}
fn estimate(value: OverviewMetric, known: i64, unknown: i64) -> bool {
    known >= 0
        && unknown >= 0
        && value
            == if unknown == 0 {
                OverviewMetric::Value(known)
            } else {
                OverviewMetric::Unknown
            }
}
fn capture(c: &OverviewCapture) -> Option<usize> {
    if !name(&c.database)
        || c.database_oid == 0
        || c.reader_pid <= 0
        || [&c.collected_start, &c.collected_end]
            .iter()
            .any(|s| s.len() > 64 || chrono::DateTime::parse_from_rfc3339(s).is_err())
    {
        return None;
    }
    Some(c.database.capacity() + c.collected_start.capacity() + c.collected_end.capacity())
}
fn scope_heap(s: &RelationStatsScope) -> usize {
    match s {
        RelationStatsScope::Database => 0,
        RelationStatsScope::Schema { name, .. } => name.capacity(),
        RelationStatsScope::Relation { schema, name, .. } => schema.capacity() + name.capacity(),
    }
}
fn cursor_heap(c: &RelationStatsCursor) -> usize {
    c.connection.capacity()
        + c.document.capacity()
        + c.database.capacity()
        + c.schema.capacity()
        + c.name.capacity()
        + scope_heap(&c.scope)
}
struct Counter(usize);
impl Write for Counter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(buf.len())
            .filter(|n| *n <= MAX_OVERVIEW_BYTES)
            .ok_or_else(|| io::Error::other("overview bound"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn encoded(value: &impl Serialize) -> Option<usize> {
    let mut count = Counter(0);
    serde_json::to_writer(&mut count, value).ok()?;
    Some(count.0)
}
impl DatabaseOverviewSnapshot {
    pub fn encoded_bytes(&self) -> Option<usize> {
        encoded(self)
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let heap = size_of::<Self>().checked_add(capture(&self.capture)?)?;
        if heap > MAX_OVERVIEW_BYTES
            || self.table_count < 0
            || self.schema_count < 0
            || self.index_count < 0
            || self.unknown_estimate_relations > self.table_count
            || !estimate(
                self.row_count_estimate,
                self.known_row_count_estimate,
                self.unknown_estimate_relations,
            )
            || [
                self.database_size_bytes,
                self.table_size_bytes,
                self.index_size_bytes,
                self.connection_count,
            ]
            .iter()
            .any(|v| !metric(*v) || *v == OverviewMetric::NotApplicable)
        {
            return None;
        }
        self.encoded_bytes()?;
        Some(heap)
    }
}
impl RelationStatsSnapshot {
    pub fn encoded_bytes(&self) -> Option<usize> {
        encoded(self)
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let t = &self.totals;
        if !scope_valid(&self.scope)
            || self.rows.len() > MAX_RELATION_STATS_ROWS
            || self.schema_oid == Some(0)
            || self.relation_oid == Some(0)
            || t.relation_count < self.rows.len() as i64
            || t.table_count < 0
            || t.view_count < 0
            || t.materialized_view_count < 0
            || t.table_count
                .checked_add(t.view_count)?
                .checked_add(t.materialized_view_count)?
                != t.relation_count
            || t.unknown_estimate_relations
                > t.table_count.checked_add(t.materialized_view_count)?
            || !estimate(
                t.row_count_estimate,
                t.known_row_count_estimate,
                t.unknown_estimate_relations,
            )
            || !metric(t.total_size_bytes)
            || t.total_size_bytes == OverviewMetric::NotApplicable
        {
            return None;
        }
        let identity_valid = match &self.scope {
            RelationStatsScope::Database => {
                self.schema_oid.is_none() && self.relation_oid.is_none()
            }
            RelationStatsScope::Schema { expected_oid, .. } => {
                self.schema_oid.is_some()
                    && self.relation_oid.is_none()
                    && expected_oid.is_none_or(|oid| Some(oid) == self.schema_oid)
            }
            RelationStatsScope::Relation { expected, .. } => {
                self.schema_oid.is_some()
                    && self.relation_oid.is_some()
                    && t.relation_count == 1
                    && expected.is_none_or(|id| {
                        id.database_oid == self.capture.database_oid
                            && Some(id.relation_oid) == self.relation_oid
                    })
            }
        };
        if !identity_valid
            || self
                .rows
                .iter()
                .filter(|r| {
                    matches!(
                        r.kind,
                        OverviewRelationKind::Table | OverviewRelationKind::PartitionedTable
                    )
                })
                .count() as i64
                > t.table_count
            || self
                .rows
                .iter()
                .filter(|r| r.kind == OverviewRelationKind::View)
                .count() as i64
                > t.view_count
            || self
                .rows
                .iter()
                .filter(|r| r.kind == OverviewRelationKind::MaterializedView)
                .count() as i64
                > t.materialized_view_count
        {
            return None;
        }
        let mut heap = size_of::<Self>()
            .checked_add(capture(&self.capture)?)?
            .checked_add(scope_heap(&self.scope))?
            .checked_add(
                self.rows
                    .capacity()
                    .checked_mul(size_of::<RelationStats>())?,
            )?;
        for (index, r) in self.rows.iter().enumerate() {
            if r.identity.database_oid != self.capture.database_oid
                || r.identity.relation_oid == 0
                || r.schema_oid == 0
                || !name(&r.schema)
                || !name(&r.name)
                || !metric(r.row_count_estimate)
                || !metric(r.total_size_bytes)
                || self.rows[..index]
                    .iter()
                    .any(|p| p.identity.relation_oid == r.identity.relation_oid)
                || match &self.scope {
                    RelationStatsScope::Database => {
                        r.is_partition
                            || r.schema == "pg_catalog"
                            || r.schema == "information_schema"
                            || r.schema.starts_with("pg_toast")
                    }
                    RelationStatsScope::Schema { name, .. } => r.schema != *name || r.is_partition,
                    RelationStatsScope::Relation { schema, name, .. } => {
                        r.schema != *schema || r.name != *name
                    }
                }
                || self.schema_oid.is_some_and(|oid| oid != r.schema_oid)
                || self
                    .relation_oid
                    .is_some_and(|oid| oid != r.identity.relation_oid)
                || (r.kind == OverviewRelationKind::View)
                    != (r.row_count_estimate == OverviewMetric::NotApplicable)
                || (r.kind == OverviewRelationKind::View)
                    != (r.total_size_bytes == OverviewMetric::NotApplicable)
                || r.row_count_estimate == OverviewMetric::Restricted
            {
                return None;
            }
            if index > 0 {
                let p = &self.rows[index - 1];
                if (&p.schema, &p.name, p.identity.relation_oid)
                    >= (&r.schema, &r.name, r.identity.relation_oid)
                {
                    return None;
                }
            }
            heap = heap
                .checked_add(r.schema.capacity())?
                .checked_add(r.name.capacity())?;
        }
        if let Some(c) = &self.next_cursor {
            let last = self.rows.last()?;
            if c.checked_heap_bytes().is_none()
                || c.scope != self.scope
                || c.database_oid != self.capture.database_oid
                || c.database != self.capture.database
                || c.schema_oid != self.schema_oid
                || c.relation_oid != self.relation_oid
                || (&c.schema, &c.name, c.oid)
                    != (&last.schema, &last.name, last.identity.relation_oid)
            {
                return None;
            }
            heap = heap.checked_add(cursor_heap(c))?;
        }
        if heap > MAX_OVERVIEW_BYTES {
            return None;
        }
        self.encoded_bytes()?;
        Some(heap)
    }
}
impl OverviewSnapshot {
    pub fn encoded_bytes(&self) -> Option<usize> {
        encoded(self)
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let mut heap = size_of::<Self>().checked_add(
            self.relations
                .checked_heap_bytes()?
                .checked_sub(size_of::<RelationStatsSnapshot>())?,
        )?;
        if let Some(database) = &self.database {
            if self.relations.scope != RelationStatsScope::Database
                || database.capture.database_oid != self.relations.capture.database_oid
                || database.capture.database != self.relations.capture.database
                || database.capture.reader_pid != self.relations.capture.reader_pid
                || database.capture.collected_start != self.relations.capture.collected_start
            {
                return None;
            }
            heap = heap.checked_add(
                database
                    .checked_heap_bytes()?
                    .checked_sub(size_of::<DatabaseOverviewSnapshot>())?,
            )?;
        }
        if heap > MAX_OVERVIEW_BYTES {
            return None;
        }
        self.encoded_bytes()?;
        Some(heap)
    }
}

impl RelationStatsCursor {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if !cursor_valid(self) {
            return None;
        }
        let heap = size_of::<Self>().checked_add(cursor_heap(self))?;
        (heap <= 4096).then_some(heap)
    }
}
impl RelationStatsRequest {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if !scope_valid(&self.scope) || self.expected_database_oid == Some(0) {
            return None;
        }
        let mut heap = size_of::<Self>().checked_add(scope_heap(&self.scope))?;
        if let Some(c) = &self.cursor {
            if c.scope != self.scope
                || self
                    .expected_database_oid
                    .is_some_and(|oid| oid != c.database_oid)
            {
                return None;
            }
            heap = heap.checked_add(
                c.checked_heap_bytes()?
                    .checked_sub(size_of::<RelationStatsCursor>())?,
            )?;
        }
        (heap <= 8192).then_some(heap)
    }
}
