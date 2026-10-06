# Plan 032: Drizzle-style PostgreSQL table grid

**Status:** IN PROGRESS. Specs: `plans/evidence/032-drizzle-studio-spec.md` (observed Drizzle Studio behavior), `plans/evidence/032-table-tab-inventory.md` (pre-change inventory). Paths are relative to `/Users/imran/projects/Code/dbunk`. Line numbers are from commit `543595c`.

## 0. Corrections to the inventory, and decisions

I checked the inventory against the code. These findings change the design:

| # | Finding | Effect on the plan |
|---|---|---|
| V1 | The production strip does not follow the sidebar selection. `workspace_shell.rs:602-614` (`current_connection`) uses the active document's `connection_id` first. It only falls back to the sidebar when the document has no connection, and an engine surface overrides both. | No `workspace_shell.rs` change is needed. The table tab gets its own policy through `DocumentView::set_connection_metadata`. |
| V2 | The protocol cannot send an already-confirmed apply in one step. `Backend::apply_review` always calls `apply_review_inner(.., false)` (`backend/src/backend/data/mutations.rs`). Only a `MutationConfirmation` created from a `PolicyNeedsConfirmation` refusal sends `confirmed: true`. | The UI sends the confirmation automatically after the user's single Confirm or typed `confirm`. The backend API does not change, so the invariant "only a policy refusal can create this acknowledgement" holds. |
| V3 | `set_connection_metadata` already runs on load (`workspace.rs:519`), on open (`:648`) and on rebind (`:1705`). Its body (`document_view.rs:796`) only forwards to `Content::Admin`. | Add one `Content::Table` branch. `workspace.rs` is not touched. |
| V4 | Inside the grid, an embedded editor would have its keys stolen. Grid bindings for `space`, `shift-enter`, arrows, `shift-arrows`, `cmd-a`, `shift-space` and `cmd-g` only use `!ValueInspector` (`main.rs:201-262`). GPUI evaluates `!X` against the whole context stack (`keymap/context.rs:291`), and `Editor` has no binding for `space`. | Package P2 adds `&& !Editor` to every `ResultGrid` binding. |
| V5 | New connections default to `SafeMode::Protected` (`forms.rs:132`), not `Inherit`. After the policy change, every native-created development connection will need a confirm round trip. For the user this is still one click, a plain Confirm. | **Decided:** keep the `Protected` default. The user still sees one plain Confirm click; the backend confirm round trip is automatic. |
| V6 | `TableChanges::new_query` is only used by tests (`table_changes/retention/tests.rs:148`). | The bottom review panel can be replaced by overlays. Query mode stays compilable and tested but loses its render path, which nothing renders today. |
| V7 | A staged change records `CapturedRow { identity, originals }` (`data_model/mutations.rs:9`), and `MutationOp::Update.guards` always includes the old value of each edited column. | The per-change diff can be built from the reviewed `MutationPlan` alone. The grid overlay can be matched by identity without a fresh analysis, so staged tints don't flicker while a new page is being analysed. |
| V8 | GPUI at rev `506beb3` provides the needed pieces. `on_drag` and `on_drag_move::<T>` fire for every move during a drag of type `T` (`div.rs:360`). `on_drop`, `on_mouse_up_out`, `deferred` and `anchored` exist, `gpui::Empty` implements `Render`, and `MouseDownEvent.click_count` is available. Zed's `ui/components/redistributable_columns.rs:680-725` is a working reference for a resize handle. | Use these primitives for column resize and popovers. |

**Decisions**

- Tokens stay as they are. Rows are `ROW` (20), toolbars `TOOLBAR` (28), header and footer `FOOTER` (24), controls `TOOL` (20). Nothing takes Drizzle's 32 px.
- Staged edits stay visible across paging, sorting and filtering, because the overlay matches rows by identity. Unlike Drizzle, Filter, Sort and Columns are not hidden while there are staged edits.
- Limit and Offset become **Limit** (page size) and **Page** (jump), because the browse contract is page-based (`PageAction::Jump`, `data_model/browse.rs:27`).
- Popovers, selects and menus use `deferred(anchored())` and close on `on_mouse_down_out`, which is Zed's standard behaviour: the outside click still goes through. Modal dialogs use an occluding backdrop.

## 1. Target layout

The area below the tab bar, from top to bottom (`[]` marks a conditional element):

```
┌ toolbar 28 ───────────────────────────────────────────────────────────────────────────────────┐
│ [Data|Structure] │ ⏷Filter 2  ⇅Sort 1  👁Columns 8/10 │ +Row  [🗑 Delete 2 rows]  ··grow··       │
│ [3 changes ▾] [Discard] [Review 3 ⌘S]  [Read-only]  12 ms  ‹ 1–100 of ~18,204 ›  ⟳  ⋯          │
├ filter bar (only while open) 26/row, at most 6 rows then scroll ──────────────────────────────┤
│ × where [status ▾] [= ▾] [open      ]                                                         │
│ × and   [total  ▾] [> ▾] [100       ]   + Add filter · Apply · Clear · SQL                    │
├ notice strip 24 (only when there is a notice) ────────────────────────────────────────────────┤
│ ⚠ Outcome unknown: refresh, then Mark resolved   [Refresh] [Mark resolved]                    │
├ grid ─────────────────────────────────────────────────────────────────────────────────────────┤
│ ☐ #  │ id int4 ↑1 ┆ │ name text ⇅ ┆ │ …   header 24; 6 px resize handle at each right edge      │
│ + ×  │ DEFAULT     │ New name      │     insert band: at most 5 rows × 20, scrolls             │
│ ☐ 1  │ 1           │ [Ada▌      ]  │     inline editor overlay at the cell                     │
│ ☑ 2• │ 2           │ Bob (warn14%) │     edited cell tint                                      │
├ FK reference / related-row detail panels (unchanged, contextual) ─────────────────────────────┤
├ footer 24 (status_line) ──────────────────────────────────────────────────────────────────────┤
│ status (live, dim) ··grow·· 2 selected · 3 staged (warn) · page 2                             │
└───────────────────────────────────────────────────────────────────────────────────────────────┘
```

Overlays inside the document:

| Overlay | Placement |
|---|---|
| Header menu | Deferred popover under the header cell |
| Sort, Columns, pager, overflow (`⋯`), staged-changes list | Deferred popovers under their trigger |
| Cell context menu | At the pointer |
| Cell popover editor | 480×240, under the cell |
| Review, Discard and Virtual key dialogs | In-document modals |

### Toolbar elements

| Element | Kit component | Rules |
|---|---|---|
| Data / Structure | new `ui::segment_group()` containing `ui::segment("table-data","Data",true,..)` and `ui::segment("table-structure","Structure",false,connected)` | Structure emits the existing `TableEvent::OpenStructure` |
| Filter / Sort / Columns | `ui::tool_button` with icons `filter`, `chevron_up_down`, `eye`, plus a new `ui::count_badge(n)` | `ui::pressed(..)` while the filter bar is open or a popover is open. Columns badge reads `visible/total` and only shows when something is hidden. |
| `+ Row` | `tool_button(.., "icons/plus.svg")` | Disabled with a tooltip giving the reason when read-only or analysis is not ready |
| Delete N rows | `tool_button(.., "icons/trash.svg")` with `text_color(style::bad_text())` | Only when checkbox rows are checked. It **stages** deletes. |
| Staged group | text button "N changes ▾" (warn) opens the change-list popover, ghost "Discard", and new `ui::tool_button_accent("Review N", "⌘S")` | This accent button is the single primary action on the page |
| Read-only | `ui::badge("Read-only")` with a tooltip giving the reason | Only when read-only |
| Latency | faint mono text | |
| Pager | new `ui::icon_button` for ‹ and ›; range trigger opens the pager popover; total trigger runs `Count` | Total tooltip: "Estimated; click to count exactly" |
| Refresh / Cancel | `icon_button` `rotate_cw`, or `stop` "Cancel" while busy | |
| `⋯` | `icon_button` `ellipsis`, opens the overflow menu | |
| Disconnected | `tool_button(power) "Connect"` replaces the pager and Refresh | |

The toolbar uses a new `ui::toolbar_strip()`: fixed height 28, no wrapping, overflow hidden. `ui::toolbar()` is left unchanged for other documents.

### Where today's buttons move

Every action stays reachable. A test asserts this (§5, P5).

