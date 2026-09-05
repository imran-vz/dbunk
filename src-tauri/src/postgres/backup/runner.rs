use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::process::Command;

use super::manager::JobContext;
use super::protocol::*;
use crate::postgres::tls;
use crate::{PgStoredConnection, StoredConnection};

const STDERR_LIMIT: usize = 64 * 1024;
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(10);
// Leave a small bookkeeping margin inside the manager's absolute five-second
// teardown bound after a child consumes nearly all of its kill/reap wait.
const REAP_TIMEOUT: Duration = Duration::from_millis(4_800);
// Default pg_dump guards fit comfortably. Oversized or noncanonical wrappers
// stay in the stream and are rejected by the private psql restriction.
const PLAIN_GUARD_PROBE_LIMIT: usize = 512;
const FALLBACK_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/Applications/Postgres.app/Contents/Versions/latest/bin",
];

struct ProgressTask(tokio::task::JoinHandle<()>);
impl Drop for ProgressTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) enum Request {
    Backup(StartPgBackupPayload),
    Restore(StartPgRestorePayload),
}
impl Request {
    pub(crate) fn connection_id(&self) -> &str {
        match self {
            Self::Backup(p) => &p.connection_id,
            Self::Restore(p) => &p.connection_id,
        }
    }
    fn path(&self) -> &Path {
        Path::new(match self {
            Self::Backup(p) => &p.destination_path,
            Self::Restore(p) => &p.source_path,
        })
    }
    fn format(&self) -> PgBackupFormat {
        match self {
            Self::Backup(p) => p.format,
            Self::Restore(p) => p.format,
        }
    }
    fn tool(&self) -> &'static str {
        match self {
            Self::Backup(_) => "pg_dump",
            Self::Restore(p) if p.format == PgBackupFormat::Plain => "psql",
            Self::Restore(_) => "pg_restore",
        }
    }
    pub(crate) fn snapshot(&self) -> PgToolJobSnapshot {
        PgToolJobSnapshot {
            job_id: String::new(),
            connection_id: self.connection_id().into(),
            kind: match self {
                Self::Backup(_) => PgToolJobKind::Backup,
                Self::Restore(_) => PgToolJobKind::Restore,
            },
            format: self.format(),
            file_name: self
                .path()
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            phase: PgToolJobPhase::Queued,
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            bytes_processed: matches!(self, Self::Backup(_)).then_some(0),
            total_bytes: None,
            tool_version: None,
            failure: None,
        }
    }
    pub(crate) async fn validate(&self) -> Result<(), PgToolJobError> {
        let path = self.path();
        let field = if matches!(self, Self::Backup(_)) {
            "destinationPath"
        } else {
            "sourcePath"
        };
        if !path.is_absolute() || path.file_name().is_none() {
            return Err(PgToolJobError::invalid(
                field,
                "An absolute file path is required",
            ));
        }
        match self {
            Self::Backup(p) => {
                if p.clean && p.format != PgBackupFormat::Plain {
                    return Err(PgToolJobError::invalid(
                        "clean",
                        "Cleanup is only supported in plain backups",
                    ));
                }
                let identifiers: Vec<&str> = match &p.scope {
                    PgBackupScope::Database => vec![],
                    PgBackupScope::Schema { schema } => vec![schema],
                    PgBackupScope::Table { schema, table } => vec![schema, table],
                };
                if identifiers.iter().any(|s| s.is_empty() || s.contains('\0')) {
                    return Err(PgToolJobError::invalid(
                        "scope",
                        "Non-empty identifiers without NUL are required",
                    ));
                }
                match tokio::fs::symlink_metadata(path).await {
                    Ok(_) => return Err(PgToolJobError::DestinationExists),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(PgToolJobError::io("destinationMetadata", &e)),
                }
                let parent = tokio::fs::metadata(path.parent().expect("absolute file parent"))
                    .await
                    .map_err(|e| PgToolJobError::io("parentMetadata", &e))?;
                if !parent.is_dir() {
                    return Err(PgToolJobError::invalid(
                        field,
                        "Parent must be an existing directory",
                    ));
                }
            }
            Self::Restore(p) => {
                if p.clean && p.format == PgBackupFormat::Plain {
                    return Err(PgToolJobError::invalid(
                        "clean",
                        "Plain restore cannot add cleanup; use a plain backup made with clean",
                    ));
                }
                regular_nonempty(path, "sourceMetadata").await?;
            }
        }
        Ok(())
    }
}

