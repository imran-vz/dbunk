# Plan 024: Tauri baseline, external measurement harness and GPUI spike

- Priority: P1. Effort: L. Risk: HIGH. Source: stage 01 of the
  [GPUI migration proposal](./gpui-migration.html).
- Planned against: `102568b`, 2026-10-02.
- Depends on: stage 00 of the proposal (decided 2026-10-02). Step 3 uses the
  config-directory change from Plan 025 Step 3.
- Execution status: see [README.md](./README.md).
- Category: measurement and throwaway spike. Nothing in this plan ships. The
  spike lives outside the application's Cargo graph and is deleted or replaced
  when stage 03 starts.

> **Executor instructions**: read this plan and the proposal's "Why migrate,
> and when to stop" section completely before starting. Follow the steps in
> order unless a step says it is independent. Update the Plan 024 row in
> `plans/README.md` after each completed step. Stop on every STOP condition.
> The stage 01 gate is Imran's decision; this plan produces the evidence and a
> recommendation, and marks `READY FOR REVIEW`. Commits, pushes and PRs need
> separate authorization.

## Outcome

A written answer to the four continue criteria, with the measurements, so the
migration either continues to stage 03 or is recorded as `REJECTED`.

Fixed in stage 00 and not open here:

- Reason: architecture. Criterion 1 is met by the recorded statement that no
  performance gain is required.
- Criterion 2: no measured metric more than 10% worse than Tauri. Measured
  set: startup, editor key-to-pixel latency, scroll frame pacing on the
  wide-row and large-cell fixtures, process-tree memory, idle CPU.
- Criterion 3: the editor, grid, accessibility and schema-map probes show no
  capability gap without a costed fix.
- Criterion 4: the chosen dependency path builds from a clean checkout with
  one GPUI package and the accepted licence (GPL-3.0-or-later), and starts
  without a network service, telemetry or a Node runtime.
- Dependency path: Z (Zed's editor on Zed's in-tree GPUI) is built first.
  Path K's grid and controls are considered only if a compile shows they share
  one GPUI package with the editor.

Because the reason is architecture, the gate also needs the cost side written
down for Imran: what the second host removes beyond core extraction alone,
against the gaps, build times, dependency closure and upstream tracking cost
found here.

## Facts established on 2026-10-02

- Pinned Zed revision `506beb34de3f433707b7ebe8d8ad2d80f856af6c` exists
  (committed 2026-10-01T20:03:55Z). Its `rust-toolchain.toml` requires Rust
  1.98.1. This machine has 1.97.1, so the spike carries its own toolchain
  file.
- `crates/editor/Cargo.toml` at that revision is GPL-3.0-or-later and depends
  unconditionally on `project`, `workspace`, `client`, `rpc`, `lsp`, `dap`,
  `db`, `git`, `telemetry`, `settings`, `theme` and `ui`.
- From this session's shell, `CGPreflightScreenCaptureAccess`,
  `AXIsProcessTrusted` and `CGPreflightPostEventAccess` all return true. Plan
  022's validation was denied both capture and synthetic input, so recheck in
  the shell that runs the harness.
- Machine: Apple M4 Pro, 14 cores, 48 GB, macOS 27.0.1, built-in Liquid
  Retina XDR display (ProMotion). The machine was on battery when checked.
- `DBUNK_DEV_CONFIG_DIR` is honored in debug builds only
  (`src-tauri/src/storage.rs:461-471`). A release build always uses
  `~/.config/dbunk`, which is the daily-driver profile. Plan 022 recorded that
  the variable does not isolate the keychain entry or WebView storage.
- PostgreSQL fixtures are Docker compose services under
  `infrastructure/test-db/` (`pnpm db:postgres`, port 15432).

## Isolation rules for every run in this plan

- Tauri measurement builds use the `isolated-profile` Cargo feature (Step 3),
  a config directory under `/tmp/dbunk-plan024/`, a separate bundle
  identifier (`codes.imran.dbunk.plan024`) and their own Cargo target
  directory. The feature is never enabled in a release artefact.
- Fixture profiles use plain SQLite credential storage, so no keychain entry
  is created or read.