| Action | New home |
|---|---|
| Backup, Restore, Import CSV, Export CSV, Export table, Copy table, Seed table, Count (exact), Virtual key…, Filter history…, Presets… / Save preset…, Inspect query (copy SQL / params), Export rows (grid export), Reload column preferences, Show all columns, Auto-fit all columns | `⋯` overflow |
| Edit cell, Set NULL, Revert cell, Revert row, Copy, Inspect value, Follow foreign key, Duplicate row, Delete row, Bulk edit column (selection spans rows) | Cell context menu |
| Sort ascending / descending / Clear sort, Multi-column sort…, Filter on this column…, Hide, Pin/Unpin, Move left/right, Narrow/Widen (keyboard path for width), Auto-fit | Header menu |
| Structure | Segmented control |
| First / Last, page size | Pager popover |

### Grid visuals

| Element | Spec |
|---|---|
| Header | 24 px tall. Medium-weight name in `dim`, type in faint mono at `FONT_SMALL`. Sort indicator is `arrow_up` or `arrow_down` at 9 px in `accent`, plus a priority digit when there is more than one sort key. A `chevron_up_down` placeholder shows in faint on hover. |
| Resize handle | 6 px wide, absolute at the right edge, `cursor_col_resize`. A 1 px `accent` line shows on hover and while dragging. |
| Edited cell | `style::edited_fill()` (warn at 14 %) and `warn` text |
| Inserted row | Every cell `edited_fill()`. Gutter shows `+` in warn, then a `×` button that removes the draft row. |
| Deleted row | `style::deleted_fill()` (bad at 10 %), faint strikethrough text, gutter `−` in bad |
| Excluded change | The same tints at half alpha |
| Change named in the last apply failure | 1 px `bad_line` row outline |
| Gutter (table mode) | 20 px checkbox using new `ui::tri_check_box`, then the row number. The header gutter is select-all with a mixed state. |
| Empty string | Faint `''`, so it is distinct from `NULL` (faint italic) |

### Dialogs and popovers

| Surface | Spec |
|---|---|
| Review dialog | 640 px wide, at most 80 % of the document height. 34 px header with the title and an environment chip. Scrollable body: diff, then SQL. 40 px footer. |
| Popover panels | 8 px radius, `panel` background, `line` border, `shadow_lg`, 4 px vertical padding |
| Menus | Minimum width 170, rows 22 px |
| Sort popover | 420 × at most 320 |
| Columns popover | 260 × at most 360 |
| Pager popover | 220 wide |

## 2. New kit components (P1, owned files `ui.rs` and `ui/*`)

The kit stays styling-only, like `ui.rs` today: callers own focus, tab order and listeners. The exception is `TypedConfirm`, which is a small entity.

### Additions to `apps/native/src/ui.rs`

```rust
pub mod popover; pub mod dialog; pub mod confirm;
pub fn toolbar_strip() -> Div;                                  // h(TOOLBAR), no wrap, overflow_hidden
pub fn icon_button(id: impl Into<ElementId>, label: impl Into<SharedString> /*AX + tooltip*/,
    icon: &'static str, enabled: bool) -> Stateful<Div>;       // 20×20; .tooltip(tooltip(label)).tooltip_delay
pub fn tool_button_accent(id: impl Into<ElementId>, label: impl Into<SharedString>,
    shortcut: Option<&'static str>, enabled: bool) -> Stateful<Div>; // primary_fill/line/text
pub fn count_badge(text: impl Into<SharedString>) -> Div;      // 14 px mono pill, primary_fill
pub fn segment_group() -> Div;                                  // 20 px bordered row of segments
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum CheckState { Off, Mixed, On }
pub fn tri_check_box(state: CheckState) -> Div;                 // check.svg / dash.svg
```

### `apps/native/src/ui/popover.rs` (new)

```rust
pub type AnchorSlot = Rc<Cell<Option<Bounds<Pixels>>>>;
pub fn anchor_slot() -> AnchorSlot;
/// Absolute size_full canvas that records its bounds each prepaint (accessible_editor.rs:148 pattern).
pub fn probe(slot: AnchorSlot) -> impl IntoElement;
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Placement { Below, BelowEnd, AtPoint }
/// Pure: top-left for `size` near `anchor`, flips above when there is no room, 8 px window margin.
pub fn place(anchor: Bounds<Pixels>, size: Size<Pixels>, window: Size<Pixels>, placement: Placement) -> Point<Pixels>;
/// deferred(anchored().position(..).snap_to_window_with_margin(px(8.)).child(panel)).with_priority(1)
pub fn layer(anchor: Bounds<Pixels>, placement: Placement, panel: impl IntoElement) -> Deferred;
pub fn panel(id: impl Into<ElementId>, role: Role, label: impl Into<SharedString>) -> Stateful<Div>; // occlude()
pub fn item(id: impl Into<ElementId>, label: impl Into<SharedString>, icon: Option<&'static str>,
    hint: Option<SharedString> /*SQL badge or shortcut*/, highlighted: bool, enabled: bool) -> Stateful<Div>; // Role::MenuItem
pub fn check_item(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool,
    highlighted: bool, enabled: bool) -> Stateful<Div>;          // Role::MenuItemCheckBox, aria_toggled
pub fn divider() -> Div;
pub fn heading(label: impl Into<SharedString>) -> Div;
pub fn select_trigger(id: impl Into<ElementId>, label: impl Into<SharedString> /*AX*/,
    value: impl Into<SharedString>, open: bool, enabled: bool) -> Stateful<Div>; // field look + chevron_down; Role::Button + aria_expanded
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MenuNav { pub len: usize, pub highlighted: usize }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum MenuKey { Moved, Activate(usize), Dismiss, Ignored }
impl MenuNav { pub fn new(len: usize) -> Self; pub fn set_len(&mut self, len: usize);
    pub fn key(&mut self, key: &str) -> MenuKey; } // up/down wrap, home/end, enter/space → Activate, escape → Dismiss
```

### `apps/native/src/ui/dialog.rs` (new)

```rust
pub fn backdrop(id: impl Into<ElementId>) -> Stateful<Div>;   // absolute inset_0, occlude, black @45%, centers child
pub fn modal(id: impl Into<ElementId>, title: impl Into<SharedString>, width: f32) -> Stateful<Div>; // Role::Dialog, aria_modal, appear()
pub fn header(title: impl Into<SharedString>, trailing: Option<AnyElement>) -> Div;
pub fn body(id: impl Into<ElementId>) -> Stateful<Div>;       // overflow_y_scroll, flex_col, gap 8, px 12
pub fn footer() -> Div;                                         // 40 px, border_t line, justify_end, gap 6
pub fn env_notice(environment: Option<DevelopmentEnvironment>, text: impl Into<SharedString>, loud: bool) -> Div; // 2 px env bar; loud ⇒ warn_fill
```

### `apps/native/src/ui/confirm.rs` (new)

```rust
pub const CONFIRM_WORD: &str = "confirm";
pub fn typed_confirmation_matches(input: &str) -> bool;        // input.trim() == CONFIRM_WORD (case-sensitive)
pub enum TypedConfirmEvent { Changed(bool), Submit }
pub struct TypedConfirm { /* Entity<Editor> (single_line), Entity<AccessibleEditor>, matched: bool */ }
impl TypedConfirm {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self; // always empty; never reused across reviews
    pub fn matches(&self) -> bool;
    pub fn focus(&self, window: &mut Window, cx: &mut App);
}
impl EventEmitter<TypedConfirmEvent> for TypedConfirm {}
impl Focusable for TypedConfirm {}
impl Render for TypedConfirm {} // labelled("Type confirm to apply", input_frame(..)); Enter → Submit when matched
```

### Additions to `apps/native/src/style.rs`

```rust
pub fn edited_fill() -> Rgba;  // warn @ 0x24 (14 %)
pub fn deleted_fill() -> Rgba; // bad @ 0x1a
```

DESIGN.md §2 and §4 document the new components and tints.

## 3. Architecture

### 3.1 Data flow

```
DocumentView::set_connection_metadata ──► TableView::set_connection_metadata ──► TablePolicy
                                                     │
          ┌─────────────── GridEvent ◄───────────────┤──────────► ChangesEvent ─────────┐
   ResultGrid (render, resize, checks,               ▼                                  │
   inline slot host) ◄── sync_grid() ── TableView (toolbar, popovers, routing) ──► TableChanges
                                                     │                    (draft, edits, review, apply)
                                     BrowseControls (filter bar, sort/history/preset panels)
```

`TableView::sync_grid(cx)` is the only path that pushes changes state into the grid. It runs after every `ChangesEvent` and after page and policy changes. It is cheap and idempotent:

