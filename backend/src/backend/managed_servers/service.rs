//! Host-injected Managed Server operations. Ownership is proved by labels on
//! the exact container ID or volume name; a name alone is never trusted.
use super::runner::{
    self, bounded, classify, is_container_id, owned_labels, parse_labels, DockerCommand,
    ManagedHost, LABEL_MANAGED, LABEL_PROFILE, LABEL_RECORD, LIFECYCLE, PULL_AND_RUN, QUICK,
};
use super::*;
use crate::backend::development::Authority;
use crate::credentials::native_connections::{self, Change};
use crate::{credentials, storage, DatabaseEngine, ManagedServer, StoredConnection};
use sqlx::SqlitePool;
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

const PORT_BASE: u16 = 5433;
const PORT_SPAN: u16 = 500;
const MAX_NAME: usize = 128;
pub(super) const PROVISION_READY: Duration = Duration::from_secs(120);
pub(super) const START_READY: Duration = Duration::from_secs(60);

pub(super) struct Scope<'a> {
    pub profile_id: &'a str,
    pub pool: &'a SqlitePool,
    pub credentials: &'a credentials::Context,
    pub authority: &'a Authority,
}

// --- Per-record exclusion -------------------------------------------------

static BUSY: std::sync::Mutex<BTreeSet<String>> = std::sync::Mutex::new(BTreeSet::new());

/// Concurrent operations on one record are refused, not queued: a second
/// recreate must never clean up the first one's container.
pub(super) struct Busy(String);

impl Busy {
    pub(super) fn claim(id: &str) -> Result<Self, String> {
        let mut busy = BUSY.lock().unwrap();
        if !busy.insert(id.to_owned()) {
            return Err("Another operation on this managed server is still running".into());
        }
        Ok(Self(id.to_owned()))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.lock().unwrap().remove(&self.0);
    }
}

// --- Naming and validation -----------------------------------------------

fn container_slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let mut slug = slug.trim_matches('-').to_owned();
    slug.truncate(24);
    let slug = slug.trim_end_matches('-').to_owned();
    if slug.is_empty() {
        "postgres".into()
    } else {
        slug
    }
}

fn identifier(name: &str) -> String {
    let mut ident: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    ident = ident.trim_matches('_').to_owned();
    if ident.is_empty() {
        return "app".into();
    }
    if ident.starts_with(|c: char| c.is_ascii_digit()) {
        ident.insert(0, '_');
    }
    ident.truncate(48);
    ident
}

fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Managed server name is required".into());
    }
    if name.len() > MAX_NAME || name.chars().any(char::is_control) {
        return Err("Managed server name is too long or contains control characters".into());
    }
    Ok(name.to_owned())
}

fn validate_version(version: &str) -> Result<(), String> {
    if !MANAGED_POSTGRES_VERSIONS.contains(&version) {
        return Err(format!("Unsupported PostgreSQL version '{version}'"));
    }
    Ok(())
}

fn validate_port(port: u16) -> Result<(), String> {
    if port < 1024 {
        return Err("Managed server port must be between 1024 and 65535".into());
    }
    Ok(())
}

/// The image changed its data directory layout at 18.
fn data_mount(version: &str) -> &'static str {
    if version == "18" {
        "/var/lib/postgresql"
    } else {
        "/var/lib/postgresql/data"
    }
}

/// Every reviewed value derives from the record ID and request, so a plan
/// submitted for provisioning can be recomputed and compared exactly.
pub(super) fn derive_plan(
    record_id: &str,
    name: &str,
    version: &str,
    port: u16,
) -> Result<ManagedProvisionPlan, String> {
    let name = validate_name(name)?;
    validate_version(version)?;
    validate_port(port)?;
    crate::backend::development::canonical_uuid(record_id)
        .map_err(|_| "Invalid managed server identity".to_string())?;
    let short = &record_id[..8];
    let slug = container_slug(&name);
    let ident = identifier(&name);
    Ok(ManagedProvisionPlan {
        record_id: record_id.to_owned(),
        image: format!("postgres:{version}"),
        version: version.to_owned(),
        host_binding: format!("127.0.0.1:{port}"),
        port,
        container_name: format!("dbunk-native-{slug}-{short}"),
        volume_name: format!("dbunk-native-{slug}-{short}-data"),
        database: ident.clone(),
        user: ident,
        connection_name: name.clone(),
        name,
    })
}

