mod clickhouse;
mod commands;
mod credentials;
mod dispatch;
mod keychain;
mod postgres;
mod redis;
mod storage;
mod tunnel;
mod types;
mod xlsx;

// Re-export DTOs at the crate root so existing `crate::Foo` paths in child
// modules and the `#[tauri::command]` macros keep working unchanged.
pub(crate) use types::*;

use sqlx::SqlitePool;
use tauri::Manager;

use crate::storage::Paths;

pub(crate) const MAX_QUERY_HISTORY: usize = storage::QUERY_HISTORY_CAP as usize;

pub(crate) const DEFAULT_TABLE_PAGE_SIZE: u32 = 100;
pub(crate) const MAX_TABLE_PAGE_SIZE: u32 = 1000;
// Keep in sync with `app.windows[0].trafficLightPosition` in tauri.conf.json.
const MACOS_TRAFFIC_LIGHT_X: f64 = 18.0;
const MACOS_TRAFFIC_LIGHT_Y: f64 = 26.0;

pub(crate) struct AppState {
    pool: SqlitePool,
    paths: Paths,
}

// ---------------------------------------------------------------------------
// Utility functions used by engine modules
// ---------------------------------------------------------------------------

pub(crate) fn quote_double(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub(crate) fn quote_backtick(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

pub(crate) fn qualified_table_name(engine: &DatabaseEngine, schema: &str, table: &str) -> String {
    match engine {
        DatabaseEngine::PostgreSQL | DatabaseEngine::SQLite => {
            if schema.is_empty() {
                quote_double(table)
            } else {
                format!("{}.{}", quote_double(schema), quote_double(table))
            }
        }
        DatabaseEngine::MySQL | DatabaseEngine::ClickHouse => {
            if schema.is_empty() {
                quote_backtick(table)
            } else {
                format!("{}.{}", quote_backtick(schema), quote_backtick(table))
            }
        }
        DatabaseEngine::Redis => {
            unreachable!("BUG: qualified_table_name called on Redis connection")
        }
    }
}

pub(crate) fn parse_total_rows(result: &QueryResult) -> Option<u64> {
    result
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(|cell| cell.parse::<u64>().ok())
}

/// Bytes → `0x` hex string. Used by every engine's value coercion path
/// for binary column display (Postgres `bytea`, sqlx-Any `Vec<u8>`),
/// kept at the crate root so each engine module can reach it without
/// re-implementing.
pub(crate) fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2 + 2);
    output.push_str("0x");
    for byte in bytes {
        output.push_str(&format!("{:02x}", byte));
    }
    output
}

// ---------------------------------------------------------------------------
// macOS traffic-light positioning
// ---------------------------------------------------------------------------

#[tauri::command]
fn restore_window_traffic_light_position(window: tauri::Window) -> Result<(), String> {
    apply_window_traffic_light_position(&window)
}

#[cfg(target_os = "macos")]
fn apply_window_traffic_light_position(window: &tauri::Window) -> Result<(), String> {
    use objc2_app_kit::{NSView, NSWindow, NSWindowButton};

    let ns_window = window.ns_window().map_err(|error| error.to_string())?;
    let ns_window: &NSWindow = unsafe { &*ns_window.cast::<NSWindow>() };

    unsafe {
        let Some(close) = ns_window.standardWindowButton(NSWindowButton::CloseButton) else {
            return Ok(());
        };
        let Some(miniaturize) = ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton)
        else {
            return Ok(());
        };
        let zoom = ns_window.standardWindowButton(NSWindowButton::ZoomButton);
        let Some(title_bar_container_view) = close.superview().and_then(|view| view.superview())
        else {
            return Ok(());
        };

        let close_rect = NSView::frame(&close);
        let title_bar_frame_height = close_rect.size.height + MACOS_TRAFFIC_LIGHT_Y;
        let mut title_bar_rect = NSView::frame(&title_bar_container_view);
        title_bar_rect.size.height = title_bar_frame_height;
        title_bar_rect.origin.y = ns_window.frame().size.height - title_bar_frame_height;
        title_bar_container_view.setFrame(title_bar_rect);

        let space_between = NSView::frame(&miniaturize).origin.x - close_rect.origin.x;
        let mut window_buttons = vec![close, miniaturize];
        if let Some(zoom) = zoom {
            window_buttons.push(zoom);
        }

        for (index, button) in window_buttons.into_iter().enumerate() {
            let mut rect = NSView::frame(&button);
            rect.origin.x = MACOS_TRAFFIC_LIGHT_X + (index as f64 * space_between);
            button.setFrameOrigin(rect.origin);
        }
    }

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn apply_window_traffic_light_position(_window: &tauri::Window) -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

/// Builds the application-wide logger via `tauri-plugin-log`. Dev
/// targets are stdout (visible in the terminal where `pnpm tauri dev`
/// runs) and the webview console (visible in browser DevTools so
/// frontend developers see backend logs too). Release builds also
/// rotate to a file under the per-OS app log directory.
fn build_log_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_log::{Target, TargetKind};

    let crate_level = if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    let mut targets = vec![
        Target::new(TargetKind::Stdout),
        Target::new(TargetKind::Webview),
    ];
    if !cfg!(debug_assertions) {
        targets.push(Target::new(TargetKind::LogDir { file_name: None }));
    }

    tauri_plugin_log::Builder::default()
        .targets(targets)
        .level(log::LevelFilter::Warn)
        .level_for("dbunk_lib", crate_level)
        .build()
}

