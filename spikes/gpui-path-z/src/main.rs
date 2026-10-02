//! Plan 024 spike: Zed's editor and a virtualized result grid in a bare GPUI
//! window, fed by a synthetic bounded result stream. No project, workspace,
//! client, language server, database or Node runtime is constructed.

mod accessible_editor;
mod fixture;
mod grid;
mod results;
mod schema_map;
mod sql;

use std::rc::Rc;

use editor::Editor;
use gpui::{
    App, Bounds, Context, Entity, Focusable, KeyBinding, Task, Window, WindowBounds, WindowOptions,
    actions, div, prelude::*, px, size,
};
use language::Buffer;
use multi_buffer::MultiBufferOffset;
use settings::{DEFAULT_KEYMAP_PATH, KeymapFile};
use theme::ActiveTheme;

use fixture::Fixture;
use grid::{CopyCells, ResultGrid};
use schema_map::SchemaMap;

actions!(spike, [RunStatement, SwitchPane]);

const EDITOR_FIXTURE: &str = include_str!("../../../tools/measure/fixtures/editor-2000.sql");

struct Workbench {
    editor: Entity<Editor>,
    accessible_editor: Entity<accessible_editor::AccessibleEditor>,
    grid: Entity<ResultGrid>,
    /// Identifies the running execution. A batch from an earlier one is
    /// dropped, as the Query Session generation check does.
    execution: u64,
    stream: Option<Task<()>>,
}

impl Workbench {
    fn switch_pane(&mut self, _: &SwitchPane, window: &mut Window, cx: &mut Context<Self>) {
        let focus = if self.grid.focus_handle(cx).contains_focused(window, cx) {
            self.editor.focus_handle(cx)
        } else {
            self.grid.read(cx).pane_focus(cx)
        };
        window.focus(&focus, cx);
    }

    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let language = sql::language(cx).expect("SQL language");
        let buffer = cx.new(|cx| {
            let mut buffer = Buffer::local(EDITOR_FIXTURE, cx);
            buffer.set_language(Some(language), cx);
            buffer
        });
        let editor = cx.new(|cx| {
            let mut editor = Editor::for_buffer(buffer, None, window, cx);
            editor.set_completion_provider(Some(Rc::new(sql::SchemaCompletions {
                names: vec![
                    "plan024",
                    "fixture_wide",
                    "fixture_large",
                    "fixture_many",
                    "orders",
                    "customers",
                    "order_lines",
                    "products",
                    "shipments",
                    "invoices",
                    "payments",
                ],
            })));
            editor
        });
        let accessible_editor =
            cx.new(|cx| accessible_editor::AccessibleEditor::new(editor.clone(), "SQL editor", cx));
        let grid = cx.new(ResultGrid::new);
        Self {
            editor,
            accessible_editor,
            grid,
            execution: 0,
            stream: None,
        }
    }

    /// The statement the cursor is in: the text between the semicolons on
    /// either side of it.
    fn statement_at_cursor(&self, cx: &mut Context<Self>) -> String {
        self.editor.update(cx, |editor, cx| {
            let text = editor.text(cx);
            let cursor = editor
                .selections
                .newest::<MultiBufferOffset>(&editor.display_snapshot(cx))
                .head()
                .0
                .min(text.len());
            let start = text[..cursor].rfind(';').map_or(0, |index| index + 1);
            let end = text[cursor..]
                .find(';')
                .map_or(text.len(), |index| cursor + index);
            text[start..end].trim().to_string()
        })
    }

    fn run_statement(&mut self, _: &RunStatement, _window: &mut Window, cx: &mut Context<Self>) {
        let statement = self.statement_at_cursor(cx);
        if let Some(fixture) = Fixture::from_sql(&statement) {
            self.run(fixture, cx);
        }
    }

    fn run(&mut self, fixture: Fixture, cx: &mut Context<Self>) {
        self.execution += 1;
        let execution = self.execution;
        self.grid
            .update(cx, |grid, cx| grid.begin(fixture.columns(), cx));
        // Replacing the task drops the previous stream.
        self.stream = Some(cx.spawn(async move |this, cx| {
            let mut next_row = 1;
            loop {
                // Rows are produced off the UI thread, as a driver would.
                let (batch, resumed_at) = cx
                    .background_spawn(async move {
                        let mut next_row = next_row;
                        let batch = results::next_batch(fixture, &mut next_row);
                        (batch, next_row)
                    })
                    .await;
                next_row = resumed_at;
                let Some(batch) = batch else { break };
                let current = this.update(cx, |this, cx| {
                    if this.execution != execution {
                        return false;
                    }
                    this.grid.update(cx, |grid, cx| grid.push(batch, cx));
                    true
                });
                if !matches!(current, Ok(true)) {
                    return;
                }
            }
            this.update(cx, |this, cx| {
                if this.execution == execution {
                    this.grid.update(cx, |grid, cx| grid.finish(cx));
                }
            })
            .ok();
        }));
    }
}

