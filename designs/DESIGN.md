# dbunk — UI Shell Design System ("Workbench Rail")

Status: **adopted**. This is the single source of truth for the app shell and
layout. It replaces the earlier exploratory handoff that used to live in this
file (persistent sidebar + stacked tab rows). Color/typography tokens live in
[`../src/styles.css`](../src/styles.css); this doc maps the shell to those
tokens so it can be reproduced without hardcoding hex values.

A live, clickable reference implementation is at the `/mock` route
(`../src/components/mock/designs/workbench-rail.tsx`). The mock is throwaway and
hardcodes colors for exploration; **production must use the CSS-variable tokens
described in §4**, not the mock's literals.

> **Primary accent: amber `#f5a623`.** dbunk's brand/primary accent is amber.
> The previous lime-green accent is retired. All "active / primary / selected /
> healthy-affirmative" states use amber.

---

## 1. The one rule that drives this design

**There is never more than one row of tabs on screen.**

The previous shell stacked tab systems: global workspace tabs, then a per-view
tab row (overview sub-tabs / table sub-tabs / results tabs) directly beneath. It
was hard to tell "where am I" and it was eye-straining.

Workbench Rail collapses that:

- **One** horizontal tab row: open objects (tables / queries).
- **Sections inside an object** (Data / Columns / Keys, etc.) are a **segmented
  control on the right end of that same tab row** — not a second row.
- **Top-level navigation** (Tables / Queries / Schema map / History / Admin)
  moves to a vertical **activity rail** on the far left.

If a new view needs sub-views, they go into the segmented control or a dropdown.
Adding a second tab row is a design regression — don't.

---

## 2. Layout architecture

```text
┌──┬───────────────────────────────────────────────────────────────┐
│  │ HEADER: connection switcher ……………………………… host · status dot     │ 40px
│  ├───────────┬───────────────────────────────────────────────────┤
│ R│ NAVIGATOR │ OBJECT TAB ROW          ……… [ Data | Columns | Keys ]│ 36px
│ A│ (tree)    ├───────────────────────────────────────────────────┤
│ I│           │ META STRIP  schema.table · rows · size · comment    │ 28px
│ L│ schemas   ├───────────────────────────────────────────────────┤
│  │   tables  │                                                     │
│48│           │ CONTENT  (data grid / columns / indexes)            │ flex
│px│           │                                                     │
│  │           ├───────────────────────────────────────────────────┤
│  │           │ DOCK TABS  [SQL Console] [Output]            ▾      │ 28px
│  │           │ DOCK BODY  (collapsible)                            │ 160px
│  ├───────────┴───────────────────────────────────────────────────┤
│  │ STATUS BAR  ● Connected · rows · size ……………… ⚡12ms · UTF-8     │ 24px
└──┴───────────────────────────────────────────────────────────────┘
```

### 2.1 Regions

| # | Region          | Source design | Size                | Notes |
|---|-----------------|---------------|---------------------|-------|
| 1 | Activity rail   | Command Rail  | `w-12` (48px), full height above status bar | Vertical icon nav + logo top, settings pinned bottom |
| 2 | Header          | Workbench Dock| `h-10` (40px)       | Connection switcher (left), host + health dot (right) |
| 3 | Navigator tree  | Workbench Dock| `w-56` (224px), resizable | Schema → tables tree |
| 4 | Object tab row  | Workbench Dock| `h-9` (36px)        | Open objects (left) + section segmented control (right) |
| 5 | Meta strip      | Workbench Dock| ~28px               | `schema.table`, row count, size, comment |
| 6 | Content         | Workbench Dock| flex-1              | Data grid / columns / indexes |
| 7 | Bottom dock     | Workbench Dock| 28px tabs + 160px body, collapsible | SQL console + output |
| 8 | Status bar      | Command Rail  | `h-6` (24px), **full width** | Spans under rail + content |

### 2.2 Stacking / ownership

- The **activity rail** and **status bar** are the outer frame. The rail is a
  full-height column on the left; the status bar is a full-width row at the very
  bottom. The rail sits _above_ the status bar (status bar wins the corner).
- Everything else (header → dock) is the **workbench column** to the right of
  the rail and above the status bar.
- The header spans only the workbench column (it starts to the right of the
  rail), so the rail reads as the app's primary identity strip.

---

## 3. Component specs

### 3.1 Activity rail (region 1)

