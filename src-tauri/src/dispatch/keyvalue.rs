//! KeyValue-class dispatch (Redis today).
//!
//! Counterpart to `dispatch::relational`. Routed to from `dispatch.rs`
//! when `engine.storage_class() == KeyValue`. Phase 1.2 surface:
//! capabilities probe (ping), keyspace SCAN, per-key metadata + per-
//! type fetchers. Phase 1.3+ adds CLI, pub/sub, server info.

use crate::redis::capabilities;
use crate::redis::cli::{
    self, CloseSessionPayload as CliCloseSessionPayload, RunCommandPayload, RunCommandResult,
};
use crate::redis::key_inspector::{
    self, FetchHashPayload, FetchJsonPayload, FetchListPayload, FetchSetPayload,
    FetchSortedSetPayload, FetchStreamGroupsPayload, FetchStreamPayload, FetchStringPayload,
    HashValuePayload, JsonValuePayload, KeyMetadata, KeyPayload, ListValuePayload, SetValuePayload,
    SortedSetValuePayload, StreamGroupsPayload, StreamValuePayload, StringValuePayload,
};
use crate::redis::key_ops::{
    self, ApplyListEditsPayload, BulkDeleteByPatternPayload, BulkDeleteByPatternResult,
    BulkExpireByPatternPayload, BulkExpireByPatternResult, CreateKeyPayload,
    CreateStreamGroupPayload, DelKeysPayload, DelKeysResult, DeleteHashFieldsPayload,
    DestroyStreamGroupPayload, JsonDeletePayload, JsonSetPayload, RenameKeyPayload,
    SetExpirePayload, SetHashFieldsPayload, SetMembersPayload, SetStringPayload, SetStringResult,
    SortedSetEditsPayload, StreamEditsPayload,
};
use crate::redis::keyspace::{
    self, CancelScanSessionPayload, CancelScanSessionResult, CloseScanSessionPayload,
    OpenScanSessionPayload, OpenScanSessionResult, ScanKeysPayload, ScanKeysResult,
};
use crate::redis::pubsub::{
    self, CloseSessionPayload, DiscoverChannelsPayload, DiscoverChannelsResult, DrainPayload,
    DrainResult, PublishPayload, PublishResult, StartSessionPayload, StartSessionResult,
};
use crate::redis::server_info::{
    self, AclListPayload, AclSelfPayload, ClientListPayload, ConfigPayload, KeyValueOverviewStats,
    LatencyPayload,
};
use crate::{ConnectResult, RedisStoredConnection, StoredConnection};

/// Narrow a `StoredConnection` to its Redis variant. Every function
/// in this module receives a `&StoredConnection` from the cross-class
/// router in `dispatch.rs`, which guarantees only Redis connections
/// reach us — but the routing contract is runtime, so we localize the
/// narrowing here once per call rather than asserting inside every
/// redis/* submodule.
fn as_redis(connection: &StoredConnection) -> Result<&RedisStoredConnection, String> {
    match connection {
        StoredConnection::Redis(r) => Ok(r),
        _ => Err(
            "keyvalue dispatch called with a non-Redis connection — router contract violated"
                .to_string(),
        ),
    }
}

/// Connect + run the capabilities probe to verify the server is live.
pub async fn ping_connection(connection: &StoredConnection) -> Result<ConnectResult, String> {
    let (latency_ms, caps) = capabilities::probe(as_redis(connection)?).await?;
    Ok(ConnectResult {
        latency_ms,
        redis_capabilities: Some(caps),
    })
}

pub async fn scan_keys(
    connection: &StoredConnection,
    payload: &ScanKeysPayload,
) -> Result<ScanKeysResult, String> {
    keyspace::scan_keys(as_redis(connection)?, payload).await
}

pub async fn open_scan_session(
    connection: &StoredConnection,
    payload: &OpenScanSessionPayload,
) -> Result<OpenScanSessionResult, String> {
    keyspace::open_session(as_redis(connection)?, payload).await
}

pub async fn cancel_scan_session(
    connection: &StoredConnection,
    payload: &CancelScanSessionPayload,
) -> Result<CancelScanSessionResult, String> {
    keyspace::cancel_session(as_redis(connection)?, payload).await
}

