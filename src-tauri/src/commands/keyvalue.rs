//! Redis / keyvalue-engine commands.

use tauri::State;

use crate::dispatch;
use crate::redis;
use crate::storage;
use crate::{AppState, ConnectionPayload, RedisCliHistoryEntry, SavedRedisCommand};

use super::with_active_connection;

// ---------------------------------------------------------------------------
// Saved Redis commands + CLI history
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn load_redis_cli_history(
    state: State<'_, AppState>,
    connection_id: String,
    limit: Option<u32>,
) -> Result<Vec<RedisCliHistoryEntry>, String> {
    storage::read_redis_cli_history(&state.inner().pool, &connection_id, limit).await
}

#[tauri::command]
pub async fn append_redis_cli_history(
    state: State<'_, AppState>,
    entry: RedisCliHistoryEntry,
) -> Result<(), String> {
    storage::insert_redis_cli_history(&state.inner().pool, &entry).await
}

#[tauri::command]
pub async fn load_saved_redis_commands(
    state: State<'_, AppState>,
) -> Result<Vec<SavedRedisCommand>, String> {
    storage::read_saved_redis_commands(&state.inner().pool).await
}

#[tauri::command]
pub async fn save_saved_redis_command(
    state: State<'_, AppState>,
    command: SavedRedisCommand,
) -> Result<Vec<SavedRedisCommand>, String> {
    let state = state.inner();
    let now = chrono::Utc::now().to_rfc3339();
    let mut next = command.clone();
    next.updated_at = now.clone();
    if command.created_at.is_empty() {
        next.created_at = now;
    }
    storage::upsert_saved_redis_command(&state.pool, &next).await?;
    storage::read_saved_redis_commands(&state.pool).await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSavedRedisCommandPayload {
    id: String,
}

#[tauri::command]
pub async fn delete_saved_redis_command(
    state: State<'_, AppState>,
    payload: DeleteSavedRedisCommandPayload,
) -> Result<Vec<SavedRedisCommand>, String> {
    let state = state.inner();
    storage::delete_saved_redis_command(&state.pool, &payload.id).await?;
    storage::read_saved_redis_commands(&state.pool).await
}

// ---------------------------------------------------------------------------
// Keyspace scanning
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn redis_scan_keys(
    state: State<'_, AppState>,
    payload: redis::keyspace::ScanKeysPayload,
) -> Result<redis::keyspace::ScanKeysResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::scan_keys(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_open_scan_session(
    state: State<'_, AppState>,
    payload: redis::keyspace::OpenScanSessionPayload,
) -> Result<redis::keyspace::OpenScanSessionResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::open_scan_session(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_cancel_scan_session(
    state: State<'_, AppState>,
    payload: redis::keyspace::CancelScanSessionPayload,
) -> Result<redis::keyspace::CancelScanSessionResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::cancel_scan_session(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub fn redis_close_scan_session(payload: redis::keyspace::CloseScanSessionPayload) {
    dispatch::keyvalue::close_scan_session(&payload);
}

// ---------------------------------------------------------------------------
// Key inspection
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn redis_fetch_key_metadata(
    state: State<'_, AppState>,
    payload: redis::key_inspector::KeyPayload,
) -> Result<redis::key_inspector::KeyMetadata, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_key_metadata(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_fetch_string(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchStringPayload,
) -> Result<redis::key_inspector::StringValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_string(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_fetch_hash(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchHashPayload,
) -> Result<redis::key_inspector::HashValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_hash(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_fetch_list(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchListPayload,
) -> Result<redis::key_inspector::ListValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_list(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_fetch_set(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchSetPayload,
) -> Result<redis::key_inspector::SetValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_set(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_fetch_sorted_set(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchSortedSetPayload,
) -> Result<redis::key_inspector::SortedSetValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_sorted_set(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_fetch_stream(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchStreamPayload,
) -> Result<redis::key_inspector::StreamValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_stream(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_fetch_stream_groups(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchStreamGroupsPayload,
) -> Result<redis::key_inspector::StreamGroupsPayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_stream_groups(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_fetch_json(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchJsonPayload,
) -> Result<redis::key_inspector::JsonValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_json(&connection, &payload).await
    })
    .await
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn redis_cli_close_session(payload: redis::cli::CloseSessionPayload) {
    dispatch::keyvalue::cli_close_session(&payload);
}

#[tauri::command]
pub async fn redis_run_command(
    state: State<'_, AppState>,
    payload: redis::cli::RunCommandPayload,
) -> Result<redis::cli::RunCommandResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::run_command(&connection, &payload).await
    })
    .await
}

// ---------------------------------------------------------------------------
// Server info
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn redis_fetch_overview(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<redis::server_info::KeyValueOverviewStats, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::keyvalue::fetch_overview(&connection).await },
    )
    .await
}

#[tauri::command]
pub async fn redis_fetch_client_list(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<redis::server_info::ClientListPayload, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::keyvalue::fetch_client_list(&connection).await },
    )
    .await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchAclSelfPayload {
    connection_id: String,
}

#[tauri::command]
pub async fn redis_fetch_acl_self(
    state: State<'_, AppState>,
    payload: FetchAclSelfPayload,
) -> Result<redis::server_info::AclSelfPayload, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::keyvalue::fetch_acl_self(&connection).await },
    )
    .await
}

#[tauri::command]
pub async fn redis_fetch_acl_list(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<redis::server_info::AclListPayload, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::keyvalue::fetch_acl_list(&connection).await },
    )
    .await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchConfigPayload {
    connection_id: String,
    #[serde(default = "default_star")]
    pattern: String,
}