- Databases are the disposable compose fixtures only.
- The harness posts key events to the target process only (`postToPid`), so
  nothing can be typed into another application. Scroll steps cannot be
  delivered that way to a WebView, so they go through the session event tap:
  the pointer is parked over the target window, checked several times a
  second, and the run is discarded if the pointer leaves or another window
  covers it. A run brings the target window forward and needs the machine
  left alone for its duration (about six minutes per host).

## Steps

### Step 1: Path Z dependency proof (independent)

1. Create `spikes/gpui-path-z/`: its own `[workspace]`, `publish = false`,
   `license = "GPL-3.0-or-later"`, `rust-toolchain.toml` at 1.98.1, Git
   dependencies on `gpui` and `editor` pinned to the revision above, and its
   own target directory. It is not a member of any application workspace.
2. Build a window that hosts `Editor::for_buffer(buffer, None, ..)` on a
   plain-text buffer. Record every initialization call that was required
   (settings store, theme, assets, language registry, client, fs, etc.) and
   which of them construct a network client, telemetry or a Node runtime.
3. Record: resolved crate count and the GPUI package count (`cargo tree -d`
   must show one `gpui`), clean build time, incremental rebuild time after a
   one-line view change, binary size, and licence of every crate in the
   closure that is not MIT, Apache-2.0, BSD, ISC, Zlib, Unicode or MPL-2.0.
4. Start the binary with networking denied (`sandbox-exec` deny network) and
   confirm it opens, edits and quits. Check that no child process is spawned.
5. Only if step 2 succeeds: add `gpui-component` (GPUI Kit) to a scratch
   branch of the spike and record whether it resolves to the same `gpui`
   package. Expected: it does not (`gpui` versus `gpui-pre`).

Gate: criterion 4 answered yes or no with the output attached.
STOP: the editor cannot be constructed without a network client, telemetry or
Node being started, and there is no initialization path that keeps them
inert.

### Step 2: External measurement harness

`tools/measure/`, a Swift package, one tool for both hosts.

- Input latency: post key events with `CGEvent`; capture the window with
  ScreenCaptureKit at the display's refresh rate; report the interval from
  post to the display time of the first frame whose pixels changed. p50 and
  p95 over 300 keystrokes at 120 ms, three runs. State the capture resolution
  (one refresh interval) with every result. ScreenCaptureKit's own frame
  status and dirty rectangles are not used: for a window it reports every
  refresh as a full-window change.
- Scrolling: post scroll-wheel events at a fixed velocity; report frame
  intervals from the same capture stream: p50, p95, and frames over 1.5
  refresh intervals.
- Memory and idle: sum `phys_footprint` and CPU time over the whole process
  tree, found by walking children and, for Tauri, the WebKit XPC helpers
  whose responsible PID is the app. Sample at rest for 60 s for idle CPU.
- Startup: launch to the first frame in which the editor region is drawn and
  a posted keystroke lands.
- Output: one JSON file per run with machine, display mode, power state,
  window size, build identity and raw samples.

Self-test before trusting it: a calibration target (a small AppKit window in
the same package that paints a counter on key-down) must measure within one
refresh interval of its known behavior, and two consecutive runs on the same
target must agree within 10% at p95.

Gate: calibration passes on this machine. If two runs cannot agree within
10%, the 10% tolerance in criterion 2 is not measurable; STOP and report.

### Step 3: Tauri baseline

1. Add the `isolated-profile` feature: when enabled, the config directory
   override is honored in release builds. Off by default; CI builds without
   it. This lands with Plan 025 Step 3, which moves path resolution to the
   host edge.
2. Build a release binary with the production frontend bundle, the feature,
   the separate identifier and a separate target directory.
3. Fixtures (`tools/measure/fixtures/`), sized to stay inside the Query
   Session retention limits (10,000 rows per Result Set, 32 MiB per
   execution), so both hosts hold the same data:
   - editor: a 2,000-line SQL document;
   - wide rows: 10,000 rows by 100 short columns;
   - large cells: 400 rows by 8 columns of 8,192 characters;
   - many rows: 10,000 rows by 8 mixed columns.
