# Plan 024 Step 1: dependency path Z

Zed's editor on Zed's in-tree GPUI, as Git dependencies pinned to
`506beb34de3f433707b7ebe8d8ad2d80f856af6c` (committed 2026-10-01). Built and
run on 2026-10-02 in `spikes/gpui-path-z/`, Apple M4 Pro, macOS 27.0.1,
Xcode 27.0. This replaces the proposal's "nothing compiled" caveat for path Z.

## Criterion 4

> The chosen dependency path builds from a clean checkout with one GPUI
> package and an accepted licence, and starts without a network service,
> telemetry or a Node runtime.

**Met, with two build prerequisites that a clean machine does not have.**

| Part | Result |
| --- | --- |
| Builds from a clean checkout | Yes, after the two prerequisites below. |
| One GPUI package | Yes. `gpui 0.2.2` from the Zed revision, once. |
| Licence | GPL-3.0-or-later (stage 00 decision 2). The spike crate declares it. |
| Starts without a network service | Yes. Launched under `sandbox-exec` with `(deny network*)`: the window opens and edits; the process holds no TCP or UDP socket. |
| Starts without telemetry | Yes. The `telemetry` and `client` crates are linked but nothing constructs a client. |
| Starts without a Node runtime | Yes. `node_runtime` is linked; no child process is spawned. |

Prerequisites:

1. **Rust 1.98.1.** Zed's `rust-toolchain.toml` requires it; the application
   builds on 1.97.1. The spike carries its own toolchain file, and `rustup`
   installed 1.98.1 beside the existing toolchain on first build.
2. **Xcode's Metal Toolchain, or runtime shaders.** GPUI compiles its Metal
   shaders at build time with the `metal` tool, which Xcode 27 ships as a
   separate download (`xcodebuild -downloadComponent MetalToolchain`). This
   machine does not have it, so the first build failed in `gpui_apple`'s build
   script. The spike enables `gpui_platform/runtime_shaders`, which compiles
   the shaders at launch instead. A shipped app should use build-time shaders,
   so CI and release machines need the component.

## What the editor needs before it can be constructed

In order, all in `init_editor_globals` (`src/main.rs`):

1. `Application::with_assets(assets::Assets)`
2. `settings::init`
3. `theme_settings::init(theme::LoadThemes::JustBase, ..)`
4. `release_channel::init(version, ..)`
5. `assets::Assets.load_fonts`
6. `editor::init`
7. `KeymapFile::load_asset_allow_partial_failure(DEFAULT_KEYMAP_PATH, ..)`:
   the default keymap names actions from crates the spike does not link, and
   those bindings are skipped.

Then `Editor::for_buffer(buffer, None, window, cx)` with `project` as `None`.

Not needed and not called: `workspace::init`, a `Project`, a `Client`, an
`Fs`, a `LanguageRegistry`, an HTTP client, a `NodeRuntime`.

SQL is not a built-in Zed language (Zed loads it as a WebAssembly extension).
The spike compiles `tree-sitter-sequel` 0.3.11 (MIT) in natively, builds a
`language::Language` from it and sets the theme on it by hand, which the
language registry would otherwise do. Completion is a `CompletionProvider`
implementation set on the editor; no language server is involved.

## Two things that bite an embedding application

- **Dev builds read Zed's assets from "the repository".** `util::fs_embed!`
  embeds assets in release builds, but in dev builds it reads them at runtime
  from the nearest ancestor of the executable that contains `.git`. For an
  embedding app that is the app's own checkout, which has no `assets/`, and
  `settings::init` panics on `settings/default.json`. The spike enables
  `util/debug-embed` so dev builds embed too.
- **`[patch]` does not travel.** Zed's root manifest patches six crates. A Git
  dependency does not inherit them, so the spike repeats the ones its closure
  needs (`tree-sitter-language`, `async-process`, `async-task`, `calloop`,
  `notify`, `notify-types`). The spike's `Cargo.lock` was seeded from Zed's so
  third-party versions match what Zed tests against.

## Dependency closure

| Measure | Value |
| --- | --- |
| Packages in the lockfile (all platforms) | 1,034 |
| Packages resolved for this host | 811 |
| Zed workspace crates among them | 102 |
| of which GPL-3.0-or-later | 73, plus `language_core`, which has no `license` field and ships `LICENSE-GPL` |
| of which Apache-2.0 | 28 (GPUI and its support crates) |
| Third-party crates outside MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, MPL-2.0 | 1: `libbz2-rs-sys` (bzip2-1.0.6, permissive) |
| For comparison: the Tauri app | 555 packages; 362 without the Tauri host |

Linked because `editor` depends on them unconditionally, and never
constructed by the spike: `project`, `workspace`, `client`, `rpc`,
`cloud_api_client`, `lsp`, `dap`, `remote`, `node_runtime`, `telemetry`,
`db`, `git`, and `wasmtime` (the extension host). `reqwest` is not in the
closure: without `Application::with_http_client` GPUI has no HTTP client.

## Build cost

Fourteen cores. The dev and release clean builds ran while other builds were
using the machine, so treat them as upper bounds.

| Measure | Value |
| --- | --- |
| Clean dev build | about 82 s wall (45 s to the Metal failure, 37 s to finish) |
| Clean release build (thin LTO, one codegen unit, as Zed configures it) | 357 s |
| Release rebuild after adding three source files and one dependency | 300 s |
| Dev rebuild after a one-line change in the spike's own code | 1.9 s |
| Release rebuild after a change in the spike's own code only | 90 s |
| Dev binary | 441 MB |
| Release binary | 101 MB, 78 MB stripped (the Tauri release binary is 48 MB, 37 MB stripped) |
| `target/` after dev and release builds | 29 GB by the end of the plan (19 GB dev, 10 GB release) |

## Mixing with GPUI Kit (path K)

Resolved in a scratch crate with both dependencies (`cargo tree`, no build):
Zed's editor brings `gpui 0.2.2` from Git, and `gpui-component 0.7.0` brings
`gpui-pre 0.3.7` with eight `gpui-pre-*` support crates. Two GPUI packages,
as the proposal predicted from the manifests. GPUI Kit's grid cannot share a
window with Zed's editor without a fork of one side.

**Consequence for the plan:** on path Z the grid and the form controls come
from GPUI primitives and Zed's own `ui` crate, not from GPUI Kit. The spike's
grid is built that way.

## Observed

- A GPUI window that is not frontmost stops repainting. Screenshots of an
  unfocused spike window show its first frame. This is the behavior the
  acceptance gate asks for ("idle windows do not request continuous frames"),
  and it means every visual check has to bring the window forward.
- Resident memory of the debug spike with the editor open and no result: 113
  MB. Release figures are in the measurement table.
