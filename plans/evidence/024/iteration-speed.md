# Plan 024 Step 6: iteration speed

2026-10-02, Apple M4 Pro, fourteen cores.

| Loop | Time |
| --- | --- |
| Native, dev build: change one string in the spike's `grid.rs`, `cargo build` | 1.9 s (1.87, 1.85, 1.94), then relaunch the app |
| Native, release build: same kind of change | 90 s (thin LTO, one codegen unit) |
| Native, clean dev build of the whole closure | about 82 s |
| Native, clean release build | 357 s |
| Tauri backend, clean release build | 107 s |
| React UI through Vite hot reload | not measured; see below |

The Vite number was not taken. `pnpm dev` pins port 3000 with `strictPort`,
and another project's dev server was using that port for the whole session.
Hot module reload updates a component in place, without restarting the app or
losing its state, typically well under a second.

What changes for day-to-day work:

- A native view change costs about two seconds to build, plus a relaunch and
  the steps to get back to the screen being worked on. State is lost on every
  change. The React loop keeps state.
- The spike's own code is small. The two-second figure will grow with the
  application crate, and it grows faster if the UI lives in one crate.
  Splitting views across crates is how Zed keeps its own loop short.
- Release builds are for measurement and packaging only. Ninety seconds per
  change rules them out for iteration.
- Agent-driven work loses the WebView's DOM and the evaluation bridge. What
  replaced them in this plan: window screenshots, the accessibility dump, and
  synthetic input from `tools/measure`. A window that is not frontmost does
  not repaint, so every visual check has to bring it forward.
