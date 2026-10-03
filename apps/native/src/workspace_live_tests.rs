//! Ignored, owned-fixture tests of the real multi-document runtime and stream
//! admission. No GPUI window is exercised. Shares the stage03 ownership guard,
//! serialization lock and PostgreSQL teardown measurement with its parent.
use super::{LIVE_LOCK, WAIT, backend_count};
use crate::controller::{Command, Controls, Host};
use crate::mailbox::{self, Message, Receiver};
use crate::results::{ResultModel, TerminalStatus, encoded_size};
use crate::stream::Stream;
use anyhow::{Context as _, Result, anyhow, bail, ensure};
use dbunk_lib::backend::{
    AckPayload, Backend, DevelopmentEnvironment, DevelopmentFixtures,
    DevelopmentPostgresConnection, DevelopmentSafeMode, DevelopmentStorageMode,
    DevelopmentTlsOptions, ExecutePayload, ExecutionPayload, QueryEvent, QueryEventEnvelope,
};
use futures_util::FutureExt as _;
use std::{future::Future, path::PathBuf, pin::Pin, sync::Arc, time::Duration};

struct Document {
    tab: String,
    session: String,
    connection: String,
    controls: Controls,
    receiver: Receiver,
    stream: Stream,
    last_event: Option<QueryEventEnvelope>,
}

impl Document {
    async fn connect(host: &Host, connection: &str, tab: String) -> Result<Self> {
        let session = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = host.mailbox();
        let controls = host
            .connect_document(tab.clone(), connection.into(), session.clone(), sender)
            .map_err(|error| anyhow!(error))?;
        let mut document = Self {
            stream: Stream::for_document(session.clone(), connection.into(), tab.clone()),
            tab,
            session,
            connection: connection.into(),
            controls,
            receiver,
            last_event: None,
        };
        loop {
            match document.message().await? {
                Message::Ready => return Ok(document),
                Message::Event(event) => {
                    document.admit(&event, None)?;
                    document.acknowledge(&event, true);
                }
                message => bail!("Unexpected document-open message: {message:?}"),
            }
        }
    }

    async fn message(&self) -> Result<Message> {
        tokio::time::timeout(WAIT, async {
            loop {
                if let Some(failure) = self.receiver.failure() {
                    bail!("Document {} failed: {failure}", self.tab);
                }
                if let Some(message) = self.receiver.receive() {
                    return Ok(message);
                }
                self.receiver.wake.recv().await.context("Mailbox closed")?;
            }
        })
        .await
        .context("Workspace document event timed out")?
    }

    fn admit(&mut self, event: &QueryEventEnvelope, execution: Option<&str>) -> Result<()> {
        ensure!(
            event.tab_id == self.tab,
            "Event crossed document identities"
        );
        ensure!(
            event.connection_id == self.connection,
            "Event crossed connection bindings"
        );
        ensure!(
            self.stream
                .admit(event, execution)
                .map_err(|error| anyhow!(error))?,
            "Current workspace event was rejected as stale"
        );
        self.last_event = Some(event.clone());
        Ok(())
    }

    fn acknowledge(&self, event: &QueryEventEnvelope, retain_more_rows: bool) {
        if event.requires_ack
            && let Some(execution_id) = &event.execution_id
        {
            self.controls.acknowledge(AckPayload {
                session_id: self.session.clone(),
                execution_id: execution_id.clone(),
                ack_through_sequence: event.sequence,
                retain_more_rows,
            });
        }
    }

    fn run(&self, sql: &str) -> Result<String> {
        let execution_id = uuid::Uuid::new_v4().to_string();
        self.controls
            .send(Command::Run(ExecutePayload {
                session_id: self.session.clone(),
                execution_id: execution_id.clone(),
                sql: sql.into(),
                confirmed: false,
                parameters: None,
                row_limit: None,
            }))
            .map_err(|error| anyhow!(error))?;
        Ok(execution_id)
    }

    fn cancel(&self, execution: &str) -> Result<()> {
        self.controls
            .send(Command::Cancel(ExecutionPayload {
                session_id: self.session.clone(),
                execution_id: execution.into(),
            }))
            .map_err(|error| anyhow!(error))
    }

    async fn started(&mut self, execution: &str) -> Result<()> {
        loop {
            match self.message().await? {
                Message::Event(event) => {
                    self.admit(&event, Some(execution))?;
                    self.acknowledge(&event, true);
                    if matches!(event.event, QueryEvent::ExecutionStarted) {
                        return Ok(());
                    }
                }
                Message::Acked { .. } => {}
                message => bail!("Execution did not start: {message:?}"),
            }
        }
    }

