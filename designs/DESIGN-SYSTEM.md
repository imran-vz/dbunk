# dbunk Design System — Dense Developer-Tool UI Specification

**Status:** Authoritative. Supersedes `designs/DESIGN.md` (Workbench Rail shell spec) wherever the two conflict.
**Scope:** The complete visual and behavioral system for dbunk. This document is written to be portable: another application could adopt it and reproduce the same design language without access to dbunk itself.
**Design-primary target:** dark mode, default (amber) preset. All values in this document are authored against it; light mode and the other presets (Dracula, GitHub, Gruvbox) are derived through the semantic token layer and verified after.

---

## 1. Principles

1. **Density is respect for the user's screen.** The user is here to see data. Chrome exists to be ignored. Every vertical pixel spent on toolbars, headers, and padding is a row of data the user cannot see.
2. **Dense, not cramped.** Hierarchy comes from **color and weight, never size**. Three text-color steps and one weight bump express everything a size ramp would, at zero pixel cost. Generous *horizontal* padding, tight *vertical* rhythm: scanning is vertical, reading is horizontal.
3. **Predictability over cleverness.** The UI never changes shape on its own. No auto-switching density, no panels that open themselves, no layout animation. State the user set is state the user finds.
4. **Established intuition over novelty.** When VS Code, JetBrains, and TablePlus agree on an interaction, we adopt it verbatim. Muscle memory from other tools must transfer.
5. **Behavior is part of the system.** A component is its looks *and* its keyboard handling, focus behavior, state transitions, and persistence. A visually-correct component with wrong behavior is off-spec.
6. **Everything frequently used is reachable without the mouse**, and every keyboard action is discoverable through the command palette and inline shortcut hints.
7. **Data safety is ambient.** Which environment you are pointed at, whether the connection is read-only, and what mutations are pending must be visible at all times, not discovered at commit time.

---

## 2. Foundations

### 2.1 Typography

Two families, fixed roles:

| Family | Stack | Role |
|---|---|---|
| **UI sans** | `system-ui, -apple-system, "Segoe UI", sans-serif` | All chrome: trees, tabs, buttons, menus, labels, column headers, status bar |
| **Mono** | `"JetBrains Mono Variable", "JetBrains Mono", ui-monospace, Menlo, Consolas, monospace` | SQL editor, grid cell values, IDs/keys, CLI, logs, code snippets, kbd hints |

JetBrains Mono is bundled and **must be first** in the stack — identical rendering across macOS/Windows/Linux is required for a data tool (column alignment, screenshots, docs). The editor (Monaco) must read the same font token, never hardcode its own stack.

**Type ramp** — fixed across all density modes. Exactly six sizes exist; arbitrary sizes are banned.

| Token | Size / Line height | Weight | Use |
|---|---|---|---|
| `text-2xs` | 11px / 16px | 500–600, `+0.04em` tracking when uppercase | Section headers ("TABLES"), badges, kbd hints, micro-meta |
| `text-xs` | 12px / 16px | 400 | Secondary text: status bar, timestamps, column type hints, counts, dock/console |
| `text-sm` | **13px / 18px** | 400 | **Default body**: tree items, tab labels, inputs, buttons, menus, dialog body |
| `text-md` | 14px / 20px | 500 | Dialog and panel titles |
| `text-lg` | 16px / 22px | 600 | Rare: onboarding, empty-state titles on full-screen surfaces |
| `mono-grid` | 12px / 16px (mono) | 400 | Data grid cells |
| `mono-editor` | 13px / 20px (mono) | 400 | SQL editor (Monaco `fontSize: 13, lineHeight: 20`), value inspector |

