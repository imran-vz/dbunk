# Retained-grid keyboard commands, 2026-10-03

Implementation and verification in progress, following the frozen pinning
increment. This does not close Plan 028 or full PostgreSQL parity.

Home/End address the first/last displayed column in the current row; Cmd or Ctrl
with Home/End addresses the first/last retained cell. Page Up/Down moves by the
measured visible row count minus one, clamped to at least one and to the retained
result or current table page. Shift extends the existing rectangle from its
anchor, including a first extension from an otherwise unselected grid. Escape
clears the rectangle while retaining the focused source cell. No command fetches
rows, moves between database pages, executes SQL or stages a write.

The handler requires the grid itself to own focus, preserving embedded editor,
Go-to-row field and IME key ownership. Column coordinates remain displayed
coordinates and reuse pin-aware reveal plus source mapping. There is no new
layout or unbounded retained allocation. Independent checkbox/noncontiguous row
selection, broader grid shortcuts and complete grid acceptance remain open.

Focused endpoint/page calculations cover empty results, stale positions, tiny
viewports and saturating page movement. Required checks, native debug/release
Clippy/tests (273 passed, 13 ignored), package and dependency proof pass; see
[the frozen source evidence](./grid-keyboard-source-checks/README.md).
Actual keyboard and AX verification remains pending because the active tool
connection no longer exposes native desktop control. Pinning's earlier visual
capture/foreground limitation is recorded separately, as is deferred VoiceOver.
