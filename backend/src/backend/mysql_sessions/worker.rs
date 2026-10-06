//! The session worker: owns the connection and its SSH route, runs one
//! request at a time and closes for good on failure or retirement.
use std::sync::Arc;
use std::time::Instant;

use futures_util::TryStreamExt;
use sqlx::mysql::{MySqlConnectOptions, MySqlConnection, MySqlRow};
use sqlx::{Column, ConnectOptions, Connection, Either, Executor, Row};
use tokio::sync::{mpsc, watch, Mutex};

use super::rows::Collector;
use super::{
    Envelope, MySqlCancel, MySqlColumn, MySqlConstraint, MySqlForeignKey, MySqlIndex,
    MySqlObjectKind, MySqlObjects, MySqlResult, MySqlRoutine, MySqlRoutineKind, MySqlServerInfo,
    MySqlSessionError, MySqlSessionStatus, MySqlStructure, MySqlTrigger, Reply, Request, Shared,
    Target, Tracker, KILL_TIMEOUT, METADATA_KILL_GRACE, METADATA_TIMEOUT, MYSQL_MAX_CATALOG_ITEMS,
    QUEUE,
};
use crate::app::AppState;
use crate::backend::bastions::ProbeRoute;
use crate::backend::Inner;
use crate::safety::policy::{assert_permitted, AuditDisposition, SafetyRefusal, WriteIntent};
use crate::{quote_backtick, StoredConnection};

pub(super) struct Session {
    connection: MySqlConnection,
    thread: u64,
    server: MySqlServerInfo,
}

pub(super) struct Opened {
    pub connection_id: String,
    pub resolved: StoredConnection,
    pub route: ProbeRoute,
    pub options: MySqlConnectOptions,
    pub session: Session,
}

/// Connects and captures the server facts. A read-only record also makes the
/// server session read-only, in addition to the statement policy.
pub(super) async fn connect(
    options: &MySqlConnectOptions,
    read_only: bool,
) -> Result<Session, MySqlSessionError> {
    let host = options.get_host().to_owned();
    let port = options.get_port();
    let mut connection = options.connect().await.map_err(|error| {
        MySqlSessionError::Unavailable(crate::dispatch::friendly_sqlx_error(error, &host, port))
    })?;
    let setup = async {
        if read_only {
            connection
                .execute("SET SESSION TRANSACTION READ ONLY")
                .await?;
        }
        let row = connection
            .fetch_one("SELECT CONNECTION_ID(), VERSION(), DATABASE()")
            .await?;
        Ok::<_, sqlx::Error>(row)
    };
    let row = match setup.await {
        Ok(row) => row,
        Err(error) => {
            let _ = connection.close().await;
            return Err(MySqlSessionError::Unavailable(message(&error)));
        }
    };
    let text = |index: usize| text_at(&row, index);
    let thread = text(0).and_then(|id| id.parse().ok()).unwrap_or_default();
    let server = MySqlServerInfo {
        version: text(1).unwrap_or_default(),
        database: text(2),
        read_only,
    };
    Ok(Session {
        connection,
        thread,
        server,
    })
}

/// Registers and starts the worker. `None` when the backend is shutting down
/// (the connection is then closed here).
pub(super) fn spawn(
    inner: &Arc<Inner>,
    state: Arc<AppState>,
    opened: Opened,
) -> Option<super::MySqlSession> {
    let (retire, retired) = watch::channel(false);
    let Some(key) = inner.mysql.register(&opened.connection_id, retire.clone()) else {
        let connection = opened.session.connection;
        inner
            .tasks
            .track_task(tokio::spawn(async move { drop(connection.close().await) }));
        return None;
    };
    let (requests, receive) = mpsc::channel(QUEUE);
    let (status, observe) = watch::channel(MySqlSessionStatus::Open);
    let tracker = Arc::new(Mutex::new(Tracker::default()));
    let kill = (opened.options.clone(), opened.session.thread);
    let shared = Arc::new(Shared {
        connection_id: opened.connection_id.clone(),
        server: opened.session.server.clone(),
        requests,
        retire,
        status: observe,
        cancel: kill.clone(),
        tracker: tracker.clone(),
        next_id: std::sync::atomic::AtomicU64::new(0),
    });
    let owner = inner.clone();
    let default_database = opened.session.server.database.clone();
    let worker = Worker {
        connection: opened.session.connection,
        connection_id: opened.connection_id,
        resolved: opened.resolved,
        state,
        status,
        tracker,
        kill,
        current_database: default_database.clone(),
        default_database,
    };
    let route = opened.route;
    inner.tasks.track_task(tokio::spawn(async move {
        worker.run(receive, retired).await;
        drop(route);
        owner.mysql.remove(key);
    }));
    Some(super::MySqlSession(shared))
}

