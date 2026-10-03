//! Isolated macOS host for the stage 03 PostgreSQL fixture.
mod accessible_editor;
mod admin_model;
mod admin_view;
mod apply_flow;
mod bounded_field;
mod browse_controls;
mod browse_preferences;
mod catalog;
mod catalog_view;
mod cell_value;
mod connection_uri;
mod console_model;
mod controller;
mod csv_transfer_model;
mod csv_transfer_store;
mod csv_transfer_view;
mod explain_view;
mod pg_tool_jobs;
mod pg_tool_store;
mod pg_tool_view;
mod safety_audit_model;
mod schema_changes;
mod schema_compare_model;
mod schema_compare_reader;
mod schema_compare_store;
mod schema_compare_view;
mod schema_map_model;
mod schema_map_png;
mod schema_map_view;
mod schema_view;
mod sequence_runtime;
mod sequence_view;
// Mutation preparation is exercised before the review UI is activated.
mod column_widths;
mod connection_settings_model;
mod connection_settings_view;
#[allow(dead_code)]
mod data_model;
mod ddl_export_model;
mod ddl_export_view;
mod diagnostics;
mod dock_view;
mod document_view;
mod export_view;
mod file_log;
mod file_runtime;
mod fk_navigation;
mod forms;
mod geometry_preview;
mod grid;
mod grid_columns;
mod launch;
#[cfg(test)]
mod live_tests;
mod mailbox;
mod maintenance_view;
mod navigator_model;
mod navigator_view;
mod object_ddl_model;
mod object_ddl_view;
mod object_details;
mod open_anything;
mod overview_model;
mod overview_view;
mod palette_view;
mod persistence;
mod query_library;
mod query_library_view;
mod query_result;
mod result_export;
mod results;
mod server_details_model;
mod sql;
mod sql_completion;
mod sql_find;
mod sql_format;
mod stream;
mod style;
mod table_changes;
mod table_copy_store;
mod table_copy_view;
mod table_ddl_model;
mod table_ddl_view;
mod table_seed_model;
mod table_seed_runtime;
mod table_seed_store;
mod table_seed_view;
mod table_structure_model;
mod table_structure_view;
mod table_view;
mod value_inspector;
#[cfg(feature = "fixture-verification")]
mod verification;
mod whole_table_export_model;
mod whole_table_export_view;
mod window_geometry;
mod workbench;
mod workspace;

use gpui::{App, Bounds, KeyBinding, WindowBounds, WindowOptions, prelude::*, px, size};
use settings::{DEFAULT_KEYMAP_PATH, KeymapFile};
use std::process::ExitCode;
use workbench::*;