fn default_star() -> String {
    "*".to_string()
}

#[tauri::command]
pub async fn redis_fetch_config(
    state: State<'_, AppState>,
    payload: FetchConfigPayload,
) -> Result<redis::server_info::ConfigPayload, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move {
            dispatch::keyvalue::fetch_config(&connection, &payload.pattern).await
        },
    )
    .await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetConfigPayload {
    connection_id: String,
    key: String,
    value: String,
}

#[tauri::command]
pub async fn redis_set_config(
    state: State<'_, AppState>,
    payload: SetConfigPayload,
) -> Result<(), String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move {
            dispatch::keyvalue::set_config(&connection, &payload.key, &payload.value).await
        },
    )
    .await
}

#[tauri::command]
pub async fn redis_fetch_latency(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<redis::server_info::LatencyPayload, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::keyvalue::fetch_latency(&connection).await },
    )
    .await
}

// ---------------------------------------------------------------------------
// Pub/Sub
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn redis_pubsub_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: redis::pubsub::StartSessionPayload,
) -> Result<redis::pubsub::StartSessionResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::pubsub_start(&app, &connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_pubsub_discover(
    state: State<'_, AppState>,
    payload: redis::pubsub::DiscoverChannelsPayload,
) -> Result<redis::pubsub::DiscoverChannelsResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::pubsub_discover(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub fn redis_pubsub_drain(payload: redis::pubsub::DrainPayload) -> redis::pubsub::DrainResult {
    dispatch::keyvalue::pubsub_drain(&payload)
}

#[tauri::command]
pub fn redis_pubsub_close(payload: redis::pubsub::CloseSessionPayload) {
    dispatch::keyvalue::pubsub_close(&payload);
}

#[tauri::command]
pub async fn redis_pubsub_publish(
    state: State<'_, AppState>,
    payload: redis::pubsub::PublishPayload,
) -> Result<redis::pubsub::PublishResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::pubsub_publish(&connection, &payload).await
    })
    .await
}

// ---------------------------------------------------------------------------
// Key operations (writes)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn redis_set_string(
    state: State<'_, AppState>,
    payload: redis::key_ops::SetStringPayload,
) -> Result<redis::key_ops::SetStringResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::set_string(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_set_hash_fields(
    state: State<'_, AppState>,
    payload: redis::key_ops::SetHashFieldsPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::set_hash_fields(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_delete_hash_fields(
    state: State<'_, AppState>,
    payload: redis::key_ops::DeleteHashFieldsPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::delete_hash_fields(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_del_keys(
    state: State<'_, AppState>,
    payload: redis::key_ops::DelKeysPayload,
) -> Result<redis::key_ops::DelKeysResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::del_keys(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_bulk_delete_by_pattern(
    state: State<'_, AppState>,
    payload: redis::key_ops::BulkDeleteByPatternPayload,
) -> Result<redis::key_ops::BulkDeleteByPatternResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::bulk_delete_by_pattern(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_bulk_expire_by_pattern(
    state: State<'_, AppState>,
    payload: redis::key_ops::BulkExpireByPatternPayload,
) -> Result<redis::key_ops::BulkExpireByPatternResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::bulk_expire_by_pattern(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_bulk_rename_by_prefix(
    state: State<'_, AppState>,
    payload: redis::key_ops::BulkRenameByPrefixPayload,
) -> Result<redis::key_ops::BulkRenameByPrefixResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::bulk_rename_by_prefix(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_set_bit(
    state: State<'_, AppState>,
    payload: redis::key_ops::SetBitPayload,
) -> Result<redis::key_ops::SetBitResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::set_bit(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_set_expire(
    state: State<'_, AppState>,
    payload: redis::key_ops::SetExpirePayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::set_expire(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_rename_key(
    state: State<'_, AppState>,
    payload: redis::key_ops::RenameKeyPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::rename_key(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_create_key(
    state: State<'_, AppState>,
    payload: redis::key_ops::CreateKeyPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::create_key(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_apply_list_edits(
    state: State<'_, AppState>,
    payload: redis::key_ops::ApplyListEditsPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::apply_list_edits(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_apply_set_edits(
    state: State<'_, AppState>,
    payload: redis::key_ops::SetMembersPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::apply_set_edits(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_apply_sorted_set_edits(
    state: State<'_, AppState>,
    payload: redis::key_ops::SortedSetEditsPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::apply_sorted_set_edits(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_apply_stream_edits(
    state: State<'_, AppState>,
    payload: redis::key_ops::StreamEditsPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::apply_stream_edits(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_create_stream_group(
    state: State<'_, AppState>,
    payload: redis::key_ops::CreateStreamGroupPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::create_stream_group(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_destroy_stream_group(
    state: State<'_, AppState>,
    payload: redis::key_ops::DestroyStreamGroupPayload,
) -> Result<u64, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::destroy_stream_group(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_set_json_path(
    state: State<'_, AppState>,
    payload: redis::key_ops::JsonSetPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::set_json_path(&connection, &payload).await
    })
    .await
}

#[tauri::command]
pub async fn redis_delete_json_path(
    state: State<'_, AppState>,
    payload: redis::key_ops::JsonDeletePayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::delete_json_path(&connection, &payload).await
    })
    .await
}
