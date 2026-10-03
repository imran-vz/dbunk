//! Exact SQL pages and retained artifact admission. The working allowance covers
//! bounded editor/AX representations and old/new page overlap, not process RSS.
use dbunk_lib::backend::ddl_export::{
    DdlExportArtifact, DdlExportIdentity, DdlExportRequest, DdlExportScope,
};
use std::{
    cell::Cell,
    fmt::{self, Write},
    ops::Range,
    rc::Rc,
    sync::Arc,
};

pub const PAGE_BYTES: usize = 32 * 1024;
pub const PAGE_BREAKS: usize = 128;
const METADATA_BYTES: usize = 512 * 1024;
const SHARED_BYTES: usize = 128 * 1024 * 1024;
const EDITOR_BYTES: usize = 64 * PAGE_BYTES + 4096 * (PAGE_BREAKS + 1) + 64 * 1024;

pub(crate) struct Lease {
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Lease {
    pub(crate) fn new(budget: Rc<Cell<usize>>, bytes: usize) -> Result<Rc<Self>, &'static str> {
        if bytes > SHARED_BYTES.saturating_sub(budget.get()) {
            return Err("DDL export exceeds the available shared retention allowance");
        }
        budget.set(budget.get() + bytes);
        Ok(Rc::new(Self { budget, bytes }))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Sql,
    Metadata,
}
impl Section {
    pub const ALL: [Self; 2] = [Self::Sql, Self::Metadata];
    pub fn label(self) -> &'static str {
        match self {
            Self::Sql => "SQL preview",
            Self::Metadata => "Capture and omissions",
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
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
    pub fn request(self, schema: &str, relation: &str) -> Result<DdlExportRequest, &'static str> {
        let valid = |name: &str| !name.is_empty() && name.len() <= 63 && !name.contains('\0');
        let scope = match self {
            Self::Database => DdlExportScope::Database,
            Self::Schema if valid(schema) => DdlExportScope::Schema {
                name: schema.into(),
                expected_oid: None,
            },
            Self::Relation if valid(schema) && valid(relation) => DdlExportScope::Relation {
                schema: schema.into(),
                name: relation.into(),
                expected: None,
            },
            _ => return Err("Use exact nonempty names of at most 63 UTF-8 bytes, without NUL"),
        };
        Ok(DdlExportRequest {
            scope,
            expected_database_oid: None,
        })
    }
}
pub struct Capture {
    pub(crate) artifact: Arc<DdlExportArtifact>,
    pub(crate) lease: Rc<Lease>,
    metadata: String,
    sql_pages: Vec<Range<usize>>,
    metadata_pages: Vec<Range<usize>>,
}
impl Capture {
    pub fn new(
        artifact: DdlExportArtifact,
        connection: &str,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        let heap = artifact
            .checked_heap_bytes()
            .ok_or("Invalid or oversized DDL artifact")?;
        if artifact.connection_id != connection {
            return Err("DDL artifact belongs to another connection");
        }
        let mut counter = Counter(0);
        metadata(&artifact, &mut counter).map_err(|_| "DDL metadata exceeds 512 KiB")?;
        let sql_count = Ranges::new(&artifact.sql).count();
        // Metadata page descriptors reserve their worst case before metadata is
        // allocated. A page ends only at a byte cap or 128 newline bytes.
        let meta_count_bound = counter.0 / PAGE_BREAKS + counter.0 / (PAGE_BYTES - 3) + 2;
        let bytes = heap
            .checked_add(counter.0)
            .and_then(|n| {
                n.checked_add(
                    (sql_count + meta_count_bound).checked_mul(size_of::<Range<usize>>())?,
                )
            })
            .and_then(|n| n.checked_add(EDITOR_BYTES + size_of::<Self>() + 64))
            .ok_or("DDL retention size overflow")?;
        let lease = Lease::new(budget, bytes)?;
        let mut text = String::with_capacity(counter.0);
        metadata(&artifact, &mut text).map_err(|_| "Could not format DDL metadata")?;
        if text.len() != counter.0 {
            return Err("DDL metadata size changed");
        }
        let sql_pages = ranges(&artifact.sql, sql_count);
        let metadata_pages = ranges(&text, meta_count_bound);
        Ok(Self {
            artifact: Arc::new(artifact),
            lease,
            metadata: text,
            sql_pages,
            metadata_pages,
        })
    }
    pub fn pages(&self, section: Section) -> usize {
        self.ranges(section).len()
    }
    fn ranges(&self, section: Section) -> &[Range<usize>] {
        match section {
            Section::Sql => &self.sql_pages,
            Section::Metadata => &self.metadata_pages,
        }
    }
    pub fn page(&self, section: Section, index: usize) -> Option<&str> {
        let text = match section {
            Section::Sql => &self.artifact.sql,
            Section::Metadata => &self.metadata,
        };
        let page = text.get(self.ranges(section).get(index)?.clone())?;
        (page.len() <= PAGE_BYTES
            && page.bytes().filter(|b| matches!(b, b'\r' | b'\n')).count() <= PAGE_BREAKS)
            .then_some(page)
    }
    pub fn status(&self, section: Section, index: usize) -> String {
        let range = self.ranges(section).get(index);
        format!(
            "{}: page {} of {}; bytes {}..{}. Full artifact: {} SQL bytes, {} relations. UTF-8 pages may split a long SQL line; saving uses every exact byte.",
            section.label(),
            index + 1,
            self.pages(section),
            range.map_or(0, |r| r.start),
            range.map_or(0, |r| r.end),
            self.artifact.sql.len(),
            self.artifact.relations.len()
        )
    }
    pub fn matches(&self, request: &DdlExportRequest) -> bool {
        same_scope(&self.artifact.request.scope, &request.scope)
    }
    pub fn refresh_for(&self, request: DdlExportRequest) -> DdlExportRequest {
        if !self.matches(&request) {
            return request;
        }
        let scope = match &self.artifact.request.scope {
            DdlExportScope::Database => DdlExportScope::Database,
            DdlExportScope::Schema { name, .. } => DdlExportScope::Schema {
                name: name.clone(),
                expected_oid: self.artifact.schemas.first().map(|s| s.oid),
            },
            DdlExportScope::Relation { schema, name, .. } => DdlExportScope::Relation {
                schema: schema.clone(),
                name: name.clone(),
                expected: self.artifact.relations.first().map(|r| DdlExportIdentity {
                    database_oid: r.identity.database_oid,
                    relation_oid: r.identity.relation_oid,
                }),
            },
        };
        DdlExportRequest {
            scope,
            expected_database_oid: Some(self.artifact.database_oid),
        }
    }
    /// Reserve before copying SQL for a file worker. The detached UI waiter
    /// keeps this lease plus the artifact lease until the owned worker joins.
    pub(crate) fn file_lease(&self) -> Result<Rc<Lease>, &'static str> {
        Lease::new(
            self.lease.budget.clone(),
            self.artifact
                .sql
                .len()
                .checked_mul(2)
                .and_then(|n| n.checked_add(64 * 1024))
                .ok_or("DDL file reservation overflow")?,
        )
    }
}
pub fn same_scope(a: &DdlExportScope, b: &DdlExportScope) -> bool {
    match (a, b) {
        (DdlExportScope::Database, DdlExportScope::Database) => true,
        (DdlExportScope::Schema { name: a, .. }, DdlExportScope::Schema { name: b, .. }) => a == b,
        (
            DdlExportScope::Relation {
                schema: a, name: b, ..
            },
            DdlExportScope::Relation {
                schema: c, name: d, ..
            },
        ) => a == c && b == d,
        _ => false,
    }
}
struct Counter(usize);
impl Write for Counter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.0 = self
            .0
            .checked_add(value.len())
            .filter(|n| *n <= METADATA_BYTES)
            .ok_or(fmt::Error)?;
        Ok(())
    }
}
fn metadata(a: &DdlExportArtifact, output: &mut impl Write) -> fmt::Result {
    writeln!(
        output,
        "DDL reconstruction, not a complete database dump. Review every omission before use. No SQL is executed by this tool.\nConnection: {}\nDatabase: {:?} (OID {})\nReader PID: {}\nCollected: {} to {}\nRequested scope: {:?}\nSQL: {} UTF-8 bytes\n",
        a.connection_id,
        a.database,
        a.database_oid,
        a.reader_pid,
        a.collected_start,
        a.collected_end,
        a.request.scope,
        a.sql.len()
    )?;
    writeln!(output, "Omissions and limits")?;
    for omission in &a.omissions {
        writeln!(output, "{}", omission.explanation())?;
    }
    writeln!(output, "\nSchemas ({}):", a.schemas.len())?;
    for schema in &a.schemas {
        writeln!(
            output,
            "{:?}: OID {}; CREATE SCHEMA included: {}",
            schema.name, schema.oid, schema.declared
        )?;
    }
    writeln!(
        output,
        "\nRelations ({}), in artifact order:",
        a.relations.len()
    )?;
    for r in &a.relations {
        writeln!(
            output,
            "{:?}.{:?}: {:?}; database OID {}, schema OID {}, relation OID {}; SQL bytes {}..{}",
            r.schema,
            r.name,
            r.kind,
            r.identity.database_oid,
            r.schema_oid,
            r.identity.relation_oid,
            r.sql_start,
            r.sql_end
        )?;
    }
    Ok(())
}
struct Ranges<'a> {
    text: &'a str,
    at: usize,
    empty: bool,
}
impl<'a> Ranges<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            at: 0,
            empty: text.is_empty(),
        }
    }
}
impl Iterator for Ranges<'_> {
    type Item = Range<usize>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.empty {
            self.empty = false;
            return Some(0..0);
        }
        if self.at >= self.text.len() {
            return None;
        }
        let start = self.at;
        let mut end = start;
        let mut breaks = 0;
        for (offset, ch) in self.text[start..].char_indices() {
            let next = start + offset + ch.len_utf8();
            if next - start > PAGE_BYTES {
                break;
            }
            end = next;
            if matches!(ch, '\r' | '\n') {
                breaks += 1;
            }
            if breaks == PAGE_BREAKS {
                break;
            }
        }
        self.at = end;
        Some(start..end)
    }
}
fn ranges(text: &str, capacity: usize) -> Vec<Range<usize>> {
    let mut result = Vec::with_capacity(capacity);
    result.extend(Ranges::new(text));
    result
}
#[cfg(test)]
mod tests;