/// Owns an unpublished archive. RAII removes it on every error or unwind.
pub(crate) enum Ready {
    Backup {
        partial: Arc<tempfile::NamedTempFile>,
        destination: PathBuf,
    },
    Restore,
    #[cfg(test)]
    PublicationTest {
        started: tokio::sync::oneshot::Sender<()>,
        finish: tokio::sync::oneshot::Receiver<()>,
    },
}
impl Ready {
    pub(super) fn requires_publication_claim(&self) -> bool {
        !matches!(self, Self::Restore)
    }
    pub(super) fn publish(self) -> Result<(), PgToolJobError> {
        match self {
            Self::Backup {
                partial,
                destination,
            } => {
                let partial = Arc::try_unwrap(partial)
                    .map_err(|_| PgToolJobError::invalid("job", "Archive still in use"))?;
                // persist_noclobber atomically publishes a complete file without replacing
                // an entry created after validation. A failed publish retains RAII cleanup.
                partial.persist_noclobber(destination).map_err(|e| {
                    if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                        PgToolJobError::DestinationExists
                    } else {
                        PgToolJobError::io("publish", &e.error)
                    }
                })?;
            }
            Self::Restore => {}
            #[cfg(test)]
            Self::PublicationTest { started, finish } => {
                let _ = started.send(());
                let _ = finish.blocking_recv();
            }
        }
        Ok(())
    }
}

pub(crate) async fn run(
    context: JobContext,
    connection: StoredConnection,
    request: Request,
) -> Result<Ready, PgToolJobError> {
    context.phase(PgToolJobPhase::Preflight)?;
    request.validate().await?;
    context.check_cancelled()?;
    let tool = request.tool();
    let binary = resolve_tool(tool).await;
    let version = process(
        preflight_command(&binary),
        tool,
        &context,
        None,
        None,
        Some(PREFLIGHT_TIMEOUT),
        true,
        false,
        None,
    )
    .await?;
    // Version output is trusted only when it matches the native tool's fixed banner.
    let version = String::from_utf8_lossy(&version).trim().to_string();
    if !version.starts_with(&format!("{tool} (PostgreSQL) "))
        || version.lines().count() != 1
        || version.len() > 256
    {
        return Err(PgToolJobError::ToolFailed {
            tool: tool.into(),
            exit_code: Some(0),
            message: "Unrecognized PostgreSQL tool version banner".into(),
        });
    }
    if !postgres_client_has_required_patches(tool, &version) {
        return Err(PgToolJobError::ToolFailed {
            tool: tool.into(),
            exit_code: Some(0),
            message: "A supported patched PostgreSQL client is required".into(),
        });
    }
    context.progress(None, None, Some(version));
    let StoredConnection::PostgreSQL(pg) = &connection else {
        return Err(PgToolJobError::UnsupportedEngine);
    };
    context.phase(PgToolJobPhase::Running)?;
    let ready = match &request {
        Request::Backup(payload) => {
            let path = request.path().to_owned();
            let prefix = format!(
                ".{}.dbunk-partial-{}-",
                path.file_name().unwrap().to_string_lossy(),
                context.job_id
            );
            let partial = tokio::task::spawn_blocking(move || {
                tempfile::Builder::new()
                    .prefix(&prefix)
                    .tempfile_in(path.parent().unwrap())
            })
            .await
            .map_err(|_| PgToolJobError::invalid("job", "File worker stopped"))?
            .map_err(|e| PgToolJobError::io("createPartial", &e))?;
            let partial = Arc::new(partial);
            let (mut command, material) = connection_command(pg, &binary)?;
            command.args(backup_args(payload, partial.path()));
            process(
                command,
                tool,
                &context,
                Some(partial.clone()),
                Some(material),
                None,
                false,
                false,
                None,
            )
            .await?;
            context.phase(PgToolJobPhase::Finalizing)?;
            let size = regular_nonempty(partial.path(), "partialMetadata").await?;
            let file = tokio::fs::OpenOptions::new()
                .write(true)
                .open(partial.path())
                .await
                .map_err(|e| PgToolJobError::io("openPartial", &e))?;
            file.sync_all()
                .await
                .map_err(|e| PgToolJobError::io("syncPartial", &e))?;
            context.progress(Some(size), None, None);
            Ready::Backup {
                partial,
                destination: request.path().into(),
            }
        }
        Request::Restore(payload) => {
            context.progress(
                None,
                Some(regular_nonempty(request.path(), "sourceMetadata").await?),
                None,
            );
            if payload.format == PgBackupFormat::Custom {
                // Archive inspection needs no credentials or connection arguments.
                let mut command = Command::new(&binary);
                command.args(list_args(request.path()));
                process(
                    command, tool, &context, None, None, None, false, false, None,
                )
                .await?;
            }
            let (mut command, material) = connection_command(pg, &binary)?;
            let (args, plain_input) = match payload.format {
                PgBackupFormat::Plain => {
                    let key = format!("DBUNK{}", uuid::Uuid::new_v4().simple());
                    (
                        plain_restore_args(&key),
                        Some(PathBuf::from(&payload.source_path)),
                    )
                }
                PgBackupFormat::Custom => (custom_restore_args(payload), None),
            };
            command.args(args);
            process(
                command,
                tool,
                &context,
                None,
                Some(material),
                None,
                false,
                true,
                plain_input,
            )
            .await?;
            context.phase_after_irreversible_success(PgToolJobPhase::Finalizing)?;
            Ready::Restore
        }
    };
    context.check_cancelled()?;
    Ok(ready)
}

