# XLSX import source, fixture and window checks

Scope: native XLSX workbook/sheet selection through the approved transfer Tool
tab and Bottom review, with the existing transactional CSV import service. Full
PostgreSQL parity and complete keyboard/AX/IME acceptance remain open.

## Source verification

The [source manifest](./source-sha256.json) records 545 files. Required pnpm
format/lint/typecheck and just fmt/lint/serialized test passed. The required test
configurations passed 677/71 ignored and 694/85 ignored. The isolated suite passed
1,005 tests with 90 ignored and two doctests. Final debug and release native tests
passed 282 with 13 ignored. Final native Clippy, harness Clippy and release Clippy
passed. Ignored tests are not passes. Facade tests passed 200 with 19 ignored. The Tauri custom-protocol build,
packaging and dependency proof passed; JSON receipts record each command.

Focused frozen checks passed 16 parser tests, 21 transfer tests (two ignored),
and one bounded-source-copy test. Tests cover exact numeric and SpreadsheetML
strings, hostile ZIP/XML admission, sparse extents, indexed duplicate headers,
NULL/empty distinctions, cancellation, writer failure, revision invalidation,
accepted-job ownership and join-before-unlink on timeout. Initial failed attempts
remain recorded; final successful logs supersede only their named checks.
[Independent review](./review.md) records the corrected findings.

[Dependency-edge review](./dependency-review.md) proves unchanged package IDs,
versions and resolved features for the newly direct existing ZIP/XML dependencies.
Exact license texts are included in native packaged notices. Regenerable compiler
cache reclamation is recorded separately; source, profiles, logs and frozen
packages were preserved.

## Owned PostgreSQL probe

[Probe output](./live/probe.txt), [identity](./live/identity.json) and
[teardown](./live/teardown.json) record stage03 UUID
`2283820d-33ec-4c4c-ae03-7051092bd410` at `127.0.0.1:15432/dbunk_demo`.
The actual parser, canonical CSV and PostgreSQL COPY imported two rows with exact
bigint/decimal text, Unicode, NULL/empty/literal distinctions and indexed duplicate
headings. Replacing the original workbook after inspection did not change the
private snapshot. Source files were removed only after transfer settlement.
The unique schema and table were removed using captured OIDs/owner/comment and
RESTRICT. Joined shutdown returned fixture connections 0 → 0.

## Scoped window verification

The frozen executable `c9f77b6fe84848be1dbd8cb20d76cc7753b0f9c370e6ce6cb67aeaebf607a8bb`
passed native file selection, keyboard worksheet selection, hidden-sheet visibility,
sheet-change invalidation of the previous review, indexed mapping, exact review
and a single explicit import. Before Start the target was empty. The completed
receipt reported two committed rows and complete cleanup; independent PostgreSQL
assertions verified exact signed bigint extremes, decimal text, Unicode, NULL,
cached formula value, default and generated columns. [Exact rows](./window/exact-rows.json),
[UI evidence](./window/ui) and [launch identity](./window/identity.json) identify scope.
The selected [workbook fixture](./window-file.json) remains available.

The app quit normally with its workspace/drafts retained. Both fixture connection
counts returned 0 → 0. Only the [owned target](./window/owned-target.json)
`native_xlsx_window_20261003.destination` was removed, with captured OID/owner/comment
checks and RESTRICT; see [teardown](./window/teardown.json) and [cleanup](./window/cleanup.json).

This check found stale acknowledgement text and missing Page Up/Home receipt
scrolling. [Corrections and their separate package/window recheck](./window-corrections/README.md)
pass the named navigation and message scenarios; the original package and source
manifest remain frozen. This is scoped workflow
evidence, not complete keyboard/AX/IME acceptance.

The IME precheck could read System Settings and screenshots but still produced
plain `ni` with Control-Space and Control-Option-Space after temporary Pinyin setup.
No marked text/candidate evidence was observed. This is inconclusive outside GPUI,
not a new IME pass or product failure. ABC-only input, hidden input menu and
English (United States)-only dictation were restored. VoiceOver remains deferred.

## Disclosed limits

Selected-sheet XML is buffered within strict bounds; this is not a new
large-workbook streaming feature. One 64 MiB parser reservation is separate from
the shared UI retained/delivery bounds and is not an RSS claim. Encrypted/ZIP64,
ambiguous ZIP endings, filename overrides and unsupported XML/cells refuse.
Cached formula values are never recalculated; numeric dates remain serial text.
Source and canonical disk artifacts are bounded at 256 MiB and 512 MiB.

Transfer jobs/paths remain session-only. There is no crash-time replay or private
temp-file scavenger. Local file calls are joined but not claimed hard-cancellable.
Failed settlement preserves ownership and visibly blocks release/retirement;
unknown outcomes are not automatically retried.
