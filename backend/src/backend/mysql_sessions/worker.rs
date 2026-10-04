//! The session worker: owns the connection and its SSH route, runs one
//! request at a time and closes for good on failure or retirement.
use std::sync::Arc;
use std::time::Instant;

use futures_util::TryStreamExt;
use sqlx::mysql::{MySqlConnectOptions, MySqlConnection};
use sqlx::{Column, ConnectOptions, Connection, Either, Executor, Row};
use tokio::sync::{mpsc, watch};

use super::rows::Collector;
use super::{
    Envelope, MySqlColumn, MySqlConstraint, MySqlForeignKey, MySqlIndex, MySqlObjectKind,
    MySqlObjects, MySqlResult, MySqlRoutine, MySqlRoutineKind, MySqlServerInfo, MySqlSessionError,
    MySqlSessionStatus, MySqlStructure, MySqlTrigger, Reply, Request, Shared, METADATA_TIMEOUT,
    MYSQL_MAX_CATALOG_ITEMS, QUEUE,
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
    let text = |index: usize| {
        row.try_get_raw(index)
            .ok()
            .filter(|raw| !sqlx::ValueRef::is_null(raw))
            .and_then(|raw| <&str as sqlx::Decode<sqlx::MySql>>::decode(raw).ok())
            .map(str::to_owned)
    };
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
    let shared = Arc::new(Shared {
        connection_id: opened.connection_id.clone(),
        server: opened.session.server.clone(),
        requests,
        retire,
        status: observe,
        cancel: (opened.options.clone(), opened.session.thread),
    });
    let owner = inner.clone();
    let worker = Worker {
        connection: opened.session.connection,
        connection_id: opened.connection_id,
        resolved: opened.resolved,
        state,
        status,
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
            let Some((request, reply)) = next else {
                break None;
            };
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
            request => match tokio::time::timeout(METADATA_TIMEOUT, self.metadata(request)).await {
                Ok(result) => result,
                Err(_) => Err(Failure::Fatal(
                    "Request timed out; the connection was closed".into(),
                )),
            },
        }
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
            Request::Structure { database, table } => {
                // The shared introspection opens its own short connection over
                // the same resolved endpoint; the route is held by this worker.
                crate::dispatch::fetch_table_structure(&self.resolved, &database, &table)
                    .await
                    .map(|structure| Reply::Structure(structure_of(structure)))
                    .map_err(|error| Failure::Request(MySqlSessionError::Database(error)))
            }
            Request::Definition {
                database,
                kind,
                name,
            } => self
                .definition(&database, kind, &name)
                .await
                .map(Reply::Definition),
            Request::Query { .. } => unreachable!("queries are not metadata"),
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
        let key = strings(
            &mut self.connection,
            "SELECT CAST(COLUMN_NAME AS CHAR) FROM information_schema.KEY_COLUMN_USAGE \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND CONSTRAINT_NAME = 'PRIMARY' \
             ORDER BY ORDINAL_POSITION LIMIT ?",
            &[database, table],
        )
        .await?;
        let key: Vec<String> = key.into_iter().map(first).collect();
        let sql = browse_sql(database, table, &key, offset, limit);
        let started = Instant::now();
        let mut collector = Collector::default();
        {
            let mut stream = self.connection.fetch_many(sql.as_str());
            while let Some(item) = stream.try_next().await.map_err(Failure::from_sqlx)? {
                match item {
                    Either::Left(done) => collector.done(done.rows_affected()),
                    Either::Right(row) => collector.row(&row),
                }
            }
        }
        Ok(page(collector.finish(elapsed(started)), limit))
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
        if let Some(database) = database.filter(|database| !database.is_empty()) {
            self.connection
                .execute(format!("USE {}", quote_backtick(&database)).as_str())
                .await
                .map_err(Failure::from_sqlx)?;
        }
        let started = Instant::now();
        let mut collector = Collector::default();
        {
            let mut stream = self.connection.fetch_many(sql.as_str());
            while let Some(item) = stream.try_next().await.map_err(Failure::from_sqlx)? {
                match item {
                    Either::Left(done) => collector.done(done.rows_affected()),
                    Either::Right(row) => collector.row(&row),
                }
            }
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
        if audit == AuditDisposition::RequiredAfterSuccess {
            crate::safety::gate::record_override(
                &self.state.pool,
                &self.connection_id,
                "mysql_query",
                &intent,
            )
            .await;
        }
        Ok(collector.finish(runtime))
    }
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
    let classes = crate::postgres::sql_class::classify_script(sql);
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
        } else {
            Self::Request(MySqlSessionError::Database(message(&error)))
        }
    }
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
/// inside the page itself.
pub(super) fn page(mut result: MySqlResult, limit: u32) -> MySqlResult {
    let limit = limit as usize;
    let returned = (result.total_rows as usize).min(limit);
    result.has_more = result.total_rows as usize > limit;
    result.rows.truncate(limit);
    result.truncated = result.rows.len() < returned;
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

/// `KILL QUERY` from a short side connection.
pub(super) async fn kill_query(options: &MySqlConnectOptions, thread: u64) -> Result<(), String> {
    if thread == 0 {
        return Err("The session's server thread is unknown".into());
    }
    let mut connection = options.connect().await.map_err(|error| message(&error))?;
    let result = connection
        .execute(format!("KILL QUERY {thread}").as_str())
        .await
        .map(drop)
        .map_err(|error| message(&error));
    let _ = connection.close().await;
    result
}