struct Worker {
    connection: MySqlConnection,
    connection_id: String,
    /// Secret and route applied; used for policy and the shared introspection.
    resolved: StoredConnection,
    state: Arc<AppState>,
    status: watch::Sender<MySqlSessionStatus>,
    tracker: Arc<Mutex<Tracker>>,
    /// Side connection target for interrupting an expired metadata request.
    kill: (MySqlConnectOptions, u64),
    /// The connection's default schema, pinned for scripts without one.
    default_database: Option<String>,
    /// `DATABASE()` as last observed on this connection.
    current_database: Option<String>,
}

impl Worker {
    /// Publishes `Closed` before any caller hears about the close, so a
    /// failed request and the status never disagree.
    async fn run(
        mut self,
        mut requests: mpsc::Receiver<Envelope>,
        mut retired: watch::Receiver<bool>,
    ) {
        let reason = loop {
            let next = tokio::select! {
                biased;
                _ = retired.wait_for(|retired| *retired) => break None,
                next = requests.recv() => next,
            };
            // Every handle dropped: nobody can use the session any more.
            let Some((id, request, reply)) = next else {
                break None;
            };
            // A cancelled request never runs; neither does one whose caller
            // is gone.
            if !self.tracker.lock().await.start(id) {
                let _ = reply.send(Err(MySqlSessionError::Cancelled));
                continue;
            }
            if reply.is_closed() {
                self.tracker.lock().await.finish(id);
                continue;
            }
            let outcome = tokio::select! {
                biased;
                // Abandoning a statement leaves the protocol state unknown,
                // so the connection is dropped rather than closed politely.
                _ = retired.wait_for(|retired| *retired) => {
                    self.status.send_replace(MySqlSessionStatus::Closed(None));
                    let _ = reply.send(Err(MySqlSessionError::Closed(None)));
                    return;
                }
                outcome = self.handle(request) => outcome,
            };
            // A cancel holds this lock while it sends `KILL QUERY`, so the
            // next request cannot start under a kill meant for this one.
            self.tracker.lock().await.finish(id);
            match outcome {
                Ok(value) => {
                    let _ = reply.send(Ok(value));
                }
                Err(Failure::Request(error)) => {
                    let _ = reply.send(Err(error));
                }
                Err(Failure::Fatal(reason)) => {
                    self.status
                        .send_replace(MySqlSessionStatus::Closed(Some(reason.clone())));
                    // Unknown protocol state: drop without COM_QUIT.
                    let _ = reply.send(Err(MySqlSessionError::Closed(Some(reason))));
                    return;
                }
            }
        };
        // Refuse new requests before the polite close finishes.
        self.status.send_replace(MySqlSessionStatus::Closed(reason));
        requests.close();
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(2), self.connection.close()).await;
    }

    async fn handle(&mut self, request: Request) -> Result<Reply, Failure> {
        match request {
            Request::Query {
                sql,
                database,
                confirmed,
            } => self
                .query(sql, database, confirmed)
                .await
                .map(Reply::Result),
            Request::Structure { database, table } => {
                // The shared introspection opens its own short connection over
                // the same resolved endpoint (the route is held by this
                // worker), so abandoning it leaves this session untouched.
                match tokio::time::timeout(
                    METADATA_TIMEOUT,
                    crate::dispatch::fetch_table_structure(&self.resolved, &database, &table),
                )
                .await
                {
                    Ok(result) => result
                        .map(|structure| Reply::Structure(structure_of(structure)))
                        .map_err(|error| Failure::Request(MySqlSessionError::Database(error))),
                    Err(_) => Err(Failure::Request(MySqlSessionError::Database(
                        TIMED_OUT.into(),
                    ))),
                }
            }
            request => self.bounded_metadata(request).await,
        }
    }

    /// Runs a metadata request on the session connection within
    /// `METADATA_TIMEOUT`. On expiry the request is interrupted with
    /// `KILL QUERY` and fails; only a request that still does not hand the
    /// connection back closes the session, since its state is then unknown.
    async fn bounded_metadata(&mut self, request: Request) -> Result<Reply, Failure> {
        let (options, thread) = self.kill.clone();
        let work = self.metadata(request);
        tokio::pin!(work);
        if let Ok(result) = tokio::time::timeout(METADATA_TIMEOUT, work.as_mut()).await {
            return result;
        }
        let interrupt = async move {
            let _ = tokio::time::timeout(KILL_TIMEOUT, kill_query(&options, thread)).await;
        };
        let (outcome, ()) = tokio::join!(
            tokio::time::timeout(METADATA_KILL_GRACE, work.as_mut()),
            interrupt
        );
        timed_out(outcome.ok())
    }

    async fn metadata(&mut self, request: Request) -> Result<Reply, Failure> {
        match request {
            Request::Databases => {
                let names = strings(
                    &mut self.connection,
                    "SELECT CAST(SCHEMA_NAME AS CHAR) FROM information_schema.SCHEMATA \
                     ORDER BY SCHEMA_NAME LIMIT ?",
                    &[],
                )
                .await?;
                let (names, truncated) = bounded(names.into_iter().map(first).collect());
                Ok(Reply::Databases(names, truncated))
            }
            Request::Objects(database) => self.objects(database).await.map(Reply::Objects),
            Request::Browse {
                database,
                table,
                offset,
                limit,
            } => self
                .browse(&database, &table, offset, limit)
                .await
                .map(Reply::Result),
            Request::Definition {
                database,
                kind,
                name,
            } => self
                .definition(&database, kind, &name)
                .await
                .map(Reply::Definition),
            Request::Query { .. } | Request::Structure { .. } => {
                unreachable!("handled before the bounded metadata path")
            }
        }
    }

    async fn objects(&mut self, database: String) -> Result<MySqlObjects, Failure> {
        let schema = database.as_str();
        let tables = strings(
            &mut self.connection,
            "SELECT CAST(TABLE_NAME AS CHAR), CAST(TABLE_TYPE AS CHAR) \
             FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? \
             ORDER BY TABLE_NAME LIMIT ?",
            &[schema],
        )
        .await?;
        let routines = strings(
            &mut self.connection,
            "SELECT CAST(ROUTINE_NAME AS CHAR), CAST(ROUTINE_TYPE AS CHAR) \
             FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = ? \
             ORDER BY ROUTINE_NAME LIMIT ?",
            &[schema],
        )
        .await?;
        let events = strings(
            &mut self.connection,
            "SELECT CAST(EVENT_NAME AS CHAR) FROM information_schema.EVENTS \
             WHERE EVENT_SCHEMA = ? ORDER BY EVENT_NAME LIMIT ?",
            &[schema],
        )
        .await?;
        let triggers = strings(
            &mut self.connection,
            "SELECT CAST(TRIGGER_NAME AS CHAR), CAST(EVENT_OBJECT_TABLE AS CHAR) \
             FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA = ? \
             ORDER BY TRIGGER_NAME LIMIT ?",
            &[schema],
        )
        .await?;
        Ok(objects_of(database, tables, routines, events, triggers))
    }

    async fn browse(
        &mut self,
        database: &str,
        table: &str,
        offset: u64,
        limit: u32,
    ) -> Result<MySqlResult, Failure> {
        // Unique indexes, the primary key first.
        let unique = strings(
            &mut self.connection,
            "SELECT CAST(INDEX_NAME AS CHAR), CAST(COLUMN_NAME AS CHAR), CAST(NULLABLE AS CHAR) \
             FROM information_schema.STATISTICS \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND NON_UNIQUE = 0 \
             ORDER BY INDEX_NAME = 'PRIMARY' DESC, INDEX_NAME, SEQ_IN_INDEX LIMIT ?",
            &[database, table],
        )
        .await?;
        let order = match unique_key(unique) {
            Some(columns) => BrowseOrder {
                columns,
                approximate: false,
            },
            // No key (or a view): every column in ordinal order.
            None => column_order(
                strings(
                    &mut self.connection,
                    "SELECT CAST(COLUMN_NAME AS CHAR), CAST(DATA_TYPE AS CHAR) \
                     FROM information_schema.COLUMNS \
                     WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                     ORDER BY ORDINAL_POSITION LIMIT ?",
                    &[database, table],
                )
                .await?,
            ),
        };
        let sql = browse_sql(database, table, &order.columns, offset, limit);
        let started = Instant::now();
        let mut collector = Collector::default();
        stream_into(&mut self.connection, &sql, &mut collector)
            .await
            .map_err(Failure::from_sqlx)?;
        let mut result = page(collector.finish(elapsed(started)), limit);
        result.approximate = order.approximate;
        Ok(result)
    }

    async fn definition(
        &mut self,
        database: &str,
        kind: MySqlObjectKind,
        name: &str,
    ) -> Result<String, Failure> {
        let sql = definition_sql(database, kind, name);
        let row = self
            .connection
            .fetch_optional(sql.as_str())
            .await
            .map_err(Failure::from_sqlx)?
            .ok_or_else(|| {
                Failure::Request(MySqlSessionError::Database("Object not found".into()))
            })?;
        let column = row
            .columns()
            .iter()
            .position(|column| {
                let name = column.name();
                name.starts_with("Create ") || name == "SQL Original Statement"
            })
            .ok_or_else(|| {
                Failure::Request(MySqlSessionError::Database(
                    "The server returned no definition".into(),
                ))
            })?;
        let raw = row.try_get_raw(column).map_err(Failure::from_sqlx)?;
        if sqlx::ValueRef::is_null(&raw) {
            // Definitions of objects owned by other accounts can be hidden.
            return Err(Failure::Request(MySqlSessionError::Database(
                "The definition is not visible to this account".into(),
            )));
        }
        let bytes = <&[u8] as sqlx::Decode<sqlx::MySql>>::decode(raw).unwrap_or_default();
        Ok(super::rows::render(bytes, super::rows::CellKind::Text, 1024 * 1024).0)
    }

    async fn query(
        &mut self,
        sql: String,
        database: Option<String>,
        confirmed: bool,
    ) -> Result<MySqlResult, Failure> {
        if sql.trim().is_empty() {
            return Err(Failure::Request(MySqlSessionError::Database(
                "Nothing to run".into(),
            )));
        }
        let (intent, audit, single_read) =
            authorize(&self.resolved, &sql, confirmed).map_err(Failure::Request)?;
        let target = pinned_database(
            database,
            self.default_database.as_deref(),
            self.current_database.as_deref(),
        )
        .map_err(|reason| Failure::Request(MySqlSessionError::Database(reason)))?;
        if let Some(database) = target {
            self.connection
                .execute(format!("USE {}", quote_backtick(&database)).as_str())
                .await
                .map_err(Failure::from_sqlx)?;
            self.current_database = Some(database);
        }
        // Audited as soon as execution starts: a script that fails part way
        // may already have changed data.
        if audit == AuditDisposition::RequiredAfterSuccess {
            crate::safety::gate::record_override(
                &self.state.pool,
                &self.connection_id,
                "mysql_query",
                &intent,
            )
            .await;
        }
        let started = Instant::now();
        let mut collector = Collector::default();
        if let Err(error) = stream_into(&mut self.connection, &sql, &mut collector).await {
            let failure = Failure::from_sqlx(error);
            if matches!(failure, Failure::Request(_)) {
                // The script may have switched databases before failing.
                self.refresh_database().await?;
            }
            return Err(failure);
        }
        let runtime = elapsed(started);
        // An empty SELECT carries no row to name its columns; describe it.
        if single_read && !collector.has_columns() {
            if let Ok(described) = self.connection.describe(sql.as_str()).await {
                collector.set_columns(
                    described
                        .columns()
                        .iter()
                        .map(|column| column.name().to_owned())
                        .collect(),
                );
            }
        }
        self.refresh_database().await?;
        let mut result = collector.finish(runtime);
        result.database = self.current_database.clone();
        Ok(result)
    }

    /// Re-reads `DATABASE()`; a statement error keeps the last known value.
    async fn refresh_database(&mut self) -> Result<(), Failure> {
        match self.connection.fetch_one("SELECT DATABASE()").await {
            Ok(row) => {
                self.current_database = text_at(&row, 0);
                Ok(())
            }
            Err(error) => match Failure::from_sqlx(error) {
                Failure::Fatal(reason) => Err(Failure::Fatal(reason)),
                Failure::Request(_) => Ok(()),
            },
        }
    }
}

