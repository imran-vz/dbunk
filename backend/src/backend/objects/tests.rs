//! No network calls: exercise admission and lifecycle with a held, owned read.
use super::*;
use crate::backend::{data::DataCloseOutcome, profile, QuerySessionError};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::oneshot;

async fn fixture() -> (tempfile::TempDir, Backend, DataDocument) {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let document = backend
        .open_data_document("window", "catalog", &backend.fixture().id)
        .await
        .unwrap();
    (directory, backend, document)
}

// Uses the actual facade task/admission/lease path with an owned stand-in for
// the driver. The join cannot complete until its explicit release arrives.
async fn held_read(
    backend: Backend,
    document: DataDocument,
    started: oneshot::Sender<()>,
    joined: Arc<AtomicBool>,
    release: oneshot::Receiver<()>,
) -> Result<(), DataError> {
    backend
        .object_read(&document, move |_, drivers, mut cancelled| async move {
            let driver_done = joined.clone();
            drivers.track_task(tokio::spawn(async move {
                let _ = release.await;
                driver_done.store(true, Ordering::SeqCst);
            }));
            started.send(()).unwrap();
            cancelled.changed().await.unwrap();
            drivers.drain().await;
            assert!(joined.load(Ordering::SeqCst));
            Err(CatalogError::Cancelled)
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_or_abandoned_waiter_keeps_document_until_driver_join() {
    let (_directory, backend, document) = fixture().await;
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let joined = Arc::new(AtomicBool::new(false));
    let waiter = tokio::spawn(held_read(
        backend.clone(),
        document.clone(),
        started,
        joined.clone(),
        released,
    ));
    ready.await.unwrap();
    waiter.abort();
    let _ = waiter.await;
    backend.cancel_data(&document).await.unwrap();
    let closer = backend.clone();
    let closing_document = document.clone();
    let mut closing =
        tokio::spawn(async move { closer.close_data_document(&closing_document).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut closing)
            .await
            .is_err()
    );
    assert!(!joined.load(Ordering::SeqCst));
    release.send(()).unwrap();
    assert_eq!(closing.await.unwrap().unwrap(), DataCloseOutcome::Closed);
    assert!(joined.load(Ordering::SeqCst));
    assert!(matches!(
        backend.load_object_catalog(&document).await,
        Err(DataError::Document(_))
    ));
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_retirement_cancels_reads_and_waits_before_replacement() {
    let (_directory, backend, document) = fixture().await;
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let joined = Arc::new(AtomicBool::new(false));
    let waiter = tokio::spawn(held_read(
        backend.clone(),
        document.clone(),
        started,
        joined.clone(),
        released,
    ));
    ready.await.unwrap();
    let owner = backend.clone();
    let inner = backend.0.clone();
    let connection = document.connection_id().to_owned();
    let mut retirement = tokio::spawn(async move {
        owner
            .development_call(move |state| async move {
                Ok(crate::backend::data::retire_data(&inner, &state, Some(&connection)).await)
            })
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut retirement)
            .await
            .is_err()
    );
    assert!(!joined.load(Ordering::SeqCst));
    release.send(()).unwrap();
    retirement.await.unwrap().unwrap().unwrap();
    assert!(matches!(waiter.await.unwrap(), Err(DataError::Document(_))));
    assert!(matches!(
        backend.load_object_catalog(&document).await,
        Err(DataError::Document(_))
    ));
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_and_unowned_endpoints_refuse_before_catalog_connect() {
    let (_directory, backend, document) = fixture().await;
    let other_directory = profile::directory();
    let other = Backend::open_fixture(&other_directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    assert!(matches!(
        other.load_object_catalog(&document).await,
        Err(DataError::Document(_))
    ));
    // Registering here is test-only: even a forged internal document cannot
    // bypass profile endpoint admission and reach hydration or socket startup.
    let unowned = backend
        .0
        .documents
        .register("window".into(), "bad".into(), "unowned-endpoint".into())
        .unwrap();
    assert!(matches!(
        backend.load_object_catalog(&unowned).await,
        Err(DataError::Unavailable(QuerySessionError::ConnectionLost))
    ));
    backend.close_data_document(&unowned).await.unwrap();
    backend.close_data_document(&document).await.unwrap();
    other.shutdown().await.unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_signals_reads_and_joins_abandoned_work() {
    let (_directory, backend, document) = fixture().await;
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let joined = Arc::new(AtomicBool::new(false));
    let waiter = tokio::spawn(held_read(
        backend.clone(),
        document,
        started,
        joined.clone(),
        released,
    ));
    ready.await.unwrap();
    waiter.abort();
    let _ = waiter.await;
    let owner = backend.clone();
    let mut shutdown = tokio::spawn(async move { owner.shutdown().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut shutdown)
            .await
            .is_err()
    );
    assert!(!joined.load(Ordering::SeqCst));
    release.send(()).unwrap();
    shutdown.await.unwrap().unwrap();
    assert!(joined.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_cancel_does_not_poison_next_read_on_same_document() {
    let (_directory, backend, document) = fixture().await;
    for _ in 0..2 {
        let (started, ready) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let joined = Arc::new(AtomicBool::new(false));
        let mut waiter = tokio::spawn(held_read(
            backend.clone(),
            document.clone(),
            started,
            joined,
            released,
        ));
        ready.await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut waiter)
            .await
            .is_err());
        backend.cancel_data(&document).await.unwrap();
        release.send(()).unwrap();
        assert!(matches!(
            waiter.await.unwrap(),
            Err(DataError::Catalog(CatalogError::Cancelled))
        ));
    }
    backend.close_data_document(&document).await.unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn description_refuses_malformed_or_stale_identity_before_socket_startup() {
    let (_directory, backend, document) = fixture().await;
    let reference = |kind| PgObjectRef {
        kind,
        schema: Some("public".into()),
        name: "object".into(),
        identity_args: None,
    };
    assert!(matches!(
        backend
            .describe_object(&document, reference(PgObjectKind::Function))
            .await,
        Err(DataError::Catalog(CatalogError::InvalidReference))
    ));
    backend.close_data_document(&document).await.unwrap();
    assert!(matches!(
        backend
            .describe_object(&document, reference(PgObjectKind::View))
            .await,
        Err(DataError::Document(_))
    ));
    for kind in [
        PgObjectKind::Table,
        PgObjectKind::ForeignTable,
        PgObjectKind::Type,
        PgObjectKind::Domain,
    ] {
        assert!(matches!(
            backend.describe_object(&document, reference(kind)).await,
            Err(DataError::Document(_))
        ));
    }
    let foreign = crate::backend::data_documents::Documents::default()
        .register("foreign".into(), "objects".into(), backend.fixture().id)
        .unwrap();
    assert!(matches!(
        backend
            .describe_object(&foreign, reference(PgObjectKind::View))
            .await,
        Err(DataError::Document(_))
    ));
    backend.shutdown().await.unwrap();
}
