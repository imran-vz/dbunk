//! The Tauri desktop host: window chrome, logging, lifecycle wiring and the
//! application entry point. This and `commands` are the only modules that
//! name Tauri.

use tauri::Manager;

use crate::storage::Paths;
use crate::{
    close_socket_managers_for_exit, commands, dispatch, host, postgres, storage, AppState,
    TOKIO_POSTGRES_LOG_LEVEL,
};

// Keep in sync with `app.windows[0].trafficLightPosition` in tauri.conf.json.
#[cfg(target_os = "macos")]
const MACOS_TRAFFIC_LIGHT_X: f64 = 18.0;
#[cfg(target_os = "macos")]
const MACOS_TRAFFIC_LIGHT_Y: f64 = 26.0;

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
        .level_for("tokio_postgres", TOKIO_POSTGRES_LOG_LEVEL)
        .build()
}

#[derive(Default)]
struct ExitCleanupState {
    started: std::sync::atomic::AtomicBool,
    finished: std::sync::Mutex<bool>,
    completion: std::sync::Condvar,
}

#[derive(Debug, PartialEq, Eq)]
enum ExitRequestAction {
    BeginCleanup,
    PreventExit,
    WaitForCleanup,
    AllowExit,
}

impl ExitCleanupState {
    fn request(&self, restarting: bool) -> ExitRequestAction {
        if !self.started.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return ExitRequestAction::BeginCleanup;
        }
        if *self.finished.lock().unwrap() {
            return ExitRequestAction::AllowExit;
        }
        if restarting {
            ExitRequestAction::WaitForCleanup
        } else {
            ExitRequestAction::PreventExit
        }
    }

    fn finish(&self) {
        *self.finished.lock().unwrap() = true;
        self.completion.notify_all();
    }

    fn wait(&self) {
        let mut finished = self.finished.lock().unwrap();
        while !*finished {
            finished = self.completion.wait(finished).unwrap();
        }
    }
}

/// Runs exit cleanup on its own task so a panic inside any manager's close
/// path cannot skip `finish`; otherwise a restart request arriving during an
/// ordinary exit would wait on the cleanup condition variable forever.
async fn run_exit_cleanup(
    close: impl std::future::Future<Output = ()> + Send + 'static,
    exit_cleanup: &ExitCleanupState,
) {
    if let Err(error) = tauri::async_runtime::spawn(close).await {
        log::error!("exit cleanup task failed: {error}");
    }
    exit_cleanup.finish();
}

/// Where this host keeps the local database when no isolation override
/// applies. Existing profiles live here, so the resolution must not change.
fn default_config_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::path::BaseDirectory;
    #[cfg(target_os = "windows")]
    let (path, base) = ("dbunk", BaseDirectory::AppData);
    #[cfg(not(target_os = "windows"))]
    let (path, base) = (".config/dbunk", BaseDirectory::Home);
    app.path()
        .resolve(path, base)
        .map_err(|error| error.to_string())
}