pub fn close_scan_session(payload: &CloseScanSessionPayload) {
    keyspace::close_session(payload);
}

pub async fn fetch_key_metadata(
    connection: &StoredConnection,
    payload: &KeyPayload,
) -> Result<KeyMetadata, String> {
    key_inspector::fetch_key_metadata(as_redis(connection)?, payload).await
}

pub async fn fetch_string(
    connection: &StoredConnection,
    payload: &FetchStringPayload,
) -> Result<StringValuePayload, String> {
    key_inspector::fetch_string(as_redis(connection)?, payload).await
}

pub async fn fetch_hash(
    connection: &StoredConnection,
    payload: &FetchHashPayload,
) -> Result<HashValuePayload, String> {
    key_inspector::fetch_hash(as_redis(connection)?, payload).await
}

pub async fn fetch_list(
    connection: &StoredConnection,
    payload: &FetchListPayload,
) -> Result<ListValuePayload, String> {
    key_inspector::fetch_list(as_redis(connection)?, payload).await
}

pub async fn fetch_set(
    connection: &StoredConnection,
    payload: &FetchSetPayload,
) -> Result<SetValuePayload, String> {
    key_inspector::fetch_set(as_redis(connection)?, payload).await
}

pub async fn fetch_sorted_set(
    connection: &StoredConnection,
    payload: &FetchSortedSetPayload,
) -> Result<SortedSetValuePayload, String> {
    key_inspector::fetch_sorted_set(as_redis(connection)?, payload).await
}

pub async fn fetch_stream(
    connection: &StoredConnection,
    payload: &FetchStreamPayload,
) -> Result<StreamValuePayload, String> {
    key_inspector::fetch_stream(as_redis(connection)?, payload).await
}

pub async fn fetch_stream_groups(
    connection: &StoredConnection,
    payload: &FetchStreamGroupsPayload,
) -> Result<StreamGroupsPayload, String> {
    key_inspector::fetch_stream_groups(as_redis(connection)?, payload).await
}

pub async fn fetch_json(
    connection: &StoredConnection,
    payload: &FetchJsonPayload,
) -> Result<JsonValuePayload, String> {
    key_inspector::fetch_json(as_redis(connection)?, payload).await
}

pub async fn run_command(
    connection: &StoredConnection,
    payload: &RunCommandPayload,
) -> Result<RunCommandResult, String> {
    cli::run_command(as_redis(connection)?, payload).await
}

pub fn cli_close_session(payload: &CliCloseSessionPayload) {
    cli::close_session(payload);
}

pub async fn fetch_overview(
    connection: &StoredConnection,
) -> Result<KeyValueOverviewStats, String> {
    server_info::fetch_overview(as_redis(connection)?).await
}

pub async fn fetch_client_list(connection: &StoredConnection) -> Result<ClientListPayload, String> {
    server_info::fetch_client_list(as_redis(connection)?).await
}

pub async fn fetch_acl_list(connection: &StoredConnection) -> Result<AclListPayload, String> {
    server_info::fetch_acl_list(as_redis(connection)?).await
}

pub async fn fetch_acl_self(connection: &StoredConnection) -> Result<AclSelfPayload, String> {
    server_info::fetch_acl_self(as_redis(connection)?).await
}

pub async fn fetch_config(
    connection: &StoredConnection,
    pattern: &str,
) -> Result<ConfigPayload, String> {
    server_info::fetch_config(as_redis(connection)?, pattern).await
}

pub async fn set_config(
    connection: &StoredConnection,
    key: &str,
    value: &str,
) -> Result<(), String> {
    server_info::set_config(as_redis(connection)?, key, value).await
}

pub async fn fetch_latency(connection: &StoredConnection) -> Result<LatencyPayload, String> {
    server_info::fetch_latency(as_redis(connection)?).await
}

pub async fn pubsub_start(
    app: &tauri::AppHandle,
    connection: &StoredConnection,
    payload: &StartSessionPayload,
) -> Result<StartSessionResult, String> {
    pubsub::start_session(app, as_redis(connection)?, payload).await
}

pub async fn pubsub_discover(
    connection: &StoredConnection,
    payload: &DiscoverChannelsPayload,
) -> Result<DiscoverChannelsResult, String> {
    pubsub::discover_channels(as_redis(connection)?, payload).await
}