- Width `w-12`. Background one step darker than the sidebar
  (`#0c1116` in mock → use `--surface-sidebar` or a hair darker).
- Top: square brand mark (`size-7`, **amber** fill, `--primary-foreground`
  glyph).
- Items: `Tables`, `Queries`, `Schema map`, `History`, `Admin`. Each is a
  `size-9` icon button.
- `Settings` pinned to the bottom (`mt-auto`).
- **Active item**: amber-tinted background (`--accent-subdued` / `amber @ 14%`)
  + a `0.5px × 20px` amber bar flush to the left edge.
- Inactive icon color `--text-disabled`; active `--accent` (amber).
- Every item needs a `title`/tooltip — the rail is icon-only.

### 3.2 Header (region 2)

- `h-10`, `--surface-window`/`--surface-panel`, bottom hairline
  `--border-subtle`.
- **Connection switcher** (left): a button styled like an input
  (`--surface-panel-elevated`, `--border-subtle`), server icon in amber, bold
  connection name, ` · Engine` muted, chevron-down. Opens the connection list.
- Right: host string, muted, preceded by a health `StatusDot`.
- No logo here — the rail owns identity.

### 3.3 Navigator tree (region 3)

- `w-56`, resizable (reuse `ResponsiveEdgePanel` from the real shell).
- Tiny uppercase section header ("Database navigator") with bottom hairline.
- Schema group: chevron + server icon + name. Tables nested one indent
  (`pl-6`), table icon, name truncates.
- Active table: `--accent-subdued` (amber tint) background, amber icon, primary
  text.
- Inactive: `--text-muted` text, `--text-disabled` icon.

### 3.4 Object tab row + section toggle (region 4)

- `h-9`, `--surface-app` background, bottom hairline.
- **Tabs** (left, horizontally scrollable): table/query icon, name, close `×`.
  - Active tab: `--surface-panel` background, **2px amber bottom border**,
    primary text, right hairline between tabs.
  - Inactive: transparent, muted text.
- **Section toggle** (right, `ml-auto`): `Segmented` control
  (Data / Columns / Keys for tables). This is the _only_ place sub-views live.
  - Track: `--surface-panel-elevated` + `--border-subtle`, `rounded-md`, `p-0.5`.
  - Active segment: amber fill, `--accent-foreground` text. Inactive: muted.

### 3.5 Meta strip (region 5)

- One line, `text-[11px]`. `schema.table` (primary, medium) then muted
  `rows`, `size`, italic `comment`. Bottom hairline.

### 3.6 Content (region 6)

- **Data grid** (`MockGrid` reference → real `DataGrid`): sticky header on
  `--surface-panel-elevated`; left `#` gutter; per-column name + `PK`/`FK`
  badges + dim type label; zebra rows via `--surface-row` alternation; hover
  `--surface-row-hover`; selected row = `--accent-subdued` tint + 2px amber
  inset on the left. Monospace cells, `12px`. `NULL` = muted italic.
- **Columns** view: 4-col grid (Column / Type / Null / Default), PK dot in
  amber.
- **Indexes** view: name + definition, `PRIMARY`/`UNIQUE` badges.

### 3.7 Bottom dock (region 7)

- Tab bar (`SQL Console`, `Output`) with a chevron on the right to
  collapse/expand the body.
- Active dock tab: amber-tint pill.
- Body fixed height (~160px) when open; `0` when collapsed. Console shows the
  scratch SQL with a `Run` button (amber); Output lists recent statements with
  timing.
- The dock is **per-workbench**, replacing the old separate results tab row.

### 3.8 Status bar (region 8)

- `h-6`, **full width**, `--surface-panel`, top hairline, `text-[11px]`,
  `--text-muted`.
- Left → right: `● Connected`, row count, size … (spacer) … `⚡ {ms}`,
  encoding, engine + version. The health dot uses amber when affirmative.
- Reuse the real `StatusBar` component (`data-slot="status-bar"`), which already
  has compact-density rules.

---

## 4. Design tokens

Use the CSS variables in `../src/styles.css` (`@theme inline` exposes them as
Tailwind utilities like `bg-surface-panel`, `text-text-muted`,
`border-border-subtle`). **Do not** hardcode hex in shell components.

### 4.1 Surfaces (dark, default preset)