// ---------------------------------------------------------------------------
// Application entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // SQLx 0.8 has no public way to clear inherited PostgreSQL certificate
    // paths or PGOPTIONS after constructing `PgConnectOptions`. Do this before
    // Tauri starts worker threads so StoredConnection remains authoritative.
    postgres::tls::prepare_sqlx_environment();
    dispatch::ensure_sqlx_drivers();
    let app = tauri::Builder::default()
        // Configured windows can commit their first document before setup runs.
        .manage(postgres::schema_compare::manager::CompareManager::new())
        .plugin(build_log_plugin())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // Persist and restore window geometry across launches (D7).
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            log::info!("dbunk starting up");
            let paths = Paths::resolve(|| default_config_dir(app.handle()))
                .map_err(|error| format!("Failed to resolve config dir: {error}"))?;
            log::info!("config dir: {}", paths.config_dir().display());
            let pool = tauri::async_runtime::block_on(storage::open_pool(&paths))
                .map_err(|error| format!("Failed to open local database: {error}"))?;
            log::info!("SQLite pool ready, migrations applied");
            // A configured window can commit its first document before setup
            // runs, so the comparison manager is managed before the builder
            // reaches here and shared into the state.
            let pg_schema_compare = app
                .state::<postgres::schema_compare::manager::CompareManager>()
                .inner()
                .clone();
            let state = AppState::new(pool, paths, pg_schema_compare);
            // Setup runs on the main thread with no ambient Tokio context, so
            // every monitor is spawned on Tauri's runtime through its handle.
            state.start_monitors(tauri::async_runtime::handle().inner());
            app.manage(state);
            Ok(())
        })
        .on_page_load(|webview, payload| {
            let event = match payload.event() {
                tauri::webview::PageLoadEvent::Started => host::DocumentLoad::Started,
                tauri::webview::PageLoadEvent::Finished => host::DocumentLoad::Finished,
            };
            webview
                .state::<postgres::schema_compare::manager::CompareManager>()
                .transport_document_load(webview.label(), event);
        })
        .on_window_event(|window, event| {
            let manager = window.state::<AppState>().query_sessions.clone();
            let label = window.label().to_string();
            match event {
                tauri::WindowEvent::Focused(focused) => {
                    let focused = *focused;
                    tauri::async_runtime::spawn(async move {
                        manager.set_focused(&label, focused).await;
                    });
                }
                tauri::WindowEvent::Destroyed => {
                    window
                        .state::<AppState>()
                        .pg_schema_compare
                        .transport_destroyed(&label);
                    tauri::async_runtime::spawn(async move {
                        manager.close_window(&label).await;
                    });
                }
                _ => {}
            }
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
            commands::settings::load_ui_state,
            commands::settings::save_ui_state,
            commands::settings::delete_ui_state,
            // Bastion servers
            commands::bastions::load_bastion_servers,
            commands::bastions::save_bastion_server,
            commands::bastions::delete_bastion_server,
            commands::bastions::reset_bastion_host_key,
            commands::bastions::test_bastion_server,
            // Managed servers
            commands::managed::check_docker,
            commands::managed::provision_managed_server,
            commands::managed::list_managed_servers,
            commands::managed::start_managed_server,
            commands::managed::stop_managed_server,
            commands::managed::destroy_managed_server,
            commands::managed::recreate_managed_server,
            // Connections
            commands::connections::load_connections,
            commands::connections::save_connection,
            commands::connections::duplicate_connection,
            commands::connections::update_connection_organization,
            commands::connections::delete_connection,
            commands::connections::connect_connection,
            commands::connections::disconnect_connection,
            commands::diagnosis::diagnose_connection,
            commands::connections::health_check_connection,
            // Relational: schema
            commands::relational::load_schema_explorer,
            commands::pg_objects::load_pg_object_catalog,
            commands::pg_objects::describe_pg_object,
            commands::pg_objects::load_pg_drop_impact,
            commands::pg_objects::preview_object_ddl,
            commands::pg_objects::apply_object_ddl,
            commands::relational::load_schema_relationships,
            commands::relational::load_table_schema_relationships,
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
            commands::query_session::register_query_session_owner,
            commands::query_session::open_query_session,
            commands::query_session::execute_query_session,
            commands::query_session::describe_query_parameters,
            commands::query_session::ack_query_session_events,
            commands::query_session::heartbeat_query_sessions,
            commands::query_session::cancel_query_execution,
            commands::query_session::refresh_query_transaction_state,
            commands::query_session::set_query_transaction_mode,
            commands::query_session::set_query_transaction_isolation,
            commands::query_session::commit_query_transaction,
            commands::query_session::rollback_query_transaction,
            commands::query_session::close_query_session,
            commands::relational::load_table_data,
            commands::table_browse::browse_table_data,
            commands::table_browse::cancel_table_browse,
            commands::table_browse::count_table_browse_rows,
            commands::table_browse::close_table_browse_for_tab,
            commands::table_browse::load_table_grid_prefs,
            commands::table_browse::save_table_grid_prefs,
            // PostgreSQL result mutation (dark until Plan 006)
            commands::result_mutation::analyze_result_set,
            commands::result_mutation::preview_result_mutations,
            commands::result_mutation::apply_result_mutations,
            commands::result_mutation::cancel_result_mutation,
            commands::result_mutation::close_result_mutation_for_connection,
            commands::result_mutation::load_virtual_key,
            commands::result_mutation::save_virtual_key,
            commands::result_mutation::clear_virtual_key,
            commands::safety::load_safety_overrides,
            commands::relational::load_table_structure,
            commands::relational::execute_ddl,
            commands::relational::export_ddl,
            commands::pg_backup::start_pg_backup,
            commands::pg_schema_compare::start_pg_schema_compare,
            commands::pg_schema_compare::list_pg_schema_compares,
            commands::pg_schema_compare::get_pg_schema_compare,
            commands::pg_schema_compare::cancel_pg_schema_compare,
            commands::pg_schema_compare::release_pg_schema_compare,
            commands::pg_schema_compare::read_pg_schema_compare,
            commands::pg_schema_compare::get_pg_schema_compare_transport,
            commands::pg_schema_compare::acknowledge_pg_schema_compare,
            commands::pg_transfer::inspect_pg_transfer,
            commands::pg_transfer::release_pg_transfer_inspection,
            commands::pg_transfer::start_pg_csv_import,
            commands::pg_transfer::start_pg_csv_export,
            commands::pg_transfer::get_pg_transfer_job,
            commands::pg_transfer::list_pg_transfer_jobs,
            commands::pg_transfer::cancel_pg_transfer_job,
            commands::pg_transfer::release_pg_transfer_job,
            commands::pg_transfer::pick_pg_transfer_file,
            commands::pg_backup::pick_pg_tool_file,
            commands::pg_backup::start_pg_restore,
            commands::pg_backup::get_pg_tool_job,
            commands::pg_backup::list_pg_tool_jobs,
            commands::pg_backup::cancel_pg_tool_job,
            commands::pg_backup::release_pg_tool_job,
            commands::relational::refresh_materialized_view,
            commands::relational::run_pg_maintenance,
            // Relational: mutations
            commands::relational::commit_cell_edits,
            commands::relational::insert_row,
            commands::relational::import_rows,
            commands::relational::seed_table,
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
            commands::xlsx::parse_xlsx,
            commands::xlsx::export_xlsx,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    let exit_cleanup = std::sync::Arc::new(ExitCleanupState::default());
    app.run(move |handle, event| {
        if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
            let restarting = code == Some(tauri::RESTART_EXIT_CODE);
            match exit_cleanup.request(restarting) {
                ExitRequestAction::BeginCleanup => {
                    let handle = handle.clone();
                    let manager = handle.state::<AppState>().query_sessions.clone();
                    let table_browse = handle.state::<AppState>().table_browse.clone();
                    let result_mutations = handle.state::<AppState>().result_mutations.clone();
                    let pg_tool_jobs = handle.state::<AppState>().pg_tool_jobs.clone();
                    let pg_transfers = handle.state::<AppState>().pg_transfers.clone();
                    let pg_schema_compare = handle.state::<AppState>().pg_schema_compare.clone();
                    let close = close_socket_managers_for_exit(
                        manager,
                        table_browse,
                        result_mutations,
                        pg_tool_jobs,
                        pg_transfers,
                        pg_schema_compare,
                    );
                    if restarting {
                        // Tauri deliberately ignores prevent_exit for restart. Keep
                        // this callback open until cleanup finishes, then let the
                        // original request preserve restart_on_exit semantics.
                        tauri::async_runtime::block_on(run_exit_cleanup(close, &exit_cleanup));
                    } else {
                        api.prevent_exit();
                        let exit_cleanup = exit_cleanup.clone();
                        tauri::async_runtime::spawn(async move {
                            run_exit_cleanup(close, &exit_cleanup).await;
                            handle.exit(code.unwrap_or(0));
                        });
                    }
                }
                ExitRequestAction::PreventExit => api.prevent_exit(),
                ExitRequestAction::WaitForCleanup => {
                    // A restart can race an ordinary exit whose async cleanup still
                    // owns native jobs. Tauri cannot prevent this request, so hold
                    // it until that existing cleanup has completed.
                    exit_cleanup.wait();
                }
                ExitRequestAction::AllowExit => {}
            }
        }
    });
}