fn init_editor(cx: &mut App) -> anyhow::Result<()> {
    settings::init(cx);
    theme_settings::init(theme::LoadThemes::JustBase, cx);
    release_channel::init(semver::Version::new(0, 0, 0), cx);
    assets::Assets.load_fonts(cx)?;
    editor::init(cx);
    let theme_settings = cx.update_global::<settings::SettingsStore, _>(|store, cx| {
        store.set_user_settings(r##"{"languages":{"SQL":{"completions":{"words":"disabled"}}},"theme":"One Dark","buffer_font_size":11.5,"ui_font_size":11,"buffer_line_height":"standard","experimental.theme_overrides":{"background":"#0c0d0f","editor.background":"#0c0d0f","editor.foreground":"#cdd2d9","editor.gutter.background":"#0c0d0f","editor.active_line.background":"#15181c","editor.line_number":"#5b626c","editor.active_line_number":"#8a929c","error.background":"#0c0d0f","error.border":"#f85149","text":"#cdd2d9","text.muted":"#8a929c","border":"#24282e"}}"##, cx)
    });
    theme_settings.result()?;
    theme_settings::reload_theme(cx);
    cx.bind_keys(KeymapFile::load_asset_allow_partial_failure(
        DEFAULT_KEYMAP_PATH,
        cx,
    )?);
    cx.bind_keys([
        KeyBinding::new(
            "tab",
            connection_settings_view::NextControl,
            Some("ConnectionSettings > Editor"),
        ),
        KeyBinding::new(
            "shift-tab",
            connection_settings_view::PreviousControl,
            Some("ConnectionSettings > Editor"),
        ),
        // Override Zed's editor newline/code-action bindings in their own context.
        KeyBinding::new("cmd-enter", RunStatement, Some("Editor")),
        KeyBinding::new("cmd-shift-enter", RunScript, Some("Editor")),
        KeyBinding::new("cmd-.", StopQuery, Some("Editor")),
        KeyBinding::new("cmd-enter", RunStatement, Some("Workbench")),
        KeyBinding::new("cmd-shift-enter", RunScript, Some("Workbench")),
        KeyBinding::new("cmd-shift-f", FormatSql, Some("Workbench && Editor")),
        KeyBinding::new("cmd-f", FindInSql, Some("Workbench && Editor")),
        KeyBinding::new("cmd-g", FindNext, Some("Workbench && Editor")),
        KeyBinding::new("cmd-shift-g", FindPrevious, Some("Workbench && Editor")),
        KeyBinding::new("cmd-.", StopQuery, Some("Workbench")),
        KeyBinding::new("cmd-q", Quit, Some("Workbench")),
        KeyBinding::new("f6", SwitchPane, Some("Workbench")),
        KeyBinding::new("shift-f6", SwitchPane, Some("Workbench")),
        KeyBinding::new(
            "tab",
            SwitchPane,
            Some("ResultGrid && !Editor && !ValueInspector"),
        ),
        KeyBinding::new(
            "shift-tab",
            SwitchPane,
            Some("ResultGrid && !Editor && !ValueInspector"),
        ),
        KeyBinding::new("f8", FocusToolbar, Some("Editor")),
        KeyBinding::new("f8", FocusToolbar, Some("Workbench")),
        KeyBinding::new("escape", LeaveToolbar, Some("NativeToolbar")),
        KeyBinding::new("tab", NextControl, Some("NativeToolbar")),
        KeyBinding::new("shift-tab", PreviousControl, Some("NativeToolbar")),
        KeyBinding::new("cmd-c", grid::CopyCells, Some("ResultGrid")),
        KeyBinding::new(
            "cmd-g",
            grid::GoToRow,
            Some("ResultGrid && !ValueInspector"),
        ),
        KeyBinding::new(
            "shift-space",
            grid::SelectCurrentRow,
            Some("ResultGrid && !ValueInspector"),
        ),
        KeyBinding::new("cmd-shift-e", grid::ExportCells, Some("ResultGrid")),
        KeyBinding::new(
            "cmd-a",
            grid::SelectAllCells,
            Some("ResultGrid && !ValueInspector"),
        ),
        KeyBinding::new(
            "space",
            grid::InspectCell,
            Some("ResultGrid && !ValueInspector"),
        ),
        KeyBinding::new(
            "shift-enter",
            grid::InspectCell,
            Some("ResultGrid && !ValueInspector"),
        ),
        KeyBinding::new("up", grid::MoveUp, Some("ResultGrid && !ValueInspector")),
        KeyBinding::new(
            "down",
            grid::MoveDown,
            Some("ResultGrid && !ValueInspector"),
        ),
        KeyBinding::new(
            "left",
            grid::MoveLeft,
            Some("ResultGrid && !ValueInspector"),
        ),
        KeyBinding::new(
            "right",
            grid::MoveRight,
            Some("ResultGrid && !ValueInspector"),
        ),
        KeyBinding::new(
            "shift-up",
            grid::SelectUp,
            Some("ResultGrid && !ValueInspector"),
        ),
        KeyBinding::new(
            "shift-down",
            grid::SelectDown,
            Some("ResultGrid && !ValueInspector"),
        ),
        KeyBinding::new(
            "shift-left",
            grid::SelectLeft,
            Some("ResultGrid && !ValueInspector"),
        ),
        KeyBinding::new(
            "shift-right",
            grid::SelectRight,
            Some("ResultGrid && !ValueInspector"),
        ),
    ]);
    #[cfg(feature = "fixture-verification")]
    if verification::enabled() {
        cx.bind_keys([
            KeyBinding::new(
                "ctrl-alt-cmd-v",
                verification::ResumeDrain,
                Some("Workbench"),
            ),
            KeyBinding::new(
                "ctrl-alt-cmd-r",
                verification::ReplaceView,
                Some("Workbench"),
            ),
            KeyBinding::new("ctrl-alt-cmd-o", verification::Reconnect, Some("Workbench")),
        ]);
    }
    Ok(())
}

fn init_workspace_commands(cx: &mut App) {
    use gpui::{Menu, MenuItem};
    use workspace::*;
    cx.bind_keys([
        KeyBinding::new(
            "tab",
            whole_table_export_view::NextControl,
            Some("WholeTableExport > Editor"),
        ),
        KeyBinding::new(
            "shift-tab",
            whole_table_export_view::PreviousControl,
            Some("WholeTableExport > Editor"),
        ),
        KeyBinding::new(
            "tab",
            schema_map_view::NextControl,
            Some("SchemaMap > Editor"),
        ),
        KeyBinding::new(
            "shift-tab",
            schema_map_view::PreviousControl,
            Some("SchemaMap > Editor"),
        ),
        KeyBinding::new(
            "tab",
            table_ddl_view::NextControl,
            Some("TableDdl > Editor"),
        ),
        KeyBinding::new(
            "tab",
            object_ddl_view::NextControl,
            Some("ObjectDdl > Editor"),
        ),
        KeyBinding::new(
            "shift-tab",
            object_ddl_view::PreviousControl,
            Some("ObjectDdl > Editor"),
        ),
        KeyBinding::new(
            "tab",
            table_structure_view::NextControl,
            Some("TableStructure > Editor"),
        ),
        KeyBinding::new("tab", overview_view::NextControl, Some("Overview > Editor")),
        KeyBinding::new(
            "tab",
            ddl_export_view::NextControl,
            Some("DdlExport > Editor"),
        ),
        KeyBinding::new(
            "shift-tab",
            ddl_export_view::PreviousControl,
            Some("DdlExport > Editor"),
        ),
        KeyBinding::new(
            "shift-tab",
            overview_view::PreviousControl,
            Some("Overview > Editor"),
        ),
        KeyBinding::new(
            "shift-tab",
            table_ddl_view::PreviousControl,
            Some("TableDdl > Editor"),
        ),
        KeyBinding::new(
            "shift-tab",
            table_structure_view::PreviousControl,
            Some("TableStructure > Editor"),
        ),
        KeyBinding::new(
            "tab",
            table_seed_view::NextControl,
            Some("TableSeed > Editor"),
        ),
        KeyBinding::new(
            "shift-tab",
            table_seed_view::PreviousControl,
            Some("TableSeed > Editor"),
        ),
        KeyBinding::new("cmd-t", NewTab, Some("NativeWorkspace")),
        KeyBinding::new("cmd-w", CloseTab, Some("NativeWorkspace")),
        KeyBinding::new("ctrl-tab", NextTab, Some("NativeWorkspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("NativeWorkspace")),
        KeyBinding::new("cmd-shift-r", RenameTab, Some("NativeWorkspace")),
        KeyBinding::new("cmd-shift-p", PinTab, Some("NativeWorkspace")),
        KeyBinding::new("cmd-alt-left", MoveTabLeft, Some("NativeWorkspace")),
        KeyBinding::new("cmd-alt-right", MoveTabRight, Some("NativeWorkspace")),
        KeyBinding::new("cmd-n", NewConnection, Some("NativeWorkspace")),
        KeyBinding::new("cmd-shift-o", FocusNavigator, Some("NativeWorkspace")),
        KeyBinding::new("cmd-k", OpenAnything, Some("NativeWorkspace")),
        KeyBinding::new("ctrl-`", ToggleConsole, Some("NativeWorkspace")),
        KeyBinding::new("cmd-\\", ToggleSidebar, Some("NativeWorkspace")),
        KeyBinding::new("cmd-j", ToggleStatusBar, Some("NativeWorkspace")),
        KeyBinding::new("cmd-0", ShowAllEnvironments, Some("NativeWorkspace")),
        KeyBinding::new("cmd-1", ShowDevelopment, Some("NativeWorkspace")),
        KeyBinding::new("cmd-2", ShowTest, Some("NativeWorkspace")),
        KeyBinding::new("cmd-3", ShowStaging, Some("NativeWorkspace")),
        KeyBinding::new("cmd-4", ShowProduction, Some("NativeWorkspace")),
        KeyBinding::new("cmd-m", MinimizeWindow, Some("NativeWorkspace")),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, Some("NativeWorkspace")),
        KeyBinding::new("cmd-,", CredentialSettings, Some("NativeWorkspace")),
        KeyBinding::new("cmd-q", Quit, Some("NativeWorkspace")),
    ]);
    cx.set_menus([
        Menu::new("dbunk Native").items([
            MenuItem::action("Open Anything…", OpenAnything),
            MenuItem::action("Credentials…", CredentialSettings),
            MenuItem::action("Quit", Quit),
        ]),
        Menu::new("Query").items([
            MenuItem::action("New query", NewTab),
            MenuItem::action("Close query", CloseTab),
            MenuItem::action("Next query", NextTab),
            MenuItem::action("Previous query", PreviousTab),
            MenuItem::action("Rename query…", RenameTab),
            MenuItem::action("Pin query", PinTab),
            MenuItem::action("Move query left", MoveTabLeft),
            MenuItem::action("Move query right", MoveTabRight),
            MenuItem::action("Run statement", RunStatement),
            MenuItem::action("Run script", RunScript),
            MenuItem::action("Format SQL", FormatSql),
            MenuItem::action("Find in SQL…", FindInSql),
            MenuItem::action("Find next", FindNext),
            MenuItem::action("Find previous", FindPrevious),
            MenuItem::action("Stop query", StopQuery),
            MenuItem::action("Clear results", ClearResults),
            MenuItem::action("Insert snippet: Top rows", InsertTopRows),
            MenuItem::action("Insert snippet: Grouped count", InsertGroupedCount),
            MenuItem::action("Insert snippet: Recent rows", InsertRecentRows),
            MenuItem::action(
                "Refresh SQL completions",
                workbench::RefreshCompletionMetadata,
            ),
        ]),
        Menu::new("Results").items([
            MenuItem::action("Export retained rows…", grid::ExportCells),
            MenuItem::action("Inspect selected value", grid::InspectCell),
            MenuItem::action("Auto-fit selected column", grid::AutoFitColumn),
            MenuItem::action("Auto-fit visible columns", grid::AutoFitAllColumns),
            MenuItem::action("Pin / unpin selected column", grid::ToggleColumnPin),
            MenuItem::action("Go to retained row…", grid::GoToRow),
            MenuItem::action("Select current row", grid::SelectCurrentRow),
            MenuItem::action("Select all retained cells", grid::SelectAllCells),
            MenuItem::action("Copy retained selection", grid::CopyCells),
            MenuItem::action("Copy retained selection as CSV", grid::CopyCsv),
            MenuItem::action("Copy retained selection as JSON", grid::CopyJson),
            MenuItem::action("Copy retained selection as INSERT", grid::CopySql),
            MenuItem::action("Copy retained selection as Markdown", grid::CopyMarkdown),
            MenuItem::action("Copy retained selection as HTML", grid::CopyHtml),
            MenuItem::action("Copy retained selection as TXT", grid::CopyTxt),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle sidebar", ToggleSidebar),
            MenuItem::action("Toggle status bar", ToggleStatusBar),
            MenuItem::action("All environments", ShowAllEnvironments),
            MenuItem::action("Development connections", ShowDevelopment),
            MenuItem::action("Test connections", ShowTest),
            MenuItem::action("Staging connections", ShowStaging),
            MenuItem::action("Production connections", ShowProduction),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", MinimizeWindow),
            MenuItem::action("Zoom", ZoomWindow),
            MenuItem::action("Toggle Full Screen", ToggleFullScreen),
        ]),
        Menu::new("Connections").items([
            MenuItem::action("New connection…", NewConnection),
            MenuItem::action("Filter schemas and objects…", FocusNavigator),
            MenuItem::action("Toggle console", ToggleConsole),
            MenuItem::action("Disconnect query", Disconnect),
        ]),
    ]);
}

fn run() -> anyhow::Result<()> {
    let launch = launch::Launch::parse(
        std::env::args_os().skip(1),
        std::env::var_os("DBUNK_NATIVE_VERIFY").is_some(),
    )?;
    match file_log::install(launch.profile()) {
        Ok(path) => log::info!(
            "dbunk-native {} logging to {}",
            env!("CARGO_PKG_VERSION"),
            path.display()
        ),
        Err(error) => eprintln!("Native file logging unavailable: {error}"),
    }
    let workspace_mode = launch.workspace();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    if let Some(manifest) = runtime.block_on(launch.import_legacy())? {
        println!("{manifest}");
        return Ok(());
    }
    let backend = runtime.block_on(launch.open())?;
    let layout = runtime
        .block_on(backend.layout())
        .map_err(anyhow::Error::msg)?;
    // Advisory only: an unreadable record falls back to the default frame.
    let geometry = if workspace_mode {
        runtime.block_on(backend.window_geometry()).ok().flatten()
    } else {
        None
    };
    #[cfg(feature = "fixture-verification")]
    verification::initialize()?;
    let host = if workspace_mode {
        controller::Host::new_workspace(
            backend,
            runtime.handle().clone(),
            uuid::Uuid::new_v4().to_string(),
        )
    } else {
        controller::Host::new(backend, runtime.handle().clone())
    };
    let application_host = host.clone();
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            init_editor(cx).expect("native editor initialization");
            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            if workspace_mode {
                init_workspace_commands(cx);
                let primary = cx.primary_display();
                let mut displays = primary.iter().cloned().collect::<Vec<_>>();
                displays.extend(cx.displays().into_iter().filter(|display| {
                    primary
                        .as_ref()
                        .is_none_or(|primary| primary.id() != display.id())
                }));
                let described = displays
                    .iter()
                    .map(|display| {
                        let size = display.bounds().size;
                        window_geometry::Display {
                            uuid: display.uuid().ok().map(|uuid| uuid.to_string()),
                            width: f32::from(size.width),
                            height: f32::from(size.height),
                        }
                    })
                    .collect::<Vec<_>>();
                let (window_bounds, display_id) =
                    match window_geometry::restore(geometry.as_ref(), &described) {
                        Some(placement) => (
                            WindowBounds::Windowed(Bounds {
                                origin: gpui::point(px(placement.frame.x), px(placement.frame.y)),
                                size: size(px(placement.frame.width), px(placement.frame.height)),
                            }),
                            Some(displays[placement.display].id()),
                        ),
                        None => (WindowBounds::Windowed(bounds), None),
                    };
                let window = cx
                    .open_window(
                        WindowOptions {
                            window_bounds: Some(window_bounds),
                            display_id,
                            show: false,
                            focus: false,
                            titlebar: Some(gpui::TitlebarOptions {
                                title: Some("dbunk".into()),
                                appears_transparent: true,
                                traffic_light_position: Some(gpui::point(px(12.), px(11.))),
                            }),
                            is_movable: true,
                            app_owns_titlebar_drag: true,
                            ..Default::default()
                        },
                        |window, cx| {
                            let workspace = cx
                                .new(|cx| workspace::Workspace::new(application_host, window, cx));
                            window.on_window_should_close(cx, move |window, cx| {
                                if let Some(Some(workspace)) = window.root::<workspace::Workspace>()
                                {
                                    workspace
                                        .update(cx, |workspace, cx| workspace.close(window, cx));
                                }
                                false
                            });
                            workspace
                        },
                    )
                    .expect("native workspace window");
                window
                    .update(cx, |_, window, _| window.activate_window())
                    .expect("activate native workspace");
            } else {
                let window = cx
                    .open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(bounds)),
                            show: false,
                            focus: false,
                            ..Default::default()
                        },
                        |window, cx| {
                            let workbench =
                                cx.new(|cx| Workbench::new(application_host, layout, window, cx));
                            window.on_window_should_close(cx, move |window, cx| {
                                if let Some(Some(workbench)) = window.root::<Workbench>() {
                                    workbench.update(cx, |workbench, cx| workbench.close(cx));
                                }
                                false
                            });
                            workbench.update(cx, |workbench, cx| workbench.start(window, cx));
                            workbench
                        },
                    )
                    .expect("native window");
                // Start visibility after GPUI registers the root and frame callbacks.
                // Showing during construction can leave macOS's display link dormant.
                window
                    .update(cx, |_, window, _| window.activate_window())
                    .expect("activate native window");
            }
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
    // GPUI's quit hooks have a 200 ms deadline. Own the final join outside its
    // event loop so OS-driven quit also waits for sockets and profile storage.
    let result = runtime
        .block_on(host.shutdown())
        .map_err(anyhow::Error::msg);
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    result
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Before the logger is installed, stderr is the only record.
            if log::max_level() == log::LevelFilter::Off {
                eprintln!("Native host: {error:#}");
            } else {
                log::error!("Native host: {error:#}");
            }
            ExitCode::FAILURE
        }
    }
}
