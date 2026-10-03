//! Optional native owners. Legacy calls use the default context unchanged.
use super::protocol::TransferError;
use crate::postgres::dedicated::DriverJoins;
use std::{sync::Arc, time::Duration};
#[derive(Clone, Default)]
pub(crate) struct IoContext(Arc<State>);
#[derive(Default)]
struct State {
    #[cfg(feature = "isolated-profile")]
    owner: Option<crate::postgres::backup::native::Ownership>,
    drivers: DriverJoins,
    complete: std::sync::atomic::AtomicBool,
    #[cfg(feature = "isolated-profile")]
    cleanup_gate: tokio::sync::Mutex<()>,
    #[cfg(feature = "isolated-profile")]
    partial: std::sync::Mutex<Option<Arc<crate::postgres::backup::native::archive::Archive>>>,
}
impl IoContext {
    #[cfg(feature = "isolated-profile")]
    pub(crate) fn new(
        owner: crate::postgres::backup::native::Ownership,
        drivers: DriverJoins,
    ) -> Self {
        Self(Arc::new(State {
            owner: Some(owner),
            drivers,
            partial: Default::default(),
            complete: Default::default(),
            cleanup_gate: Default::default(),
        }))
    }
    pub(crate) fn retains_cleanup(&self) -> bool {
        self.tracked() && !self.0.complete.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(crate) fn tracked(&self) -> bool {
        #[cfg(feature = "isolated-profile")]
        {
            self.0.owner.is_some()
        }
        #[cfg(not(feature = "isolated-profile"))]
        {
            false
        }
    }
    pub(crate) fn drivers(&self) -> Option<&DriverJoins> {
        self.tracked().then_some(&self.0.drivers)
    }
    pub(crate) async fn file_work<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, TransferError> + Send + 'static,
    ) -> Result<T, TransferError> {
        #[cfg(feature = "isolated-profile")]
        if let Some(owner) = &self.0.owner {
            return owner
                .spawn_blocking(work)
                .map_err(|_| TransferError::JobLimitReached)?
                .await
                .map_err(|_| worker_error())?
                .map_err(|_| worker_error())?;
        }
        tokio::task::spawn_blocking(work)
            .await
            .map_err(|_| worker_error())?
    }
    pub(crate) async fn close(&self, connection: crate::postgres::dedicated::DedicatedConnection) {
        if !self.tracked() {
            connection.close().await;
            return;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        if tokio::time::timeout_at(deadline, connection.close())
            .await
            .is_err()
        {
            self.0.drivers.abort_all();
        }
        if tokio::time::timeout_at(deadline, self.0.drivers.drain())
            .await
            .is_err()
        {
            self.0.drivers.abort_all();
            self.0.drivers.drain().await;
        }
    }
    #[cfg(feature = "isolated-profile")]
    pub(crate) async fn join_drivers(&self) {
        if self.tracked() {
            self.0.drivers.abort_all();
            self.0.drivers.drain().await;
        }
    }
    pub(super) fn retain_partial(&self, file: Arc<tempfile::NamedTempFile>) {
        #[cfg(feature = "isolated-profile")]
        if self.tracked() {
            *self.0.partial.lock().unwrap() = Some(Arc::new(
                crate::postgres::backup::native::archive::Archive::new(file),
            ));
            return;
        }
        drop(file);
    }
    pub(super) async fn publish_partial(
        &self,
        destination: std::path::PathBuf,
    ) -> Result<(), TransferError> {
        #[cfg(feature = "isolated-profile")]
        {
            let archive = self
                .0
                .partial
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(worker_error)?;
            return self
                .file_work(move || {
                    archive.publish(destination).map_err(|error| match error {
                        crate::postgres::backup::protocol::PgToolJobError::DestinationExists => {
                            TransferError::DestinationExists
                        }
                        _ => TransferError::Io {
                            operation: "publish".into(),
                            reason: "Unable to publish CSV file".into(),
                        },
                    })
                })
                .await;
        }
        #[cfg(not(feature = "isolated-profile"))]
        {
            let _ = destination;
            Err(worker_error())
        }
    }
    #[cfg(feature = "isolated-profile")]
    pub(crate) async fn cleanup(&self, deadline: tokio::time::Instant) -> Result<(), ()> {
        let _cleanup = tokio::time::timeout_at(deadline, self.0.cleanup_gate.lock())
            .await
            .map_err(|_| ())?;
        if self.0.complete.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        self.join_drivers().await;
        if let Some(owner) = &self.0.owner {
            owner.drain_until(deadline).await?;
        }
        let partial = self.0.partial.lock().unwrap().clone();
        if let Some(partial) = partial {
            self.file_work(move || partial.cleanup().map_err(|_| worker_error()))
                .await
                .map_err(|_| ())?;
            self.0.partial.lock().unwrap().take();
        }
        if let Some(owner) = &self.0.owner {
            owner.drain_until(deadline).await?;
        }
        self.0
            .complete
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }
}
fn worker_error() -> TransferError {
    TransferError::Io {
        operation: "fileWorker".into(),
        reason: "CSV filesystem owner did not complete".into(),
    }
}

#[cfg(all(test, feature = "isolated-profile"))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn publication_is_known_success_and_failed_private_cleanup_retains_owner() {
        let directory = tempfile::tempdir().unwrap();
        let file = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        let private = file.path().to_owned();
        std::fs::write(&private, "exact,\"日本語\"\n".as_bytes()).unwrap();
        let io = IoContext::new(Default::default(), Default::default());
        io.retain_partial(Arc::new(file));
        let destination = directory.path().join("result.csv");
        io.publish_partial(destination.clone()).await.unwrap();
        let archive = io.0.partial.lock().unwrap().clone().unwrap();
        archive
            .fail_cleanup
            .store(true, std::sync::atomic::Ordering::Release);
        assert!(io
            .cleanup(tokio::time::Instant::now() + Duration::from_secs(1))
            .await
            .is_err());
        assert!(io.retains_cleanup());
        assert!(private.exists());
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            "exact,\"日本語\"\n".as_bytes()
        );
        archive
            .fail_cleanup
            .store(false, std::sync::atomic::Ordering::Release);
        io.cleanup(tokio::time::Instant::now() + Duration::from_secs(1))
            .await
            .unwrap();
        assert!(!io.retains_cleanup());
        assert!(!private.exists());
        assert!(destination.exists());
    }
}
