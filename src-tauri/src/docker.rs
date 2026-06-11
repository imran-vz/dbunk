//! Thin async wrapper around the Docker CLI (ADR-0019).
//!
//! Managed Servers shell out to `docker` rather than speaking the
//! Engine API: dbunk owns orchestration only, and the CLI keeps the
//! dependency surface at zero while inheriting the user's Docker
//! context (Docker Desktop, colima, remote contexts) for free.

use std::path::Path;
use std::time::Duration;

use tokio::process::Command;

use crate::DockerStatus;

/// GUI-launched apps on macOS get a minimal PATH, so probe the common
/// install locations before giving up.
const FALLBACK_PATHS: &[&str] = &[
    "/usr/local/bin/docker",
    "/opt/homebrew/bin/docker",
    "/usr/bin/docker",
];

fn docker_binary() -> String {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("docker");
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    for candidate in FALLBACK_PATHS {
        if Path::new(candidate).exists() {
            return (*candidate).to_string();
        }
    }
    "docker".to_string()
}

/// Run `docker <args>`, returning trimmed stdout on success and a
/// trimmed stderr-derived message on failure.
async fn docker(args: &[&str], timeout: Duration) -> Result<String, String> {
    let binary = docker_binary();
    let output = tokio::time::timeout(
        timeout,
        Command::new(&binary).args(args).kill_on_drop(true).output(),
    )
    .await
    .map_err(|_| {
        format!(
            "docker {} timed out after {}s",
            args.first().unwrap_or(&""),
            timeout.as_secs()
        )
    })?
    .map_err(|error| format!("failed to run docker: {error}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("docker {} exited with {}", args.join(" "), output.status)
        } else {
            stderr
        })
    }
}

const QUICK: Duration = Duration::from_secs(15);
/// `docker run` pulls the image on first use; allow several minutes.
const PULL_AND_RUN: Duration = Duration::from_secs(600);

pub async fn status() -> DockerStatus {
    match docker(&["version", "--format", "{{.Server.Version}}"], QUICK).await {
        Ok(version) => DockerStatus {
            available: true,
            version: Some(version),
            error: None,
        },
        Err(error) => DockerStatus {
            available: false,
            version: None,
            error: Some(error),
        },
    }
}

pub async fn create_volume(name: &str) -> Result<(), String> {
    docker(&["volume", "create", name], QUICK).await.map(|_| ())
}

pub async fn volume_exists(name: &str) -> bool {
    docker(&["volume", "inspect", name], QUICK).await.is_ok()
}

pub async fn remove_volume(name: &str) -> Result<(), String> {
    docker(&["volume", "rm", name], QUICK).await.map(|_| ())
}

/// Create and start the container (pulling the image if needed).
/// Restart policy is deliberately `no` (ADR-0019): nothing dbunk-made
/// auto-starts at boot.
pub struct RunContainerSpec<'a> {
    pub name: &'a str,
    pub image: &'a str,
    pub host_port: u16,
    pub container_port: u16,
    pub volume_name: &'a str,
    pub volume_mount: &'a str,
    pub env: Vec<(String, String)>,
}

pub async fn run_container(spec: &RunContainerSpec<'_>) -> Result<(), String> {
    let port_mapping = format!("127.0.0.1:{}:{}", spec.host_port, spec.container_port);
    let volume_mapping = format!("{}:{}", spec.volume_name, spec.volume_mount);
    let mut args: Vec<String> = vec![
        "run".into(),
        "--detach".into(),
        "--restart".into(),
        "no".into(),
        "--name".into(),
        spec.name.into(),
        "--label".into(),
        "dev.dbunk.managed=true".into(),
        "--publish".into(),
        port_mapping,
        "--volume".into(),
        volume_mapping,
    ];
    for (key, value) in &spec.env {
        args.push("--env".into());
        args.push(format!("{key}={value}"));
    }
    args.push(spec.image.into());
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    docker(&args_ref, PULL_AND_RUN).await.map(|_| ())
}

pub async fn start_container(name: &str) -> Result<(), String> {
    docker(&["start", name], Duration::from_secs(60))
        .await
        .map(|_| ())
}

pub async fn stop_container(name: &str) -> Result<(), String> {
    docker(&["stop", name], Duration::from_secs(60))
        .await
        .map(|_| ())
}

pub async fn remove_container(name: &str) -> Result<(), String> {
    docker(&["rm", "--force", name], Duration::from_secs(60))
        .await
        .map(|_| ())
}

/// Container state from `docker inspect`: `Some("running")`,
/// `Some("exited")`, … or `None` when the container does not exist.
pub async fn container_state(name: &str) -> Option<String> {
    docker(&["inspect", "--format", "{{.State.Status}}", name], QUICK)
        .await
        .ok()
}
