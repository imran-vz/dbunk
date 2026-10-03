//! Transient CSV setup and bounded observation captures. No path, preview,
//! mapping or execution authority in this module is workspace recovery data.
use dbunk_lib::backend::csv_transfers::*;
use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
};

mod mapping;
mod observations;
pub use mapping::Mapping;
pub use observations::{
    Capture, ImportChanges, ObservationOrder, direction_label, releasable, workbook_summary,
};

const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
const SETUP_BYTES: usize = 64 * 1024;
pub const INSPECTION_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

/// A lease must remain beside the object it accounts for, including opaque
/// review/confirmation handles held by the app-owned store.
pub struct Lease {
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Lease {
    pub fn inspection(budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        Self::new(budget, INSPECTION_CAPTURE_BYTES)
    }
    fn new(budget: Rc<Cell<usize>>, bytes: usize) -> Result<Self, &'static str> {
        if bytes > WORKSPACE_BYTES.saturating_sub(budget.get()) {
            return Err(
                "CSV capture exceeds the shared 128 MiB allowance; previous capture retained",
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

/// A closed/recreated view cannot accept another view's picker or inspection,
/// even if connection and revision numbers happen to match.
pub struct SetupToken {
    owner: uuid::Uuid,
    revision: u64,
}
pub struct Setup {
    owner: uuid::Uuid,
    revision: u64,
    connection: String,
    generation: u64,
    direction: CsvDirection,
    xlsx: bool,
    target: Option<CsvTarget>,
    options: CsvOptions,
    path: Option<PathBuf>,
    _lease: Lease,
}
impl Setup {
    pub fn new(
        connection: String,
        generation: u64,
        direction: CsvDirection,
        target: Option<CsvTarget>,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        if !valid_connection(&connection)
            || target.as_ref().is_some_and(|target| !valid_target(target))
        {
            return Err("CSV connection or target exceeds its bounds");
        }
        let lease = Lease::new(budget, SETUP_BYTES)?;
        Ok(Self {
            owner: uuid::Uuid::new_v4(),
            revision: 0,
            connection,
            generation,
            direction,
            xlsx: false,
            target,
            options: CsvOptions::default(),
            path: None,
            _lease: lease,
        })
    }
    pub fn owner(&self) -> uuid::Uuid {
        self.owner
    }
    pub fn connection(&self) -> &str {
        &self.connection
    }
    pub fn direction(&self) -> CsvDirection {
        self.direction
    }
    pub fn xlsx(&self) -> bool {
        self.xlsx && self.direction == CsvDirection::Import
    }
    pub fn set_xlsx(&mut self, xlsx: bool) -> Result<(), &'static str> {
        if self.xlsx != xlsx {
            self.invalidate()?;
            self.xlsx = xlsx;
            self.path = None;
        }
        if xlsx {
            self.set_direction(CsvDirection::Import)?;
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn target(&self) -> Option<&CsvTarget> {
        self.target.as_ref()
    }
    pub fn options(&self) -> &CsvOptions {
        &self.options
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub fn token(&self) -> SetupToken {
        SetupToken {
            owner: self.owner,
            revision: self.revision,
        }
    }
    pub fn is_current(&self, token: &SetupToken) -> bool {
        token.owner == self.owner && token.revision == self.revision
    }
    fn invalidate(&mut self) -> Result<(), &'static str> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("CSV setup revision exhausted; reopen setup")?;
        Ok(())
    }
    pub fn invalidate_review(&mut self) -> Result<(), &'static str> {
        self.invalidate()
    }
    pub fn set_direction(&mut self, direction: CsvDirection) -> Result<(), &'static str> {
        if self.direction != direction {
            self.invalidate()?;
            self.direction = direction;
            self.options = CsvOptions::default();
            self.path = None;
        }
        Ok(())
    }
    /// Option edits invalidate inspection/review authority but preserve the
    /// exact selected file, matching the existing CSV setup workflow.
    pub fn set_options(&mut self, options: CsvOptions) -> Result<(), &'static str> {
        validate_options(&options)?;
        if self.options != options {
            self.invalidate()?;
            self.options = options;
        }
        Ok(())
    }
    pub fn set_target(&mut self, target: Option<CsvTarget>) -> Result<(), &'static str> {
        if target.as_ref().is_some_and(|target| !valid_target(target)) {
            return Err("Choose a valid CSV schema and table");
        }
        if self.target != target {
            self.invalidate()?;
            self.target = target;
            self.path = None;
        }
        Ok(())
    }
    pub fn retarget(&mut self, connection: String, generation: u64) -> Result<(), &'static str> {
        if !valid_connection(&connection) {
            return Err("CSV connection exceeds its bounds");
        }
        if self.connection != connection || self.generation != generation {
            self.invalidate()?;
            self.connection = connection;
            self.generation = generation;
            self.target = None;
            self.path = None;
        }
        Ok(())
    }
    pub fn accept_path(&mut self, token: &SetupToken, path: PathBuf) -> Result<(), &'static str> {
        if !self.is_current(token) {
            return Err("File selection belongs to an older CSV setup");
        }
        if path.to_str().is_none()
            || !path.is_absolute()
            || path.file_name().is_none()
            || path.as_os_str().as_encoded_bytes().contains(&0)
            || path.as_os_str().len() > MAX_CSV_PATH_BYTES
            || path.capacity() > MAX_CSV_PATH_BYTES
            || path.file_name().is_some_and(|name| name.len() > 1024)
        {
            return Err("Select an absolute local Unicode file path within 4 KiB");
        }
        self.invalidate()?;
        self.path = Some(path);
        Ok(())
    }
    /// Exact identity check in addition to the local revision fence. Matching
    /// fields alone never make a late response authoritative.
    pub fn matches_inspection(&self, token: &SetupToken, data: &CsvInspectionData) -> bool {
        self.is_current(token)
            && data.connection_id == self.connection
            && data.direction == self.direction
            && self.target.as_ref() == Some(&data.target)
            && if self.xlsx() {
                data.workbook
                    .as_ref()
                    .is_some_and(|source| source.null_token == self.options.null_token)
            } else {
                data.workbook.is_none() && data.options == self.options
            }
    }
    pub fn inspection_intent(&self) -> Result<CsvInspectionIntent, &'static str> {
        let target = self
            .target
            .clone()
            .ok_or("Choose the exact schema and table first")?;
        validate_options(&self.options)?;
        match self.direction {
            CsvDirection::Import if self.xlsx() => CsvInspectionIntent::xlsx(
                self.path.clone().ok_or("Select an XLSX source first")?,
                target,
                self.options.null_token.clone(),
            ),
            CsvDirection::Import => CsvInspectionIntent::import(
                self.path.clone().ok_or("Select a CSV source first")?,
                target,
                self.options.clone(),
            ),
            CsvDirection::Export => CsvInspectionIntent::export(target, self.options.clone()),
        }
        .map_err(|_| "CSV inspection inputs exceed their bounds")
    }
}

