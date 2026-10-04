//! Plan 031 step 4: owned native Redis sessions.
//!
//! A session owns two dedicated connections, opened once under one deadline:
//! a console lane that keeps the user's `SELECT`/`MULTI` state, and a browse
//! lane for the keyspace tree and key inspector. Neither reconnects: a lost
//! connection or an unanswered command latches the session as lost and every
//! later call reports it until the host opens a new session. Pages, replies
//! and inspected values are bounded here, before they cross to the host.
use super::*;
use crate::redis::key_inspector::{
    self, FetchHashPayload, FetchListPayload, FetchSetPayload, FetchSortedSetPayload,
    FetchStreamPayload, FetchStringPayload, HashMode, KeyPayload, SetMode, ZsetMode,
};
use crate::redis::value::SerializedValue;
use crate::redis::{cli, connection as redis_connection, console_policy};
use crate::RedisStoredConnection;
use redis::aio::MultiplexedConnection;
use std::time::Duration;

/// Opening resolves credentials, the SSH route and both connections.
pub const REDIS_OPEN_TIMEOUT: Duration = Duration::from_secs(10);
/// Every command; an unanswered one latches the session as lost.
pub const REDIS_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
/// `COUNT` hint for one SCAN call.
pub const REDIS_SCAN_COUNT: u32 = 200;
/// SCAN calls per page; sparse keyspaces may return empty batches.
pub const REDIS_SCAN_CALLS: usize = 8;
/// Names returned by one page, whatever the server's batch sizes.
pub const REDIS_PAGE_KEYS: usize = 1_000;
/// Logical databases listed, matching the connection form's 0–15 range.
pub const REDIS_MAX_DATABASES: u8 = 16;
/// Console input bounds.
pub const REDIS_COMMAND_BYTES: usize = 64 * 1024;
pub const REDIS_COMMAND_TOKENS: usize = 1_024;
/// Reply budget shared by the console and the inspector.
pub const REDIS_REPLY_NODES: usize = 2_000;
pub const REDIS_REPLY_BYTES: usize = 256 * 1024;
/// Elements fetched for one inspected collection.
pub const REDIS_INSPECT_ITEMS: u32 = 200;
/// Prefix fetched for one inspected string.
pub const REDIS_INSPECT_STRING_BYTES: u32 = 64 * 1024;
const PATTERN_BYTES: usize = 1_024;

