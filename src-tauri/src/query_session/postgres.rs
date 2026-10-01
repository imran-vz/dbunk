use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::StreamExt;
use tokio::sync::{mpsc, Mutex};
use tokio_postgres::error::SqlState;
use tokio_postgres::types::private::BytesMut;
use tokio_postgres::types::{to_sql_checked, Format, IsNull, ToSql, Type};
use tokio_postgres::{Client, SimpleQueryMessage};

use super::protocol::{QueryDatabaseError, QuerySessionError, RowLimitOutcome};
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::postgres::dedicated::{self, DedicatedConnection, DedicatedError, NoticeSink};
use crate::postgres::sql_params::{BoundValues, ExecutionPlan, ExecutionShape};

pub(crate) use crate::postgres::dedicated::Notice;
pub(crate) use crate::postgres::row_budget::{shrink_row, truncate_utf8};

pub(crate) struct SessionConnection {
    inner: DedicatedConnection,
    pub pid: i32,
    pub backend_start: String,
    pub notices: Arc<Mutex<mpsc::Receiver<Notice>>>,
    dropped_notices: Arc<AtomicU32>,
}

impl std::ops::Deref for SessionConnection {
    type Target = DedicatedConnection;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub(crate) async fn connect(
    spec: &ResolvedPostgresConnectSpec,
) -> Result<SessionConnection, QuerySessionError> {
    let (notice_tx, notice_rx) = mpsc::channel(500);
    let dropped_notices = Arc::new(AtomicU32::new(0));
    let inner = dedicated::connect(
        spec,
        NoticeSink::Bounded {
            tx: notice_tx,
            dropped: dropped_notices.clone(),
        },
    )
    .await
    .map_err(map_dedicated)?;
    let identity = inner
        .client
        .query_one(
            "SELECT pg_backend_pid(), backend_start::text FROM pg_stat_activity WHERE pid = pg_backend_pid()",
            &[],
        )
        .await
        .map_err(|error| map_dedicated(dedicated::database_error(error)))?;
    Ok(SessionConnection {
        inner,
        pid: identity.get(0),
        backend_start: identity.get(1),
        notices: Arc::new(Mutex::new(notice_rx)),
        dropped_notices,
    })
}

fn map_dedicated(error: DedicatedError) -> QuerySessionError {
    match error {
        DedicatedError::ConnectionLost => QuerySessionError::ConnectionLost,
        DedicatedError::Timeout { operation } => QuerySessionError::Timeout { operation },
        DedicatedError::Tls { kind, message } => QuerySessionError::TlsFailed {
            tls_kind: kind,
            message,
        },
        DedicatedError::Database {
            code,
            message,
            severity,
            position,
        } => QuerySessionError::Database {
            code,
            message,
            severity,
            position,
        },
    }
}

impl SessionConnection {
    pub(crate) fn take_dropped_notices(&self) -> u32 {
        self.dropped_notices.swap(0, Ordering::Relaxed)
    }
}

#[derive(Debug, Default)]
pub(crate) struct ExecutionTotals {
    pub omitted_rows: u64,
    pub omitted_result_sets: u32,
    pub omitted_notices: u32,
    pub omitted_metadata_bytes: u64,
    pub truncation_reasons: Vec<String>,
    retained_metadata_bytes: usize,
}

/// How an execution ended, as far as the driver can tell.
#[derive(Debug)]
pub(crate) enum Outcome {
    Completed,
    Failed(QuerySessionError),
    /// A requested Stop was honored between statements.
    Stopped,
    /// Nothing ran: the statement cannot be executed in its shape.
    Refused(&'static str),
    /// The session closed. Nothing further was sent to the server.
    Abandoned,
}

#[derive(Debug)]
pub(crate) enum DriverEvent {
    ResultStarted {
        result_set_index: u32,
        columns: Vec<Option<String>>,
    },
    RowBatch {
        result_set_index: u32,
        rows: Vec<Vec<Option<String>>>,
    },
    ResultCompleted {
        result_set_index: u32,
        row_count: u64,
        limit: Option<RowLimitOutcome>,
    },
    ResultAborted {
        result_set_index: u32,
        row_count: u64,
    },
    Notice(Notice),
    /// Always the last event, and always after any cleanup statement.
    Finished {
        totals: ExecutionTotals,
        outcome: Outcome,
    },
}

/// How the session's transaction state admits one execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TransactionEntry {
    /// Autocommit and idle. A cursor read gets its own wrapper transaction;
    /// anything else runs in the server's implicit one.
    Autocommit,
    /// Manual mode and idle: this statement opens the user's transaction.
    Begin(String),
    /// A transaction is already open or failed; run inside it.
    Inside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Checkpoint {
    Proceed,
    /// A Stop was requested for this execution.
    Stop,
    Closed,
}

/// Held while cleanup statements are sent. The session cannot close until it
/// is dropped, so nothing is ever sent after a close. No event may be emitted
/// while it is held: delivering one needs the same lock.
pub(crate) struct CleanupPermit<'a> {
    pub stop_requested: bool,
    pub _open: Box<dyn Send + 'a>,
}

/// What a statement-scoped execution asks its session between statements.
pub(crate) trait ExecutionControl: Send + Sync {
    /// Asked before each statement is sent.
    fn checkpoint(&self) -> BoxFuture<'_, Checkpoint>;
    /// Ends the cancel window, so a late Stop cannot hit a cleanup statement.
    /// `None` when the session has closed.
    fn begin_cleanup(&self) -> BoxFuture<'_, Option<CleanupPermit<'_>>>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum RowLimit {
    #[default]
    None,
    /// Retain at most this many rows per Result Set and keep reading.
    Retain(u32),
    /// The request asked for one row more than this. That row only proves
    /// more exist; it is dropped without being counted.
    Probe(u32),
}

const DECLARE_PREFIX: &str = "DECLARE dbunk_query_cursor NO SCROLL CURSOR FOR ";
const CLOSE_CURSOR: &str = "CLOSE dbunk_query_cursor";
const COMMIT_CURSOR: &str = "CLOSE dbunk_query_cursor; COMMIT";
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);
/// More than one FETCH can emit: a Result Set retains at most 10,000 rows,
/// so at most that many batches, plus its start, its end, and 500 notices.
const FETCH_EVENT_CAPACITY: usize = 16_384;
const PARAMETERS_RETURN_ROWS: &str = "parametersReturnRows";

/// A value sent in text format; the server infers its type from context.
/// `Debug` never prints it, because the driver logs parameters at Debug.
struct TextParameter<'a>(&'a Option<String>);

impl fmt::Debug for TextParameter<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TextParameter(<redacted>)")
    }
}

impl ToSql for TextParameter<'_> {
    fn to_sql(
        &self,
        _type: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        Ok(match self.0 {
            Some(value) => {
                out.extend_from_slice(value.as_bytes());
                IsNull::No
            }
            None => IsNull::Yes,
        })
    }

    fn accepts(_type: &Type) -> bool {
        true
    }

    fn encode_format(&self, _type: &Type) -> Format {
        Format::Text
    }

    to_sql_checked!();
}

/// Runs one planned execution and reports it through a capacity-one handoff.
/// On the Script shape that makes frontend credit apply directly to polling
/// the driver: when the actor pauses, the driver and the TCP socket pause. A
/// cursor read finishes its FETCH and its cleanup first and only then hands
/// its rows over, so credit never holds a transaction open.
pub(crate) fn execute_plan(
    client: Arc<Client>,
    notices: Arc<Mutex<mpsc::Receiver<Notice>>>,
    plan: ExecutionPlan,
    entry: TransactionEntry,
    control: Arc<dyn ExecutionControl>,
) -> mpsc::Receiver<DriverEvent> {
    let (sender, receiver) = mpsc::channel(1);
    tokio::spawn(async move {
        let mut notices = notices.lock().await;
        let mut run = Run {
            client: &client,
            notices: &mut notices,
            control: &*control,
            plan: &plan,
            reducer: Reducer::new(sender, RowLimit::None),
        };
        let outcome = match &plan.shape {
            ExecutionShape::Script { sql, row_limit } => {
                run.reducer.limit = row_limit.map_or(RowLimit::None, RowLimit::Retain);
                run.script(sql, &entry).await
            }
            ExecutionShape::CursorRead {
                statement,
                values,
                row_limit,
            } => {
                run.reducer.limit = row_limit.map_or(RowLimit::None, RowLimit::Probe);
                run.reducer.hold_completion = true;
                run.cursor_read(statement, values, *row_limit, &entry).await
            }
            ExecutionShape::BoundCommand { statement, values } => {
                run.bound_command(statement, values, &entry).await
            }
        };
        // `None` means the receiver is gone and nobody is left to tell.
        if let Some(outcome) = outcome {
            run.reducer.finish(outcome).await;
        }
    });
    receiver
}

struct Run<'a> {
    client: &'a Client,
    notices: &'a mut mpsc::Receiver<Notice>,
    control: &'a dyn ExecutionControl,
    plan: &'a ExecutionPlan,
    reducer: Reducer,
}

