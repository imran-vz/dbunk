use super::*;

use super::testing::*;

fn entries(value: &str) -> Credentials {
    HashMap::from([("same-connection-id".into(), value.into())])
}

#[test]
fn strict_missing_entry_is_empty_and_cached() {
    let io = Arc::new(RecordingStore::default());
    let store = store(&io, "missing", ReadPolicy::Strict);
    assert!(store.get_all().unwrap().is_empty());
    io.state.lock().unwrap().deny_read = true;
    assert!(store.get_all().unwrap().is_empty());
    assert_eq!(io.state.lock().unwrap().calls.len(), 1);
}

#[test]
fn strict_denial_and_corruption_preserve_the_entry_and_allow_retry() {
    let io = Arc::new(RecordingStore::default());
    let store = store(&io, "retry", ReadPolicy::Strict);
    let key = (
        store.identity.service.clone(),
        store.identity.account.clone(),
    );
    io.state.lock().unwrap().deny_read = true;
    let error = store.get_all().unwrap_err();
    assert!(error.contains("denied or locked"));
    assert!(!error.contains("synthetic private"));
    {
        let mut state = io.state.lock().unwrap();
        state.deny_read = false;
        state
            .blobs
            .insert(key.clone(), "{private corrupt contents".into());
    }
    let error = store.get_all().unwrap_err();
    assert!(error.contains("unreadable"));
    assert!(!error.contains("private corrupt"));
    {
        let mut state = io.state.lock().unwrap();
        assert_eq!(state.blobs.get(&key).unwrap(), "{private corrupt contents");
        state
            .blobs
            .insert(key, serde_json::to_string(&entries("recovered")).unwrap());
    }
    assert_eq!(store.get_all().unwrap(), entries("recovered"));
    assert!(io
        .state
        .lock()
        .unwrap()
        .calls
        .iter()
        .all(|(action, _, _)| *action == "read"));
}

#[test]
fn failed_write_or_delete_does_not_publish_new_cache_state() {
    let io = Arc::new(RecordingStore::default());
    let store = store(&io, "failed-save", ReadPolicy::Strict);
    store.replace_all(&entries("original")).unwrap();
    io.state.lock().unwrap().deny_write = true;
    assert!(store.replace_all(&entries("replacement")).is_err());
    assert_eq!(store.get_all().unwrap(), entries("original"));
    assert!(store.replace_all(&Credentials::new()).is_err());
    assert_eq!(store.get_all().unwrap(), entries("original"));
    io.state.lock().unwrap().deny_write = false;
    store.replace_all(&Credentials::new()).unwrap();
    assert!(store.get_all().unwrap().is_empty());
}

#[test]
fn independent_stores_scope_reads_writes_deletes_and_caches() {
    let io = Arc::new(RecordingStore::default());
    let first = store(&io, "first", ReadPolicy::Strict);
    let second = store(&io, "second", ReadPolicy::Strict);
    first.replace_all(&entries("first")).unwrap();
    second.replace_all(&entries("second")).unwrap();
    assert_eq!(first.get_all().unwrap(), entries("first"));
    assert_eq!(second.get_all().unwrap(), entries("second"));
    first.replace_all(&Credentials::new()).unwrap();
    let reopened = store(&io, "second", ReadPolicy::Strict);
    assert_eq!(reopened.get_all().unwrap(), entries("second"));
    let state = io.state.lock().unwrap();
    assert_eq!(state.blobs.len(), 1);
    assert!(state
        .calls
        .iter()
        .all(|(_, service, account)| { service != SERVICE && account != ACCOUNT }));
}

#[test]
fn legacy_read_policy_still_caches_failures_as_empty() {
    let io = Arc::new(RecordingStore::default());
    let store = store(&io, "legacy-policy", ReadPolicy::LegacyEmptyOnError);
    io.state.lock().unwrap().deny_read = true;
    assert!(store.get_all().unwrap().is_empty());
    io.state.lock().unwrap().deny_read = false;
    assert!(store.get_all().unwrap().is_empty());
    assert_eq!(io.state.lock().unwrap().calls.len(), 1);
    store.replace_all(&entries("explicitly-saved")).unwrap();
    assert_eq!(store.get_all().unwrap(), entries("explicitly-saved"));
}

#[tokio::test]
async fn async_adapters_run_os_io_off_the_runtime_thread_and_keep_errors() {
    struct ThreadProbe(Mutex<Vec<std::thread::ThreadId>>);
    impl BlobStore for ThreadProbe {
        fn read(&self, _: &Identity) -> Result<Option<String>, keyring::Error> {
            self.0.lock().unwrap().push(std::thread::current().id());
            Ok(None)
        }
        fn write(&self, _: &Identity, _: Option<&str>) -> Result<(), keyring::Error> {
            self.0.lock().unwrap().push(std::thread::current().id());
            Ok(())
        }
    }
    let probe = Arc::new(ThreadProbe(Mutex::new(Vec::new())));
    let probed = Arc::new(Store {
        identity: Identity {
            service: "probe".into(),
            account: "probe".into(),
        },
        policy: ReadPolicy::Strict,
        io: probe.clone(),
        cache: Mutex::new(None),
    });
    assert!(load(&probed).await.unwrap().is_empty());
    replace(&probed, entries("saved")).await.unwrap();
    assert_eq!(load(&probed).await.unwrap(), entries("saved"));
    let caller = std::thread::current().id();
    let threads = probe.0.lock().unwrap().clone();
    assert_eq!(threads.len(), 2, "cached read must not reach the OS");
    assert!(threads.iter().all(|thread| *thread != caller));

    let io = Arc::new(RecordingStore::default());
    let denied = Arc::new(store(&io, "async-denied", ReadPolicy::Strict));
    io.state.lock().unwrap().deny_read = true;
    assert!(load(&denied)
        .await
        .unwrap_err()
        .contains("denied or locked"));
    io.state.lock().unwrap().deny_write = true;
    assert!(replace(&denied, entries("refused")).await.is_err());
}