fn generate_password() -> String {
    use rand::{distributions::Alphanumeric, Rng};
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

fn label_args(profile: &str, record: &str) -> Vec<String> {
    vec![
        "--label".into(),
        format!("{LABEL_MANAGED}=true"),
        "--label".into(),
        format!("{LABEL_PROFILE}={profile}"),
        "--label".into(),
        format!("{LABEL_RECORD}={record}"),
    ]
}

fn owner_filters(profile: &str, record: Option<&str>) -> Vec<String> {
    let mut filters = vec![
        "--filter".into(),
        format!("label={LABEL_PROFILE}={profile}"),
    ];
    if let Some(record) = record {
        filters.push("--filter".into());
        filters.push(format!("label={LABEL_RECORD}={record}"));
    }
    filters
}

// --- Docker primitives -----------------------------------------------------

async fn docker(
    host: &dyn ManagedHost,
    command: DockerCommand,
    action: &str,
) -> Result<String, String> {
    host.docker(command)
        .await
        .map_err(|error| error.message(action))
}

pub(super) async fn availability(host: &dyn ManagedHost) -> DockerAvailability {
    classify(host.docker(runner::version_command()).await)
}

async fn require_docker(host: &dyn ManagedHost) -> Result<(), String> {
    let status = availability(host).await;
    if !status.is_available() {
        return Err(status.describe());
    }
    Ok(())
}

/// Full IDs of containers carrying this profile's and record's labels.
async fn owned_containers(
    host: &dyn ManagedHost,
    profile: &str,
    record: &str,
) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = vec!["ps".into(), "--all".into(), "--no-trunc".into()];
    args.extend(owner_filters(profile, Some(record)));
    args.extend(["--format".into(), "{{.ID}}".into()]);
    let output = docker(host, DockerCommand::new(args, QUICK), "container listing").await?;
    let ids = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if ids.iter().any(|id| !is_container_id(id)) {
        return Err("Docker returned an unexpected container listing".into());
    }
    Ok(ids)
}

/// Inspect one exact ID and prove its labels. Returns its state.
async fn verify_container(
    host: &dyn ManagedHost,
    profile: &str,
    record: &str,
    id: &str,
) -> Result<String, String> {
    let output = docker(
        host,
        DockerCommand::new(
            [
                "container",
                "inspect",
                "--format",
                "{{.Id}}\t{{.State.Status}}\t{{json .Config.Labels}}",
                id,
            ],
            QUICK,
        ),
        "inspect",
    )
    .await?;
    let mut parts = output.splitn(3, '\t');
    let (Some(found), Some(state), Some(labels)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err("Docker returned an unexpected container description".into());
    };
    let labels = parse_labels(labels).unwrap_or_default();
    if found != id || !owned_labels(&labels, profile, record) {
        return Err("Container is not owned by this managed server; it was not touched".into());
    }
    Ok(state.trim().to_owned())
}

/// Zero or one owned container; more than one is an explicit conflict.
async fn resolve(
    host: &dyn ManagedHost,
    profile: &str,
    record: &str,
) -> Result<Option<(String, String)>, String> {
    let ids = owned_containers(host, profile, record).await?;
    match ids.as_slice() {
        [] => Ok(None),
        [id] => {
            let state = verify_container(host, profile, record, id).await?;
            Ok(Some((id.clone(), state)))
        }
        _ => Err(
            "More than one container carries this managed server's labels; resolve them in Docker first"
                .into(),
        ),
    }
}

async fn volume_names(host: &dyn ManagedHost, filters: Vec<String>) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = vec!["volume".into(), "ls".into(), "--quiet".into()];
    args.extend(filters);
    let output = docker(host, DockerCommand::new(args, QUICK), "volume listing").await?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