/// The database a script runs in: the document's, else the connection
/// default. With neither, a script may only run while the session has no
/// database, so it never silently runs where another tab left the session.
pub(super) fn pinned_database(
    requested: Option<String>,
    default: Option<&str>,
    current: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(database) = requested.filter(|database| !database.trim().is_empty()) {
        return Ok(Some(database));
    }
    if let Some(database) = default.filter(|database| !database.trim().is_empty()) {
        return Ok(Some(database.to_owned()));
    }
    match current {
        None => Ok(None),
        Some(current) => Err(format!(
            "Choose a database for this tab; the session is currently using `{current}`"
        )),
    }
}

const TIMED_OUT: &str = "Request timed out and was stopped; the session is still open";

/// Outcome of an expired metadata request after `KILL QUERY`: `None` when
/// it did not hand the connection back within the grace period.
pub(super) fn timed_out<T>(outcome: Option<Result<T, Failure>>) -> Result<T, Failure> {
    match outcome {
        // It finished just as the kill was sent.
        Some(Ok(value)) => Ok(value),
        Some(Err(Failure::Fatal(reason))) => Err(Failure::Fatal(reason)),
        Some(Err(Failure::Request(_))) => Err(Failure::Request(MySqlSessionError::Database(
            TIMED_OUT.into(),
        ))),
        None => Err(Failure::Fatal(
            "Request timed out and could not be stopped; the connection was closed".into(),
        )),
    }
}

