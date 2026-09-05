use std::fs::Metadata;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use once_cell::sync::Lazy;
use tokio::sync::Semaphore;

use super::protocol::TransferError;

pub(super) const SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const FILE_WORKERS: usize = 4;
static FILE_WORKER_PERMITS: Lazy<Arc<Semaphore>> =
    Lazy::new(|| Arc::new(Semaphore::new(FILE_WORKERS)));

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileFingerprint {
    len: u64,
    modified: SystemTime,
    platform: PlatformFingerprint,
}

impl FileFingerprint {
    pub(super) fn len(&self) -> u64 {
        self.len
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct PlatformFingerprint {
    device: u64,
    inode: u64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct PlatformFingerprint {
    created: u64,
    last_write: u64,
}

#[cfg(not(any(unix, windows)))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct PlatformFingerprint;

pub(super) struct SourceFile {
    file: BoundedFile,
    pub fingerprint: FileFingerprint,
}

impl SourceFile {
    pub(super) async fn read(&self, limit: usize) -> Result<Vec<u8>, TransferError> {
        self.file.read(limit, "readSource").await
    }

    pub(super) async fn current_fingerprint(&self) -> Result<FileFingerprint, TransferError> {
        self.file.metadata("sourceMetadata").await
    }
}

/// Opens a source without following an already-present symlink and verifies
/// that the opened handle still describes the entry that was inspected.
pub(super) async fn open_source(path: &Path) -> Result<SourceFile, TransferError> {
    validate_absolute(path, "sourcePath")?;
    let path = path.to_owned();
    let (file, opened) = file_work(move || {
        let before = std::fs::symlink_metadata(&path)
            .map_err(|error| TransferError::io("sourceMetadata", &error))?;
        validate_source_metadata(&before)?;
        let before = fingerprint(&before)?;
        let file = source_open_options()
            .open(&path)
            .map_err(|error| TransferError::io("openSource", &error))?;
        let opened_metadata = file
            .metadata()
            .map_err(|error| TransferError::io("sourceMetadata", &error))?;
        validate_source_metadata(&opened_metadata)?;
        let opened = fingerprint(&opened_metadata)?;
        if before != opened {
            return Err(TransferError::SourceChanged);
        }
        Ok((file, opened))
    })
    .await?;
    Ok(SourceFile {
        file: BoundedFile::new(file),
        fingerprint: opened,
    })
}

fn source_open_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    configure_source_open(&mut options);
    options
}

#[cfg(target_os = "linux")]
fn configure_source_open(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    const O_NOFOLLOW: i32 = 0x20_000;
    const O_NONBLOCK: i32 = 0x800;

    // Avoid following a symlink or blocking forever on a FIFO swapped in
    // between the path metadata check and open.
    options.custom_flags(O_NOFOLLOW | O_NONBLOCK);
}

#[cfg(target_os = "macos")]
fn configure_source_open(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    const O_NOFOLLOW: i32 = 0x100;
    const O_NONBLOCK: i32 = 0x4;
    options.custom_flags(O_NOFOLLOW | O_NONBLOCK);
}

#[cfg(windows)]
fn configure_source_open(options: &mut std::fs::OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    // FILE_FLAG_OPEN_REPARSE_POINT keeps the opened object available for the
    // regular-file check instead of following a swapped-in link.
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn configure_source_open(_options: &mut std::fs::OpenOptions) {}

pub(super) async fn path_fingerprint(path: &Path) -> Result<FileFingerprint, TransferError> {
    let path = path.to_owned();
    file_work(move || {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| TransferError::io("sourceMetadata", &error))?;
        validate_source_metadata(&metadata)?;
        fingerprint(&metadata)
    })
    .await
}

pub(super) fn file_name(path: &Path) -> Result<String, TransferError> {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| TransferError::invalid("sourcePath", "A file name is required"))
}

pub(super) struct PartialFile {
    destination: PathBuf,
    temporary: tempfile::NamedTempFile,
    writer: Option<BoundedFile>,
}

impl PartialFile {
    pub(super) async fn create(destination: PathBuf, job_id: &str) -> Result<Self, TransferError> {
        validate_absolute(&destination, "destinationPath")?;
        let parent = destination
            .parent()
            .ok_or_else(|| {
                TransferError::invalid("destinationPath", "A parent directory is required")
            })?
            .to_owned();
        let name = destination
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| TransferError::invalid("destinationPath", "A file name is required"))?
            .to_string_lossy()
            .chars()
            .take(80)
            .collect::<String>();
        let prefix = format!(".{name}.dbunk-partial-{job_id}-");
        let destination_check = destination.clone();
        let (temporary, writer) = file_work(move || {
            let parent_metadata = std::fs::symlink_metadata(&parent)
                .map_err(|error| TransferError::io("destinationParent", &error))?;
            if !parent_metadata.is_dir() {
                return Err(TransferError::invalid(
                    "destinationPath",
                    "The parent must be an existing directory",
                ));
            }
            match std::fs::symlink_metadata(&destination_check) {
                Ok(_) => return Err(TransferError::DestinationExists),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(TransferError::io("destinationMetadata", &error)),
            }
            // tempfile uses create-new semantics and private permissions. It
            // remains the cleanup owner until publication succeeds.
            let temporary = tempfile::Builder::new()
                .prefix(&prefix)
                .tempfile_in(parent)
                .map_err(|error| TransferError::io("createPartial", &error))?;
            let writer = temporary
                .reopen()
                .map_err(|error| TransferError::io("openPartial", &error))?;
            Ok((temporary, writer))
        })
        .await?;
        Ok(Self {
            destination,
            temporary,
            writer: Some(BoundedFile::new(writer)),
        })
    }

