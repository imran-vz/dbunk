# Drizzle Studio data grid: observed spec

Observed live (drizzle-kit 0.31.11, SQLite). Screenshots in /tmp/drizzle-research/shots/ (view with the Read tool). Grid = react-data-grid; Radix menus; CodeMirror editors.

## Key screenshots
01-users-grid (header/NULL), 02-header-menu, 05-header-menu-sorted-col, 06-multicolumn-sort-popover, 09-multisort-grid, 11-filters-open-empty, 12-filter-operators-integer, 14-filter-condition-typed, 15-filter-applied, 20-filter-two-applied, 23-columns-popover, 25-resize-hover, 26-resizing-narrow, 27-after-resize-dblclick, 28-cell-hover-icons, 30-date-editor-popover, 32-cell-inline-editor, 47-panel-after-enter, 48-grid-dirty-cell, 50-inline-input-editor-int, 55-after-multiline-save, 56-add-record, 57-new-row-filled, 58-rows-selected, 62-cell-range-drag, 63-after-delete-click, 64-after-delete-confirmed, 75-json-editor, 77-json-invalid-save, 84-page-size-menu, 86-export-menu, 91-cleared-cell, 95-paste-result.

## Layout
Toolbar L→R: sidebar toggle; segmented DATA | STRUCTURE; back/forward + history; Filters, Sort, Columns (each with "!" badge when active); Add record (primary); right side: query time ("16ms"), `<`, clickable range "1 - 50" (Limit/Offset popover), "of", clickable total "200" (re-count), `>`, Refresh, `⋯` menu (Refresh rows, Refresh schema, Export ›, Copy ›).
With staged edits: Filters/Sort/Columns hidden; Add record, green Save changes, link-style Discard changes. With checked rows: red "Delete N records".
Header 32px: bold name + small muted mono DB type; truncated; chevrons-up-down icon → asc/desc icon when sorted; 10px col-resize handle at right edge; draggable to reorder.
Rows 32px mono. Frozen 32px checkbox column first (header select-all with indeterminate). Row hover shows expand-row icon. Default data col width 200px.
NULL = muted "NULL"; empty string = muted "EMPTY_STRING"; JSON raw one line truncated. FK cells show → on hover.
No footer; range + total in toolbar.

## Interactions
Resize: drag handle, min 100px clamp; double-click handle autofits.
Columns popover: search, eye toggle per column, drag reorder, hide-all.
Sorting: header click opens menu (Sort Ascending / Sort Descending; + Clear Sort, Multicolumn sort when sorted). Header sort replaces existing sort. Sort popover two panes: left searchable unsorted columns (click appends), right ordered list with drag handle, ASC/DESC toggle, ×; "Clear sorting". Server-side, re-queries immediately.
Filters: bar under toolbar, one row per condition: ×, "where"/"and", column picker, operator picker, value input. AND only. Operators with SQL badge: = <> > >= < <= LIKE NOT LIKE IN(comma list) is null / is not null. Dirty condition shows Apply; Enter applies. count(*) button, Add filter, Open in SQL, Clear filters. Known bug to avoid: stale total after filter change.
Selection: click selects cell (blue outline, row highlight); drag/shift+arrows rectangular range; arrows move. Row checkboxes separate; shift-click ranges.
Copy/paste: ⌘C TSV; ⌘V into cells → staged edits. Right-click: Copy, Paste, Export ›, Copy as ›, Expand Row.
Editing: double-click, Enter, or typing a char opens in-cell single-line input; Backspace opens cleared. Enter / click-away stages; Esc cancels; Tab → multiline editor with draft. Hover buttons: multiline editor, JSON editor {}, date editor. Multiline/JSON popover ~480px anchored under cell with line numbers; footer Set NULL / Cancel (Esc) / Save (⌘↵). Invalid JSON → error, draft kept. Date editor: NULL, mode, calendar, time.
Expanded row panel: right side, one field per column, dirty field orange label + revert icon.
Dirty state: all edits batched; edited cells orange bg/text; new rows fully orange. Save changes (⌘↵) commits batch with no confirmation; Discard reverts. Race to avoid: ⌘↵ in an open editor must include the in-progress value.
Add record: draft row at top; columns with default show DEFAULT, else NULL; × removes draft.
Delete: check rows → Delete N records → confirm modal → immediate. Failure modal shows SQL + error.

## Pagination
Default limit 50 offset 0; range popover edits Limit/Offset; < > page; total via count(*), click to refresh.

## Confirmations
Drizzle: none for edits, modal for deletes, SQL in error modal. (dbunk must do better: env-aware review with diff + SQL.)
