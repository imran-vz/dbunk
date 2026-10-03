//! Bounded, exact-text XLSX import. The caller owns the immutable input snapshot,
//! output artifact, cancellation lifetime and 64 MiB execution reservation.
//! This module never opens paths, publishes files, evaluates formulas or connects.

use std::{
    fmt,
    io::{Read, Seek, Write},
    sync::atomic::{AtomicBool, Ordering},
};

mod archive;
mod cells;
mod metadata;
mod sheet;
mod strings;
#[cfg(test)]
mod tests;
mod xml;

pub(crate) const WORKING_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_XML_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_METADATA_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_FIELD_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_RECORD_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_COLUMNS: usize = 1_600;
pub(crate) const MAX_LOGICAL_CELLS: u64 = 4_000_000;
pub(crate) const MAX_SHEETS: usize = 256;
const MAX_STRINGS: usize = 100_000;
const MAX_STRINGS_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SheetId(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SheetVisibility {
    Visible,
    Hidden,
    VeryHidden,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SheetInfo {
    pub id: SheetId,
    pub name: String,
    pub visibility: SheetVisibility,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MaterializedSheet {
    pub rows: u64,
    pub columns: u16,
    pub header_detected: bool,
    /// Formulas are never evaluated. Only present cached values are imported.
    pub cached_formula_cells: u64,
}

/// Errors never retain paths, cell contents, XML or library error strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum XlsxError {
    Cancelled,
    InputIo,
    OutputIo,
    InvalidZip,
    UnsupportedZip,
    AmbiguousZip,
    InvalidXml,
    UnsupportedXml,
    InvalidWorkbook,
    UnsupportedCell,
    MissingFormulaCache,
    InvalidSheet,
    InvalidNullToken,
    Limit(Limit),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Limit {
    Archive,
    Xml,
    Structure,
    Sheets,
    SharedStrings,
    Field,
    Record,
    Columns,
    Sparse,
}

impl fmt::Display for XlsxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Cancelled => "XLSX preparation cancelled",
            Self::InputIo => "Cannot read the XLSX snapshot",
            Self::OutputIo => "Cannot write the private import artifact",
            Self::InvalidZip => "Invalid XLSX ZIP archive",
            Self::UnsupportedZip => {
                "XLSX requires a classic, unencrypted stored/deflated ZIP archive"
            }
            Self::AmbiguousZip => "XLSX archive has ambiguous ZIP end signatures",
            Self::InvalidXml => "Invalid XLSX XML",
            Self::UnsupportedXml => "XLSX requires UTF-8 XML without DTDs or custom entities",
            Self::InvalidWorkbook => "Invalid XLSX workbook structure or references",
            Self::UnsupportedCell => "Unsupported XLSX cell value",
            Self::MissingFormulaCache => {
                "XLSX formula has no cached value; recalculate and save the workbook first"
            }
            Self::InvalidSheet => "Selected XLSX sheet is unavailable",
            Self::InvalidNullToken => "Invalid XLSX NULL token",
            Self::Limit(limit) => return write!(f, "XLSX exceeds the native {limit:?} limit"),
        };
        f.write_str(message)
    }
}
impl std::error::Error for XlsxError {}

pub(crate) struct Workbook<R: Read + Seek> {
    archive: zip::ZipArchive<R>,
    sheets: Vec<SheetInfo>,
    paths: Vec<String>,
    strings_path: Option<String>,
}

impl<R: Read + Seek> Workbook<R> {
    /// Requires an immutable, already owned snapshot. No workbook values are retained.
    pub fn open(mut source: R, cancelled: &AtomicBool) -> Result<Self, XlsxError> {
        archive::preflight(&mut source, cancelled)?;
        let archive = zip::ZipArchive::with_config(
            zip::read::Config {
                archive_offset: zip::read::ArchiveOffset::Known(0),
            },
            source,
        )
        .map_err(|_| XlsxError::InvalidZip)?;
        let mut workbook = Self {
            archive,
            sheets: Vec::new(),
            paths: Vec::new(),
            strings_path: None,
        };
        metadata::load(&mut workbook, cancelled)?;
        Ok(workbook)
    }

    pub fn sheets(&self) -> &[SheetInfo] {
        &self.sheets
    }

    /// Writes canonical CSV (header always present). Numeric values retain their
    /// XML spelling; dates stored as numbers remain serial numbers. Partial output
    /// on error is unusable and remains the caller's cleanup responsibility.
    pub fn write_sheet(
        &mut self,
        sheet: SheetId,
        null_token: &str,
        output: &mut impl Write,
        cancelled: &AtomicBool,
    ) -> Result<MaterializedSheet, XlsxError> {
        check_cancel(cancelled)?;
        if null_token.len() > 1024
            || null_token.chars().count() > 64
            || null_token
                .bytes()
                .any(|c| matches!(c, 0 | b',' | b'"' | b'\r' | b'\n'))
        {
            return Err(XlsxError::InvalidNullToken);
        }
        let path = self
            .paths
            .get(sheet.0 as usize)
            .ok_or(XlsxError::InvalidSheet)?;
        // Validate hostile XML while there is no retained shared-string table.
        let bytes = archive::part(&mut self.archive, path, MAX_XML_BYTES, cancelled)?;
        xml::validate(&bytes, cancelled)?;
        drop(bytes);
        let strings = match &self.strings_path {
            Some(path) => {
                let bytes = archive::part(&mut self.archive, path, MAX_XML_BYTES, cancelled)?;
                xml::validate(&bytes, cancelled)?;
                strings::read(&bytes, cancelled)?
            }
            None => Vec::new(),
        };
        // The private source is immutable, so the previously validated bytes cannot change.
        let bytes = archive::part(&mut self.archive, path, MAX_XML_BYTES, cancelled)?;
        // Escaping quote-heavy cells must not produce one filesystem syscall per
        // quote. Explicitly disassemble on error so Drop cannot flush cancelled work.
        let mut buffered = std::io::BufWriter::with_capacity(64 * 1024, output);
        let result = sheet::write(&bytes, &strings, null_token, &mut buffered, cancelled);
        let result = result.and_then(|summary| {
            buffered.flush().map_err(|_| XlsxError::OutputIo)?;
            check_cancel(cancelled)?;
            Ok(summary)
        });
        let _ = buffered.into_parts();
        result
    }
}

fn check_cancel(cancelled: &AtomicBool) -> Result<(), XlsxError> {
    if cancelled.load(Ordering::Relaxed) {
        Err(XlsxError::Cancelled)
    } else {
        Ok(())
    }
}