4. Measure startup, typing latency, scroll pacing on each grid fixture,
   memory after each fixture is loaded, memory after closing the tab, and
   idle CPU. Three runs each. Same window size (1440 by 900 points), display
   mode and power state for every run in this plan.

Gate: baseline JSON recorded under `plans/evidence/024/tauri/` with the
summary table in this file. This baseline is also the profile for the cheaper
alternative.

### Step 4: Native spike

In `spikes/gpui-path-z/`, on the path chosen in Step 1:

- SQL editor: Zed's editor with a SQL language (Tree-sitter grammar), a
  custom `CompletionProvider` returning fixture schema names, search, undo
  and redo, multi-cursor, clipboard, IME composition, and current-statement
  detection wired to a Run action.
- Grid: virtualized rows and columns over a bounded result model, fed by a
  synthetic stream that delivers the four fixtures in batches of 200 rows
  with the retention caps of `query_session/postgres.rs`. Selection, copy and
  column resize.
- No database, no persistence, no chrome.

Measure with the Step 2 harness, same conditions as Step 3.

Gate: results recorded under `plans/evidence/024/native/`. Each of the
listed editor and grid behaviors is marked works, partial with the gap named,
or absent.

### Step 5: Probes for parts with no ready-made component

- Schema map: a canvas with pan, zoom, nodes and edges at the size of the
  largest fixture schema. Record frame pacing while panning.
- One specialized cell editor (the JSON editor of ADR-0014).
- Accessibility: read the AX tree of the editor, the grid and one form with
  the Accessibility API and record roles, names and focus order beside the
  same dump from the Tauri app. Operating the spike with VoiceOver needs a
  person; list it for Imran with the exact steps.

Gate: each probe has a result and, for every gap, a costed fix or a named
blocker.

### Step 6: Inventories

