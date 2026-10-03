# Plan 027, Step 1 progress and fixture isolation correction

2026-10-02, working tree based on `3f987c9`. Step 1 is **in progress**, not
gate-complete. No commit, PR, deployment or daily-driver launch. This records
the initial extraction; the [context continuation](./step01-context-progress.md)
supersedes the remaining-work list below.

## Implemented

- Moved app settings, credential lifecycle and UI-state operations into the
  host-neutral `src-tauri/src/settings.rs` service. Tauri commands delegate
  without changing names, argument names, DTO serialization, confirmation
  behavior or global session fences. Existing job-lifecycle tests now target
  the extracted service. Five new service tests run in both backend builds.
  CI uses the documented `RUST_TEST_THREADS=1` workaround in its Rust and
  native jobs while the legacy credential key/password cache remains global.
- Refactored Keychain I/O behind a store that owns its identity, cache and
  read policy. The ordinary Tauri entry and empty-on-read-error policy remain
  the default. The strict primitive distinguishes missing entries from denied,
  corrupt and unavailable reads, does not cache failures, and retains its old
  cache after failed writes/deletes. Strict error strings contain no raw blob
  or OS details. Five recording-store tests cover these behaviors and separate
  identities/caches. This private primitive is not yet wired to a native
  credential context or exposed through a native facade.
- Fixed the fresh stage03 fixture seed so it writes only that fixture's
  SQLite metadata and plaintext credential. `Backend::open_fixture`, its
  accepted profile shape and validation/ownership guard remain unchanged.
  No generic credential cleanup runs during fresh seed initialization.

## Correction to earlier isolation claims

The existing call path was:

```text
Backend::open_fixture
  → profile::open, fresh database
  → connections::save
  → credentials::upsert → write_all(PlainSqlite)
  → clear_inactive_storage
  → keychain::clear_all
  → delete dbunk / connection-credentials
```

The profile constructor avoided `credentials::configure`, but its connection
save still performed cross-backend cleanup. The source hashes recorded in
Step 0 match the committed pre-existing call path; this was not introduced by
the Step 1 Keychain refactor. [Hash proof](./step01/preexisting-call-path.json).

The no-Keychain statements in Step 0 were wrong. Earlier fresh fixture runs
could have removed an existing normal-app Keychain blob; deletion success
and an absent entry both return success. The real Keychain has not been
read or changed to investigate that historical outcome. No conclusion about
whether saved passwords were present or lost is possible from these logs.
The two task-created pre-fix bundles were verified against their original
file hashes and renamed from `.app` to `.app.disabled`, preserving them for
inspection while preventing ordinary app opening. [Retained paths](./step01/disabled-pre-fix-bundles.json).
They must be rebuilt before further use. The historical bundle size/AX/cleanup
measurements are retained with a correction.

The diagnostic test launches a separate child test process with a recording
keyring builder returning only in-memory mock entries. It opens a fresh
marked fixture, hydrates the known fixture credential, shuts down and reopens
the same profile. It asserts **zero entry constructions**, so even a denied
attempt to select a production entry fails the test. No OS Keychain is used.

```sh
RUST_TEST_THREADS=1 cargo test --manifest-path src-tauri/Cargo.toml \
  --no-default-features --features isolated-profile --lib \
  backend::tests::fixture_startup_never_opens_a_keychain_entry -- --exact --nocapture
```

Before the fix, the test failed with the recorded identity
`[("dbunk", "connection-credentials")]`:
[red test](./step01/fixture-keychain-red.txt). Changing only the fresh seed to
direct SQLite initialization makes the same test pass for fresh open and
reopen: [green test](./step01/fixture-keychain-green.txt). The injected child
runs in both isolated backend feature configurations through `check-native`.

## Verification and evidence

Required repository and backend checks pass; raw logs are under `step01/`.

