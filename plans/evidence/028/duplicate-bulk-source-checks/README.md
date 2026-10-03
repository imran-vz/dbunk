# Duplicate rows, bulk edits and SQL snippets, 2026-10-03

Status: required checks, native debug/release checks, separate package and dependency proof pass. This increment
follows the frozen [export/impact source](../../029/export-impact-source-checks/README.md).
No actual-window, keyboard/AX or real IME pass is recorded for these controls.
VoiceOver remains deferred, not passed.

## Row actions

Duplicate row copies original loaded values into the existing insert JSON editor.
Generated and non-writable columns are excluded. Writable keys, serial values and
BY DEFAULT identity values remain exact, matching baseline `102568b`; removing a
property requests its default. Insertable tables do not require an update key.
Staged overlays do not change the copied source.

Bulk edit assigns one literal value or SQL NULL to 1–128 selected retained rows.
The selected source column can be changed among writable columns. It uses the
existing raw, JSON, array and spatial literal editors and Bottom review layout.
The exact page and analysis are captured; stale, omitted or truncated source pages
are refused. The operation prepares one candidate draft and publishes only when
every row passes identity, writability and size checks. Failure leaves the draft
unchanged. Original keys/guards, other staged cells and inclusion choices survive;
assigning an original value removes that cell's staged edit. Changed counts
include reversions and exclude unchanged rows.

Independent review found missing shared-budget ownership for draft working copies
and unbounded editor history. Fixes include moving recovered intent into its sole
owner before cloning table preferences, admission for working copies, visible
recovery retry and bounded composition-aware raw/array editor history. Draft/recovery
retention counts actual owned capacities. A 32 MiB working allowance is admitted
before candidate/editor allocation, then released to actual draft retention when
the editor closes. The array editor retains its separate 32 MiB allowance. Current
literal text caps at 1 MiB. Undo history refreshes at a conservative 4 MiB edit-cost
bound when unmarked; marked input is refused at 8 MiB. Refusal restores the last
committed literal visibly. These post-event guards bound retained state/history,
not the transient peak allocation from an enormous paste or total process RSS.

## Workspace snapshots

An 8 MiB persistence working allowance is admitted before query/table results.
Borrowed durable-draft measurement and SQL byte lengths reject a workspace that
already exceeds the 448 KiB storage limit before value cloning. The writer still
checks the exact encoded envelope. Preflight refusal advances the writer revision,
so an older save acknowledgement cannot release a refused apply barrier. The
editor owns current SQL; workspace metadata no longer retains its original copy.

Workspace export may exceed the SQLite limit. It admits a separate conservative
snapshot allowance before cloning (four times measured payload plus 4 MiB), then
keeps that allowance through picker wait and file-job completion. SQL, mixed JSON
and raw-recovery file writes now use the host's bounded joined file-job owner.
Cancellation is checked before creation; an admitted workspace write may finish
during shutdown, which waits for its job. These existing workspace writers remain
streaming create-new operations, distinct from retained-result atomic publication.

## SQL snippets

The native Query menu offers the baseline Top rows, Grouped count and Recent rows
templates. Each appends to the document, separated by two newlines when nonempty,
through an ordinary undoable editor transaction. It does not execute SQL or replace
the current selection. Active composition and execution refuse insertion. Native
menu discovery, focus, undo and composition acceptance remain pending.

## Verification

- `pnpm format`, `pnpm lint` and `pnpm typecheck` pass for the initial integration.
- `just fmt`, `just lint` and serialized `just test` pass: core 675 passed/71
  ignored; Tauri 692 passed/85 ignored. The 252 backend inputs match the prior
  source manifest, so its isolated facade/custom-protocol checks carry forward.
- Native debug/release tests each pass 149 tests with 13 ignored. Format, debug
  and release all-target Clippy, fixture-harness Clippy and debug build pass.
- Focused coverage includes atomic batch refusal/reverts, original duplication,
  moved recovery refusal/retry, finite history, borrowed snapshot encoding,
  rejected-save revision fencing, snapshot admission and owned file-worker join.
- A Clippy unit-value warning after generalizing file-job results was corrected;
  the initial failed log is preserved separately from final passes.
- Separate package `/private/tmp/dbunk-native-package-20261003-duplicate-bulk/dbunk Native Preflight.app`
  builds successfully; executable SHA256
  `79c9da692a5853115e0d0eb6a2c4a03c0e0ef5af5c1415444113a54ad162e671`.
  Bundle size is 122,460,812 bytes. Dependency proof passes and all 310 recorded
  source hashes match. The package has not been launched.
- No database fixture or daily-driver profile was used for this increment. The
  package destination is separate. The [availability probe](./window-availability.json)
  read Finder Desktop AX but Go to Folder returned `cgWindowNotFound`; no native
  app was launched and no settings changed. This does not establish a system-wide
  desktop outage or a feature acceptance pass.
