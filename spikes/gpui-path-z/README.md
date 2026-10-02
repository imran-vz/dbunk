# GPUI spike

Isolated, synthetic Plan 024 application. It does not open a database,
load a dbunk profile, or access the keychain. Run commands from this directory
so rustup selects the pinned toolchain.

```sh
cargo build --release --locked
./target/release/dbunk-gpui-spike
```

`DBUNK_SPIKE_FIXTURE=many` preloads synthetic query results. The window starts
with the shared 2,000-line SQL fixture; Cmd-Enter runs a recognized fixture
statement. This is not the real PostgreSQL workflow.

## Editor accessibility

`AccessibleEditor` wraps the existing editor without changing its layout or
editing behavior. It exposes text, primary caret/selection, focus and shaped
character bounds using GPUI's public AccessKit APIs. Selection requests go back into
Zed's editor, including autoscroll. Text runs are built only when accessibility
is active and cached until the buffer changes. No dependency fork is needed.

Visible runs follow soft-wrap rows, resize and scrolling. Offscreen text
remains readable without stale bounds. The JSON cell editor uses the same
adapter. Secondary cursors, folded/inlay text and bidirectional layout need
further work before full editor parity. Imran confirmed human VoiceOver
validation complete on 2026-10-02; the automated geometry/focus follow-up is in
`plans/evidence/024/stage01-gate.md`.

F6 or Shift-F6 switches between SQL and results, restoring an open cell editor
on return. Tab or Shift-Tab returns from results to SQL; Tab inside an editor
keeps its editing behavior. Enter opens the selected result cell, or the first
cell when none is selected. Escape discards and Cmd-S stages a cell edit, both
returning focus to results.

Run the focused macOS round trip from the repository root, against this spike's
PID only. It replaces the synthetic SQL document while testing and leaves a
small Unicode fixture open. It does not use the clipboard.

```sh
swiftc tools/measure/editor-accessibility.swift -o /tmp/dbunk-editor-ax
/tmp/dbunk-editor-ax <spike-pid>
```

Native verification (from this directory):

```sh
cargo fmt --check
cargo clippy --release --locked --all-targets -- -D warnings
cargo test --release --locked
```

Repository checks also apply: `pnpm format`, `pnpm lint`, `pnpm typecheck`,
`just fmt`, `just lint`, and `just test`. The root `just` recipes check the
backend; they do not include this separate spike workspace.
