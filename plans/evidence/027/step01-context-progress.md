# Plan 027, Step 1 credential-context continuation

2026-10-02, working tree based on `3f987c9`. Step 1 remains **in progress**.
This continues the [service extraction and fixture correction](./step01-progress.md).
The [stage04 profile continuation](./step01-profile-progress.md) supersedes the
remaining-work list below.
No commit, PR, deployment, daily-driver launch or OS Keychain experiment.

## Implemented

`credentials::Context` owns the SQLite pool, encryption key, password cache,
mutation lock and Keychain store. `AppState` derives its pool from that context.
Connection services, settings, bastion secrets, tunnel setup, connection
diagnosis, managed-server provisioning/rollback/restart and schema-comparison
workers carry the same context explicitly. No credential I/O function selects
an ambient default store. Default Tauri construction alone selects the existing
`dbunk / connection-credentials` entry and its legacy read-error policy.

The stage03 fixture constructs a SQLite-only context whose Keychain adapter
cannot construct any OS entry. Its public constructor and marker/ownership
validation remain unchanged. SQLite-mode writes skip inactive Keychain cleanup.
The existing fresh/reopen child-process recording-builder test covers this path.

SQLite credential-map rewrites now run in a transaction in both backend builds.
For SQLite-only contexts, configure/change/reset also commit the verifier and
settings in that transaction. The key and password cache change after commit.
Configuration refuses an already-onboarded profile, conversion rejects a stale
source mode, and reset retains connection metadata and UI state. Invalid nonce
lengths return an error instead of panicking inside AES-GCM.

Seven focused tests cover:

- Overlapping connection IDs in two profiles, independent keys/caches/locks,
  reset isolation, reopening locked storage and wrong-password retry.
- Failed onboarding with no published verifier, key, mode or completion flag.
- Failed conversion/reset with unchanged durable rows, verifier, mode, key and
  cache; old-password unlock after reopening; successful retry and metadata/
  draft retention.
- Failed inserts after deletion rolling the full credential rewrite back in
  both SQLite modes, without replacing the working cache.
- Identity propagation through hydration, upsert, delete, bastion secrets,
  configure, conversion and reset using a recording strict store. This test
  exercises legacy lifecycle ordering with a test-only identity; it is **not**
  evidence of atomic native Keychain conversion.
- Denied/corrupt reads remaining errors through the credential cache layer,
  followed by a successful retry without revealing synthetic secret content.
- Malformed nonce lengths returning errors without a panic.

The recording store is injected in tests only. There is no environment override,
production fallback or native API for selecting a raw service/account.

## Verification

All required checks pass; logs are under `step01-context/`.

| Check | Result |
| --- | --- |
| `pnpm format`, `pnpm lint`, `pnpm typecheck` | Pass |
| `pnpm test` | 130 files, 1,488 tests pass |
| `just fmt`, `just lint` | Pass, both backend configurations |
| `RUST_TEST_THREADS=1 just test` | Core 651 pass / 70 ignored; Tauri 675 pass / 85 ignored |
| Isolated backend core | 660 pass / 70 ignored; two compile-fail doctests pass |
| Isolated backend with Tauri | Nine facade tests pass |
| `RUST_TEST_THREADS=1 just check-native` | Debug/release each 30 pass / nine live tests ignored; Clippy/build pass; 12 Python tests pass |
| Custom-protocol Tauri build | Pass |
| Dependency proof | Exact GPUI revision `506beb34de3f433707b7ebe8d8ad2d80f856af6c`; no Tauri/Wry/Tao/WebKit |
| Packaged startup/query/quit | Pass, exit zero, PostgreSQL activity 0 → 0 |

The fresh release bundle is at
`/private/tmp/dbunk-native-plan027-context-20261002/dbunk Native Preflight.app`,
with the existing separate preflight identity. It is 113,037,724 bytes. The AX
probe verifies bundle/process identity, cold startup from an external empty
working directory, query completion and Cmd-Q. Shutdown took **175.392 ms**.
The temporary profile and working directory were removed on success; the owned
fixture remains running. [AX transcript](./step01-context/bundle/accessibility.txt),
[teardown](./step01-context/bundle/teardown.json),
[bundle manifest](./step01-context/bundle/bundle.json).

This is an actual-window check of the changed credential hydration path. The
no-Keychain evidence remains the injected child-process test, not the AX probe.
The full nine-test live streaming suite, CLI AX matrix, window-race and
performance suites were not rerun for this context/storage change. Earlier
results remain tied to their recorded binaries. No new visual design, human
VoiceOver, general native onboarding or workspace acceptance is claimed.

The existing `block 0.1.6` future-incompatibility warning and non-fatal debug
linker `__eh_frame` size warning remain. Release Clippy and builds pass. Remote
CI has not run for this uncommitted work.

## Remaining gate

The stage04 marked development-profile constructor, namespace bound to SQLite,
allowed fixture manifest and process ownership are still required. The native
settings/connection facade and typed workspace state remain unexposed.

SQLite-only profiles deliberately refuse Keychain lifecycle transitions before
any mutation. General native Keychain configure/change/reset needs a durable
cross-store recovery protocol: a SQLite transaction cannot atomically delete
an OS Keychain entry. The legacy Tauri lifecycle ordering and read-failure
policy remain unchanged; the scoped-store test does not make that ordering
safe for native use. Separate-process stage04 isolation and packaged/CLI
Keychain checks remain pending. No real Keychain checks are authorized by this
partial gate evidence. Steps 2–5 remain unimplemented.