    pub(super) async fn write_all(&self, bytes: &[u8]) -> Result<(), TransferError> {
        self.writer
            .as_ref()
            .expect("partial writer remains open")
            .write_all(bytes.to_vec(), "writePartial")
            .await
    }

    pub(super) async fn finish_writing(&mut self) -> Result<(), TransferError> {
        self.writer
            .as_ref()
            .expect("partial writer remains open")
            .sync_all("syncPartial")
            .await?;
        self.writer.take();
        Ok(())
    }

    /// Atomically publishes without replacing a destination created after
    /// validation. The blocking filesystem call remains owned by the bounded
    /// worker through completion after the job has claimed finalization.
    pub(super) async fn publish(self) -> Result<(), TransferError> {
        debug_assert!(self.writer.is_none());
        file_work(move || {
            self.temporary
                .persist_noclobber(self.destination)
                .map_err(|error| {
                    if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                        TransferError::DestinationExists
                    } else {
                        TransferError::io("publish", &error.error)
                    }
                })?;
            Ok(())
        })
        .await
    }
}

#[derive(Clone)]
struct BoundedFile(Arc<Mutex<std::fs::File>>);

impl BoundedFile {
    fn new(file: std::fs::File) -> Self {
        Self(Arc::new(Mutex::new(file)))
    }

    async fn read(&self, limit: usize, operation: &'static str) -> Result<Vec<u8>, TransferError> {
        let file = self.0.clone();
        file_work(move || {
            let mut bytes = vec![0; limit];
            let read = file
                .lock()
                .map_err(|_| file_worker_error(operation))?
                .read(&mut bytes)
                .map_err(|error| TransferError::io(operation, &error))?;
            bytes.truncate(read);
            Ok(bytes)
        })
        .await
    }

    async fn write_all(
        &self,
        bytes: Vec<u8>,
        operation: &'static str,
    ) -> Result<(), TransferError> {
        let file = self.0.clone();
        file_work(move || {
            file.lock()
                .map_err(|_| file_worker_error(operation))?
                .write_all(&bytes)
                .map_err(|error| TransferError::io(operation, &error))
        })
        .await
    }

    async fn sync_all(&self, operation: &'static str) -> Result<(), TransferError> {
        let file = self.0.clone();
        file_work(move || {
            file.lock()
                .map_err(|_| file_worker_error(operation))?
                .sync_all()
                .map_err(|error| TransferError::io(operation, &error))
        })
        .await
    }

    async fn metadata(&self, operation: &'static str) -> Result<FileFingerprint, TransferError> {
        let file = self.0.clone();
        file_work(move || {
            let metadata = file
                .lock()
                .map_err(|_| file_worker_error(operation))?
                .metadata()
                .map_err(|error| TransferError::io(operation, &error))?;
            validate_source_metadata(&metadata)?;
            fingerprint(&metadata)
        })
        .await
    }
}

/// A detached blocking syscall retains one of four permits until the syscall
/// actually returns. Cancellation can free its transfer job, but repeated work
/// cannot create an unbounded queue of abandoned filesystem threads.
async fn file_work<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, TransferError> + Send + 'static,
) -> Result<T, TransferError> {
    let permit = FILE_WORKER_PERMITS
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| file_worker_error("fileWorker"))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await
    .map_err(|_| file_worker_error("fileWorker"))?
}

fn file_worker_error(operation: &str) -> TransferError {
    TransferError::Io {
        operation: operation.into(),
        reason: "File worker stopped unexpectedly".into(),
    }
}

fn validate_absolute(path: &Path, field: &str) -> Result<(), TransferError> {
    if !path.is_absolute() {
        return Err(TransferError::invalid(
            field,
            "An absolute path is required",
        ));
    }
    if path.as_os_str().len() > 4_096 {
        return Err(TransferError::invalid(field, "The path is too long"));
    }
    Ok(())
}

fn validate_source_metadata(metadata: &Metadata) -> Result<(), TransferError> {
    if !metadata.file_type().is_file() {
        return Err(TransferError::invalid(
            "sourcePath",
            "A regular file is required",
        ));
    }
    if metadata.len() > SAFE_INTEGER {
        return Err(TransferError::invalid(
            "sourcePath",
            "The file is too large to report safely",
        ));
    }
    Ok(())
}

fn fingerprint(metadata: &Metadata) -> Result<FileFingerprint, TransferError> {
    let modified = metadata
        .modified()
        .map_err(|error| TransferError::io("sourceMetadata", &error))?;
    Ok(FileFingerprint {
        len: metadata.len(),
        modified,
        platform: platform_fingerprint(metadata),
    })
}

#[cfg(unix)]
fn platform_fingerprint(metadata: &Metadata) -> PlatformFingerprint {
    use std::os::unix::fs::MetadataExt;

    PlatformFingerprint {
        device: metadata.dev(),
        inode: metadata.ino(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    }
}

#[cfg(windows)]
fn platform_fingerprint(metadata: &Metadata) -> PlatformFingerprint {
    use std::os::windows::fs::MetadataExt;

    PlatformFingerprint {
        created: metadata.creation_time(),
        last_write: metadata.last_write_time(),
    }
}

#[cfg(not(any(unix, windows)))]
fn platform_fingerprint(_metadata: &Metadata) -> PlatformFingerprint {
    PlatformFingerprint
}