async fn owned_volume(
    host: &dyn ManagedHost,
    profile: &str,
    record: &str,
    name: &str,
) -> Result<bool, String> {
    Ok(volume_names(host, owner_filters(profile, Some(record)))
        .await?
        .iter()
        .any(|volume| volume == name))
}

async fn volume_name_taken(host: &dyn ManagedHost, name: &str) -> Result<bool, String> {
    Ok(
        volume_names(host, vec!["--filter".into(), format!("name={name}")])
            .await?
            .iter()
            .any(|volume| volume == name),
    )
}

async fn create_volume(
    host: &dyn ManagedHost,
    profile: &str,
    record: &str,
    name: &str,
) -> Result<(), String> {
    let mut args: Vec<String> = vec!["volume".into(), "create".into()];
    args.extend(label_args(profile, record));
    args.push(name.into());
    docker(host, DockerCommand::new(args, QUICK), "volume create").await?;
    // `volume create` succeeds on an existing name; prove this one is ours.
    if !owned_volume(host, profile, record, name).await? {
        return Err(format!(
            "Docker volume {name} is not owned by this managed server; it was not adopted"
        ));
    }
    Ok(())
}

struct RunSpec<'a> {
    record_id: &'a str,
    container_name: &'a str,
    version: &'a str,
    port: u16,
    volume_name: &'a str,
    database: &'a str,
    user: &'a str,
    password: &'a str,
}

/// Creates and starts a new container; returns its verified full ID.
async fn run_container(
    host: &dyn ManagedHost,
    profile: &str,
    spec: &RunSpec<'_>,
) -> Result<String, String> {
    let mut args: Vec<String> = vec![
        "run".into(),
        "--detach".into(),
        "--restart".into(),
        "no".into(),
        "--name".into(),
        spec.container_name.into(),
    ];
    args.extend(label_args(profile, spec.record_id));
    args.extend([
        "--publish".into(),
        format!("127.0.0.1:{}:5432", spec.port),
        "--volume".into(),
        format!("{}:{}", spec.volume_name, data_mount(spec.version)),
        "--env".into(),
        format!("POSTGRES_DB={}", spec.database),
        "--env".into(),
        format!("POSTGRES_USER={}", spec.user),
        "--env".into(),
        "POSTGRES_PASSWORD".into(),
        format!("postgres:{}", spec.version),
    ]);
    let command = DockerCommand {
        args,
        env: vec![("POSTGRES_PASSWORD".into(), spec.password.into())],
        timeout: PULL_AND_RUN,
    };
    let output = docker(host, command, "run").await?;
    let id = output.lines().last().unwrap_or_default().trim().to_owned();
    if !is_container_id(&id) {
        return Err("Docker did not report the new container ID".into());
    }
    verify_container(host, profile, spec.record_id, &id).await?;
    Ok(id)
}

/// Removes every container carrying the record's labels, each re-verified by
/// exact ID. Unlabelled containers with the same name are never touched.
async fn remove_owned_containers(
    host: &dyn ManagedHost,
    profile: &str,
    record: &str,
) -> Result<(), String> {
    for id in owned_containers(host, profile, record).await? {
        verify_container(host, profile, record, &id).await?;
        docker(
            host,
            DockerCommand::new(["rm", "--force", id.as_str()], LIFECYCLE),
            "remove",
        )
        .await?;
    }
    Ok(())
}

async fn remove_owned_volume(
    host: &dyn ManagedHost,
    profile: &str,
    record: &str,
    name: &str,
) -> Result<(), String> {
    if owned_volume(host, profile, record, name).await? {
        docker(
            host,
            DockerCommand::new(["volume", "rm", name], LIFECYCLE),
            "volume remove",
        )
        .await?;
    }
    Ok(())
}

// --- Records ---------------------------------------------------------------

async fn record(scope: &Scope<'_>, id: &str) -> Result<ManagedServer, String> {
    storage::managed::read_managed_server_by_id(scope.pool, id)
        .await
        .map_err(|_| "Managed servers could not be loaded".to_string())?
        .filter(|server| server.engine == DatabaseEngine::PostgreSQL)
        .ok_or_else(|| "Managed server no longer exists; reload and retry".into())
}

