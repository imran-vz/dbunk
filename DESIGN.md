# dbunk design

The single design reference for the native app (`apps/native`). It starts from
the [native redesign mock](plans/mocks/native-redesign/index.html) (Plan 031)
and covers every surface: the workspace shell, documents, tools, and full-window
pages such as credential setup, the connection form and managed servers.

Older documents in `designs/` describe the removed Tauri/React UI and are kept
for history only.

Code is the source of truth for values: tokens live in
[`apps/native/src/style.rs`](apps/native/src/style.rs) and shared components in
[`apps/native/src/ui.rs`](apps/native/src/ui.rs). Views take colours, sizes and
motion from there, never from literals.

## 1. Direction

A compact instrument panel. Graphite surfaces, 11 px type, data in monospace,
and **one loud signal**: the environment colour that frames the window.
Everything else stays quiet so that signal is never missed.

- Density first: chrome exists to be ignored; vertical pixels belong to data.
- Hierarchy comes from colour and weight, not size. Three text steps (`text`,
  `dim`, `faint`) and one weight bump (semibold) carry it.
- One theme everywhere. A page that is not part of the workspace (setup, unlock,
  connection editor, managed servers) uses the same surfaces, type, controls
  and motion as the workspace.
- Predictable: nothing changes shape on its own; motion only confirms what the
  user just did.

## 2. Tokens

### Colour

| Token | Hex | Use |
| --- | --- | --- |
| `bg` | `#0c0d0f` | Window, documents, input wells |
| `panel` | `#111316` | Sidebar, tab bar, status bar, cards, popovers |
| `raised` | `#171a1e` | Secondary buttons, pressed toggles, tooltips |
| `hover` | `#1d2126` | Hover and keyboard focus fill |
| `pressed` | `#0f1114` | Control held down |
| `select` | `#22324a` | Selected row, tab, list item |
| `line` | `#24282e` | Borders and dividers |
| `line_soft` | `#1b1e22` | Inner dividers, grid lines |
| `text` | `#cdd2d9` | Primary text |
| `dim` | `#8a929c` | Secondary text, labels, idle icons |
| `faint` | `#5b626c` | Hints, metadata, disabled |
| `accent` | `#6aa6ff` | Focus ring, selected cell, primary action |
| `ok` / `warn` / `bad` | `#3fb950` / `#d29922` / `#f85149` | Status |

Derived fills (alpha over the surface): `primary_fill`/`primary_line`
(accent at 15 % / 45 %), `bad_fill`/`bad_line`/`bad_text` for errors and
destructive actions, `ok_fill` for success notes, `warn_fill` (warn at 11 %)
for confirmations and policy notes that need a second look.

Data colours in grids: numbers `#79c0ff`, booleans `#d2a8ff`, `NULL` faint
italic, empty strings a faint `''`.

Staged grid edits (Plan 032):

| Token | Value | Use |
| --- | --- | --- |
| `edited_fill` | `warn` at 14 % (`#d2992224`) | Edited cells (with `warn` text) and every cell of an inserted row |
| `deleted_fill` | `bad` at 10 % (`#f851491a`) | Rows staged for deletion, with faint strikethrough text |

A change excluded from review shows the same tint at half alpha. The change
named by the last apply failure gets a 1 px `bad_line` row outline.

Object kinds in sidebar trees (`style::kind_icon`); containers (databases,
schemas, groups) stay `dim`:

| Token | Hex | Use |
| --- | --- | --- |
| `kind_table` | `#6aa6ff` (`accent`) | Tables |
| `kind_view` | `#b392f0` | Views |
| `kind_materialized` | `#d2a8ff` | Materialized views |
| `kind_type` | `#79c0ff` | Types, domains, dictionaries, foreign tables, triggers |
| `kind_routine` | `#56d4dd` | Functions, procedures, aggregates |

Engine badges (14 px, 8 px mono label): PG `#6c9bd2`, MY `#e6a23c`,
CH `#f4d03f`, RD `#e5534b`, SQ `#8bb8a8`.

### Environment signal

| Environment | Colour |
| --- | --- |
| Development | `#3fb950` |
| Test / local | `#8a929c` |
| Staging | `#d29922` |
| Production | `#f85149` |

Shown as **frame + tint**: a 2 px window border and a ~4 % wash in the current
connection's colour, plus the 2 px bar on the active tab and the logo mark.
Production adds a 22 px strip under the tab bar: "Production · writes require
review and confirmation". With no connection the signal is neutral `faint`.

### Type

