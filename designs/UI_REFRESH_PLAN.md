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

**Why:** the shell and screens (P3+) must be rebuilt *from* correct primitives.

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

**Changes**
1. Central shortcut registry (single source for bindings → handlers, palette rows, menu hints, tooltips); platform-aware.
2. Full map per DESIGN-SYSTEM §6.1: tab management (`Cmd+T/W/1..9`, `Ctrl+Tab` MRU switcher popup, `Cmd+Shift+[/]`), panel toggles, `Cmd+,`, `Cmd+.`, Esc focus-stack walker (§6.2).
3. Palette upgrade per §4.10: unified nouns+verbs, `>` prefix, frecency + reset, kbd hints, grouped results, virtualized list, rebuilt on Dialog primitive (portal/focus-trap — currently hand-rolled); contextual selection mode (actions shared with grid context menu).
4. Navigator keyboard completion (§5.5): roving tabindex, type-ahead, filter focus command.

**Acceptance:** every §6.1 binding works and appears in the palette; palette opens < 50ms with hundreds of items; `Ctrl+Tab` shows MRU popup; Esc walk verified against §6.2 order (and never destructive); menus/tooltips show hints from the registry (no hardcoded hint strings).

---

## Phase 8 — Persistence & session restore

**Changes**
1. UI-state store in app SQLite per DESIGN-SYSTEM §8 (namespaced, versioned `ui.v1.*`): global layout blob + per-connection content state.
2. Session restore: open tabs/order/pinned/active + per-tab view state per connection; last rail view; expanded tree nodes.
3. Hot-exit: dirty editor text saved continuously (debounced), restored on relaunch.
4. Migration: import existing `dbunk.*` localStorage values once, then remove; delete dead keys; GC per-tab/per-connection state on deletion (fixes unbounded `dock.query-{tabId}` growth).
5. Corrupt-state fallback: any parse failure → defaults, silently.

**Acceptance:** relaunch restores connection tabs, active tab, layout, unsaved SQL exactly; localStorage contains only theme boot-cache keys; deleting a connection removes its persisted state; corrupted store value doesn't block launch (test with garbage data).

---

## Phase 9 — Remaining screens conformance (Redis/KeyValue, Connections, Overview, Settings)

**Changes**
1. KeyValue workspace: key inspector (fixed `w-110`) and pub/sub sidebar (bespoke hook) → `Panel` instances; CLI/viewers onto tokens (removes `bg-white/*`, `amber-400`, `emerald-400` literals — the tree is dark-only today); raw buttons → primitives.
2. Connections: first-class rail view (list on tree/list primitives, not settings-tab cards); connection form gains env tag + read-only fields (P6 backend).
3. Overview rail view (D1): health, stats, table catalog, recent queries — dense list rows per §5.7, reusing salvaged data hooks.
4. Settings: conform to primitives/tokens; density + theme controls live here; `p-6` largesse normalized.
5. Sweep: remaining raw `<button>`/`<input>` (126/30 at audit), remaining hardcoded colors (`bg-black`, `#080c10`-family, `#0b1014`, …) in `table-structure/`, `mutation-review/` if not already covered.

**Acceptance:** `grep` gates: zero raw hex outside token files; zero `bg-white/`, `bg-black` in components; all panels app-wide are `Panel` instances (grep for retired primitives → 0); Redis section renders correctly in light mode + all presets.

---

## Phase 10 — Theme refinement & verification sweep

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