/// Streams a script into the collector.
async fn stream_into(
    connection: &mut MySqlConnection,
    sql: &str,
    collector: &mut Collector,
) -> Result<(), sqlx::Error> {
    let mut stream = connection.fetch_many(sql);
    while let Some(item) = stream.try_next().await? {
        match item {
            Either::Left(done) => collector.done(done.rows_affected()),
            Either::Right(row) => collector.row(&row),
        }
    }
    Ok(())
}

/// The native query policy: statements are classified with the shared SQL
/// classifier (unknown syntax is treated as a possible write) and checked
/// against the record's read-only flag and safety level. Also reports whether
/// the script is a single read, for column description.
pub(super) fn authorize(
    connection: &StoredConnection,
    sql: &str,
    confirmed: bool,
) -> Result<(WriteIntent, AuditDisposition, bool), MySqlSessionError> {
    let classes = crate::postgres::sql_class::classify_script_dialect(
        sql,
        crate::postgres::sql_class::SqlDialect::MySql,
    );
    let single_read = classes.len() == 1
        && matches!(classes[0], crate::postgres::sql_class::StatementClass::Read);
    let intent = WriteIntent::Statement { classes };
    let policy = crate::safety::gate::resolved_policy(connection);
    match assert_permitted(&policy, &intent, confirmed) {
        Ok(authorization) => Ok((intent, authorization.audit_disposition(), single_read)),
        Err(SafetyRefusal::Blocked { reason, .. }) => {
            Err(MySqlSessionError::ReadOnly(reason.into()))
        }
        Err(SafetyRefusal::NeedsConfirmation { statements }) => {
            Err(MySqlSessionError::NeedsConfirmation(statements))
        }
    }
}