async fn regular_nonempty(path: &Path, operation: &str) -> Result<u64, PgToolJobError> {
    let meta = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| PgToolJobError::io(operation, &e))?;
    if !meta.is_file() || meta.len() == 0 {
        return Err(PgToolJobError::invalid(
            "file",
            "A non-empty regular file is required",
        ));
    }
    Ok(meta.len())
}

fn literal_pattern(identifier: &str) -> String {
    crate::quote_double(identifier)
}
pub(super) fn backup_args(p: &StartPgBackupPayload, path: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        match p.format {
            PgBackupFormat::Plain => "--format=plain",
            PgBackupFormat::Custom => "--format=custom",
        }
        .into(),
        "--file".into(),
        path.into(),
    ];
    if p.clean {
        args.extend(["--clean".into(), "--if-exists".into()]);
    }
    match &p.scope {
        PgBackupScope::Database => {}
        PgBackupScope::Schema { schema } => {
            args.extend(["--schema".into(), literal_pattern(schema).into()])
        }
        PgBackupScope::Table { schema, table } => args.extend([
            "--table".into(),
            format!("{}.{}", literal_pattern(schema), literal_pattern(table)).into(),
        ]),
    }
    args
}
pub(super) fn plain_restore_args(restrict_key: &str) -> Vec<OsString> {
    vec![
        "--single-transaction".into(),
        "--no-psqlrc".into(),
        "--set=ON_ERROR_STOP=on".into(),
        "--command".into(),
        format!(r"\restrict {restrict_key}").into(),
        "--file".into(),
        "-".into(),
    ]
}

pub(super) fn custom_restore_args(p: &StartPgRestorePayload) -> Vec<OsString> {
    let mut args = vec!["--single-transaction".into()];
    if p.clean {
        args.extend(["--clean".into(), "--if-exists".into()]);
    }
    args.push(p.source_path.clone().into());
    args
}

// These are the first minors that fix the PostgreSQL client vulnerabilities
// covered by this runner, including CVE-2026-18408 and CVE-2026-19385.
fn postgres_client_has_required_patches(tool: &str, version: &str) -> bool {
    let prefix = format!("{tool} (PostgreSQL) ");
    let Some(number) = version.strip_prefix(&prefix) else {
        return false;
    };
    let mut parts = number.split('.');
    let Some(major) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
        return false;
    };
    let Some(minor) = parts.next().and_then(|part| {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        (!digits.is_empty())
            .then(|| digits.parse::<u32>().ok())
            .flatten()
    }) else {
        return false;
    };

    match major {
        14 => minor >= 24,
        15 => minor >= 19,
        16 => minor >= 15,
        17 => minor >= 11,
        18 => minor >= 6,
        19.. => true,
        _ => false,
    }
}
fn list_args(path: &Path) -> Vec<OsString> {
    vec!["--list".into(), path.into()]
}
fn preflight_command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command.arg("--version");
    command
}