impl Render for Workbench {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        div()
            .key_context("Workbench")
            .on_action(cx.listener(Self::run_statement))
            .on_action(cx.listener(Self::switch_pane))
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.editor_background)
            .text_color(colors.text)
            .child(div().h(px(28.)).flex_shrink_0())
            .child(
                div()
                    .h(gpui::relative(0.45))
                    .w_full()
                    .child(self.accessible_editor.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(colors.border)
                    .child(self.grid.clone()),
            )
    }
}

/// The window's root: the workbench, or one probe on its own.
enum Root {
    Workbench(Entity<Workbench>),
    Map(Entity<SchemaMap>),
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        match self {
            Self::Workbench(workbench) => workbench.clone().into_any_element(),
            Self::Map(map) => map.clone().into_any_element(),
        }
    }
}

/// Everything the editor needs before it can be constructed. Each call is
/// recorded in the Plan 024 evidence; none of them opens a socket or spawns
/// a process.
fn init_editor_globals(cx: &mut App) -> anyhow::Result<()> {
    settings::init(cx);
    theme_settings::init(theme::LoadThemes::JustBase, cx);
    release_channel::init(semver::Version::new(0, 0, 0), cx);
    assets::Assets.load_fonts(cx)?;
    editor::init(cx);
    // The default keymap names actions from crates this binary does not link
    // (agent, git UI, terminal). Bindings whose action is absent are skipped.
    cx.bind_keys(KeymapFile::load_asset_allow_partial_failure(
        DEFAULT_KEYMAP_PATH,
        cx,
    )?);
    // Bound after the defaults, so these win inside the same context.
    cx.bind_keys([
        KeyBinding::new("cmd-enter", RunStatement, Some("Editor")),
        KeyBinding::new("f6", SwitchPane, Some("Workbench")),
        KeyBinding::new("shift-f6", SwitchPane, Some("Workbench")),
        KeyBinding::new("tab", SwitchPane, Some("ResultGrid && !Editor")),
        KeyBinding::new("shift-tab", SwitchPane, Some("ResultGrid && !Editor")),
        KeyBinding::new("enter", grid::EditCell, Some("ResultGrid && !Editor")),
        KeyBinding::new("cmd-c", CopyCells, Some("ResultGrid")),
        KeyBinding::new("cmd-s", grid::CommitCell, Some("CellEditor > Editor")),
    ]);
    Ok(())
}

fn main() {
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            init_editor_globals(cx).expect("editor globals");
            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    // Probe: `DBUNK_SPIKE_MAP=<tables>` opens the schema map
                    // alone instead of the workbench.
                    let tables = std::env::var("DBUNK_SPIKE_MAP")
                        .ok()
                        .and_then(|tables| tables.parse::<usize>().ok());
                    if let Some(tables) = tables {
                        let map = cx.new(|_| SchemaMap::new(tables));
                        return cx.new(|_| Root::Map(map));
                    }
                    let workbench = cx.new(|cx| Workbench::new(window, cx));
                    workbench.update(cx, |workbench, cx| {
                        window.focus(&workbench.editor.focus_handle(cx), cx);
                        // Measurement runs start with a fixture already loading.
                        let preload = std::env::var("DBUNK_SPIKE_FIXTURE").ok();
                        if let Some(fixture) = preload.as_deref().and_then(Fixture::from_name) {
                            workbench.run(fixture, cx);
                        }
                    });
                    cx.new(|_| Root::Workbench(workbench))
                },
            )
            .expect("open window");
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            // Smoke runs leave focus where it is; measurement runs need it.
            if std::env::var_os("DBUNK_SPIKE_NO_ACTIVATE").is_none() {
                cx.activate(true);
            }
        });
}