/// Request failures keep the session; fatal ones close it.
#[derive(Debug)]
pub(super) enum Failure {
    Request(MySqlSessionError),
    Fatal(String),
}

impl Failure {
    pub(super) fn from_sqlx(error: sqlx::Error) -> Self {
        if is_fatal(&error) {
            Self::Fatal(message(&error))
        } else if is_interrupted(&error) {
            Self::Request(MySqlSessionError::Cancelled)
        } else {
            Self::Request(MySqlSessionError::Database(message(&error)))
        }
    }
}

/// 1317: the statement was stopped by `KILL QUERY`.
fn is_interrupted(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(error) if error.code().as_deref() == Some("1317"))
}

/// Transport and protocol failures, and server errors that end the
/// connection, are fatal. Ordinary statement errors are not.
pub(super) fn is_fatal(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(error) => {
            // 1053 shutdown in progress, 1927/4031 connection killed or idle
            // timeout, 3169 session killed, 2006/2013 server gone or lost.
            matches!(
                error.code().as_deref(),
                Some("1053" | "1927" | "4031" | "3169" | "2006" | "2013")
            )
        }
        sqlx::Error::Io(_)
        | sqlx::Error::Tls(_)
        | sqlx::Error::Protocol(_)
        | sqlx::Error::WorkerCrashed
        | sqlx::Error::PoolClosed
        | sqlx::Error::PoolTimedOut => true,
        _ => false,
    }
}

