//! Bounded Docker CLI invocations for native Managed Servers. Every call has a
//! deadline and no retries. Secrets travel through the child environment
//! (`--env NAME` without a value), never through argv.
use futures_util::future::BoxFuture;
use std::{collections::HashMap, path::Path, time::Duration};

/// Output text kept from a failed command, so a noisy pull cannot grow a
/// message without bound.
const MAX_DETAIL: usize = 512;

pub(super) const QUICK: Duration = Duration::from_secs(15);
pub(super) const LIFECYCLE: Duration = Duration::from_secs(45);
/// `docker run` pulls the image on first use.
pub(super) const PULL_AND_RUN: Duration = Duration::from_secs(600);

pub(super) const LABEL_MANAGED: &str = "dev.dbunk.managed";
pub(super) const LABEL_PROFILE: &str = "dev.dbunk.native.profile";
pub(super) const LABEL_RECORD: &str = "dev.dbunk.native.record";

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct DockerCommand {
    pub args: Vec<String>,
    /// Passed to the child process only; redacted from `Debug`.
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
}

impl std::fmt::Debug for DockerCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockerCommand")
            .field("args", &self.args)
            .field(
                "env",
                &self
                    .env
                    .iter()
                    .map(|(key, _)| format!("{key}=<redacted>"))
                    .collect::<Vec<_>>(),
            )
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl DockerCommand {
    pub(super) fn new<I, S>(args: I, timeout: Duration) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            args: args.into_iter().map(Into::into).collect(),
            env: Vec::new(),
            timeout,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DockerError {
    /// The CLI binary could not be found.
    Missing,
    TimedOut,
    Spawn(String),
    Exit {
        stderr: String,
    },
}

impl DockerError {
    pub(super) fn message(&self, action: &str) -> String {
        match self {
            Self::Missing => "Docker CLI was not found".into(),
            Self::TimedOut => format!("Docker {action} timed out; nothing was retried"),
            Self::Spawn(detail) => format!("Docker {action} could not start: {detail}"),
            Self::Exit { stderr } if stderr.is_empty() => format!("Docker {action} failed"),
            Self::Exit { stderr } => format!("Docker {action} failed: {stderr}"),
        }
    }
}

/// Injected side effects. Production uses the Docker CLI, loopback TCP and
/// PostgreSQL; tests script every response.
pub(crate) trait ManagedHost: Send + Sync {
    fn docker(&self, command: DockerCommand) -> BoxFuture<'_, Result<String, DockerError>>;
    /// Authenticated readiness before `deadline`. Never retries past it.
    fn ready<'a>(
        &'a self,
        port: u16,
        database: &'a str,
        user: &'a str,
        password: &'a str,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<(), String>>;
    fn port_free(&self, port: u16) -> bool;
}

/// Docker availability, classified without guessing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum DockerAvailability {
    Available { server_version: String },
    CliMissing,
    DaemonUnavailable { detail: String },
    PermissionDenied { detail: String },
    TimedOut,
    Failed { detail: String },
}

impl DockerAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Available { server_version } => format!("Docker {server_version} is available"),
            Self::CliMissing => "Docker CLI is not installed or not on PATH".into(),
            Self::DaemonUnavailable { detail } => {
                format!("Docker daemon is not running or unreachable: {detail}")
            }
            Self::PermissionDenied { detail } => {
                format!("Permission denied talking to the Docker daemon: {detail}")
            }
            Self::TimedOut => "Docker did not answer in time".into(),
            Self::Failed { detail } => format!("Docker check failed: {detail}"),
        }
    }
}

pub(super) fn classify(result: Result<String, DockerError>) -> DockerAvailability {
    match result {
        Ok(version) if !version.trim().is_empty() => DockerAvailability::Available {
            server_version: bounded(version.trim()),
        },
        Ok(_) => DockerAvailability::DaemonUnavailable {
            detail: "the daemon reported no server version".into(),
        },
        Err(DockerError::Missing) => DockerAvailability::CliMissing,
        Err(DockerError::TimedOut) => DockerAvailability::TimedOut,
        Err(DockerError::Spawn(detail)) => DockerAvailability::Failed { detail },
        Err(DockerError::Exit { stderr }) => {
            let lower = stderr.to_ascii_lowercase();
            if lower.contains("permission denied") {
                DockerAvailability::PermissionDenied { detail: stderr }
            } else if [
                "cannot connect to the docker daemon",
                "is the docker daemon running",
                "error during connect",
                "connection refused",
                "no such file or directory",
                "daemon is not running",
            ]
            .iter()
            .any(|needle| lower.contains(needle))
            {
                DockerAvailability::DaemonUnavailable { detail: stderr }
            } else {
                DockerAvailability::Failed { detail: stderr }
            }
        }
    }
}

