# Native column pinning, 2026-10-03

Implementation and verification in progress. Plans 027–030 remain IN PROGRESS.
The baseline is Tauri `102568b`, DataGrid ordered left pins and its separate
layout preference record. No right-side pinning is claimed.

Table pins use unambiguous source names in `pinnedColumns` and the existing
latest-record preference worker. Query pins use original source indices, so
identical or omitted headings cannot redirect selection, copy or editing. Each
query result owns its layout; query pins are ephemeral. The query layout vector
is charged through column-metadata admission against the shared 128 MiB retained
payload limit. This is not process RSS.

Pins append in selection order. Unpin restores underlying table order or query
source order among unpinned columns. Hidden or absent table pins remain stored.
Table reordering stays within its current pin group and refuses stale neighbors
or changed visibility. Unknown preference fields survive; the complete 64 KiB
record limit and exact acknowledged publication remain in force. Import of old
Tauri's separate layout records belongs to the pending profile migration gate.

A frozen prefix and scrolling remainder use the same display/source mapping.
The prefix uses at most half the viewport while unpinned columns remain; all-pinned
results may use the whole viewport. Overflow pins pan independently with horizontal
input over the prefix or keyboard selection. Both panes render only their visible
column ranges plus bounded overscan. The existing Results menu and table column
controls expose Pin / unpin.

[Final source/package checks](./pinning-source-checks/README.md) pass, including
required repository checks and native debug/release Clippy/tests: 271 passed,
13 ignored. Independent review's all-hidden recovery issue was fixed with an
explicit Show all columns refusal and a focused test. The frozen package matches
467 source hashes.

[Scoped AX/copy checks](./pinning-window-20261003/README.md) and
[persisted table reopen](./pinning-reopen-20261003/README.md) pass their listed
cases, with normal quits and both fixtures 0 → 0. Visual alignment and resize
acceptance remain open: captured surfaces became stale or lost unchanged content,
also reproduced on the preceding package, and wheel input reported
noWindowsAvailable. One resize frame showed header/body offsets; no complete
resize pass or speculative source fix is claimed. Broader keyboard/AX and new-tool
IME remain pending. Temporary keyboard settings were restored; VoiceOver deferred.