- UI: system font, 11 px (`FONT`); small text 10 px (`FONT_SMALL`). Page titles
  15 px semibold (`FONT_TITLE`); section headings 10 px uppercase semibold
  `faint`.
- Data: `.ZedMono` (`MONO`) for grid cells, SQL, hosts, latencies, counts.

### Sizes

| Token | px | Use |
| --- | --- | --- |
| `ROW` | 20 | List, tree and grid rows |
| `TOOL` | 20 | Toolbar buttons, icon buttons |
| `TOOLBAR` | 28 | Document toolbars |
| `FOOTER` | 24 | Document footers, segmented rows |
| `STATUS` | 22 | Status bar |
| `BAR` | 34 | Title row and tab bar |
| `SIDEBAR` | 248 | Default sidebar width |
| `ICON` | 11 | Icons |

Form controls are 24 px (buttons, chips) and 26 px (text fields).

Radii: 3 px badges, 4 px tool/icon buttons, 5 px buttons, fields and banners,
6 px cards, 8 px popovers (palette, menus).

## 3. Layout

```
┌ sidebar (248) ─────┬ tab bar (34): tabs · + · Tools ─────────────┐
│ ●●● ▢        dbunk │ [production strip]                          │
│ project ▾  A L D S P│                                             │
│ 🔍 Search   ⌘K   + │ document: toolbar (28)                       │
│ DEV                │           content                           │
│  PG billing-dev  ● │           footer (24)                       │
│ ───────────────── │                                             │
│ objects tree       │ [error strip]                               │
├────────────────────┴ status bar (22) ───────────────────────────┤
```

- **Sidebar**: traffic lights, hide-sidebar button and logo on one row;
  project switcher and environment chips; connection search; connections
  grouped by environment; below, the object tree for the selected connection.
- **Tab bar**: one row of tabs for open documents. Each tab has a kind icon,
  name and its own close button (shown on hover, always on the active tab).
  New query (`+`) and the Tools menu sit at the right. When the sidebar is
  hidden, the tab bar's left inset holds the show-sidebar button clear of the
  traffic lights.
- **Status bar**: environment tag, connection state, database, last query
  latency, host. Collapses to a 4 px environment-coloured line (`⌘J`).
- **Pages** (forms, setup, managed servers) cover the window. A 34 px titlebar
  strip keeps the window draggable; dismissable pages put a close button at
  its right. Content is one centred column (380 px unlock, 420 px small
  dialogs, 520 px credential setup, 640–680 px editors) with a header, sections
  and a footer of actions. Overlays block pointer input to the workspace below.

## 4. Components (`ui.rs`)

- **`tool_button`**: 20 px toolbar button, transparent until hovered;
  `primary` gets the raised fill. `segmented`/`segment` for in-document tab
  rows.
- **`button(variant)`**: 24 px page button.
  - Primary: accent fill and border; the one action a page exists for (Save,
    Unlock, Continue). At most one per page.
  - Secondary: raised fill, line border, 1 px drop shadow.
  - Ghost: no fill until hovered; Cancel, Back, quiet links.
  - Danger: red wash; destructive confirmation only.
  - Disabled: 55 % opacity, not focusable, AX disabled.
- **Chips** (`forms/render.rs::chip`): radio choices (engine, environment,
  TLS mode). Selected chips take the accent fill; environment chips carry their
  colour dot.
- **Checkbox rows**: 12 px box plus subject; AX toggled. Use for on/off
  settings instead of "X: on/off" buttons.
- **`choice_card`**: icon tile, title, optional badge ("Recommended"), body.
  For decisions that need explanation (credential storage).
- **Fields**: `labelled(label, input_frame(error), note)`. Label above in
  10 px `dim`; 26 px bordered well on `bg`; hint in `faint` or error in
  `bad_text` underneath. Errors also turn the border `bad_line`.
- **`section(title)`**: uppercase 10 px heading over a soft divider.
- **`error_banner`**: warning icon, red wash, AX alert.
- **Messages** have a tone: error (banner, shakes), success (`ok`, check
  icon), info (`dim`, info icon). Live region polite.
- **Tooltip**: raised panel, line border, 11 px text; shows after 450 ms.
  Every icon-only button has one, including its shortcut where there is one.
- **`toolbar_strip`**: the table toolbar. Fixed 28 px, never wraps, clips;
  secondary actions live in its `⋯` overflow menu. `toolbar` (wrapping) stays
  for other documents.
- **`icon_button`**: 20 × 20 icon-only button. Its label is the AX name and
  the hover tooltip.
- **`tool_button_accent`**: the single primary toolbar action (`Review 3 ⌘S`):
  primary fill, line and text, optional trailing shortcut.