| Check | Result |
| --- | --- |
| `pnpm format`, `pnpm lint`, `pnpm typecheck` | Pass |
| `pnpm test` | 130 files, 1,488 tests pass |
| `just fmt`, `just lint` | Pass, both backend feature configurations |
| `RUST_TEST_THREADS=1 just test` | Core 644 pass / 70 ignored; Tauri 668 pass / 85 ignored |
| Isolated backend core within `check-native` | 653 pass / 70 ignored; two compile-fail doctests pass |
| Isolated backend with Tauri | Nine facade tests pass |
| `RUST_TEST_THREADS=1 just check-native` | Debug/release each 30 pass / nine live tests ignored; Clippy/build pass; 12 Python tests pass |
| Custom-protocol Tauri build | Pass |
| Native graph proof | Exact GPUI pin, no Tauri/Wry/Tao/WebKit |
| Repaired bundle and full CLI AX E2E | Pass, exit zero and PostgreSQL baseline restored |
| `just test-native-live` | All nine owned-fixture tests pass in 299.97 seconds; final PostgreSQL backend count zero |

The repaired bundle passes the actual-window probe:

```sh
just test-native-bundle /private/tmp/dbunk-native-plan027-repaired-20261002
```

The new binary was built from the corrected sources, in the ordinary release
variant without verification hooks. The bundle is 113,070,908 bytes, using the
same pinned graph and separate `codes.imran.dbunk.native.stage04.preflight`
identity. The bundle and working directory are outside the checkout. Cold
startup and the query pass; Cmd-Q takes **174.130 ms**, exits zero and returns
PostgreSQL activity **0 → 0**. Queue high-water is **1,169 bytes**, released
bytes **0**, retained result **93 bytes**. The profile is removed after
success; the pre-existing owned fixture remains running. [AX transcript](./step01/bundle/accessibility.txt),
[cleanup](./step01/bundle/teardown.json), [bundle hashes](./step01/bundle/bundle.json).
The no-Keychain evidence is the separate injected guard test, not an inference
from the successful window launch.

The full CLI AX workflow also passes after rebuilding, including error,
cancellation, layout, connection-loss/reconnect and close-during-query checks.
The live suite additionally checks the 135-second background lease/refocus,
125-second held-ACK timeout, saturated queue/reconnect, idle and active socket
loss, retention refusal, independent terminal ACK, cancellation and joined
shutdown. [Live results](./step01/native-live.txt),
[final backend count](./step01/post-live-backend-count.txt).
No 27-run window-race or performance rerun is claimed for this slice. The
existing `block 0.1.6` future-incompatibility warning remains; the debug build
also emitted a non-fatal linker warning about the `__eh_frame` section size.
The release build and Clippy checks passed. Remote CI has not run for this
uncommitted work.

New service tests cover:

- Exact settings JSON, cold onboarding, successful encrypted configuration,
  locked startup, wrong-password retry and successful unlock.
- Refused configuration and unavailable SQLite remain errors, not Ready.
- Mode-change confirmation and reset preserving connection metadata/drafts.
- Namespaced workspace batches reject invalid keys without partially saving.
- Configure/change/reset wait for an affected job to terminate before changing
  storage, block new admission while waiting and reopen admission afterward.

All new credential-store tests use injected memory stores. No disposable or
normal OS Keychain integration test has run. No new forms, dialogs, human
VoiceOver, IME or full workspace persistence is claimed.

## Remaining Step 1 work

The new stage04 constructor, marker/manifest validation, process ownership,
generated credential namespace and end-to-end credential context are still
required. `credentials.rs` retains its global AES key/password cache and
legacy mode-conversion/reset ordering. Those are **not safe to reuse as-is**
for the native shell. The strict Keychain primitive alone does not establish
profile isolation or failure-safe conversion.

Next, thread a profile-owned context through every credential consumer,
including cleanup, managed-server and tunnel paths; implement durable
configure/change/reset failure handling; prove all three modes with recording
stores and separate-process profile tests; then expose narrow native settings
and connection operations. Only after those checks may a uniquely owned OS
Keychain entry be exercised, independently in the CLI and packaged app.
No change to the Tauri read-failure policy is authorized or implemented here.
