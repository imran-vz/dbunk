//! Complete-table captures and file preparation have separate shared leases.
//! A CSV route never requires or pretends to use a bounded retained capture.
use dbunk_lib::backend::{
    export_configurations::{ExportCompression, ExportEncoding, ExportFormat, ExportOptions},
    table_export::{TableExportCapture, TableExportIdentity, TableExportRequest},
};
use std::{cell::Cell, rc::Rc};
mod formatting;
pub use formatting::prepare;
pub const SHARED_BYTES: usize = 128 * 1024 * 1024;
pub const FIELD_BYTES: usize = 2 * 1024 * 1024;
pub const FILE_WORK_BYTES: usize = 36 * 1024 * 1024;
pub const FORMATS: [(ExportFormat, &str); 7] = [
    (ExportFormat::Csv, "CSV"),
    (ExportFormat::Json, "JSON"),
    (ExportFormat::Sql, "SQL INSERT"),
    (ExportFormat::Html, "HTML"),
    (ExportFormat::Markdown, "Markdown"),
    (ExportFormat::Txt, "Text"),
    (ExportFormat::Xlsx, "XLSX"),
];
pub struct Lease {
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Lease {
    pub fn new(budget: Rc<Cell<usize>>, bytes: usize) -> Result<Rc<Self>, &'static str> {
        if bytes > SHARED_BYTES.saturating_sub(budget.get()) {
            return Err(
                "Export needs shared memory; clear retained results or another capture and retry",
            );
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
#[derive(Clone)]
pub struct Capture {
    pub source: TableExportCapture,
    _lease: Rc<Lease>,
}
impl Capture {
    pub fn new(
        source: TableExportCapture,
        connection: &str,
        request: &TableExportRequest,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        let bytes = source
            .checked_heap_bytes()
            .ok_or("Complete table capture exceeds its bounds")?;
        let data = source.data();
        if data.connection_id != connection
            || data.schema != request.schema
            || data.table != request.table
            || request
                .expected
                .as_ref()
                .is_some_and(|expected| expected != &data.identity)
        {
            return Err("Complete table capture does not match the requested connection and table");
        }
        let lease = Lease::new(budget, bytes)?;
        Ok(Self {
            source,
            _lease: lease,
        })
    }
}
pub fn request(
    schema: &str,
    table: &str,
    expected: Option<TableExportIdentity>,
) -> TableExportRequest {
    TableExportRequest {
        schema: schema.to_owned(),
        table: table.to_owned(),
        expected,
    }
}
pub fn route(options: &ExportOptions) -> Result<bool, &'static str> {
    options
        .validate()
        .map_err(|_| "Export options exceed their bounds")?;
    if options.format == ExportFormat::Csv {
        if options.null_token.len() > 1024
            || options.null_token.chars().count() > 64
            || options.null_token.contains(['\0', '\r', '\n'])
        {
            return Err(
                "CSV NULL token must be at most 64 characters and contain no NUL or line breaks",
            );
        }
        if options.encoding != ExportEncoding::Utf8
            || options.compression != ExportCompression::None
        {
            return Err(
                "Whole-table CSV supports UTF-8 without gzip; change options to use the CSV transfer tool",
            );
        }
        return Ok(true);
    }
    Ok(false)
}
pub fn extension(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Csv => "csv",
        ExportFormat::Json => "json",
        ExportFormat::Sql => "sql",
        ExportFormat::Html => "html",
        ExportFormat::Markdown => "md",
        ExportFormat::Txt => "txt",
        ExportFormat::Xlsx => "xlsx",
    }
}
#[derive(Default)]
pub struct ReadState {
    serial: u64,
    pending: Option<(u64, bool)>,
}
impl ReadState {
    pub fn begin(&mut self) -> Result<u64, &'static str> {
        if self.pending.is_some() {
            return Err("Wait for the current capture to settle");
        }
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or("Capture request IDs exhausted")?;
        self.pending = Some((self.serial, false));
        Ok(self.serial)
    }
    pub fn cancel(&mut self) {
        if let Some((_, cancelled)) = &mut self.pending {
            *cancelled = true;
        }
    }
    pub fn settle(&mut self, id: u64) -> bool {
        if self.pending.is_none_or(|(pending, _)| pending != id) {
            return false;
        }
        !self.pending.take().unwrap().1
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn clear(&mut self) {
        self.pending = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> ExportOptions {
        ExportOptions {
            format: ExportFormat::Csv,
            encoding: ExportEncoding::Utf8,
            compression: ExportCompression::None,
            null_token: "\\N".into(),
        }
    }
    #[test]
    fn csv_route_is_independent_and_never_silently_changes_options() {
        let mut o = options();
        assert_eq!(route(&o), Ok(true));
        o.compression = ExportCompression::Gzip;
        assert!(route(&o).is_err());
        o.format = ExportFormat::Json;
        assert_eq!(route(&o), Ok(false));
        o.format = ExportFormat::Xlsx;
        o.encoding = ExportEncoding::Utf16Le;
        assert_eq!(route(&o), Ok(false));
    }
    #[test]
    fn literal_null_token_and_utf8_bounds() {
        let mut o = options();
        o.null_token = "雪".repeat(64);
        assert!(route(&o).is_ok());
        o.null_token.push('x');
        assert!(route(&o).is_err());
        o.null_token = String::new();
        assert!(route(&o).is_ok());
        o.null_token = "line\n".into();
        assert!(route(&o).is_err());
    }
    #[test]
    fn file_waiter_clone_keeps_source_allowance_and_overlap_refuses() {
        let budget = Rc::new(Cell::new(0));
        let old = Lease::new(budget.clone(), 80 * 1024 * 1024).unwrap();
        let worker = old.clone();
        assert!(Lease::new(budget.clone(), 64 * 1024 * 1024).is_err());
        drop(old);
        assert_eq!(budget.get(), 80 * 1024 * 1024);
        drop(worker);
        assert_eq!(budget.get(), 0);
    }
    #[test]
    fn cancelled_and_late_replies_cannot_replace_a_new_capture() {
        let mut read = ReadState::default();
        let old = read.begin().unwrap();
        read.cancel();
        assert!(!read.settle(old));
        let new = read.begin().unwrap();
        assert!(!read.settle(old));
        assert!(read.busy());
        assert!(read.settle(new));
        read.clear();
        assert!(!read.settle(new));
    }
}
