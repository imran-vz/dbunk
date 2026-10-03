//! Native backup partial ownership survives runner errors and publication refusal.
use crate::postgres::backup::protocol::PgToolJobError;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

struct State {
    file: Option<Arc<tempfile::NamedTempFile>>,
    published: bool,
}
pub(crate) struct Archive {
    state: Mutex<State>,
    #[cfg(test)]
    pub(crate) fail_cleanup: std::sync::atomic::AtomicBool,
}
impl Archive {
    pub(crate) fn new(file: Arc<tempfile::NamedTempFile>) -> Self {
        Self {
            state: Mutex::new(State {
                file: Some(file),
                published: false,
            }),
            #[cfg(test)]
            fail_cleanup: std::sync::atomic::AtomicBool::new(false),
        }
    }
    pub(crate) fn publish(&self, destination: PathBuf) -> Result<(), PgToolJobError> {
        let mut owner = self.state.lock().unwrap();
        if owner.published {
            return Err(PgToolJobError::invalid("job", "Archive already published"));
        }
        let file = owner
            .file
            .as_ref()
            .ok_or_else(|| PgToolJobError::invalid("job", "Archive ownership missing"))?;
        if Arc::strong_count(file) != 1 {
            return Err(PgToolJobError::invalid("job", "Archive still in use"));
        }
        // The partial is created in the destination directory. One hard link
        // publishes complete bytes without overwriting; private-name removal is
        // separately checked cleanup, never tempfile's silent unlink fallback.
        std::fs::hard_link(file.path(), destination).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                PgToolJobError::DestinationExists
            } else {
                PgToolJobError::io("publish", &error)
            }
        })?;
        owner.published = true;
        Ok(())
    }
    pub(crate) fn cleanup(&self) -> Result<(), ()> {
        #[cfg(test)]
        if self.fail_cleanup.load(std::sync::atomic::Ordering::Acquire) {
            return Err(());
        }
        let mut owner = self.state.lock().unwrap();
        if let Some(file) = owner.file.as_ref() {
            if Arc::strong_count(file) != 1 {
                return Err(());
            }
            // Only the private name, never the published destination.
            std::fs::remove_file(file.path()).map_err(|_| ())?;
        }
        owner.file.take();
        Ok(())
    }
}
