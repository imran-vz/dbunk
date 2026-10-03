# Plan 027: Native services and full PostgreSQL target

2026-10-02, uncommitted working tree based on `3f987c9`. Plan 027 remains
**IN PROGRESS**. The previous profile/credential work is preserved. No commit,
push, release, production connection or daily-driver profile access.

Imran clarified the requested outcome as **complete PostgreSQL parity with the
Tauri app**, rather than only the fixture query workflow. The current
[scope and three table-review mocks](../../mocks/native-postgres-parity/index.html)
record that distinction. Plan 028 now defines the stage05 data-workflow milestone;
Imran selected table-review A, Bottom review, on 2026-10-02. Shell A from Plan 027 remains
selected. Later PostgreSQL tooling and compatibility are still required.

## Implemented in this continuation

- Typed native connection list/create/edit/duplicate/delete, organization,
  disconnect and explicit connection test. Passwords are separate inputs and
  never appear in response DTOs. Blank edit-password retains the stored secret.
- Atomic SQLite metadata/credential writes. Native Keychain CRUD uses a separate
  namespace-scoped rollback entry and a secret-free SQLite phase record. Failed
  and interrupted writes distinguish rollback from committed cleanup.
- Native lifecycle recovery for all three credential modes. Failed transitions
  preserve the authoritative source; postcommit cleanup failure surfaces
  `NeedsRecovery`. Reads, hydration and mutation fail closed until explicit
  recovery. SQLite-only startup/configuration makes no Keychain calls.
- Typed 16-document, 448 KiB workspace snapshots with SQL, connection bindings,
  order/pinning, primary selection and geometry. Commit revisions prevent stale
  writers, including pre-reset writers, from replacing newer state. Oversize,
  corrupt and unsupported snapshots are preserved; export/reset are explicit.
  SQL remains plaintext independently of credential encryption.
- A native admission gate spans stored metadata lookup, credential hydration and
  completed socket startup, and serializes native connection/credential fences.
  This prevents an older pending open becoming live after save/delete/reset.
  Queued work rechecks shutdown after admission. ACK, cancel and heartbeat stay
  outside this gate. Settings snapshots hold the credential mutation lock.
- One-shot probes settle child driver joins before returning, including timeout
  and post-connect errors; global shutdown retains those joins if the caller is
  cancelled. Disconnect waits for an admitted bounded probe. Synthetic protocol
  tests include forced shutdown after the actual driver starts.
- Strict native parsing flags malformed or unknown stored TLS/driver options as
  unsupported. Listing preserves them; edits, tests and session opens refuse
  them without rewriting or logging the original values. The Tauri decoder and
  legacy Keychain failure policy retain their existing behavior.

The native facade serializes startup and metadata/credential operations. This
is a deliberate correctness choice; a slow bounded connection test can delay
another metadata operation. Future native APIs that create replacement sockets
must use the same gate. This does not change the underlying Tauri manager fence
model or establish multi-tab UI lifecycle behavior.

## Focused evidence

Injected stores cover all mode transitions, denial/corruption, wrong-password
retry, interrupted reset, failed SQLite commit, every Keychain CRUD journal
phase, missing backup, postcommit cleanup failure and successful recovery.
Secrets are kept in the owned Keychain rollback entry, never in the SQLite
transition journal. These injected tests made no OS Keychain calls. A later
[disposable OS CLI probe](./os-keychain-cli-18e8a1e5/environment.json) passed save,
independent reopen, conversion and reset with exact scoped identities; both
owned entries were absent after cleanup. Packaged GPUI acceptance is separate.

Snapshot tests cover exact Unicode text/order/binding restoration, UTF-8-safe
selection clamping, encoded JSON limits, corrupt/future records, failed save and
retry, concurrent compare-and-swap, reset fencing and FULL-WAL disk reopen.
Admission tests use subprocess-isolated private profiles and deterministic gates;
probe tests own their synthetic listeners. Neither contacts arbitrary endpoints.

`just test-native-workspace /private/tmp/dbunk-native-workspace-services-20261002`
passed against owned fixture instance `2283820d-33ec-4c4c-ae03-7051092bd410`.
The first process created two saved connections, explicitly probed both and ran
two independent session IDs under one window owner, then saved Unicode drafts
and selected encrypted SQLite. The second rejected a wrong password, unlocked,
and restored exact acknowledged drafts/geometry without any open/test/execute
call. PostgreSQL activity returned **0 → 0** after both processes. The marked
profile remains for inspection. This is headless public-facade evidence, not
stage04 window acceptance. [Transcript](./step01-services/workspace-probe.txt).

The final release stage03 GPUI window passed its real keyboard/AX workflow:
exact results, Unicode editing/geometry, errors, cancellation, layouts, socket
loss/reconnect, terminal ACK and window close during a query. Close-to-termination
was **61.187 ms**, exit zero and PostgreSQL activity **0 → 0**. The temporary
profile was removed after successful cleanup.
[AX transcript](./step01-services/cli-e2e/accessibility.txt),
[teardown](./step01-services/cli-e2e/teardown.json).

## Verification

Final logs are in `step01-services/`; source hashes and environment are recorded
there. Compilation fixes and independent review findings were resolved before
the final runs. The initial custom-protocol command used the wrong feature name;
the corrected command is `cargo build --manifest-path src-tauri/Cargo.toml
--features tauri/custom-protocol`.

| Check | Result |
| --- | --- |
| `pnpm format`, `pnpm lint`, `pnpm typecheck` | Pass |
| `pnpm test` | 130 files / 1,488 tests pass |
| `just fmt`, `just lint` | Pass |
| `RUST_TEST_THREADS=1 just test` | Core 660 pass / 70 ignored; Tauri 684 pass / 85 ignored |
| Isolated backend core | 693 pass / 70 ignored; two compile-fail doctests pass |
| Tauri + isolated backend facade | 30 tests pass |
| `RUST_TEST_THREADS=1 just check-native` | Debug/release each 30 pass / 9 ignored; Clippy/build/dependency proof pass; 12 Python tests pass |
| Custom-protocol Tauri build | Pass |
| Public-facade two-process Postgres probe | Pass |
| Final release actual-window E2E | Pass |
| Final owned-fixture live suite | Pass, nine live tests |
| Isolated app-bundle build | Pass, 113,098,476 bytes; no stage04 packaged acceptance claimed |

Existing `block 0.1.6` future-compatibility and debug linker `__eh_frame` size
warnings remain. Remote CI has not run for this uncommitted tree.

## Remaining gates

The disposable real OS Keychain CLI check passed. Step 1 still needs packaged
stage04 identity/recovery acceptance. The following UI continuation is in
progress and is not covered by this first-wave source hash set.

Plan 027 Steps 2–5 remain: owned TLS fixture, approved shell and accessible
forms/menus, interactive multi-tab lifecycle/shared budgets, single debounced
snapshot writer and close-save UI, combined window acceptance, human VoiceOver
and IME. No new stage04 window or full PostgreSQL parity is claimed.

Plan 028 adds transaction controls, Table Browse, typed value editing and
identity-safe mutation review. Full parity additionally requires the PostgreSQL
catalog/DDL/designer, SQL tools/history, schema maps, backup/restore, transfers,
schema comparison, SSH/bastions/managed connections, administration and
profile/package compatibility. Table-review A is selected. The backend wave recorded here preceded the
ongoing native UI implementation.
