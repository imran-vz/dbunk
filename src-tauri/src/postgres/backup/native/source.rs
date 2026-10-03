use std::{
    fs::{Metadata, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
#[derive(Clone, Default)]
pub(crate) struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
pub(crate) struct Snapshot {
    pub(crate) file: tempfile::NamedTempFile,
    pub(crate) bytes: u64,
}
impl Snapshot {
    /// Explicit unlink result before the final handle is dropped. On failure the
    /// caller retains this owner and its path; Drop is never cleanup evidence.
    pub(crate) fn remove(&self) -> std::io::Result<()> {
        std::fs::remove_file(self.file.path())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    Io,
    Limit,
    Changed,
    Cancelled,
    Cleanup,
}
pub(crate) struct Failure {
    pub(crate) error: Error,
    pub(crate) partial: Option<Snapshot>,
}
impl std::fmt::Debug for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapshotFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self {
            error,
            partial: None,
        }
    }
}
fn failed_copy(target: tempfile::NamedTempFile, error: Error) -> Failure {
    let partial = Snapshot {
        file: target,
        bytes: 0,
    };
    if partial.remove().is_ok() {
        error.into()
    } else {
        Failure {
            error: Error::Cleanup,
            partial: Some(partial),
        }
    }
}
fn same(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.len() == b.len()
            && a.mtime() == b.mtime()
            && a.mtime_nsec() == b.mtime_nsec()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        a.len() == b.len()
            && a.modified().ok() == b.modified().ok()
            && a.created().ok() == b.created().ok()
    }
}
/// Copies only the initially inspected length into an app-owned private file.
/// Changes detectable in handle/path metadata or exact EOF refuse the snapshot.
/// The snapshot, not the selected pathname, is authoritative after this returns.
pub(crate) fn copy(path: &Path, cancel: &Cancellation) -> Result<Snapshot, Failure> {
    copy_observed(path, cancel, |_| {})
}
// The observer is an allocation-free test seam after a completed copy chunk.
// Production monomorphizes it to a no-op; file ownership and EOF checks are shared.
pub(crate) fn copy_observed(
    path: &Path,
    cancel: &Cancellation,
    mut copied: impl FnMut(u64),
) -> Result<Snapshot, Failure> {
    let mut target = tempfile::Builder::new()
        .prefix("dbunk-native-restore-")
        .tempfile()
        .map_err(|_| Error::Io)?;
    match copy_into_bounded(path, cancel, &mut target, u64::MAX, &mut copied) {
        Ok(bytes) => Ok(Snapshot {
            file: target,
            bytes,
        }),
        Err(error) => Err(failed_copy(target, error)),
    }
}

/// The caller registers and retains the private target before entering this
/// function, including on errors. The opened source handle is checked before
/// copying and the byte limit is enforced before reading its contents.
pub(crate) fn copy_into_bounded(
    path: &Path,
    cancel: &Cancellation,
    target: &mut tempfile::NamedTempFile,
    maximum: u64,
    mut copied: impl FnMut(u64),
) -> Result<u64, Error> {
    if cancel.cancelled() {
        return Err(Error::Cancelled);
    }
    let initial = std::fs::symlink_metadata(path).map_err(|_| Error::Io)?;
    if !initial.is_file() || initial.len() == 0 {
        return Err(Error::Changed);
    }
    if initial.len() > maximum {
        return Err(Error::Limit);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut source = options.open(path).map_err(|_| Error::Io)?;
    let opened = source.metadata().map_err(|_| Error::Io)?;
    if !opened.is_file() || !same(&initial, &opened) {
        return Err(Error::Changed);
    }
    let mut remaining = initial.len();
    let mut buffer = [0_u8; 64 * 1024];
    while remaining > 0 {
        if cancel.cancelled() {
            return Err(Error::Cancelled);
        }
        let cap = remaining.min(buffer.len() as u64) as usize;
        let read = source.read(&mut buffer[..cap]).map_err(|_| Error::Io)?;
        if read == 0 {
            return Err(Error::Changed);
        }
        target.write_all(&buffer[..read]).map_err(|_| Error::Io)?;
        remaining -= read as u64;
        copied(initial.len() - remaining);
    }
    if source.read(&mut buffer[..1]).map_err(|_| Error::Io)? != 0 {
        return Err(Error::Changed);
    }
    let after = source.metadata().map_err(|_| Error::Io)?;
    let selected = std::fs::symlink_metadata(path).map_err(|_| Error::Changed)?;
    if !selected.is_file() || !same(&initial, &after) || !same(&initial, &selected) {
        return Err(Error::Changed);
    }
    if cancel.cancelled() {
        return Err(Error::Cancelled);
    }
    target.as_file().sync_all().map_err(|_| Error::Io)?;
    Ok(initial.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_copy_unlink_keeps_exact_partial_owner() {
        let target = tempfile::NamedTempFile::new().unwrap();
        let path = target.path().to_owned();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let failure = failed_copy(target, Error::Cancelled);
        assert_eq!(failure.error, Error::Cleanup);
        let partial = failure
            .partial
            .expect("failed unlink must retain the path owner");
        assert_eq!(partial.file.path(), path);
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, b"owned obstruction repair").unwrap();
        partial.remove().unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn bounded_copy_refuses_before_writing_and_preserves_registered_target() {
        let mut input = tempfile::NamedTempFile::new().unwrap();
        input.write_all(b"12345").unwrap();
        let mut target = tempfile::NamedTempFile::new().unwrap();
        let result = copy_into_bounded(
            input.path(),
            &Cancellation::default(),
            &mut target,
            4,
            |_| panic!("oversized input must not be copied"),
        );
        assert_eq!(result, Err(Error::Limit));
        assert_eq!(target.as_file().metadata().unwrap().len(), 0);
        assert!(
            target.path().exists(),
            "caller retains the registered target for explicit cleanup"
        );
        std::fs::remove_file(target.path()).unwrap();
    }
}
