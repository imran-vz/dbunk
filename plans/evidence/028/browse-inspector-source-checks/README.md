# Browse controls, value inspection and typed editors, 2026-10-03

Status: Rust source verification and isolated release packaging passed. These changes postdate
both the column package and the frozen Tool tabs package. Neither package proves
this variant. No new actual-window, keyboard/AX or IME pass is recorded.

Implemented: typed/raw filtering, same-column replacement and removal, clear,
sort direction/NULL placement, bounded successful-query history, named preset
save/apply, exact accepted-page SQL/typed-parameter inspection, and serial
read/patch/commit preference updates across native table documents. Native SQLite
preference reads reject oversized/corrupt/future records before materializing
large text; unrelated stored fields survive patches. Table preferences cap at
64 KiB, history at 20 entries, and controls reserve 2 MiB from the shared 128 MiB
encoded-payload budget. Reservation can be retried after other data is cleared.

The value inspector distinguishes SQL NULL, empty string and retained text. Copy
uses exact original text. JSON pretty-printing preserves numeric spelling, key
order and duplicate keys. Hex shows UTF-8 bytes of retained textual data, capped
at 4096 preview bytes. Input and formatted values cap at 1 MiB. Owned raw/derived
inspection values share the retained-payload budget and refusal preserves the
current view. This is not binary decoding or an RSS limit.

Typed cell editors validate JSON, known comma-delimited one-dimensional arrays,
and the baseline WKT prefix heuristic. Raw fallback remains explicit for nested,
dimension-prefixed and unknown-delimiter arrays. Database validation remains
authoritative. Per-element array controls and a geometry map remain absent.

Read-only integration review found four issues corrected in source: inspector
Tab intercepted by its table parent; premature clearing of rejected typed input;
analysis and preference-save competing for the one-slot request lane; and failure
to reacquire a refused control reservation. Successful browse saves now wait for
analysis settlement; cancellation/disconnection remove the deferred save. Review
also found staged-cell reopening and specialized keyboard shortcuts; those
corrections are integrated. Staged-cell reopening now preserves the staged value,
including changed keys, while retaining original optimistic guards. Specialized
editor shortcuts respect active composition. Virtual-key selection/save/clear
is ordered, schema-bound and serialized; pending, staged, uncertain or
unrestorable drafts prevent key changes. Permission stays invalid until fresh
analysis after a committed preference change. Key reads refuse oversized,
corrupt or future data without rewriting it.

Initial checks (before review corrections/virtual keys): native debug/release
110 passed, 12 ignored; all-target and fixture-harness Clippy pass. Required
backend checks pass: `just fmt`, `just lint`, serialized `just test` (675 core,
692 Tauri), isolated-profile checks (748 passed/75 ignored, 54/4 Tauri subset,
two compile-fail doctests), and custom-protocol build. Ignored tests are not
passes. The final frozen source variant passes native debug/release tests
(117 passed, 13 ignored), all-target and fixture-harness Clippy, native build,
release package build and dependency proof. `just fmt`, `just lint` and serialized
`just test` pass (675 core, 692 Tauri); isolated-profile checks pass (761/76
ignored, 60/5 Tauri subset, two compile-fail doctests). The custom-protocol build
passes. See `final-*.txt`. `pnpm format`, `pnpm lint` and `pnpm typecheck` also pass; see
`frontend-*.txt`.

EXPLAIN and EXPLAIN ANALYZE open explicit durable query drafts using the existing
Query Session execution/policy path. The retained plan binds exact executed SQL
and complete result output; partial, failed, cancelled and oversized output is
refused. The native virtual tree exposes metrics, collapse/expand, original JSON
and source SQL. Plan retention shares the 128 MiB allowance. Tree keyboard/AX
behavior is implemented but lacks window verification.

The owned catalog read uses a dedicated joined socket, DataDocument admission and
cancellation, a read-only repeatable-read snapshot, and an absolute 30-second
operation deadline. It preserves per-kind truncation markers and additionally
refuses above 10,000 scanned nodes, 8 MiB encoded output or 8 KiB text fields.
Catalog UI, object descriptions and DDL are not part of this frozen variant.

Two exact read-only fixture probes pass: `catalog-live.txt` verifies catalog kinds,
repeat/cancel, retired handles and cleanup; `explain-live.txt` verifies EXPLAIN
and ANALYZE through Query Session plus incomplete-plan refusal. The owned stage03
fixture identity was checked and activity returned from zero to zero.

Package: `/private/tmp/dbunk-native-package-20261003-browse-tools/dbunk Native Preflight.app`.
Executable SHA256: `d9f98dddce6cd72aa5ccc112008c2b50cb2fc72a98bc7d37cd60c7c33a961a8d`.
All 280 recorded Rust/Cargo hashes matched after packaging; see
`source-sha256.json` and `package-identity.json`. Subsequent source work is not
covered by this package or these results.

Window automation remains unavailable: `cua.getApp` again returned
`cgWindowNotFound` for the previously opened System Settings window. No additional
native process was launched and no settings were changed. See the prior
[Tool tabs discovery attempts](../../029/tool-tabs-window-20261003.md). Earlier
scoped Pinyin evidence stays valid only for its named controls and binaries;
VoiceOver remains deferred and its checklist is preserved.