- **`count_badge`**: 14 px mono pill in `primary_fill` beside Filter, Sort or
  Columns.
- **`segment_group`**: 20 px bordered row of `segment`s (Data | Structure).
- **`tri_check_box(CheckState)`**: 12 px box, empty, dash (mixed) or check.
  Row checkboxes and the select-all header; the owning control reports AX
  toggled (`CheckState::toggled`).
- **Popovers** (`ui::popover`; palette, menus, selects): `panel`, 8 px
  radius, line border, large shadow, 4 px vertical padding. They paint in a
  deferred layer anchored under their trigger (`probe` records its bounds),
  stay 8 px inside the window and close on a click outside, which still goes
  through. Menu rows are 22 px, at least 170 px wide, with an optional icon
  and a trailing faint mono hint (shortcut or SQL badge); `check_item` rows
  report AX toggled. Keyboard: up/down wrap, home/end, enter or space
  activate, escape closes (`MenuNav`).
- **Select** (`popover::select_trigger`): 20 px field-styled button showing
  the value and a chevron; accent border while open; AX button with expanded
  state. Its list is a popover menu.
- **Dialogs** (`ui::dialog`): in-document modals over an occluding 45 % black
  backdrop. Panel as popovers, AX modal dialog, at most 80 % of the document
  height; 34 px header (title, optional environment chip), scrolling body,
  40 px footer with actions right-aligned. Write reviews open with
  `env_notice`: a 2 px bar in the environment colour beside the policy text,
  with the `warn_fill` wash when typed confirmation is required.
- **Typed confirm** (`ui::confirm::TypedConfirm`): a labelled field, "Type
  confirm to apply". Apply unlocks only on the exact lower-case word
  (surrounding whitespace ignored); Enter submits once it matches. Created
  empty for every review and never reused.
- **`badge`, `crumbs`, `status_line`, `shortcut`, `separator`** as in the
  mock.

## 5. Motion

Minimal and fast. Motion confirms an action or draws attention to an error;
it never decorates. No sliding pages, no scaling, no overshoot. GPUI skips all
of it when macOS "Reduce motion" is on.

| What | How | Token |
| --- | --- | --- |
| Press | Face darkens to `pressed` with a 1 px inset shadow while held; layout never moves | `ui::press` |
| Page / document / popover entrance | Opacity 0→1 and a 4 px rise, ease-out, once per page id | `APPEAR_MS` 160, `APPEAR_RISE` 4 |
| Error | Three damped horizontal swings of at most 3 px, replayed for each new or repeated error | `SHAKE_MS` 320, `SHAKE_PX` 3 |
| Tooltip | Opacity only | `TOOLTIP_MS` 120 |
| Sidebar hide/show | Width follows a critically damped spring (no bounce); content is clipped, not reflowed; the tab bar inset springs in step | `SIDEBAR_SPRING` (420, 41, 1), ~0.3 s |

Rules: animate opacity, small offsets and widths only. Key entrance animations
by the page or document id so switching replays them; key shakes by an error
sequence so a repeated identical error still moves.

## 6. Feedback and validation

- Every click gives immediate feedback (press depth), then a result: a closed
  page, a success note, or an error that shakes.
- Forms validate inline. Errors appear after the first Save or Test and then
  follow typing live. The summary names the single problem, or "Fix the N
  highlighted fields". The backend still validates everything.
- Busy pages show "Working…" in the footer and make fields read-only.
- A blocked action always says why, in the error strip, instead of silently
  doing nothing.

## 7. Credential storage

Same three modes as before the native migration:

1. **Encrypted SQLite** (recommended, default): passwords encrypted with an app
   password entered once per session. Setting it needs the password twice and
   an acknowledgement that it cannot be recovered.
2. **OS keychain**: macOS keychain, no app password.
3. **Unencrypted SQLite**: plain text; needs an acknowledgement.

Only Encrypted SQLite takes a password; the backend refuses a password sent
with another mode. Setup, unlock and recovery are full-window gates that
cannot be dismissed until storage is ready. Unlock offers "Forgot password?",
which leads to a red confirmation before credential storage is reset.

## 8. Keyboard and accessibility

- Every control is reachable by Tab, shows a focus state (accent border or
  hover fill) and has an AX role and label; toggles report their state.
- Escape dismisses a dismissable page; gates ignore it.
- Shortcuts: `⌘K` palette, `⌘O` open table, `⌘P` switch project (Projects
  menu), `⌘⇧C` focus connection search, `⌘T` new query, `⌘W` close tab, `⌘\`
  sidebar, `⌘J` status bar, `⌘,` credentials.
