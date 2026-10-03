//! A simulated Docker host scripts every response. No test here talks to a
//! real Docker daemon except the ignored live test at the end.
use super::runner::{
    classify, DockerCommand, DockerError, ManagedHost, LABEL_MANAGED, LABEL_PROFILE, LABEL_RECORD,
};
use super::service::{self, Busy, Scope};
use super::*;
use crate::backend::development::{Authority, EndpointCapability};
use crate::{credentials, storage, CredentialStorageMode};
use futures_util::future::BoxFuture;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

#[derive(Clone, Debug)]
struct Container {
    id: String,
    name: String,
    labels: BTreeMap<String, String>,
    state: String,
}

#[derive(Clone, Debug)]
struct Volume {
    name: String,
    labels: BTreeMap<String, String>,
}

#[derive(Default)]
struct World {
    available: Option<DockerError>,
    containers: Vec<Container>,
    volumes: Vec<Volume>,
    /// Verb (e.g. "run", "volume create") whose next call fails.
    fail: HashMap<&'static str, DockerError>,
    /// `run` creates the container but fails to start it.
    run_creates_then_fails: bool,
    ready: Option<String>,
    busy_ports: HashSet<u16>,
    log: Vec<DockerCommand>,
    ready_passwords: Vec<String>,
    next_id: u64,
    /// Exact-ID inspection reports another profile's labels.
    spoof_inspect: bool,
}

#[derive(Default)]
struct FakeHost(Mutex<World>);

