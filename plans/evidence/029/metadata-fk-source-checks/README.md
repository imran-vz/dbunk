# Complete object-description kinds and composite FK navigation, 2026-10-03

Status: integrated source checks, exact fixture probes and separate package pass. This follows
[the frozen catalog/array variant](../catalog-array-source-checks/README.md).
No new actual-window, keyboard/AX or real IME pass is recorded. VoiceOver remains
deferred, not passed.

## Implemented scope

All baseline `PgObjectKind` descriptions now have bounded dedicated readers.
Table, foreign-table, type and domain reads stream guarded component rows under
a shared 4096-component/8 MiB JSON allowance. SQL assembly refuses before exceeding
1 MiB; returned descriptions still cap at 8 MiB. Metadata fields cap at 8 KiB.
Every query uses the existing document admission, read-only repeatable-read
snapshot, cancellation and joined socket/deadline path. Foreign metadata never
selects foreign rows or contacts a foreign server.

Table reconstruction retains primary/unique/check/FK/exclusion constraints and
standalone indexes, avoids inherited/constraint-backed index duplication, and
correctly renders identity/generated columns. Partition parent/key/bound data,
FDW options with embedded `=`, qualified collations, enum ordering, composite
attributes, range/multirange options and domain default/nullability/check facts
are preserved. Separate legacy server SQL handles absent generated/multirange
catalog columns. Only the current owned server was live-tested.

Definitions are limited reconstruction. Table SQL omits ordinary INHERITS,
storage options, RLS, triggers and grants. Foreign partition membership,
composite attribute collations and domain CHECK names remain outside baseline
reconstruction. These omissions are disclosed in the native viewer; exact source
text and metadata copying remain available. Multirange descriptions reconstruct
the parent range with its multirange name, matching baseline behavior. No DDL
apply, drop impact, structure editor or cluster administration is activated.

Foreign-key navigation reads at most 256 constraints/4096 ordered column pairs/
1 MiB metadata through the same owned reader. It binds full composite values as
typed equality filters, uses exact names/connection, and never interpolates row
values into SQL. The chooser uses original loaded values, preserving staged edits.
It checks the exact page allocation, connection and selected source cell on reply
and Open. Cancellation discards a late successful navigation reply. NULL components,
partial/truncated pages, missing/duplicate columns and oversized values refuse.
Metadata/review reserve 8 MiB from shared retention; choices cap at 2 MiB and
filters at 60 KiB. Captured pages release before their original page allowance.
New table filters survive preference loading and persist as ordinary table intent.

Parent table traversal now yields to active composition in raw/typed filter and
preset-name fields as well as specialized/raw/array cell editors. This is source
handling only, not observed composition evidence.

## Live evidence and corrections

- `foreign-keys-live.txt`: exact ignored probe passes. Ordered composite and
  single constraints, quoted/dotted identifiers, apostrophe/backslash/Unicode text,
  bigint extrema, parameterized target browsing, empty metadata for a handler-free
  foreign table, missing relations, cancellation and retired-document refusal.
- `remaining-descriptions-live.txt`: exact ignored probe passes for table,
  partition parent/leaf, foreign table, enum/composite/range/multirange and domain
  metadata, wrong-kind refusal and owned cleanup. Earlier eight-kind probe remains
  scoped to its frozen package, not silently rerun against this variant.
- Initial remaining-kind probe failed because the fixture's unqualified explicit
  multirange name resolved into `public`. Root verified its dependency on the owned
  range and moved that exact OID into the owned schema. The failed log and correction
  record are preserved. This was a fixture correction, not an application fix.
- The rerun exposed a reader bug: this PostgreSQL 17.11 fixture has a separate
  `contype = n` domain NOT NULL entry. It was incorrectly returned as a CHECK.
  Native domain queries now select CHECK entries only; relation reconstruction
  also avoids duplicating column NOT NULL entries as table constraints. The failed
  log, catalog diagnosis and successful rerun are retained.
- The only new objects were owned `native_metadata_20261003` and its handler-free
  wrapper/server on stage03 `127.0.0.1:15432/dbunk_demo`, UUID
  `2283820d-33ec-4c4c-ae03-7051092bd410`. Setup refused existing names. Cleanup
  rechecked UUID, OIDs and ownership comments, then removed the owned schema,
  server and wrapper. The moved multirange OID is also gone; activity is zero.
  See `fixture-setup.sql`, identity/correction records and `fixture-cleanup.json`.

## Final verification

- `pnpm format`, `pnpm lint`, `pnpm typecheck`: pass.
- `just fmt`, `just lint`, serialized `just test`: pass; core 675 passed/71
  ignored, Tauri 692 passed/85 ignored.
- Isolated backend: 785 passed/79 ignored, two documentation tests passed;
  Tauri facade subset 61 passed/8 ignored. Native dependency proof passes.
- Native format, debug/release all-target Clippy, fixture-harness Clippy,
  debug build and debug/release tests pass: 125 passed/13 ignored in each.
- Tauri custom-protocol build passes.
- Separate build-only package: `/private/tmp/dbunk-native-package-20261003-metadata-fk/dbunk Native Preflight.app`.
  Executable SHA256: `eb23cfcd4f4fbbf5afde789191d81ad0ffbc6e5913d300e681462bba8e134a16`.
  All 295 recorded source hashes match. Package identity and build log are retained.

Ignored tests in aggregate runs are not passes. The two named opt-in probes
above were explicitly executed. This package was not launched.

A fresh `cua.getApp('com.apple.systempreferences')` probe again returned
`cgWindowNotFound`. No OS setting changed and no further native app was launched.
The new controls still need actual-window acceptance. Complete PostgreSQL parity,
profile/package compatibility, broader keyboard/AX/IME checks and the combined
acceptance workload remain open.
