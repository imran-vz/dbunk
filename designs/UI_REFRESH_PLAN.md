# dbunk UI Refresh — Implementation Plan

**Companion document:** `designs/DESIGN-SYSTEM.md` (the target spec; this plan is the route to it).
**Basis:** full codebase audit (2026-08-23), reference research on dense developer tools (VS Code, JetBrains/DataGrip, TablePlus, Zed, Linear, Warp, Raycast, ChatGPT macOS, Codex), and a decision interview with the project owner.
**Rollout:** phases ship individually as completed (pre-1.0 alpha; churn cost accepted). Each phase leaves the app internally consistent within the surfaces it touched. Each phase can be broken into `plans/NNN-*.md` executor plans when implementation starts.

---

## 1. Decision record (interview outcomes — binding)

| # | Decision |
|---|---|
| D1 | Delete the legacy shell (~3,000 unreachable lines); salvage the database overview as a workbench rail view |
| D2 | App-wide density setting: Compact / Default / Comfortable; metrics-only (typography fixed); **remove** width-based auto-compact |
| D3 | Resizable editor/results split; **dock retained** as a global console (connection events, notices, task progress, query log); results pane holds Results + Explain per tab |
| D4 | Keep the activity rail, slimmed to 40px; Connections promoted to a first-class rail item; header connection switcher rebuilt on the menu primitive |
| D5 | Unified `Cmd+K` palette + contextual `Cmd+K` on grid selections |
| D6 | Keyboard scheme adopted as specified in DESIGN-SYSTEM §6.1 |
| D7 | Full session restore: layout state global; content state (tabs, view state, column widths, hot-exit SQL, tree expansion) per connection; window geometry via `tauri-plugin-window-state` |
| D8 | Sash gestures: double-click = auto-fit-to-content; **Alt+double-click = collapse**; drag below threshold snaps closed |
| D9 | JetBrains Mono first for all mono surfaces (incl. Monaco); system sans for UI |
| D10 | Thin auto-hiding overlay scrollbars (~8px) restored on all data surfaces |
| D11 | Import/Copy/Seed → modal dialogs; Add Row → in-grid staged insertion; Bulk Edit → right-side mutation panel |
| D12 | All four theme presets kept, colors refined; amber accent retained; cool-tinted dark ramp rebuilt systematically; dark default preset is design-primary |
| D13 | Re-expose all three hidden table sub-views (Relations, per-table Schema Map, Specialized) |
| D14 | Grid cells 12px mono; rows 22/26/30px by density |
| D15 | Dock: `Ctrl+`` toggle, hidden by default, badge instead of auto-open; results: `Cmd+J`, collapses to status strip |
| D16 | Panel metrics per DESIGN-SYSTEM §3.3; mutation review unified onto the standard right-panel primitive |
| D17 | Connection safety in scope: environment tags (dev/staging/prod ambient color) + per-connection read-only flag |

---

## 2. Phase overview & dependency graph

```
P0 demolition ─▶ P1 tokens ─▶ P2 primitives ─▶ P3 shell/panels ─▶ P4 query surface ─▶ P5 grid ─▶ P6 table editor & safety
                                            └▶ P7 keyboard/command (after P3; palette items land per phase)
                                            └▶ P8 persistence (after P3; extended in P4–P6)