// ---------------------------------------------------------------------------
// Application entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    dispatch::ensure_sqlx_drivers();
    tauri::Builder::default()
        .plugin(build_log_plugin())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            log::info!("dbunk starting up");
            let paths = Paths::from_app(app.handle())
                .map_err(|error| format!("Failed to resolve config dir: {error}"))?;
            log::info!("config dir: {}", paths.config_dir().display());
            let pool = tauri::async_runtime::block_on(storage::open_pool(&paths))
                .map_err(|error| format!("Failed to open local database: {error}"))?;
            log::info!("SQLite pool ready, migrations applied");
            app.manage(AppState { pool, paths });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Window chrome
            restore_window_traffic_light_position,
            // Settings + credentials
            commands::settings::load_app_settings,
            commands::settings::save_app_settings,
            commands::settings::configure_credential_storage,
            commands::settings::unlock_credentials,
            commands::settings::change_credential_storage,
            commands::settings::reset_credential_storage,
            // Bastion servers
            commands::bastions::load_bastion_servers,
            commands::bastions::save_bastion_server,
            commands::bastions::delete_bastion_server,
            commands::bastions::reset_bastion_host_key,
            commands::bastions::test_bastion_server,
            // Connections
            commands::connections::load_connections,
            commands::connections::save_connection,
            commands::connections::delete_connection,
            commands::connections::connect_connection,
            commands::connections::disconnect_connection,
            commands::connections::test_connection,
            commands::connections::health_check_connection,
            // Relational: schema
            commands::relational::load_schema_explorer,
            commands::relational::load_schema_relationships,
            commands::relational::load_schema_map_positions,
            commands::relational::save_schema_map_position,
            commands::relational::reset_schema_map_positions,
            commands::relational::load_schema_map_prefs,
            commands::relational::save_schema_map_prefs,
            // Relational: overview + admin
            commands::relational::load_database_overview_stats,
            commands::relational::load_relation_stats,
            commands::relational::load_server_details,
            commands::relational::load_pg_admin_snapshot,
            commands::relational::cancel_pg_backend,
            commands::relational::terminate_pg_backend,
            // Relational: queries + data
            commands::relational::run_query,
            commands::relational::load_table_data,
            commands::relational::load_table_structure,
            commands::relational::execute_ddl,
            commands::relational::export_ddl,
            commands::relational::run_pg_dump,
            commands::relational::run_pg_restore,
            commands::relational::refresh_materialized_view,
            commands::relational::run_pg_maintenance,
            // Relational: mutations
            commands::relational::commit_cell_edits,
            commands::relational::insert_row,
            commands::relational::import_rows,
            commands::relational::copy_table_rows,
            commands::relational::delete_rows,
            commands::relational::poll_mutation_status,
            // Relational: history + saved
            commands::relational::load_query_history,
            commands::relational::append_query_history,
            commands::relational::clear_query_history,
            commands::relational::load_saved_queries,
            commands::relational::save_saved_query,
            commands::relational::delete_saved_query,
            // Redis: history + saved commands
            commands::keyvalue::load_redis_cli_history,
            commands::keyvalue::append_redis_cli_history,
            commands::keyvalue::load_saved_redis_commands,
            commands::keyvalue::save_saved_redis_command,
            commands::keyvalue::delete_saved_redis_command,
            // Redis: keyspace
            commands::keyvalue::redis_scan_keys,
            commands::keyvalue::redis_open_scan_session,
            commands::keyvalue::redis_cancel_scan_session,
            commands::keyvalue::redis_close_scan_session,
            // Redis: key inspection
            commands::keyvalue::redis_fetch_key_metadata,
            commands::keyvalue::redis_fetch_string,
            commands::keyvalue::redis_fetch_hash,
            commands::keyvalue::redis_fetch_list,
            commands::keyvalue::redis_fetch_set,
            commands::keyvalue::redis_fetch_sorted_set,
            commands::keyvalue::redis_fetch_stream,
            commands::keyvalue::redis_fetch_stream_groups,
            commands::keyvalue::redis_fetch_json,
            // Redis: CLI
            commands::keyvalue::redis_run_command,
            commands::keyvalue::redis_cli_close_session,
            // Redis: server info
            commands::keyvalue::redis_fetch_overview,
            commands::keyvalue::redis_fetch_client_list,
            commands::keyvalue::redis_fetch_acl_list,
            commands::keyvalue::redis_fetch_acl_self,
            commands::keyvalue::redis_fetch_config,
            commands::keyvalue::redis_set_config,
            commands::keyvalue::redis_fetch_latency,
            // Redis: pub/sub
            commands::keyvalue::redis_pubsub_start,
            commands::keyvalue::redis_pubsub_discover,
            commands::keyvalue::redis_pubsub_drain,
            commands::keyvalue::redis_pubsub_close,
            commands::keyvalue::redis_pubsub_publish,
            // Redis: key ops (writes)
            commands::keyvalue::redis_set_string,
            commands::keyvalue::redis_set_hash_fields,
            commands::keyvalue::redis_delete_hash_fields,
            commands::keyvalue::redis_del_keys,
            commands::keyvalue::redis_bulk_delete_by_pattern,
            commands::keyvalue::redis_bulk_expire_by_pattern,
            commands::keyvalue::redis_bulk_rename_by_prefix,
            commands::keyvalue::redis_set_bit,
            commands::keyvalue::redis_set_expire,
            commands::keyvalue::redis_rename_key,
            commands::keyvalue::redis_create_key,
            commands::keyvalue::redis_apply_list_edits,
            commands::keyvalue::redis_apply_set_edits,
            commands::keyvalue::redis_apply_sorted_set_edits,
            commands::keyvalue::redis_apply_stream_edits,
            commands::keyvalue::redis_create_stream_group,
            commands::keyvalue::redis_destroy_stream_group,
            commands::keyvalue::redis_set_json_path,
            commands::keyvalue::redis_delete_json_path,
            // XLSX import/export
            xlsx::parse_xlsx,
            xlsx::export_xlsx,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
