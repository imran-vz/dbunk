# Plan 027, Step 1 stage04 profile and settings continuation

2026-10-02, working tree based on `3f987c9`. Step 1 remains **in progress**.
This follows the [credential-context continuation](./step01-context-progress.md).
No commit, PR, deployment, daily-driver launch or real OS Keychain experiment.

## Implemented

The opt-in backend now exposes separate `create_development` and
`open_development` constructors. Creation requires a new canonical private
directory. Rust generates independent profile and credential-namespace UUIDs.
The versioned marker binds these to the canonical path and the launcher-verified
fixture instance. A read-only database check proves the same identity before
migrations or credential construction. Files must be private, user-owned,
regular and not hard-linked; symlinks, unknown entries and copied/mismatched
profiles are refused. An exclusive file lock prevents another process opening
the profile. A process cannot switch stage04 profiles.

Creation failures retain the newly created directory for inspection. There is
no adoption of unmarked directories or automatic repair of interrupted profile
initialization. The existing stage03 guard and its accepted profile shape are
unchanged. Backend construction shares only the existing manager/monitor setup.

The generated namespace selects a strict, lazy Keychain store with no production
fallback. Plain/encrypted SQLite lifecycle paths construct no OS entries. Native
Keychain mode is still unavailable: the public storage-mode enum does not offer
it, persisted Keychain mode is rejected, and settings checks reject unsupported
modes before reading a store. The injected denial test records only the exact
namespace derived from the validated marker and proves failures remain retryable.

A typed, redacted settings facade supports SQLite onboarding, unlock, mode
change and explicitly confirmed password reset. It uses the existing bounded,
owned backend calls and service fences. Corrupt flags, inconsistent settings or
populated unconfigured storage produce errors; onboarding cannot silently erase
those credentials. Explicit reset preserves connection metadata and drafts.
Native pools use SQLite FULL synchronous commits; Tauri retains NORMAL.

Session admission checks stored endpoints against the manifest before hydration
or sockets. This manifest version supports only the existing owned, non-TLS
stage03 fixture at `127.0.0.1:15432/dbunk_demo`, user `dbunk`. Foreign endpoints,
transport overrides and SSH routes are refused without rewriting their metadata.
General connection editing/listing and the owned TLS fixture remain subsequent
work.

Headless setup is documented in `apps/native/README.md`:

```sh
just native-profile-create /private/tmp/dbunk-native-stage04-my-profile
just native-profile-check /private/tmp/dbunk-native-stage04-my-profile
```

The Python helper verifies the live fixture, writes a private temporary manifest,
and calls the isolated Rust example. It never supplies a credential namespace
or password and removes only its temporary manifest directory. The profile
persists. The GPUI launcher and app-bundle probe still accept stage03 profiles.

## Focused evidence

Six tests run in both isolated backend configurations. Valid profile variants
run in separate child processes with an injected recording Keychain builder.
They cover lifecycle/reset preservation, cross-process exclusion and encrypted
reopen/unlock, foreign marker/database/path refusal, manifest validation, lazy
namespace selection, corrupt-state refusal/recovery, and session admission
before credentials or network access. The denied-endpoint test owns its listener;
it never contacts an arbitrary saved endpoint.

The corrupt-state test failed against the first version of the new facade,
which treated an invalid onboarding flag as fresh state. It now passes after
strict metadata validation and preservation checks. [Red result](./step01-profile/corruption-red.txt).
This was found and fixed before exposing the new facade in the native window.

## Verification

All required checks pass; logs are under `step01-profile/`.

| Check | Result |
| --- | --- |
| `pnpm format`, `pnpm lint`, `pnpm typecheck` | Pass |
| `pnpm test` | 130 files, 1,488 tests pass |
| `just fmt`, `just lint` | Pass, both backend configurations |
| `RUST_TEST_THREADS=1 just test` | Core 651 pass / 70 ignored; Tauri 675 pass / 85 ignored |
| Final isolated core check | 666 pass / 70 ignored; two compile-fail doctests pass |
| Final isolated backend with Tauri | 15 facade tests pass |
| `RUST_TEST_THREADS=1 just check-native` | Debug/release each 30 pass / nine live tests ignored; Clippy/build pass; 12 Python tests pass |
| Custom-protocol Tauri build | Pass |
| Native dependency proof | Exact GPUI revision `506beb34de3f433707b7ebe8d8ad2d80f856af6c`; no Tauri/Wry/Tao/WebKit |
| Real headless profile create/reopen | Pass in separate processes; final fixture backend count zero |

The final admission-boundary test was added during the broader run; the complete
isolated backend checks were repeated afterward in
[`native-backend-final.txt`](./step01-profile/native-backend-final.txt). This is
the final six-test profile suite in both isolated feature configurations.
The existing `block 0.1.6` future-incompatibility and non-fatal debug linker
`__eh_frame` size warnings remain. Release builds and Clippy pass. Remote CI has
not run for this uncommitted work. No AX, packaged-window, live streaming,
window-race or performance suite was rerun for this backend-only profile slice;
earlier results remain tied to their recorded binaries.

The real headless helper created
`/private/tmp/dbunk-native-stage04-profile-check-20261002`, then reopened it in a
second process. Both returned profile ID
`1cd8d4c3-3863-4d4f-85a2-ca08bedf54c6` and `needs-onboarding`.
The owned fixture instance was
`2283820d-33ec-4c4c-ae03-7051092bd410`. The empty profile is retained for inspection;
its pool, lock and monitor were closed. [Create](./step01-profile/profile-create.txt),
[reopen](./step01-profile/profile-reopen.txt).

This is backend/headless evidence. No stage04 window, new AX/VoiceOver/IME result,
OS Keychain prompt behavior or workspace acceptance is claimed.

## Remaining Step 1 gate

Native Keychain configure/change/reset still needs durable recovery across
SQLite and Keychain. A strict scoped store and SQLite transaction do not make
cross-store deletion atomic. All three modes, interrupted transitions and
cleanup need the full injected-store matrix before any disposable OS Keychain
entry is exercised in CLI and packaged variants.

Connection/workspace facade operations and typed bounded workspace snapshots
remain incomplete. The existing fixture manifest must be extended only after a
dedicated owned TLS fixture is available. Native shell, connection forms, tabs,
workspace persistence and combined acceptance remain Steps 2–5. The default
Tauri Keychain read-failure policy has not changed.