- `grid.set_overlay(rc)` does nothing when `Rc::ptr_eq`.
- `grid.set_inline_editor(slot)`
- `grid.set_table_editing(..)`
- `grid.set_sort_indicators(..)`

### 3.2 Environment, safe mode and read-only reach the tab

- `TableView::set_connection_metadata(&mut self, records: &[DevelopmentConnection], cx)` finds the record whose `id` equals `self.connection`. It computes `TablePolicy::from_connection(record)`, or `TablePolicy::UNKNOWN` when the record is missing or not PostgreSQL. It then calls `changes.set_policy(policy, cx)` and `sync_grid`.
- `document_view.rs:796` gains `Content::Table(view) => view.update(cx, |v, cx| v.set_connection_metadata(connections, cx))`.
- `TablePolicy` mirrors `resolve_policy` (`backend/src/safety/policy.rs:81`) from `DevelopmentPostgresConnection.{environment, safe_mode, read_only}`. It drives the UI only; the backend stays authoritative (ADR-0024).

### 3.3 Write-safety rules (pure, in `data_model/write_safety.rs`)

| Policy | `confirm_style()` | Dialog | Backend expects confirmation | After the user clicks |
|---|---|---|---|---|
| read_only | `None` | Not openable. Editing is disabled with a reason. Discard is still allowed. | — | — |
| environment Production **or** effective Strict | `Typed` | Diff, SQL, `TypedConfirm`. Apply is enabled only when it matches. | yes (Strict); no only for Production with Disabled | Apply; on `NeedsConfirmation`, confirm automatically |
| effective Protected | `Plain` | Diff, SQL, Confirm button | yes (after the P1 backend change) | Apply; on `NeedsConfirmation`, confirm automatically |
| effective Disabled | `Plain` | Diff, SQL, Confirm button | no | Apply → applied |
| `UNKNOWN` (before metadata arrives) | `Typed` | as Typed | assumed yes | as Typed |

There is one escalation rule. If the backend returns `NeedsConfirmation` but the click was not pre-confirmed (the UI believed the policy was Disabled, so its view is stale), the dialog switches to the Typed requirement. It never confirms automatically in that case.

The automatic confirmation reuses the existing machinery:

1. `ApplyFlow::needs_confirmation(token)`
2. `ApplyFlow::confirm(next_id)`
3. emit `ChangesEvent::PersistApply(id)`, which saves the journal again
4. `apply_saved` sends `TableCommand::Confirm`

It is bounded to one confirmation per `PendingApply` (`auto_confirms < 1`).

### 3.4 Draft overlay (staged values painted in the grid)

`MutationDraft::overlay(page, page_key, failed)` builds a `DraftOverlay`:

- It indexes the page rows by identity in a `HashMap<Vec<Option<String>>, usize>`. Identity values are projected by column name; `CtidFallback` uses `page.row_identity`.
- It walks the changes once. For `VirtualKey` and `CtidFallback`, it also requires `originals` to equal the page row, the same rule as `existing_in` at `mutations.rs:274`. Otherwise it leaves the row unmarked.
- It works on an invalidated draft (`analysis == None`), so tints survive the gap between the page arriving and the analysis finishing.
- Staged display text is capped at 2 KiB on a char boundary.

`TableChanges::overlay()` caches the result in an `Rc` keyed by `OverlayKey { draft: (owner, revision), page: Rc::as_ptr as usize }`.

### 3.5 Live drag resize (P2, `grid/resize.rs`)

- **Handle.** In `header_cell`: an absolutely positioned 6 px div with `.on_drag(ColumnResizeDrag { grid: EntityId, display }, |_,_,_,cx| cx.new(|_| gpui::Empty))`. Its `.on_mouse_down(Left)` calls `stop_propagation` so the header click menu does not open. Its `.on_click` with `click_count() >= 2` auto-fits the column (`auto_fit_source`). This mirrors Zed's `redistributable_columns.rs:700-725`.
- **Live move.** The grid root has `.on_drag_move::<ColumnResizeDrag>(..)`, filtered by `drag.grid == entity_id`. On the first move it records `start_x` and `start_width`. Then `GridColumns::set_live_width(display, resize_width(start_width, start_x, x))`, which is clamped to `RESIZE_MIN = 48` and `RESIZE_MAX = 1200`, updates `widths` and `offsets`, and notifies only when the width changes by at least 1 px. The local override lives in `GridColumns.overrides: HashMap<String, f32>`.
- **Persist on mouse-up.** The grid root's `.on_drop::<ColumnResizeDrag>` and `.on_mouse_up_out(Left)` both call the idempotent `finish_resize()`. It emits `GridEvent::Preferences(PreferencePatch::ColumnWidth { name, width })`. If neither fires (the window lost focus), the next `on_mouse_down` on the grid finishes the resize.
- **Authority.** `GridColumns::load(prefs)` clears overrides, so the stored record wins. `table_columns(page)` re-applies overrides, so a new page does not snap widths back. If saving the preference fails, TableView calls `grid.clear_width_overrides()` and the width returns to the stored value.
- **Busy tab.** TableView ignores grid events while busy. P5 therefore keeps `pending_width: Option<PreferencePatch>` (latest wins) and flushes it when the busy state clears, next to `after_analysis`.
- **Query grids** on other engines get the drag too, through `ColumnWidths::set_explicit`. That is local only, with no preferences.

### 3.6 In-cell editing

**Triggers (grid).**

| Input | Emits `GridEvent::EditCell { cell, source, seed }` with |
|---|---|
| `on_mouse_down` with `click_count == 2` | `seed: Keep` |
| `jump_key`: `enter` (no modifiers) or `f2` | `Keep` |
| `jump_key`: `backspace` / `delete` | `Clear` |
| `jump_key`: printable `key_char` without cmd or ctrl | `Replace(char)` |

`space` stays Inspect because its binding runs first. Table mode always emits; `begin_edit` returns the refusal reason, which TableView shows in the footer status.

**Presentation (`TableChanges::begin_edit`).** A pure function `edit_presentation(kind, value, seed)` returns `Popover` when any of these hold, and `Inline` otherwise:

- `cell_value::classify` is Json, Array or Geometry
- the value contains `\n`
- the value is longer than 4 KiB
- the cell is in a bulk edit

**Inline editor.** `CellEditor` (P3, `table_changes/cell_editor.rs`) wraps `Editor::single_line` in mono text inside an accent border. Key context is `"CellEditor"`. Its `capture_key_down` is the same proven pattern as `table_changes/view.rs:106`:

| Key | Result |
|---|---|
| `enter` or `cmd-enter` | `Commit { advance: Stay }` |
| `tab` / `shift-tab` | `Commit { Right }` / `Commit { Left }` |
| `escape` | `Cancel` |
| `alt-enter` | `Expand` (switches to the popover, keeping the draft text) |

All of these are skipped while IME composition is active. `on_mouse_down_out` commits with `Stay`, so clicking away stages the edit.

**Hosting.** The grid renders `InlineEditorSlot.view` in an absolutely positioned layer that is a sibling *after* the `uniform_list`. It sits at `cell_rect(..)` and is clipped to the body. Because it is outside the virtualized rows, the editor is always in the element tree and focus is never lost when its row scrolls out of view.

**Popover editor.** TableChanges renders it through `ui::popover::layer` at the anchor TableView supplies (`grid.cell_bounds(cell, source)` → `changes.set_popover_anchor`). It holds a multiline `Editor` (or the existing `ArrayView` for arrays) and a footer: Set NULL · Format (JSON/array) · Raw literal · Copy EWKT (geometry) · grow · Cancel `Esc` · Save `⌘↵`. Invalid JSON keeps the draft and shows the error, using the existing `cell_value::validate_json` path in `stage()`. Clicking outside does not close the popover; it is modal-like.

**After staging.** TableChanges emits `EditClosed { advance }`. TableView then calls `grid.advance(advance)`, focuses the grid and runs `sync_grid`.

### 3.7 Insert rows, checkboxes and deletes

- **Add row.** `TableChanges::add_row()` calls `draft.stage_insert(0, vec![])`. The row appears in the insert band, with every cell showing `DEFAULT`. Editing a band cell (`CellRef::Insert(id)`) goes through `MutationDraft::set_insert_value(id, column, Option<Option<String>>)`: `None` means DEFAULT and drops the value, `Some(None)` means NULL. Non-writable columns refuse ("Generated column").
- **Remove a draft row.** The band's `×` button emits `GridEvent::RemoveInsert { change }`, which calls `TableChanges::remove_insert`, which calls `draft.remove(id)`.
- **Duplicate row** (context menu). Uses the existing `duplicate_values` from `data_model/mutations/batch.rs` and stages the insert directly; the copy appears in the band.
- **Checkboxes.** `grid/checked.rs` holds `CheckedRows { set: BTreeSet<usize>, anchor }`. Shift-click selects a range, the header box selects all, and the set clears on every `table_page` call.
- **Delete N rows.** `changes.stage_deletes(&grid.checked_rows())` calls the atomic `draft.stage_deletes(..)`. Then `grid.clear_checked()`. The rows show the deleted tint and go through the same review.

