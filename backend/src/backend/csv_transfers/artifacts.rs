//! Registry-owned files. UI handles contain metadata only. Every file is installed
//! here before work can fail or its reply can be abandoned; cleanup explicitly
//! joins the file/parser workers before unlinking and retains failed owners.
use super::*;
use crate::postgres::backup::native::{source, Ownership};
use std::{
    io::{Seek, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit};

pub(super) const MAX_WORKBOOK_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const MAX_CANONICAL_BYTES: u64 = 512 * 1024 * 1024;
#[derive(Default)]
struct Files {
    snapshot: Option<tempfile::NamedTempFile>,
    canonical: Option<tempfile::NamedTempFile>,
    bytes: u64,
}
pub(super) struct Artifacts {
    files: Mutex<Files>,
    owner: Ownership,
    cancellation: source::Cancellation,
    cancelled: AtomicBool,
    cleaned: AtomicBool,
    cleanup_gate: AsyncMutex<()>,
}
impl Artifacts {
    pub fn new(owner: Ownership) -> Arc<Self> {
        Arc::new(Self {
            files: Mutex::new(Files::default()),
            owner,
            cancellation: Default::default(),
            cancelled: AtomicBool::new(false),
            cleaned: AtomicBool::new(false),
            cleanup_gate: AsyncMutex::new(()),
        })
    }
    #[cfg(test)]
    pub fn paths(&self) -> Vec<PathBuf> {
        let files = self.files.lock().unwrap();
        files
            .snapshot
            .iter()
            .chain(files.canonical.iter())
            .map(|f| f.path().to_owned())
            .collect()
    }
    pub fn cancel(&self) {
        self.cancellation.cancel();
        self.cancelled.store(true, Ordering::Release);
    }
    pub async fn snapshot(
        self: &Arc<Self>,
        id: CsvInspectionId,
        path: PathBuf,
        permit: OwnedSemaphorePermit,
    ) -> Result<Arc<CsvWorkbookData>, CsvError> {
        let this = self.clone();
        let task = self
            .owner
            .spawn_blocking(move || {
                let _permit = permit;
                let mut files = this.files.lock().map_err(|_| CsvError::Cleanup)?;
                files.snapshot = Some(
                    tempfile::Builder::new()
                        .prefix("dbunk-native-workbook-")
                        .tempfile()
                        .map_err(|_| CsvError::FileIo)?,
                );
                files.bytes = source::copy_into_bounded(
                    &path,
                    &this.cancellation,
                    files.snapshot.as_mut().unwrap(),
                    MAX_WORKBOOK_BYTES,
                    |_| {},
                )
                .map_err(|e| match e {
                    source::Error::Cancelled => CsvError::Cancelled,
                    source::Error::Changed => CsvError::SourceChanged,
                    source::Error::Limit => CsvError::Limit,
                    _ => CsvError::FileIo,
                })?;
                let file = files.snapshot.as_mut().unwrap().as_file_mut();
                file.rewind().map_err(|_| CsvError::FileIo)?;
                let workbook = crate::xlsx_native::Workbook::open(file, &this.cancelled)
                    .map_err(parser_error)?;
                let sheets = workbook
                    .sheets()
                    .iter()
                    .enumerate()
                    .map(|(index, s)| CsvWorkbookSheet {
                        index: index as u16,
                        name: s.name.clone(),
                        visibility: match s.visibility {
                            crate::xlsx_native::SheetVisibility::Visible => {
                                CsvWorkbookVisibility::Visible
                            }
                            crate::xlsx_native::SheetVisibility::Hidden => {
                                CsvWorkbookVisibility::Hidden
                            }
                            crate::xlsx_native::SheetVisibility::VeryHidden => {
                                CsvWorkbookVisibility::VeryHidden
                            }
                        },
                    })
                    .collect();
                drop(workbook);
                let data = CsvWorkbookData {
                    inspection_id: id,
                    file_name: path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .ok_or(CsvError::InvalidRequest)?
                        .to_owned(),
                    workbook_bytes: files.bytes,
                    sheets,
                };
                data.checked_heap_bytes().ok_or(CsvError::Limit)?;
                Ok(Arc::new(data))
            })
            .map_err(|_| CsvError::Busy)?;
        task.await
            .map_err(|_| CsvError::Cleanup)?
            .map_err(|_| CsvError::Cleanup)?
    }
    pub async fn materialize(
        self: &Arc<Self>,
        sheet: u16,
        null_token: String,
        permit: OwnedSemaphorePermit,
    ) -> Result<(PathBuf, CsvWorkbookSource), CsvError> {
        let this = self.clone();
        let task = self
            .owner
            .spawn_blocking(move || {
                let _permit = permit;
                let mut files = this.files.lock().map_err(|_| CsvError::Cleanup)?;
                if this.cancelled.load(Ordering::Acquire) {
                    return Err(CsvError::Cancelled);
                }
                // A new selection cannot inherit any previous canonical bytes.
                remove(&mut files.canonical)?;
                files.canonical = Some(
                    tempfile::Builder::new()
                        .prefix("dbunk-native-sheet-")
                        .tempfile()
                        .map_err(|_| CsvError::FileIo)?,
                );
                let Files {
                    snapshot,
                    canonical,
                    bytes,
                } = &mut *files;
                let input = snapshot
                    .as_mut()
                    .ok_or(CsvError::StaleReview)?
                    .as_file_mut();
                input.rewind().map_err(|_| CsvError::FileIo)?;
                let mut workbook = crate::xlsx_native::Workbook::open(input, &this.cancelled)
                    .map_err(parser_error)?;
                let chosen = workbook
                    .sheets()
                    .get(sheet as usize)
                    .ok_or(CsvError::InvalidRequest)?
                    .clone();
                let output = canonical.as_mut().unwrap();
                let mut bounded = LimitedWriter {
                    file: output.as_file_mut(),
                    written: 0,
                    exceeded: false,
                };
                let result =
                    workbook.write_sheet(chosen.id, &null_token, &mut bounded, &this.cancelled);
                if bounded.exceeded {
                    return Err(CsvError::Limit);
                }
                let summary = result.map_err(parser_error)?;
                let canonical_bytes = bounded.written;
                drop(workbook);
                output.as_file().sync_all().map_err(|_| CsvError::FileIo)?;
                let provenance = CsvWorkbookSource {
                    sheet_index: sheet,
                    sheet_name: chosen.name,
                    workbook_bytes: *bytes,
                    canonical_bytes,
                    null_token,
                    header_detected: summary.header_detected,
                    rows: summary.rows,
                    cached_formula_cells: summary.cached_formula_cells,
                };
                provenance.checked_heap_bytes().ok_or(CsvError::Limit)?;
                Ok((output.path().to_owned(), provenance))
            })
            .map_err(|_| CsvError::Busy)?;
        task.await
            .map_err(|_| CsvError::Cleanup)?
            .map_err(|_| CsvError::Cleanup)?
    }
    pub async fn cleanup(self: &Arc<Self>, deadline: tokio::time::Instant) -> Result<(), CsvError> {
        let _gate = self.cleanup_gate.lock().await;
        if self.cleaned.load(Ordering::Acquire) {
            return Ok(());
        }
        self.cancel();
        self.owner
            .drain_until(deadline)
            .await
            .map_err(|_| CsvError::Cleanup)?;
        let this = self.clone();
        let task = self
            .owner
            .spawn_blocking(move || {
                let mut files = this.files.lock().map_err(|_| CsvError::Cleanup)?;
                remove(&mut files.canonical)?;
                remove(&mut files.snapshot)?;
                this.cleaned.store(true, Ordering::Release);
                Ok(())
            })
            .map_err(|_| CsvError::Cleanup)?;
        tokio::time::timeout_at(deadline, task)
            .await
            .map_err(|_| CsvError::Cleanup)?
            .map_err(|_| CsvError::Cleanup)?
            .map_err(|_| CsvError::Cleanup)?
    }
}
fn remove(slot: &mut Option<tempfile::NamedTempFile>) -> Result<(), CsvError> {
    if let Some(file) = slot.as_ref() {
        std::fs::remove_file(file.path()).map_err(|_| CsvError::Cleanup)?;
        slot.take();
    }
    Ok(())
}
struct LimitedWriter<'a> {
    file: &'a mut std::fs::File,
    written: u64,
    exceeded: bool,
}
impl Write for LimitedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.written.saturating_add(bytes.len() as u64) > MAX_CANONICAL_BYTES {
            self.exceeded = true;
            return Err(std::io::Error::other("canonical CSV limit"));
        }
        let written = self.file.write(bytes)?;
        self.written += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
