# Object Tool tab, bounded descriptions and array editing, 2026-10-03

Status: frozen source verification and isolated release packaging passed. This source variant follows the
[frozen browse/inspector package](../../028/browse-inspector-source-checks/README.md).
No actual-window, keyboard/AX or real IME pass is recorded for these additions.
VoiceOver remains deferred, not passed.

## Implemented scope

The Objects Tool tab binds to its own connection and uses the existing owned
DataDocument worker. Catalog and description replies carry request IDs, share the
16 MiB delivery allowance and retain data under the workspace 128 MiB encoded
payload allowance. These are not process RSS claims. Close/cancel use the same
joined runtime as table data; restored tabs remain disconnected. Catalog reads
retain per-kind truncation markers and their existing 10,000 scanned-node,
8 MiB response and 8 KiB text-field limits.

The virtual list searches names, schemas, kinds and exact overload signatures.
Display labels are clipped independently of object identity. Relation opening
carries the catalog tab's connection, schema and full name. Catalog refresh
recomputes visible indexes from the last accepted search, preserving valid
indexes even when a newer search draft is oversized. Selected comments and
truncated groups remain visible. Cluster objects have catalog metadata only;
their administration surfaces are still absent.

Object descriptions share one admitted, cancelled, deadline-bound dedicated
socket path with catalog reads. Supported kinds are schemas, views, materialized
views, sequences, functions, procedures, aggregates and extensions. Identity
includes schema, kind and routine arguments. Unsupported table/foreign-table/
type/domain descriptions explicitly refuse before hydration. Payloads are exact
or refused: 8 KiB metadata/reference fields, 1 MiB definition/body/arguments,
and 8 MiB encoded descriptions. Definition and metadata JSON appear in read-only
native editors with exact copying. Changing views replaces the editor buffer,
so programmatic changes do not retain an invisible undo history. Description
retention includes the active editor representation. DDL and drop impact remain
inactive; description SQL is never dispatched as an action.

Array editing adds one active element editor, 16-item pages, add/remove and
explicit NULL/text controls. It preserves original literal spelling until an
explicit valid edit. Raw mode remains available for unsupported shapes or budget
refusal. Input/output cap at 1 MiB and 4096 elements. A conservative 32 MiB shared
reservation covers escaped retained representations; it releases on closing or
returning to raw mode. Programmatic element changes replace editor buffers to
avoid hidden undo accumulation. Parent table traversal and editor shortcuts now
yield to active IME composition. This is source behavior, not new real IME evidence.

## Verification

- Native debug and release tests: 122 passed, 13 ignored in each. Ignored fixture tests are not passes.
- Native all-target Clippy passes. An initial incorrect GPUI disabled-state API
  produced the archived `native-clippy-initial-errors.txt`; the corrected code
  uses the existing AccessKit synthetic-node disabled state and visible focus.
- The exact ignored description probe passes in `descriptions-live.txt`. It covers
  all eight supported kinds, both function overloads, named procedure arguments,
  bigint extrema without advancing the sequence, unpopulated materialized view,
  the independently verified `pg_catalog.plpgsql` extension, wrong schema/kind/
  overload refusal, idle cancellation, retired/replacement document ownership and
  joined shutdown.
- The only new database objects were under owned `native_objects_20261003` on
  stage03 `127.0.0.1:15432/dbunk_demo`, UUID
  `2283820d-33ec-4c4c-ae03-7051092bd410`. Setup refused an existing schema. Cleanup
  rechecked fixture UUID, schema OID and ownership comment before dropping it.
  The schema is gone and fixture activity is zero; see `fixture-setup.json` and
  `fixture-cleanup.json`.
- `pnpm format`, `pnpm lint` and `pnpm typecheck` pass.
- `just fmt`, `just lint` and serialized `just test` pass (675 core and 692
  Tauri tests). Isolated backend checks pass (769 passed/77 ignored; Tauri subset
  61 passed/6 ignored; two compile-fail doctests). Custom-protocol build passes.
- Native debug/release and fixture-harness Clippy, builds and dependency proof
  pass. The package build verifies one pinned GPUI revision and no Tauri/Wry/
  Tao/WebKit packages. No dependency changes were made.
- Package: `/private/tmp/dbunk-native-package-20261003-catalog-array/dbunk Native Preflight.app`.
  Executable SHA256: `531722961979d0522cf6862dbe8303ac51769c52b3a2f2ccd7d92a29d776b1a7`.
  All 286 recorded Rust/Cargo source hashes matched after packaging. See
  `package-identity.json`. Later source work is not covered by this frozen variant.

## Acceptance still open

The automation inventory still lists running apps without window identities.
Earlier native/System Settings discovery returned `cgWindowNotFound`; this
iteration did not launch another native process or alter OS settings. The pending
availability question remains unanswered. See [window discovery evidence](../tool-tabs-window-20261003.md).

The approved layouts are unchanged. History/saved queries, copy formats, browse
controls, virtual keys, EXPLAIN, catalog and structured array controls still need
their own actual-window scenarios and broader keyboard/AX/real IME verification.
Prior scoped Pinyin evidence remains limited to its named binaries/controls.
Full PostgreSQL parity, profile/package compatibility and the combined acceptance
workload remain incomplete.