P9 remaining screens (after P2, ideally after P5) ─▶ P10 theme refinement & verification sweep (last)
```

P0–P2 are strictly sequential. P4→P5→P6 are sequential. P7/P8/P9 can interleave once P3 lands.

---

## Phase 0 — Demolition & correctness

**Status: implemented** (branch `ui-refresh-p0`). Notes: the dock was reduced to an output-only surface that reveals on section change (the minimal wiring for change 2); tests covering legacy-only UI (per-tab connection selector, legacy sub-tab row, result-set chips) were removed or retargeted — P4/P6 restore those affordances with new UI and tests.

**Why first:** ~3,000 lines of unreachable UI would otherwise be redesigned for nothing, and two shipped defects actively mislead users.

**Changes**
1. Delete the dead shell: `app-shell.tsx:299-334` branch, `app-shell/app-shell-header.tsx`, `sidebar.tsx`, `workspace-view.tsx`, `workspace-tabs.tsx`, unreachable `workspace-overview/*` (keep `admin-tab`, `query-history-tab`, `schema-map-tab`, `disconnected-card`; park overview cards' data hooks for P9's overview rail view). Remove `usesWorkbenchShell` and the `variant="default"` branches in `query-editor-panel.tsx` / `table-editor/header.tsx` — workbench is the only shell.
2. Fix the inert Results/Explain/Output segmented control (`relational-workbench.tsx:180` → dock gating) — minimal wiring now; real layout in P4.
3. Delete the dock's read-only SQL `<pre>` duplicate (`query-editor-panel.tsx:1002-1004`).
4. Remove or wire the handler-less Download/Copy buttons (`results-view.tsx:192-201`) — remove now, reintroduce wired in P4.
5. Unused `ui/` primitives: keep `state-panel`, `field`, `combobox`, `kbd` (all adopted by later phases); delete nothing in `ui/` yet.
6. Dead persistence keys: stop writing `dbunk.sidebar.global*`; note `dbunk.workbench.dock.query-{tabId}.*` unbounded growth for P8 migration.

**Acceptance:** app builds and behaves identically (minus the two defect fixes); `pnpm format`, `pnpm lint`, `pnpm typecheck` pass; no import path reaches a deleted file; grep for deleted component names returns nothing.

---

## Phase 1 — Foundation tokens

**Status: implemented** (branch `ui-refresh-p1`). Notes: density is localStorage-backed via `src/lib/density.ts` until P8 moves UI prefs into SQLite; in P1 only the data grid consumes `--row-grid` (P2 wires controls); the `destructive` Button variant name stays (component variant, not a color token) while all app-level `*-destructive` color classes migrated to `danger`; `border-subtle`/`border-strong` names kept with new rgba two-tier values (full two-tier re-audit lands with P2 components).

**Why:** every later phase expresses itself in these tokens; landing them first prevents re-touching files twice.

**Changes** (`src/styles.css` `@theme`, `src/lib/theme.ts`, new token structure)
1. Type ramp per DESIGN-SYSTEM §2.1 as `--text-*` theme tokens; add lint/grep gate banning arbitrary `text-[...]` values (17 arbitrary sizes → 6 tokens; sub-11px sizes migrate up).
2. Spacing/radius rationalization: allowed spacing steps; radius tokens 4/6/8; ban `rounded-xl`+ in app surfaces.
3. Color ramp per §2.4: rebuild dark default preset (`bg-app/sidebar/panel/elevated/input`, two borders, three text steps, overlays, semantics); retire the `destructive` alias in favor of `danger`; map the shadcn compat layer on top; keep preset architecture.
4. Density system: `data-density` attribute + CSS variable blocks (§2.3 metric table); a store-backed setting (default "default"); **delete** width-based `data-density`/`data-workspace-density` auto-triggers (`app-shell.tsx:139`, `query-editor-panel.tsx:829`, `table-editor/body.tsx:156`) and the old compact CSS block (`styles.css:582-641`).
5. Fonts: reorder `--font-mono` to JetBrains Mono first; Monaco reads the token (remove hardcoded stack at `query-editor-panel.tsx:805`).
6. Scrollbars: remove global suppression (`styles.css:531-548`); implement §4.13 overlay scrollbar styling.
7. Motion: global rule set per §2.7 (kill any panel/layout transitions).

**Acceptance:** app renders on the new ramp (visual diffs expected and reviewed); zero `text-[` arbitrary values in `src/`; zero `rounded-xl|2xl|3xl` in app surfaces; scrollbars visible on grid/tree/editor; density setting switches the metric variables live; JetBrains Mono renders in editor and grid on macOS.

---

## Phase 2 — Primitive components

**Status: implemented** (branch `ui-refresh-p2`). Notes: a standard (non-alert) `Dialog` primitive was added (`ui/dialog.tsx`) with the shared sm/md/lg/xl size scale; the three connection dialogs and the schema-map glossary moved onto it. `Kbd` gained a `bare` variant that powers the `DropdownMenuShortcut`/`ContextMenuShortcut` `keys` slots. State primitives now match §4.11 (loading = 2px `LoadingBar`, empty = one muted line, error = left-border banner with expandable details) and were adopted at the divergent sites in the query/table-editor/overview surfaces. The Toaster follows the app theme by observing the root `.dark` class; TanStackDevtools is dev-only and moved out of the toast corner. The `title=` attribute is eliminated from `ui/` (tooltips + aria-labels instead); remaining app-surface `title=` usages are removed as P3–P6/P9 rebuild those surfaces, and the CI grep gate lands with P10.

**Changes** (`src/components/ui/`)
1. Control alignment: Button default = `--control-h`; Input and SelectTrigger identical height/bg/border — the 32/28/24px mismatch dies. Variants per §4.1; icon sizing owned by the button (callsite `size-3.5` overrides removed).
2. Dialogs: size prop (sm/md/lg/xl) on `AlertDialogContent`; delete the 3 copy-pasted class-string overrides + divergent safety-dialog variant; add a standard (non-alert) Dialog for forms/wizards.
3. Menus: dropdown + context menu get kbd-hint slot, destructive item style, submenu support; **the hand-rolled header listbox and all raw-`<button>` menus are rebuilt on these in their phases** — primitive readiness here.
4. Tooltip: single app-level provider, ~400ms delay, kbd integration; ban `title=` attribute (lint grep).
5. State primitives: finalize `state-panel` (empty/loading/error/skeleton per §4.11) and adopt it at the 10+ divergent sites (5 loading idioms, 4 empty-state reimplementations, 2 error vocabularies).
6. Toasts: follow app theme (fix hardcoded `theme="system"`, `__root.tsx:55`); move TanStackDevtools out of the toast corner / dev-only.
7. `Kbd` per §4.12; `Segmented` restyled; `Badge`, `StatusDot` on new tokens.

**Acceptance:** a toolbar mock with Button+Input+Select shows one shared height at all 3 densities; all dialogs via size prop (grep: no `max-w-[26rem]` strings); state primitives are the only loading/empty/error implementations in files touched so far; every icon-button has tooltip + aria-label in `ui/`.

---

## Phase 3 — Panel system & application shell

**Status: implemented** (branch `ui-refresh-p2`). Notes: `Panel`/`usePanelState` (ui/panel.tsx) + `SplitPane` (ui/split-pane.tsx) landed on the full sash spec (`Sash` in resizer-handle.tsx; the old `ResizerHandle` export is gone), with pointer/keyboard tests covering snap-close, same-drag reopen, auto-fit, Alt+double-click, and Enter/Home/End. Navigator, row-details, query-details, and pub/sub panels are `Panel` instances (`dbunk.panel.*`/`dbunk.workbench.navigator` localStorage keys until P8); `ResponsiveEdgePanel`, `use-resizable-width` (`useContainerWidth` moved to `src/lib/use-container-width.ts`), and `use-pubsub-sidebar` are deleted. Pressure-yield is wired for the navigator via `useLayoutPressure`; right-panel pressure wiring extends in P4–P6 as those surfaces restructure. The dock got a resizable persisted height on the sash spec but keeps its P0 output-only content (P4 replaces it). The rail includes Connections (renders the existing ConnectionsView; P9 rebuilds it) and Overview (placeholder empty state until P9). Results-pane-as-SplitPane ships in P4 per that phase's scope. Tab strip: Cmd+W/Cmd+T/Ctrl+Tab/Cmd+1..9 bindings arrive with P7's registry; `Cmd+B` navigator toggle is wired now. `use-window-viewport-zoom` was evaluated and kept — it animates double-click maximize, which `tauri-plugin-window-state` (added, with 900×560 minimums) does not replace. The header connection switcher now single-click selects and auto-connects disconnected targets (menus have no double-click affordance).

**Why:** the panel primitive is the redesign's behavioral core; the shell is its first consumer.

**Changes**
1. **New `Panel` + `SplitPane` primitives** implementing DESIGN-SYSTEM §3.2–3.6 and the sash spec §3.4 (extend `resizer-handle.tsx`): min/default/max, snap-close, double-click auto-fit, Alt+double-click collapse, keyboard resize + Enter-collapse, pressure-yield vs user-collapse distinction, persistence hooks. Retire `ResponsiveEdgePanel`/`use-resizable-width`/`use-pubsub-sidebar` divergence — every panel becomes an instance (navigator, right panels, results, dock).
2. Activity rail → 40px, 18px icons, items per D4 (Connections, Tables, Queries, History, Schema Map, Admin, Overview, Settings-bottom); built on real button primitives.
3. Header: rebuild on `--h-header`; connection switcher on DropdownMenu (replaces hand-rolled listbox `workbench-header.tsx:84-166`); env tag + read-only badge slots (wired in P6); traffic-light gutter collapses to one constant/utility.
4. Status bar per §3.1: 24px, 12px text, defined segments (pending badge and dock badge wired in P4/P6).
5. Object tab strip per §4.4: height token, overflow scroll + chevron, middle-click close, drag reorder, dirty dot, context menu, roving tabindex/ARIA (fixes `object-tab-row.tsx` all-tabs-tabbable).
6. Window: `minWidth/minHeight` 900×560 in `tauri.conf.json`; add `tauri-plugin-window-state`; evaluate and (if redundant) remove the bespoke `use-window-viewport-zoom` scale system.

**Acceptance:** navigator + one right panel + results + dock all resize/collapse per invariant checklist §12.1–2; sash behaviors verified by pointer tests; layout persists across relaunch (widths, collapsed, window geometry); tab strip passes keyboard/ARIA review; no hand-rolled popup remains in the shell.

---

## Phase 4 — Query surface (editor · results · dock)

**Status: implemented** (branch `ui-refresh-p2`). Notes: the editor/results split is a `SplitPane` (default 60:40, editor ≥120px, `dbunk.workbench.query-split`; collapse flag in `dbunk.workbench.query-results.collapsed` until P8) with the §5.3 status strip as the collapsed form — `Cmd+J` toggles, any run restores the pane. The results pane owns the Results/Explain toggle (moved out of the object tab row), result-set chips, pin-result snapshots (in-memory per tab until P8 persistence), and wired Export/Copy format menus (CSV/JSON/TSV/INSERT/Markdown). The old "Output" view is gone: its console content moved to the dock; the budget-owner release UI stayed as a results-pane banner. The dock was rebuilt as the global console (`GlobalConsoleDock` in `WorkbenchShell`, so both workbenches get it) on a new store slice — events capped at 500, status-bar badge for unread, never auto-opens, `` Ctrl+` `` toggle, severity filter, follow toggle, persisted resizable height (`dbunk.panel.dock`). Wired streams: connection connect/disconnect/failure, server notices + session-lost from the query-session channel, the cross-tab query log from `runQuery`, and results-pane export tasks (table-editor import/export progress joins in P6). Execution UX: Run over a multi-statement selection opens a statement picker with "Run all"; `Cmd+Shift+Enter` run-all keybinding added; `Cmd+.` cancels in place (cancel is now fail-safe when the session is already gone). Live elapsed timer renders in the results toolbar and the status strip. Run-glyphs, streaming first page, editable-while-running, and the §5.1 Monaco config already existed from P0–P3. Old per-tab dock keys (`dbunk.workbench.dock.*`) are no longer written; P8 GCs them.

**Why:** the single largest density loss today (results fixed at 160px, ≤4 rows).

**Changes**
1. Replace the workbench dock-results arrangement with the editor/results `SplitPane` (default 60:40, persisted; editor ≥120px). The dead-branch split logic (`query-editor-panel.tsx:1094-1125`) is the seed; the old per-tab dock localStorage keys retire (migration in P8).
2. Results pane per §5.2: Results/Explain tabs (per query tab), result-set chips, pinnable results, wired Export/Copy with format menu; collapse → status strip (§5.3); run auto-restores pane; `Cmd+J` toggle.
3. Dock per §5.6: global console (connection lifecycle, server notices, task/export progress, cross-tab query log), `Ctrl+`` toggle, hidden default, status-bar badge, resizable persisted height, severity filter.
4. Execution UX per §5.1: run-glyph per statement, statement picker on multi-statement, Run⇄Cancel in place (`Cmd+.`), live elapsed timer, streaming first page, editor stays editable.
5. Monaco config per §5.1 (font token, lineHeight 20, overlay scrollbars restored).

**Acceptance:** results pane resizable full-range and collapsible to strip; Explain toggle visibly switches; a canceled long query stops server-side; dock badge increments on background notice without opening; zero references to `WorkbenchDock` in the query path; editor/results ratio survives relaunch.

---

## Phase 5 — Data grid

**Status: implemented** (branch `ui-refresh-p2`). Notes: the grid is rebuilt as a virtualized div-based ARIA grid (`data-grid.tsx` + new `grid-cells.tsx`/`grid-model.ts`/`value-inspector.tsx`). Virtualization is a bespoke fixed-height row/column windower in `grid-model.ts` rather than `@tanstack/react-virtual` — fixed `--row-grid` heights make the math trivial, it adds no dependency, and it degrades to a default viewport when measurement is unavailable (first paint, jsdom) instead of rendering nothing; `@tanstack/react-table` stays as the row model (filtering, visibility, checkbox selection), with client pagination deleted (all rows render virtually). Columns: content-derived initial widths (sampled, clamp 60–400), drag-resize, double-click auto-fit (clamp 500), hide, pin-left, and a header context menu (sort asc/desc/clear in browse mode, auto-fit/auto-fit-all, pin, hide, copy name); widths+pins persist per table per connection via the new `gridLayoutKey` prop (`dbunk.grid.layout.*` until P8). Cells per §5.4: 12px mono `tnum`, right-aligned numbers (typed or sampled), faint-italic NULL, `↵` multi-line collapse, 100-char ellipsis preview with the value inspector (`Space`/`Shift+Enter` → Text/JSON/Hex dialog); state tints tokenized (`yellow-500`/`rose-400`/`amber-400`/`indigo-400`/`bg-black` literals killed). Keyboard/selection: focused-cell model with arrows, `Shift`+arrows ranges, `Alt+Up` expanding selection (cell→column→row→grid), `Cmd+A`, Page/Home/End + `Cmd+Home/End`, `Cmd+G` go-to-row, `Cmd+C` TSV (no header) + copy-as CSV/JSON/INSERT/Markdown via the cell context menu; editing is Enter/F2/double-click (single click now only focuses; typing replaces, `Esc` cancels, Enter commits staged, Tab commits+moves); `Cmd+D`/`Delete` route to the table editor's existing duplicate/delete flows over checkbox-selected rows via new `onCloneSelectedRow`/`onDeleteSelectedRows` props. The copy-format list in `grid-model.ts` is the action seed P7's contextual `Cmd+K` shares. Toolbar per §4.5: one `--h-toolbar` non-wrapping row with a measured two-stage `⋯` overflow (Sort/Refresh/Inspect/Expand fold first) and `Esc`-dismissed transient filter/sort/inspection bars. Deferred: `Cmd+Click` discontiguous selection, multi-sort ordinal badges (the browse sort editor already covers multi-sort), and the palette wiring itself (P7); read-only-reason and dirty-original cells keep a native `title` (per-cell Tooltip components are too heavy for a virtualized grid — revisit at the P10 gate).

**Why:** the highest-frequency surface; performance-critical (Core Priority 1).

**Changes** (`data-grid.tsx` largely rebuilt)
1. Virtualization (add `@tanstack/react-virtual`): row + column; fixed `--row-grid` heights (22/26/30 by density).
2. Columns: content-derived initial widths (replace `100/N%` + 150px min), drag-resize, double-click auto-fit (clamp 500px), per-table width persistence, hide/pin-left, header context menu.
3. Cells per §5.4: 12px mono + `tnum`, right-aligned numbers, faint-italic NULL, `↵` multi-line indicator, ellipsis + value inspector (`Space`/`Shift+Enter`), FK affordance not overlapping text; state tints from tokens (kills `yellow-500/rose-400/indigo-400` literals).
4. Keyboard & selection per §5.4: full navigation, expanding selection, `Cmd+C` TSV + copy-as formats, `Cmd+G`, edit enter/exit semantics, `Cmd+D` clone, `Delete` stages deletion.
5. Grid context menu + contextual `Cmd+K` actions (registry shared with P7).
6. Toolbars: `--h-toolbar`, **no wrap**, overflow `⋯` menu (fixes silent double-height toolbars); filter bar as transient second row dismissed by `Esc`.

**Acceptance:** 100k-row result scrolls at 60fps; 200-column table discoverable via overlay scrollbar; column widths persist per table per connection; keyboard matrix passes; copy formats verified; no hardcoded palette colors in grid files; toolbar never wraps at any window width ≥ min.

---

## Phase 6 — Table editor, mutations & safety

**Status: implemented** (branch `ui-refresh-p2`). Notes: all six sub-views (Data/Columns/Keys/Relations/Schema Map/Specialized) are reachable from the expanded segmented control — the hidden sub-view components already existed and are now wired (D13). Native prompts are gone: a promise-based app confirmation service (`src/lib/confirm.ts` + `ConfirmDialogHost` on the AlertDialog primitive, same queue pattern as safety confirmations) replaced all 19 `window.confirm`/`window.prompt` sites, including store-level ones (the store awaits the service rather than the confirmation moving out of the store; destructive confirms render the danger button non-default with the named object emphasized per §6.4); the `confirmDiscardStagedChanges`/`confirmProductionTarget` contracts became async. D11: Import/Copy/Seed/Add-row open as lg/xl dialogs and Bulk Edit moved into the right panel — nothing pushes the grid down; the in-grid staged insertion row itself (Add-row entering rows directly in the grid) is deferred as a follow-up on the P5 grid. Mutation review is one right-panel chrome (`MutationReviewAside`, 420px/42vw, tokenized surface) used by both the query surface and table editor; the status bar gained the clickable "N staged" pending badge on both surfaces and `Cmd+S` opens the stage → preview → apply flow. D17 was largely pre-existing (env + read-only connection fields, header badges, P3's tab-strip env underline); P6 added the ambient environment status-bar segment so prod/staging color shows on every screen. Remaining hardcoded colors in `table-editor/`, `table-structure/`, and `mutation-review/` were tokenized (the P9 sweep item for those trees is done early).

**Changes**
1. Re-expose Relations, per-table Schema Map, Specialized as sub-views (D13); segmented control lists all six, each wired.
2. Inline forms (D11): Import/Copy/Seed → lg/xl dialogs with steps; Add Row → in-grid staged insertion row; Bulk Edit → right mutation panel. Nothing pushes the grid down.
3. Mutation review: single right-panel instance (unify the two widths), Preview SQL one click away, per-change revert, `Cmd+S` apply flow per safety level; status-bar pending badge wired.
4. Connection safety (D17): env tag + read-only flag as connection properties; ambient env color (tab strip underline + status bar segment); read-only disables mutation affordances at source.
5. Replace all 20 `window.confirm/alert/prompt` sites with themed dialogs per §6.4 (incl. store-level call sites — confirmation moves to the UI layer); destructive-dialog spec (named objects, non-default danger button).
6. Pagination footer, status banners, row-details panel conform to primitives/tokens.

**Acceptance:** all six sub-views reachable and rendering; opening any form causes zero grid layout shift; `grep -r "window.confirm\|window.alert\|window.prompt" src/` → 0; a prod-tagged connection shows ambient color on every screen; read-only connection exposes no edit affordance; mutation flow = stage → preview → apply everywhere.

---

## Phase 7 — Keyboard & command layer

**Status: implemented (core)** (branch `ui-refresh-p2`). Notes: `src/lib/shortcuts.ts` is the central registry — static §6.1 definitions (id, kbd tokens, label, group) plus a mounted-surface handler registry (`useShortcutHandler`); palette rows, menu hints, and tooltips read from it (grid context-menu hints, the results-collapse and dock tooltips are registry-driven; no new hardcoded hint strings). Tab management landed in `TabShortcuts` (mounted in the app shell): `Cmd+T`, `Cmd+W` (skips pinned, confirms open transactions), `Cmd+1..9` (9 = last), `Cmd+Shift+[`/`]` (matched on `event.code` so layouts don't break it), `Cmd+,`, and the `Ctrl+Tab` MRU switcher popup (hold-Ctrl cycling, Shift reverses, release commits). Panel/query bindings kept their physical homes (Monaco actions, panel capture listeners, grid keymap) and registered palette-invokable handlers: toggle navigator/results/console, run statement/all, format, cancel, commit-staged, navigator filter focus. The palette was rebuilt on the Dialog primitive (portal + focus trap at 15vh): commands from the registry with kbd hints, `>` prefix restricts to commands, per-item frecency (localStorage `dbunk.palette.frecency`, capped, until P8) with a Recent group on open. Navigator per §5.5: one tab stop with roving tabindex, arrows navigate/expand/collapse (Left jumps to parent), type-ahead jump, Home/End, Enter opens, muted table counts right-aligned, filter shows matches with ancestors open, and the palette's "Filter tables…" command focuses the filter. Deferred to follow-ups: the contextual grid-selection `Cmd+K` mode (the grid context menu already mirrors those actions), the global Esc focus-stack walker as one system (each layer — overlays, cell edit, selection, filter bars — already handles its own Esc per §6.2), palette list virtualization (item cap keeps it fast), and per-item frecency reset.

**Changes**
1. Central shortcut registry (single source for bindings → handlers, palette rows, menu hints, tooltips); platform-aware.
2. Full map per DESIGN-SYSTEM §6.1: tab management (`Cmd+T/W/1..9`, `Ctrl+Tab` MRU switcher popup, `Cmd+Shift+[/]`), panel toggles, `Cmd+,`, `Cmd+.`, Esc focus-stack walker (§6.2).
3. Palette upgrade per §4.10: unified nouns+verbs, `>` prefix, frecency + reset, kbd hints, grouped results, virtualized list, rebuilt on Dialog primitive (portal/focus-trap — currently hand-rolled); contextual selection mode (actions shared with grid context menu).
4. Navigator keyboard completion (§5.5): roving tabindex, type-ahead, filter focus command.

**Acceptance:** every §6.1 binding works and appears in the palette; palette opens < 50ms with hundreds of items; `Ctrl+Tab` shows MRU popup; Esc walk verified against §6.2 order (and never destructive); menus/tooltips show hints from the registry (no hardcoded hint strings).

---

## Phase 8 — Persistence & session restore

**Status: implemented** (branch `ui-refresh-p2`). Notes: the app SQLite gained a `ui_state` table (migration 16) with namespace-validated `ui.v1.*` keys, batch upsert, and key/prefix delete (`load_ui_state`/`save_ui_state`/`delete_ui_state` commands; size-capped values). `src/lib/ui-state.ts` is the single frontend read/write point: in the desktop app it loads the namespace into an in-memory cache before the workbench mounts (the shell's loading gate waits on it) and flushes debounced write/delete batches (plus a best-effort `beforeunload` flush); in a plain browser it passes through to localStorage so dev/tests behave as before. Callers keep their historical `dbunk.*` key names — the module maps them onto `ui.v1.*` at the boundary — and every persistence call site (panel sizes/collapsed, split ratio, results-collapsed, grid layouts, palette frecency, export tasks, last rail view) now routes through it. Session restore: a global `ui.v1.session` blob holds serialized query/table tabs (including hot-exit SQL, which lives on the tab and is captured by the same debounced subscription), tab order/pinning, the active tab, and expanded navigator nodes; `restoreSession()` runs once after connections load, validates every field (corrupt blob → silent defaults), drops tabs whose connection is gone, bumps the tab/label counters past restored ids, and only then does `startSessionPersistence()` begin mirroring changes (so an empty boot can't clobber a stored session). The one-shot migration imports legacy `dbunk.*` localStorage into SQLite, deletes the dead `dbunk.workbench.dock.*`/`dbunk.sidebar.global*` keys, and leaves only the theme/density boot-cache mirrors the pre-paint script reads. Connection deletion prefix-GCs `ui.v1.grid.layout.<id>.`; the session blob self-heals via the existing tab-close cascade. The tab list is one global blob rather than per-connection blobs — tabs interleave across connections, so a global ordered list is the correct restore unit; per-connection state stays per-connection-keyed for GC. Not restored (by design or deferred): KeyValue-workspace tabs (their backing sessions are runtime state), per-tab transient view choices (Results/Explain toggle, table sub-view), and theme/density which stay canonical in `app_settings` + boot-cache per §8.

**Changes**
1. UI-state store in app SQLite per DESIGN-SYSTEM §8 (namespaced, versioned `ui.v1.*`): global layout blob + per-connection content state.
2. Session restore: open tabs/order/pinned/active + per-tab view state per connection; last rail view; expanded tree nodes.
3. Hot-exit: dirty editor text saved continuously (debounced), restored on relaunch.
4. Migration: import existing `dbunk.*` localStorage values once, then remove; delete dead keys; GC per-tab/per-connection state on deletion (fixes unbounded `dock.query-{tabId}` growth).
5. Corrupt-state fallback: any parse failure → defaults, silently.

**Acceptance:** relaunch restores connection tabs, active tab, layout, unsaved SQL exactly; localStorage contains only theme boot-cache keys; deleting a connection removes its persisted state; corrupted store value doesn't block launch (test with garbage data).

---

## Phase 9 — Remaining screens conformance (Redis/KeyValue, Connections, Overview, Settings)

**Status: implemented** (branch `ui-refresh-p2`). Notes: the KeyValue key inspector's fixed `w-110` became a `Panel` instance (`dbunk.panel.key-inspector`, 440px default, resizable/collapsible); the pub/sub sidebar was already a `Panel` since P3. The full color sweep landed: every `bg-white/*`, `bg-black`, `amber-400`/`emerald-400`/`indigo-400`/`rose-400`, and `#0b1014`-family literal across `keyvalue/`, `table-structure-view`, `table-editor-panel`, and `query-editor-panel` is now a token (`hover:bg-surface-row-hover`, `warning`/`success`/`info`/`danger`, surface steps) — the Redis tree is theme-correct by construction; the P10 visual pass confirms light mode + presets. The only remaining `bg-black/60` occurrences are the dialog overlay scrims (two in `ui/` primitives + the specialized-cell-editor scrim that mirrors them) — the P10 grep gate should whitelist that scrim pattern or introduce an `--overlay-scrim` token. The schema-map image-export background uses the `white` keyword (export-only, not themed). Connections gained the first-class rail variant (`<ConnectionsView variant="rail">`): dense list rows always (no card grid), compact chrome, form panel closed until requested; the settings tab keeps the responsive card/list behavior; the connection form already carried env + read-only fields from earlier phases. The Overview rail view (D1) shipped on the salvaged `useDatabaseOverview` hook: health header, database stat rows, the full table catalog with row-count estimates (opens tables), and this connection's recent queries (reopens them) — all `--row-tree` dense list rows per §5.7, no stat cards. Settings already had the density + theme controls (P1/P2); its `p-6` content padding normalized to `p-4`. Raw-element sweep: counts fell from the audit's 126/30 to 93 `<button>` / 26 `<input>`, all now on tokens — the remainder are dense list/tree/grid rows and chips where the Button primitive's chrome is wrong; before enabling the P10 primitive-usage gate, either scope it to form/toolbar controls or add an unstyled row-button primitive.

**Changes**
1. KeyValue workspace: key inspector (fixed `w-110`) and pub/sub sidebar (bespoke hook) → `Panel` instances; CLI/viewers onto tokens (removes `bg-white/*`, `amber-400`, `emerald-400` literals — the tree is dark-only today); raw buttons → primitives.
2. Connections: first-class rail view (list on tree/list primitives, not settings-tab cards); connection form gains env tag + read-only fields (P6 backend).
3. Overview rail view (D1): health, stats, table catalog, recent queries — dense list rows per §5.7, reusing salvaged data hooks.
4. Settings: conform to primitives/tokens; density + theme controls live here; `p-6` largesse normalized.
5. Sweep: remaining raw `<button>`/`<input>` (126/30 at audit), remaining hardcoded colors (`bg-black`, `#080c10`-family, `#0b1014`, …) in `table-structure/`, `mutation-review/` if not already covered.

**Acceptance:** `grep` gates: zero raw hex outside token files; zero `bg-white/`, `bg-black` in components; all panels app-wide are `Panel` instances (grep for retired primitives → 0); Redis section renders correctly in light mode + all presets.

---

## Phase 10 — Theme refinement & verification sweep

**Status: automated portion implemented; manual §12 matrix pending** (branch `ui-refresh-p2`). Notes: permanent CI gates landed as `scripts/check-ui-gates.mjs` (`pnpm run check:ui-gates`, wired into `ci.yml`): no arbitrary `text-[` sizes, no `rounded-xl`+ radii, no native prompts, no Tailwind palette literals / raw hex / `bg-white`/`bg-black` in components (the `bg-black/60` dialog scrim is the one sanctioned exception), and no retired-primitive references — all pass repo-wide today; the last five palette literals (Redis type icons, column-row type text) were tokenized to enable the gate. Two gates from the §3 table stay documented-pending inside the script: the `title=` ban (the virtualized grid deliberately keeps native titles at 100k-cell scale; ~90 toolbar uses remain) and the raw `<button>`/`<select>` ban (93 remaining are dense list/tree/grid rows — needs a scoped rule or an unstyled row-button primitive first). Theme refinement ran as a computational WCAG audit across all 7 preset×mode blocks: `text-primary`/`text-secondary` clear 4.5:1 on every surface in every preset; the one real failure — Dracula's `text-muted` at 2.51:1 on elevated surfaces — was fixed by lightening it to `#8492c4` (3.87–5.18:1) with `text-disabled` lifted to `#5c6390`; remaining sub-3:1 pairs are all `text-disabled`, which WCAG 1.4.3 exempts. Docs: `CONTEXT.md` gained the shell-vocabulary glossary (rail, navigator, object tab strip, results pane/status strip, dock, status bar, sash, panel); `designs/DESIGN.md` already carried the superseded banner; `FOLLOWUPS.md` items resolved by the refresh are ship-checked (live transaction status-bar segment; themed confirm on the connection switcher). **Remaining (needs eyes, not automatable):** the full §12 invariant checklist across 4 presets × dark/light × 3 densities × min/typical/large windows, and any visual polish it surfaces.

**Changes**
1. Refine all four presets + light mode against the new token roles (amber default is reference; Dracula/GitHub/Gruvbox re-derived; contrast-check text steps on all surfaces).
2. Full pass of DESIGN-SYSTEM §12 invariants across: 4 presets × dark/light (where supported) × 3 densities × min-window/typical/large.
3. Add permanent CI gates (below); update `CONTEXT.md` glossary with shell vocabulary (rail, navigator, dock, object tab strip, status strip, sash); mark `designs/DESIGN.md` superseded; close out `designs/FOLLOWUPS.md` items resolved by this refresh.

**Acceptance:** invariant checklist signed off per cell of the matrix; CI gates green; docs updated.

---

## 3. Consistency enforcement (permanent gates)

Added to CI alongside `pnpm format/lint/typecheck` as phases land:

| Gate | Rule |
|---|---|
| No arbitrary text sizes | `text-[` forbidden in `src/**/*.tsx` |
| No raw colors | hex/`rgba(`/Tailwind palette literals forbidden outside token files |
| No native prompts | `window.confirm|alert|prompt` forbidden |
| No `title=` tooltips | `title=` forbidden in TSX (Tooltip primitive only) |
| No banned radii | `rounded-(xl|2xl|3xl|4xl)` forbidden in app surfaces |
| Primitive usage | raw `<button`/`<select` forbidden in `src/components/**` outside `ui/` |
| Retired components | imports of deleted/retired primitives fail |

Plus per-phase acceptance criteria above, and the §12 invariant checklist as the release-gating manual pass.

## 4. Migration considerations

- **Persisted state:** one-shot localStorage → SQLite import (P8); dead keys removed; versioned namespace allows future schema changes.
- **User-visible churn:** each phase ships whole surfaces (per D-record); release notes state what moved (notably: results pane resize, new shortcuts, density setting).
- **Docs:** `DESIGN.md` superseded banner (done alongside this plan); `CONTEXT.md` glossary in P10; ADRs unaffected (0022/0023 table-browse & mutation contracts are honored — this refresh changes their UI, not their contracts).
- **Rust side:** minimal — window config, window-state plugin, possible UI-state store commands (P8), env/read-only connection fields (P6). Rust changes follow `just fmt/lint/test` per repo policy.