fn message(error: &sqlx::Error) -> String {
    match error {
        sqlx::Error::Database(error) => match error.code() {
            Some(code) => format!("{} ({code})", error.message()),
            None => error.message().to_owned(),
        },
        sqlx::Error::Io(error) => format!("Connection lost: {error}"),
        error => error.to_string(),
    }
}

fn elapsed(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

/// Runs a bound catalog query whose last placeholder is the item limit.
/// Every selected column must be cast to text.
async fn strings(
    connection: &mut MySqlConnection,
    sql: &str,
    binds: &[&str],
) -> Result<Vec<Vec<String>>, Failure> {
    let mut query = sqlx::query(sql);
    for bind in binds {
        query = query.bind(*bind);
    }
    query = query.bind((MYSQL_MAX_CATALOG_ITEMS + 1) as u64);
    let rows = query
        .fetch_all(connection)
        .await
        .map_err(Failure::from_sqlx)?;
    Ok(rows
        .iter()
        .map(|row| {
            (0..row.columns().len())
                .map(|index| {
                    row.try_get::<Option<String>, _>(index)
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                })
                .collect()
        })
        .collect())
}

fn first(row: Vec<String>) -> String {
    row.into_iter().next().unwrap_or_default()
}

/// One text-protocol value as a string; `None` for NULL.
fn text_at(row: &MySqlRow, index: usize) -> Option<String> {
    row.try_get_raw(index)
        .ok()
        .filter(|raw| !sqlx::ValueRef::is_null(raw))
        .and_then(|raw| <&str as sqlx::Decode<sqlx::MySql>>::decode(raw).ok())
        .map(str::to_owned)
}

/// How a browse page is ordered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BrowseOrder {
    pub columns: Vec<String>,
    /// Not a total order: pages may overlap or skip rows.
    pub approximate: bool,
}

/// The first unique index (primary key first) whose key parts are all plain,
/// non-null columns. Rows are `[index, column, nullable]` in index order;
/// functional key parts have no column name.
pub(super) fn unique_key(rows: Vec<Vec<String>>) -> Option<Vec<String>> {
    let mut indexes: Vec<(String, Vec<String>, bool)> = Vec::new();
    for row in rows {
        let mut row = row.into_iter();
        let index = row.next().unwrap_or_default();
        let column = row.next().unwrap_or_default();
        let nullable = row.next().unwrap_or_default();
        let usable = !column.is_empty() && !nullable.eq_ignore_ascii_case("YES");
        match indexes.last_mut() {
            Some((name, columns, ok)) if *name == index => {
                columns.push(column);
                *ok &= usable;
            }
            _ => indexes.push((index, vec![column], usable)),
        }
    }
    indexes
        .into_iter()
        .find(|(_, _, usable)| *usable)
        .map(|(_, columns, _)| columns)
}