pub(super) async fn linked_connection_id(
    scope: &Scope<'_>,
    id: &str,
) -> Result<Option<String>, String> {
    Ok(record(scope, id).await?.connection_id)
}

async fn connection_ref(
    scope: &Scope<'_>,
    connection_id: Option<&str>,
) -> Result<Option<ManagedConnectionRef>, String> {
    let Some(connection_id) = connection_id else {
        return Ok(None);
    };
    Ok(storage::read_connection_by_id(scope.pool, connection_id)
        .await
        .map_err(|_| "Connections could not be loaded".to_string())?
        .map(|connection| ManagedConnectionRef {
            id: connection.id().into(),
            name: connection.name().into(),
        }))
}

async fn stored_password(scope: &Scope<'_>, server: &ManagedServer) -> Result<String, String> {
    let connection_id = server
        .connection_id
        .as_deref()
        .ok_or("Managed server has no linked connection; destroy and provision again")?;
    let mut connection = storage::read_connection_by_id(scope.pool, connection_id)
        .await
        .map_err(|_| "Connections could not be loaded".to_string())?
        .ok_or("The linked connection was deleted; destroy and provision again")?;
    let mode = credentials::credential_mode(scope.pool)
        .await?
        .ok_or("Credential storage is not configured")?;
    credentials::hydrate(scope.credentials, mode, &mut connection).await?;
    if connection.password().is_empty() {
        return Err("The managed server's saved password is unavailable".into());
    }
    Ok(connection.password().to_owned())
}

fn status_of(state: &str) -> ManagedContainerStatus {
    if state == "running" {
        ManagedContainerStatus::Running
    } else {
        ManagedContainerStatus::Stopped {
            state: bounded(state),
        }
    }
}

// --- Operations ------------------------------------------------------------

