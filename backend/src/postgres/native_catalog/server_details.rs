//! Bounded, reader-session server inspection. No settings or extension writes.
use super::*;
use serde::Serialize;
use std::{fmt, mem::size_of};

mod bounds;
mod queries;
mod reader;
pub const MAX_SERVER_DETAILS_BYTES: usize = 1024 * 1024;
pub const MAX_SERVER_SETTINGS: usize = 1024;
pub const MAX_SERVER_EXTENSIONS: usize = 256;
pub const MAX_SERVER_TEXT_BYTES: usize = 8192;
pub const MAX_SERVER_IDENTITY_BYTES: usize = 256;
const HEADER_BYTES: usize = 64 * 1024;
const SETTINGS_BYTES: usize = 768 * 1024;
const EXTENSIONS_BYTES: usize = 192 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerLimit {
    RowLimit,
    ByteLimit,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "value", rename_all = "camelCase")]
pub enum ServerSection<T> {
    Loaded(T),
    Restricted,
    Unavailable,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerRows<T> {
    pub rows: Vec<T>,
    pub limit: Option<ServerLimit>,
}

// Default owned storage is the public DTO. Borrowed instances let the reader
// validate and count the identical serialized shape before copying wire text.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "value", rename_all = "camelCase")]
pub enum ServerText<S = String> {
    Value(S),
    Null,
    Omitted { bytes: u64 },
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderContext<S = String> {
    pub current_user: ServerText<S>,
    pub session_user: ServerText<S>,
    pub search_path: ServerText<S>,
    pub statement_timeout_ms: u32,
    pub lock_timeout_ms: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerFacts<S = String> {
    pub server_version: ServerText<S>,
    pub encoding: ServerText<S>,
    /// The current database's LC_COLLATE catalog value, not a session GUC or ICU locale.
    pub locale: ServerText<S>,
    pub timezone: ServerText<S>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSetting<S = String> {
    pub name: S,
    pub category: S,
    pub source: S,
    pub setting: ServerText<S>,
    pub unit: ServerText<S>,
    pub short_desc: ServerText<S>,
    pub boot_val: ServerText<S>,
    pub reset_val: ServerText<S>,
    pub inspection_override: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerExtension<S = String> {
    pub name: S,
    pub schema: S,
    pub version: ServerText<S>,
    pub description: ServerText<S>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDetailsSnapshot {
    pub database: String,
    pub reader_pid: i32,
    pub collected_start: String,
    pub collected_end: String,
    pub reader: ReaderContext,
    pub facts: ServerSection<ServerFacts>,
    pub settings: ServerSection<ServerRows<ServerSetting>>,
    pub extensions: ServerSection<ServerRows<ServerExtension>>,
}

impl ServerDetailsSnapshot {
    /// Allocation-free validation of all field, row, capacity and encoded byte
    /// limits. Returns retained DTO bytes including vector backing and strings.
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        bounds::snapshot(self)
    }
}

// Settings can contain credentials. Debug reports shape only, including for
// nested values accidentally included in a diagnostic or assertion.
impl<S> fmt::Debug for ServerText<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(_) => f.write_str("Value(<redacted>)"),
            Self::Null => f.write_str("Null"),
            Self::Omitted { bytes } => f.debug_struct("Omitted").field("bytes", bytes).finish(),
        }
    }
}
impl<T> fmt::Debug for ServerSection<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Loaded(_) => "Loaded(<redacted>)",
            Self::Restricted => "Restricted",
            Self::Unavailable => "Unavailable",
        })
    }
}
impl<T> fmt::Debug for ServerRows<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerRows")
            .field("rows", &self.rows.len())
            .field("limit", &self.limit)
            .finish()
    }
}
macro_rules! redacted_debug {
    ($($name:ident),+) => { $(impl<S> fmt::Debug for $name<S> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(concat!(stringify!($name), "(<redacted>)")) }
    })+ };
}
redacted_debug!(ReaderContext, ServerFacts, ServerSetting, ServerExtension);
impl fmt::Debug for ServerDetailsSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerDetailsSnapshot")
            .field("facts", &self.facts)
            .field("settings", &self.settings)
            .field("extensions", &self.extensions)
            .finish_non_exhaustive()
    }
}

pub(crate) async fn read(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
) -> Result<ServerDetailsSnapshot, CatalogError> {
    owned_read(
        spec,
        drivers,
        cancellation,
        Duration::from_secs(30),
        |client, timeout| Box::pin(reader::load(client, timeout)),
    )
    .await
}

#[cfg(test)]
mod tests;