### 3.8 Header menu and sort

- A click on a header cell (`on_click`, not mouse-down, so it does not fight drag) emits `HeaderMenu { source, anchor }`. Shift-click keeps the existing `GridEvent::Sort { append: true }` multi-key cycle.
- Header sort uses `browse_controls::header_sort(current, column, HeaderSort)` (P4). Asc or Desc **replaces** the whole sort with that one column, as Drizzle does. Clear removes only that column. The result goes to `TableView::apply_browse`, which already re-queries and clears `exact_count` through `TableDocument::set_query` → `invalidate`. That avoids Drizzle's stale-total bug.
- "Multi-column sort…" calls `browse_controls.open_panel(BrowsePanel::Sort, anchor)`. The panel has two panes: unsorted columns with a search, and the ordered key list with ASC/DESC, NULLS cycle, up/down and ×. "Clear sorting" is included. Every change applies immediately.

## 4. Shared interfaces (exact; code against these)

### 4.1 `data_model` (P1 owns)

Re-exported from `crate::data_model`. The module is gpui-free.

```rust
// data_model/mutations/overlay.rs  (child of mutations ⇒ may read private Change/CapturedRow)
pub const OVERLAY_TEXT_BYTES: usize = 2048;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub enum CellRef { Page(usize), Insert(Uuid) }
#[derive(Clone, Debug, PartialEq, Eq)] pub enum EditSeed { Keep, Replace(String), Clear }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Advance { Stay, Left, Right }
#[derive(Clone, Debug, PartialEq, Eq)] pub struct OverlayValue { pub text: Option<String> /*None = NULL*/, pub truncated: bool }
#[derive(Clone, Debug, PartialEq, Eq)] pub enum RowMark {
    Updated { change: Uuid, included: bool, cells: Vec<(usize /*source*/, OverlayValue)> },
    Deleted { change: Uuid, included: bool },
}
#[derive(Clone, Debug, PartialEq, Eq)] pub enum InsertCell { Default, Value(OverlayValue) }
#[derive(Clone, Debug, PartialEq, Eq)] pub struct InsertRow { pub change: Uuid, pub included: bool, pub cells: Vec<InsertCell> /*len == page.columns.len()*/ }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct OverlayKey { pub draft: Option<(Uuid, u64)>, pub page: usize }
#[derive(Clone, Debug, PartialEq, Eq)] pub struct DraftOverlay { pub key: OverlayKey, pub rows: Vec<Option<RowMark>>, pub inserts: Vec<InsertRow>, pub failed: Option<Uuid> }
impl DraftOverlay {
    pub fn empty(page_rows: usize) -> Self;
    pub fn mark(&self, row: usize) -> Option<&RowMark>;
    pub fn cell(&self, row: usize, source: usize) -> Option<&OverlayValue>;
    pub fn is_empty(&self) -> bool;
}
impl MutationDraft {
    pub fn owner(&self) -> Uuid;
    pub fn revision(&self) -> u64;
    pub fn overlay(&self, page: &BrowseTableResult, page_key: usize, failed: Option<Uuid>) -> DraftOverlay;
    pub fn insert_value(&self, id: Uuid, column: &str) -> Result<Option<Option<&str>>, ModelError>;
    pub fn set_insert_value(&mut self, id: Uuid, column: &str, value: Option<Option<String>>) -> Result<(), ModelError>;
    pub fn stage_deletes(&mut self, table_index: usize, rows: &[(&[Option<String>], Option<&[String]>)], truncated: bool) -> Result<usize, ModelError>; // atomic
}

// data_model/write_safety.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum EffectiveSafeMode { Disabled, Protected, Strict }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ConfirmStyle { Plain, Typed }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct TablePolicy { pub environment: Option<DevelopmentEnvironment>, pub safe_mode: EffectiveSafeMode, pub read_only: bool }
impl TablePolicy {
    pub const UNKNOWN: Self = Self { environment: None, safe_mode: EffectiveSafeMode::Strict, read_only: false };
    pub fn resolve(environment: DevelopmentEnvironment, safe_mode: DevelopmentSafeMode, read_only: bool) -> Self;
    pub fn from_connection(connection: &DevelopmentConnection) -> Self;   // postgres: None ⇒ UNKNOWN
    pub fn confirm_style(&self) -> Option<ConfirmStyle>;
    pub fn expects_backend_confirmation(&self) -> bool;                   // !read_only && safe_mode != Disabled
    pub fn read_only_reason(&self) -> Option<&'static str>;               // "Read-only connection: editing is disabled. Change it in connection settings."
    pub fn describe(&self) -> String;                                     // "Production · Strict safe mode"
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Preconfirmation { Granted, NotGranted }
pub fn preconfirmation(policy: &TablePolicy, typed_matched: bool) -> Preconfirmation;
// Typed && matched ⇒ Granted; Plain && expects_backend_confirmation ⇒ Granted; else NotGranted
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ConfirmationStep { AutoConfirm, AskTyped }
pub fn on_needs_confirmation(pre: Preconfirmation, auto_confirms_sent: u32) -> ConfirmationStep; // Granted && sent == 0 ⇒ AutoConfirm

// data_model/review_diff.rs
pub const DIFF_TEXT_CHARS: usize = 256;
#[derive(Clone, Debug, PartialEq, Eq)] pub struct DiffValue { pub text: Option<String>, pub truncated: bool }
#[derive(Clone, Debug, PartialEq, Eq)] pub struct DiffCell { pub column: String, pub old: Option<DiffValue>, pub new: Option<DiffValue> }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum DiffKind { Update, Insert, Delete }
#[derive(Clone, Debug, PartialEq, Eq)] pub struct DiffChange { pub op_index: usize, pub kind: DiffKind, pub target: String, pub identity: String, pub cells: Vec<DiffCell>, pub omitted_defaults: bool }
pub fn review_diff(plan: &MutationPlan) -> Vec<DiffChange>;
pub fn diff_summary(changes: &[DiffChange]) -> String;                 // "2 updates · 1 insert · 3 deletes"
pub fn format_param(index: usize, param: &DmlParam) -> String;         // "$1 = 'text'" | "$1 = NULL", truncated
pub fn apply_error_message(error: &ResultMutationError) -> String;     // 1-based "change 3"; never includes values
```

### 4.2 Grid (P2 owns `crate::grid`)

```rust
pub enum GridEvent {
    Sort { column: String, append: bool },                          // shift-click (kept)
    Preferences(crate::browse_preferences::PreferencePatch),
    HeaderMenu { source: usize, anchor: Bounds<Pixels> },
    ContextMenu { cell: CellRef, source: usize, position: Point<Pixels> },
    EditCell { cell: CellRef, source: usize, seed: EditSeed },
    RemoveInsert { change: uuid::Uuid },
    CheckedRowsChanged,
    Status(SharedString),                                           // replaces the grid status row in table mode
}
#[derive(Clone, Debug, Default, PartialEq)] pub struct TableEditing { pub editable: bool, pub checkboxes: bool, pub reason: Option<SharedString> }
#[derive(Clone)] pub struct InlineEditorSlot { pub cell: CellRef, pub source: usize, pub view: AnyView }
#[derive(Clone, Debug, PartialEq)] pub struct ColumnEntry { pub source: usize, pub name: String, pub cast_type: String, pub visible: bool, pub pinned: bool }
impl ResultGrid {   // new_table(..) now also sets bare chrome (no grid status/export row)
    pub fn set_table_editing(&mut self, editing: TableEditing, cx: &mut Context<Self>);
    pub fn set_overlay(&mut self, overlay: Rc<DraftOverlay>, cx: &mut Context<Self>);
    pub fn set_sort_indicators(&mut self, sort: Vec<BrowseSortKey>, cx: &mut Context<Self>);
    pub fn set_inline_editor(&mut self, slot: Option<InlineEditorSlot>, cx: &mut Context<Self>);
    pub fn checked_rows(&self) -> Vec<usize>;
    pub fn clear_checked(&mut self, cx: &mut Context<Self>);
    pub fn cell_bounds(&self, cell: CellRef, source: usize) -> Option<Bounds<Pixels>>;
    pub fn header_bounds(&self, source: usize) -> Option<Bounds<Pixels>>;
    pub fn column_entries(&self) -> Vec<ColumnEntry>;
    pub fn column_patch(&self, source: usize, action: ColumnAction) -> Result<PreferencePatch, &'static str>;
    pub fn visibility_patch(&self, source: usize, visible: bool) -> Result<PreferencePatch, &'static str>;
    pub fn auto_fit_source(&mut self, source: usize, cx: &mut Context<Self>);
    pub fn clear_width_overrides(&mut self, cx: &mut Context<Self>);
    pub fn advance(&mut self, advance: Advance, cx: &mut Context<Self>);
    pub fn open_export(&mut self, window: &mut Window, cx: &mut Context<Self>);
}
// browse_preferences.rs (P2): new PreferencePatch variants
ColumnWidth { name: String, width: f32 },        // valid_column_name, finite, 48.0..=1200.0
ColumnVisibility { name: String, visible: bool }, // hide adds to hiddenColumns, show removes
```