Rules:
- **Hierarchy inside a panel never uses a size bump.** Use the text-color steps (§2.4) and a single weight bump (400 → 500). `text-md`/`text-lg` are reserved for titles of dialogs and whole-screen surfaces.
- Numeric UI text that column-aligns (row counts, durations) uses `font-feature-settings: "tnum"`.
- No text below 11px, anywhere, ever. (The legacy codebase's 9.6–10.4px sizes are all migrated up to 11 or 12.)
- Line height is set by the row/control for single-line contexts (text centered in a fixed-height row); the ramp's line heights apply to multi-line text.

### 2.2 Spacing

4px base grid. Allowed values: **2, 4, 6, 8, 12, 16, 24**. (`6` is a legitimate dense step: icon-to-label gap, cell padding.)

| Slot | Value |
|---|---|
| Icon ↔ label gap | 6 |
| Cell horizontal padding (grid) | 8 |
| Control horizontal padding | 8 (10 for `control-lg`) |
| Panel edge padding | 8 (Compact 6, Comfortable 12) |
| Toolbar item gap | 6 |
| Section gap inside forms/dialogs | 12 |
| Dialog padding | 16 |
| Tree indent per level | 12 |

Whitespace never delimits regions — separators and background steps do (§2.5). Padding above 16px appears only in dialogs and full-screen empty states (24 max).

### 2.3 Sizing & density

Density is a **user setting** with three modes: **Compact / Default / Comfortable**. Default is the default.

- Density shifts **metrics only** — control heights, row heights, toolbar heights, panel padding. **Typography never changes** with density (shrinking text with spacing is a zoom-out, not a density gain).
- Implementation: `data-density="compact|default|comfortable"` on the root element sets a block of CSS variables. Components consume the variables; they never hardcode heights.
- **There is no width-based automatic density switching.** Constrained windows are handled by panel-yield rules (§3.6), never by silently changing the whole UI's density.

**Metric table** (Compact / Default / Comfortable, px):

| Variable | C | D | Cf | Applies to |
|---|---|---|---|---|
| `--control-h-sm` | 20 | 22 | 24 | Inline chips, in-cell buttons, pagination buttons |
| `--control-h` | 24 | **26** | 30 | Buttons, inputs, selects, toolbar controls — **one height, all aligned** |
| `--control-h-lg` | 28 | 30 | 34 | Dialog primary actions, onboarding |
| `--row-tree` | 22 | 24 | 28 | Navigator/tree rows, list rows, connection rows |
| `--row-grid` | 22 | **26** | 30 | Data grid rows *and* grid header row |
| `--row-menu` | 24 | 26 | 28 | Menu/palette/select items (min-height) |
| `--h-tab` | 28 | 30 | 34 | Object tab strip, results-pane tab strip |
| `--h-toolbar` | 32 | 36 | 40 | Editor toolbar, grid toolbar, filter bar |
| `--h-header` | 34 | 36 | 40 | Window header (plus macOS traffic-light gutter, a constant) |
| `--h-statusbar` | 22 | 24 | 24 | Status bar |
| `--pad-panel` | 6 | 8 | 12 | Panel edge padding |

Fixed, not density-scaled: activity rail width **40px** (18px icons, 28px hit targets), sash metrics (§3.4), scrollbar width, dialog padding.

### 2.4 Color tokens & semantic roles

A small Zed-sized semantic namespace. Components reference **only** semantic tokens; raw hex/palette literals outside the token definition file are a lint failure.

**Surface ramp** — one cool hue (≈ hue 214, low saturation), four lightness steps ~3–5% apart. Chrome is darker; the content/data surface is the lightest, making data focal.

| Token | Dark default value | Role |
|---|---|---|
| `bg-app` | `#0a0d12` | Window base: activity rail, titlebar/header, status bar |
| `bg-sidebar` | `#0e1218` | Navigator, dock, right panels, results tab strip |
| `bg-panel` | `#131820` | Content: editor, grid, main working surfaces |
| `bg-elevated` | `#1a2029` | Overlays only: menus, popovers, dialogs, palette (always paired with shadow) |
| `bg-input` | `#0f141b` | Input/select fields (reads as "recessed") |

**Interaction overlays** (composited over any surface):

| Token | Value | Role |
|---|---|---|
| `hover` | `rgba(255,255,255,0.04)` | Hover on rows, items, icon buttons |
| `active` | `rgba(255,255,255,0.07)` | Pressed / active-but-unselected |
| `selected` | `rgba(245,166,35,0.10)` | Selected row/item (accent-tinted) |
| `selected-muted` | `rgba(255,255,255,0.06)` | Selection in unfocused containers |

**Borders** — exactly two tiers:

| Token | Value | Role |
|---|---|---|
| `border` | `rgba(255,255,255,0.09)` | Major boundaries: panel edges, toolbar bottoms, dialog edges |
| `border-subtle` | `rgba(255,255,255,0.05)` | Row separators, grid lines, minor splits |

**Text** — three steps plus disabled:

| Token | Value | Role |
|---|---|---|
| `text` | `#dde3eb` | Primary |
| `text-muted` | `#9aa5b1` (~70%) | Secondary: meta, timestamps, type hints, inactive tabs |
| `text-faint` | `#626d7a` (~45%) | Tertiary: placeholders, NULL, disabled-looking-but-readable |
| `text-disabled` | `rgba(221,227,235,0.35)` | Disabled controls |

**Accent & semantics:**

| Token | Value | Role |
|---|---|---|
| `accent` | `#f5a623` | Focus rings, selection tint, primary buttons, active indicators, links |
| `accent-hover` | `#ffb83d` | Hover on accent elements |
| `on-accent` | `#1a1205` | Text/icons on accent fills |
| `success` | `#3fb950` | Committed, connected, inserted rows |
| `warning` | `#d29922` | Staging env, pending/modified cells, non-fatal notices |
| `danger` | `#f85149` | Errors, destructive actions, deleted rows, production env |
| `info` | `#539bf5` | Informational notices, FK links |

State-tint rule: semantic backgrounds are the semantic color at **8–12% alpha** over the surface, with the full-strength color for text/icon — e.g. modified cell = `warning` at 10% bg + `warning` text. Never Tailwind palette literals (`yellow-500`, `emerald-400`, …).

**Environment colors** (connection property, §6.4): production → `danger`, staging → `warning`, development → none. Rendered as a 2px underline on the tab strip and a status-bar segment tint — ambient, on every screen of that connection.

Light mode and presets redefine the same tokens; the amber accent is the brand and persists across the default preset's light/dark. A `.export-light` token scope exists for image export.

### 2.5 Borders, radii, shadows, elevation, surfaces

- **Surface hierarchy is background steps + 1px borders. In-layout shadows are banned.** Shadow exists only on `bg-elevated` overlays: menus/popovers `0 4px 16px rgba(0,0,0,0.4)`, dialogs `0 8px 32px rgba(0,0,0,0.5)`.
- Radii: **0** on panels, grid, sidebars, toolbars, tabs-in-strip (anything layout); **4px** controls (buttons, inputs, badges); **6px** menus, popovers, palette; **8px** dialogs. Nothing larger. `rounded-full` only for status dots and circular spinners.
- One separator per boundary: a border **or** a background step **or** a gap — never two of the three.
- Focus ring: `1px` accent border + `1px` offset outline where feasible; a single crisp ring, never a glow.

### 2.6 Icons

- Library: Tabler (stroke icons). Stroke width **1.5**.
- Sizes: **16px** standard (toolbars, menus, buttons, tree items) · **14px** inline/decorator (in-cell affordances, meta rows, status bar) · **18px** activity rail. Nothing else.
- Monochrome, inheriting `text-muted` (or the row's text color); full color only for semantics: connection status, dirty state, errors, env tags. Filled variants only for stateful toggles (pinned, favorite).
- Icon-only buttons require a tooltip (with kbd hint when bound) and an `aria-label`.

### 2.7 Motion

- **Never animate:** panel resize, panel collapse/expand, sidebar toggle, tab switching, density change, data appearing, layout restore on launch (render final state directly), row insertion/removal in lists and grids.
- **May animate (100–150ms, ease-out, opacity/transform only):** command palette open, popover/menu open, toast enter/exit, tooltip fade.
- Continuous indicators: indeterminate 2px progress bar (top of relevant surface), spinner ≥500ms operations only, elapsed-time counters. Never modal progress.
- All animation honors `prefers-reduced-motion`. The theme-switch view-transition wipe is exempt as an explicit user action, and is skipped under reduced motion.
- Interaction-to-response budget: user-caused state changes render **< 50ms**; if work takes longer, show the old state plus a progress indicator — never a blank.

---

## 3. Layout & the application shell

### 3.1 Shell regions

```
┌────┬──────────────────────────────────────────────────────┐
│    │ Header (--h-header): traffic-light gutter (macOS) ·  │
│ R  │ connection switcher · env tag · global actions       │
│ a  ├───────────┬──────────────────────────────────────────┤
│ i  │ Navigator │ Object tab strip (--h-tab)               │
│ l  │ (sidebar, ├──────────────────────────────────────────┤
│    │ resizable,│ Content: editor / grid / view            │
│ 40 │ collaps-  │   … with optional right panel and        │
│ px │ ible)     │   bottom results split                   │
│    │           ├──────────────────────────────────────────┤
│    │           │ Dock (global console — hidden by default)│
├────┴───────────┴──────────────────────────────────────────┤
│ Status bar (--h-statusbar) — full width                   │
└───────────────────────────────────────────────────────────┘
```

- **Activity rail** (40px, fixed): top-level surfaces — Connections, Tables, Queries, History, Schema Map, Admin, Overview; Settings pinned at bottom. Active item: accent 2px left indicator + `selected` bg. The rail is permanent (never collapses) and doubles as the sidebar's restore affordance.
- **Header**: connection switcher (real menu component, not hand-rolled), environment tag, read-only badge when applicable. macOS traffic-light gutter is applied in exactly one place (a single constant/utility).
- **Status bar**: left → connection name · env color segment · read-only badge; center → active-query elapsed time / row count · duration of last result; right → pending-mutations badge, dock badge ("2 notices"), background-task indicator, density/preset quick access. All segments are click targets (e.g. pending badge opens mutation review, dock badge opens dock).
- **One route, state-driven views** is acceptable; the shell contract is regions + behavior, not the router.

### 3.2 Panel taxonomy

Every sidebar and auxiliary panel is an instance of **one** panel primitive with one interaction model. No bespoke panels.

| Panel | Side | Resizable | Collapsible | Collapsed form |
|---|---|---|---|---|
| Navigator (schema tree / query list / key browser) | left | ✅ | ✅ | **0px** — restore via rail icon, `Cmd+B`, or edge-drag |
| Right panels (row details, key inspector, mutation review, pub/sub channels) | right | ✅ | ✅ | **0px** — restore via toolbar toggle button or edge-drag |
| Results pane | bottom of editor split | ✅ | ✅ | **status strip** (see §5.3) |
| Dock (console) | bottom, full width | ✅ (height) | ✅ | **hidden** — restore via `` Ctrl+` `` or status-bar badge |

**Invariant:** a collapsed panel costs 0px unless it carries ambient information worth a strip. Only the results status strip qualifies.

### 3.3 Panel dimensions

All px; all persisted (§8).

| Panel | Min | Default | Max | Snap-close below |
|---|---|---|---|---|
| Navigator | 180 | 260 | 50% of window | 90 |
| Right panels | 280 | 340 | 50% of window | 140 |
| Results pane | 100 | 40% of split | editor keeps ≥ 120 | 60 |
| Dock | 100 | 200 | 60% of content height | 60 |

The **content region is the flex region**: window resizes are absorbed by content; panels hold their pixel sizes until the yield rules (§3.6) trigger.

### 3.4 Sash (resize handle) specification

- Visible: 1px line in `border`. Hit target: **8px** (±4px around the line, extended via pseudo-element; never layout width).
- Cursor: `col-resize` / `row-resize` across the whole hit target.
- Hover highlight: sash brightens to `accent` after **~250ms** hover delay (anti-flicker); instantly while dragging.
- Drag: pointer-captured, live resize (no ghost/deferred preview), clamped to min/max. Dragging below the snap threshold **snaps the panel closed** (collapse, not 0-width limbo); continuing the same drag back past the threshold reopens it.
- **Double-click: auto-fit to content** — sidebar fits the widest visible tree label (clamped to min/max); a grid column sash fits content (clamped ~500px); an editor-split sash equalizes.
- **Alt+double-click: collapse** the panel on the yielding side.
- Keyboard: sash is focusable (`role="separator"`, `aria-orientation`, `aria-valuenow/min/max`), arrow keys resize in 8px steps (32px with Shift), `Enter` toggles collapse, `Home`/`End` jump to min/max.

### 3.5 Split panes

- Editor/results vertical split: default **60:40**, ratio persisted per layout (global). Running a query with the results pane collapsed **auto-restores** it to its last size (the one sanctioned automatic layout change — it is the direct result of the user's run command).
- Split panes use the same sash spec, snap behavior, and keyboard access as panels. No nested-split geometry beyond one vertical (editor/results) + optional right panel + dock; complexity beyond that must be argued for.

### 3.6 Window resizing & constrained space

- Window minimums (enforced at the window level): **900 × 560**.
- Yield order when width shrinks: content shrinks to its minimum (560px protected workspace) → right panel compresses to min, then auto-collapses → navigator compresses to min, then auto-collapses. Panels auto-collapsed by pressure **restore automatically** when space returns (their persisted "user-collapsed" flag is untouched — pressure-collapse and user-collapse are distinct states).
- Height shrink: dock yields first (collapses below usable), then results pane to its strip. Editor keeps ≥120px.
- No reflowing toolbars: toolbars never wrap. Overflow items collapse into a trailing `⋯` overflow menu.

---

## 4. Components

Every interactive element in the app is built from these primitives. Ad-hoc `<button>`/`<input>`/hand-rolled menus are banned in product code.

### 4.1 Button

- Heights: `--control-h` default, `--control-h-sm`, `--control-h-lg`. Radius 4px. Padding-x 8 (10 for lg). Icon 16px, gap 6.
- Variants: **primary** (accent bg, `on-accent` text — at most one per view), **secondary** (transparent, `border`, `text`), **ghost** (transparent, no border — toolbars/icon buttons), **danger** (danger text; danger bg only inside confirmation dialogs).
- States: hover = `hover` overlay (`accent-hover` for primary) · active = `active` overlay · focus-visible = accent ring · disabled = `text-disabled` + no pointer events (tooltip may explain why) · loading = spinner replaces icon, label persists, width locked (no layout shift).
- Icon-only buttons: square at control height, tooltip + kbd hint mandatory.

### 4.2 Input / Textarea

- Height `--control-h`, `bg-input`, 1px `border`, radius 4, 13px text, placeholder `text-faint`.
- Focus: border → accent (no glow). Error: border → danger + 12px danger message below; never color alone.
- Inline validation on blur or submit — never on keystroke.
- Search/filter inputs: leading 14px search icon, `Esc` clears then blurs (two presses), optional trailing match count ("3/41").
- Textarea min-height 2 rows; mono when content is code/SQL/JSON.

### 4.3 Select / Combobox

- Trigger = input metrics exactly (same height, bg, border, radius). Popup = elevated menu (§4.6); typing filters when > ~8 options.
- Full keyboard: open on `Enter`/`Space`/`ArrowDown`, type-ahead, `Esc` closes without change.

### 4.4 Tabs (object tab strip)

- Height `--h-tab`. Tab: 13px label, 16px object-type icon, close ×. Active = `bg-panel` + 2px accent **top** indicator; inactive = `text-muted` on strip bg, `hover` overlay.
- Env-tagged connections add the env color as the strip's bottom 2px underline.
- Overflow: horizontal scroll (natural widths, no shrinking) + trailing chevron listing all tabs. Mouse wheel scrolls the strip.
- **Dirty indicator:** dot replaces the close × until hover (dirty = unsaved SQL or uncommitted grid edits).
- Middle-click closes. Drag to reorder. Pinned tabs: leftmost, icon-width, excluded from `Cmd+W` and "Close Others".
- Context menu: Close · Close Others · Close to the Right · Close All · Pin/Unpin · Copy Name.
- Keyboard: `Cmd+W` close (skips pinned), `Cmd+T` new query tab, `Ctrl+Tab` MRU switcher (hold Ctrl for the popup list), `Cmd+1..9` by position (9 = last), `Cmd+Shift+[`/`]` visual order.
- Sub-view toggles (Data/Columns/Keys/Relations/Schema Map/Specialized; Results/Explain) are **segmented controls** in the tab row region, and every segment must be wired to a visible change — an inert control is a broken invariant.

### 4.5 Toolbar

- Height `--h-toolbar`, `bg-panel`, bottom 1px `border`. Contents: ghost buttons, inputs, selects — all at `--control-h`, vertically centered, 6px gaps, 8px edge padding.
- **Never wraps** (§3.6): overflow collapses into `⋯`. One toolbar per surface; a second row exists only as the transient filter bar, dismissed with `Esc`.
- Text labels on toolbar buttons only for the primary verb (Run, Commit); everything else icon + tooltip.

### 4.6 Menus / Context menus

- `bg-elevated`, radius 6, shadow, 4px padding; items `--row-menu` min-height, 13px, radius 4, padding-x 8; icon 16 left, **kbd hint right-aligned in `text-faint` mono 11px**; separators `border-subtle`; destructive items in `danger` text, placed last, separated.
- Keyboard: full arrow navigation, type-ahead, `Esc` closes, focus returns to the invoker. Submenus open on hover (150ms intent delay) or `ArrowRight`.
- **Right-click works everywhere it plausibly should:** grid cells/rows/headers, tabs, tree nodes, connections, editor. Context menu contents mirror the contextual palette (§4.10) — same actions, two surfaces.

### 4.7 Popover / Tooltip

- Popover: elevated surface, focus moves in, `Esc`/outside-click closes, focus restores.
- Tooltip: 12px on `bg-elevated`, 4px radius, ~400ms show delay, instant reshow while any tooltip is live (shared provider), never blocks pointer, includes kbd hint when the control is bound. Tooltips are the only `title=`-replacement; the raw `title` attribute is banned.

### 4.8 Dialogs

- Modal, centered, `bg-elevated`, radius 8, shadow, overlay `rgba(0,0,0,0.6)`. Sizes: **sm 384 / md 448 / lg 560 / xl 720px** via a size prop — never per-callsite class strings.
- Structure: 14px/500 title · body (12px section gap) · right-aligned footer (primary rightmost).
- `Esc` cancels (equals Cancel exactly); backdrop-click cancels only when the dialog holds no user input. Focus trapped; initial focus on the first field or the least-destructive action; restore focus on close.
- `Enter` submits single-field dialogs; `Cmd+Enter` submits multi-field ones.
- **Native `window.confirm`/`alert`/`prompt` are banned.** All confirmation flows use themed dialogs (destructive spec in §6.4).
- Wizards (Import, Copy Table, Seed) are `lg`/`xl` dialogs with step indicators. **Nothing inserts itself into a content surface's vertical flow** — forms overlay (dialog/popover/right panel), never push the grid down.

### 4.9 Toasts

- Bottom-right, `bg-elevated`, radius 6, 13px, max 3 stacked, 5s auto-dismiss (errors persist until dismissed), action button optional ("Undo", "Open dock"). Theme follows the **app** theme setting, not the OS.
- Toasts are for **completed/failed async outcomes** the user isn't looking at. Never for validation (inline), never for confirmations of visible results (the grid updating *is* the confirmation).

### 4.10 Command palette

- `Cmd+K`, single unified surface: overlay at 15vh, elevated, radius 8, 560px wide, input + virtualized result list.
- Bare query matches **everything**: commands, tables/views, saved queries, connections — grouped, recent/frecency-first. `>` prefix restricts to commands. Fuzzy subsequence matching with word-boundary bonus.
- Rows: 16px type icon · label · muted context ("public · users") · right-aligned kbd hint. `Enter` runs, `Cmd+Enter` where a secondary action exists (e.g. open table in new tab).
- **Contextual palette:** with a grid selection, `Cmd+K` opens scoped to selection actions (Copy as INSERT/JSON/CSV, Set NULL, Delete row (staged), View row, Filter by value…), identical to the right-click menu. First row shows the selection ("3 rows").
- The palette is the discoverability layer: every command in the app is registered in it, with its shortcut. Frecency ranking, with per-item reset.

### 4.11 State primitives (empty / loading / error / skeleton / partial)

One shared set; local reimplementations are banned.

- **Empty:** one `text-muted` 13px line + optional one action button, centered. No illustrations, no 20px titles, no icon circles. ("No rows. — Run a query to see results here.")
- **Loading:** initial load of a region → 2px indeterminate accent bar at the region's top edge, content area keeps the previous content (dimmed to 60% + `aria-busy`) if any, else stays empty. Spinner only for operations ≥500ms with no region (button-level). Skeletons only for *structured* first paints (settings panes, overview cards) — never for the data grid (the grid shows its header + progress bar).
- **Error:** inline banner at the failed region's top — danger 10% bg, danger left 2px border, 13px message, monospace details expandable, Retry action where retry is meaningful. Full-region replacement only when nothing else is showable.
- **Partial data:** rendered rows stay; the fetch boundary shows a one-line strip ("Loaded 500 of ~12,400 · Load more"). Streaming results render as they arrive; the toolbar shows a live elapsed timer.
- All states use one token vocabulary: `danger` (the `destructive` alias is retired).

### 4.12 Kbd

11px mono on `bg-input`, 1px `border`, radius 4, 2px 5px padding. Used in tooltips, menu items, palette rows, empty states. Platform-aware (⌘ on macOS, Ctrl elsewhere).

### 4.13 Scrollbars

- Thin overlay scrollbars on all scroll surfaces (grid, tree, editor, results, dock, menus): **8px**, thumb `rgba(255,255,255,0.15)` (0.25 on hover), no track, no buttons, radius 4.
- Auto-hide: visible while scrolling and while the pointer is over the scroll surface's edge zone; fade out 300ms after.
- Scrollbars are never fully suppressed on a data surface — invisible overflow is a correctness bug. Horizontal overflow in the grid must always be discoverable.

---

## 5. Core surfaces

### 5.1 SQL editor

- Monaco: `fontSize 13, lineHeight 20`, font token (JetBrains Mono), minimap off, `scrollBeyondLastLine: false`, padding 8 top/bottom, line numbers min 3 chars, subtle current-line highlight allowed, overlay scrollbars per §4.13 (not hidden).
- Run affordances: `Cmd+Enter` run statement at caret (multi-statement selection → statement picker with "Run all"), `Cmd+Shift+Enter` run all, run-glyph in the gutter per statement. `Cmd+Shift+F` format. While running: Run button becomes **Cancel** in place (`Cmd+.` also cancels); the editor stays fully editable.
- The editor never appears twice: no read-only mirrors of its content anywhere.

### 5.2 Results pane

- Bottom half of the editor split (§3.5). Tab strip (`--h-tab`): **Results / Explain** — per query tab, tied to the executed statement; multiple result sets appear as chips. Toolbar end: Export and Copy (wired, with format submenu: CSV / JSON / TSV / INSERT / Markdown).
- Explain renders in the same pane; the Results/Explain segmented control in the tab row switches it — visibly, always.
- Pinnable result tabs: a pinned result is not overwritten by the next run (DataGrip pattern).

### 5.3 Results status strip (collapsed form)

- Height `--h-statusbar`, always present under the editor when results are collapsed: `1,204 rows · 128 ms · LIMIT 500` + expand chevron. Click anywhere on it (or `Cmd+J`) to restore the pane to its previous size.

### 5.4 Data grid (query results & table browser — one component)

**Rendering**
- Virtualized (row + column virtualization; only visible cells in the DOM). Fixed row height `--row-grid`; cells never wrap.
- Cells: `mono-grid` 12px, padding-x 8, top/bottom centered. Numbers right-aligned with `tnum`; booleans centered; text left. Header: 13px sans column name + 11px `text-muted` type hint, `bg-sidebar`, sticky, height `--row-grid`, sortable.
- Row separators: 1px `border-subtle`. **No zebra striping** (separators *or* stripes, never both; we choose separators).
- `NULL`: italic `text-faint` `NULL` keyword. Empty string renders as nothing but is distinguishable from NULL by the absence of the keyword. Multi-line values collapse to one line with a `↵` indicator.
- Truncation: in-cell ellipsis; full value via the value inspector (`Space` or `Shift+Enter` on a cell → popover/panel with pretty-printed JSON, text, hex).
- Cell state tints: modified = warning 10%, inserted = success 8%, deleted = danger 8% + strikethrough, selected row = `selected`, focused cell = 1px accent inset border. FK cells: `info` underline on hover; `Cmd+Click` or the drill-down action follows.

**Columns**
- Drag header divider to resize; **double-click divider auto-fits** to visible content (clamp 500px); widths persisted per table (per connection). Default widths from sampled content, min 60px, max 400px initial.
- Header context menu: sort asc/desc/clear, hide column, auto-fit, auto-fit all, pin left, copy column name.
- Sort: click header cycles asc → desc → none; `Cmd+Click` multi-sort with ordinal badges. In the table browser, sorting is **server-side** (re-issues with ORDER BY); in an ad-hoc result set it sorts the fetched set and labels itself "client sort".

**Keyboard & selection**
- Arrow keys move the focused cell; `Shift+Arrows` range; `Cmd+A` all; click gutter = row; click header = column; `Cmd+Click` discontiguous; `Alt+Up` expanding selection (cell → column → row → grid).
- `PageUp/Down`, `Home/End` (row ends), `Cmd+Home/End` (grid corners), `Cmd+G` go-to-row.
- `Cmd+C` copies selection as TSV; copy-as submenu (context menu / palette) for CSV, JSON, INSERT, Markdown.
- Editing: `Enter`/`F2`/double-click edits (preserving content); typing replaces; `Esc` cancels the cell; `Enter` commits **locally (staged)**; `Tab` commits + moves right. Editing never talks to the database directly (§6.4).
- `Cmd+D` clone row (staged); `Delete` stages row deletion when full rows are selected.

**Data volume**
- Server-side pagination, default page 500 rows, page size selectable; first page streams in while the query still runs. Footer strip: range, total (or estimate), page controls, load-more.

### 5.5 Navigator (tree)

- Rows `--row-tree`, 13px, 16px icons, 12px indent per level, twistie + type icon + name + muted count/meta right-aligned.
- Filter input pinned at top (`Cmd+Shift+F` in-panel or via palette); filtering shows matches with their ancestor chain.
- Full keyboard: arrows navigate/collapse/expand, `Enter` opens (`Cmd+Enter` new tab), type-ahead jump, `Home/End`. Roving tabindex; the tree is one tab stop.
- Context menu per node type (table: Open, Open in new tab, Copy name, Row count, Drop… etc.).
- Expanded-node state persisted per connection.

### 5.6 Dock (global console)

- Full-width bottom surface, hidden by default. Toggle `` Ctrl+` `` or status-bar badge. Height resizable (§3.3), persisted globally.
- Content: app-wide streams — connection lifecycle events, server notices/warnings, background task & export progress, cross-tab query log. 12px, mono for payloads, timestamped, filterable by severity, auto-scroll with follow-toggle.
- **Never auto-opens.** New content while hidden increments the status-bar badge.

### 5.7 Overview (per-connection landing)

- A rail view: connection health, database stats, table catalog (dense list rows, not cards), recent queries. Composed entirely from system list/table primitives — no stat-card grids with 24px padding.

---

## 6. Interaction system

### 6.1 Keyboard map (macOS shown; Ctrl replaces Cmd elsewhere)

| Binding | Action |
|---|---|
| `Cmd+K` | Command palette (contextual on grid selection) |
| `Cmd+B` | Toggle navigator |
| `Cmd+J` | Toggle results pane |
| `` Ctrl+` `` | Toggle dock |
| `Cmd+Enter` | Run statement at caret / (in grid) — reserved for future submit |
| `Cmd+Shift+Enter` | Run all |
| `Cmd+.` | Cancel running query |
| `Cmd+Shift+F` | Format SQL |
| `Cmd+S` | Commit staged mutations (opens review/preview per safety level) |
| `Cmd+T` | New query tab |
| `Cmd+W` | Close tab (skips pinned) |
| `Ctrl+Tab` | MRU tab switcher |
| `Cmd+1..9` | Tab by position (9 = last) |
| `Cmd+Shift+[` / `]` | Previous / next tab (visual order) |
| `Cmd+,` | Settings |
| `Cmd+G` | Go to row (grid focused) |
| `Space` / `Shift+Enter` | Value inspector on focused cell |
| `Esc` | Walk the focus stack (below) |

Every binding is registered in the palette and shown in menus/tooltips. Shortcuts are global to the window unless noted; text inputs swallow only what they must.

### 6.2 Focus model

- One focus owner at all times; visible focus ring on keyboard focus only (`:focus-visible`).
- **Esc discipline** — walks up, one level per press, never destructive: overlay (palette/menu/dialog-cancel) → cell edit (cancel cell) → clear selection → filter bar (close) → focus editor. `Esc` never closes a tab, never discards staged mutations, never disconnects.
- Focus regions (navigator, editor, results, dock) are each a single tab stop with internal roving focus.
- Every hand-rolled interactive pattern (listbox, tabs, tree, grid) carries the matching ARIA role, state attributes, and the keyboard behavior that role promises. If the full pattern isn't implementable, use the primitive that already has it.

### 6.3 Selection model

- Selection ≠ focus ≠ hover: three distinct visual treatments (§2.4).
- Selection in an unfocused container renders `selected-muted`.
- Selection is preserved across data refreshes when row identity allows (keyed by PK).

### 6.4 Mutations, destructive actions & safety

- **All grid edits are staged locally.** Modified/inserted/deleted rows tint per §5.4; the status bar shows a pending count; the mutation review panel (right panel primitive) lists changes with **Preview SQL** always one click away. `Cmd+S` applies per the connection's safety level (strict = the preview dialog *is* the apply path). Revert per-change and revert-all available.
- **Connection properties:** environment tag (dev/staging/prod → ambient color §2.4) and **read-only flag** (mutations disabled at the source: no edit affordances, no staging, tooltip explains).
- DDL / destructive commands (DROP, TRUNCATE, delete connection): modal dialog naming the exact objects, consequence stated plainly, destructive button in danger style and **not the default** — plain `Enter` never fires it; `Cmd+Enter` or explicit click required. Type-to-confirm only for irreversible bulk destruction on production-tagged connections.
- Closing a dirty tab: themed dialog — Save/Commit · Discard · Cancel. Hot-exit (§8) makes this rare for SQL text.

---

## 7. Component state matrix

Every interactive component defines all applicable states with tokens — none may be visually undefined:

| State | Treatment |
|---|---|
| hover | `hover` overlay; reveal-on-hover secondary affordances (row actions, close ×) |
| focus (keyboard) | accent ring, §2.5 |
| active/pressed | `active` overlay |
| selected | `selected` tint (+ indicator edge where applicable) |
| disabled | `text-disabled`, no hover response, cursor default, tooltip may explain |
| loading | in-place indicator, geometry locked (no layout shift), `aria-busy` |
| error | danger border/text + adjacent message (never color alone) |
| success (transient) | success tint ≤2s or toast — no persistent green |
| warning/pending | warning tint 10% + warning text |
| destructive | danger text; danger fill only in confirmation contexts |

---

## 8. Persistence

**Rule: persist state, not settings.** Layout memory is invisible infrastructure — silently saved, silently restored, never animated on restore. No "default sidebar width" setting exists.

| Scope | State |
|---|---|
| **Global** (per app) | Window geometry (via window-state plugin) · theme mode + preset · density · panel layout: navigator width + collapsed, right-panel widths + collapsed, results split ratio + collapsed, dock height + visibility · palette frecency data |
| **Per connection** | Open tabs + order + pinned + active tab · per-tab view state (sub-tab, results view, page, filters, sorts) · per-table column widths & visibility · expanded tree nodes · unsaved editor text (**hot-exit**: dirty SQL survives relaunch) · last-selected rail view |

- Storage: the app's SQLite store (structured, cleanable), not scattered localStorage keys. Keys are namespaced and versioned (`ui.v1.*`); orphaned per-tab state is garbage-collected when tabs/connections are deleted.
- User-collapse vs pressure-collapse (§3.6) are stored separately; only user intent persists.
- Corrupt/absent persisted state falls back to defaults silently — persistence must never break launch.

---

## 9. Content & phrasing

- **Sentence case everywhere** (buttons, titles, menus, labels). No Title Case, no ALL CAPS except 11px section headers.
- Buttons are verbs ("Run", "Commit 3 changes", "Delete table"); destructive buttons name the object, never bare "Delete".
- Menu items that open a dialog end with "…".
- Counts and durations: `1,204 rows · 128 ms` — thousands separators, `·` separators, units spaced.
- Errors: what failed + why + what to do. Show the database's real error message in mono — never paraphrase away the actionable detail. No blame, no "Oops".
- Empty states: one statement + one action, ≤ 2 lines.
- Timestamps: relative under 24h ("3m ago"), absolute after (`2026-08-23 14:02`); tooltips always show the absolute form.
- Never repeat a label the context already provides (no "Name:" prefix on the name field in a "Rename table" dialog).
- Placeholders show format examples ("localhost:5432"), not restated labels.

---

## 10. Composition & reuse rules

1. New UI **must** compose the primitives in §4. A new one-off pattern requires adding it to this document first.
2. One implementation per pattern. Two components disagreeing on the same behavior (two panel widths for one panel, two error-token vocabularies) is a defect regardless of appearance.
3. Tokens only: raw hex/rgba/palette literals outside the token file fail lint. Sizes only from §2.2/§2.3 slots.
4. Primitives own their variants: dialog sizes via prop, not caller class strings; button icon sizes come from the button, not the callsite.
5. Behavior ships with the component: a menu without keyboard navigation, a panel without its sash contract, or a segmented control without a wired effect is incomplete, not "MVP".
6. Delete unused components or adopt them — a mandated-but-unimported primitive is a lie in the codebase.

---

## 11. Anti-patterns (banned even when they look good)

- Cards, nested elevated surfaces, or shadows inside the layout — panels are flat regions split by hairlines.
- Radius > 8px anywhere; rounded "pill" controls.
- Size-based hierarchy inside panels (no 18px headings in a sidebar).
- Illustrated/oversized empty states; icon-in-circle decorations.
- Zebra stripes *and* row borders together; double separators of any kind.
- Wrapping toolbars; wrapping grid cells; multi-line tree rows.
- Auto-opening panels, auto-switching density, layout animation, animated list reordering.
- `window.confirm/alert/prompt`; browser `title` tooltips.
- Always-visible per-row action buttons (reveal on hover/focus instead).
- Badges/pills for plain metadata (muted text instead).
- Toolbar buttons with labels *and* icons for secondary actions.
- Placeholder text as the only label of a form field.
- Hidden-by-hover functionality for anything high-frequency (hover-reveal is for *secondary* affordances only).
- Text below 11px; arbitrary one-off sizes; hardcoded colors.
- Non-functional controls in shipped UI (a visible button always has a handler).
- Blocking modal progress; spinners for < 500ms operations.

---

## 12. Interaction invariants (verification checklist)

1. Every panel resizes via the same sash spec; double-click auto-fits; Alt+double-click collapses; drag-below-threshold snaps closed; keyboard resizing works.
2. Every collapsed panel has ≥2 restore paths (shortcut + click affordance) and costs 0px (results strip excepted).
3. Every control on a toolbar is exactly `--control-h` tall and vertically centered — no per-callsite height overrides.
4. Every command is in the palette; every palette command with a binding shows it; every menu item with a binding shows it.
5. Every async operation ≥500ms shows progress and (if a query) is cancellable in place.
6. Every destructive action is a themed dialog, names its object, and is never triggered by plain Enter.
7. Every mutation path goes through staging → preview-SQL → apply.
8. Every screen renders correctly in all 4 presets × light/dark (where the preset supports it) × 3 densities.
9. No layout shift on: loading→loaded, hover, focus, toggling any panel.
10. Esc never destroys anything; window relaunch restores the exact prior session.
11. Grid handles 100k loaded rows at 60fps scroll (virtualization) and 200-column tables with discoverable horizontal overflow.
12. The app is fully operable at 900×560.