fn flag_values(args: &[String], flag: &str) -> Vec<String> {
    args.windows(2)
        .filter(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
        .collect()
}

fn label_filters(args: &[String]) -> Vec<(String, String)> {
    flag_values(args, "--filter")
        .into_iter()
        .filter_map(|filter| {
            let label = filter.strip_prefix("label=")?;
            let (key, value) = label.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn labels_of(args: &[String]) -> BTreeMap<String, String> {
    flag_values(args, "--label")
        .into_iter()
        .filter_map(|label| {
            let (key, value) = label.split_once('=')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn matches(labels: &BTreeMap<String, String>, filters: &[(String, String)]) -> bool {
    filters
        .iter()
        .all(|(key, value)| labels.get(key) == Some(value))
}

fn exit(stderr: &str) -> DockerError {
    DockerError::Exit {
        stderr: stderr.into(),
    }
}

impl World {
    fn verb(args: &[String]) -> &'static str {
        match (
            args.first().map(String::as_str),
            args.get(1).map(String::as_str),
        ) {
            (Some("version"), _) => "version",
            (Some("ps"), _) => "ps",
            (Some("container"), Some("inspect")) => "inspect",
            (Some("run"), _) => "run",
            (Some("rm"), _) => "rm",
            (Some("start"), _) => "start",
            (Some("stop"), _) => "stop",
            (Some("volume"), Some("create")) => "volume create",
            (Some("volume"), Some("ls")) => "volume ls",
            (Some("volume"), Some("rm")) => "volume rm",
            _ => "unknown",
        }
    }

    fn handle(&mut self, command: &DockerCommand) -> Result<String, DockerError> {
        self.log.push(command.clone());
        let args = &command.args;
        let verb = Self::verb(args);
        if verb == "version" {
            if let Some(error) = self.available.clone() {
                return Err(error);
            }
            return Ok("27.1.0".into());
        }
        if let Some(error) = self.fail.remove(verb) {
            return Err(error);
        }
        match verb {
            "ps" => {
                let filters = label_filters(args);
                let format = flag_values(args, "--format").pop().unwrap_or_default();
                Ok(self
                    .containers
                    .iter()
                    .filter(|c| matches(&c.labels, &filters))
                    .map(|c| {
                        if format == "{{.ID}}" {
                            c.id.clone()
                        } else {
                            format!(
                                "{}\t{}\t{}",
                                c.id,
                                c.state,
                                c.labels.get(LABEL_RECORD).cloned().unwrap_or_default()
                            )
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "inspect" => {
                let id = args.last().unwrap();
                let c = self
                    .containers
                    .iter()
                    .find(|c| &c.id == id)
                    .ok_or_else(|| exit("Error: No such container"))?;
                let mut labels = c.labels.clone();
                if self.spoof_inspect {
                    labels.insert(LABEL_PROFILE.into(), "someone-else".into());
                }
                Ok(format!(
                    "{}\t{}\t{}",
                    c.id,
                    c.state,
                    serde_json::to_string(&labels).unwrap()
                ))
            }
            "run" => {
                let name = flag_values(args, "--name").pop().unwrap();
                if self.containers.iter().any(|c| c.name == name) {
                    return Err(exit("Conflict. The container name is already in use"));
                }
                self.next_id += 1;
                let id = format!("{:064x}", self.next_id + 0xabc000);
                let failed = std::mem::take(&mut self.run_creates_then_fails);
                self.containers.push(Container {
                    id: id.clone(),
                    name,
                    labels: labels_of(args),
                    state: if failed { "created" } else { "running" }.into(),
                });
                if failed {
                    return Err(exit("port is already allocated"));
                }
                Ok(format!("Pulling...\n{id}"))
            }
            "rm" => {
                let target = args.last().unwrap();
                let before = self.containers.len();
                self.containers.retain(|c| &c.id != target);
                if before == self.containers.len() {
                    return Err(exit("No such container"));
                }
                Ok(target.clone())
            }
            "start" | "stop" => {
                let target = args.last().unwrap();
                let c = self
                    .containers
                    .iter_mut()
                    .find(|c| &c.id == target)
                    .ok_or_else(|| exit("No such container"))?;
                c.state = if verb == "start" { "running" } else { "exited" }.into();
                Ok(target.clone())
            }
            "volume create" => {
                let name = args.last().unwrap().clone();
                if !self.volumes.iter().any(|v| v.name == name) {
                    self.volumes.push(Volume {
                        name: name.clone(),
                        labels: labels_of(args),
                    });
                }
                Ok(name)
            }
            "volume ls" => {
                let filters = label_filters(args);
                let name = flag_values(args, "--filter")
                    .into_iter()
                    .find_map(|f| f.strip_prefix("name=").map(str::to_owned));
                let quiet = args.iter().any(|arg| arg == "--quiet");
                Ok(self
                    .volumes
                    .iter()
                    .filter(|v| matches(&v.labels, &filters))
                    .filter(|v| name.as_ref().is_none_or(|name| v.name.contains(name)))
                    .map(|v| {
                        if quiet {
                            v.name.clone()
                        } else {
                            format!(
                                "{}\t{}",
                                v.name,
                                v.labels.get(LABEL_RECORD).cloned().unwrap_or_default()
                            )
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "volume rm" => {
                let name = args.last().unwrap();
                self.volumes.retain(|v| &v.name != name);
                Ok(name.clone())
            }
            _ => Err(exit("unsupported")),
        }
    }
}

impl ManagedHost for FakeHost {
    fn docker(&self, command: DockerCommand) -> BoxFuture<'_, Result<String, DockerError>> {
        let result = self.0.lock().unwrap().handle(&command);
        Box::pin(async move { result })
    }

    fn ready<'a>(
        &'a self,
        _port: u16,
        _database: &'a str,
        _user: &'a str,
        password: &'a str,
        _deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<(), String>> {
        let mut world = self.0.lock().unwrap();
        world.ready_passwords.push(password.to_owned());
        let result = match world.ready.take() {
            Some(error) => Err(error),
            None => Ok(()),
        };
        Box::pin(async move { result })
    }

    fn port_free(&self, port: u16) -> bool {
        !self.0.lock().unwrap().busy_ports.contains(&port)
    }
}

impl FakeHost {
    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.0.lock().unwrap()
    }

    fn mutating_targets(&self) -> Vec<String> {
        self.world()
            .log
            .iter()
            .filter(|c| matches!(World::verb(&c.args), "rm" | "start" | "stop"))
            .map(|c| c.args.last().unwrap().clone())
            .collect()
    }
}

struct Fixture {
    _directory: tempfile::TempDir,
    pool: sqlx::SqlitePool,
    credentials: Arc<credentials::Context>,
    authority: Authority,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let paths = storage::Paths::from_dir(directory.path().to_owned());
        let pool = storage::open_native_profile_pool(&paths).await.unwrap();
        credentials::set_credential_mode(&pool, CredentialStorageMode::PlainSqlite)
            .await
            .unwrap();
        credentials::mark_onboarding_completed(&pool).await.unwrap();
        let credentials = credentials::Context::development(pool.clone(), uuid::Uuid::new_v4());
        Self {
            _directory: directory,
            pool,
            credentials,
            authority: Authority {
                capability: EndpointCapability::GeneralPostgres,
                profile_id: uuid::Uuid::new_v4().to_string(),
            },
        }
    }

    fn scope(&self) -> Scope<'_> {
        Scope {
            profile_id: &self.authority.profile_id,
            pool: &self.pool,
            credentials: &self.credentials,
            authority: &self.authority,
        }
    }

    async fn provisioned(&self, host: &FakeHost) -> (ManagedProvisionPlan, ManagedProvisioned) {
        let plan = service::plan(&self.scope(), host, &request(None))
            .await
            .unwrap();
        let done = service::provision(&self.scope(), host, &plan, async { Ok(()) })
            .await
            .unwrap();
        (plan, done)
    }

    async fn password(&self, connection_id: &str) -> String {
        let mut connection = storage::read_connection_by_id(&self.pool, connection_id)
            .await
            .unwrap()
            .unwrap();
        credentials::hydrate(
            &self.credentials,
            CredentialStorageMode::PlainSqlite,
            &mut connection,
        )
        .await
        .unwrap();
        connection.password().to_owned()
    }

    async fn assert_nothing_left(&self, host: &FakeHost, plan: &ManagedProvisionPlan) {
        {
            let world = host.world();
            assert!(world.containers.is_empty(), "{:?}", world.containers);
            assert!(world.volumes.is_empty(), "{:?}", world.volumes);
        }
        assert!(
            storage::managed::read_managed_server_by_id(&self.pool, &plan.record_id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(storage::read_connections(&self.pool)
            .await
            .unwrap()
            .is_empty());
    }
}

fn request(port: Option<u16>) -> ManagedProvisionRequest {
    ManagedProvisionRequest {
        name: "Orders API".into(),
        version: "17".into(),
        port,
    }
}

fn foreign_container(host: &FakeHost, name: &str, labels: &[(&str, &str)]) -> String {
    let mut world = host.world();
    world.next_id += 1;
    let id = format!("{:064x}", world.next_id + 0xf00000);
    world.containers.push(Container {
        id: id.clone(),
        name: name.into(),
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
        state: "running".into(),
    });
    id
}

#[test]
fn availability_distinguishes_missing_cli_daemon_down_and_permission() {
    assert_eq!(
        classify(Err(DockerError::Missing)),
        DockerAvailability::CliMissing
    );
    assert_eq!(
        classify(Err(DockerError::TimedOut)),
        DockerAvailability::TimedOut
    );
    assert!(matches!(
        classify(Err(exit("Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?"))),
        DockerAvailability::DaemonUnavailable { .. }
    ));
    assert!(matches!(
        classify(Err(exit(
            "permission denied while trying to connect to the Docker daemon socket"
        ))),
        DockerAvailability::PermissionDenied { .. }
    ));
    assert!(matches!(
        classify(Err(exit("unexpected context error"))),
        DockerAvailability::Failed { .. }
    ));
    assert!(matches!(
        classify(Ok(String::new())),
        DockerAvailability::DaemonUnavailable { .. }
    ));
    assert_eq!(
        classify(Ok("27.1.0\n".into())),
        DockerAvailability::Available {
            server_version: "27.1.0".into()
        }
    );
}

#[test]
fn owned_fixture_profiles_refuse_managed_servers() {
    assert!(require_general(NativeProfileKind::OwnedFixtures).is_err());
    assert!(require_general(NativeProfileKind::GeneralPostgres).is_ok());
}

#[test]
fn concurrent_operations_on_one_record_are_refused() {
    let first = Busy::claim("record-a").unwrap();
    assert!(Busy::claim("record-a").is_err());
    let _other = Busy::claim("record-b").unwrap();
    drop(first);
    assert!(Busy::claim("record-a").is_ok());
}

#[tokio::test]
async fn plan_validates_version_and_port_and_binds_loopback_only() {
    let fixture = Fixture::new().await;
    let host = FakeHost::default();
    host.world().busy_ports.insert(5433);
    let plan = service::plan(&fixture.scope(), &host, &request(None))
        .await
        .unwrap();
    assert_eq!(plan.port, 5434, "occupied ports are skipped");
    assert_eq!(plan.host_binding, "127.0.0.1:5434");
    assert_eq!(plan.image, "postgres:17");
    assert!(plan.container_name.ends_with(&plan.record_id[..8]));
    assert!(plan.container_name.starts_with("dbunk-native-orders-api-"));
    assert_eq!(plan.database, "orders_api");
    for bad in [
        ManagedProvisionRequest {
            version: "9.6".into(),
            ..request(None)
        },
        ManagedProvisionRequest {
            name: "  ".into(),
            ..request(None)
        },
        request(Some(80)),
        request(Some(5433)),
    ] {
        assert!(service::plan(&fixture.scope(), &host, &bad).await.is_err());
    }
    assert!(host.world().log.is_empty(), "planning never calls Docker");
}

#[tokio::test]
async fn provision_creates_labelled_loopback_container_and_journaled_connection() {
    let fixture = Fixture::new().await;
    let host = FakeHost::default();
    let (plan, done) = fixture.provisioned(&host).await;
    let (run, container, volume, ready_password) = {
        let world = host.world();
        (
            world
                .log
                .iter()
                .find(|c| World::verb(&c.args) == "run")
                .unwrap()
                .clone(),
            world.containers[0].clone(),
            world.volumes[0].clone(),
            world.ready_passwords[0].clone(),
        )
    };
    for labels in [&container.labels, &volume.labels] {
        assert_eq!(labels[LABEL_MANAGED], "true");
        assert_eq!(labels[LABEL_PROFILE], fixture.authority.profile_id);
        assert_eq!(labels[LABEL_RECORD], plan.record_id);
    }
    assert_eq!(
        flag_values(&run.args, "--publish"),
        vec![format!("127.0.0.1:{}:5432", plan.port)]
    );
    assert_eq!(flag_values(&run.args, "--restart"), vec!["no".to_string()]);
    let password = fixture.password(&done.connection_id).await;
    assert_eq!(password.len(), 32);
    assert_eq!(password, ready_password);
    assert!(
        run.args.iter().all(|arg| !arg.contains(&password)),
        "the password never appears in argv"
    );
    assert!(!format!("{run:?}").contains(&password));
    assert_eq!(run.env, vec![("POSTGRES_PASSWORD".into(), password)]);
    let list = service::list(&fixture.scope(), &host).await.unwrap();
    let server = &list.servers[0];
    assert_eq!(server.id, plan.record_id);
    assert_eq!(server.status, ManagedContainerStatus::Running);
    assert_eq!(server.container_id.as_deref(), Some(container.id.as_str()));
    assert_eq!(server.volume_present, Some(true));
    assert_eq!(
        server.connection.as_ref().map(|c| c.id.as_str()),
        Some(done.connection_id.as_str())
    );
    // The reviewed plan cannot be replayed.
    assert!(
        service::provision(&fixture.scope(), &host, &plan, async { Ok(()) })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn provision_refuses_a_plan_that_differs_from_its_review() {
    let fixture = Fixture::new().await;
    let host = FakeHost::default();
    let plan = service::plan(&fixture.scope(), &host, &request(None))
        .await
        .unwrap();
    for tampered in [
        ManagedProvisionPlan {
            image: "evil:latest".into(),
            ..plan.clone()
        },
        ManagedProvisionPlan {
            host_binding: "0.0.0.0:5433".into(),
            ..plan.clone()
        },
        ManagedProvisionPlan {
            volume_name: "someone-elses-data".into(),
            ..plan.clone()
        },
    ] {
        let error = service::provision(&fixture.scope(), &host, &tampered, async { Ok(()) })
            .await
            .unwrap_err();
        assert!(error.contains("review"), "{error}");
    }
    assert!(host.world().log.is_empty());
}

#[tokio::test]
async fn unavailable_docker_and_foreign_volume_names_stop_provisioning_untouched() {
    let fixture = Fixture::new().await;
    let host = FakeHost::default();
    let plan = service::plan(&fixture.scope(), &host, &request(None))
        .await
        .unwrap();
    host.world().available = Some(exit("Cannot connect to the Docker daemon"));
    let error = service::provision(&fixture.scope(), &host, &plan, async { Ok(()) })
        .await
        .unwrap_err();
    assert!(error.contains("daemon"), "{error}");
    host.world().available = None;
    host.world().volumes.push(Volume {
        name: plan.volume_name.clone(),
        labels: BTreeMap::new(),
    });
    let error = service::provision(&fixture.scope(), &host, &plan, async { Ok(()) })
        .await
        .unwrap_err();
    assert!(error.contains("never adopted"), "{error}");
    let world = host.world();
    assert_eq!(world.volumes.len(), 1, "the foreign volume is kept");
    assert!(world.containers.is_empty());
    assert!(world
        .log
        .iter()
        .all(|c| !matches!(World::verb(&c.args), "volume create" | "run" | "volume rm")));
}

#[tokio::test]
async fn every_provisioning_failure_step_rolls_back_completely() {
    #[derive(Debug, Clone, Copy)]
    enum Step {
        VolumeCreate,
        Run,
        RunCreatedButNotStarted,
        Ready,
        Gate,
        ConnectionJournal,
    }
    for step in [
        Step::VolumeCreate,
        Step::Run,
        Step::RunCreatedButNotStarted,
        Step::Ready,
        Step::Gate,
        Step::ConnectionJournal,
    ] {
        let fixture = Fixture::new().await;
        let host = FakeHost::default();
        // Unrelated containers on the machine must survive any rollback.
        let bystander = foreign_container(&host, "unrelated-postgres", &[]);
        let plan = service::plan(&fixture.scope(), &host, &request(None))
            .await
            .unwrap();
        {
            let mut world = host.world();
            match step {
                Step::VolumeCreate => {
                    world.fail.insert("volume create", exit("disk full"));
                }
                Step::Run => {
                    world.fail.insert("run", exit("pull access denied"));
                }
                Step::RunCreatedButNotStarted => world.run_creates_then_fails = true,
                Step::Ready => world.ready = Some("never became ready".into()),
                _ => {}
            }
        }
        if matches!(step, Step::ConnectionJournal) {
            sqlx::query("CREATE TRIGGER reject_connection BEFORE INSERT ON connections BEGIN SELECT RAISE(ABORT, 'injected'); END")
                .execute(&fixture.pool)
                .await
                .unwrap();
        }
        let gate = async move {
            if matches!(step, Step::Gate) {
                Err("Native backend is closing".to_string())
            } else {
                Ok(())
            }
        };
        let error = service::provision(&fixture.scope(), &host, &plan, gate)
            .await
            .unwrap_err();
        assert!(error.contains("rolled back"), "{step:?}: {error}");
        host.world().containers.retain(|c| c.id != bystander);
        fixture.assert_nothing_left(&host, &plan).await;
        assert!(
            !host.mutating_targets().contains(&bystander),
            "{step:?} touched an unrelated container"
        );
    }
}

#[tokio::test]
async fn list_and_lifecycle_touch_only_this_profiles_labelled_container_by_id() {
    let fixture = Fixture::new().await;
    let host = FakeHost::default();
    let (plan, _) = fixture.provisioned(&host).await;
    let owned = host.world().containers[0].id.clone();
    // Same record label under another profile, and an unlabelled container
    // that reuses the expected name.
    let other_profile = uuid::Uuid::new_v4().to_string();
    let foreign = foreign_container(
        &host,
        "elsewhere",
        &[
            (LABEL_MANAGED, "true"),
            (LABEL_PROFILE, &other_profile),
            (LABEL_RECORD, &plan.record_id),
        ],
    );
    let list = service::list(&fixture.scope(), &host).await.unwrap();
    assert_eq!(list.servers.len(), 1);
    assert_eq!(
        list.servers[0].container_id.as_deref(),
        Some(owned.as_str())
    );
    service::stop(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap();
    assert!(matches!(
        service::list(&fixture.scope(), &host)
            .await
            .unwrap()
            .servers[0]
            .status,
        ManagedContainerStatus::Stopped { .. }
    ));
    service::start(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap();
    assert_eq!(host.mutating_targets(), vec![owned.clone(), owned.clone()]);
    // With the owned container gone, a same-named unlabelled container is
    // never adopted.
    host.world().containers.retain(|c| c.id != owned);
    let impostor = foreign_container(&host, &plan.container_name, &[]);
    let list = service::list(&fixture.scope(), &host).await.unwrap();
    assert_eq!(list.servers[0].status, ManagedContainerStatus::Missing);
    assert!(service::stop(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap_err()
        .contains("missing"));
    assert!(service::start(&fixture.scope(), &host, &plan.record_id)
        .await
        .is_err());
    let targets = host.mutating_targets();
    assert!(!targets.contains(&impostor) && !targets.contains(&foreign));
    assert!(targets.iter().all(|target| target == &owned));
}

#[tokio::test]
async fn a_listed_id_whose_labels_do_not_verify_is_refused() {
    let fixture = Fixture::new().await;
    let host = FakeHost::default();
    let (plan, _) = fixture.provisioned(&host).await;
    let id = host.world().containers[0].id.clone();
    host.world().spoof_inspect = true;
    for result in [
        service::stop(&fixture.scope(), &host, &plan.record_id).await,
        service::start(&fixture.scope(), &host, &plan.record_id).await,
    ] {
        assert!(result.unwrap_err().contains("not owned"));
    }
    let review = service::destroy_plan(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap();
    assert!(
        service::destroy(&fixture.scope(), &host, &plan.record_id, &review)
            .await
            .is_err()
    );
    assert!(!host.mutating_targets().contains(&id));
    assert_eq!(host.world().containers.len(), 1);
}

#[tokio::test]
async fn destroy_requires_matching_review_and_removes_exactly_what_was_reviewed() {
    let fixture = Fixture::new().await;
    let host = FakeHost::default();
    let (plan, done) = fixture.provisioned(&host).await;
    let bystander = foreign_container(&host, "unrelated", &[]);
    let review = service::destroy_plan(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap();
    assert!(review.container_id.is_some());
    assert_eq!(review.volume.as_deref(), Some(plan.volume_name.as_str()));
    assert_eq!(
        review.connection.as_ref().map(|c| c.id.as_str()),
        Some(done.connection_id.as_str())
    );
    // A stale or empty review changes nothing.
    let stale = ManagedDestroyPlan {
        volume: None,
        ..review.clone()
    };
    match service::destroy(&fixture.scope(), &host, &plan.record_id, &stale)
        .await
        .unwrap()
    {
        ManagedDestroy::ReviewRequired { plan: current } => assert_eq!(current, review),
        ManagedDestroy::Destroyed => panic!("destroyed without matching review"),
    }
    assert_eq!(host.world().containers.len(), 2);
    assert_eq!(host.world().volumes.len(), 1);
    assert_eq!(
        service::destroy(&fixture.scope(), &host, &plan.record_id, &review)
            .await
            .unwrap(),
        ManagedDestroy::Destroyed
    );
    {
        let world = host.world();
        assert_eq!(world.containers.len(), 1);
        assert_eq!(world.containers[0].id, bystander);
        assert!(world.volumes.is_empty());
    }
    assert!(
        storage::read_connection_by_id(&fixture.pool, &done.connection_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        credentials::read_all(&fixture.credentials, CredentialStorageMode::PlainSqlite)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(service::list(&fixture.scope(), &host)
        .await
        .unwrap()
        .servers
        .is_empty());
}

#[tokio::test]
async fn recreate_preserves_identity_and_reuses_the_owned_volume() {
    let fixture = Fixture::new().await;
    let host = FakeHost::default();
    let (plan, done) = fixture.provisioned(&host).await;
    let password = fixture.password(&done.connection_id).await;
    let original = host.world().containers[0].id.clone();
    assert!(service::recreate(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap_err()
        .contains("still exists"));
    host.world().containers.clear();
    service::recreate(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap();
    let run = {
        let world = host.world();
        assert_eq!(world.containers.len(), 1);
        let recreated = &world.containers[0];
        assert_ne!(recreated.id, original);
        assert_eq!(recreated.labels[LABEL_RECORD], plan.record_id);
        assert_eq!(recreated.name, plan.container_name);
        assert_eq!(world.volumes.len(), 1, "the surviving volume is reused");
        assert_eq!(world.ready_passwords.last(), Some(&password));
        world
            .log
            .iter()
            .rev()
            .find(|c| World::verb(&c.args) == "run")
            .unwrap()
            .clone()
    };
    assert_eq!(
        flag_values(&run.args, "--publish"),
        vec![format!("127.0.0.1:{}:5432", plan.port)]
    );
    let list = service::list(&fixture.scope(), &host).await.unwrap();
    assert_eq!(list.servers[0].id, plan.record_id);
    assert_eq!(
        list.servers[0].connection.as_ref().map(|c| c.id.clone()),
        Some(done.connection_id.clone())
    );
    assert_eq!(fixture.password(&done.connection_id).await, password);
}

#[tokio::test]
async fn failed_recreation_removes_only_what_it_created() {
    let fixture = Fixture::new().await;
    let host = FakeHost::default();
    let (plan, done) = fixture.provisioned(&host).await;
    // Volume survived: a failed attempt keeps it and removes the new container.
    host.world().containers.clear();
    host.world().ready = Some("never became ready".into());
    let error = service::recreate(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap_err();
    assert!(error.contains("rolled back"), "{error}");
    assert!(host.world().containers.is_empty());
    assert_eq!(host.world().volumes.len(), 1);
    // Volume also gone: the attempt's new volume is removed too.
    host.world().volumes.clear();
    host.world().fail.insert("run", exit("pull access denied"));
    service::recreate(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap_err();
    assert!(host.world().containers.is_empty());
    assert!(host.world().volumes.is_empty());
    // Identity is preserved across failures.
    let list = service::list(&fixture.scope(), &host).await.unwrap();
    assert_eq!(list.servers[0].id, plan.record_id);
    assert_eq!(list.servers[0].status, ManagedContainerStatus::Missing);
    assert!(!fixture.password(&done.connection_id).await.is_empty());
    // A same-named foreign volume is never adopted by recreation.
    host.world().volumes.push(Volume {
        name: plan.volume_name.clone(),
        labels: BTreeMap::new(),
    });
    assert!(service::recreate(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap_err()
        .contains("never adopted"));
    assert_eq!(host.world().volumes.len(), 1);
}

#[tokio::test]
async fn docker_outage_is_reported_without_guessing_status() {
    let fixture = Fixture::new().await;
    let host = FakeHost::default();
    let (plan, _) = fixture.provisioned(&host).await;
    host.world().available = Some(DockerError::Missing);
    let list = service::list(&fixture.scope(), &host).await.unwrap();
    assert_eq!(list.docker, DockerAvailability::CliMissing);
    assert_eq!(list.servers[0].status, ManagedContainerStatus::Unknown);
    assert_eq!(list.servers[0].volume_present, None);
    assert!(service::stop(&fixture.scope(), &host, &plan.record_id)
        .await
        .is_err());
}

/// Provisions and destroys one uniquely labelled container on the real
/// daemon. Run manually only:
/// `cargo test --no-default-features --features isolated-profile managed_servers_live -- --ignored`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a running Docker daemon; pulls postgres:17 if absent"]
async fn managed_servers_live_provision_and_destroy() {
    let fixture = Fixture::new().await;
    let host = super::runner::CliHost;
    assert!(service::availability(&host).await.is_available());
    let plan = service::plan(
        &fixture.scope(),
        &host,
        &ManagedProvisionRequest {
            name: format!("dbunk live {}", &uuid::Uuid::new_v4().to_string()[..8]),
            version: "17".into(),
            port: None,
        },
    )
    .await
    .unwrap();
    service::provision(&fixture.scope(), &host, &plan, async { Ok(()) })
        .await
        .unwrap();
    let list = service::list(&fixture.scope(), &host).await.unwrap();
    assert_eq!(list.servers[0].status, ManagedContainerStatus::Running);
    service::stop(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap();
    service::start(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap();
    let review = service::destroy_plan(&fixture.scope(), &host, &plan.record_id)
        .await
        .unwrap();
    assert_eq!(
        service::destroy(&fixture.scope(), &host, &plan.record_id, &review)
            .await
            .unwrap(),
        ManagedDestroy::Destroyed
    );
    assert!(service::list(&fixture.scope(), &host)
        .await
        .unwrap()
        .servers
        .is_empty());
}