    async fn finish(&mut self, execution: &str, mut model: ResultModel) -> Result<ResultModel> {
        let mut terminal = None;
        loop {
            match self.message().await? {
                Message::Event(event) => {
                    self.admit(&event, Some(execution))?;
                    ensure!(
                        !matches!(
                            event.event,
                            QueryEvent::SessionLost { .. } | QueryEvent::SessionClosed
                        ),
                        "Document was lost while draining"
                    );
                    if matches!(event.event, QueryEvent::ExecutionCompleted { .. }) {
                        terminal = Some(event.sequence);
                    }
                    let retain = model.consume(event.event.clone());
                    self.acknowledge(&event, retain);
                }
                Message::Acked {
                    execution: id,
                    sequence,
                } => {
                    if id == execution && terminal.is_some_and(|terminal| sequence >= terminal) {
                        return Ok(model);
                    }
                }
                message => bail!("Unexpected document execution message: {message:?}"),
            }
        }
    }

    async fn query(&mut self, sql: &str) -> Result<ResultModel> {
        let execution = self.run(sql)?;
        self.finish(&execution, ResultModel::default()).await
    }

    /// Hold the backend's four-batch credit independently in each document.
    async fn hold_credit(&mut self, execution: &str) -> Result<(ResultModel, u64)> {
        let mut model = ResultModel::default();
        let mut batches = 0;
        loop {
            match self.message().await? {
                Message::Event(event) => {
                    self.admit(&event, Some(execution))?;
                    if matches!(event.event, QueryEvent::RowBatch { .. }) {
                        batches += 1;
                    }
                    if batches == 0 {
                        self.acknowledge(&event, true);
                    }
                    ensure!(
                        !matches!(event.event, QueryEvent::ExecutionCompleted { .. }),
                        "Stream ended before held credit"
                    );
                    let sequence = event.sequence;
                    model.consume(event.event);
                    if batches == 4 {
                        return Ok((model, sequence));
                    }
                }
                Message::Acked { .. } => {}
                message => bail!("Unexpected held-credit message: {message:?}"),
            }
        }
    }

    fn release_credit(&self, execution: &str, sequence: u64) {
        self.controls.acknowledge(AckPayload {
            session_id: self.session.clone(),
            execution_id: execution.into(),
            ack_through_sequence: sequence,
            retain_more_rows: true,
        });
    }
}

type CaseFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

struct Profile {
    path: PathBuf,
    marker: Vec<u8>,
}
impl Drop for Profile {
    fn drop(&mut self) {
        if std::fs::read(self.path.join(".dbunk-native-stage04"))
            .ok()
            .as_ref()
            == Some(&self.marker)
            && let Err(error) = std::fs::remove_dir_all(&self.path)
        {
            eprintln!(
                "Could not remove owned test profile {}: {error}",
                self.path.display()
            );
        }
    }
}

async fn open_workspace() -> Result<(Profile, Backend, String)> {
    let manifest = tokio::task::spawn_blocking(|| -> Result<String> {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/native");
        let output = std::process::Command::new("python3")
            .current_dir(directory)
            .args(["-c", "import json, fixture, profile; owned, _ = fixture.check(); print(json.dumps(profile.manifest(owned)))"])
            .output()?;
        ensure!(output.status.success(), "Owned manifest verification failed: {}", String::from_utf8_lossy(&output.stderr));
        Ok(String::from_utf8(output.stdout)?)
    }).await??;
    let path = std::env::temp_dir()
        .canonicalize()?
        .join(format!("dbunk-workspace-live-{}", uuid::Uuid::new_v4()));
    let fixtures = DevelopmentFixtures::from_json(&manifest).map_err(|error| anyhow!(error))?;
    let backend = Backend::create_development(&path, fixtures)
        .await
        .map_err(|error| anyhow!(error))?;
    let profile = Profile {
        marker: std::fs::read(path.join(".dbunk-native-stage04"))?,
        path,
    };
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await
        .map_err(|error| anyhow!(error))?;
    let saved = backend
        .save_development_connection(
            None,
            DevelopmentPostgresConnection {
                name: "Owned runtime test".into(),
                host: "127.0.0.1".into(),
                port: 15432,
                database: "dbunk_demo".into(),
                user: "dbunk".into(),
                environment: DevelopmentEnvironment::Development,
                safe_mode: DevelopmentSafeMode::Protected,
                read_only: false,
                tls: DevelopmentTlsOptions::default(),
                driver_options: Default::default(),
            },
            "dbunk".into(),
        )
        .await
        .map_err(|error| anyhow!(error))?;
    Ok((profile, backend, saved.id))
}

