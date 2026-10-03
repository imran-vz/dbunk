//! Ignored fixture-only integration tests of Host -> services -> PostgreSQL ->
//! mailbox -> result reducer. These exercise no GPUI window and are not UI E2E.

#[path = "workspace_live_tests.rs"]
mod workspace_live_tests;

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use dbunk_lib::backend::{
    AckPayload, Backend, ExecutePayload, ExecutionPayload, QueryEvent, QueryEventEnvelope,
};
use futures_util::FutureExt as _;

use crate::controller::{Command, Controls, Host};
use crate::mailbox::{self, Message, Receiver};
use crate::results::{ResultModel, TerminalStatus};

static LIVE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const WAIT: Duration = Duration::from_secs(20);

struct Profile {
    path: PathBuf,
    marker: Vec<u8>,
}
impl Profile {
    fn new() -> Result<Self> {
        let id = uuid::Uuid::new_v4();
        let parent = std::env::temp_dir().canonicalize()?;
        let path = parent.join(format!("dbunk-native-live-{id}"));
        std::fs::create_dir(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
        let marker = serde_json::to_vec(&serde_json::json!({
            "version": 1, "fixture": "dbunk-native-stage03", "host": "127.0.0.1",
            "port": 15432, "database": "dbunk_demo", "profile_id": id.to_string(),
        }))?;
        std::fs::write(path.join(".dbunk-native-stage03"), &marker)?;
        Ok(Self { path, marker })
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        // Only remove this test's fresh private directory while its exact
        // ownership marker is intact. Never use the platform profile resolver.
        if std::fs::read(self.path.join(".dbunk-native-stage03"))
            .ok()
            .as_ref()
            == Some(&self.marker)
            && let Err(error) = std::fs::remove_dir_all(&self.path)
        {
            eprintln!(
                "Could not remove test profile {}: {error}",
                self.path.display()
            );
        }
    }
}

struct Fixture {
    host: Arc<Host>,
    controls: Controls,
    receiver: Receiver,
    session: String,
    sequence: u64,
    generation: Option<u64>,
    _profile: Profile,
}
impl Fixture {
    async fn new(capacity: usize) -> Result<Self> {
        let profile = Profile::new()?;
        let backend = Backend::open_fixture(&profile.path)
            .await
            .map_err(|error| anyhow!(error))?;
        let host = Host::new(backend, tokio::runtime::Handle::current());
        let (sender, receiver) = mailbox::channel(capacity, mailbox::QUEUE_BYTES);
        let session = uuid::Uuid::new_v4().to_string();
        let controls = host
            .connect(uuid::Uuid::new_v4().to_string(), session.clone(), sender)
            .map_err(|error| anyhow!(error))?;
        let mut fixture = Self {
            host,
            controls,
            receiver,
            session,
            sequence: 0,
            generation: None,
            _profile: profile,
        };
        if let Err(error) = fixture.ready().await {
            let cleanup = fixture.host.shutdown().await;
            return Err(error.context(format!("open cleanup: {cleanup:?}")));
        }
        Ok(fixture)
    }

    async fn message(&self) -> Result<Message> {
        tokio::time::timeout(WAIT, async {
            loop {
                if let Some(failure) = self.receiver.failure() {
                    bail!("{failure}");
                }
                if let Some(message) = self.receiver.receive() {
                    return Ok(message);
                }
                self.receiver
                    .wake
                    .recv()
                    .await
                    .context("mailbox wake closed")?;
            }
        })
        .await
        .context("native event timed out")?
    }