| Token (CSS var)            | Tailwind utility            | Dark value | Role in shell |
|----------------------------|-----------------------------|------------|---------------|
| `--surface-app`            | `bg-surface-app`            | `#080c10`  | Content area, tab row bg |
| `--surface-window`         | `bg-surface-window`         | `#0d1117`  | Header |
| `--surface-sidebar`        | `bg-surface-sidebar`        | `#0a0f15`  | Rail / navigator |
| `--surface-panel`          | `bg-surface-panel`          | `#111820`  | Active tab, dock, status bar |
| `--surface-panel-elevated` | `bg-surface-panel-elevated` | `#151c24`  | Grid header, segmented track |
| `--surface-row`            | `bg-surface-row`            | `#10161d`  | Zebra rows |
| `--surface-row-hover`      | `bg-surface-row-hover`      | `#17202a`  | Row hover |

### 4.2 Borders & text

| Token             | Utility               | Dark value | Use |
|-------------------|-----------------------|------------|-----|
| `--border-subtle` | `border-border-subtle`| `#222b35`  | All hairlines, default borders |
| `--border-strong` | `border-border-strong`| `#303a46`  | Hover/emphasis borders |
| `--text-primary`  | `text-foreground`     | `#e6edf3`  | Names, values |
| `--text-secondary`| `text-text-secondary` | `#b4bdc7`  | Secondary labels |
| `--text-muted`    | `text-text-muted`     | `#7d8793`  | Meta, inactive nav |
| `--text-disabled` | `text-text-disabled`  | `#5f6873`  | Idle icons, NULL |

### 4.3 Accent (amber) & semantic

Amber is the primary accent. **This is live in `../src/styles.css`** — the old
`--accent-green*` family was renamed to `--accent*` across the codebase
(`bg-accent`, `text-accent`, `bg-accent-hover`, `bg-accent-subdued`,
`bg-accent/10`, …) and the values are amber. The shadcn `--accent` /
`--accent-foreground` tokens were merged into this one accent (menu/highlight
chrome now uses the same accent), and `--primary` / `--ring` alias `--accent`.

| Token                 | Utility                  | Dark value | Light value | Use |
|-----------------------|--------------------------|------------|-------------|-----|
| `--accent`            | `bg/text/border-accent`  | `#f5a623`  | `#b45309`   | Active, primary, selected, healthy |
| `--accent-hover`      | `bg/text-accent-hover`   | `#ffb733`  | `#92400e`   | Primary hover; readable accent text |
| `--accent-subdued`    | `bg-accent-subdued`      | `#3a2c10`  | `#fbe7c2`   | Active tints (rail item, active row, selected row) |
| `--accent-foreground` | `text-accent-foreground` | `#1a1205`  | `#ffffff`   | Text/glyph on an amber fill |
| `--accent-overlay`    | `bg-accent-overlay`      | `#3a2c10`  | `#fbe7c2`   | Grid match/selection overlay |
| `--semantic-success`  | `text-success`           | `#8fdd4c`  | `#1a7f37`   | Success (kept green — distinct from amber) |
| `--semantic-warning`  | `text-warning`           | `#f5b84b`  | `#c97a17`   | Warning (visually near amber; pair icon/label) |
| `--semantic-danger`   | `text-danger`            | `#ef6b6b`  | `#d23535`   | Error / destructive |
| `--semantic-info`     | `text-info`              | `#67b7ff`  | `#0969da`   | Info |

Notes:

- **Light amber is darkened** (`#b45309`) so `text-accent` stays AA-legible on
  white and amber fills carry white text. Dark uses vivid `#f5a623` with dark
  (`#1a1205`) text on fills.
- **Dark mode primary fills keep dark foreground.** `--primary-foreground` and
  the sidebar `*-foreground` on-accent tokens are `#1a1205` in dark.

> **Caution — amber vs warning.** Primary amber (`#f5a623`) sits close to the
> warning color. Keep warning states paired with a warning icon + text label so
> they never read as "primary". If collisions feel bad in practice, shift
> warning toward a redder orange on a later pass.

> **Presets keep their own identity.** Dracula → pink (`#ff79c6`), GitHub →
> blue (`#0969da` / `#58a6ff`), Gruvbox → orange (`#af3a03` / `#fe8019`). Each
> preset now owns a single, self-consistent `--accent` family (the old split
> between a brand-green accent and a separate shadcn accent was collapsed).
> Amber applies only to the default light/dark themes.