/// Where a cursor read stopped before its cleanup.
enum CursorEnd {
    Fetched,
    Stopped,
    Failed {
        error: QuerySessionError,
        /// A server error has already aborted the surrounding transaction.
        server: bool,
    },
}

impl Run<'_> {
    async fn script(&mut self, sql: &str, entry: &TransactionEntry) -> Option<Outcome> {
        if let TransactionEntry::Begin(begin) = entry {
            if let Err(error) = self.client.batch_execute(begin).await {
                return Some(Outcome::Failed(database_error(error)));
            }
        }
        match reduce_simple(&mut self.reducer, self.client, self.notices, sql).await {
            Ok(()) => Some(Outcome::Completed),
            Err(Some(error)) => Some(Outcome::Failed(database_error(error))),
            Err(None) => None,
        }
    }

    async fn cursor_read(
        &mut self,
        statement: &str,
        values: &BoundValues,
        row_limit: Option<u32>,
        entry: &TransactionEntry,
    ) -> Option<Outcome> {
        let wrapper = *entry == TransactionEntry::Autocommit;
        let opening = match entry {
            // A cursor is planned for its first rows. Without a limit the
            // read runs to exhaustion, so plan it for all of them; the
            // setting ends with the wrapper transaction.
            TransactionEntry::Autocommit if row_limit.is_none() => {
                Some("BEGIN; SET LOCAL cursor_tuple_fraction = 1")
            }
            TransactionEntry::Autocommit => Some("BEGIN"),
            TransactionEntry::Begin(begin) => Some(begin.as_str()),
            TransactionEntry::Inside => None,
        };
        let mut opened = false;
        let mut declared = false;
        let mut fetched = None;
        let end = 'statements: {
            if let Some(opening) = opening {
                match self.control.checkpoint().await {
                    Checkpoint::Closed => return Some(Outcome::Abandoned),
                    Checkpoint::Stop => break 'statements CursorEnd::Stopped,
                    Checkpoint::Proceed => {}
                }
                opened = true;
                if let Err(error) = self.client.batch_execute(opening).await {
                    break 'statements self.failed(error, None);
                }
            }
            match self.control.checkpoint().await {
                Checkpoint::Closed => return Some(Outcome::Abandoned),
                Checkpoint::Stop => break 'statements CursorEnd::Stopped,
                Checkpoint::Proceed => {}
            }
            // One round trip and an unnamed statement: every type is left
            // unspecified, so the server infers it and nothing is prepared
            // that could outlive the execution.
            let parameters = values.0.iter().map(TextParameter).collect::<Vec<_>>();
            let typed = parameters
                .iter()
                .map(|parameter| (parameter as &(dyn ToSql + Sync), Type::UNKNOWN))
                .collect::<Vec<_>>();
            let declare = format!("{DECLARE_PREFIX}{statement}");
            if let Err(error) = self.client.execute_typed(&declare, &typed).await {
                break 'statements self.failed(error, Some(DECLARE_PREFIX.len() as u32));
            }
            declared = true;
            match self.control.checkpoint().await {
                Checkpoint::Closed => return Some(Outcome::Abandoned),
                Checkpoint::Stop => break 'statements CursorEnd::Stopped,
                Checkpoint::Proceed => {}
            }
            // One FETCH for the whole read: the server discards a cancel that
            // arrives between requests, so chunks would lose a Stop.
            let fetch = match row_limit {
                Some(limit) => format!("FETCH FORWARD {} FROM dbunk_query_cursor", limit + 1),
                None => "FETCH ALL FROM dbunk_query_cursor".into(),
            };
            // The result is read to its end into memory, not handed over
            // under frontend credit. What is kept is bounded by the retention
            // limits, and the transaction around the cursor can then end at
            // once instead of staying open for as long as a frontend takes
            // to acknowledge rows.
            let (buffer, receiver) = mpsc::channel(FETCH_EVENT_CAPACITY);
            fetched = Some(receiver);
            let live = std::mem::replace(&mut self.reducer.sender, buffer);
            let reduced = reduce_simple(&mut self.reducer, self.client, self.notices, &fetch).await;
            self.reducer.sender = live;
            match reduced {
                Ok(()) => CursorEnd::Fetched,
                Err(Some(error)) => self.failed(error, None),
                Err(None) => return None,
            }
        };

        let outcome = if wrapper {
            if !opened {
                // Stopped before BEGIN was sent: there is nothing to undo.
                Outcome::Stopped
            } else {
                let permit = self.control.begin_cleanup().await;
                let Some(permit) = permit else {
                    return Some(Outcome::Abandoned);
                };
                match end {
                    CursorEnd::Fetched if !permit.stop_requested => {
                        match cleanup(self.client, COMMIT_CURSOR).await {
                            Ok(()) => Outcome::Completed,
                            Err(error) => {
                                rollback(self.client).await;
                                Outcome::Failed(error)
                            }
                        }
                    }
                    CursorEnd::Fetched | CursorEnd::Stopped => {
                        rollback(self.client).await;
                        Outcome::Stopped
                    }
                    CursorEnd::Failed { error, .. } => {
                        rollback(self.client).await;
                        Outcome::Failed(error)
                    }
                }
            }
        } else {
            // The user owns this transaction. Only the cursor is ours, and a
            // server error has already closed it with the transaction.
            let cursor_open = declared && !matches!(end, CursorEnd::Failed { server: true, .. });
            if !cursor_open {
                match end {
                    CursorEnd::Failed { error, .. } => Outcome::Failed(error),
                    CursorEnd::Fetched | CursorEnd::Stopped => Outcome::Stopped,
                }
            } else {
                let permit = self.control.begin_cleanup().await;
                let Some(permit) = permit else {
                    return Some(Outcome::Abandoned);
                };
                let closed = cleanup(self.client, CLOSE_CURSOR).await;
                match (end, closed) {
                    (CursorEnd::Failed { error, .. }, _) => Outcome::Failed(error),
                    (CursorEnd::Stopped, _) => Outcome::Stopped,
                    (CursorEnd::Fetched, _) if permit.stop_requested => Outcome::Stopped,
                    (CursorEnd::Fetched, Ok(())) => Outcome::Completed,
                    (CursorEnd::Fetched, Err(error)) => Outcome::Failed(error),
                }
            }
        };

        // The permit is released: events may flow again, now under frontend
        // credit. A result is reported complete only now that the cleanup
        // behind it has succeeded.
        if let Some(mut fetched) = fetched {
            while let Some(event) = fetched.recv().await {
                self.reducer.sender.send(event).await.ok()?;
            }
        }
        let settled = matches!(outcome, Outcome::Completed);
        self.reducer.release_completion(settled).await.ok()?;
        drain_notices(
            &self.reducer.sender,
            self.notices,
            &mut self.reducer.totals,
            &mut self.reducer.retained_notices,
        )
        .await
        .ok()?;
        Some(outcome)
    }

    async fn bound_command(
        &mut self,
        statement: &str,
        values: &BoundValues,
        entry: &TransactionEntry,
    ) -> Option<Outcome> {
        match self.control.checkpoint().await {
            Checkpoint::Closed => return Some(Outcome::Abandoned),
            Checkpoint::Stop => return Some(Outcome::Stopped),
            Checkpoint::Proceed => {}
        }
        // Prepared before any transaction is opened, so a refusal leaves the
        // session exactly as it was. Dropping `prepared` closes it.
        let prepared = match self.client.prepare(statement).await {
            Ok(prepared) => prepared,
            Err(error) => return Some(self.failed(error, Some(0)).into_outcome()),
        };
        if !prepared.columns().is_empty() {
            // The driver can only read extended-protocol rows in binary.
            return Some(Outcome::Refused(PARAMETERS_RETURN_ROWS));
        }
        if let TransactionEntry::Begin(begin) = entry {
            match self.control.checkpoint().await {
                Checkpoint::Closed => return Some(Outcome::Abandoned),
                Checkpoint::Stop => return Some(Outcome::Stopped),
                Checkpoint::Proceed => {}
            }
            if let Err(error) = self.client.batch_execute(begin).await {
                return Some(self.failed(error, None).into_outcome());
            }
        }
        match self.control.checkpoint().await {
            Checkpoint::Closed => return Some(Outcome::Abandoned),
            Checkpoint::Stop => return Some(Outcome::Stopped),
            Checkpoint::Proceed => {}
        }
        let parameters = values.0.iter().map(TextParameter).collect::<Vec<_>>();
        let result = self.client.execute(&prepared, &as_sql(&parameters)).await;
        let outcome = match result {
            Ok(affected) => {
                self.reducer.complete(affected).await.ok()?;
                Outcome::Completed
            }
            Err(error) => self.failed(error, Some(0)).into_outcome(),
        };
        drain_notices(
            &self.reducer.sender,
            self.notices,
            &mut self.reducer.totals,
            &mut self.reducer.retained_notices,
        )
        .await
        .ok()?;
        Some(outcome)
    }

    /// Maps a driver error on a statement-scoped path. `prefix_chars` is the
    /// length of the text sent before the user's statement; `None` when the
    /// failed request did not hold the user's statement at all.
    fn failed(&self, error: tokio_postgres::Error, prefix_chars: Option<u32>) -> CursorEnd {
        let server = error.as_db_error().is_some();
        let error = match statement_error(self.client, error) {
            QuerySessionError::Database {
                code,
                message,
                severity,
                position,
            } => QuerySessionError::Database {
                code,
                message,
                severity,
                position: position
                    .zip(prefix_chars)
                    .and_then(|(position, prefix)| self.plan.original_position(position, prefix)),
            },
            other => other,
        };
        CursorEnd::Failed { error, server }
    }
}

