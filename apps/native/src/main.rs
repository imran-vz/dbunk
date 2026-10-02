//! Isolated macOS host for the stage 03 PostgreSQL fixture.
mod accessible_editor;
mod controller;
mod diagnostics;
mod grid;
#[cfg(test)]
mod live_tests;
mod mailbox;
mod results;
mod sql;
mod stream;
#[cfg(feature = "fixture-verification")]
mod verification;
mod workbench;

use gpui::{App, Bounds, KeyBinding, WindowBounds, WindowOptions, prelude::*, px, size};
use settings::{DEFAULT_KEYMAP_PATH, KeymapFile};
use std::{path::PathBuf, process::ExitCode};
use workbench::*;

fn init_editor(cx: &mut App) -> anyhow::Result<()> {
    settings::init(cx);
    theme_settings::init(theme::LoadThemes::JustBase, cx);
    release_channel::init(semver::Version::new(0, 0, 0), cx);
    assets::Assets.load_fonts(cx)?;
    editor::init(cx);
    let theme_settings = cx.update_global::<settings::SettingsStore, _>(|store, cx| {
        store.set_user_settings(r##"{"theme":"One Dark","experimental.theme_overrides":{"background":"#000000","editor.background":"#000000","editor.foreground":"#ffffff","editor.gutter.background":"#000000","error.background":"#000000","error.border":"#f87171","text":"#ffffff"}}"##, cx)
    });
    theme_settings.result()?;
    theme_settings::reload_theme(cx);
    cx.bind_keys(KeymapFile::load_asset_allow_partial_failure(
        DEFAULT_KEYMAP_PATH,
        cx,
    )?);
    cx.bind_keys([
        // Override Zed's editor newline/code-action bindings in their own context.
        KeyBinding::new("cmd-enter", RunStatement, Some("Editor")),
        KeyBinding::new("cmd-shift-enter", RunScript, Some("Editor")),
        KeyBinding::new("cmd-.", StopQuery, Some("Editor")),
        KeyBinding::new("cmd-enter", RunStatement, Some("Workbench")),
        KeyBinding::new("cmd-shift-enter", RunScript, Some("Workbench")),
        KeyBinding::new("cmd-.", StopQuery, Some("Workbench")),
        KeyBinding::new("cmd-q", Quit, Some("Workbench")),
        KeyBinding::new("f6", SwitchPane, Some("Workbench")),
        KeyBinding::new("shift-f6", SwitchPane, Some("Workbench")),
        KeyBinding::new("tab", SwitchPane, Some("ResultGrid && !Editor")),
        KeyBinding::new("shift-tab", SwitchPane, Some("ResultGrid && !Editor")),
        KeyBinding::new("f8", FocusToolbar, Some("Editor")),
        KeyBinding::new("f8", FocusToolbar, Some("Workbench")),
        KeyBinding::new("escape", LeaveToolbar, Some("NativeToolbar")),
        KeyBinding::new("tab", NextControl, Some("NativeToolbar")),
        KeyBinding::new("shift-tab", PreviousControl, Some("NativeToolbar")),
        KeyBinding::new("cmd-c", grid::CopyCells, Some("ResultGrid")),
        KeyBinding::new("up", grid::MoveUp, Some("ResultGrid")),
        KeyBinding::new("down", grid::MoveDown, Some("ResultGrid")),
        KeyBinding::new("left", grid::MoveLeft, Some("ResultGrid")),
        KeyBinding::new("right", grid::MoveRight, Some("ResultGrid")),
        KeyBinding::new("shift-up", grid::SelectUp, Some("ResultGrid")),
        KeyBinding::new("shift-down", grid::SelectDown, Some("ResultGrid")),
        KeyBinding::new("shift-left", grid::SelectLeft, Some("ResultGrid")),
        KeyBinding::new("shift-right", grid::SelectRight, Some("ResultGrid")),
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

fn run() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    anyhow::ensure!(
        args.next().as_deref() == Some(std::ffi::OsStr::new("--profile")),
        "Usage: dbunk-native --profile <isolated-profile>"
    );
    let profile = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("Missing isolated profile"))?,
    );
    anyhow::ensure!(
        args.next().is_none() && profile.is_absolute(),
        "An absolute isolated profile path is required"
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    let backend = runtime
        .block_on(dbunk_lib::backend::Backend::open_fixture(&profile))
        .map_err(anyhow::Error::msg)?;
    let layout = runtime
        .block_on(backend.layout())
        .map_err(anyhow::Error::msg)?;
    #[cfg(feature = "fixture-verification")]
    verification::initialize()?;
    let host = controller::Host::new(backend, runtime.handle().clone());
    let application_host = host.clone();
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            init_editor(cx).expect("native editor initialization");
            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
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
            eprintln!("Native host: {error:#}");
            ExitCode::FAILURE
        }
    }
}
