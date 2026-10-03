//! Bounded transient comparison state. Accepted endpoint identities are never
//! inferred from editable setup, and read navigation never computes byte cuts.
use dbunk_lib::backend::schema_comparisons::*;
use std::{cell::Cell, rc::Rc};
mod presentation;
mod reader;
pub use presentation::{
    ValueState, coverage_text, difference_label, failure_text, field_label, field_side_label,
    incomparable_label, job_failure_text, object_detail, object_identity, phase_label,
    summary_text, value_kind_label, value_state,
};
pub use reader::{Dispatch, Intent, ReadToken, ReaderState, Turn};

const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
pub struct Lease {
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Lease {
    pub(crate) fn new(budget: Rc<Cell<usize>>, bytes: usize) -> Result<Self, &'static str> {
        if bytes > WORKSPACE_BYTES.saturating_sub(budget.get()) {
            return Err(
                "Schema comparison exceeds the shared 128 MiB allowance; previous capture retained",
            );
        }
        budget.set(budget.get() + bytes);
        Ok(Self { budget, bytes })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}

/// Reserve while the opaque backend response still owns its serialization
/// lease, before moving its payload into the native model.
pub struct PageLease {
    _lease: Lease,
    heap: usize,
}
impl PageLease {
    pub fn new(page: &SchemaComparisonPage, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        let heap = page
            .checked_heap_bytes()
            .filter(|bytes| *bytes <= 4 * 1024 * 1024)
            .ok_or("Schema comparison page exceeds its decoded bound")?;
        let bytes = heap
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(64 * 1024))
            .ok_or("Schema comparison page size overflow")?;
        Ok(Self {
            _lease: Lease::new(budget, bytes)?,
            heap,
        })
    }
    fn admits(&self, page: &SchemaComparisonPage) -> bool {
        page.checked_heap_bytes()
            .is_some_and(|bytes| bytes <= self.heap)
    }
}
struct PageCapture {
    page: SchemaComparisonPage,
    _lease: PageLease,
}

/// Only safe connection labels cross into a comparison view.
pub struct ConnectionChoice {
    pub id: String,
    pub name: String,
    pub database: String,
    pub environment: String,
    pub schemas: Vec<String>,
}
pub struct Connections {
    rows: Vec<ConnectionChoice>,
    _lease: Lease,
}
impl Connections {
    pub fn new(rows: Vec<ConnectionChoice>, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        if rows.len() > 1024 {
            return Err("Comparison connection choices exceed 1,024 entries");
        }
        let mut bytes = rows
            .capacity()
            .checked_mul(std::mem::size_of::<ConnectionChoice>())
            .ok_or("Connection choice size overflow")?;
        for (index, row) in rows.iter().enumerate() {
            if !valid_text(&row.id, 128)
                || row.id.is_empty()
                || !valid_text(&row.name, 256)
                || !valid_text(&row.database, 256)
                || !valid_text(&row.environment, 64)
                || rows[..index].iter().any(|other| other.id == row.id)
            {
                return Err(
                    "Comparison connection choices contain invalid or duplicate identities",
                );
            }
            if row.schemas.len() > 512
                || row
                    .schemas
                    .iter()
                    .any(|name| name.is_empty() || !valid_text(name, 63))
            {
                return Err("Cached comparison schema choices exceed their bounds");
            }
            bytes = bytes
                .checked_add(
                    row.schemas
                        .capacity()
                        .checked_mul(std::mem::size_of::<String>())
                        .ok_or("Schema choice size overflow")?,
                )
                .ok_or("Schema choice size overflow")?;
            for schema in &row.schemas {
                bytes = bytes
                    .checked_add(schema.capacity())
                    .ok_or("Schema choice size overflow")?;
            }
            for value in [&row.id, &row.name, &row.database, &row.environment] {
                bytes = bytes
                    .checked_add(value.capacity())
                    .ok_or("Connection choice size overflow")?;
            }
        }
        if bytes > 1024 * 1024 {
            return Err("Comparison connection choices exceed 1 MiB");
        }
        Ok(Self {
            rows,
            _lease: Lease::new(budget, bytes * 2 + 64 * 1024)?,
        })
    }
    pub fn rows(&self) -> &[ConnectionChoice] {
        &self.rows
    }
    pub fn get(&self, id: &str) -> Option<&ConnectionChoice> {
        self.rows.iter().find(|row| row.id == id)
    }
}

pub struct JobCapture {
    rows: Vec<Status>,
    _lease: Lease,
}
impl JobCapture {
    pub fn new(list: SchemaComparisonList, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        if list.checked_heap_bytes().is_none() {
            return Err("Comparison job observation exceeds its bounds");
        }
        let rows = list.jobs;
        if rows.len() > 4 || rows.capacity() > 8 {
            return Err("Comparison job observation exceeds its bounds");
        }
        for (index, row) in rows.iter().enumerate() {
            if !valid_text(&row.job_id, 128)
                || !valid_text(&row.request_id, 128)
                || !valid_endpoint(&row.source)
                || !valid_endpoint(&row.target)
                || rows[..index].iter().any(|other| other.job_id == row.job_id)
            {
                return Err("Comparison job observation identity is invalid");
            }
            match &row.state {
                StatusState::Completed { result_id } if !valid_text(result_id, 128) => {
                    return Err("Comparison result identity exceeds bounds");
                }
                StatusState::Failed {
                    failure: CompareError::UnsupportedVersion { version, .. },
                } if !valid_text(version, 256) => return Err("Comparison failure exceeds bounds"),
                _ => {}
            }
        }
        Ok(Self {
            rows,
            _lease: Lease::new(budget, 64 * 1024)?,
        })
    }
    pub fn rows(&self) -> &[Status] {
        &self.rows
    }
    pub fn row(&self, id: &str) -> Option<&Status> {
        self.rows.iter().find(|row| row.job_id == id)
    }
    pub fn has_active(&self) -> bool {
        self.rows.iter().any(|row| active(&row.state))
    }
    pub fn matches(&self, list: &SchemaComparisonList) -> bool {
        self.rows == list.jobs
    }
}
pub fn active(state: &StatusState) -> bool {
    !matches!(
        state,
        StatusState::Completed { .. } | StatusState::Cancelled | StatusState::Failed { .. }
    )
}
fn valid_text(value: &String, max: usize) -> bool {
    value.len() <= max && value.capacity() <= max.saturating_mul(2) && !value.contains('\0')
}
fn valid_endpoint(endpoint: &Endpoint) -> bool {
    !endpoint.connection_id.is_empty()
        && valid_text(&endpoint.connection_id, 128)
        && !endpoint.schema.is_empty()
        && valid_text(&endpoint.schema, 63)
}
fn valid_request(request: &ResultRequest) -> bool {
    valid_endpoint(&request.source)
        && valid_endpoint(&request.target)
        && !request.identity.job_id.is_empty()
        && !request.identity.result_id.is_empty()
        && valid_text(&request.identity.job_id, 128)
        && valid_text(&request.identity.result_id, 128)
}
fn valid_relation(value: &RelationIdentity) -> bool {
    !value.name.is_empty() && valid_text(&value.name, 63)
}
fn valid_field(value: &FieldPath) -> bool {
    match value {
        FieldPath::Table { .. } => true,
        FieldPath::Column { name, .. } | FieldPath::Constraint { name, .. } => valid_text(name, 63),
        FieldPath::Index { name, owner, .. } | FieldPath::IndexKey { name, owner, .. } => {
            valid_text(name, 63) && owner.as_ref().is_none_or(|name| valid_text(name, 63))
        }
    }
}

#[cfg(test)]
mod tests;