### 4.3 TableChanges (P3 owns `crate::table_changes`)

```rust
pub enum ChangesEvent { Changed, PersistApply(u64), Applied, KeyChanged, FocusGrid(Vec<FocusHandle>),
    EditOpened { cell: CellRef, source: usize, popover: bool }, EditClosed { advance: Advance }, OverlayChanged }
pub enum ChangesCommand { Review, Discard, RetryRecovery, Reconcile, CancelPending, OpenVirtualKey, OpenChangeList(Bounds<Pixels>) }
pub enum ChangesNotice { OutcomeUnknown, Unrestored, ReadOnlyWithStaged, Unavailable(SharedString) }
pub struct ChangesSummary { pub staged: usize, pub included: usize, pub updates: usize, pub inserts: usize, pub deletes: usize,
    pub pending: bool, pub editing: bool, pub dialog_open: bool, pub can_review: Result<(), SharedString>,
    pub notice: Option<ChangesNotice>, pub message: SharedString }
impl TableChanges {   // existing pub API retained; duplicate() changes signature
    pub fn set_policy(&mut self, policy: TablePolicy, cx: &mut Context<Self>);
    pub fn policy(&self) -> TablePolicy;
    pub fn can_edit_now(&self) -> Result<(), SharedString>;
    pub fn overlay(&self) -> Rc<DraftOverlay>;
    pub fn inline_editor(&self) -> Option<(CellRef, usize, AnyView)>;
    pub fn set_popover_anchor(&mut self, anchor: Option<Bounds<Pixels>>, cx: &mut Context<Self>);
    pub fn begin_edit(&mut self, cell: CellRef, source: usize, seed: EditSeed, window: &mut Window, cx: &mut Context<Self>) -> Result<(), SharedString>;
    pub fn set_null(&mut self, cell: CellRef, source: usize, cx: &mut Context<Self>) -> Result<(), SharedString>;
    pub fn revert(&mut self, cell: CellRef, source: Option<usize>, cx: &mut Context<Self>) -> Result<(), SharedString>;
    pub fn add_row(&mut self, cx: &mut Context<Self>) -> Result<Uuid, SharedString>;
    pub fn remove_insert(&mut self, change: Uuid, cx: &mut Context<Self>);
    pub fn stage_deletes(&mut self, rows: &[usize], cx: &mut Context<Self>) -> Result<usize, SharedString>;
    pub fn duplicate(&mut self, row: usize, cx: &mut Context<Self>) -> Result<Uuid, SharedString>;
    pub fn command(&mut self, command: ChangesCommand, window: &mut Window, cx: &mut Context<Self>);
    pub fn summary(&self) -> ChangesSummary;
}
// Render for TableChanges = overlay layer only (modals + popover editor + change list); no hitbox when idle.
```

### 4.4 BrowseControls (P4 owns `crate::browse_controls`)

```rust
pub enum BrowseEvent { Apply(BrowseState, bool), Mode(FilterMode), SavePreset(Preset) }   // unchanged
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum BrowsePanel { Sort, History, Presets, Inspect }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum HeaderSort { Asc, Desc, Clear }
pub fn header_sort(current: &[BrowseSortKey], column: &str, choice: HeaderSort) -> Vec<BrowseSortKey>;
impl BrowseControls {  // existing pub API retained
    pub fn set_bar_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>);
    pub fn bar_open(&self) -> bool;
    pub fn add_condition(&mut self, column: Option<&str>, window: &mut Window, cx: &mut Context<Self>);
    pub fn open_panel(&mut self, panel: BrowsePanel, anchor: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>);
    pub fn close_panel(&mut self, cx: &mut Context<Self>);
    pub fn active_filter_count(&self) -> usize;   // applied typed + (raw non-empty as 1)
    pub fn sort_count(&self) -> usize;
}
```

## 5. Work packages

There are five packages. Each runs in its own worktree off `main`. File ownership is strictly disjoint, and no agent builds or runs tests (CLAUDE.md).

| Pkg | Scope | Depends on (types) |
|---|---|---|
| P1 | Foundations: kit, draft overlay, safety and diff models, backend policy, docs | none |
| P2 | Grid | P1 (`data_model` overlay types, `ui::tri_check_box`, `style::*_fill`) |
| P3 | Changes controller and overlays | P1 |
| P4 | Browse bar | P1 (kit) |
| P5 | Table shell | P1–P4 |

### P1: Foundations

**Owns and edits:**

- `apps/native/src/style.rs`
- `apps/native/src/ui.rs`
- new `apps/native/src/ui/popover.rs`
- new `apps/native/src/ui/dialog.rs`
- new `apps/native/src/ui/confirm.rs`
- `apps/native/src/data_model.rs`
- `apps/native/src/data_model/mutations.rs`
- new `apps/native/src/data_model/mutations/overlay.rs` and `overlay/tests.rs`
- new `apps/native/src/data_model/write_safety.rs`
- new `apps/native/src/data_model/review_diff.rs`
- `apps/native/src/data_model_tests.rs`
- `backend/src/safety/policy.rs`
- `backend/src/safety/gate.rs` (tests only)
- `backend/src/result_mutation/mod.rs` (tests only)
- `docs/adr/0024-backend-enforced-production-safety-policy.md`
- `CONTEXT.md`
- `DESIGN.md`

**Steps**

1. Kit and tokens: everything in §2.
2. `mutations.rs`:
   - Add `pub fn owner`, `pub fn revision` and `mod overlay; pub use overlay::*`.
   - Add `stage_deletes`: clone `changes` once, run the `stage_delete` logic for each row against the clone, then `publish` once, so it is all-or-nothing and respects `CHANGE_LIMIT` and `DRAFT_BYTES`.
   - Add `insert_value` and `set_insert_value`. They find the `Insert` change by id; check `editable()` and `writes()`; `None` removes the value; then `publish`.
3. `overlay.rs`: `DraftOverlay` plus `MutationDraft::overlay` as described in §3.4. Use `selection` rules only; it must not require `analysis`.
4. `write_safety.rs` and `review_diff.rs`, exactly as in §4.1.
5. `data_model.rs`: `mod write_safety; mod review_diff;` and re-exports.
6. Backend policy:
   - In `protected_requires_confirmation`, move `WriteIntent::RowMutation | WriteIntent::ApplyMutations { .. }` to `true`.
   - Update `protected_requires_only_destructive_overrides`: remove both intents from the ordinary list, add them to the confirmation list, and rename the test to `protected_requires_overrides_for_destructive_writes_and_row_mutations`.
   - Add a `gate.rs` assertion that `Staging` with `Inherit` and the legacy `RowMutation` refuses with `CONFIRM_TAG`.
   - Add a `result_mutation/mod.rs` test copying the production case at `:1821` with `Environment::Staging` and `SafeMode::Inherit`, expecting `PolicyNeedsConfirmation`.
7. Docs:
   - ADR-0024 amendment: "Plan 032: `protected` also requires a confirmed override for row mutations (result-mutation apply and legacy row writes); `strict` unchanged."
   - Update the `CONTEXT.md` Safe Mode definition.
   - DESIGN.md §2 tints and §4 popover, select, dialog, typed confirm and icon button.

**Tests**