pub(super) async fn list(
    scope: &Scope<'_>,
    host: &dyn ManagedHost,
) -> Result<ManagedServerList, String> {
    let records = storage::managed::read_managed_servers(scope.pool)
        .await
        .map_err(|_| "Managed servers could not be loaded".to_string())?
        .into_iter()
        .filter(|server| server.engine == DatabaseEngine::PostgreSQL)
        .collect::<Vec<_>>();
    let connections = storage::read_native_connections(scope.pool)
        .await
        .map_err(|_| "Connections could not be loaded".to_string())?
        .into_iter()
        .map(|(connection, _)| (connection.id().to_owned(), connection.name().to_owned()))
        .collect::<HashMap<_, _>>();
    let docker_status = availability(host).await;
    // One listing each for containers and volumes, scoped to this profile.
    let observed = if docker_status.is_available() {
        let mut args: Vec<String> = vec!["ps".into(), "--all".into(), "--no-trunc".into()];
        args.extend(owner_filters(scope.profile_id, None));
        args.extend([
            "--format".into(),
            format!("{{{{.ID}}}}\t{{{{.State}}}}\t{{{{.Label \"{LABEL_RECORD}\"}}}}"),
        ]);
        let containers = docker(host, DockerCommand::new(args, QUICK), "container listing").await;
        let mut volume_args: Vec<String> = vec!["volume".into(), "ls".into()];
        volume_args.extend(owner_filters(scope.profile_id, None));
        volume_args.extend([
            "--format".into(),
            format!("{{{{.Name}}}}\t{{{{.Label \"{LABEL_RECORD}\"}}}}"),
        ]);
        let volumes = docker(
            host,
            DockerCommand::new(volume_args, QUICK),
            "volume listing",
        )
        .await;
        match (containers, volumes) {
            (Ok(containers), Ok(volumes)) => Ok((containers, volumes)),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    } else {
        Err(docker_status.describe())
    };
    let mut by_record: HashMap<&str, Vec<(String, String)>> = HashMap::new();
    let mut volumes_by_record: HashMap<&str, Vec<&str>> = HashMap::new();
    let (observed, observe_error) = match &observed {
        Ok(lists) => (Some(lists), None),
        Err(error) => (None, Some(error.clone())),
    };
    if let Some((containers, volumes)) = observed {
        for line in containers.lines() {
            let mut parts = line.trim().splitn(3, '\t');
            if let (Some(id), Some(state), Some(record)) =
                (parts.next(), parts.next(), parts.next())
            {
                if is_container_id(id) {
                    by_record
                        .entry(record.trim())
                        .or_default()
                        .push((id.to_owned(), state.trim().to_owned()));
                }
            }
        }
        for line in volumes.lines() {
            let mut parts = line.trim().splitn(2, '\t');
            if let (Some(name), Some(record)) = (parts.next(), parts.next()) {
                volumes_by_record
                    .entry(record.trim())
                    .or_default()
                    .push(name.trim());
            }
        }
    }
    let servers = records
        .into_iter()
        .map(|server| {
            let (status, container_id, volume_present) = if observed.is_some() {
                let containers = by_record.get(server.id.as_str());
                let volume_present = volumes_by_record
                    .get(server.id.as_str())
                    .is_some_and(|names| names.contains(&server.volume_name.as_str()));
                match containers.map(Vec::as_slice) {
                    None | Some([]) => {
                        (ManagedContainerStatus::Missing, None, Some(volume_present))
                    }
                    Some([(id, state)]) => {
                        (status_of(state), Some(id.clone()), Some(volume_present))
                    }
                    Some(_) => (ManagedContainerStatus::Conflict, None, Some(volume_present)),
                }
            } else {
                (ManagedContainerStatus::Unknown, None, None)
            };
            ManagedServerSummary {
                image: format!("postgres:{}", server.version),
                connection: server.connection_id.as_ref().and_then(|id| {
                    connections.get(id).map(|name| ManagedConnectionRef {
                        id: id.clone(),
                        name: name.clone(),
                    })
                }),
                status,
                container_id,
                volume_present,
                id: server.id,
                name: server.name,
                version: server.version,
                port: server.port,
                container_name: server.container_name,
                volume_name: server.volume_name,
                database: server.database,
                user: server.user,
                created_at: server.created_at,
            }
        })
        .collect();
    Ok(ManagedServerList {
        docker: docker_status,
        observe_error,
        servers,
    })
}

pub(super) async fn plan(
    scope: &Scope<'_>,
    host: &dyn ManagedHost,
    request: &ManagedProvisionRequest,
) -> Result<ManagedProvisionPlan, String> {
    let name = validate_name(&request.name)?;
    validate_version(&request.version)?;
    let claimed = storage::managed::claimed_ports(scope.pool)
        .await
        .map_err(|_| "Managed servers could not be loaded".to_string())?;
    let port = match request.port {
        Some(port) => {
            validate_port(port)?;
            if claimed.contains(&port) {
                return Err(format!(
                    "Port {port} is already assigned to a managed server"
                ));
            }
            if !host.port_free(port) {
                return Err(format!("Port {port} is in use on 127.0.0.1"));
            }
            port
        }
        None => (PORT_BASE..PORT_BASE + PORT_SPAN)
            .find(|port| !claimed.contains(port) && host.port_free(*port))
            .ok_or("No free loopback port was found for a managed server")?,
    };
    derive_plan(
        &uuid::Uuid::new_v4().to_string(),
        &name,
        &request.version,
        port,
    )
}

/// What one attempt created. The connection is saved last, so a failed
/// attempt never owns one.
#[derive(Default)]
struct Created {
    volume: Option<String>,
    record: bool,
}

async fn rollback(
    scope: &Scope<'_>,
    host: &dyn ManagedHost,
    record_id: &str,
    created: &Created,
) -> Vec<String> {
    let mut errors = Vec::new();
    if created.record {
        if let Err(error) = storage::managed::delete_managed_server(scope.pool, record_id).await {
            errors.push(format!("record: {}", bounded(&error)));
        }
    }
    // Always sweep owned containers: `docker run` can create one and still
    // fail to start it.
    if let Err(error) = remove_owned_containers(host, scope.profile_id, record_id).await {
        errors.push(format!("container: {error}"));
    }
    if let Some(volume) = &created.volume {
        if let Err(error) = remove_owned_volume(host, scope.profile_id, record_id, volume).await {
            errors.push(format!("volume: {error}"));
        }
    }
    errors
}

fn rolled_back(error: String, cleanup: Vec<String>, what: &str) -> String {
    if cleanup.is_empty() {
        format!("{error}; {what} was rolled back")
    } else {
        format!(
            "{error}; {what} rollback was incomplete ({})",
            cleanup.join("; ")
        )
    }
}

/// Provision exactly the reviewed plan. `gate` is awaited (and held) before
/// any profile record is written.
pub(super) async fn provision<G>(
    scope: &Scope<'_>,
    host: &dyn ManagedHost,
    reviewed: &ManagedProvisionPlan,
    gate: impl std::future::Future<Output = Result<G, String>>,
) -> Result<ManagedProvisioned, String> {
    let plan = derive_plan(
        &reviewed.record_id,
        &reviewed.name,
        &reviewed.version,
        reviewed.port,
    )?;
    if &plan != reviewed {
        return Err("Provisioning plan does not match its review; review again".into());
    }
    let _busy = Busy::claim(&plan.record_id)?;
    if storage::managed::read_managed_server_by_id(scope.pool, &plan.record_id)
        .await
        .map_err(|_| "Managed servers could not be loaded".to_string())?
        .is_some()
    {
        return Err("This plan was already provisioned; review a new plan".into());
    }
    require_docker(host).await?;
    let claimed = storage::managed::claimed_ports(scope.pool)
        .await
        .map_err(|_| "Managed servers could not be loaded".to_string())?;
    if claimed.contains(&plan.port) || !host.port_free(plan.port) {
        return Err(format!(
            "Port {} is no longer free; review a new plan",
            plan.port
        ));
    }
    if volume_name_taken(host, &plan.volume_name).await? {
        return Err(format!(
            "A Docker volume named {} already exists; it is never adopted",
            plan.volume_name
        ));
    }
    let password = generate_password();
    let mut created = Created::default();
    let result = async {
        // Rollback removes the volume only if it carries this record's labels.
        created.volume = Some(plan.volume_name.clone());
        create_volume(host, scope.profile_id, &plan.record_id, &plan.volume_name).await?;
        run_container(
            host,
            scope.profile_id,
            &RunSpec {
                record_id: &plan.record_id,
                container_name: &plan.container_name,
                version: &plan.version,
                port: plan.port,
                volume_name: &plan.volume_name,
                database: &plan.database,
                user: &plan.user,
                password: &password,
            },
        )
        .await?;
        host.ready(
            plan.port,
            &plan.database,
            &plan.user,
            &password,
            tokio::time::Instant::now() + PROVISION_READY,
        )
        .await?;
        let _gate = gate.await?;
        let connection_id = uuid::Uuid::new_v4().to_string();
        let server = ManagedServer {
            id: plan.record_id.clone(),
            name: plan.name.clone(),
            engine: DatabaseEngine::PostgreSQL,
            version: plan.version.clone(),
            port: plan.port,
            container_name: plan.container_name.clone(),
            volume_name: plan.volume_name.clone(),
            database: plan.database.clone(),
            user: plan.user.clone(),
            connection_id: Some(connection_id.clone()),
            created_at: storage::now(),
        };
        created.record = true;
        storage::managed::upsert_managed_server(scope.pool, &server)
            .await
            .map_err(|_| "Managed server record could not be saved".to_string())?;
        let connection = managed_connection(&server);
        if !scope.authority.permits(&connection) {
            return Err("The managed connection is outside this profile's limits".to_string());
        }
        native_connections::mutate(scope.credentials, |all| {
            if all.iter().any(|existing| existing.id() == connection_id) {
                return Err("Connection identity collision; nothing was saved".into());
            }
            Ok(Change::Save {
                connection,
                password: password.clone(),
                copy_from: None,
            })
        })
        .await?;
        Ok::<_, String>(connection_id)
    }
    .await;
    match result {
        Ok(connection_id) => Ok(ManagedProvisioned {
            record_id: plan.record_id,
            connection_id,
        }),
        Err(error) => {
            let cleanup = rollback(scope, host, &plan.record_id, &created).await;
            Err(rolled_back(error, cleanup, "provisioning"))
        }
    }
}

fn managed_connection(server: &ManagedServer) -> StoredConnection {
    StoredConnection::PostgreSQL(crate::PgStoredConnection {
        id: server.connection_id.clone().unwrap_or_default(),
        name: server.name.clone(),
        database: server.database.clone(),
        host: "127.0.0.1".into(),
        port: server.port,
        user: server.user.clone(),
        password: String::new(),
        role: "read/write".into(),
        environment: crate::Environment::Development,
        safe_mode: crate::SafeMode::default(),
        read_only: false,
        last_activity_at: None,
        organization: Default::default(),
        ssl: false,
        tls_options: None,
        driver_options: None,
        ssh_tunnel: crate::SshTunnelConfig::default(),
    })
}

pub(super) async fn start(
    scope: &Scope<'_>,
    host: &dyn ManagedHost,
    id: &str,
) -> Result<(), String> {
    let _busy = Busy::claim(id)?;
    let server = record(scope, id).await?;
    let password = stored_password(scope, &server).await?;
    require_docker(host).await?;
    let (container, state) = resolve(host, scope.profile_id, &server.id)
        .await?
        .ok_or("The managed container is missing; Recreate it")?;
    if state != "running" {
        docker(
            host,
            DockerCommand::new(["start", container.as_str()], LIFECYCLE),
            "start",
        )
        .await?;
    }
    host.ready(
        server.port,
        &server.database,
        &server.user,
        &password,
        tokio::time::Instant::now() + START_READY,
    )
    .await
}

/// Connect intent for the connection linked to a managed record: a stopped
/// owned container is started through [`start`] (label-verified, bounded by
/// `LIFECYCLE` and `START_READY`). Returns `Ok(true)` only when a start ran.
/// A connection without a managed record, or one already running, is left
/// alone. A missing container is an explicit error; connect never recreates.
pub(super) async fn ensure_running_for_connection(
    scope: &Scope<'_>,
    host: &dyn ManagedHost,
    connection_id: &str,
) -> Result<bool, String> {
    let Some(server) =
        storage::managed::read_managed_server_by_connection_id(scope.pool, connection_id)
            .await
            .map_err(|_| "Managed servers could not be loaded".to_string())?
            .filter(|server| server.engine == DatabaseEngine::PostgreSQL)
    else {
        return Ok(false);
    };
    let name = bounded(&server.name);
    let observed = match resolve(host, scope.profile_id, &server.id).await {
        Ok(observed) => observed,
        Err(error) => {
            let status = availability(host).await;
            let detail = if status.is_available() {
                error
            } else {
                status.describe()
            };
            return Err(format!(
                "Managed server '{name}' could not be checked before connecting: {detail}"
            ));
        }
    };
    match observed {
        None => Err(format!(
            "Managed server '{name}' has no container; Recreate it under Managed Servers"
        )),
        Some((_, state)) if state == "running" => Ok(false),
        Some(_) => {
            start(scope, host, &server.id).await.map_err(|error| {
                format!("Managed server '{name}' could not be started: {error}")
            })?;
            Ok(true)
        }
    }
}

pub(super) async fn stop(
    scope: &Scope<'_>,
    host: &dyn ManagedHost,
    id: &str,
) -> Result<(), String> {
    let _busy = Busy::claim(id)?;
    let server = record(scope, id).await?;
    require_docker(host).await?;
    let (container, state) = resolve(host, scope.profile_id, &server.id)
        .await?
        .ok_or("The managed container is missing; nothing to stop")?;
    if state == "running" {
        docker(
            host,
            DockerCommand::new(["stop", "--time", "10", container.as_str()], LIFECYCLE),
            "stop",
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn destroy_plan(
    scope: &Scope<'_>,
    host: &dyn ManagedHost,
    id: &str,
) -> Result<ManagedDestroyPlan, String> {
    let server = record(scope, id).await?;
    require_docker(host).await?;
    let containers = owned_containers(host, scope.profile_id, &server.id).await?;
    if containers.len() > 1 {
        return Err(
            "More than one container carries this managed server's labels; resolve them in Docker first"
                .into(),
        );
    }
    let volume = owned_volume(host, scope.profile_id, &server.id, &server.volume_name)
        .await?
        .then(|| server.volume_name.clone());
    Ok(ManagedDestroyPlan {
        record_id: server.id.clone(),
        name: server.name.clone(),
        container_id: containers.into_iter().next(),
        volume,
        connection: connection_ref(scope, server.connection_id.as_deref()).await?,
    })
}

/// Deletes exactly what `reviewed` names; any difference returns the current
/// plan for a new review and changes nothing.
pub(super) async fn destroy(
    scope: &Scope<'_>,
    host: &dyn ManagedHost,
    id: &str,
    reviewed: &ManagedDestroyPlan,
) -> Result<ManagedDestroy, String> {
    let _busy = Busy::claim(id)?;
    let current = destroy_plan(scope, host, id).await?;
    if &current != reviewed {
        return Ok(ManagedDestroy::ReviewRequired { plan: current });
    }
    if let Some(container) = &current.container_id {
        verify_container(host, scope.profile_id, id, container).await?;
        docker(
            host,
            DockerCommand::new(["rm", "--force", container.as_str()], LIFECYCLE),
            "remove",
        )
        .await?;
    }
    if let Some(volume) = &current.volume {
        remove_owned_volume(host, scope.profile_id, id, volume).await?;
    }
    if let Some(connection) = &current.connection {
        native_connections::mutate(scope.credentials, |_| {
            Ok(Change::Delete {
                id: connection.id.clone(),
            })
        })
        .await?;
    }
    storage::managed::delete_managed_server(scope.pool, id)
        .await
        .map_err(|_| "Managed server record could not be deleted".to_string())?;
    Ok(ManagedDestroy::Destroyed)
}

/// Recreate a missing container for the same record, port, connection and
/// password, reusing the owned volume when it survived. A failed attempt
/// removes only what it created.
pub(super) async fn recreate(
    scope: &Scope<'_>,
    host: &dyn ManagedHost,
    id: &str,
) -> Result<(), String> {
    let _busy = Busy::claim(id)?;
    let server = record(scope, id).await?;
    let password = stored_password(scope, &server).await?;
    require_docker(host).await?;
    if !owned_containers(host, scope.profile_id, &server.id)
        .await?
        .is_empty()
    {
        return Err("The managed container still exists; Start it, or Destroy first".into());
    }
    if !host.port_free(server.port) {
        return Err(format!("Port {} is in use on 127.0.0.1", server.port));
    }
    let reuse_volume =
        owned_volume(host, scope.profile_id, &server.id, &server.volume_name).await?;
    if !reuse_volume && volume_name_taken(host, &server.volume_name).await? {
        return Err(format!(
            "A Docker volume named {} exists without this server's labels; it is never adopted",
            server.volume_name
        ));
    }
    let mut created = Created::default();
    let result = async {
        if !reuse_volume {
            created.volume = Some(server.volume_name.clone());
            create_volume(host, scope.profile_id, &server.id, &server.volume_name).await?;
        }
        run_container(
            host,
            scope.profile_id,
            &RunSpec {
                record_id: &server.id,
                container_name: &server.container_name,
                version: &server.version,
                port: server.port,
                volume_name: &server.volume_name,
                database: &server.database,
                user: &server.user,
                password: &password,
            },
        )
        .await?;
        host.ready(
            server.port,
            &server.database,
            &server.user,
            &password,
            tokio::time::Instant::now() + PROVISION_READY,
        )
        .await
    }
    .await;
    if let Err(error) = result {
        let cleanup = rollback(scope, host, &server.id, &created).await;
        return Err(rolled_back(error, cleanup, "recreation"));
    }
    Ok(())
}