/// A bounded Redis reply. Strings are UTF-8 text or `0x` hex.
#[derive(Debug, Clone, PartialEq)]
pub enum RedisValue {
    Nil,
    Int(i64),
    Status(String),
    Text(String),
    /// Bytes that are not printable text, as `0x`-prefixed hex.
    Bytes(String),
    Error(String),
    Array(Vec<RedisValue>),
    /// Elements dropped by the reply budget.
    Omitted(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedisSessionError {
    /// The session can no longer be used; open a new one.
    Lost(String),
    /// This operation failed; the session stays usable.
    Failed(String),
}

impl std::fmt::Display for RedisSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lost(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedisDatabase {
    pub index: u8,
    /// Exact key total from `INFO keyspace`.
    pub keys: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedisOverview {
    pub databases: Vec<RedisDatabase>,
    pub default_db: u8,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedisKey {
    /// Raw key bytes; may not be UTF-8.
    pub name: Vec<u8>,
    /// `TYPE` reply (`string`, `hash`, …, or `none` when it vanished).
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedisScanPage {
    pub keys: Vec<RedisKey>,
    /// `None` when the scan is complete.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RedisConsoleOutcome {
    Reply {
        value: RedisValue,
        truncated: bool,
        elapsed_ms: u64,
        /// Console database after the command (tracks `SELECT`).
        db: u8,
    },
    /// Run again with `confirmed` to proceed.
    NeedsConfirmation {
        command: String,
        reason: String,
    },
    Refused {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RedisKeyValue {
    Missing,
    String { value: RedisValue, total_bytes: u64 },
    Hash(Vec<(RedisValue, RedisValue)>),
    List(Vec<RedisValue>),
    Set(Vec<RedisValue>),
    SortedSet(Vec<(RedisValue, f64)>),
    Stream(Vec<(String, Vec<(RedisValue, RedisValue)>)>),
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RedisKeyInspection {
    pub kind: String,
    /// `-1` = no expiry, `-2` = missing.
    pub ttl_seconds: i64,
    pub encoding: Option<String>,
    /// Collection length, or string length in bytes.
    pub length: Option<u64>,
    pub value: RedisKeyValue,
    /// The value shows only the first elements or bytes.
    pub truncated: bool,
}

struct Lane {
    connection: MultiplexedConnection,
    db: u8,
}

/// Policy resolved from the stored connection when the session opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedisPolicy {
    pub read_only: bool,
    /// Writes need an explicit confirmation (staging/production or a
    /// protected safe mode).
    pub confirm_writes: bool,
}

pub struct RedisSession {
    connection_id: String,
    policy: RedisPolicy,
    default_db: u8,
    console: tokio::sync::Mutex<Lane>,
    browse: tokio::sync::Mutex<Lane>,
    lost: std::sync::Mutex<Option<String>>,
    /// Keeps an SSH forward alive for the session; dropped with it.
    _route: Option<crate::backend::bastions::ProbeRoute>,
}

impl Backend {
    /// Opens one owned Redis session for a saved connection. Credentials and
    /// the SSH route are resolved under the development gate; the sockets
    /// open after it is released, all within [`REDIS_OPEN_TIMEOUT`]. There is
    /// no retry: a failure returns and the caller decides.
    pub async fn open_redis_session(&self, id: String) -> Result<RedisSession, String> {
        let authority = self.development()?;
        let deadline = tokio::time::Instant::now() + REDIS_OPEN_TIMEOUT;
        let (route, connection) = self
            .development_call(move |state| async move {
                Ok(async {
                    let _guard = credentials::mutation_guard(&state.credentials).await;
                    let rows = storage::read_native_connections(&state.pool).await?;
                    if rows
                        .iter()
                        .any(|(connection, valid)| !valid && connection.id() == id)
                    {
                        return Err("Stored connection options are unsupported or unreadable; metadata preserved".into());
                    }
                    let all = rows
                        .into_iter()
                        .map(|(connection, _)| connection)
                        .collect::<Vec<_>>();
                    let mut connection = supported(&all, &id, &authority)?.clone();
                    if !matches!(connection, StoredConnection::Redis(_)) {
                        return Err("Not a Redis connection".into());
                    }
                    if !credentials::onboarding_completed(&state.pool).await? {
                        return Err("Configure credential storage before connecting".into());
                    }
                    let mode = crate::app::current_credential_mode(&state).await?;
                    let secrets =
                        crate::credentials::read_all_cached(&state.credentials, mode).await?;
                    connection.set_password(secrets.get(&id).cloned().unwrap_or_default());
                    crate::backend::bastions::route_probe(&state, connection, deadline)
                        .await
                        .map_err(|reason| match reason {
                            DevelopmentConnectionFailure::SshHostKey => {
                                "The SSH host key changed or is not trusted".to_string()
                            }
                            DevelopmentConnectionFailure::Timeout => {
                                "Timed out opening the SSH route".to_string()
                            }
                            _ => "The SSH route could not be opened".to_string(),
                        })
                }
                .await)
            })
            .await
            .map_err(|_| "Native backend is closing".to_string())??;
        let StoredConnection::Redis(redis) = connection else {
            return Err("Not a Redis connection".into());
        };
        tokio::time::timeout_at(deadline, RedisSession::connect(redis, Some(route)))
            .await
            .map_err(|_| {
                format!(
                    "Timed out after {} s connecting to Redis",
                    REDIS_OPEN_TIMEOUT.as_secs()
                )
            })?
    }
}

impl RedisSession {
    async fn connect(
        connection: RedisStoredConnection,
        route: Option<crate::backend::bastions::ProbeRoute>,
    ) -> Result<Self, String> {
        let policy = crate::safety::policy::resolve_policy(
            StoredConnection::Redis(connection.clone()).policy(),
        );
        let (console, browse) = tokio::try_join!(
            redis_connection::open_oneshot(&connection),
            redis_connection::open_oneshot(&connection),
        )?;
        Ok(Self {
            connection_id: connection.id.clone(),
            policy: RedisPolicy {
                read_only: policy.read_only,
                confirm_writes: policy.level != crate::safety::policy::SafetyLevel::Disabled,
            },
            default_db: connection.db_number,
            console: tokio::sync::Mutex::new(Lane {
                connection: console,
                db: connection.db_number,
            }),
            browse: tokio::sync::Mutex::new(Lane {
                connection: browse,
                db: connection.db_number,
            }),
            lost: std::sync::Mutex::new(None),
            _route: route,
        })
    }

    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    pub fn policy(&self) -> RedisPolicy {
        self.policy
    }

    pub fn default_db(&self) -> u8 {
        self.default_db
    }

    fn check(&self) -> Result<(), RedisSessionError> {
        match self.lost.lock().unwrap().as_ref() {
            Some(reason) => Err(RedisSessionError::Lost(reason.clone())),
            None => Ok(()),
        }
    }

    fn lose(&self, reason: String) -> RedisSessionError {
        let mut lost = self.lost.lock().unwrap();
        RedisSessionError::Lost(lost.get_or_insert(reason).clone())
    }

    /// One bounded command. Transport failures and timeouts latch the
    /// session; server error replies are returned as `Failed`.
    async fn query<T: redis::FromRedisValue>(
        &self,
        connection: &mut MultiplexedConnection,
        command: &redis::Cmd,
    ) -> Result<T, RedisSessionError> {
        match tokio::time::timeout(REDIS_COMMAND_TIMEOUT, command.query_async(connection)).await {
            Err(_) => Err(self.lose(timeout_message())),
            Ok(Err(error)) if is_transport(&error) => {
                Err(self.lose(redis_connection::redis_err(error)))
            }
            Ok(Err(error)) => Err(RedisSessionError::Failed(redis_connection::redis_err(
                error,
            ))),
            Ok(Ok(value)) => Ok(value),
        }
    }

    async fn select(&self, lane: &mut Lane, db: u8) -> Result<(), RedisSessionError> {
        if db >= REDIS_MAX_DATABASES {
            return Err(RedisSessionError::Failed(format!(
                "Redis database must be 0–{} (got {db})",
                REDIS_MAX_DATABASES - 1
            )));
        }
        if lane.db != db {
            self.query::<()>(&mut lane.connection, redis::cmd("SELECT").arg(db))
                .await?;
            lane.db = db;
        }
        Ok(())
    }

    /// Database list with exact key totals, and the server version.
    /// `CONFIG GET databases` may be refused on managed servers; the list
    /// then covers every database that holds keys plus the default one.
    pub async fn overview(&self) -> Result<RedisOverview, RedisSessionError> {
        self.check()?;
        let mut lane = self.browse.lock().await;
        let keyspace: String = self
            .query(&mut lane.connection, redis::cmd("INFO").arg("keyspace"))
            .await?;
        let server: String = self
            .query(&mut lane.connection, redis::cmd("INFO").arg("server"))
            .await?;
        let configured = match self
            .query::<Vec<String>>(
                &mut lane.connection,
                redis::cmd("CONFIG").arg("GET").arg("databases"),
            )
            .await
        {
            Ok(reply) => reply.get(1).and_then(|count| count.parse::<u32>().ok()),
            Err(RedisSessionError::Failed(_)) => None,
            Err(lost) => return Err(lost),
        };
        Ok(RedisOverview {
            databases: databases(configured, &parse_keyspace(&keyspace), self.default_db),
            default_db: self.default_db,
            version: info_field(&server, "redis_version")
                .or_else(|| info_field(&server, "valkey_version")),
        })
    }

    /// One bounded keyspace page: up to [`REDIS_SCAN_CALLS`] SCAN calls until
    /// [`REDIS_SCAN_COUNT`] names arrive, then one pipelined `TYPE` per name.
    pub async fn scan(
        &self,
        db: u8,
        cursor: Option<String>,
        pattern: &str,
    ) -> Result<RedisScanPage, RedisSessionError> {
        self.check()?;
        if pattern.is_empty() || pattern.len() > PATTERN_BYTES {
            return Err(RedisSessionError::Failed(format!(
                "Key pattern must be 1–{PATTERN_BYTES} bytes"
            )));
        }
        let mut lane = self.browse.lock().await;
        self.select(&mut lane, db).await?;
        let mut cursor = cursor.unwrap_or_else(|| "0".into());
        let mut names: Vec<Vec<u8>> = Vec::new();
        for _ in 0..REDIS_SCAN_CALLS {
            let (next, batch): (String, Vec<Vec<u8>>) = self
                .query(
                    &mut lane.connection,
                    redis::cmd("SCAN")
                        .arg(&cursor)
                        .arg("MATCH")
                        .arg(pattern)
                        .arg("COUNT")
                        .arg(REDIS_SCAN_COUNT),
                )
                .await?;
            names.extend(batch);
            cursor = next;
            if cursor == "0" || names.len() >= REDIS_SCAN_COUNT as usize {
                break;
            }
        }
        // COUNT is a hint; a large batch is cut and resumed from the server's
        // cursor, so the cut names are skipped for this page only. Callers
        // treat pages as samples, never as an exact listing.
        names.truncate(REDIS_PAGE_KEYS);
        let kinds: Vec<String> = if names.is_empty() {
            Vec::new()
        } else {
            let mut pipe = redis::pipe();
            for name in &names {
                pipe.cmd("TYPE").arg(name);
            }
            match tokio::time::timeout(
                REDIS_COMMAND_TIMEOUT,
                pipe.query_async(&mut lane.connection),
            )
            .await
            {
                Err(_) => return Err(self.lose(timeout_message())),
                Ok(Err(error)) if is_transport(&error) => {
                    return Err(self.lose(redis_connection::redis_err(error)))
                }
                Ok(Err(error)) => {
                    return Err(RedisSessionError::Failed(redis_connection::redis_err(
                        error,
                    )))
                }
                Ok(Ok(kinds)) => kinds,
            }
        };
        Ok(RedisScanPage {
            keys: names
                .into_iter()
                .zip(kinds)
                .map(|(name, kind)| RedisKey { name, kind })
                .collect(),
            next_cursor: (cursor != "0").then_some(cursor),
        })
    }

    /// Runs one console command on the console lane after the CLI guard, the
    /// console policy and the connection's read-only/confirmation policy.
    pub async fn run(
        &self,
        tokens: Vec<String>,
        confirmed: bool,
    ) -> Result<RedisConsoleOutcome, RedisSessionError> {
        self.check()?;
        if let Some(refusal) = admit(&tokens, confirmed, self.policy) {
            return Ok(refusal);
        }
        let mut lane = self.console.lock().await;
        let mut command = redis::cmd(&tokens[0]);
        for argument in &tokens[1..] {
            command.arg(argument);
        }
        let started = std::time::Instant::now();
        let reply = self
            .query::<redis::Value>(&mut lane.connection, &command)
            .await;
        let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let value = match reply {
            Ok(value) => value,
            Err(RedisSessionError::Failed(message)) => {
                return Ok(RedisConsoleOutcome::Reply {
                    value: RedisValue::Error(message),
                    truncated: false,
                    elapsed_ms,
                    db: lane.db,
                })
            }
            Err(lost) => return Err(lost),
        };
        if tokens[0].eq_ignore_ascii_case("SELECT") && matches!(value, redis::Value::Okay) {
            if let Some(db) = tokens.get(1).and_then(|db| db.parse().ok()) {
                lane.db = db;
            }
        }
        let mut budget = Budget::default();
        let value = bound(crate::redis::value::serialize(value), &mut budget);
        Ok(RedisConsoleOutcome::Reply {
            value,
            truncated: budget.truncated,
            elapsed_ms,
            db: lane.db,
        })
    }

    /// Metadata and the first elements of one key on the browse lane.
    pub async fn inspect(
        &self,
        db: u8,
        key: String,
    ) -> Result<RedisKeyInspection, RedisSessionError> {
        self.check()?;
        let mut lane = self.browse.lock().await;
        self.select(&mut lane, db).await?;
        let result = tokio::time::timeout(
            REDIS_COMMAND_TIMEOUT,
            inspect_on(&mut lane.connection, &self.connection_id, key),
        )
        .await;
        match result {
            Err(_) => Err(self.lose(timeout_message())),
            Ok(Ok(inspection)) => Ok(inspection),
            Ok(Err(message)) => {
                // The inspector reports text only; a PING tells a server
                // error apart from a dead connection.
                self.query::<String>(&mut lane.connection, &redis::cmd("PING"))
                    .await?;
                Err(RedisSessionError::Failed(message))
            }
        }
    }
}

fn timeout_message() -> String {
    format!(
        "Redis did not answer within {} s; the session was closed",
        REDIS_COMMAND_TIMEOUT.as_secs()
    )
}

fn is_transport(error: &redis::RedisError) -> bool {
    error.is_unrecoverable_error() || error.is_timeout() || error.is_connection_dropped()
}

/// The pre-flight decision for one console command; `None` runs it.
pub(crate) fn admit(
    tokens: &[String],
    confirmed: bool,
    policy: RedisPolicy,
) -> Option<RedisConsoleOutcome> {
    if tokens.len() > REDIS_COMMAND_TOKENS
        || tokens.iter().map(String::len).sum::<usize>() > REDIS_COMMAND_BYTES
    {
        return Some(RedisConsoleOutcome::Refused {
            reason: format!(
                "Commands are limited to {REDIS_COMMAND_TOKENS} arguments and {} KiB",
                REDIS_COMMAND_BYTES / 1024
            ),
        });
    }
    if let Some(refusal) = cli::guard(tokens, confirmed) {
        return Some(match refusal {
            cli::RunCommandResult::NeedsConfirmation { command, severity } => {
                RedisConsoleOutcome::NeedsConfirmation {
                    reason: if severity == "soft" {
                        format!("{command} can block the server on a large keyspace")
                    } else {
                        format!("{command} is destructive")
                    },
                    command,
                }
            }
            cli::RunCommandResult::Rejected { reason } => RedisConsoleOutcome::Refused {
                reason: reason.replace("Pub/Sub tab", "a Pub/Sub tool"),
            },
            cli::RunCommandResult::Ok { .. } => unreachable!("guard never runs a command"),
        });
    }
    match console_policy::classify(tokens) {
        console_policy::ConsoleCommand::Refused(reason) => {
            Some(RedisConsoleOutcome::Refused { reason })
        }
        console_policy::ConsoleCommand::Read => None,
        console_policy::ConsoleCommand::Write if policy.read_only => {
            Some(RedisConsoleOutcome::Refused {
                reason: "This connection is read-only".into(),
            })
        }
        console_policy::ConsoleCommand::Write if policy.confirm_writes && !confirmed => {
            Some(RedisConsoleOutcome::NeedsConfirmation {
                command: tokens[0].to_uppercase(),
                reason: "Writes on this connection need confirmation".into(),
            })
        }
        console_policy::ConsoleCommand::Write => None,
    }
}

async fn inspect_on(
    connection: &mut MultiplexedConnection,
    connection_id: &str,
    key: String,
) -> Result<RedisKeyInspection, String> {
    let id = || connection_id.to_owned();
    let metadata = key_inspector::fetch_key_metadata_on(
        connection,
        &KeyPayload {
            connection_id: id(),
            key: key.clone(),
        },
    )
    .await?;
    let items = u64::from(REDIS_INSPECT_ITEMS);
    let mut budget = Budget::default();
    let mut longer = metadata.element_count.is_some_and(|count| count > items);
    let value = match metadata.r#type.as_str() {
        "none" => RedisKeyValue::Missing,
        "string" => {
            let reply = key_inspector::fetch_string_on(
                connection,
                &FetchStringPayload {
                    connection_id: id(),
                    key,
                    max_bytes: REDIS_INSPECT_STRING_BYTES,
                },
            )
            .await?;
            longer = reply.truncated;
            RedisKeyValue::String {
                value: bound(reply.value, &mut budget),
                total_bytes: reply.total_bytes,
            }
        }
        "hash" => {
            let reply = key_inspector::fetch_hash_on(
                connection,
                &FetchHashPayload {
                    connection_id: id(),
                    key,
                    mode: HashMode::Scan,
                    count: REDIS_INSPECT_ITEMS,
                    cursor: None,
                    pattern: None,
                },
            )
            .await?;
            longer |= reply.next_cursor.is_some();
            RedisKeyValue::Hash(bound_pairs(reply.entries, &mut budget))
        }
        "list" => {
            let reply = key_inspector::fetch_list_on(
                connection,
                &FetchListPayload {
                    connection_id: id(),
                    key,
                    start: 0,
                    stop: i64::from(REDIS_INSPECT_ITEMS) - 1,
                    reverse: false,
                },
            )
            .await?;
            RedisKeyValue::List(bound_all(reply.items, &mut budget))
        }
        "set" => {
            let reply = key_inspector::fetch_set_on(
                connection,
                &FetchSetPayload {
                    connection_id: id(),
                    key,
                    mode: SetMode::Scan,
                    count: REDIS_INSPECT_ITEMS,
                    cursor: None,
                    pattern: None,
                },
            )
            .await?;
            longer |= reply.next_cursor.is_some();
            RedisKeyValue::Set(bound_all(reply.members, &mut budget))
        }
        "zset" => {
            let reply = key_inspector::fetch_sorted_set_on(
                connection,
                &FetchSortedSetPayload {
                    connection_id: id(),
                    key,
                    mode: ZsetMode::Rank,
                    start: 0,
                    stop: i64::from(REDIS_INSPECT_ITEMS) - 1,
                    reverse: false,
                    score_min: None,
                    score_max: None,
                },
            )
            .await?;
            RedisKeyValue::SortedSet(
                reply
                    .entries
                    .into_iter()
                    .take(REDIS_INSPECT_ITEMS as usize)
                    .map(|(member, score)| (bound(member, &mut budget), score))
                    .collect(),
            )
        }
        "stream" => {
            let reply = key_inspector::fetch_stream_on(
                connection,
                &FetchStreamPayload {
                    connection_id: id(),
                    key,
                    start: "-".into(),
                    end: "+".into(),
                    count: REDIS_INSPECT_ITEMS,
                    reverse: false,
                },
            )
            .await?;
            RedisKeyValue::Stream(
                reply
                    .entries
                    .into_iter()
                    .take(REDIS_INSPECT_ITEMS as usize)
                    .map(|entry| (entry.id, bound_pairs(entry.fields, &mut budget)))
                    .collect(),
            )
        }
        other => RedisKeyValue::Unsupported(format!(
            "The {other} type has no viewer yet; use the console"
        )),
    };
    Ok(RedisKeyInspection {
        kind: metadata.r#type,
        ttl_seconds: metadata.ttl_seconds,
        encoding: metadata.encoding,
        length: metadata.element_count,
        value,
        truncated: longer || budget.truncated,
    })
}

struct Budget {
    nodes: usize,
    bytes: usize,
    truncated: bool,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            nodes: REDIS_REPLY_NODES,
            bytes: REDIS_REPLY_BYTES,
            truncated: false,
        }
    }
}

fn bound_all(values: Vec<SerializedValue>, budget: &mut Budget) -> Vec<RedisValue> {
    let total = values.len();
    let mut out = Vec::with_capacity(total.min(REDIS_INSPECT_ITEMS as usize));
    for (index, value) in values.into_iter().enumerate() {
        if budget.nodes == 0 || index >= REDIS_INSPECT_ITEMS as usize {
            budget.truncated = true;
            break;
        }
        out.push(bound(value, budget));
    }
    out
}

fn bound_pairs(
    pairs: Vec<(SerializedValue, SerializedValue)>,
    budget: &mut Budget,
) -> Vec<(RedisValue, RedisValue)> {
    let mut out = Vec::new();
    for (index, (field, value)) in pairs.into_iter().enumerate() {
        if budget.nodes < 2 || index >= REDIS_INSPECT_ITEMS as usize {
            budget.truncated = true;
            break;
        }
        out.push((bound(field, budget), bound(value, budget)));
    }
    out
}

/// Converts a reply within the node and byte budget. Text is cut on a
/// character boundary with an ellipsis; arrays end with `Omitted(n)`.
fn bound(value: SerializedValue, budget: &mut Budget) -> RedisValue {
    budget.nodes = budget.nodes.saturating_sub(1);
    let mut text = |value: String| {
        if value.len() <= budget.bytes {
            budget.bytes -= value.len();
            return value;
        }
        let mut end = budget.bytes;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        budget.bytes = 0;
        budget.truncated = true;
        format!("{}…", &value[..end])
    };
    match value {
        SerializedValue::Nil => RedisValue::Nil,
        SerializedValue::Int { value } => RedisValue::Int(value),
        SerializedValue::Status { value } => RedisValue::Status(text(value)),
        SerializedValue::Error { value } => RedisValue::Error(text(value)),
        SerializedValue::String { value, encoding } if encoding == "hex" => {
            RedisValue::Bytes(text(value))
        }
        SerializedValue::String { value, .. } => RedisValue::Text(text(value)),
        SerializedValue::Array { value } => {
            let total = value.len();
            let mut out = Vec::new();
            for element in value {
                if budget.nodes == 0 || budget.bytes == 0 {
                    break;
                }
                out.push(bound(element, budget));
            }
            if out.len() < total {
                budget.truncated = true;
                out.push(RedisValue::Omitted(total - out.len()));
            }
            RedisValue::Array(out)
        }
    }
}

/// `dbN` → key total from `INFO keyspace` lines like `db0:keys=3,expires=0`.
pub(crate) fn parse_keyspace(info: &str) -> Vec<(u8, u64)> {
    info.lines()
        .filter_map(|line| {
            let (name, fields) = line.trim().split_once(':')?;
            let index = name.strip_prefix("db")?.parse::<u8>().ok()?;
            let keys = fields
                .split(',')
                .find_map(|field| field.strip_prefix("keys="))?
                .parse()
                .ok()?;
            Some((index, keys))
        })
        .collect()
}

fn info_field(info: &str, name: &str) -> Option<String> {
    info.lines().find_map(|line| {
        let (field, value) = line.trim().split_once(':')?;
        (field == name).then(|| value.to_owned())
    })
}

/// Every configured database up to [`REDIS_MAX_DATABASES`]; without a
/// configured count, the default database and every one holding keys.
pub(crate) fn databases(
    configured: Option<u32>,
    keyspace: &[(u8, u64)],
    default_db: u8,
) -> Vec<RedisDatabase> {
    let keys = |index: u8| {
        keyspace
            .iter()
            .find(|(db, _)| *db == index)
            .map_or(0, |(_, keys)| *keys)
    };
    let indexes: Vec<u8> = match configured {
        Some(count) => (0..count.min(u32::from(REDIS_MAX_DATABASES)) as u8).collect(),
        None => {
            let mut indexes: Vec<u8> = keyspace
                .iter()
                .map(|(db, _)| *db)
                .chain([default_db])
                .filter(|db| *db < REDIS_MAX_DATABASES)
                .collect();
            indexes.sort_unstable();
            indexes.dedup();
            indexes
        }
    };
    indexes
        .into_iter()
        .map(|index| RedisDatabase {
            index,
            keys: keys(index),
        })
        .collect()
}

#[cfg(test)]
#[path = "redis_session_tests.rs"]
mod tests;