- Browser-provided behavior (the proposal's scope table): each item marked
  available, buildable with an estimate, or out of scope.
- Parity checklist against `102568b`, from the parity gap register, the
  command list in `src-tauri/src/lib.rs`, the accepted ADRs and the frontend
  test names. Existing limitations marked.
- Iteration speed: Step 1's rebuild times beside a Vite hot reload of a
  one-line component change.

### Step 7: Decide and record

- The dependency set with exact versions and licence notices, the SQL
  grammar and formatter strategy, and the packaging toolchain.
- The four criteria, each with its evidence.
- The cost comparison against core extraction alone.
- A recommendation: continue, revise or reject. Mark `READY FOR REVIEW`.

## STOP conditions

- A step would touch `~/.config/dbunk`, the daily-driver bundle identifier,
  a keychain entry or a non-fixture database.
- The pinned Zed revision has to change to make the spike build.
- Code under a licence other than GPL-3.0-or-later-compatible would enter the
  tree.
- GPL code would be linked into the Tauri application or copied outside
  `spikes/`.
- The harness cannot be calibrated (Step 2 gate).

## Not in this plan

- Any change to the application's `LICENSE`, manifests or README. The
  proposal records the timing: the commit that first makes a distributed
  artefact depend on a GPL crate, not before the stage 01 gate.
- Any real database access from the native spike. Stage 03 does that through
  the core.
- Windows and Linux.

## Execution record (2026-10-02)

Run against `102568b` plus the Plan 025 working tree. Nothing is committed.
All seven steps were carried out. Remaining checks are listed under "Not run";
Imran confirmed the editor VoiceOver check complete on 2026-10-02. Evidence is in `plans/evidence/024/`.

The later [editor accessibility follow-up](./evidence/024/editor-accessibility.md)
corrects the original claim that text semantics require a GPUI fork. A dbunk
adapter now uses the pinned public APIs. The original performance measurements
predate that adapter; the geometry/focus follow-up and final gate review are
recorded in [stage01-gate.md](./evidence/024/stage01-gate.md).

### The four criteria

| Criterion | Result | Evidence |
| --- | --- | --- |
| 1. Primary metric at least 30% better, or the statement that no gain is required | **Met by the statement** recorded in stage 00. For the record, typing latency p95 is 39% lower. | `measurements.md` |
| 2. No measured metric more than 10% worse than Tauri | **Met.** Eighteen metrics; the worst is +1% on a 9.9 ms frame interval. | `measurements.md` |
| 3. The editor, grid, accessibility and schema-map probes show no capability gap without a costed fix | **Met.** The public-API adapter now supplies text geometry; external AX checks verify editor/results/cell-editor keyboard focus. VoiceOver was confirmed complete by Imran on 2026-10-02. Remaining full-application gaps have estimates and deadlines in the gate review. | `stage01-gate.md`, `editor-accessibility.md`, `probes.md`, `browser-provided-behavior.md` |
| 4. One GPUI package, accepted licence, builds clean, starts with no network, telemetry or Node | **Met**, with two build prerequisites (Rust 1.98.1 and Xcode's Metal Toolchain). | `step1-dependency-path.md` |

### Gate decision, 2026-10-02

**PASS: continue.** Imran requested completion of geometry and keyboard focus
verification, then review and closure of the stage 01 gate. All four criteria
are met, including the costed remaining work required by criterion 3. The
[gate review](./evidence/024/stage01-gate.md) records the evidence, costs and
limits. No accessibility exception is used.

This closes the viability gate. Later native implementation still needs its
own plan, and new UI still needs the required mock selection. It does not
approve daily-driver cutover or Plan 025. Plan 024's status stays
`READY FOR REVIEW` until an authorized commit supplies a completion SHA;
that bookkeeping does not leave the stage 01 gate open.

### Cost comparison against core extraction alone

The reason recorded in stage 00 is architecture, and the cheaper alternative
is core extraction with no second host.

What core extraction alone gives, from Plan 025: the backend builds and tests
with no Tauri crate in its graph (362 crates against 555). The coupling below
the command layer was fifteen lines in fourteen files. It cost one plan.

What only a second host removes: the 174 registered commands and the
serialization on both sides; the sequence, ACK and credit protocol as an IPC
mechanism (ADR-0030, ADR-0031); renderer-reload lifecycle; the TypeScript
mirrors of Rust types; and, at cutover, Vite, `tsc`, oxlint, oxfmt and
Vitest.

What the second host costs, from this plan:

- 75 parity items (`parity-checklist.md`). Eight were spiked on synthetic
  data. The rest is 433 TypeScript files to replace.
- A form layer to build: radio groups, number fields, toasts, split panes,
  and an answer for every place users select and copy text today
  (`browser-provided-behavior.md`).
- A dependency on Zed at one Git revision: 102 Zed crates, 73 of them GPL,
  811 packages in all, no stable API, a newer Rust toolchain than the
  application uses, and a 29 GB build directory. Mixing in GPUI Kit is not
  possible without a fork: it brings a second GPUI package.
- A slower loop: about two seconds per view change plus a relaunch that
  loses state, against hot reload (`iteration-speed.md`).
- An application-owned editor accessibility adapter to maintain and verify
  (`editor-accessibility.md`), plus bidirectional text, which is unverified.

What it buys beyond the architecture, measured: typing latency p95 24.7 ms
against 40.7; idle CPU 1.3% of a core against 5.5%; idle footprint 231 MiB
against 523; startup to main content 0.2 s against 0.9 s. The native side is
a spike with none of the application's chrome or storage, so these are a
floor for the hard parts, not a forecast.

### Recommendation

**Continue on path Z.** The geometry and focus follow-up resolves the remaining
spike accessibility checks without a dependency fork. Accept the documented
rewrite and upstream-maintenance cost for the architecture reason chosen in
stage 00. Preserve the remaining estimates and verification deadlines in the
[gate review](./evidence/024/stage01-gate.md). Stage 02 service extraction
remains useful independently; this review does not mark it complete.

### Step by step

- **Step 1, dependency path.** Built and run. `step1-dependency-path.md`.
- **Step 2, harness.** `tools/measure/`: Swift, about 1,200 lines. Calibrated
  against a target with a known delay (53 ms measured for 50 ms injected).
  Four defects found and fixed during calibration and the first runs; they
  are listed in `measurements.md` because each one would have produced a
  wrong number.
- **Step 3, Tauri baseline.** Two release binaries with `isolated-profile`
  and their own identifier, on a profile under `/tmp/dbunk-plan024/`. The
  fixture connection ran through a real Query Session, which also exercised
  the Plan 025 refactor in a release build through the real IPC channel.
- **Step 4, native spike.** `spikes/gpui-path-z/`, about 1,300 lines.
- **Step 5, probes.** Schema map, cell editor, accessibility. `probes.md`,
  `accessibility-probe.md`.
- **Step 6, inventories.** `browser-provided-behavior.md`,
  `parity-checklist.md`, `iteration-speed.md`.
- **Step 7, decisions.**
  - Dependency set: Zed at `506beb3` (`gpui`, `gpui_platform`, `editor`,
    `language`, `settings`, `theme`, `theme_settings`, `assets`,
    `release_channel`, `project`, `text`, `multi_buffer`, `util`), Rust
    1.98.1, `tree-sitter-sequel` 0.3.11 (MIT) and `tree-sitter-json` 0.24.
    Exact versions are in `spikes/gpui-path-z/Cargo.lock`.
  - SQL grammar: Tree-sitter, compiled in. Completion: a
    `CompletionProvider` over the connection's schema cache. Statement
    boundaries: the backend's own lexer (`postgres::sql_lex`), so the editor
    and the safety classifier agree; it rejects non-ASCII outside strings and
    comments today, which the native editor would inherit.
  - SQL formatter: **not chosen.** The React app uses a JavaScript formatter.
    A Rust replacement has to be picked and compared on the dialect
    fixtures.
  - Packaging toolchain: **not chosen.** No bundle was built. Zed bundles
    with its own script; a GPUI app needs an `.app` layout, an icon set, an
    `Info.plist` and a DMG step written for it. This belongs to stage 07 but
    should be probed before stage 04 commits to chrome.

### Departures from the plan as written

- Fixtures are smaller than first written, to fit the Query Session
  retention limits. The plan text above is corrected.
- Typing runs are 300 keys, not 500, three times.
- Input is posted to the target process for keys, and through the session
  event tap for scrolling and clicks. The isolation rules above are
  corrected.
- Change detection compares pixels. ScreenCaptureKit's dirty rectangles
  turned out to be unusable for a window capture.
- Startup has three numbers instead of one "first interactive frame".
- `.oxfmtrc.json` ignores `plans/evidence`: the harness writes JSON the
  formatter would otherwise reject.
- `tools/measure/tauri/vite.config.ts` wraps the project Vite config for
  measurement builds. `pnpm build:vite` cannot run on this machine while
  another project holds port 3000: the prerender preview inherits
  `strictPort` and binds 3000 whatever port it is asked for. That is a
  latent problem for `pnpm tauri build` too, and is not fixed here.

### Not run

- IME composition in the editor and in the cell editor. Needs a person with
  an input method.
- Bidirectional text in grid cells.
- Zoom and node dragging in the schema map, single-click selection, column
  resize, undo and redo, find and multi-cursor: implemented or inherited,
  not exercised by the harness.
- Vite hot-reload timing (port 3000 was in use).
- A bundled `.app`, signing, or a DMG.
- Any platform other than macOS on Apple Silicon.

### Left on the machine

- Rust 1.98.1, installed by `rustup` beside the existing toolchain.
- `spikes/gpui-path-z/target/release` (10 GB) and `/tmp/dbunk-plan024/`
  (fixture profile and the two Tauri binaries). Both can be deleted. The
  spike's dev build directory (19 GB) and the Tauri measurement target
  directory were removed.
- Nothing running. The disposable PostgreSQL container was removed with
  `docker compose down` and OrbStack, which this session started, was
  stopped. To repeat the Tauri runs: `pnpm db:postgres`, then load
  `tools/measure/fixtures/postgres.sql`.
- Two system dialogs that this session caused and did not answer: a crash
  report for the spike's first launch, and a macOS prompt asking whether T3
  Code may bypass the window picker for screen capture.