#[cfg(test)]
mod exit_lifecycle_tests {
    use super::*;

    #[test]
    fn repeated_restart_waits_for_cleanup_already_in_progress() {
        let state = std::sync::Arc::new(ExitCleanupState::default());
        assert_eq!(state.request(false), ExitRequestAction::BeginCleanup);
        assert_eq!(state.request(true), ExitRequestAction::WaitForCleanup);
        let (entered, ready) = std::sync::mpsc::channel();
        let waiter = {
            let state = state.clone();
            std::thread::spawn(move || {
                entered.send(()).unwrap();
                state.wait();
            })
        };
        ready.recv().unwrap();
        assert!(!waiter.is_finished());
        state.finish();
        waiter.join().unwrap();
    }

    #[tokio::test]
    async fn cleanup_panic_still_finishes_so_a_restart_cannot_wait_forever() {
        let state = std::sync::Arc::new(ExitCleanupState::default());
        assert_eq!(state.request(false), ExitRequestAction::BeginCleanup);
        assert_eq!(state.request(true), ExitRequestAction::WaitForCleanup);

        run_exit_cleanup(async { panic!("manager close path panicked") }, &state).await;

        assert_eq!(state.request(true), ExitRequestAction::AllowExit);
        let waiter = {
            let state = state.clone();
            std::thread::spawn(move || state.wait())
        };
        waiter.join().unwrap();
    }

    #[test]
    fn repeated_ordinary_exit_is_prevented_until_cleanup_finishes() {
        let state = ExitCleanupState::default();
        assert_eq!(state.request(false), ExitRequestAction::BeginCleanup);
        assert_eq!(state.request(false), ExitRequestAction::PreventExit);

        state.finish();

        assert_eq!(state.request(false), ExitRequestAction::AllowExit);
    }
}
