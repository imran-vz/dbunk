# Native SQL completion integration, 2026-10-03

Status: implemented; required source checks and scoped live probe pass. The separate package and native dependency proof pass. This does not finish Plan
029 T01, which also requires SQL formatting and native-window acceptance. No
keyboard/AX or real IME acceptance is recorded for the completion menu.
VoiceOver remains deferred, not passed.

## Behavior and ownership

The pinned native editor now receives keywords and connection-bound catalog
schema/table/view suggestions, plus lazy column/type/primary-key suggestions in
predicate contexts. Catalog identifiers are quoted exactly, including mixed case,
embedded quotes and quoted dots. The lexer excludes comments and string regions,
keeps statement boundaries, and replaces the whole bounded token around the
caret. General contexts retain keyword priority even when the catalog is large.

The provider owns no database task. Requests share the query document's existing
owned data worker with result editing. Metadata waits while an edit operation is
pending; typing only invalidates publication context and never cancels the worker
or a mutation. The backend column facade reads exact qualified catalog identities
through the existing owned read lifecycle. It does not read user/foreign rows,
parse reconstructed DDL or infer mutation authority.

Completion publication rechecks connection generation, buffer version, underlying
buffer/caret and actual marked text. Metadata may populate its connection cache
after the caret moves, but only an unchanged editor context may automatically
refresh the menu. Retarget/disconnect clears cache and visible menus and temporarily makes the
editor read-only. The next window/input boundary dismisses pinned-editor
completion tasks before restoring editability, covering stale mouse/AX menu
acceptance as well as the keyboard path. Composition
start dismisses completion UI once, without a continuous repaint loop. These
source guards still need real-window keyboard/AX and composition verification.

The Query menu offers Refresh SQL completions for explicit retry and metadata
refresh. Metadata failures, partial catalog capture and candidate limits remain
visible. Ordinary SQL buffer-word suggestions are disabled so they are not
silently mixed into the provider's bounded candidate list.

## Scope and bounds

Column reads return the captured relation OID/kind and ordered name/type/PK
metadata for ordinary and partitioned tables, views, materialized views and
foreign tables. Missing relation is distinct from an empty column list. Caps
refuse the entire response: 4,096 columns, 1 MiB encoded, 63-byte names and
8 KiB type descriptions. Catalog visibility is not proof of SELECT privilege
or editability. Type text uses PostgreSQL format_type and is display metadata.

Native lexical context admits 64 KiB before the caret, 4,096 tokens and a
1,024-byte replacement token. Menus admit 128 candidates and 32 KiB text under a
2 MiB lease retained by editor-owned completion clones. The cache admits a 4 MiB
lease, at most 10,000 schema/relation nodes and 1 MiB name capacities plus 1 MiB
ordered columns. Replacement admission preserves a good prior cache on refusal.
Shared workspace 128 MiB retention and 16 MiB delivery budgets still apply.
The pinned editor's preceding completion-query allocation is outside this
provider; these retention allowances are not process RSS or transient-peak claims.

Predicate resolution deliberately follows the baseline's last-relation scope;
this is not semantic alias, CTE or subquery resolution. Public, or the first
cached schema, is a suggestion fallback, not the query session's observed
search_path. This context must never be reused to authorize result mutations.

## Verification

- Six backend focused tests, backend Clippy and format check pass.
- Thirteen native focused tests pass. Native format, debug/release all-target
  Clippy, fixture-harness Clippy, debug build and debug/release tests pass:
  176 passed/13 ignored in each test run.
- Frontend format/lint/typecheck and `just fmt`, `just lint`, serialized
  `just test` pass: core 677 passed/71 ignored; Tauri 694 passed/85 ignored.
- Isolated backend Clippy/tests pass: 825 passed/82 ignored and two doc tests;
  Tauri facade selection passes 81 tests/11 ignored. Custom-protocol build passes.
- Exact owned stage03 probe passes existing table/view identity, column order,
  type/PK values, literal identifier handling, idle-cancel reuse, retired-document
  refusal and joined cleanup. It creates no database objects. Materialized,
  foreign and partitioned relations have source/kind coverage, not live proof
  from this probe.
- Initial pinned-editor API compile mismatches and Clippy findings are preserved
  in separate logs and were corrected. Separate packaging/dependency proof pass; all 333 frozen source hashes match
  after package completion. Ignored tests are not passes.

## Frozen package

Package: `/private/tmp/dbunk-native-package-20261003-completion/dbunk Native Preflight.app`.
Executable SHA256: `278f4227054b8717b82b11cd1839289423079e8092293842e17fc5b05d9a59cc`.
Bundle size: 123,063,452 bytes. See `package-identity.json` and `package.txt`.
The owned [window attempt](../completion-window-20261003/identity.json) failed
discovery via both exact package path and bundle ID with CUA error -10005,
`cgWindowNotFound`, although app inventory reported it running. No keyboard/AX
or IME interaction was possible. Exact-owned PID 81905 was stopped with SIGTERM
after executable/profile/hash verification; the fixture returned to zero backend
connections. This forced cleanup is not a normal-quit pass. Actual-window
acceptance remains pending; no systemwide cause is established.