async fn run_case(
    name: &'static str,
    case: impl for<'a> FnOnce(&'a Arc<Host>, &'a str) -> CaseFuture<'a>,
) -> Result<()> {
    ensure!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref() == Ok("1"),
        "Verify the owned fixture and explicitly set DBUNK_NATIVE_FIXTURE_VERIFIED=1"
    );
    let _serial = LIVE_LOCK.lock().await;
    // The native backend deliberately fences one profile per process. Each
    // case runs in a fresh process while the parent retains the fixture lock.
    if std::env::var("DBUNK_WORKSPACE_TEST_CHILD").as_deref() != Ok(name) {
        let output = tokio::task::spawn_blocking(move || {
            std::process::Command::new(std::env::current_exe()?)
                .env("DBUNK_WORKSPACE_TEST_CHILD", name)
                .args([name, "--ignored", "--nocapture", "--test-threads=1"])
                .output()
        })
        .await??;
        print!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        ensure!(
            output.status.success(),
            "Workspace case subprocess failed: {name}"
        );
        return Ok(());
    }
    let baseline = backend_count().await?;
    let (profile, backend, connection) = open_workspace().await?;
    let host = Host::new_workspace(
        backend,
        tokio::runtime::Handle::current(),
        uuid::Uuid::new_v4().to_string(),
    );
    let result = std::panic::AssertUnwindSafe(case(&host, &connection))
        .catch_unwind()
        .await;
    let cleanup = tokio::time::timeout(Duration::from_secs(6), host.shutdown())
        .await
        .context("Workspace exceeded one total shutdown budget")?;
    cleanup.map_err(|error| anyhow!(error))?;
    drop(host);
    drop(profile);
    tokio::time::timeout(Duration::from_secs(7), async {
        loop {
            if backend_count().await? == baseline {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("Workspace PostgreSQL backend count did not return to baseline")??;
    match result {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly verified owned stage03 PostgreSQL fixture"]
async fn workspace_streams_cancel_and_close_independently_with_terminal_ack() -> Result<()> {
    run_case(
        "workspace_streams_cancel_and_close_independently_with_terminal_ack",
        |host, connection| {
            Box::pin(async move {
                let mut first =
                    Document::connect(host, connection, uuid::Uuid::new_v4().to_string()).await?;
                let mut second =
                    Document::connect(host, connection, uuid::Uuid::new_v4().to_string()).await?;
                let first_pid = first.query("SELECT pg_backend_pid()::text;").await?;
                let second_pid = second.query("SELECT pg_backend_pid()::text;").await?;
                ensure!(
                    first_pid.sets[0].rows[0][0] != second_pid.sets[0].rows[0][0],
                    "Documents shared a PostgreSQL session"
                );
                let sql = "SELECT i, repeat('x', 256) FROM generate_series(1, 10000000) AS i;";
                let a = first.run(sql)?;
                let b = second.run(sql)?;
                let (a_model, a_sequence) = first.hold_credit(&a).await?;
                let (b_model, b_sequence) = second.hold_credit(&b).await?;
                first.cancel(&a)?;
                first.release_credit(&a, a_sequence);
                let cancelled = first.finish(&a, a_model).await?;
                ensure!(
                    cancelled
                        .completion
                        .is_some_and(|value| value.status == TerminalStatus::Cancelled),
                    "First cancellation did not settle"
                );
                let next = first
                    .query("SELECT 'terminal ACK released first document';")
                    .await?;
                ensure!(
                    next.sets[0].rows[0][0].as_deref()
                        == Some("terminal ACK released first document"),
                    "Terminal ACK did not release Run"
                );
                host.disconnect_document(&first.tab)
                    .await
                    .map_err(|error| anyhow!(error))?;
                ensure!(
                    host.backend
                        .session_alive(crate::controller::WINDOW, &second.session)
                        .await
                        .map_err(|error| anyhow!("{error:?}"))?,
                    "Closing first document retired the second session"
                );
                second.cancel(&b)?;
                second.release_credit(&b, b_sequence);
                let cancelled = second.finish(&b, b_model).await?;
                ensure!(
                    cancelled
                        .completion
                        .is_some_and(|value| value.status == TerminalStatus::Cancelled),
                    "Peer stream was cancelled or closed by another document"
                );
                second.query("SELECT 'peer still usable';").await?;
                ensure!(
                    first.receiver.high_water() <= mailbox::QUEUE_BYTES
                        && second.receiver.high_water() <= mailbox::QUEUE_BYTES,
                    "Per-document queue exceeded its byte limit"
                );
                Ok(())
            })
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly verified owned stage03 PostgreSQL fixture"]
async fn workspace_admission_reconnect_stale_events_and_global_shutdown() -> Result<()> {
    run_case(
        "workspace_admission_reconnect_stale_events_and_global_shutdown",
        |host, connection| {
            Box::pin(async move {
                let mut documents = Vec::new();
                for _ in 0..4 {
                    documents.push(
                        Document::connect(host, connection, uuid::Uuid::new_v4().to_string())
                            .await?,
                    );
                }
                let (extra, _receiver) = host.mailbox();
                let refusal = host
                    .connect_document(
                        uuid::Uuid::new_v4().to_string(),
                        connection.into(),
                        uuid::Uuid::new_v4().to_string(),
                        extra,
                    )
                    .err()
                    .context("Fifth session was admitted")?;
                ensure!(
                    refusal.contains("Four sessions"),
                    "Wrong session admission refusal"
                );
                let a = documents[0].run("SELECT pg_sleep(30);")?;
                documents[0].started(&a).await?;
                let b = documents[1].run("SELECT pg_sleep(30);")?;
                documents[1].started(&b).await?;
                let third = documents[2].run("SELECT 'must be refused';")?;
                loop {
                    match documents[2].message().await? {
                        Message::Rejected { execution, message } => {
                            ensure!(
                                execution == third && message.contains("Two queries"),
                                "Third execution refusal lost its identity"
                            );
                            break;
                        }
                        Message::Acked { .. } => {}
                        message => bail!("Third execution was admitted: {message:?}"),
                    }
                }
                documents[0].cancel(&a)?;
                documents[0].finish(&a, ResultModel::default()).await?;
                documents[2]
                    .query("SELECT 'execution permit returned';")
                    .await?;
                let old_event = documents[2]
                    .last_event
                    .clone()
                    .context("No old document event")?;
                let old_controls = documents[2].controls.clone();
                let old_session = documents[2].session.clone();
                let tab = documents[2].tab.clone();
                let mut replacement = Document::connect(host, connection, tab).await?;
                ensure!(
                    !replacement
                        .stream
                        .admit(&old_event, None)
                        .map_err(|error| anyhow!(error))?,
                    "Stale event entered reconnected stream"
                );
                ensure!(
                    old_controls
                        .send(Command::Run(ExecutePayload {
                            session_id: old_session,
                            execution_id: uuid::Uuid::new_v4().to_string(),
                            sql: "SELECT 'stale';".into(),
                            confirmed: false,
                            parameters: None,
                            row_limit: None
                        }))
                        .is_err(),
                    "Retired document worker still accepted Run"
                );
                replacement.query("SELECT 'new session';").await?;
                ensure!(
                    host.backend
                        .session_alive(crate::controller::WINDOW, &documents[1].session)
                        .await
                        .map_err(|error| anyhow!("{error:?}"))?,
                    "Reconnect retired the other active stream"
                );
                documents[2] = replacement;
                let started = std::time::Instant::now();
                host.shutdown().await.map_err(|error| anyhow!(error))?;
                ensure!(
                    started.elapsed() < Duration::from_secs(6),
                    "Shutdown used a per-tab timeout"
                );
                host.shutdown().await.map_err(|error| anyhow!(error))?;
                ensure!(
                    documents[0].run("SELECT 'after shutdown';").is_err(),
                    "Global shutdown admitted more work"
                );
                Ok(())
            })
        },
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly verified owned stage03 PostgreSQL fixture"]
async fn workspace_mailboxes_share_exact_16_mib_and_release_on_drop() -> Result<()> {
    run_case(
        "workspace_mailboxes_share_exact_16_mib_and_release_on_drop",
        |host, _| {
            Box::pin(async move {
                // Synthetic transport pressure exercises Host's actual shared budget;
                // it makes no claim that PostgreSQL generates these control messages.
                let message = || {
                    let mut value = Message::Rejected {
                        execution: "budget".into(),
                        message: String::new(),
                    };
                    let overhead = encoded_size(&value);
                    if let Message::Rejected { message, .. } = &mut value {
                        *message = "x".repeat(mailbox::QUEUE_BYTES - overhead);
                    }
                    value
                };
                let (first, first_rx) = host.mailbox();
                let (second, second_rx) = host.mailbox();
                let (third, third_rx) = host.mailbox();
                ensure!(
                    encoded_size(&message()) == mailbox::QUEUE_BYTES,
                    "Pressure message size differs"
                );
                ensure!(
                    first.send(message()).is_ok() && second.send(message()).is_ok(),
                    "16 MiB total was refused prematurely"
                );
                ensure!(
                    third.send(Message::Ready).is_err()
                        && third_rx.failure() == Some(mailbox::Failure::Full),
                    "Workspace admitted bytes above shared16MiB"
                );
                drop(first_rx);
                let (next, next_rx) = host.mailbox();
                ensure!(
                    next.send(message()).is_ok(),
                    "Dropping a document did not return shared byte permits"
                );
                drop(second_rx);
                drop(third_rx);
                drop(next_rx);
                Ok(())
            })
        },
    )
    .await
}