fn connection_command(
    connection: &PgStoredConnection,
    binary: &Path,
) -> Result<(Command, tls::LibpqTlsMaterial), PgToolJobError> {
    let mut command = std::process::Command::new(binary);
    let resolved = tls::ResolvedTls::from_postgres(connection);
    let material = tls::apply_to_command(&resolved, &connection.host, &mut command)
        .map_err(|_| PgToolJobError::invalid("tls", "Unable to prepare TLS material"))?;
    command
        .arg("--port")
        .arg(connection.effective_port().to_string())
        .arg("--username")
        .arg(&connection.user)
        .arg("--dbname")
        // libpq expands --dbname values containing '=' or a URI as conninfo.
        // Wrap the name as one quoted conninfo value so it cannot redirect the
        // hydrated credential to a different host, database, or TLS policy.
        .arg(format!(
            "dbname='{}'",
            connection
                .database
                .replace('\\', "\\\\")
                .replace('\'', "\\'")
        ))
        .arg("--no-password")
        .env("PGPASSWORD", &connection.password)
        .env("PGAPPNAME", "dbunk-pg-tool");
    Ok((Command::from(command), material))
}

pub(super) fn resolve_tool_with(
    tool: &str,
    path: Option<&OsStr>,
    fallbacks: &[PathBuf],
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let name = if cfg!(windows) {
        format!("{tool}.exe")
    } else {
        tool.into()
    };
    path.map(std::env::split_paths)
        .into_iter()
        .flatten()
        .chain(fallbacks.iter().cloned())
        .map(|dir| dir.join(&name))
        .find(|candidate| exists(candidate))
        .unwrap_or_else(|| name.into())
}
async fn resolve_tool(tool: &str) -> PathBuf {
    let mut fallbacks: Vec<_> = FALLBACK_DIRS.iter().map(PathBuf::from).collect();
    if cfg!(target_os = "linux") {
        let mut versions = Vec::new();
        if let Ok(mut dirs) = tokio::fs::read_dir("/usr/lib/postgresql").await {
            while let Ok(Some(dir)) = dirs.next_entry().await {
                if let Ok(version) = dir.file_name().to_string_lossy().parse::<u32>() {
                    versions.push((version, dir.path().join("bin")));
                }
            }
        }
        versions.sort_by_key(|v| std::cmp::Reverse(v.0));
        fallbacks.extend(versions.into_iter().map(|(_, path)| path));
    }
    resolve_tool_with(
        tool,
        std::env::var_os("PATH").as_deref(),
        &fallbacks,
        Path::is_file,
    )
}

enum GuardProbe {
    NeedMore,
    NoGuard,
    Guard {
        line_start: usize,
        line_end: usize,
        key: Vec<u8>,
    },
}

fn probe_pg_dump_guard(input: &[u8], eof: bool) -> GuardProbe {
    const LF_PREFIX: &[u8] = b"--\n-- PostgreSQL database dump\n--\n\n\\restrict ";
    const CRLF_PREFIX: &[u8] = b"--\r\n-- PostgreSQL database dump\r\n--\r\n\r\n\\restrict ";

    for (prefix, newline) in [
        (LF_PREFIX, b"\n".as_slice()),
        (CRLF_PREFIX, b"\r\n".as_slice()),
    ] {
        if prefix.starts_with(input) {
            return if eof {
                GuardProbe::NoGuard
            } else {
                GuardProbe::NeedMore
            };
        }
        if let Some(rest) = input.strip_prefix(prefix) {
            let Some(relative_end) = rest
                .windows(newline.len())
                .position(|window| window == newline)
            else {
                return if eof || input.len() >= PLAIN_GUARD_PROBE_LIMIT {
                    GuardProbe::NoGuard
                } else {
                    GuardProbe::NeedMore
                };
            };
            let key = &rest[..relative_end];
            if key.is_empty() || !key.iter().all(u8::is_ascii_alphanumeric) {
                return GuardProbe::NoGuard;
            }
            return GuardProbe::Guard {
                line_start: prefix.len() - b"\\restrict ".len(),
                line_end: prefix.len() + relative_end + newline.len(),
                key: key.to_vec(),
            };
        }
    }

    GuardProbe::NoGuard
}

async fn write_input(writer: &mut (impl AsyncWrite + Unpin), bytes: &[u8]) -> std::io::Result<()> {
    writer.write_all(bytes).await
}

enum PlainStreamResult<W> {
    Complete,
    Failed { error: std::io::Error, writer: W },
}