/// Fallback order for objects without a usable unique key: every column in
/// ordinal order. Large text, binary, JSON and spatial values cannot be
/// ordered exactly, so they are left out and the order is approximate.
/// Rows are `[column, data type]`.
pub(super) fn column_order(rows: Vec<Vec<String>>) -> BrowseOrder {
    let mut columns = Vec::new();
    let mut approximate = false;
    for row in rows {
        let mut row = row.into_iter();
        let column = row.next().unwrap_or_default();
        let data_type = row.next().unwrap_or_default().to_ascii_lowercase();
        if column.is_empty() || inexact_order(&data_type) {
            approximate = true;
        } else {
            columns.push(column);
        }
    }
    approximate |= columns.is_empty();
    BrowseOrder {
        columns,
        approximate,
    }
}

fn inexact_order(data_type: &str) -> bool {
    matches!(
        data_type,
        "tinytext"
            | "text"
            | "mediumtext"
            | "longtext"
            | "tinyblob"
            | "blob"
            | "mediumblob"
            | "longblob"
            | "json"
            | "vector"
            | "geometry"
            | "point"
            | "linestring"
            | "polygon"
            | "multipoint"
            | "multilinestring"
            | "multipolygon"
            | "geometrycollection"
            | "geomcollection"
    )
}

/// Caps a list at the catalog budget; the flag reports the cut.
pub(super) fn bounded<T>(mut items: Vec<T>) -> (Vec<T>, bool) {
    let truncated = items.len() > MYSQL_MAX_CATALOG_ITEMS;
    items.truncate(MYSQL_MAX_CATALOG_ITEMS);
    (items, truncated)
}

/// Splits catalog rows into the tree's kinds. Tables include system and
/// versioned tables; MariaDB sequences are listed as tables.
pub(super) fn objects_of(
    database: String,
    tables: Vec<Vec<String>>,
    routines: Vec<Vec<String>>,
    events: Vec<Vec<String>>,
    triggers: Vec<Vec<String>>,
) -> MySqlObjects {
    let mut truncated = false;
    let mut cap = |rows: Vec<Vec<String>>| {
        let (rows, cut) = bounded(rows);
        truncated |= cut;
        rows
    };
    let (mut table_names, mut views) = (Vec::new(), Vec::new());
    for mut row in cap(tables) {
        let kind = row.pop().unwrap_or_default();
        let name = row.pop().unwrap_or_default();
        if kind.ends_with("VIEW") {
            views.push(name);
        } else {
            table_names.push(name);
        }
    }
    let routines = cap(routines)
        .into_iter()
        .map(|mut row| {
            let kind = row.pop().unwrap_or_default();
            MySqlRoutine {
                name: row.pop().unwrap_or_default(),
                kind: if kind.eq_ignore_ascii_case("FUNCTION") {
                    MySqlRoutineKind::Function
                } else {
                    MySqlRoutineKind::Procedure
                },
            }
        })
        .collect();
    let events = cap(events).into_iter().map(first).collect();
    let triggers = cap(triggers)
        .into_iter()
        .map(|mut row| {
            let table = row.pop().unwrap_or_default();
            MySqlTrigger {
                name: row.pop().unwrap_or_default(),
                table,
            }
        })
        .collect();
    MySqlObjects {
        database,
        tables: table_names,
        views,
        routines,
        events,
        triggers,
        truncated,
    }
}