| File | Cases |
|---|---|
| `ui/confirm.rs` | `"confirm"`, `" confirm\n"` pass. `"Confirm"`, `"CONFIRM"`, `"confirm."`, `"con firm"`, `""` fail. |
| `ui/popover.rs` | `MenuNav` wraps and clamps after `set_len` shrinks; `key` mapping. `place` stays inside the 8 px margins, flips above when there is no room below, and honours `BelowEnd` right alignment. |
| overlay | A keyed update paints only its matching row after the page is re-sorted. A ctid update is not painted when page originals differ (reused ctid). Delete marks. Insert rows with Default, NULL and Value. The overlay survives `invalidate()`. Changes on another table are ignored. Text is truncated at 2 KiB on a UTF-8 boundary (multibyte case). An empty draft gives an empty overlay. |
| `set_insert_value` | Default removes the value; NULL is kept distinct from omitted; non-writable column → `Unavailable`; unknown id → `InvalidInput`; refused while applying or outcome-unknown; revision increments. |
| `stage_deletes` | One untruncatable row leaves no deletes staged (atomic); converts an existing update on the same row; the 128 limit is checked atomically. |
| `write_safety` | Full 4×4 environment × safe-mode matrix, written out to equal `inherit_resolution_follows_environment` (`policy.rs:242`). Production with Disabled → Typed and does not expect confirmation. Staging with Inherit → Plain and expects it. Development with Inherit → Plain and does not. Development with Strict → Typed. Read-only → `None` plus a reason. `UNKNOWN` → Typed. `preconfirmation` and `on_needs_confirmation` truth tables, including a stale Disabled policy → `AskTyped` and a second confirmation → `AskTyped`. |
| `review_diff` | Primary-key update gives `old → new` per set column. A ctid update with full-row guards still lists only the set columns. Delete lists guard values as removed. Insert lists values with `omitted_defaults`. NULL rendering. Truncation at 256 chars on a char boundary. Composite identity `"a = 1, b = 'x'"`. `apply_error_message` for every `ResultMutationError` variant, with 1-based op index and no values leaked. |
| Backend | As in step 6. |

**Acceptance:** all new pure functions have tests. No existing public signature changes except the additions. The backend tests encode the new Protected rule.

### P2: Grid

**Owns and edits:**

- `apps/native/src/grid.rs`
- `apps/native/src/grid/render.rs`
- `apps/native/src/grid/keyboard.rs`
- `apps/native/src/grid/frozen.rs`
- new `apps/native/src/grid/resize.rs`
- new `apps/native/src/grid/checked.rs`
- new `apps/native/src/grid/editing.rs`
- `apps/native/src/grid_columns.rs`
- `apps/native/src/grid_columns/pinning_tests.rs`
- `apps/native/src/column_widths.rs`
- `apps/native/src/browse_preferences.rs`
- `apps/native/src/browse_preferences/pinning_tests.rs`
- `apps/native/src/main.rs` (keymap only)

**Steps**

1. `GridColumns`:
   - `overrides`, `set_live_width`, `clear_overrides`.
   - `load` clears overrides; `rebuild` applies overrides last.
   - `width_patch(display)`, `entries()`, `visibility_patch(source, visible)` (refuses hiding the last visible column).
   - `RESIZE_MIN` and `RESIZE_MAX`.
2. `browse_preferences.rs`: `ColumnWidth` and `ColumnVisibility` variants with validation in `apply`.
3. `grid/resize.rs`: `ColumnResizeDrag`, the pure `resize_width`, handle element, root `on_drag_move`, `on_drop`, `on_mouse_up_out` and `finish_resize` (§3.5). Double-click on the handle calls `auto_fit_source`. Refactor `auto_fit` into `auto_fit_range`.
4. `grid/frozen.rs`: pure `cell_rect(geometry: CellGeometry, row_offset_y, display, pinned) -> Bounds<Pixels>`. Used by `cell_bounds`, `header_bounds` and the inline-editor layer.
5. `grid/editing.rs`:
   - `TableEditing`, `InlineEditorSlot`, overlay storage (`Rc<DraftOverlay>`).
   - Pure `cell_tone(selected, mark, included, failed) -> CellTone`.
   - Insert band render: at most 5 rows, `×` emits `RemoveInsert`, double-click emits `EditCell { Insert }`.
   - Inline editor layer.
6. `grid/checked.rs`: `CheckedRows` model plus gutter checkbox render (`ui::tri_check_box`). Emits `CheckedRowsChanged`. Clears in `table_page` and `begin`.
7. `render.rs`:
   - Header 24 px with sort badge, built from `set_sort_indicators` through the pure `sort_badge(sort, column) -> Option<(BrowseSortDirection, Option<usize>)>`.
   - Header `on_click` emits `HeaderMenu`; shift-click emits `Sort { append }`. An `on_a11y_action(Click)` on the header opens the menu, which gives keyboard and VoiceOver access.
   - Cells: overlay tint and text, faint `''` for empty strings, double-click `EditCell`, right mouse-down `ContextMenu`.
   - Gutter: checkbox and markers.
8. `keyboard.rs`: pure `edit_seed(&Keystroke) -> Option<EditSeed>`, called from `jump_key` when `table` is `Some`. It emits `EditCell { Page(row), source, seed }`.
9. `grid.rs`:
   - `new_table` sets bare chrome; the status row and export button render only for query grids.
   - Bare mode emits `GridEvent::Status` wherever it sets `copy_status`.
   - Public API from §4.2; `advance`; `open_export` (wraps `export`).
   - Query grids: the resize drag calls `ColumnWidths::set_explicit`.
10. `main.rs`: append `&& !Editor` to every `ResultGrid` binding at lines 201-262 (`cmd-c` included).

**Tests**

- `grid_columns`:
  - A live width is clamped, and offsets and total width update.
  - The override survives `table_columns(new page)` and is cleared by `load`.
  - `width_patch` emits the source name and refuses ambiguous duplicate names.
  - `entries` include hidden columns.
  - Hiding the last visible column is refused.
- `browse_preferences`:
  - `ColumnWidth` writes `columnWidths[name]` and rejects NaN, 47, 1201 and an empty or NUL name.
  - Visibility hide and show round-trips and preserves `presets` and `filterHistory`.
- `resize_width` clamps and is monotonic.
- `CheckedRows`: toggle, shift range from the anchor, select-all → `On`, partial → `Mixed`, clear.
- `edit_seed`: enter → Keep; f2 → Keep; backspace/delete → Clear; `a` → Replace("a"); `cmd-a`, `ctrl-x`, `alt-a` → None; `space` → None.
- `cell_rect`: pinned versus scrolling columns with scroll offsets.
- `cell_tone` precedence: Deleted > Updated > Insert; excluded halves the alpha; failed outlines.
- `sort_badge` returns a priority only when there is more than one key.

**Acceptance**

- Query-result grids (MySQL, SQLite, ClickHouse) render as before, plus drag resize.
- Table grids show no status row.
- No grid binding fires while an `Editor` descendant is focused.

### P3: Changes controller and overlays

**Owns and edits:**

- `apps/native/src/table_changes.rs`
- `apps/native/src/table_changes/view.rs` (rewritten as the overlay layer)
- new `apps/native/src/table_changes/cell_editor.rs`
- new `apps/native/src/table_changes/review.rs`
- new `apps/native/src/table_changes/change_list.rs`
- new `apps/native/src/table_changes/key_dialog.rs`
- `apps/native/src/table_changes/batch_edit.rs` and `batch_edit/tests.rs`
- `apps/native/src/table_changes/array_view.rs`
- `apps/native/src/table_changes/retention.rs` and `retention/tests.rs`
- `apps/native/src/table_changes/source.rs`
- `apps/native/src/table_changes/literal_guard.rs`

**Steps**

1. State:
   - `policy: TablePolicy` (default `UNKNOWN`).
   - `Edit.presentation: Inline | Popover`.
   - `cell_editor: Option<(Entity<CellEditor>, Subscription)>`.
   - `dialog: Option<Dialog>` with `Dialog::{Review(ReviewDialog), Discard, VirtualKey, ChangeList(Bounds)}`.
   - `overlay_cache: RefCell<Option<Rc<DraftOverlay>>>`.
   - `last_failed: Option<Uuid>`.
   - `popover_anchor`.
   - `PendingApply { ticket, flow, preconfirmed: Preconfirmation, auto_confirms: u32 }`.
2. `begin_edit`: resolve the target.
   - `Page`: the existing `edit_cell` path (`edit_value`).
   - `Insert`: `insert_value`.
   - Apply the seed; choose the presentation with the pure `edit_presentation`; open the `CellEditor` or the popover `Editor` (reusing `open_edit`, `literal_guard` and `array_view`).
   - Emit `EditOpened`.
   - Refusal reasons come from `can_edit_now()`: the policy's read-only reason, "Checking editable columns…", "No row identity. Choose ⋯ › Virtual key… to edit", "Generated column", or the 1 MiB limit.