### 4.4 Typography

```text
Sans   --font-sans  → "SF Pro Text", -apple-system, "Inter", system-ui
Mono   --font-mono  → "SF Mono", "JetBrains Mono Variable", ui-monospace

Header connection name     13px / 600
Nav + tree items           12px / 500
Object tab label           12px / 500
Section segment            11px / 500
Meta strip                 11px / 400 (table id 11px/500)
Grid cell                  12px / 400 mono
Grid column header         12px (name) + 10px (type)
Status bar                 11px / 400
Uppercase section caption  10px / 600, tracking-wider
```

### 4.5 Radius & density

- Radius from `--radius` (0.5rem) scale: controls `rounded-md`, badges
  `rounded`/pill, segmented track `rounded-md`.
- Honor the existing density system: `data-density="compact"` (shell < 900px)
  and `data-workspace-density` already shrink buttons, inputs, grid rows, tabs,
  and the status bar via `../src/styles.css`. New shell pieces must use
  `data-slot` hooks (`top-bar`, `workspace-tabs`, `status-bar`, `button`,
  `input`, `data-grid-*`) so they inherit those rules.

---

## 5. Interaction rules

- **Rail click** switches the navigator's top-level context (Tables / Queries /
  Schema map / History / Admin); the workbench column reflects it.
- **Tree click** opens or focuses an object tab; selecting an already-open
  object just activates its tab.
- **Section segmented control** switches the active object's view in place —
  state is per-tab, never global, never a new row.
- **Tab close** (`×`) removes the object; activate the neighbor.
- **Dock** is collapsible and remembers open/closed; `Run` executes the console
  buffer; `Output` lists history with timing.
- **Status bar** always reflects the active connection + last query, even while
  idle. Never hide it.
- Keep all four context anchors visible at once: connection (header), object
  (tab), section (segment), health (status bar).

---

## 6. Reproduction guide (mapping to real files)

When implementing in the real app (one engine at a time, starting Postgres):

| Shell piece          | New/changed real file (suggested) | Replaces |
|----------------------|-----------------------------------|----------|
| Amber tokens ✅ done | `../src/styles.css` (`:root` + `.dark` accent/`--primary`) | lime-green accent |
| Activity rail        | `../src/components/app-shell/activity-rail.tsx` | top-level nav scattered in header |
| Header switcher      | `../src/components/app-shell/app-shell-header.tsx` | search-only header |
| Navigator tree       | `../src/components/sidebar.tsx` (kept) | — |
| Object tab row       | `../src/components/workspace-tabs.tsx` | hardcoded `#0a0f14` chrome |
| Section toggle       | shared `Segmented` in `../src/components/ui/` | `overview-header`, `table-editor/header`, `results-view` tab rows |
| Meta strip           | `../src/components/table-editor/header.tsx` (slimmed) | title + second tab row |
| Content grid         | `../src/components/data-grid.tsx` (kept) | — |
| Bottom dock          | new `../src/components/workbench/dock.tsx` | `query-editor/results-view` standalone tabs |
| Status bar           | `../src/components/status-bar.tsx` (kept, made full-width) | — |

Implementation order:

1. ✅ **Done.** Re-tokened the accent to **amber** in `styles.css` (default
   light + dark) and renamed `--accent-green*` → `--accent*` codebase-wide.
2. Introduce the **activity rail** + move top-level nav out of the header.
3. Collapse the **double tab rows** into one object tab row + a shared
   `Segmented` section control. (Biggest UX win.)
4. Re-theme **workspace tabs** off hardcoded `#0a0f14` onto `--surface-*`.
5. Add the **bottom dock** and fold query results/explain into it.
6. Make the **status bar** span full width under the rail.

Each step is independently shippable and reversible.

---

## 7. Don'ts

- ❌ Don't add a second tab row anywhere. Use the segmented control or a
  dropdown.
- ❌ Don't hardcode hex in shell components — use the token utilities.
- ❌ Don't reintroduce lime-green as the primary accent; primary is amber.
- ❌ Don't let warning states read as primary — always pair warning with an icon
  + label (warning sits visually close to amber).
- ❌ Don't put the brand logo in both the rail and the header.
- ❌ Don't hide the status bar or the active-connection indicator.
- ❌ Don't break the density `data-slot` hooks; compact mode must keep working.