/// Stream a plain dump without allowing any selected-file bytes to run before
/// psql's private restricted mode. New pg_dump output carries its own outer
/// guard, so remove only that canonical wrapper while preserving the body.
async fn stream_plain_restore_body(
    path: PathBuf,
    writer: &mut (impl AsyncWrite + Unpin),
) -> std::io::Result<()> {
    let mut source = tokio::fs::File::open(path).await?;
    let mut probe = Vec::with_capacity(PLAIN_GUARD_PROBE_LIMIT);
    let mut buf = [0_u8; 8192];

    let (key, initial_body) = loop {
        let remaining = PLAIN_GUARD_PROBE_LIMIT.saturating_sub(probe.len());
        let read = source.read(&mut buf[..remaining]).await?;
        if read > 0 {
            probe.extend_from_slice(&buf[..read]);
        }
        match probe_pg_dump_guard(&probe, read == 0) {
            GuardProbe::NeedMore if probe.len() < PLAIN_GUARD_PROBE_LIMIT => continue,
            GuardProbe::Guard {
                line_start,
                line_end,
                key,
            } => {
                write_input(writer, &probe[..line_start]).await?;
                break (key, probe.split_off(line_end));
            }
            GuardProbe::NeedMore | GuardProbe::NoGuard => {
                write_input(writer, &probe).await?;
                tokio::io::copy(&mut source, writer).await?;
                return Ok(());
            }
        }
    };

    let lf_suffix = [
        b"\\unrestrict ".as_slice(),
        key.as_slice(),
        b"\n\n".as_slice(),
    ]
    .concat();
    let crlf_suffix = [
        b"\\unrestrict ".as_slice(),
        key.as_slice(),
        b"\r\n\r\n".as_slice(),
    ]
    .concat();
    let retain = lf_suffix.len().max(crlf_suffix.len());
    let mut tail = initial_body;

    loop {
        let read = source.read(&mut buf).await?;
        if read == 0 {
            break;
        }
        tail.extend_from_slice(&buf[..read]);
        if tail.len() > retain {
            let flush = tail.len() - retain;
            write_input(writer, &tail[..flush]).await?;
            tail.drain(..flush);
        }
    }
    if tail.ends_with(&lf_suffix) {
        tail.truncate(tail.len() - lf_suffix.len());
    } else if tail.ends_with(&crlf_suffix) {
        tail.truncate(tail.len() - crlf_suffix.len());
    }
    write_input(writer, &tail).await
}

async fn stream_plain_restore<W>(path: PathBuf, mut writer: W) -> PlainStreamResult<W>
where
    W: AsyncWrite + Unpin,
{
    if let Err(error) = stream_plain_restore_body(path, &mut writer).await {
        return PlainStreamResult::Failed { error, writer };
    }
    match writer.shutdown().await {
        Ok(()) => PlainStreamResult::Complete,
        Err(error) => PlainStreamResult::Failed { error, writer },
    }
}

/// Drain throughout the child's lifetime while retaining only a bounded tail.
pub(super) async fn drain_tail(
    mut pipe: impl AsyncRead + Unpin,
    retain: usize,
) -> std::io::Result<Vec<u8>> {
    let mut tail = Vec::with_capacity(retain);
    let mut buf = [0; 8192];
    loop {
        let n = pipe.read(&mut buf).await?;
        if n == 0 {
            return Ok(tail);
        }
        if retain == 0 {
            continue;
        }
        let incoming = &buf[n.saturating_sub(retain)..n];
        let excess = (tail.len() + incoming.len()).saturating_sub(retain);
        tail.drain(..excess);
        tail.extend_from_slice(incoming);
    }
}

