use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rejected_close_admission_does_not_strand_a_retired_document() {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let document = backend
        .open_data_document("window", "tab", &backend.fixture().id)
        .await
        .unwrap();
    let busy = backend
        .0
        .admission
        .clone()
        .acquire_many_owned(16)
        .await
        .unwrap();
    assert!(matches!(
        backend.close_data_document(&document).await,
        Err(DataError::Unavailable(QuerySessionError::Timeout { operation }))
            if operation == "nativeAdmission"
    ));
    assert!(
        document.0.check_open().is_ok(),
        "a refused cleanup call must not retire a live document"
    );
    drop(busy);
    backend.cancel_data(&document).await.unwrap();
    assert_eq!(
        backend.close_data_document(&document).await.unwrap(),
        DataCloseOutcome::Closed
    );
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saturated_data_waiters_leave_control_admission_and_remain_owned_after_drop() {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let mut documents = Vec::new();
    let mut waiters = Vec::new();
    let (started, mut started_rx) = tokio::sync::mpsc::channel(8);
    for index in 0..8 {
        let document = backend
            .open_data_document("window", &index.to_string(), &backend.fixture().id)
            .await
            .unwrap();
        documents.push(document.clone());
        let owner = backend.clone();
        let started = started.clone();
        waiters.push(tokio::spawn(async move {
            owner
                .data_call(&document, move |_, _, admission| async move {
                    drop(admission);
                    started.send(()).await.unwrap();
                    std::future::pending::<Result<(), DataError>>().await
                })
                .await
        }));
    }
    for _ in 0..8 {
        tokio::time::timeout(Duration::from_secs(2), started_rx.recv())
            .await
            .unwrap()
            .unwrap();
    }
    assert!(matches!(
        backend
            .data_call(&documents[0], |_, _, _| async { Ok(()) })
            .await,
        Err(DataError::Unavailable(QuerySessionError::ConnectionClosing))
    ));
    tokio::time::timeout(Duration::from_secs(1), backend.cancel_data(&documents[0]))
        .await
        .unwrap()
        .unwrap();
    backend.set_layout(Layout::SideBySide).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), backend.retire_window("query-owner"))
        .await
        .unwrap()
        .unwrap();
    for waiter in waiters {
        waiter.abort();
        let _ = waiter.await;
    }
    assert_eq!(
        backend.0.data_admission.available_permits(),
        0,
        "dropping UI futures must not detach admitted work"
    );
    let now = tokio::time::Instant::now();
    backend
        .shutdown_with_deadlines(now, now + Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(backend.0.data_admission.available_permits(), 8);
    assert!(matches!(
        documents[0].0.check_open(),
        Err("Document is closed")
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_have_private_manager_tabs_and_retired_handles_cannot_access_preferences() {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let connection = backend.fixture().id;
    let first = backend
        .open_data_document("one", "tab", &connection)
        .await
        .unwrap();
    let peer = backend
        .open_data_document("two", "tab", &connection)
        .await
        .unwrap();
    assert_ne!(first.0.manager_tab, peer.0.manager_tab);
    let foreign = data_documents::Documents::default()
        .register("one".into(), "tab".into(), connection.clone())
        .unwrap();
    assert!(matches!(
        backend.cancel_data(&foreign).await,
        Err(DataError::Document(_))
    ));
    assert_eq!(
        backend.close_data_document(&first).await.unwrap(),
        DataCloseOutcome::Closed
    );
    let reopened = backend
        .open_data_document("one", "tab", &connection)
        .await
        .unwrap();
    assert_ne!(first.0.manager_tab, reopened.0.manager_tab);
    assert!(matches!(
        backend
            .load_table_preferences(&first, "public".into(), "users".into())
            .await,
        Err(DataError::Document(_))
    ));
    backend.cancel_data(&peer).await.unwrap();
    backend.cancel_data(&reopened).await.unwrap();
    backend.shutdown().await.unwrap();
}