3. Stage: the existing `stage()`, extended for `CellRef::Insert` through `set_insert_value`. On success it emits `EditClosed { advance }` and `OverlayChanged`. `CellEditor` events map to Stage, CancelEdit or Expand. Expand moves the text into the popover editor.
4. `add_row`, `remove_insert`, `stage_deletes` (maps page rows to `(values, row_identity)` and calls `draft.stage_deletes`), `set_null`, `revert`, and `duplicate` (stages through `duplicate_values`, no editor).
5. Review dialog (`review.rs`), state machine:

   ```
   Preparing → Ready{diff, sql, typed: Option<Entity<TypedConfirm>>}
             → Saving → Applying → (AutoConfirming | NeedsTyped) → Applied | Failed{message}
   ```

   - `request_review` builds `review_diff(plan.plan())` when the backend `Reviewed` reply arrives.
   - It creates a **fresh** `TypedConfirm` when `policy.confirm_style() == Some(Typed)`.
   - Apply is enabled iff not pending, and the style is Plain or the typed text matches, and the policy is not read-only. The gate is recomputed on every render, so a mid-dialog policy change takes effect.
   - The click records `preconfirmation(..)` into `PendingApply`.
   - `consume(Applied(NeedsConfirmation))` calls `on_needs_confirmation`:
     - `AutoConfirm`: reserve, `flow.needs_confirmation`, `flow.confirm(self.next())`, emit `PersistApply`, `auto_confirms += 1`.
     - `AskTyped`: create a `TypedConfirm`; Confirm runs the existing `Action::Confirm`.
   - Failure keeps the dialog open with `ui::error_banner(apply_error_message(..))`, sets `last_failed`, and emits `OverlayChanged`.
   - Success closes the dialog and emits `Applied`.
   - Esc and Cancel work only before dispatch (`flow.cancel_before_dispatch`). After dispatch the dialog shows "Stop" (`CancelPending`).
   - Footer button labels: "Apply N changes" (Disabled), "Confirm and apply" (Protected), "Apply N changes" (Typed). The Danger variant is used when Typed and the plan contains deletes.
   - Body: `env_notice(policy.environment, policy.describe(), loud = Typed)`, then the diff list (old value faint strikethrough, new value in warn), then "SQL · N statements" with mono `sql` and `format_param` lines, each with full-value AX labels.
6. Discard and Virtual key dialogs: move the existing key-editor UI (`view.rs:152-272`) into `key_dialog.rs`. Discard is a confirm modal.
7. `change_list.rs`: popover listing `change_summary` per change with include toggle (`Action::Include`) and remove (`Action::Remove`).
8. `view.rs`: render the overlay layer only. No bottom panel.
9. `summary()`, `command()`, `overlay()` (cached by `OverlayKey`), `inline_editor()`. Emit `OverlayChanged` from `changed()`, `page()`, `disconnected()`, `finish_apply`, `ConfirmDiscard` and `Reconcile`.

**Tests** (pure functions plus `TableChanges::new` without a window, following `table_changes.rs:1588`)

- `edit_presentation`: JSON column → Popover; text with `\n` → Popover; 5 KiB → Popover; short int → Inline; `Replace` seed on a JSON column → Popover keeping the replacement.
- `set_policy(read_only)`: `summary().can_review` is `Err` with the reason; `can_edit_now()` is `Err`.
- Overlay cache: same `Rc` when revision and page are unchanged; new `Rc` after `include()` changes the revision.
- Apply gate pure function `apply_enabled(style, typed_matched, phase, read_only)`, full truth table.
- `cancel_before_dispatch` is respected while the review dialog is `Saving`; a dispatched apply cannot be cancelled by Esc.
- `disconnected()` with an in-flight dispatched apply marks the outcome unknown (existing behaviour), and `summary().notice == Some(OutcomeUnknown)`.

**Acceptance**

- No bottom panel.
- Every former panel action is reachable through `command()`, the dialogs or the cell paths.
- `TableCommand::Apply` is never sent before `PersistApply` is acknowledged, for both apply and confirm.

### P4: Browse bar

**Owns and edits:**

- `apps/native/src/browse_controls.rs`
- new `apps/native/src/browse_controls/model.rs`
- new `apps/native/src/browse_controls/sort_panel.rs`
- new `apps/native/src/browse_controls/panels.rs`

**Steps**

1. `model.rs`:
   - `Operator` enum covering the 13 existing operators, each with `label()` and `sql_hint()` (`=`, `<>`, `ILIKE %x%`, `IN`, `IS NULL`, and so on).
   - `FilterDraft { rows: Vec<FilterRowDraft { column: String, operator: Operator, value: String }> }` with `from_filters(&[BrowseFilter]) -> (Self, Vec<BrowseFilter> /*unrepresentable*/)` and `to_filters() -> Result<Vec<BrowseFilter>, (usize, &'static str)>`. These reuse `build_filter` (`browse_controls.rs:778`). Several conditions on one column are allowed and are ANDed: assign `state.typed_filters` directly and never call `apply_filter`.
   - `header_sort` and `HeaderSort`.
   - Sort edits: `append`, `toggle_direction`, `cycle_nulls`, `remove`, `move_up`, `move_down`.
2. Filter bar render. One row per condition: × icon_button · "where" / "and" faint · column `select_trigger` (with cast type hint) · operator `select_trigger` (items carry SQL badges) · value field (hidden for is null / is not null). Trailing: `+ Add filter`, Apply (enabled when dirty; `Enter` in a value applies), Clear, SQL toggle (the existing `FilterMode::Raw` row).
   - Row editors are created per row, with a UI limit of 32 rows and a message beyond that.
   - Select popovers use `ui::popover::layer` and `MenuNav`, and close on `on_mouse_down_out`.
3. `sort_panel.rs`: the two-pane Drizzle sort popover (§3.8).
4. `panels.rs`: history list (apply on click), presets list plus save field, inspect (SQL and params, with copy buttons). These replace the cycle-through rows. Behaviour and limits stay the same (8 KiB name, 256 KiB inspection).
5. Public API from §4.4. `set_enabled(false)` disables every control and select while busy.

**Tests** (in `model.rs`)

- Round trip `from_filters` → `to_filters` for all 13 operators.
- Two conditions on the same column are preserved.
- An invalid `IN` list reports its row index.
- `is null` ignores the value.
- `RawSql` is reported as unrepresentable and kept out of the rows.
- Dirty detection.
- `header_sort`: Asc replaces a three-key sort with one key; Desc; Clear removes only that column; Clear on an unsorted column is a no-op.
- Sort edits keep NULL placement and order; move up at index 0 is a no-op; the 256 limit is respected.

**Acceptance**

- History, presets, inspect and raw WHERE keep their current semantics and limits.
- Every applied change still goes through `BrowseEvent::Apply` and `TableView::apply_browse`.

### P5: Table shell

**Owns and edits:**

- `apps/native/src/table_view.rs`
- new `apps/native/src/table_view/toolbar.rs`
- new `apps/native/src/table_view/menus.rs` (header menu, cell context menu, overflow)
- new `apps/native/src/table_view/columns_popover.rs`
- new `apps/native/src/table_view/pager.rs`
- new `apps/native/src/table_view/sync.rs`
- `apps/native/src/table_view/relationship_detail.rs`
- `apps/native/src/table_view/whole_config.rs`
- `apps/native/src/table_view/whole_export.rs`
- `apps/native/src/document_view.rs`

**Steps**

1. `set_connection_metadata` (§3.2) and the `document_view.rs` branch.
2. `sync.rs`: `sync_grid` (§3.1). Run it from the `ChangesEvent` handler (all variants), on page receipt, from `set_connection_metadata` and on busy changes.
   - `EditOpened { popover: true }` → `changes.set_popover_anchor(grid.cell_bounds(..))`.
   - `EditClosed` → `grid.advance` and focus the grid.
3. Grid event routing (replace `table_view.rs:234-262`):

   | Event | Handling |
   |---|---|
   | `EditCell` | `changes.begin_edit`; on `Err`, set `status` |
   | `HeaderMenu` | open the header popover |
   | `ContextMenu` | open the cell menu |
   | `RemoveInsert` | `changes.remove_insert` |
   | `CheckedRowsChanged` | notify |
   | `Status` | set `status` |
   | `Preferences` | save, or queue as `pending_width` while busy, flushed beside `after_analysis` |
   | Preference save error | additionally call `grid.clear_width_overrides()` (`drain_one` `PreferencesSaved` error branch) |