/// `limit + 1` rows so the caller can tell whether another page exists.
pub(super) fn browse_sql(
    database: &str,
    table: &str,
    key: &[String],
    offset: u64,
    limit: u32,
) -> String {
    let order = if key.is_empty() {
        String::new()
    } else {
        format!(
            " ORDER BY {}",
            key.iter()
                .map(|column| quote_backtick(column))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    format!(
        "SELECT * FROM {}.{}{order} LIMIT {} OFFSET {offset}",
        quote_backtick(database),
        quote_backtick(table),
        u64::from(limit) + 1
    )
}

/// Trims the look-ahead row. `truncated` stays only for a byte-budget cut
/// inside the page itself; the rows it dropped belong to the next page, so
/// the caller advances by the rows kept, never by `limit`.
pub(super) fn page(mut result: MySqlResult, limit: u32) -> MySqlResult {
    let limit = limit as usize;
    let returned = (result.total_rows as usize).min(limit);
    result.rows.truncate(limit);
    result.truncated = result.rows.len() < returned;
    result.has_more = result.total_rows as usize > limit || result.truncated;
    result.total_rows = returned as u64;
    result
}

pub(super) fn definition_sql(database: &str, kind: MySqlObjectKind, name: &str) -> String {
    let keyword = match kind {
        MySqlObjectKind::Table => "TABLE",
        MySqlObjectKind::View => "VIEW",
        MySqlObjectKind::Procedure => "PROCEDURE",
        MySqlObjectKind::Function => "FUNCTION",
        MySqlObjectKind::Event => "EVENT",
        MySqlObjectKind::Trigger => "TRIGGER",
    };
    format!(
        "SHOW CREATE {keyword} {}.{}",
        quote_backtick(database),
        quote_backtick(name)
    )
}

fn structure_of(structure: crate::TableStructure) -> MySqlStructure {
    MySqlStructure {
        primary_key: structure.primary_key.unwrap_or_default(),
        columns: structure
            .columns
            .into_iter()
            .map(|column| MySqlColumn {
                name: column.name,
                data_type: column.data_type,
                nullable: column.nullable,
                default_value: column.default_value,
                primary_key: column.is_primary_key,
                generated: column.derivation_kind,
            })
            .collect(),
        indexes: structure
            .indexes
            .into_iter()
            .map(|index| MySqlIndex {
                name: index.name,
                columns: index.columns,
                unique: index.is_unique,
                primary: index.is_primary,
                method: index.method,
            })
            .collect(),
        foreign_keys: structure
            .foreign_keys
            .into_iter()
            .map(|key| MySqlForeignKey {
                name: key.name,
                columns: key.columns,
                referenced_schema: key.referenced_schema,
                referenced_table: key.referenced_table,
                referenced_columns: key.referenced_columns,
                on_update: key.on_update,
                on_delete: key.on_delete,
            })
            .collect(),
        constraints: structure
            .constraints
            .into_iter()
            .map(|constraint| MySqlConstraint {
                name: constraint.name,
                kind: constraint.kind,
                definition: constraint.definition,
            })
            .collect(),
    }
}

/// `KILL QUERY` from a short side connection, whatever is running. Only the
/// worker uses this, for its own expired request.
pub(super) async fn kill_query(options: &MySqlConnectOptions, thread: u64) -> Result<(), String> {
    if thread == 0 {
        return Err(UNKNOWN_THREAD.into());
    }
    let mut connection = options.connect().await.map_err(|error| message(&error))?;
    let result = kill(&mut connection, thread).await;
    let _ = connection.close().await;
    result
}

const UNKNOWN_THREAD: &str = "The session's server thread is unknown";

async fn kill(connection: &mut MySqlConnection, thread: u64) -> Result<(), String> {
    connection
        .execute(format!("KILL QUERY {thread}").as_str())
        .await
        .map(drop)
        .map_err(|error| message(&error))
}

/// Cancels request `id`: withdraws it while queued; while it runs, sends
/// `KILL QUERY` with the tracker locked, so the worker cannot finish it and
/// start another request in between. Anything else is left alone.
pub(super) async fn cancel(
    options: &MySqlConnectOptions,
    thread: u64,
    tracker: &Mutex<Tracker>,
    id: u64,
) -> Result<MySqlCancel, String> {
    let target = tracker.lock().await.cancel(id);
    match target {
        Target::Queued => return Ok(MySqlCancel::Withdrawn),
        Target::Idle => return Ok(MySqlCancel::Finished),
        Target::Running => {}
    }
    if thread == 0 {
        return Err(UNKNOWN_THREAD.into());
    }
    // Connect before taking the lock so the worker is not held up by it.
    let mut connection = options.connect().await.map_err(|error| message(&error))?;
    let result = {
        let tracker = tracker.lock().await;
        if tracker.running == Some(id) {
            kill(&mut connection, thread)
                .await
                .map(|()| MySqlCancel::Interrupted)
        } else {
            Ok(MySqlCancel::Finished)
        }
    };
    let _ = connection.close().await;
    result
}
