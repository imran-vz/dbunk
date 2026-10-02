//! Tauri command handlers, split by domain.
//!
//! Each sub-module groups `#[tauri::command]` functions by concern.
//! The shared helpers that multiple command modules need (connection
//! lookup, activity tracking) live in `crate::app` and are imported here so
//! `super::find_connection` keeps resolving. The `handler_list!`
//! invocations live in `lib.rs`.

pub(crate) mod bastions;
pub(crate) mod connections;
pub(crate) mod diagnosis;
pub(crate) mod keyvalue;
pub(crate) mod managed;
pub(crate) mod pg_backup;
pub(crate) mod pg_objects;
#[cfg(test)]
mod pg_objects_live_tests;
pub(crate) mod pg_schema_compare;
pub(crate) mod pg_transfer;
pub(crate) mod query_session;
pub(crate) mod relational;
pub(crate) mod result_mutation;
pub(crate) mod safety;
pub(crate) mod settings;
pub(crate) mod table_browse;
pub(crate) mod xlsx;

use crate::app::{
    current_credential_mode, find_connection, touch_connection_activity, with_active_connection,
    with_gated_active_connection,
};
use crate::host::{SharedSink, SinkClosed};

// ---------------------------------------------------------------------------
// Shared helpers — used by multiple command modules
// ---------------------------------------------------------------------------

/// Tauri side of the host seam: an IPC channel as an event sink. The event is
/// serialized here, at the edge, exactly as `Channel::send` always did.
pub(super) fn channel_sink<T>(channel: tauri::ipc::Channel<T>) -> SharedSink<T>
where
    T: serde::Serialize + Send + Sync + 'static,
{
    std::sync::Arc::new(move |event: T| channel.send(event).map_err(|_| SinkClosed))
}
