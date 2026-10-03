use super::*;
use std::path::PathBuf;

struct Profile {
    path: PathBuf,
    marker: Vec<u8>,
}
impl Drop for Profile {
    fn drop(&mut self) {
        if std::fs::read(self.path.join(".dbunk-native-stage03"))
            .ok()
            .as_ref()
            == Some(&self.marker)
        {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
}
async fn backend() -> (Profile, Backend) {
    let id = uuid::Uuid::new_v4();
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("dbunk-comparison-lane-{id}"));
    std::fs::create_dir(&path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let marker = serde_json::to_vec(&serde_json::json!({"version":1,"fixture":"dbunk-native-stage03","host":"127.0.0.1","port":15432,"database":"dbunk_demo","profile_id":id})).unwrap();
    std::fs::write(path.join(".dbunk-native-stage03"), &marker).unwrap();
    let backend = Backend::open_fixture(&path).await.unwrap();
    (Profile { path, marker }, backend)
}
async fn join(runtime: &CompareRuntime) {
    let now = Instant::now();
    runtime
        .join(
            now + std::time::Duration::from_secs(1),
            now + std::time::Duration::from_secs(2),
        )
        .await
        .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn response_budget_refuses_before_job_admission_and_stop_destroys_queued_delivery() {
    let (_profile, backend) = backend().await;
    let budget = ByteBudget::new(8192);
    let runtime = CompareRuntime::new(
        backend.clone(),
        tokio::runtime::Handle::current(),
        budget.clone(),
        uuid::Uuid::new_v4().to_string(),
    );
    let (wake, awakened) = async_channel::bounded(1);
    let (controls, replies) = runtime.open(None, wake).unwrap();
    let endpoint = Endpoint {
        connection_id: "native-stage03-fixture".into(),
        schema: "public".into(),
    };
    controls
        .send(CompareCommand::Start(
            1,
            SchemaComparisonStart::new(endpoint.clone(), endpoint).unwrap(),
        ))
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), awakened.recv())
        .await
        .unwrap()
        .unwrap();
    let delivery = replies.try_recv().unwrap();
    assert!(matches!(
        delivery.result,
        Err(CompareFailure::DeliveryBudget)
    ));
    assert!(backend.list_schema_comparisons().unwrap().jobs.is_empty());
    drop(delivery);
    runtime.stop();
    join(&runtime).await;
    assert_eq!(budget.used(), 0);
    assert!(controls.send(CompareCommand::List(2)).is_err());
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn receiver_drop_joins_observer_and_reclaims_unconsumed_reply_allowance() {
    let (_profile, backend) = backend().await;
    let budget = ByteBudget::new(1024 * 1024);
    let runtime = CompareRuntime::new(
        backend.clone(),
        tokio::runtime::Handle::current(),
        budget.clone(),
        uuid::Uuid::new_v4().to_string(),
    );
    let (wake, awakened) = async_channel::bounded(1);
    let (controls, replies) = runtime.open(None, wake).unwrap();
    controls.send(CompareCommand::List(1)).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), awakened.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(!replies.is_empty());
    assert!(budget.used() >= 128 * 1024);
    drop(replies);
    join(&runtime).await;
    assert_eq!(budget.used(), 0);
    assert!(controls.send(CompareCommand::List(2)).is_err());
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_preserves_reader_lane_until_actual_tab_disposal() {
    let (_profile, backend) = backend().await;
    let host = crate::controller::Host::new_workspace(
        backend,
        tokio::runtime::Handle::current(),
        uuid::Uuid::new_v4().to_string(),
    );
    let tab = uuid::Uuid::new_v4().to_string();
    let (wake, _awakened) = async_channel::bounded(1);
    let (_controls, replies) = host
        .open_comparison_reader(tab.clone(), wake.clone())
        .unwrap();
    host.disconnect_document(&tab).await.unwrap();
    assert!(!replies.is_closed());
    assert!(
        host.open_comparison_reader(tab.clone(), wake.clone())
            .is_err()
    );
    host.close_document(&tab).await.unwrap();
    assert!(replies.is_closed());
    let (_controls, _replies) = host.open_comparison_reader(tab, wake).unwrap();
    host.shutdown().await.unwrap();
}