fn parser_error(error: crate::xlsx_native::XlsxError) -> CsvError {
    use crate::xlsx_native::XlsxError;
    match error {
        XlsxError::Cancelled => CsvError::Cancelled,
        XlsxError::Limit(_) => CsvError::Limit,
        XlsxError::InputIo | XlsxError::OutputIo => CsvError::FileIo,
        XlsxError::InvalidNullToken => CsvError::InvalidOptions,
        XlsxError::MissingFormulaCache => CsvError::MissingFormulaCache,
        XlsxError::UnsupportedZip | XlsxError::UnsupportedXml | XlsxError::UnsupportedCell => {
            CsvError::UnsupportedWorkbook
        }
        XlsxError::InvalidZip
        | XlsxError::AmbiguousZip
        | XlsxError::InvalidXml
        | XlsxError::InvalidWorkbook
        | XlsxError::InvalidSheet => CsvError::InvalidWorkbook,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_limit_is_distinct_from_io_failure_and_refuses_before_write() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let mut writer = LimitedWriter {
            file: file.as_file_mut(),
            written: MAX_CANONICAL_BYTES - 1,
            exceeded: false,
        };
        assert!(writer.write_all(b"xx").is_err());
        assert!(writer.exceeded);
        assert_eq!(writer.written, MAX_CANONICAL_BYTES - 1);
        assert_eq!(file.as_file().metadata().unwrap().len(), 0);
    }
    #[tokio::test]
    async fn failed_unlink_keeps_owner_until_explicit_success() {
        let source = Artifacts::new(Ownership::default());
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_owned();
        source.files.lock().unwrap().snapshot = Some(file);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            source
                .cleanup(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
                .await,
            Err(CsvError::Cleanup)
        );
        assert_eq!(source.paths(), vec![path.clone()]);
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, b"owned repair").unwrap();
        source
            .cleanup(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
            .await
            .unwrap();
        assert!(source.paths().is_empty());
        assert!(!path.exists());
    }
    #[tokio::test]
    async fn cleanup_joins_abandoned_file_work_before_unlink() {
        let source = Artifacts::new(Ownership::default());
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_owned();
        source.files.lock().unwrap().snapshot = Some(file);
        let (started, ready) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let worker_source = source.clone();
        let receiver = source
            .owner
            .spawn_blocking(move || {
                let _files = worker_source.files.lock().unwrap();
                started.send(()).unwrap();
                wait.recv().unwrap();
            })
            .unwrap();
        tokio::task::spawn_blocking(move || ready.recv())
            .await
            .unwrap()
            .unwrap();
        drop(receiver);
        let cleaning = source.clone();
        let cleanup = tokio::spawn(async move {
            cleaning
                .cleanup(tokio::time::Instant::now() + std::time::Duration::from_secs(2))
                .await
        });
        tokio::task::yield_now().await;
        assert!(path.exists());
        assert!(!cleanup.is_finished());
        release.send(()).unwrap();
        cleanup.await.unwrap().unwrap();
        assert!(!path.exists());
    }
}