impl CursorEnd {
    fn into_outcome(self) -> Outcome {
        match self {
            Self::Fetched => Outcome::Completed,
            Self::Stopped => Outcome::Stopped,
            Self::Failed { error, .. } => Outcome::Failed(error),
        }
    }
}

fn as_sql<'a>(parameters: &'a [TextParameter<'a>]) -> Vec<&'a (dyn ToSql + Sync)> {
    parameters
        .iter()
        .map(|parameter| parameter as &(dyn ToSql + Sync))
        .collect()
}

/// On the statement-scoped paths the driver can fail with the server and the
/// socket both healthy: a parameter count mismatch, an encoding failure, an
/// unexpected message. Only a closed client retires the session.
fn statement_error(client: &Client, error: tokio_postgres::Error) -> QuerySessionError {
    if error.as_db_error().is_some() || client.is_closed() {
        return database_error(error);
    }
    QuerySessionError::Database {
        code: None,
        message: error.to_string(),
        severity: None,
        position: None,
    }
}

async fn cleanup(client: &Client, sql: &str) -> Result<(), QuerySessionError> {
    match tokio::time::timeout(CLEANUP_TIMEOUT, client.batch_execute(sql)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(statement_error(client, error)),
        Err(_) => Err(QuerySessionError::Timeout {
            operation: "cursorCleanup".into(),
        }),
    }
}

/// A cancel sent just before cleanup began can still land on the ROLLBACK,
/// so one answered with 57014 is sent once more.
async fn rollback(client: &Client) {
    for _ in 0..2 {
        let result = tokio::time::timeout(CLEANUP_TIMEOUT, client.batch_execute("ROLLBACK")).await;
        match result {
            Ok(Err(error)) if error.code() == Some(&SqlState::QUERY_CANCELED) => {}
            _ => return,
        }
    }
}

struct Completion {
    result_set_index: u32,
    row_count: u64,
    limit: Option<RowLimitOutcome>,
}

/// Applies every retention limit to one execution's rows, whichever shape
/// produced them.
struct Reducer {
    sender: mpsc::Sender<DriverEvent>,
    limit: RowLimit,
    /// Keep the completion back until `release_completion`.
    hold_completion: bool,
    held: Option<Completion>,
    totals: ExecutionTotals,
    result_set_index: u32,
    columns: Vec<Option<String>>,
    batch: Vec<Vec<Option<String>>>,
    batch_bytes: usize,
    total: u64,
    retained_in_set: usize,
    retained_total: usize,
    retained_bytes: usize,
    result_open: bool,
    retained_notices: u32,
    limited: Option<RowLimitOutcome>,
}

impl Reducer {
    fn new(sender: mpsc::Sender<DriverEvent>, limit: RowLimit) -> Self {
        Self {
            sender,
            limit,
            hold_completion: false,
            held: None,
            totals: ExecutionTotals::default(),
            result_set_index: 0,
            columns: Vec::new(),
            batch: Vec::new(),
            batch_bytes: 0,
            total: 0,
            retained_in_set: 0,
            retained_total: 0,
            retained_bytes: 0,
            result_open: false,
            retained_notices: 0,
            limited: None,
        }
    }

    async fn columns(&mut self, columns: Vec<Option<String>>) -> Result<(), ()> {
        self.result_open = true;
        self.columns = columns;
        bound_metadata(&mut self.columns, &mut self.totals);
        if self.result_set_index < 64 {
            self.sender
                .send(DriverEvent::ResultStarted {
                    result_set_index: self.result_set_index,
                    columns: self.columns.clone(),
                })
                .await
                .map_err(|_| ())?;
        }
        Ok(())
    }

    async fn row<'a>(&mut self, cells: impl Iterator<Item = Option<&'a str>>) {
        self.total += 1;
        match self.limit {
            RowLimit::Probe(limit) if self.total > u64::from(limit) => {
                self.total -= 1;
                self.limited = Some(RowLimitOutcome::Stopped);
                return;
            }
            RowLimit::Retain(limit) if self.retained_in_set >= limit as usize => {
                if self.limited.is_none() {
                    // Nothing more will be retained for this Result Set, so
                    // deliver what was instead of holding it while the rest
                    // of the result is read and dropped.
                    self.flush().await;
                }
                self.limited = Some(RowLimitOutcome::Drained);
                self.omit("rowLimit");
                return;
            }
            _ => {}
        }
        if self.retained_total >= 50_000
            || self.retained_in_set >= 10_000
            || self.retained_bytes >= 32 * 1024 * 1024
        {
            self.omit("rowCount");
            return;
        }
        let mut values = cells
            .map(|cell| {
                cell.map(|value| {
                    truncate_utf8(value, 1024 * 1024, &mut self.totals.truncation_reasons)
                })
            })
            .collect::<Vec<_>>();
        shrink_row(&mut values, &mut self.totals.truncation_reasons);
        let bytes = serde_json::to_vec(&values)
            .map(|json| json.len())
            .unwrap_or(usize::MAX);
        if bytes > 2 * 1024 * 1024 || self.retained_bytes + bytes > 32 * 1024 * 1024 {
            self.omit("rowBytes");
        } else {
            if !self.batch.is_empty()
                && (self.batch.len() >= 200 || self.batch_bytes + bytes > 256 * 1024)
            {
                self.flush().await;
            }
            self.retained_total += 1;
            self.retained_in_set += 1;
            self.retained_bytes += bytes;
            self.batch_bytes += bytes;
            self.batch.push(values);
        }
    }

    fn omit(&mut self, reason: &str) {
        self.totals.omitted_rows += 1;
        if !self
            .totals
            .truncation_reasons
            .iter()
            .any(|existing| existing == reason)
        {
            self.totals.truncation_reasons.push(reason.into());
        }
    }

    async fn complete(&mut self, count: u64) -> Result<(), ()> {
        if self.result_set_index < 64 {
            self.flush().await;
            if self.columns.is_empty() {
                let _ = self
                    .sender
                    .send(DriverEvent::ResultStarted {
                        result_set_index: self.result_set_index,
                        columns: Vec::new(),
                    })
                    .await;
            }
            let completion = Completion {
                result_set_index: self.result_set_index,
                // The server's count includes a probe row; ours does not.
                row_count: if self.limited == Some(RowLimitOutcome::Stopped) {
                    self.total
                } else {
                    self.total.max(count)
                },
                limit: self.limited,
            };
            if self.hold_completion {
                self.held = Some(completion);
            } else {
                self.send_completion(completion).await?;
            }
        } else {
            self.totals.omitted_result_sets += 1;
            self.totals.truncation_reasons.push("resultSets".into());
            self.batch.clear();
            self.batch_bytes = 0;
        }
        self.total = 0;
        self.retained_in_set = 0;
        self.columns.clear();
        self.result_open = false;
        self.limited = None;
        self.result_set_index += 1;
        Ok(())
    }

    async fn send_completion(&mut self, completion: Completion) -> Result<(), ()> {
        self.sender
            .send(DriverEvent::ResultCompleted {
                result_set_index: completion.result_set_index,
                row_count: completion.row_count,
                limit: completion.limit,
            })
            .await
            .map_err(|_| ())
    }

    /// Reports a held completion: complete when the cleanup behind it
    /// succeeded, otherwise as an aborted, partial result.
    async fn release_completion(&mut self, settled: bool) -> Result<(), ()> {
        let Some(completion) = self.held.take() else {
            return Ok(());
        };
        if settled {
            return self.send_completion(completion).await;
        }
        self.sender
            .send(DriverEvent::ResultAborted {
                result_set_index: completion.result_set_index,
                row_count: completion.row_count,
            })
            .await
            .map_err(|_| ())
    }

    /// A request failed mid-stream: deliver what was retained and close the
    /// open result as aborted.
    async fn abort(&mut self, notices: &mut mpsc::Receiver<Notice>) -> Result<(), ()> {
        self.flush().await;
        drain_notices(
            &self.sender,
            notices,
            &mut self.totals,
            &mut self.retained_notices,
        )
        .await?;
        if self.result_open {
            if self.result_set_index < 64 {
                self.sender
                    .send(DriverEvent::ResultAborted {
                        result_set_index: self.result_set_index,
                        row_count: self.total,
                    })
                    .await
                    .map_err(|_| ())?;
            } else {
                self.totals.omitted_result_sets += 1;
                self.totals.truncation_reasons.push("resultSets".into());
            }
        }
        Ok(())
    }

    async fn flush(&mut self) {
        if self.batch.is_empty() {
            return;
        }
        let rows = std::mem::take(&mut self.batch);
        self.batch_bytes = 0;
        let _ = self
            .sender
            .send(DriverEvent::RowBatch {
                result_set_index: self.result_set_index,
                rows,
            })
            .await;
    }

    async fn finish(mut self, outcome: Outcome) {
        self.totals.truncation_reasons.sort();
        self.totals.truncation_reasons.dedup();
        let _ = self
            .sender
            .send(DriverEvent::Finished {
                totals: self.totals,
                outcome,
            })
            .await;
    }
}