4. `toolbar.rs`: the §1 toolbar, using pure `toolbar_mode(&ChangesSummary, checked: usize, policy, connected, busy) -> ToolbarMode`. Tab order goes into `tab_order`. `capture_key_down`: `cmd-s` → `ChangesCommand::Review`; `escape` closes the open popover.
5. `menus.rs`:
   - Header menu items from §1. Sort through `header_sort` → `apply_browse`. Column actions through `grid.column_patch` and `save_preferences`. "Filter on this column…" → `browse_controls.add_condition(Some(name))`. "Multi-column sort…" → `open_panel(Sort)`.
   - Cell menu items from §1, routed to `TableChanges` and the existing `Action::ForeignKeys`, `Bulk` and `InspectCell`.
   - Overflow items from §1, routed to the existing `Action::*` and `TableEvent::*`, `ChangesCommand::OpenVirtualKey`, `browse_controls.open_panel(..)` and `grid.open_export`.
6. `columns_popover.rs`: search field, `check_item` per `ColumnEntry` (eye toggle → `visibility_patch`; pin toggle → `column_patch(TogglePin)`), Show all, Auto-fit all, Reload preferences.
7. `pager.rs`:
   - Pure `page_range_label(page, page_size, rows, total: Option<(u64, bool)>) -> String` ("1–100 of ~18,204"), derived from the existing `page_summary` (`table_view.rs:2086`).
   - Popover: Limit select over `PAGE_SIZES` → `Action::PageSize`; Page field plus Go → `PageAction::Jump` (validated ≥ 1 and ≤ last when the total is known); First and Last.
8. Render:
   - Root `relative()`: toolbar · `browse_controls` · notice strip (`ChangesSummary.notice` plus `preferences_status`) · grid · reference/detail panels · footer.
   - Then, as last children, an `absolute().inset_0()` wrapper containing `self.changes.clone()`, and the TableView popovers (deferred).
   - Remove the `columns` row, the old toolbar loop and the bottom `changes` placement.
   - Keep the focus restore for detail buttons.
   - Delete the `Action` variants made unreachable; keep the `activate` gating rules.
9. Keep `TableEvent` unchanged; Structure uses `OpenStructure`.

**Tests**

- `toolbar_mode`:
  - Staged changes show Review with the count.
  - Checked rows show Delete N.
  - Read-only shows the badge, hides Add row and disables Delete with a reason.
  - Disconnected shows Connect.
  - Busy shows Cancel.
- Reachability: `fn reachable_actions() -> BTreeSet<LegacyAction>` is the union of the toolbar, overflow, header-menu and cell-menu item tables. It must equal the full set of 19 legacy toolbar and 10 column actions, minus Narrow/Widen, which the header menu covers.
- `page_range_label`: estimated `~`, exact, no rows.
- Pager jump validation.
- `set_connection_metadata` record selection: matching id → `from_connection`; missing → `UNKNOWN`; non-PostgreSQL → `UNKNOWN`. Implement it as the pure `policy_for(records, id)`.

**Acceptance:** §1 is visually met, all previous actions are reachable, and tab order covers the new controls.

## 6. Merge order and integration (coordinator)

1. Merge P1. It is self-contained.
2. Merge P2, then P3, then P4, then P5. Each depends only on earlier packages.
3. Integration glue:
   - `document_view.rs` is already in P5.
   - Reconcile any signature drift against §4. The interface text in this plan wins.
   - Check that `DmlParam` is exported through `dbunk_lib::backend::data::*`. If it is not, add the re-export in `backend/src/backend/data` (coordinator-owned).
4. Run `just fmt`, `just lint`, `just test` (backend) and `just fmt-native`, `just lint-native`, `just test-native`.
5. Manual check on the fixture (`just native-fixture-up`, `just dev-native`):
   1. Drag-resize a column live, release outside the grid, reload the tab; the width persists.
   2. Double-click a resize handle; the column auto-fits.
   3. Double-click, Enter, and typing all open the inline editor. Esc cancels; Enter stages with the warn tint; Tab moves right.
   4. A JSON cell opens the popover editor. Invalid JSON keeps the draft. Set NULL works.
   5. Add row: edit band cells, then × removes the row.
   6. Check two rows, Delete 2 rows: deleted tint, then review.
   7. Review shows `old → new`, inserts, deletes, and SQL with `$N` values.
   8. Development + Inherit: plain Confirm, applied.
   9. Staging + Inherit: plain Confirm, applied, and an audit row is recorded.
   10. Production: the field requires `confirm`, then the change applies with no second click.
   11. Read-only: inline reason, no Add row, no write path.
   12. Filter bar: two conditions with AND, Apply, Clear.
   13. Header menu Asc/Desc/Clear; multi-sort popover; total refresh after a filter change.
   14. Kill the fixture mid-apply: the outcome-unknown strip appears and editing stays blocked until Mark resolved.
6. Update `plans/README.md` (Plan 032 row) and add parity-register notes for PAR-002, PAR-003 and PAR-004.

## 7. Risks and how behaviour stays predictable

| Risk | Mitigation |
|---|---|
| A drag does not start until the pointer moves past a threshold, and drag and click share the handle | Click (double-click → auto-fit) and drag on one element is the same as Zed's handle. The handle stops mouse-down propagation, and the header menu opens on `on_click` (mouse-up) so the two never conflict. |
| Mouse-up lands outside the grid, or the window deactivates | Root `on_drop` plus `on_mouse_up_out` plus finishing on the next mouse-down. `finish_resize` is idempotent. |
| Resize cost | Recompute offsets in O(columns) and notify only on a change of at least 1 px. `uniform_list` renders only visible rows. |
| The editor loses focus when its row is virtualized away | The editor sits in a sibling layer outside `uniform_list`, so it is always rendered. Click-away commits. |
| Key conflicts inside the embedded editor | `&& !Editor` on grid bindings (V4). The cell editor uses the proven `capture_key_down` pattern (`view.rs:106`). If `enter` or `escape` in the single-line editor are swallowed by an Editor binding, the fallback is a binding in `"CellEditor > Editor"` added by the coordinator in `main.rs`, since later bindings win at equal depth. |
| IME | Commit, Cancel and Expand are skipped while `marked_text_range` is `Some` (existing `composition_active`). |
| Focus in dialogs | Dialogs keep their own `capture_key_down` Tab cycle over their handles. Closing a dialog emits `FocusGrid`. `TableView::focus_document` includes dialog handles. |
| Overlay staleness | Keyed by `(draft owner, revision, page Rc ptr)`. Rows are matched by identity, never by position. Ctid and virtual-key rows also need equal originals. |
| Stale edit target | `page()` closes edits. Insert targets are checked by change id at stage time. Bulk and duplicate keep `EditContext::current`. |
| Review drift | `ReviewPlan` revision fence (`begin_apply`). The modal blocks staging. `TypedConfirm` is recreated for every review, and the gate is recomputed live from the current policy. |
| Auto-confirm scope | Only for the `PendingApply` that recorded `Preconfirmation::Granted`, and at most once. An unexpected `NeedsConfirmation` escalates to Typed (`AskTyped`). The backend token remains the enforcement boundary (ADR-0024). |
| Outcome unknown | Connection loss, timeout or a missing reply sets `outcome_unknown`. Editing is refused, the notice strip offers Refresh and Mark resolved, and nothing is retried automatically (`finish_apply` contract). |
| Reconnect | `disconnected()` invalidates the draft and closes the edit and dialog. The overlay is rebuilt from the next page by identity. The journal is persisted before every dispatch, for apply and confirm. |
| Busy gating drops events | Width patches are queued (`pending_width`). Other grid events while busy produce a status message instead of disappearing silently. |
| Mismatched interfaces across worktrees | §4 is normative. Merge in order. The coordinator fixes drift once, at integration. |

**STOP conditions.** Stop and ask if any of these happen:

- An implementer needs a file owned by another package.
- The backend protocol must change beyond `policy.rs`.
- An existing durable format (`WorkspaceMutationDraft`, `TableGridPrefs` keys) would need a breaking change.
- A test asserting the safety matrix has to be weakened.

### Critical Files for Implementation

- /Users/imran/projects/Code/dbunk/apps/native/src/grid/render.rs
- /Users/imran/projects/Code/dbunk/apps/native/src/table_changes.rs
- /Users/imran/projects/Code/dbunk/apps/native/src/data_model/mutations.rs
- /Users/imran/projects/Code/dbunk/apps/native/src/table_view.rs
- /Users/imran/projects/Code/dbunk/backend/src/safety/policy.rs