pub fn validate_options(options: &CsvOptions) -> Result<(), &'static str> {
    // The native UI deliberately uses Unicode characters, not UTF-16 code
    // units: at most 64 characters, still below the backend's 1 KiB limit.
    if options.null_token.len() > 1024
        || options.null_token.capacity() > 1024
        || [&options.delimiter, &options.quote, &options.escape]
            .into_iter()
            .any(|value| value.len() != 1 || value.capacity() > 64)
        || options.null_token.chars().count() > 64
        || options
            .checked_heap_bytes()
            .is_none_or(|bytes| bytes > 4096)
    {
        return Err(
            "CSV options require one-byte delimiter, quote and escape; NULL is limited to 64 characters and 1 KiB",
        );
    }
    Ok(())
}
fn valid_connection(value: &String) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.capacity() <= 1024
        && !value.chars().any(char::is_control)
}
fn valid_target(target: &CsvTarget) -> bool {
    target
        .checked_heap_bytes()
        .is_some_and(|bytes| bytes <= 1024)
}

/// The sample stays in the backend's immutable Arc. The UI accounts for that
/// retained ownership and bounded display work without copying cell strings.
pub struct InspectionCapture {
    inspection: CsvInspection,
    _lease: Lease,
}
impl InspectionCapture {
    pub fn new(inspection: CsvInspection, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        if inspection.retained_bytes() > MAX_CSV_INSPECTION_BYTES
            || inspection
                .data()
                .checked_heap_bytes()
                .is_none_or(|bytes| bytes > MAX_CSV_INSPECTION_BYTES)
        {
            return Err("CSV inspection exceeds its 4 MiB bound; previous inspection retained");
        }
        Ok(Self {
            inspection,
            _lease: Lease::inspection(budget)?,
        })
    }
    pub fn data(&self) -> &CsvInspectionData {
        self.inspection.data()
    }
    /// The caller transfers this handle straight into the byte-accounted
    /// command queue. It must not retain an uncharged copy in the view.
    pub fn take(self) -> CsvInspection {
        self.inspection
    }
}

#[cfg(test)]
mod tests;