/// Feeds one simple-protocol request through the reducer. `Err(Some(_))` is
/// a failed request, with any open result already reported as aborted.
/// `Err(None)` means the receiver is gone.
async fn reduce_simple(
    reducer: &mut Reducer,
    client: &Client,
    notices: &mut mpsc::Receiver<Notice>,
    sql: &str,
) -> Result<(), Option<tokio_postgres::Error>> {
    let mut stream = match client.simple_query_raw(sql).await {
        Ok(stream) => Box::pin(stream),
        Err(error) => return Err(Some(error)),
    };
    let mut notices_open = true;
    loop {
        let message = tokio::select! {
            biased;
            notice = notices.recv(), if notices_open => {
                if let Some(notice) = notice {
                    send_notice(
                        &reducer.sender,
                        notice,
                        &mut reducer.totals,
                        &mut reducer.retained_notices,
                    )
                    .await
                    .map_err(|()| None)?;
                } else {
                    notices_open = false;
                }
                continue;
            }
            message = stream.next() => match message {
                Some(message) => message,
                None => break,
            },
        };
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                reducer.abort(notices).await.map_err(|()| None)?;
                return Err(Some(error));
            }
        };
        match message {
            SimpleQueryMessage::RowDescription(description) => reducer
                .columns(
                    description
                        .iter()
                        .map(|column| Some(column.name().to_string()))
                        .collect(),
                )
                .await
                .map_err(|()| None)?,
            SimpleQueryMessage::Row(row) => {
                reducer
                    .row((0..row.len()).map(|index| row.get(index)))
                    .await
            }
            SimpleQueryMessage::CommandComplete(count) => {
                reducer.complete(count).await.map_err(|()| None)?
            }
            _ => {}
        }
    }
    drain_notices(
        &reducer.sender,
        notices,
        &mut reducer.totals,
        &mut reducer.retained_notices,
    )
    .await
    .map_err(|()| None)
}

async fn drain_notices(
    sender: &mpsc::Sender<DriverEvent>,
    notices: &mut mpsc::Receiver<Notice>,
    totals: &mut ExecutionTotals,
    retained_notices: &mut u32,
) -> Result<(), ()> {
    while let Ok(notice) = notices.try_recv() {
        send_notice(sender, notice, totals, retained_notices).await?;
    }
    Ok(())
}

async fn send_notice(
    sender: &mpsc::Sender<DriverEvent>,
    notice: Notice,
    totals: &mut ExecutionTotals,
    retained_notices: &mut u32,
) -> Result<(), ()> {
    if *retained_notices >= 500 {
        totals.omitted_notices += 1;
        totals.truncation_reasons.push("notices".into());
        return Ok(());
    }
    let bytes = serde_json::to_vec(&(&notice.severity, &notice.message))
        .map(|json| json.len())
        .unwrap_or(usize::MAX);
    if totals.retained_metadata_bytes.saturating_add(bytes) > 1024 * 1024 {
        totals.omitted_notices += 1;
        totals.omitted_metadata_bytes = totals.omitted_metadata_bytes.saturating_add(bytes as u64);
        totals.truncation_reasons.push("metadataBytes".into());
        return Ok(());
    }
    totals.retained_metadata_bytes += bytes;
    *retained_notices += 1;
    sender
        .send(DriverEvent::Notice(notice))
        .await
        .map_err(|_| ())
}

fn bound_metadata(columns: &mut [Option<String>], result: &mut ExecutionTotals) {
    let mut remaining = (1024 * 1024_usize).saturating_sub(result.retained_metadata_bytes);
    for column in columns {
        let Some(name) = column else { continue };
        let bytes = name.len();
        if bytes <= remaining {
            remaining -= bytes;
            result.retained_metadata_bytes += bytes;
        } else {
            result.omitted_metadata_bytes += bytes as u64;
            *column = None;
            result.truncation_reasons.push("metadataBytes".into());
        }
    }
}