    fn admit(&mut self, event: &QueryEventEnvelope) -> Result<()> {
        ensure!(
            event.session_id == self.session,
            "stale session reached current receiver"
        );
        ensure!(
            event.connection_id == self.host.backend.fixture().id,
            "foreign connection"
        );
        ensure!(event.tab_id == "query", "foreign tab");
        ensure!(event.sequence == self.sequence + 1, "event sequence gap");
        if let Some(generation) = self.generation {
            ensure!(generation == event.generation, "stale generation");
        }
        self.generation = Some(event.generation);
        self.sequence = event.sequence;
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

    async fn ready(&mut self) -> Result<()> {
        loop {
            match self.message().await? {
                Message::Ready => return Ok(()),
                Message::Event(event) => {
                    self.admit(&event)?;
                    self.acknowledge(&event, true);
                }
                message => bail!("unexpected open message: {message:?}"),
            }
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

    /// Completion is usable only after the controller reports terminal ACK
    /// success. Waiting for this is what makes the next Run a real busy-gate test.
    async fn finish(&mut self, execution: &str, mut model: ResultModel) -> Result<ResultModel> {
        let mut terminal = None;
        loop {
            match self.message().await? {
                Message::Event(event) => {
                    self.admit(&event)?;
                    if let Some(id) = &event.execution_id {
                        ensure!(id == execution, "stale execution");
                    }
                    if matches!(&event.event, QueryEvent::ExecutionCompleted { .. }) {
                        terminal = Some(event.sequence);
                    }
                    if matches!(
                        &event.event,
                        QueryEvent::SessionLost { .. } | QueryEvent::SessionClosed
                    ) {
                        bail!("session lost during result drain");
                    }
                    // ACK decision follows the same reducer admission as GPUI.
                    let session_id = event.session_id.clone();
                    let execution_id = event.execution_id.clone();
                    let sequence = event.sequence;
                    let requires_ack = event.requires_ack;
                    let retain_more_rows = model.consume(event.event);
                    if requires_ack && let Some(execution_id) = execution_id {
                        self.controls.acknowledge(AckPayload {
                            session_id,
                            execution_id,
                            ack_through_sequence: sequence,
                            retain_more_rows,
                        });
                    }
                }
                Message::Acked {
                    execution: id,
                    sequence,
                } => {
                    if id == execution && terminal.is_some_and(|terminal| sequence >= terminal) {
                        return Ok(model);
                    }
                }
                Message::Rejected { message, .. } => bail!("execution refused: {message}"),
                Message::CancelFailed { message, .. } => bail!("cancellation refused: {message}"),
                Message::Ready => bail!("unexpected reconnect"),
                Message::HistoryFailed(message) => bail!("history persistence failed: {message}"),
                Message::Review { .. } | Message::Transaction { .. } => {
                    bail!("unexpected query control response")
                }
            }
        }
    }

    async fn query(&mut self, sql: &str) -> Result<ResultModel> {
        let execution = self.run(sql)?;
        self.finish(&execution, ResultModel::default()).await
    }

    async fn wait_started(&mut self, execution: &str) -> Result<()> {
        loop {
            match self.message().await? {
                Message::Event(event) => {
                    self.admit(&event)?;
                    ensure!(
                        event
                            .execution_id
                            .as_deref()
                            .is_none_or(|id| id == execution),
                        "stale execution"
                    );
                    self.acknowledge(&event, true);
                    if matches!(event.event, QueryEvent::ExecutionStarted) {
                        return Ok(());
                    }
                }
                Message::Acked { .. } => {}
                message => bail!("unexpected execution admission: {message:?}"),
            }
        }
    }

    async fn reconnect(&mut self, capacity: usize) -> Result<()> {
        self.controls.stop();
        let (sender, receiver) = mailbox::channel(capacity, mailbox::QUEUE_BYTES);
        self.session = uuid::Uuid::new_v4().to_string();
        self.controls = self
            .host
            .connect(
                uuid::Uuid::new_v4().to_string(),
                self.session.clone(),
                sender,
            )
            .map_err(|error| anyhow!(error))?;
        self.receiver = receiver;
        self.sequence = 0;
        self.generation = None;
        self.ready().await
    }
}

async fn backend_count() -> Result<u64> {
    tokio::task::spawn_blocking(|| {
        let script =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/native/fixture.py");
        let output = std::process::Command::new("python3")
            .arg(script)
            .arg("count")
            .output()?;
        ensure!(
            output.status.success(),
            "fixture ownership check/count failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?.trim().parse()?)
    })
    .await?
}

type CaseFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

async fn run_case(case: impl for<'a> FnOnce(&'a mut Fixture) -> CaseFuture<'a>) -> Result<()> {
    ensure!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref() == Ok("1"),
        "run tools/native/fixture.py check, then explicitly set DBUNK_NATIVE_FIXTURE_VERIFIED=1"
    );
    let _serial = LIVE_LOCK.lock().await;
    let baseline = backend_count().await?;
    let mut fixture = Fixture::new(mailbox::QUEUE_CAPACITY).await?;
    let result = std::panic::AssertUnwindSafe(case(&mut fixture))
        .catch_unwind()
        .await;
    let shutdown = tokio::time::timeout(Duration::from_secs(6), fixture.host.shutdown())
        .await
        .context("shutdown exceeded total native budget")?;
    shutdown.map_err(|error| anyhow!(error))?;
    drop(fixture);
    tokio::time::timeout(Duration::from_secs(7), async {
        loop {
            if backend_count().await? == baseline {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("fixture backend count did not return to baseline")??;
    match result {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly verified owned stage03 PostgreSQL fixture"]
async fn native_select_error_and_terminal_ack_allow_next_run() -> Result<()> {
    run_case(|fixture| Box::pin(async move {
        let result = fixture.query("SELECT NULL::text, ''::text, 9007199254740993::bigint::text, '雪''quote'::text; SELECT 1 AS empty_result WHERE false;").await?;
        ensure!(result.sets.len() == 2 && result.sets[1].rows.is_empty() && result.sets[1].columns.len() == 1, "script metadata lost");
        let row = &result.sets[0].rows[0];
        ensure!(row[0].is_none() && row[1].as_deref() == Some("") && row[2].as_deref() == Some("9007199254740993") && row[3].as_deref() == Some("雪'quote"), "exact values changed");
        let failed = fixture.query("SELECT 1 / 0;").await?;
        let completion = failed.completion.context("missing failed terminal")?;
        ensure!(completion.status == TerminalStatus::Failed && completion.error.as_ref().and_then(|error| error.code.as_deref()) == Some("22012"), "SQL error contract missing");
        let next = fixture.query("SELECT 'after error';").await?;
        ensure!(next.completion.is_some_and(|completion| completion.status == TerminalStatus::Completed), "next execution failed");
        Ok(())
    })).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly verified owned stage03 PostgreSQL fixture"]
async fn native_retention_refusal_drains_terminal_ack_and_preserves_session() -> Result<()> {
    run_case(|fixture| {
        Box::pin(async move {
            const BUDGET: usize = 512;
            const ROWS: u64 = 5_000;
            // This result fits all core limits. Only the injected native model
            // budget refuses retention, exercising retain_more_rows=false.
            let execution = fixture
                .run("SELECT i, repeat('x', 128) AS payload FROM generate_series(1, 5000) AS i;")?;
            let result = fixture
                .finish(&execution, ResultModel::with_byte_limit(BUDGET))
                .await?;
            ensure!(
                result.retention_limited,
                "native retention budget was not reached"
            );
            ensure!(
                result.retained_bytes <= BUDGET,
                "native retention exceeded its budget"
            );
            let set = result.sets.first().context("result metadata was lost")?;
            ensure!(
                !set.rows.is_empty(),
                "tiny budget retained no inspectable rows"
            );
            ensure!(
                set.row_count == Some(ROWS) && !set.partial,
                "server result did not drain to completion"
            );
            ensure!(
                result.native_omitted_rows > 0,
                "native omissions were not recorded"
            );
            let delivered_rows = set.rows.len() as u64 + result.native_omitted_rows;
            ensure!(
                delivered_rows < ROWS,
                "service kept delivering every row after retention refusal"
            );
            let completion = result.completion.context("terminal result was lost")?;
            ensure!(
                completion.status == TerminalStatus::Completed,
                "retention refusal failed the execution"
            );
            ensure!(
                completion.omitted_rows == 0,
                "query unexpectedly reached a core row limit"
            );
            // finish returns only after the cumulative terminal ACK succeeds.
            // A subsequent result proves the service's busy gate was released.
            let next = fixture
                .query("SELECT 'after native retention refusal';")
                .await?;
            ensure!(
                next.sets[0].rows[0][0].as_deref() == Some("after native retention refusal"),
                "session did not recover after terminal ACK"
            );
            ensure!(
                !next.retention_limited,
                "new execution inherited retention refusal"
            );
            Ok(())
        })
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly verified owned stage03 PostgreSQL fixture"]
async fn native_stop_with_four_batches_of_held_credit_resumes_and_finishes() -> Result<()> {
    run_case(|fixture| {
        Box::pin(async move {
            let execution = fixture
                .run("SELECT i, repeat('x', 256) FROM generate_series(1, 10000000) AS i;")?;
            let mut model = ResultModel::default();
            let mut held = None;
            let mut batches = 0;
            while batches < 4 {
                match fixture.message().await? {
                    Message::Event(event) => {
                        fixture.admit(&event)?;
                        if matches!(event.event, QueryEvent::RowBatch { .. }) {
                            batches += 1;
                        }
                        if batches == 0 {
                            fixture.acknowledge(&event, true);
                        } else if event.requires_ack {
                            held = Some(event.sequence);
                        }
                        ensure!(
                            !matches!(event.event, QueryEvent::ExecutionCompleted { .. }),
                            "execution finished before credit was held"
                        );
                        model.consume(event.event);
                    }
                    Message::Acked { .. } => {}
                    message => bail!("unexpected held-credit message: {message:?}"),
                }
            }
            fixture
                .controls
                .send(Command::Cancel(ExecutionPayload {
                    session_id: fixture.session.clone(),
                    execution_id: execution.clone(),
                }))
                .map_err(|error| anyhow!(error))?;
            fixture.controls.acknowledge(AckPayload {
                session_id: fixture.session.clone(),
                execution_id: execution.clone(),
                ack_through_sequence: held.context("row batch did not require ACK")?,
                retain_more_rows: true,
            });
            let result = fixture.finish(&execution, model).await?;
            ensure!(
                result
                    .completion
                    .is_some_and(|completion| completion.status == TerminalStatus::Cancelled),
                "Stop did not settle as cancelled"
            );
            fixture.query("SELECT 'after cancellation';").await?;
            Ok(())
        })
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly verified owned stage03 PostgreSQL fixture"]
async fn native_full_queue_retires_and_explicit_reconnect_restores_queries() -> Result<()> {
    run_case(|fixture| {
        Box::pin(async move {
            fixture.reconnect(2).await?;
            fixture.run("SELECT i FROM generate_series(1, 100000) AS i;")?;
            tokio::time::timeout(WAIT, async {
                loop {
                    if fixture.receiver.failure().is_some() {
                        break;
                    }
                    fixture.receiver.wake.recv().await?;
                }
                Ok::<_, anyhow::Error>(())
            })
            .await
            .context("queue never saturated")??;
            ensure!(
                fixture.receiver.failure() == Some(mailbox::Failure::Full),
                "wrong saturation failure"
            );
            fixture.reconnect(mailbox::QUEUE_CAPACITY).await?;
            let result = fixture.query("SELECT 'reconnected';").await?;
            ensure!(
                result.sets[0].rows[0][0].as_deref() == Some("reconnected"),
                "reconnect result missing"
            );
            Ok(())
        })
    })
    .await
}

/// Fault injection lives in the fixture harness, outside the service boundary.
/// Normal native calls still send confirmed=false and cannot terminate sessions.
async fn terminate_owned_backend(pid: u32, started: &str) -> Result<()> {
    let query = format!(
        "SELECT pg_terminate_backend(pid)::text FROM pg_stat_activity WHERE pid = {pid} AND backend_start = '{}'::timestamptz AND datname = 'dbunk_demo' AND usename = 'dbunk';",
        started.replace('\'', "''")
    );
    tokio::task::spawn_blocking(move || {
        use std::io::Write as _;
        use std::process::Stdio;
        let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/native");
        let mut child = std::process::Command::new("python3")
            .arg("-c")
            .arg("import sys; sys.path.insert(0, sys.argv[1]); import fixture; owned, container = fixture.check(); print(fixture.sql(container, sys.stdin.read()))")
            .arg(scripts)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().context("fixture input unavailable")?.write_all(query.as_bytes())?;
        let output = child.wait_with_output()?;
        ensure!(output.status.success(), "owned fault injection failed: {}", String::from_utf8_lossy(&output.stderr));
        ensure!(String::from_utf8(output.stdout)?.trim() == "true", "exact owned backend was not terminated");
        Ok(())
    }).await?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly verified owned stage03 PostgreSQL fixture"]
async fn native_idle_and_active_owned_socket_loss_require_reconnect() -> Result<()> {
    run_case(|fixture| Box::pin(async move {
        for active in [false, true] {
            let identity = fixture.query("SELECT pg_backend_pid()::text, backend_start::text FROM pg_stat_activity WHERE pid = pg_backend_pid();").await?;
            let row = &identity.sets[0].rows[0];
            let pid: u32 = row[0].as_deref().context("missing own backend PID")?.parse()?;
            let started = row[1].as_deref().context("missing own backend start")?;
            if active { let execution = fixture.run("SELECT pg_sleep(30);")?; fixture.wait_started(&execution).await?; }
            // Matching both fields prevents PID reuse from touching another session.
            terminate_owned_backend(pid, started).await?;
            tokio::time::timeout(WAIT, async {
                loop {
                    if fixture.receiver.failure().is_some() { break; }
                    match fixture.receiver.receive() {
                        Some(Message::Event(event)) => {
                            fixture.admit(&event)?;
                            fixture.acknowledge(&event, true);
                            if matches!(event.event, QueryEvent::SessionLost { .. } | QueryEvent::SessionClosed) { break; }
                        }
                        Some(_) => {},
                        None => { fixture.receiver.wake.recv().await?; }
                    }
                }
                Ok::<_, anyhow::Error>(())
            }).await.context("owned socket loss was not observed")??;
            fixture.reconnect(mailbox::QUEUE_CAPACITY).await?;
            fixture.query("SELECT 'after socket loss';").await?;
        }
        Ok(())
    })).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly verified owned stage03 PostgreSQL fixture"]
async fn native_shutdown_during_sleep_joins_and_is_idempotent() -> Result<()> {
    run_case(|fixture| {
        Box::pin(async move {
            let execution = fixture.run("SELECT pg_sleep(30);")?;
            fixture.wait_started(&execution).await?;
            fixture
                .host
                .shutdown()
                .await
                .map_err(|error| anyhow!(error))?;
            fixture
                .host
                .shutdown()
                .await
                .map_err(|error| anyhow!(error))?;
            ensure!(
                fixture.run("SELECT 1;").is_err(),
                "shutdown admitted more work"
            );
            Ok(())
        })
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly verified owned stage03 PostgreSQL fixture"]
async fn native_shutdown_with_streaming_credit_held_joins() -> Result<()> {
    run_case(|fixture| {
        Box::pin(async move {
            let execution = fixture
                .run("SELECT i, repeat('x', 256) FROM generate_series(1, 10000000) AS i;")?;
            fixture.wait_started(&execution).await?;
            let mut rows = 0;
            while rows < 4 {
                match fixture.message().await? {
                    Message::Event(event) => {
                        fixture.admit(&event)?;
                        if matches!(event.event, QueryEvent::RowBatch { .. }) {
                            rows += 1;
                        }
                    }
                    Message::Acked { .. } => {}
                    message => bail!("unexpected stream message: {message:?}"),
                }
            }
            fixture
                .host
                .shutdown()
                .await
                .map_err(|error| anyhow!(error))?;
            Ok(())
        })
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires verified owned fixture; exercises the real 120-second ACK lease"]
async fn native_foreground_heartbeats_do_not_prevent_ack_timeout() -> Result<()> {
    run_case(|fixture| Box::pin(async move {
        fixture.controls.focus(true);
        let execution = fixture.run("SELECT i, repeat('x', 256) FROM generate_series(1, 10000000) AS i;")?;
        fixture.wait_started(&execution).await?;
        // Leave the bounded mailbox unconsumed. Host's independent ten-second
        // foreground heartbeat continues, but never grants row credit.
        eprintln!("Native live ACK lease: holding consumption for 125 seconds with foreground heartbeats");
        tokio::time::sleep(Duration::from_secs(125)).await;
        let mut row_batches = 0;
        tokio::time::timeout(WAIT, async {
            loop {
                match fixture.receiver.receive() {
                    Some(Message::Event(event)) => {
                        fixture.admit(&event)?;
                        match event.event {
                            QueryEvent::RowBatch { .. } => row_batches += 1,
                            QueryEvent::SessionLost { reason } => {
                                ensure!(reason == "ackTimeout", "expected ACK expiry, received {reason}");
                                return Ok::<_, anyhow::Error>(());
                            }
                            _ => {},
                        }
                    }
                    Some(_) => {},
                    None => {
                        if let Some(failure) = fixture.receiver.failure() {
                            bail!("local failure before core ACK-timeout event: {failure}");
                        }
                        fixture.receiver.wake.recv().await?;
                    }
                }
            }
        }).await.context("ACK lease did not expire within one monitor interval")??;
        ensure!(row_batches == 4, "expected four-batch credit bound, got {row_batches}");
        ensure!(fixture.receiver.high_water() <= mailbox::QUEUE_BYTES, "mailbox exceeded its budget");
        fixture.reconnect(mailbox::QUEUE_CAPACITY).await?;
        fixture.query("SELECT 'after ACK timeout';").await?;
        Ok(())
    })).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires verified owned fixture; exercises background beyond the real 120-second lease"]
async fn native_background_beyond_lease_and_refocus_stay_usable() -> Result<()> {
    run_case(|fixture| {
        Box::pin(async move {
            fixture.controls.focus(false);
            eprintln!(
                "Native live background lease: leaving the unfocused session idle for 135 seconds"
            );
            tokio::time::sleep(Duration::from_secs(135)).await;
            fixture
                .query("SELECT 'still usable in background';")
                .await?;
            fixture.controls.focus(true);
            // Cross a monitor interval after refocus. Resuming foreground activity
            // must renew the lease instead of immediately expiring the old timestamp.
            tokio::time::sleep(Duration::from_secs(12)).await;
            fixture.query("SELECT 'usable after refocus';").await?;
            Ok(())
        })
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly verified owned stage03 PostgreSQL fixture"]
async fn native_explain_and_analyze_use_query_session_complete_plan_output() -> Result<()> {
    run_case(|fixture| {
        Box::pin(async move {
            for analyze in [false, true] {
                let sql = crate::explain_view::draft(
                    "SELECT id FROM plan024.fixture_many WHERE id < 3",
                    analyze,
                )
                .map_err(|error| anyhow!(error))?;
                let mut result = fixture.query(&sql).await?;
                let budget = std::rc::Rc::new(std::cell::Cell::new(0));
                let plan =
                    crate::explain_view::PlanData::from_result(&result, &sql, 0, budget.clone())
                        .map_err(|error| anyhow!(error))?;
                ensure!(budget.get() > 0, "plan retention was not charged");
                drop(plan);
                ensure!(budget.get() == 0, "plan retention lease leaked");
                result.sets[0].partial = true;
                ensure!(
                    crate::explain_view::PlanData::from_result(&result, &sql, 0, budget).is_err(),
                    "partial plan was accepted"
                );
            }
            Ok(())
        })
    })
    .await
}
