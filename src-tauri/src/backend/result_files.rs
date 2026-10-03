//! Bounded retained-result files. These synchronous operations belong on an
//! owned host worker. Cancellation is cooperative; publishing admission is atomic.
//! XLSX writes text, never inferred numbers, formulas or hyperlinks. Its explicit
//! NULL token can collide with text; empty strings become blank Excel cells.
use std::{
    fmt,
    io::{self, Cursor, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};

pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_XLSX_CELLS: usize = 100_000;
pub const MAX_XLSX_ROWS: usize = 100_000;
pub const MAX_XLSX_COLUMNS: usize = 1_024;
pub const MAX_XLSX_CELL_UNITS: usize = 32_767;
const CHUNK: usize = 64 * 1024;
const ACTIVE: u8 = 0;
const CANCELLED: u8 = 1;
const PUBLISHING: u8 = 2;
const FINISHED: u8 = 3;

/// One token per file job. cancel never waits for filesystem or encoding work.
#[derive(Clone, Debug, Default)]
pub struct Cancellation(Arc<AtomicU8>);
impl Cancellation {
    /// True only when cancellation wins publication admission (or already won).
    pub fn cancel(&self) -> bool {
        matches!(
            self.0
                .compare_exchange(ACTIVE, CANCELLED, Ordering::AcqRel, Ordering::Acquire),
            Ok(_) | Err(CANCELLED)
        )
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == CANCELLED
    }
    fn check(&self) -> Result<(), Error> {
        match self.0.load(Ordering::Acquire) {
            ACTIVE => Ok(()),
            CANCELLED => Err(Error::Cancelled),
            _ => Err(Error::JobFinished),
        }
    }
    fn admit_publication(&self) -> Result<(), Error> {
        self.0
            .compare_exchange(ACTIVE, PUBLISHING, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|state| {
                if state == CANCELLED {
                    Error::Cancelled
                } else {
                    Error::JobFinished
                }
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    Partial,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    CompleteResult,
    RetainedRows,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub completeness: Completeness,
    pub scope: Scope,
    pub row_count: usize,
}
impl Source {
    fn validate(self) -> Result<(), Error> {
        if self.completeness == Completeness::Partial && self.scope == Scope::CompleteResult {
            return Err(Error::PartialSource);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct PreparedFile {
    bytes: Vec<u8>,
    source: Source,
}
impl PreparedFile {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
    pub fn source(&self) -> Source {
        self.source
    }
}
#[derive(Debug)]
pub struct PublishedFile {
    pub path: PathBuf,
    pub bytes: usize,
    pub source: Source,
}

#[derive(Debug)]
pub enum Error {
    Cancelled,
    JobFinished,
    PartialSource,
    InputTooLarge,
    OutputTooLarge,
    InvalidProjection,
    InvalidRowWidth,
    InvalidRowCount,
    CellTooLong,
    UnsupportedCellText,
    InvalidDestination,
    DestinationExists,
    Xlsx(String),
    Io(io::Error),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("File export cancelled before publication"),
            Self::JobFinished => f.write_str("File export job has already entered publication"),
            Self::PartialSource => f.write_str("Partial results require explicit retained-row export"),
            Self::InputTooLarge => f.write_str("File export exceeds its input byte, row, column or cell limit"),
            Self::OutputTooLarge => f.write_str("Prepared file exceeds 8 MiB"),
            Self::InvalidProjection => f.write_str("XLSX projection does not match its columns or source rows"),
            Self::InvalidRowWidth => f.write_str("XLSX row does not match its columns"),
            Self::InvalidRowCount => f.write_str("Export row-count disclosure does not match its source"),
            Self::CellTooLong => f.write_str("XLSX cell exceeds 32,767 UTF-16 units"),
            Self::UnsupportedCellText => f.write_str("XLSX writer cannot safely encode this Unicode text following '_x'; use a text export"),
            Self::InvalidDestination => f.write_str("Export requires an absolute file path in an existing directory"),
            Self::DestinationExists => f.write_str("Export destination already exists; choose a new file"),
            Self::Xlsx(message) => write!(f, "XLSX preparation failed: {message}"),
            Self::Io(error) => write!(f, "File export failed: {error}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Owns existing formatted bytes without copying. Gzip covers the exact bytes,
/// including any UTF-16 BOM. Both encoded input and compressed output are capped.
pub fn prepare_bytes(
    bytes: Vec<u8>,
    compression: Compression,
    source: Source,
    cancel: &Cancellation,
) -> Result<PreparedFile, Error> {
    cancel.check()?;
    source.validate()?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::InputTooLarge);
    }
    if compression == Compression::None {
        return Ok(PreparedFile { bytes, source });
    }
    let mut output = BoundedBuffer::new(cancel);
    let result = (|| {
        let mut encoder =
            flate2::write::GzEncoder::new(&mut output, flate2::Compression::default());
        for chunk in bytes.chunks(CHUNK) {
            cancel.check()?;
            encoder.write_all(chunk).map_err(Error::Io)?;
        }
        encoder.finish().map_err(Error::Io)?;
        Ok(())
    })();
    output.check_result(result)?;
    Ok(PreparedFile {
        bytes: output.bytes.into_inner(),
        source,
    })
}

pub struct XlsxTable<'a> {
    pub columns: &'a [&'a str],
    pub rows: &'a [&'a [Option<String>]],
    /// Paired display-order indices; hidden cells are never scanned or copied.
    pub source_columns: Option<&'a [usize]>,
    pub sheet_name: &'a str,
    pub null_as: &'a str,
    pub source: Source,
}
impl XlsxTable<'_> {
    fn value<'a>(&'a self, row: &'a [Option<String>], index: usize) -> &'a str {
        row[self.source_columns.map_or(index, |columns| columns[index])]
            .as_deref()
            .unwrap_or(self.null_as)
    }
}

/// Validate before constructing any workbook cells. Input includes repeated NULL
/// tokens and headings; the selected cell cap also includes the heading row.
pub fn prepare_xlsx(
    table: &XlsxTable<'_>,
    compression: Compression,
    cancel: &Cancellation,
) -> Result<PreparedFile, Error> {
    validate_xlsx(table, cancel)?;
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet
        .set_name(table.sheet_name)
        .map_err(|error| Error::Xlsx(error.to_string()))?;
    for (column, text) in table.columns.iter().enumerate() {
        worksheet
            .write_string(0, column as u16, *text)
            .map_err(|error| Error::Xlsx(error.to_string()))?;
    }
    for (row_index, row) in table.rows.iter().enumerate() {
        cancel.check()?;
        for index in 0..table.columns.len() {
            worksheet
                .write_string(
                    (row_index + 1) as u32,
                    index as u16,
                    table.value(row, index),
                )
                .map_err(|error| Error::Xlsx(error.to_string()))?;
        }
    }
    let mut output = BoundedBuffer::new(cancel);
    let result = workbook
        .save_to_writer(&mut output)
        .map_err(|error| Error::Xlsx(error.to_string()));
    output.check_result(result)?;
    // Release workbook cells and generated XML before gzip allocates its output.
    drop(workbook);
    prepare_bytes(output.bytes.into_inner(), compression, table.source, cancel)
}

fn validate_xlsx(table: &XlsxTable<'_>, cancel: &Cancellation) -> Result<(), Error> {
    cancel.check()?;
    table.source.validate()?;
    if table.source.row_count != table.rows.len() {
        return Err(Error::InvalidRowCount);
    }
    if table.columns.len() > MAX_XLSX_COLUMNS
        || table.rows.len() > MAX_XLSX_ROWS
        || table
            .columns
            .len()
            .saturating_mul(table.rows.len().saturating_add(1))
            > MAX_XLSX_CELLS
    {
        return Err(Error::InputTooLarge);
    }
    if table
        .source_columns
        .is_some_and(|projection| projection.len() != table.columns.len())
    {
        return Err(Error::InvalidProjection);
    }
    let mut bytes = 0usize;
    let mut count = |text: &str| -> Result<(), Error> {
        bytes = bytes.saturating_add(text.len());
        if bytes > MAX_FILE_BYTES {
            return Err(Error::InputTooLarge);
        }
        if text.encode_utf16().count() > MAX_XLSX_CELL_UNITS {
            return Err(Error::CellTooLong);
        }
        // Locked rust_xlsxwriter 0.84 slices four bytes after `_x` without a
        // Unicode-boundary check. Refuse its panic case without changing text.
        for (index, _) in text.match_indices("_x") {
            if index + 7 <= text.len() && text.get(index + 2..index + 6).is_none() {
                return Err(Error::UnsupportedCellText);
            }
        }
        Ok(())
    };
    count(table.sheet_name)?;
    count(table.null_as)?;
    for text in table.columns {
        count(text)?;
    }
    for row in table.rows {
        cancel.check()?;
        match table.source_columns {
            Some(projection) if projection.iter().any(|index| *index >= row.len()) => {
                return Err(Error::InvalidProjection)
            }
            None if row.len() != table.columns.len() => return Err(Error::InvalidRowWidth),
            _ => {}
        }
        for index in 0..table.columns.len() {
            count(table.value(row, index))?;
        }
    }
    Ok(())
}

/// Private sibling temp files clean themselves up on errors/cancellation. The
/// final CAS admits an indivisible no-clobber publication: cancellation after
/// admission loses, but a filesystem failure still returns an error.
pub fn publish_new(
    prepared: PreparedFile,
    destination: &Path,
    cancel: &Cancellation,
) -> Result<PublishedFile, Error> {
    cancel.check()?;
    let parent = destination
        .parent()
        .filter(|_| destination.is_absolute() && destination.file_name().is_some())
        .ok_or(Error::InvalidDestination)?;
    if !std::fs::metadata(parent)?.is_dir() {
        return Err(Error::InvalidDestination);
    }
    match std::fs::symlink_metadata(destination) {
        Ok(_) => return Err(Error::DestinationExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::Io(error)),
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".dbunk-result-")
        .tempfile_in(parent)?;
    for chunk in prepared.bytes.chunks(CHUNK) {
        cancel.check()?;
        temporary.write_all(chunk)?;
    }
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    publish_temporary(temporary, destination, cancel)?;
    Ok(PublishedFile {
        path: destination.to_owned(),
        bytes: prepared.bytes.len(),
        source: prepared.source,
    })
}

fn publish_temporary(
    temporary: tempfile::NamedTempFile,
    destination: &Path,
    cancel: &Cancellation,
) -> Result<(), Error> {
    cancel.admit_publication()?;
    let result = temporary.persist_noclobber(destination);
    cancel.0.store(FINISHED, Ordering::Release);
    result.map_err(|error| {
        if error.error.kind() == io::ErrorKind::AlreadyExists {
            Error::DestinationExists
        } else {
            Error::Io(error.error)
        }
    })?;
    Ok(())
}

struct BoundedBuffer<'a> {
    bytes: Cursor<Vec<u8>>,
    cancel: &'a Cancellation,
    oversized: bool,
}
impl<'a> BoundedBuffer<'a> {
    fn new(cancel: &'a Cancellation) -> Self {
        Self {
            bytes: Cursor::new(Vec::new()),
            cancel,
            oversized: false,
        }
    }
    fn check_result(&self, result: Result<(), Error>) -> Result<(), Error> {
        self.cancel.check()?;
        if self.oversized {
            return Err(Error::OutputTooLarge);
        }
        result
    }
}
impl Write for BoundedBuffer<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.cancel.check().map_err(io::Error::other)?;
        if self.bytes.position().saturating_add(bytes.len() as u64) > MAX_FILE_BYTES as u64 {
            self.oversized = true;
            return Err(io::Error::other("prepared file exceeds output limit"));
        }
        let end = self.bytes.position() as usize + bytes.len();
        let buffer = self.bytes.get_mut();
        if buffer.capacity() < end {
            buffer
                .try_reserve_exact(end.saturating_sub(buffer.len()))
                .map_err(io::Error::other)?;
        }
        self.bytes.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.cancel.check().map_err(io::Error::other)
    }
}
impl Seek for BoundedBuffer<'_> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.cancel.check().map_err(io::Error::other)?;
        let position = match from {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::End(offset) => self.bytes.get_ref().len() as i128 + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.bytes.position()) + i128::from(offset),
        };
        if !(0..=MAX_FILE_BYTES as i128).contains(&position) {
            self.oversized = true;
            return Err(io::Error::other("prepared file seek exceeds output limit"));
        }
        self.bytes.set_position(position as u64);
        Ok(position as u64)
    }
}

#[cfg(test)]
mod tests;