pub(crate) async fn cancel(cancel: tokio_postgres::CancelToken, tls: dedicated::TlsConfig) -> bool {
    crate::postgres::dedicated::cancel(cancel, tls).await
}
pub(crate) fn database_error(error: tokio_postgres::Error) -> QuerySessionError {
    map_dedicated(dedicated::database_error(error))
}
pub(crate) fn display_error(error: &QuerySessionError) -> Option<QueryDatabaseError> {
    match error {
        QuerySessionError::Database {
            code,
            message,
            severity,
            position,
        } => Some(QueryDatabaseError {
            code: code.clone(),
            message: message.clone(),
            severity: severity.clone(),
            position: *position,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PgDriverOptions;
    use std::sync::atomic::AtomicUsize;
    #[test]
    fn utf8_truncation_stays_on_boundary() {
        let mut reasons = Vec::new();
        assert_eq!(truncate_utf8("éé", 3, &mut reasons), "é");
        assert_eq!(reasons, ["cellBytes"]);
    }
    #[test]
    fn oversized_rows_shrink_later_cells_first() {
        let mut values = vec![Some("a".repeat(1024 * 1024)), Some("b".repeat(1024 * 1024))];
        let mut reasons = Vec::new();
        shrink_row(&mut values, &mut reasons);
        assert!(serde_json::to_vec(&values).unwrap().len() <= 2 * 1024 * 1024);
    }

    #[tokio::test]
    async fn notices_share_the_metadata_budget_and_count_limit() {
        let (sender, receiver) = mpsc::channel(501);
        let mut totals = ExecutionTotals::default();
        let mut retained = 0;
        for _ in 0..501 {
            send_notice(
                &sender,
                Notice {
                    severity: "NOTICE".into(),
                    message: "message".into(),
                },
                &mut totals,
                &mut retained,
            )
            .await
            .unwrap();
        }
        assert_eq!(retained, 500);
        assert_eq!(totals.omitted_notices, 1);
        assert_eq!(receiver.len(), 500);

        totals.retained_metadata_bytes = 1024 * 1024;
        retained = 0;
        send_notice(
            &sender,
            Notice {
                severity: "NOTICE".into(),
                message: "too large".into(),
            },
            &mut totals,
            &mut retained,
        )
        .await
        .unwrap();
        assert_eq!(totals.omitted_notices, 2);
        assert!(totals.omitted_metadata_bytes > 0);
        assert_eq!(receiver.len(), 500);
    }

    #[tokio::test]
    async fn queued_notices_are_drained_before_a_terminal_event() {
        let (notice_sender, mut notices) = mpsc::channel(1);
        notice_sender
            .send(Notice {
                severity: "NOTICE".into(),
                message: "before terminal".into(),
            })
            .await
            .unwrap();
        let (event_sender, mut events) = mpsc::channel(2);
        let mut totals = ExecutionTotals::default();
        let mut retained = 0;

        drain_notices(&event_sender, &mut notices, &mut totals, &mut retained)
            .await
            .unwrap();
        event_sender
            .send(DriverEvent::Finished {
                totals,
                outcome: Outcome::Completed,
            })
            .await
            .unwrap();

        assert!(matches!(events.recv().await, Some(DriverEvent::Notice(_))));
        assert!(matches!(
            events.recv().await,
            Some(DriverEvent::Finished {
                outcome: Outcome::Completed,
                ..
            })
        ));
    }

    /// Answers a driver path the way a session would, on a script: the nth
    /// checkpoint can report a Stop or a close, and so can the cleanup.
    #[derive(Default)]
    struct TestControl {
        checkpoints: AtomicUsize,
        stop_at: Option<usize>,
        close_at: Option<usize>,
        stop_at_cleanup: bool,
        close_at_cleanup: bool,
        cleanups: AtomicUsize,
    }

    impl ExecutionControl for TestControl {
        fn checkpoint(&self) -> BoxFuture<'_, Checkpoint> {
            let reached = self.checkpoints.fetch_add(1, Ordering::SeqCst) + 1;
            Box::pin(async move {
                if self.close_at == Some(reached) {
                    Checkpoint::Closed
                } else if self.stop_at == Some(reached) {
                    Checkpoint::Stop
                } else {
                    Checkpoint::Proceed
                }
            })
        }

        fn begin_cleanup(&self) -> BoxFuture<'_, Option<CleanupPermit<'_>>> {
            self.cleanups.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                (!self.close_at_cleanup).then(|| CleanupPermit {
                    stop_requested: self.stop_at_cleanup,
                    _open: Box::new(()),
                })
            })
        }
    }

    struct Recorded {
        columns: Vec<Vec<Option<String>>>,
        rows: Vec<Vec<Option<String>>>,
        batches: Vec<usize>,
        completed: Vec<(u32, u64, Option<RowLimitOutcome>)>,
        aborted: Vec<(u32, u64)>,
        notices: Vec<String>,
        totals: ExecutionTotals,
        outcome: Outcome,
        /// Kinds in arrival order, to assert on sequencing.
        order: Vec<&'static str>,
    }

    async fn record(mut events: mpsc::Receiver<DriverEvent>) -> Recorded {
        let mut recorded = Recorded {
            columns: Vec::new(),
            rows: Vec::new(),
            batches: Vec::new(),
            completed: Vec::new(),
            aborted: Vec::new(),
            notices: Vec::new(),
            totals: ExecutionTotals::default(),
            outcome: Outcome::Abandoned,
            order: Vec::new(),
        };
        while let Some(event) = events.recv().await {
            match event {
                DriverEvent::ResultStarted { columns, .. } => {
                    recorded.order.push("started");
                    recorded.columns.push(columns);
                }
                DriverEvent::RowBatch { rows, .. } => {
                    recorded.order.push("batch");
                    recorded.batches.push(rows.len());
                    recorded.rows.extend(rows);
                }
                DriverEvent::ResultCompleted {
                    result_set_index,
                    row_count,
                    limit,
                } => {
                    recorded.order.push("completed");
                    recorded
                        .completed
                        .push((result_set_index, row_count, limit));
                }
                DriverEvent::ResultAborted {
                    result_set_index,
                    row_count,
                } => {
                    recorded.order.push("aborted");
                    recorded.aborted.push((result_set_index, row_count));
                }
                DriverEvent::Notice(notice) => recorded.notices.push(notice.message),
                DriverEvent::Finished { totals, outcome } => {
                    recorded.order.push("finished");
                    recorded.totals = totals;
                    recorded.outcome = outcome;
                }
            }
        }
        recorded
    }

    async fn run_plan(
        connection: &SessionConnection,
        sql: &str,
        parameters: Option<&[(&str, Option<&str>)]>,
        row_limit: Option<i64>,
        entry: TransactionEntry,
        control: TestControl,
    ) -> Recorded {
        let supplied = parameters.map(|parameters| {
            parameters
                .iter()
                .map(
                    |(name, value)| crate::postgres::sql_params::ParameterValue {
                        name: (*name).into(),
                        value: value.map(Into::into),
                    },
                )
                .collect::<Vec<_>>()
        });
        let plan =
            crate::postgres::sql_params::plan_execution(sql.into(), supplied.as_deref(), row_limit)
                .expect("plan");
        record(execute_plan(
            connection.client.clone(),
            connection.notices.clone(),
            plan,
            entry,
            Arc::new(control),
        ))
        .await
    }

    fn cell_row(bytes: usize) -> String {
        "x".repeat(bytes)
    }

    /// Feeds synthetic result sets through the reducer: `(rows, cell bytes)`
    /// with one column each, completed with the server's row count.
    async fn reduce(limit: RowLimit, sets: &[(usize, usize)]) -> Recorded {
        let (sender, receiver) = mpsc::channel(200_000);
        let mut reducer = Reducer::new(sender, limit);
        for (rows, bytes) in sets {
            let cell = cell_row(*bytes);
            reducer.columns(vec![Some("c".into())]).await.unwrap();
            for _ in 0..*rows {
                reducer.row(std::iter::once(Some(cell.as_str()))).await;
            }
            reducer.complete(*rows as u64).await.unwrap();
        }
        reducer.finish(Outcome::Completed).await;
        record(receiver).await
    }

    #[tokio::test]
    async fn script_rows_keep_the_per_set_cap_and_two_hundred_row_batches() {
        let run = reduce(RowLimit::None, &[(10_450, 4), (3, 4)]).await;
        assert_eq!(run.batches.len(), 51);
        assert!(run.batches[..50].iter().all(|rows| *rows == 200));
        assert_eq!(run.batches[50], 3);
        assert_eq!(run.completed, [(0, 10_450, None), (1, 3, None)]);
        assert_eq!(run.totals.omitted_rows, 450);
        assert_eq!(run.totals.truncation_reasons, ["rowCount"]);
    }

    #[tokio::test]
    async fn script_rows_keep_the_execution_cap_across_result_sets() {
        let run = reduce(RowLimit::None, &[(10_000, 1); 6]).await;
        assert_eq!(run.rows.len(), 50_000);
        assert_eq!(run.completed.len(), 6);
        assert!(run.completed.iter().all(|(_, rows, _)| *rows == 10_000));
        assert_eq!(run.totals.omitted_rows, 10_000);
        assert_eq!(run.totals.truncation_reasons, ["rowCount"]);
    }

    #[tokio::test]
    async fn script_batches_split_at_the_byte_budget() {
        // A 100 KiB cell serializes to 102,404 bytes; a third would pass 256 KiB.
        let run = reduce(RowLimit::None, &[(7, 100 * 1024)]).await;
        assert_eq!(run.batches, [2, 2, 2, 1]);
        assert_eq!(run.totals.omitted_rows, 0);
        assert!(run.totals.truncation_reasons.is_empty());
    }

    #[tokio::test]
    async fn script_rows_keep_the_cell_and_execution_byte_caps() {
        // Each cell is cut to 1 MiB; the 32nd row would pass 32 MiB.
        let run = reduce(RowLimit::None, &[(40, 1024 * 1024 + 10)]).await;
        assert_eq!(run.rows.len(), 31);
        assert!(run
            .rows
            .iter()
            .all(|row| row[0].as_ref().unwrap().len() == 1024 * 1024));
        assert_eq!(run.batches, vec![1; 31]);
        assert_eq!(run.completed, [(0, 40, None)]);
        assert_eq!(run.totals.omitted_rows, 9);
        assert_eq!(run.totals.truncation_reasons, ["cellBytes", "rowBytes"]);
    }

    #[tokio::test]
    async fn script_keeps_sixty_four_result_sets_and_reports_commands() {
        let run = reduce(RowLimit::None, &[(1, 1); 65]).await;
        assert_eq!(run.columns.len(), 64);
        assert_eq!(run.completed.len(), 64);
        assert_eq!(run.rows.len(), 64);
        assert_eq!(run.totals.omitted_result_sets, 1);
        assert_eq!(run.totals.truncation_reasons, ["resultSets"]);

        // A command has no RowDescription: it reports the server's count.
        let (sender, receiver) = mpsc::channel(8);
        let mut reducer = Reducer::new(sender, RowLimit::None);
        reducer.complete(7).await.unwrap();
        reducer.finish(Outcome::Completed).await;
        let run = record(receiver).await;
        assert_eq!(run.columns, [Vec::<Option<String>>::new()]);
        assert_eq!(run.completed, [(0, 7, None)]);
        assert_eq!(run.order, ["started", "completed", "finished"]);
    }

    #[tokio::test]
    async fn a_retained_limit_drains_every_result_set_and_counts_what_it_withheld() {
        let run = reduce(RowLimit::Retain(3), &[(5, 1), (3, 1), (2, 1)]).await;
        assert_eq!(run.batches, [3, 3, 2]);
        assert_eq!(
            run.completed,
            [
                (0, 5, Some(RowLimitOutcome::Drained)),
                // A limit equal to the row count withheld nothing.
                (1, 3, None),
                (2, 2, None),
            ]
        );
        assert_eq!(run.totals.omitted_rows, 2);
        assert_eq!(run.totals.truncation_reasons, ["rowLimit"]);

        // The retained rows are delivered when the limit is reached, not
        // after the rest of the result has been read and dropped.
        let (sender, mut receiver) = mpsc::channel(8);
        let mut reducer = Reducer::new(sender, RowLimit::Retain(2));
        reducer.columns(vec![Some("c".into())]).await.unwrap();
        for _ in 0..3 {
            reducer.row(std::iter::once(Some("a"))).await;
        }
        assert!(matches!(
            receiver.recv().await,
            Some(DriverEvent::ResultStarted { .. })
        ));
        assert!(matches!(
            receiver.try_recv(),
            Ok(DriverEvent::RowBatch { rows, .. }) if rows.len() == 2
        ));
    }

    #[tokio::test]
    async fn a_probe_row_proves_more_rows_exist_without_being_counted() {
        // The FETCH asked for limit + 1 rows and got them all.
        let run = reduce(RowLimit::Probe(3), &[(4, 1)]).await;
        assert_eq!(run.rows.len(), 3);
        assert_eq!(run.completed, [(0, 3, Some(RowLimitOutcome::Stopped))]);
        assert_eq!(run.totals.omitted_rows, 0);
        assert!(run.totals.truncation_reasons.is_empty());

        // Exactly the limit: nothing was withheld.
        let run = reduce(RowLimit::Probe(3), &[(3, 1)]).await;
        assert_eq!(run.rows.len(), 3);
        assert_eq!(run.completed, [(0, 3, None)]);

        // Byte caps still apply before the limit: 31 rows fit in 32 MiB.
        let run = reduce(RowLimit::Probe(40), &[(41, 1024 * 1024)]).await;
        assert_eq!(run.rows.len(), 31);
        assert_eq!(run.completed, [(0, 40, Some(RowLimitOutcome::Stopped))]);
        assert_eq!(run.totals.omitted_rows, 9);
        assert_eq!(run.totals.truncation_reasons, ["rowBytes"]);
    }

    #[tokio::test]
    async fn a_held_completion_is_reported_only_once_cleanup_settles() {
        for settled in [true, false] {
            let (sender, receiver) = mpsc::channel(8);
            let mut reducer = Reducer::new(sender, RowLimit::Probe(1));
            reducer.hold_completion = true;
            reducer.columns(vec![Some("c".into())]).await.unwrap();
            reducer.row(std::iter::once(Some("a"))).await;
            reducer.row(std::iter::once(Some("b"))).await;
            reducer.complete(2).await.unwrap();
            // Rows are delivered; the completion is not.
            assert_eq!(reducer.sender.max_capacity() - reducer.sender.capacity(), 2);
            reducer.release_completion(settled).await.unwrap();
            reducer.finish(Outcome::Completed).await;
            let run = record(receiver).await;
            if settled {
                assert_eq!(run.completed, [(0, 1, Some(RowLimitOutcome::Stopped))]);
                assert_eq!(run.order, ["started", "batch", "completed", "finished"]);
            } else {
                assert_eq!(run.aborted, [(0, 1)]);
                assert_eq!(run.order, ["started", "batch", "aborted", "finished"]);
            }
        }
    }

    #[tokio::test]
    async fn a_failed_request_delivers_retained_rows_and_aborts_the_open_result() {
        let (sender, receiver) = mpsc::channel(8);
        let (_notice_sender, mut notices) = mpsc::channel(1);
        let mut reducer = Reducer::new(sender, RowLimit::None);
        reducer.columns(vec![Some("c".into())]).await.unwrap();
        reducer.row(std::iter::once(Some("a"))).await;
        reducer.row(std::iter::once(None)).await;
        reducer.abort(&mut notices).await.unwrap();
        reducer
            .finish(Outcome::Failed(QuerySessionError::ConnectionLost))
            .await;
        let run = record(receiver).await;
        assert_eq!(run.rows, [vec![Some("a".to_owned())], vec![None]]);
        assert_eq!(run.aborted, [(0, 2)]);
        assert_eq!(run.order, ["started", "batch", "aborted", "finished"]);
    }

    #[test]
    fn parameter_values_stay_out_of_debug_output_and_driver_logs() {
        let secret = Some("hunter2-secret".to_owned());
        let rendered = format!("{:?}", TextParameter(&secret));
        assert!(!rendered.contains("hunter2"), "{rendered}");
        // The driver logs bound parameters with `Debug` at Debug level. The
        // redaction above is the first line; this pin is the second.
        assert!(crate::TOKIO_POSTGRES_LOG_LEVEL <= log::LevelFilter::Warn);
    }

    #[test]
    fn text_parameters_are_sent_as_text_and_accept_every_type() {
        let value = Some("42".to_owned());
        let parameter = TextParameter(&value);
        let mut out = BytesMut::new();
        for accepted in [Type::INT4, Type::TEXT_ARRAY, Type::UNKNOWN, Type::JSONB] {
            assert!(<TextParameter as ToSql>::accepts(&accepted));
            assert!(matches!(parameter.encode_format(&accepted), Format::Text));
        }
        assert!(matches!(
            parameter.to_sql(&Type::INT4, &mut out),
            Ok(IsNull::No)
        ));
        assert_eq!(&out[..], b"42");
        assert!(matches!(
            TextParameter(&None).to_sql(&Type::INT4, &mut out),
            Ok(IsNull::Yes)
        ));
    }

    fn live_spec(port: u16, prefer_tls: bool) -> ResolvedPostgresConnectSpec {
        let tls = if prefer_tls {
            crate::postgres::tls::ResolvedTls::prefer("127.0.0.1")
        } else {
            crate::postgres::tls::ResolvedTls::plain("127.0.0.1")
        };
        ResolvedPostgresConnectSpec {
            connection_id: format!("live-{port}"),
            host: "127.0.0.1".into(),
            port,
            database: "dbunk_demo".into(),
            user: "dbunk".into(),
            password: "dbunk".into(),
            tls,
            connect_timeout: Some(Duration::from_secs(5)),
            keepalive: None,
            driver_options: PgDriverOptions::default(),
            safety_policy: Default::default(),
        }
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn query_session_live_plaintext_refusal_results_temp_state_and_cancel() {
        let connection = connect(&live_spec(15432, true))
            .await
            .expect("Prefer falls back when TLS is refused");
        connection.client.batch_execute("CREATE TEMP TABLE query_session_live_temp(value int); INSERT INTO query_session_live_temp VALUES (1)").await.expect("temp state");
        let run = run_plan(
            &connection,
            "SELECT * FROM query_session_live_temp; SELECT 1 WHERE false",
            None,
            None,
            TransactionEntry::Autocommit,
            TestControl::default(),
        )
        .await;
        assert!(matches!(run.outcome, Outcome::Completed));
        assert_eq!(run.columns.len(), 2);
        assert_eq!(run.rows.len(), 1);
        assert_eq!(run.columns[1].len(), 1);
        let cancel = connection.cancel.clone();
        let query = connection.client.simple_query("SELECT pg_sleep(30)");
        let (result, requested) = tokio::join!(query, cancel_query_for_test(cancel, None));
        assert!(requested);
        assert!(result.is_err());
    }

    /// A session socket plus a second one to read its backend state with.
    struct LiveDriver {
        connection: SessionConnection,
        admin: SessionConnection,
    }

    impl LiveDriver {
        async fn open() -> Self {
            Self {
                connection: connect(&live_spec(15432, false)).await.expect("session"),
                admin: connect(&live_spec(15432, false)).await.expect("admin"),
            }
        }

        async fn run(
            &self,
            sql: &str,
            parameters: Option<&[(&str, Option<&str>)]>,
            row_limit: Option<i64>,
            entry: TransactionEntry,
            control: TestControl,
        ) -> Recorded {
            run_plan(&self.connection, sql, parameters, row_limit, entry, control).await
        }

        /// `pg_stat_activity.state` once the backend is no longer running.
        async fn state(&self) -> String {
            for _ in 0..200 {
                let state: String = self
                    .admin
                    .client
                    .query_one(
                        "SELECT state FROM pg_stat_activity WHERE pid = $1",
                        &[&self.connection.pid],
                    )
                    .await
                    .expect("backend state")
                    .get(0);
                if state != "active" {
                    return state;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            "active".into()
        }

        /// One value read on the session socket through the simple protocol,
        /// which prepares nothing.
        async fn scalar(&self, sql: &str) -> String {
            self.connection
                .client
                .simple_query(sql)
                .await
                .expect("scalar query")
                .into_iter()
                .find_map(|message| match message {
                    SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
                    _ => None,
                })
                .expect("one value")
        }

        async fn execute(&self, sql: &str) {
            self.connection.client.batch_execute(sql).await.expect(sql);
        }
    }

    fn stop_at(checkpoint: usize) -> TestControl {
        TestControl {
            stop_at: Some(checkpoint),
            ..Default::default()
        }
    }

    fn close_at(checkpoint: usize) -> TestControl {
        TestControl {
            close_at: Some(checkpoint),
            ..Default::default()
        }
    }

    fn failure(outcome: &Outcome) -> (Option<&str>, Option<u32>) {
        match outcome {
            Outcome::Failed(QuerySessionError::Database { code, position, .. }) => {
                (code.as_deref(), *position)
            }
            other => panic!("expected a database failure, got {other:?}"),
        }
    }

    const SERIES: &str = "SELECT g FROM generate_series(1, 5) g";
    const LEFT_OVERS: &str =
        "SELECT (SELECT count(*) FROM pg_cursors) + (SELECT count(*) FROM pg_prepared_statements)";

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn query_session_live_text_parameters_infer_types_and_stay_data() {
        let live = LiveDriver::open().await;
        live.execute(
            "CREATE TEMP TABLE bound_types(i int4, t text, ts timestamptz, n numeric, \
             b bool, a text[], j jsonb, raw bytea); \
             SET TIME ZONE 'UTC'; \
             INSERT INTO bound_types VALUES (7, 'seven', '2026-01-02T03:04:05Z', 1.50, true, \
             ARRAY['x', 'y'], '{\"k\": [1, 2]}', '\\xdeadbeef')",
        )
        .await;
        // Each value is plain text; the server infers the type from the column.
        let run = live
            .run(
                "SELECT i, t, ts, n, b, a, j, raw FROM bound_types \
                 WHERE i = :i AND t = :t AND ts = :ts AND n = :n AND b = :b AND a = :a \
                 AND (:nothing::text IS NULL OR t = :nothing) AND t <> :attack",
                Some(&[
                    ("i", Some("7")),
                    ("t", Some("seven")),
                    ("ts", Some("2026-01-02 03:04:05+00")),
                    ("n", Some("1.5")),
                    ("b", Some("true")),
                    ("a", Some("{x,y}")),
                    ("nothing", None),
                    ("attack", Some("'; DROP TABLE bound_types; --")),
                ]),
                None,
                TransactionEntry::Autocommit,
                TestControl::default(),
            )
            .await;
        assert!(
            matches!(run.outcome, Outcome::Completed),
            "{:?}",
            run.outcome
        );
        // Rows are the server's own text rendering, as on the Script shape.
        let expected = [
            "7",
            "seven",
            "2026-01-02 03:04:05+00",
            "1.50",
            "t",
            "{x,y}",
            "{\"k\": [1, 2]}",
            "\\xdeadbeef",
        ]
        .map(|value| Some(value.to_owned()));
        assert_eq!(run.rows, [expected.to_vec()]);
        assert_eq!(run.completed, [(0, 1, None)]);
        assert_eq!(live.scalar("SELECT count(*) FROM bound_types").await, "1");
        assert_eq!(live.state().await, "idle");
        assert_eq!(live.scalar(LEFT_OVERS).await, "0");
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn query_session_live_wrapper_exit_rules() {
        let live = LiveDriver::open().await;
        let wrapper = || TransactionEntry::Autocommit;

        // Success: rows, then the completion, only after CLOSE and COMMIT.
        let run = live
            .run(SERIES, None, Some(5), wrapper(), TestControl::default())
            .await;
        assert!(matches!(run.outcome, Outcome::Completed));
        assert_eq!(run.order, ["started", "batch", "completed", "finished"]);
        assert_eq!(run.completed, [(0, 5, None)]);
        assert_eq!(live.state().await, "idle");

        // A Stop before BEGIN sends nothing; later ones roll the wrapper back.
        let run = live.run(SERIES, None, Some(5), wrapper(), stop_at(1)).await;
        assert!(matches!(run.outcome, Outcome::Stopped));
        assert_eq!(run.order, ["finished"]);
        for checkpoint in [2, 3] {
            let run = live
                .run(SERIES, None, Some(5), wrapper(), stop_at(checkpoint))
                .await;
            assert!(matches!(run.outcome, Outcome::Stopped), "{checkpoint}");
            assert_eq!(run.order, ["finished"], "{checkpoint}");
            assert_eq!(live.state().await, "idle", "{checkpoint}");
        }

        // A Stop seen once the FETCH is done: the rows were delivered, the
        // result is not reported complete, and the wrapper is rolled back.
        let control = TestControl {
            stop_at_cleanup: true,
            ..Default::default()
        };
        let run = live.run(SERIES, None, Some(5), wrapper(), control).await;
        assert!(matches!(run.outcome, Outcome::Stopped));
        assert_eq!(run.order, ["started", "batch", "aborted", "finished"]);
        assert_eq!(run.aborted, [(0, 5)]);
        assert_eq!(live.state().await, "idle");

        // A server error at DECLARE maps its position to the user's text.
        let sql = "  SELECT missing_column, :a FROM generate_series(1, 2)";
        let run = live
            .run(
                sql,
                Some(&[("a", Some("1"))]),
                None,
                wrapper(),
                TestControl::default(),
            )
            .await;
        let position = sql.find("missing_column").unwrap() as u32 + 1;
        assert_eq!(failure(&run.outcome), (Some("42703"), Some(position)));
        assert_eq!(run.order, ["finished"]);
        assert_eq!(live.state().await, "idle");

        // A value the inferred type rejects fails at DECLARE.
        let run = live
            .run(
                "SELECT g FROM generate_series(1, 2) g WHERE g = :g",
                Some(&[("g", Some("abc"))]),
                None,
                wrapper(),
                TestControl::default(),
            )
            .await;
        assert_eq!(failure(&run.outcome).0, Some("22P02"));
        assert_eq!(live.state().await, "idle");

        // A parameter with no type context at all is the server's to refuse.
        let run = live
            .run(
                "SELECT 1 WHERE :lonely IS NULL",
                Some(&[("lonely", None)]),
                None,
                wrapper(),
                TestControl::default(),
            )
            .await;
        assert_eq!(failure(&run.outcome).0, Some("42P18"));
        assert_eq!(live.state().await, "idle");

        // So is one whose first use has none: `:x IS NULL OR t = :x` needs
        // `:x::text IS NULL`. A cursor skips the parse-time check a plain
        // statement gets, so the server reports it when the FETCH runs.
        let run = live
            .run(
                "SELECT g FROM generate_series(1, 2) g WHERE :x IS NULL OR g::text = :x",
                Some(&[("x", Some("1"))]),
                None,
                wrapper(),
                TestControl::default(),
            )
            .await;
        assert_eq!(failure(&run.outcome).0, Some("42804"));
        assert_eq!(live.state().await, "idle");

        // The server runs a FETCH to its end before it sends the first row,
        // so an error during it delivers no rows at all.
        let run = live
            .run(
                "SELECT 10 / (3 - g) FROM generate_series(1, 5) g",
                None,
                Some(5),
                wrapper(),
                TestControl::default(),
            )
            .await;
        assert_eq!(failure(&run.outcome), (Some("22012"), None));
        assert_eq!(run.order, ["finished"]);
        assert_eq!(live.state().await, "idle");
        assert_eq!(live.scalar(LEFT_OVERS).await, "0");
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn query_session_live_wrapper_ends_before_anyone_takes_a_row() {
        let live = LiveDriver::open().await;
        let plan = crate::postgres::sql_params::plan_execution(
            "SELECT repeat('x', 1000) FROM generate_series(1, 2000)".into(),
            None,
            Some(1_000),
        )
        .expect("plan");
        // Nothing receives yet, as when a frontend withholds credit.
        let events = execute_plan(
            live.connection.client.clone(),
            live.connection.notices.clone(),
            plan,
            TransactionEntry::Autocommit,
            Arc::new(TestControl::default()),
        );
        // The read is fetched, closed and committed regardless: a backend
        // left idle in a transaction would hold its snapshot for as long as
        // the frontend stalls, and an idle-in-transaction timeout would end
        // the session.
        let mut state = String::new();
        for _ in 0..300 {
            state = live.state().await;
            if state == "idle" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(state, "idle");
        assert_eq!(live.scalar("SELECT count(*) FROM pg_cursors").await, "0");

        let run = record(events).await;
        assert!(matches!(run.outcome, Outcome::Completed));
        assert_eq!(run.rows.len(), 1_000);
        assert_eq!(run.completed, [(0, 1_000, Some(RowLimitOutcome::Stopped))]);
        assert_eq!(run.order.first(), Some(&"started"));
        assert_eq!(run.order[run.order.len() - 2..], ["completed", "finished"]);
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn query_session_live_wrapper_sends_nothing_after_a_close() {
        // Checkpoint 1 is before BEGIN, 2 before DECLARE, 3 before FETCH.
        for (checkpoint, state) in [
            (1, "idle"),
            (2, "idle in transaction"),
            (3, "idle in transaction"),
        ] {
            let live = LiveDriver::open().await;
            let control = close_at(checkpoint);
            let run = live
                .run(SERIES, None, Some(5), TransactionEntry::Autocommit, control)
                .await;
            assert!(matches!(run.outcome, Outcome::Abandoned), "{checkpoint}");
            assert_eq!(run.order, ["finished"], "{checkpoint}");
            // Nothing was rolled back or committed: the socket drop does that.
            assert_eq!(live.state().await, state, "{checkpoint}");
        }

        // Closed while the FETCH ran: no CLOSE, no COMMIT, no completion.
        let live = LiveDriver::open().await;
        let control = TestControl {
            close_at_cleanup: true,
            ..Default::default()
        };
        let run = live
            .run(SERIES, None, Some(5), TransactionEntry::Autocommit, control)
            .await;
        assert!(matches!(run.outcome, Outcome::Abandoned));
        assert_eq!(run.order, ["finished"]);
        assert_eq!(live.state().await, "idle in transaction");
        assert_eq!(live.scalar("SELECT count(*) FROM pg_cursors").await, "1");
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn query_session_live_cursor_inside_a_user_transaction() {
        let live = LiveDriver::open().await;

        // Manual mode opens the user's transaction and leaves it open.
        let begin = TransactionEntry::Begin("BEGIN ISOLATION LEVEL REPEATABLE READ".into());
        let run = live
            .run(SERIES, None, Some(2), begin, TestControl::default())
            .await;
        assert!(matches!(run.outcome, Outcome::Completed));
        assert_eq!(run.completed, [(0, 2, Some(RowLimitOutcome::Stopped))]);
        assert_eq!(run.rows.len(), 2);
        assert_eq!(live.state().await, "idle in transaction");
        assert_eq!(
            live.scalar("SHOW transaction_isolation").await,
            "repeatable read"
        );
        assert_eq!(live.scalar("SELECT count(*) FROM pg_cursors").await, "0");

        // Inside it, checkpoint 1 is before DECLARE and 2 before FETCH. A Stop
        // closes the cursor if it was declared and keeps the transaction.
        for control in [
            stop_at(1),
            stop_at(2),
            TestControl {
                stop_at_cleanup: true,
                ..Default::default()
            },
        ] {
            let run = live
                .run(SERIES, None, Some(2), TransactionEntry::Inside, control)
                .await;
            assert!(matches!(run.outcome, Outcome::Stopped));
            assert_eq!(live.state().await, "idle in transaction");
            assert_eq!(live.scalar("SELECT count(*) FROM pg_cursors").await, "0");
        }

        // A user cursor with the reserved name collides and is reported as is.
        live.execute("DECLARE dbunk_query_cursor CURSOR FOR SELECT 1")
            .await;
        let run = live
            .run(
                SERIES,
                None,
                Some(2),
                TransactionEntry::Inside,
                TestControl::default(),
            )
            .await;
        assert_eq!(failure(&run.outcome).0, Some("42P03"));
        // A server error aborts the user's transaction, as the statement
        // failing directly would. Nothing is sent to clean up.
        assert_eq!(live.state().await, "idle in transaction (aborted)");
        live.execute("ROLLBACK").await;

        // The same collision in autocommit, from a holdable cursor, rolls the
        // wrapper back and leaves the user's cursor alone.
        live.execute("BEGIN; DECLARE dbunk_query_cursor CURSOR WITH HOLD FOR SELECT 1; COMMIT")
            .await;
        let run = live
            .run(
                SERIES,
                None,
                Some(2),
                TransactionEntry::Autocommit,
                TestControl::default(),
            )
            .await;
        assert_eq!(failure(&run.outcome).0, Some("42P03"));
        assert_eq!(live.state().await, "idle");
        assert_eq!(live.scalar("SELECT count(*) FROM pg_cursors").await, "1");
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn query_session_live_bound_command_runs_refuses_and_stops() {
        let live = LiveDriver::open().await;
        live.execute(
            "CREATE TEMP TABLE bound_rows(id int4 PRIMARY KEY, v int4); \
             INSERT INTO bound_rows VALUES (1, 0), (2, 0), (3, 0)",
        )
        .await;
        let update = "UPDATE bound_rows SET v = :v WHERE id <= :id";
        let values: &[(&str, Option<&str>)] = &[("v", Some("9")), ("id", Some("2"))];
        let autocommit = || TransactionEntry::Autocommit;

        let run = live
            .run(
                update,
                Some(values),
                Some(1),
                autocommit(),
                TestControl::default(),
            )
            .await;
        assert!(
            matches!(run.outcome, Outcome::Completed),
            "{:?}",
            run.outcome
        );
        // One command-only Result Set carrying the affected-row count.
        assert_eq!(run.columns, [Vec::<Option<String>>::new()]);
        assert_eq!(run.completed, [(0, 2, None)]);
        assert_eq!(live.scalar("SELECT sum(v) FROM bound_rows").await, "18");

        // Rows cannot be read through the extended protocol: refused unrun.
        for sql in [
            "UPDATE bound_rows SET v = :v WHERE id <= :id RETURNING id",
            "EXPLAIN SELECT * FROM bound_rows WHERE v = :v AND id <= :id",
            "SELECT * FROM bound_rows WHERE v = :v AND id <= :id FOR UPDATE",
        ] {
            let manual = TransactionEntry::Begin("BEGIN ISOLATION LEVEL READ COMMITTED".into());
            let run = live
                .run(
                    sql,
                    Some(&[("v", Some("5")), ("id", Some("3"))]),
                    None,
                    manual,
                    TestControl::default(),
                )
                .await;
            assert!(
                matches!(run.outcome, Outcome::Refused("parametersReturnRows")),
                "{sql}: {:?}",
                run.outcome
            );
            assert_eq!(run.order, ["finished"], "{sql}");
            // Prepared before the manual transaction: none was opened.
            assert_eq!(live.state().await, "idle", "{sql}");
        }
        assert_eq!(live.scalar("SELECT sum(v) FROM bound_rows").await, "18");

        // A failure maps its position; the session stays usable.
        let sql = "UPDATE bound_rows SET nope = :v WHERE id <= :id";
        let run = live
            .run(
                sql,
                Some(values),
                None,
                autocommit(),
                TestControl::default(),
            )
            .await;
        let position = sql.find("nope").unwrap() as u32 + 1;
        assert_eq!(failure(&run.outcome), (Some("42703"), Some(position)));
        let run = live
            .run(
                update,
                Some(&[("v", Some("abc")), ("id", Some("2"))]),
                None,
                autocommit(),
                TestControl::default(),
            )
            .await;
        assert_eq!(failure(&run.outcome).0, Some("22P02"));
        assert_eq!(live.state().await, "idle");

        // Checkpoint 1 is before the prepare, 2 before the manual BEGIN, 3
        // before the statement. Only the last leaves the user's transaction.
        for (checkpoint, state) in [(1, "idle"), (2, "idle"), (3, "idle in transaction")] {
            let manual = TransactionEntry::Begin("BEGIN ISOLATION LEVEL READ COMMITTED".into());
            let run = live
                .run(update, Some(values), None, manual, stop_at(checkpoint))
                .await;
            assert!(matches!(run.outcome, Outcome::Stopped), "{checkpoint}");
            assert_eq!(live.state().await, state, "{checkpoint}");
        }
        live.execute("ROLLBACK").await;
        assert_eq!(live.scalar("SELECT sum(v) FROM bound_rows").await, "18");

        // Manual mode opens the transaction and runs inside it.
        let manual = TransactionEntry::Begin("BEGIN ISOLATION LEVEL READ COMMITTED".into());
        let run = live
            .run(
                update,
                Some(&[("v", Some("1")), ("id", Some("3"))]),
                None,
                manual,
                TestControl::default(),
            )
            .await;
        assert_eq!(run.completed, [(0, 3, None)]);
        assert_eq!(live.state().await, "idle in transaction");
        live.execute("ROLLBACK").await;

        // Nothing prepared for user SQL outlives success, error, stop, or refusal.
        assert_eq!(live.scalar(LEFT_OVERS).await, "0");
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn query_session_live_driver_side_failures_do_not_retire_the_session() {
        let live = LiveDriver::open().await;
        let client = &live.connection.client;
        // The driver refuses this after its prepare round trip; the server
        // never sees an error and the socket is healthy.
        let error = client
            .execute("SELECT $1::int4, $2::int4", &[&1_i32])
            .await
            .expect_err("parameter count mismatch");
        assert!(error.as_db_error().is_none());
        assert!(matches!(
            statement_error(client, error),
            QuerySessionError::Database { code: None, .. }
        ));
        // The Script shape keeps its mapping: any non-server error is a loss.
        let error = client
            .execute("SELECT $1::int4, $2::int4", &[&1_i32])
            .await
            .expect_err("parameter count mismatch");
        assert!(matches!(
            database_error(error),
            QuerySessionError::ConnectionLost
        ));
        assert_eq!(live.scalar("SELECT 1").await, "1");
        assert_eq!(live.scalar(LEFT_OVERS).await, "0");
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres-tls"]
    async fn query_session_live_tls_connects_and_cancels() {
        let connection = connect(&live_spec(15433, true))
            .await
            .expect("self-signed TLS");
        let cancel = connection.cancel.clone();
        let query = connection.client.simple_query("SELECT pg_sleep(30)");
        let (result, requested) =
            tokio::join!(query, cancel_query_for_test(cancel, connection.tls.clone()));
        assert!(requested);
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres-tls"]
    async fn query_session_live_tls_cursor_read_bound_command_and_stop() {
        let connection = connect(&live_spec(15433, true))
            .await
            .expect("self-signed TLS");
        connection
            .client
            .batch_execute("CREATE TEMP TABLE tls_rows(id int4, v text); INSERT INTO tls_rows VALUES (1, 'a'), (2, 'b'), (3, 'c')")
            .await
            .expect("fixture");
        let run = run_plan(
            &connection,
            "SELECT id, v FROM tls_rows WHERE id >= :from ORDER BY id",
            Some(&[("from", Some("1"))]),
            Some(2),
            TransactionEntry::Autocommit,
            TestControl::default(),
        )
        .await;
        assert!(
            matches!(run.outcome, Outcome::Completed),
            "{:?}",
            run.outcome
        );
        assert_eq!(run.rows.len(), 2);
        assert_eq!(run.completed, [(0, 2, Some(RowLimitOutcome::Stopped))]);

        let run = run_plan(
            &connection,
            "UPDATE tls_rows SET v = :v WHERE id = :id",
            Some(&[("v", Some("z")), ("id", Some("3"))]),
            None,
            TransactionEntry::Autocommit,
            TestControl::default(),
        )
        .await;
        assert_eq!(run.completed, [(0, 1, None)]);

        // A Stop during the FETCH, with the cancel request sent over TLS.
        let cancel = connection.cancel.clone();
        let read = run_plan(
            &connection,
            "SELECT pg_sleep(30)",
            None,
            Some(1),
            TransactionEntry::Autocommit,
            TestControl::default(),
        );
        let (run, requested) =
            tokio::join!(read, cancel_query_for_test(cancel, connection.tls.clone()));
        assert!(requested);
        assert!(matches!(
            run.outcome,
            Outcome::Failed(QuerySessionError::Database { code: Some(ref code), .. }) if code == "57014"
        ));
        // The wrapper was rolled back: the session socket is usable and idle.
        let state = connection
            .client
            .simple_query("SELECT count(*) FROM pg_cursors")
            .await
            .expect("usable after the stop");
        assert!(!state.is_empty());
    }

    async fn cancel_query_for_test(
        token: tokio_postgres::CancelToken,
        tls: dedicated::TlsConfig,
    ) -> bool {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel(token, tls).await
    }
}