pub fn pubsub_drain(payload: &DrainPayload) -> DrainResult {
    pubsub::drain(payload)
}

pub fn pubsub_close(payload: &CloseSessionPayload) {
    pubsub::close_session(payload);
}

pub async fn pubsub_publish(
    connection: &StoredConnection,
    payload: &PublishPayload,
) -> Result<PublishResult, String> {
    pubsub::publish(as_redis(connection)?, payload).await
}

pub async fn set_string(
    connection: &StoredConnection,
    payload: &SetStringPayload,
) -> Result<SetStringResult, String> {
    key_ops::set_string(as_redis(connection)?, payload).await
}

pub async fn set_hash_fields(
    connection: &StoredConnection,
    payload: &SetHashFieldsPayload,
) -> Result<(), String> {
    key_ops::set_hash_fields(as_redis(connection)?, payload).await
}

pub async fn delete_hash_fields(
    connection: &StoredConnection,
    payload: &DeleteHashFieldsPayload,
) -> Result<(), String> {
    key_ops::delete_hash_fields(as_redis(connection)?, payload).await
}

pub async fn del_keys(
    connection: &StoredConnection,
    payload: &DelKeysPayload,
) -> Result<DelKeysResult, String> {
    key_ops::del_keys(as_redis(connection)?, payload).await
}

pub async fn bulk_delete_by_pattern(
    connection: &StoredConnection,
    payload: &BulkDeleteByPatternPayload,
) -> Result<BulkDeleteByPatternResult, String> {
    key_ops::bulk_delete_by_pattern(as_redis(connection)?, payload).await
}

pub async fn bulk_expire_by_pattern(
    connection: &StoredConnection,
    payload: &BulkExpireByPatternPayload,
) -> Result<BulkExpireByPatternResult, String> {
    key_ops::bulk_expire_by_pattern(as_redis(connection)?, payload).await
}

pub async fn set_expire(
    connection: &StoredConnection,
    payload: &SetExpirePayload,
) -> Result<(), String> {
    key_ops::set_expire(as_redis(connection)?, payload).await
}

pub async fn rename_key(
    connection: &StoredConnection,
    payload: &RenameKeyPayload,
) -> Result<(), String> {
    key_ops::rename_key(as_redis(connection)?, payload).await
}

pub async fn create_key(
    connection: &StoredConnection,
    payload: &CreateKeyPayload,
) -> Result<(), String> {
    key_ops::create_key(as_redis(connection)?, payload).await
}

pub async fn apply_list_edits(
    connection: &StoredConnection,
    payload: &ApplyListEditsPayload,
) -> Result<(), String> {
    key_ops::apply_list_edits(as_redis(connection)?, payload).await
}

pub async fn apply_set_edits(
    connection: &StoredConnection,
    payload: &SetMembersPayload,
) -> Result<(), String> {
    key_ops::apply_set_edits(as_redis(connection)?, payload).await
}

pub async fn apply_sorted_set_edits(
    connection: &StoredConnection,
    payload: &SortedSetEditsPayload,
) -> Result<(), String> {
    key_ops::apply_sorted_set_edits(as_redis(connection)?, payload).await
}

pub async fn apply_stream_edits(
    connection: &StoredConnection,
    payload: &StreamEditsPayload,
) -> Result<(), String> {
    key_ops::apply_stream_edits(as_redis(connection)?, payload).await
}

pub async fn create_stream_group(
    connection: &StoredConnection,
    payload: &CreateStreamGroupPayload,
) -> Result<(), String> {
    key_ops::create_stream_group(as_redis(connection)?, payload).await
}

pub async fn destroy_stream_group(
    connection: &StoredConnection,
    payload: &DestroyStreamGroupPayload,
) -> Result<u64, String> {
    key_ops::destroy_stream_group(as_redis(connection)?, payload).await
}

pub async fn set_json_path(
    connection: &StoredConnection,
    payload: &JsonSetPayload,
) -> Result<(), String> {
    key_ops::set_json_path(as_redis(connection)?, payload).await
}

pub async fn delete_json_path(
    connection: &StoredConnection,
    payload: &JsonDeletePayload,
) -> Result<(), String> {
    key_ops::delete_json_path(as_redis(connection)?, payload).await
}