pub(super) fn version_command() -> DockerCommand {
    DockerCommand::new(["version", "--format", "{{.Server.Version}}"], QUICK)
}

pub(super) fn bounded(text: &str) -> String {
    let text = text.trim();
    if text.len() <= MAX_DETAIL {
        return text.to_owned();
    }
    let mut end = MAX_DETAIL;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

pub(super) fn is_container_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(super) fn parse_labels(json: &str) -> Option<HashMap<String, String>> {
    let trimmed = json.trim();
    if trimmed == "null" {
        return Some(HashMap::new());
    }
    serde_json::from_str(trimmed).ok()
}

pub(super) fn owned_labels(labels: &HashMap<String, String>, profile: &str, record: &str) -> bool {
    labels.get(LABEL_MANAGED).map(String::as_str) == Some("true")
        && labels.get(LABEL_PROFILE).map(String::as_str) == Some(profile)
        && labels.get(LABEL_RECORD).map(String::as_str) == Some(record)
}

/// The production host: the Docker CLI on this machine.
pub(crate) struct CliHost;

/// GUI-launched apps on macOS get a minimal PATH.
const FALLBACK_PATHS: &[&str] = &[
    "/usr/local/bin/docker",
    "/opt/homebrew/bin/docker",
    "/usr/bin/docker",
    "/Applications/Docker.app/Contents/Resources/bin/docker",
];

fn docker_binary() -> Option<std::path::PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("docker");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    FALLBACK_PATHS
        .iter()
        .map(Path::new)
        .find(|candidate| candidate.is_file())
        .map(Path::to_path_buf)
}

impl ManagedHost for CliHost {
    fn docker(&self, command: DockerCommand) -> BoxFuture<'_, Result<String, DockerError>> {
        Box::pin(async move {
            let binary = docker_binary().ok_or(DockerError::Missing)?;
            let mut child = tokio::process::Command::new(binary);
            child
                .args(&command.args)
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true);
            for (key, value) in &command.env {
                child.env(key, value);
            }
            let output = tokio::time::timeout(command.timeout, child.output())
                .await
                .map_err(|_| DockerError::TimedOut)?
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        DockerError::Missing
                    } else {
                        DockerError::Spawn(bounded(&error.to_string()))
                    }
                })?;
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
            } else {
                Err(DockerError::Exit {
                    stderr: bounded(&String::from_utf8_lossy(&output.stderr)),
                })
            }
        })
    }

    fn ready<'a>(
        &'a self,
        port: u16,
        database: &'a str,
        user: &'a str,
        password: &'a str,
        deadline: tokio::time::Instant,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            use sqlx::ConnectOptions;
            let mut last = String::from("server did not accept connections");
            while tokio::time::Instant::now() < deadline {
                let options = sqlx::postgres::PgConnectOptions::new()
                    .host("127.0.0.1")
                    .port(port)
                    .database(database)
                    .username(user)
                    .password(password)
                    .ssl_mode(sqlx::postgres::PgSslMode::Disable);
                let attempt = tokio::time::timeout_at(
                    deadline.min(tokio::time::Instant::now() + Duration::from_secs(5)),
                    options.connect(),
                )
                .await;
                match attempt {
                    Ok(Ok(connection)) => {
                        let _ = tokio::time::timeout(
                            Duration::from_secs(2),
                            sqlx::Connection::close(connection),
                        )
                        .await;
                        return Ok(());
                    }
                    Ok(Err(error)) => last = bounded(&error.to_string()),
                    Err(_) => last = "connection attempt timed out".into(),
                }
                tokio::time::sleep_until(
                    deadline.min(tokio::time::Instant::now() + Duration::from_millis(750)),
                )
                .await;
            }
            Err(format!("PostgreSQL did not become ready in time: {last}"))
        })
    }

    fn port_free(&self, port: u16) -> bool {
        std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
    }
}