// Database stderr can echo SQL, row values, passwords, and arbitrary server
// messages. Return recognized diagnostic categories only, never raw lines.
fn diagnostic(tail: &[u8]) -> String {
    let text = String::from_utf8_lossy(tail);
    let known = [
        (
            "does not exist",
            "A referenced database object or role does not exist",
        ),
        ("already exists", "A database object already exists"),
        ("permission denied", "Permission denied"),
        ("authentication failed", "Database authentication failed"),
        ("Connection refused", "Database connection refused"),
        (
            "connection to server",
            "Unable to connect to the database server",
        ),
        (
            "server version mismatch",
            "Client and server versions are incompatible",
        ),
        ("unsupported version", "Unsupported archive version"),
        ("syntax error", "SQL syntax error"),
        (
            "invalid command",
            "Unsupported SQL meta-command; check client tool versions",
        ),
        (
            "does not appear to be a valid archive",
            "Invalid PostgreSQL archive",
        ),
    ];
    let messages: Vec<_> = known
        .iter()
        .filter_map(|(pattern, message)| text.contains(pattern).then_some(*message))
        .collect();
    if messages.is_empty() {
        "Native tool failed; unrecognized diagnostic text withheld because it may contain database data".into()
    } else {
        messages.join("; ")
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn process(
    mut command: Command,
    tool: &str,
    context: &JobContext,
    partial: Option<Arc<tempfile::NamedTempFile>>,
    material: Option<tls::LibpqTlsMaterial>,
    deadline: Option<Duration>,
    capture_stdout: bool,
    irreversible_success: bool,
    stdin_source: Option<PathBuf>,
) -> Result<Vec<u8>, PgToolJobError> {
    context.check_cancelled()?;
    command
        .kill_on_drop(true)
        .stdin(if stdin_source.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped())
        .stdout(if capture_stdout {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = command
        .spawn()
        .map_err(|_| PgToolJobError::ToolUnavailable { tool: tool.into() })?;
    let stderr = tokio::spawn(drain_tail(
        child.stderr.take().expect("piped stderr"),
        STDERR_LIMIT,
    ));
    let stdout = child
        .stdout
        .take()
        .map(|pipe| tokio::spawn(drain_tail(pipe, 1024)));
    let mut stdin = stdin_source.map(|path| {
        let pipe = child.stdin.take().expect("piped stdin");
        tokio::spawn(stream_plain_restore(path, pipe))
    });
    let mut held_stdin = None;
    // Filesystem metadata can stall independently of the child. Keep it out of
    // this select loop so cancellation always reaches start_kill promptly.
    let progress = partial.as_ref().map(|file| {
        let path = file.path().to_owned();
        let context = context.clone();
        ProgressTask(tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(200));
            loop {
                tick.tick().await;
                if let Ok(meta) = tokio::fs::metadata(&path).await {
                    context.progress(Some(meta.len()), None, None);
                }
            }
        }))
    });
    let timeout = async {
        match deadline {
            Some(duration) => tokio::time::sleep(duration).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(timeout);
    let reason = loop {
        let outcome = tokio::select! {
            biased;
            _ = context.cancelled() => {
                match child.try_wait() {
                    Ok(Some(status)) if irreversible_success && status.success() => {
                        context.irreversible_success();
                        Some(Ok(status))
                    }
                    _ => Some(Err(PgToolJobError::Cancelled)),
                }
            }
            _ = &mut timeout => Some(Err(PgToolJobError::Timeout { operation: "preflight".into() })),
            input = async {
                match stdin.as_mut() {
                    Some(task) => Some(task.await),
                    None => std::future::pending().await,
                }
            } => {
                stdin = None;
                match input.expect("stdin task is present") {
                    Ok(PlainStreamResult::Complete) => None,
                    Ok(PlainStreamResult::Failed { error, writer })
                        if error.kind() == std::io::ErrorKind::BrokenPipe => {
                            drop(writer);
                            None
                        }
                    Ok(PlainStreamResult::Failed { error, writer }) => {
                        // Keep the pipe open until kill has been requested and the
                        // child reaped. EOF must never turn a partial read into a
                        // successful single-transaction commit.
                        held_stdin = Some(writer);
                        Some(Err(PgToolJobError::io("streamSource", &error)))
                    }
                    Err(_) => Some(Err(PgToolJobError::invalid("file", "Source stream stopped"))),
                }
            }
            status = child.wait() => Some(status.map_err(|e| PgToolJobError::io("wait", &e))),
        };
        if let Some(outcome) = outcome {
            break outcome;
        }
    };
    if reason.is_err() {
        let _ = child.start_kill();
        if let Some(stdin) = stdin.take() {
            stdin.abort();
        }
        let reap_context = context.clone();
        let reaped = reap(
            context,
            async move {
                let status = child.wait().await;
                if irreversible_success && status.as_ref().is_ok_and(|status| status.success()) {
                    // Record committed restore success in the cleanup owner so
                    // a teardown watchdog cannot mislabel it before the worker
                    // task resumes from the reap wait.
                    reap_context.irreversible_success();
                }
                stderr.abort();
                if let Some(stdout) = stdout {
                    stdout.abort();
                }
                drop(held_stdin);
                drop(progress);
                drop(material);
                drop(partial);
                status
            },
            REAP_TIMEOUT,
        )
        .await;
        let Ok(status) = reaped else {
            return Err(PgToolJobError::Timeout {
                operation: "reap".into(),
            });
        };
        if reaped_irreversible_success(irreversible_success, &status) {
            context.irreversible_success();
            return Ok(Vec::new());
        }
        return Err(reason.expect_err("interrupted child"));
    }
    drop(progress);
    if let Some(stdin) = stdin {
        stdin.abort();
    }
    drop(held_stdin);
    // Only the direct child is supported; do not let an inherited pipe in a
    // descendant hold teardown forever after that child has been reaped.
    let mut stderr = stderr;
    let tail = match tokio::time::timeout(Duration::from_millis(100), &mut stderr).await {
        Ok(Ok(Ok(tail))) => tail,
        _ => {
            stderr.abort();
            Vec::new()
        }
    };
    let output = if let Some(mut stdout) = stdout {
        match tokio::time::timeout(Duration::from_millis(100), &mut stdout).await {
            Ok(Ok(Ok(out))) => out,
            _ => {
                stdout.abort();
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let status = reason?;
    if irreversible_success && status.success() {
        context.irreversible_success();
    }
    if !status.success() {
        return Err(PgToolJobError::ToolFailed {
            tool: tool.into(),
            exit_code: status.code(),
            message: diagnostic(&tail),
        });
    }
    Ok(output)
}

fn reaped_irreversible_success(
    irreversible_success: bool,
    status: &std::io::Result<std::process::ExitStatus>,
) -> bool {
    irreversible_success && status.as_ref().is_ok_and(|status| status.success())
}

/// The task owns the child and cleanup after the bounded foreground wait.
pub(super) async fn reap<T, F>(context: &JobContext, work: F, deadline: Duration) -> Result<T, ()>
where
    T: Send + 'static,
    F: std::future::Future<Output = T> + Send + 'static,
{
    let mut finished = context.spawn_reaper(work);
    match tokio::time::timeout(deadline, &mut finished).await {
        Ok(Ok(Ok(output))) => Ok(output),
        Ok(Ok(Err(_))) | Ok(Err(_)) | Err(_) => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn filtered_plain_input(source: &[u8]) -> Vec<u8> {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), source).unwrap();
        let (writer, mut reader) = tokio::io::duplex(37);
        let path = file.path().to_owned();
        let pump = tokio::spawn(async move { stream_plain_restore(path, writer).await });
        let mut output = Vec::new();
        reader.read_to_end(&mut output).await.unwrap();
        assert!(matches!(pump.await.unwrap(), PlainStreamResult::Complete));
        output
    }

    #[test]
    fn preflight_and_list_never_use_connection_arguments() {
        let command = preflight_command(Path::new("pg_dump"));
        assert_eq!(
            command.as_std().get_args().collect::<Vec<_>>(),
            vec!["--version"]
        );
        assert_eq!(list_args(Path::new("/archive")), vec!["--list", "/archive"]);
        assert_eq!(PREFLIGHT_TIMEOUT, Duration::from_secs(10));
    }
    #[test]
    fn diagnostic_never_returns_credentials_paths_or_database_output() {
        let input = b"psql:/private/archive.sql: ERROR: role secret does not exist\nDETAIL: password=secret TLS-KEY secret\nLINE 1: INSERT secret /private/key.pem\n";
        let message = diagnostic(input);
        assert!(message.contains("role does not exist"));
        for forbidden in ["secret", "/private", "TLS-KEY", "INSERT"] {
            assert!(!message.contains(forbidden));
        }
    }

    #[test]
    fn connection_database_is_one_escaped_conninfo_value() {
        let StoredConnection::PostgreSQL(mut connection) =
            crate::commands::pg_objects::tests::connection("id", crate::SafeMode::Disabled, false)
        else {
            unreachable!()
        };
        connection.database = "db' host=attacker\\tail".into();
        let (command, _material) = connection_command(&connection, Path::new("psql")).unwrap();
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let dbname = args
            .windows(2)
            .find_map(|args| (args[0] == "--dbname").then_some(args[1].as_str()))
            .unwrap();
        assert_eq!(dbname, "dbname='db\\' host=attacker\\\\tail'");
    }

    #[test]
    fn successful_reap_status_preserves_irreversible_restore_success() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "postgres::backup::tests::native_child_helper",
                "--ignored",
            ])
            .env("DBUNK_PG_TOOL_HELPER_MODE", "success")
            .status();
        assert!(reaped_irreversible_success(true, &status));
        assert!(!reaped_irreversible_success(false, &status));
    }

    #[test]
    fn all_postgres_tools_require_the_patched_client_floor() {
        for tool in ["pg_dump", "psql", "pg_restore"] {
            for version in ["14.24", "15.19", "16.15", "17.11", "18.6", "19.1"] {
                let banner = format!("{tool} (PostgreSQL) {version}");
                assert!(
                    postgres_client_has_required_patches(tool, &banner),
                    "{banner}"
                );
            }
            for version in ["13.23", "14.23", "15.18", "16.14", "17.10", "18.5"] {
                let banner = format!("{tool} (PostgreSQL) {version}");
                assert!(
                    !postgres_client_has_required_patches(tool, &banner),
                    "{banner}"
                );
            }
        }
    }

    #[test]
    fn postgres_client_floor_rejects_wrong_tool_banners() {
        for expected_tool in ["pg_dump", "psql", "pg_restore"] {
            for banner_tool in ["pg_dump", "psql", "pg_restore"] {
                let banner = format!("{banner_tool} (PostgreSQL) 16.15");
                assert_eq!(
                    postgres_client_has_required_patches(expected_tool, &banner),
                    expected_tool == banner_tool,
                    "expected {expected_tool}, received {banner}"
                );
            }
            for banner in [
                format!("{expected_tool} (PostgreSQL) invalid"),
                format!("{expected_tool} 16.15"),
            ] {
                assert!(
                    !postgres_client_has_required_patches(expected_tool, &banner),
                    "{banner}"
                );
            }
        }
    }

    #[tokio::test]
    async fn source_stream_failure_keeps_stdin_open_until_process_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let (writer, mut reader) = tokio::io::duplex(16);
        let result = stream_plain_restore(directory.path().join("missing.sql"), writer).await;
        let PlainStreamResult::Failed { writer, .. } = result else {
            panic!("missing source must fail");
        };
        let mut byte = [0_u8];
        assert!(
            tokio::time::timeout(Duration::from_millis(20), reader.read(&mut byte))
                .await
                .is_err(),
            "failed input must not deliver a clean EOF to psql"
        );
        drop(writer);
        assert_eq!(reader.read(&mut byte).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn plain_stream_removes_only_a_canonical_pg_dump_guard_pair() {
        let source = b"--\n-- PostgreSQL database dump\n--\n\n\\restrict SourceKey123\n\nCOPY public.items (value) FROM stdin;\n\\! is copy data\n\\copy PROGRAM is copy data\n\\.\n\\i /private/hostile.sql\n\\connect attacker\n\n--\n-- PostgreSQL database dump complete\n--\n\n\\unrestrict SourceKey123\n\n";
        let expected = b"--\n-- PostgreSQL database dump\n--\n\n\nCOPY public.items (value) FROM stdin;\n\\! is copy data\n\\copy PROGRAM is copy data\n\\.\n\\i /private/hostile.sql\n\\connect attacker\n\n--\n-- PostgreSQL database dump complete\n--\n\n";
        assert_eq!(filtered_plain_input(source).await, expected);
    }

    #[tokio::test]
    async fn plain_stream_preserves_copy_data_and_fails_closed_on_extra_guards() {
        let copy_data =
            b"COPY public.items (value) FROM stdin;\n\\restrict copydata\n\\.\nSELECT 1;\n";
        assert_eq!(filtered_plain_input(copy_data).await, copy_data);

        let multiple = b"--\n-- PostgreSQL database dump\n--\n\n\\restrict First\n\\restrict Second\nSELECT 1;\n\\unrestrict Second\n\\unrestrict First\n\n";
        assert_eq!(
            filtered_plain_input(multiple).await,
            b"--\n-- PostgreSQL database dump\n--\n\n\\restrict Second\nSELECT 1;\n\\unrestrict Second\n"
        );

        let mismatched = b"--\n-- PostgreSQL database dump\n--\n\n\\restrict First\nSELECT 1;\n\\unrestrict Wrong\n\n";
        assert_eq!(
            filtered_plain_input(mismatched).await,
            b"--\n-- PostgreSQL database dump\n--\n\nSELECT 1;\n\\unrestrict Wrong\n\n"
        );
    }
}
