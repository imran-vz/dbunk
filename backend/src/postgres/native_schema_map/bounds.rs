use super::*;
use std::{collections::BTreeSet, io, mem::size_of};
fn name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 63 && !s.contains('\0')
}
fn id(id: SchemaMapIdentity) -> bool {
    id.database_oid != 0 && id.relation_oid != 0
}
pub(super) fn scope_valid(scope: &SchemaMapScope) -> bool {
    match scope {
        SchemaMapScope::Database => true,
        SchemaMapScope::Schema {
            name: s,
            expected_oid,
        } => name(s) && *expected_oid != Some(0),
        SchemaMapScope::Relation {
            schema,
            table,
            expected,
        } => name(schema) && name(table) && expected.is_none_or(id),
    }
}
fn scope_heap(scope: &SchemaMapScope) -> usize {
    match scope {
        SchemaMapScope::Database => 0,
        SchemaMapScope::Schema { name, .. } => name.capacity(),
        SchemaMapScope::Relation { schema, table, .. } => {
            schema.capacity().saturating_add(table.capacity())
        }
    }
}
#[derive(Default)]
struct Heap(usize);
impl Heap {
    fn add(&mut self, bytes: usize) -> Option<()> {
        self.0 = self
            .0
            .checked_add(bytes)
            .filter(|n| *n <= MAX_SCHEMA_MAP_BYTES)?;
        Some(())
    }
    fn vec<T>(&mut self, values: &Vec<T>) -> Option<()> {
        self.add(values.capacity().checked_mul(size_of::<T>())?)
    }
    fn text(&mut self, s: &String, max: usize, empty: bool) -> Option<()> {
        if s.len() > max || s.contains('\0') || (!empty && s.is_empty()) {
            return None;
        }
        self.add(s.capacity())
    }
}
struct Encoded(usize);
impl io::Write for Encoded {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(b.len())
            .filter(|n| *n <= MAX_SCHEMA_MAP_BYTES)
            .ok_or_else(|| io::Error::other("Schema map exceeds encoding allowance"))?;
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl SchemaMapRequest {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if !scope_valid(&self.scope) || self.expected_database_oid == Some(0) {
            return None;
        }
        size_of::<Self>()
            .checked_add(scope_heap(&self.scope))
            .filter(|n| *n <= 8192)
    }
}
impl SchemaMapSnapshot {
    /// Actual reachable Vec/String capacities plus inline structures. No text
    /// serialization allocation is needed to check the separate encoded bound.
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.tables.len() > MAX_SCHEMA_MAP_TABLES
            || self.foreign_keys.len() > MAX_SCHEMA_MAP_FOREIGN_KEYS
            || self.database_oid == 0
            || self.server_version < MIN_SCHEMA_MAP_SERVER_VERSION
            || !scope_valid(&self.scope)
            || self.schema_oid == Some(0)
        {
            return None;
        }
        chrono::DateTime::parse_from_rfc3339(&self.captured_at).ok()?;
        let mut heap = Heap(size_of::<Self>());
        heap.text(&self.database, 63, false)?;
        heap.text(&self.captured_at, 64, false)?;
        heap.add(scope_heap(&self.scope))?;
        heap.vec(&self.tables)?;
        heap.vec(&self.foreign_keys)?;
        let mut ids = BTreeSet::new();
        let mut trigger_ids = BTreeSet::new();
        let mut columns = 0usize;
        let mut triggers = 0usize;
        let mut trigger_columns = 0usize;
        for table in &self.tables {
            if !id(table.identity)
                || table.identity.database_oid != self.database_oid
                || !ids.insert(table.identity)
                || table.schema_oid == 0
                || table.columns.len() > 1600
            {
                return None;
            }
            heap.text(&table.schema, 63, false)?;
            heap.text(&table.name, 63, false)?;
            heap.vec(&table.columns)?;
            heap.vec(&table.triggers)?;
            columns = columns.checked_add(table.columns.len())?;
            triggers = triggers.checked_add(table.triggers.len())?;
            let mut last = 0;
            let mut names = BTreeSet::new();
            for column in &table.columns {
                if column.attnum <= last
                    || column.attnum > 1600
                    || !names.insert(column.name.as_str())
                {
                    return None;
                }
                last = column.attnum;
                heap.text(&column.name, 63, false)?;
                heap.text(&column.data_type, MAX_SCHEMA_MAP_TYPE_BYTES, false)?;
                if let Some(comment) = &column.comment {
                    heap.text(comment, MAX_SCHEMA_MAP_COMMENT_BYTES, true)?;
                }
            }
            for trigger in &table.triggers {
                if trigger.oid == 0
                    || trigger.function_oid == 0
                    || !trigger_ids.insert(trigger.oid)
                    || trigger.columns.len() > 1600
                    || trigger.events.is_empty()
                    || trigger.events.len() > 4
                    || !matches!(trigger.timing.as_str(), "BEFORE" | "AFTER" | "INSTEAD OF")
                    || !matches!(trigger.orientation.as_str(), "ROW" | "STATEMENT")
                {
                    return None;
                }
                heap.text(&trigger.name, 63, false)?;
                heap.text(&trigger.function_schema, 63, false)?;
                heap.text(&trigger.function_name, 63, false)?;
                heap.text(&trigger.timing, 16, false)?;
                heap.text(&trigger.orientation, 16, false)?;
                heap.vec(&trigger.columns)?;
                heap.vec(&trigger.events)?;
                let mut attrs = BTreeSet::new();
                for attr in &trigger.columns {
                    if !attrs.insert(attr) || !table.columns.iter().any(|c| c.attnum == *attr) {
                        return None;
                    }
                }
                trigger_columns = trigger_columns.checked_add(trigger.columns.len())?;
                let mut events = BTreeSet::new();
                for event in &trigger.events {
                    if !events.insert(event)
                        || !matches!(event.as_str(), "INSERT" | "UPDATE" | "DELETE" | "TRUNCATE")
                    {
                        return None;
                    }
                    heap.text(event, 16, false)?;
                }
            }
        }
        if columns > MAX_SCHEMA_MAP_COLUMNS
            || triggers > MAX_SCHEMA_MAP_TRIGGERS
            || trigger_columns > MAX_SCHEMA_MAP_COLUMNS
        {
            return None;
        }
        let mut constraints = BTreeSet::new();
        let mut pairs = 0usize;
        for fk in &self.foreign_keys {
            if fk.database_oid != self.database_oid
                || fk.constraint_oid == 0
                || !constraints.insert(fk.constraint_oid)
                || fk.columns.is_empty()
                || fk.columns.len() > 64
                || !matches!(fk.match_type.as_str(), "SIMPLE" | "FULL" | "PARTIAL")
            {
                return None;
            }
            let source = self.tables.iter().find(|t| t.identity == fk.source)?;
            let target = self.tables.iter().find(|t| t.identity == fk.target)?;
            if fk.columns_unique != (fk.cardinality == SchemaMapCardinality::OneToOne)
                || fk.junction_participant && !source.junction
            {
                return None;
            }
            let mut sources = BTreeSet::new();
            let mut targets = BTreeSet::new();
            let mut nullable = false;
            for pair in &fk.columns {
                if !sources.insert(pair.source) || !targets.insert(pair.target) {
                    return None;
                }
                let column = source.columns.iter().find(|c| c.attnum == pair.source)?;
                target.columns.iter().find(|c| c.attnum == pair.target)?;
                nullable |= column.nullable;
            }
            if nullable != fk.columns_nullable {
                return None;
            }
            pairs = pairs.checked_add(fk.columns.len())?;
            heap.text(&fk.name, 63, false)?;
            heap.text(&fk.match_type, 16, false)?;
            heap.text(&fk.cardinality_reason, 512, false)?;
            heap.vec(&fk.columns)?;
        }
        if pairs > MAX_SCHEMA_MAP_PAIRS {
            return None;
        }
        match &self.scope {
            SchemaMapScope::Database => {
                if self.schema_oid.is_some()
                    || self.focus.is_some()
                    || self.tables.iter().any(|t| t.external)
                {
                    return None;
                }
            }
            SchemaMapScope::Schema { name, expected_oid } => {
                let schema_oid = self.schema_oid?;
                if self.focus.is_some() || expected_oid.is_some_and(|e| e != schema_oid) {
                    return None;
                }
                for t in &self.tables {
                    if t.external != (t.schema != *name)
                        || (!t.external && t.schema_oid != schema_oid)
                        || t.external && !self.foreign_keys.iter().any(|fk| fk.target == t.identity)
                    {
                        return None;
                    }
                }
                if self.foreign_keys.iter().any(|fk| {
                    self.tables
                        .iter()
                        .find(|t| t.identity == fk.source)
                        .is_none_or(|t| t.schema != *name)
                }) {
                    return None;
                }
            }
            SchemaMapScope::Relation {
                schema,
                table,
                expected,
            } => {
                let focus = self.focus?;
                let central = self.tables.iter().find(|t| t.identity == focus)?;
                if expected.is_some_and(|e| e != focus)
                    || central.schema != *schema
                    || central.name != *table
                    || Some(central.schema_oid) != self.schema_oid
                    || self.tables.iter().any(|t| t.external)
                {
                    return None;
                }
                if self
                    .foreign_keys
                    .iter()
                    .any(|fk| fk.source != focus && fk.target != focus)
                {
                    return None;
                }
                if self.tables.iter().any(|t| {
                    t.identity != focus
                        && !self
                            .foreign_keys
                            .iter()
                            .any(|fk| fk.source == t.identity || fk.target == t.identity)
                }) {
                    return None;
                }
            }
        }
        serde_json::to_writer(Encoded(0), self).ok()?;
        Some(heap.0)
    }
}
