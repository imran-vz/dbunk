# Plan 027, Step 0: packaging preflight

2026-10-02. Implementation authorized by Imran. Step 0 passes locally; Steps
1–5 remain unimplemented. No completion, parity, commit or release is claimed.
The pre-existing Plan 026 closure edits are preserved.

**Isolation correction, discovered during Step 1:** the original fresh-profile
seed called `connections::save`, whose inactive-store cleanup could delete the
normal `dbunk / connection-credentials` Keychain entry. The no-Keychain claims
below were incorrect for that binary; the packaging/AX measurements remain
historical observations, not isolation evidence. An injected separate-process
test reproduced the call without OS access. Fixture seeding now writes only
the validated fixture's SQLite records, and the same test passes on fresh open
and reopen. See [Step 1 progress and correction](./step01-progress.md). The
older Step 0 bundles must be rebuilt before use. The real Keychain has not
been inspected to determine whether earlier launches removed an existing entry.

## Delivered

`tools/native/package.py` builds the locked release target, proves the native
dependency graph, and creates a new unsigned development `.app`. New just
recipes expose build-only and foreground AX verification. The macOS CI job
builds the bundle and runs the ownership tests without launching a window.
The existing AX probe accepts an explicitly identified preflight bundle for
only startup, one query and quit; the CLI's repository-derived helper guards
remain intact. Setup and cleanup are documented in `apps/native/README.md`.

The probe uses the unchanged `open_fixture` constructor and a new stage03
marked profile with plain SQLite. This satisfies Step 0's marked-fixture
packaging requirement. Step 1 still owns the new stage04 profile constructor,
fixture manifest and credential namespace. Packaging has not enabled Keychain.

## Actual-window evidence

Command from the repo root:

```sh
just test-native-bundle /private/tmp/dbunk-native-plan027-preflight-20261002
```

- macOS 27.0.1 (26A434), arm64, Rust 1.98.1 native toolchain. Full environment
  and working-tree hashes are under `step00/`.
- Ordinary release binary, no verification hooks, exact pinned Zed revision
  `506beb34de3f433707b7ebe8d8ad2d80f856af6c`. No dependency or licence change.
- Bundle and launch working directory both outside the repository. Bundle
  size **113,550,924 bytes (108.29 MiB)**, including the existing debug info.
  Bundle file hashes and runtime identity: [bundle.json](./step00/bundle/bundle.json)
  and [identity.json](./step00/bundle/identity.json).
- Cocoa/AX bundle ID `codes.imran.dbunk.native.stage04.preflight`, exact bundle
  executable, process arguments, PID and fresh profile marker all match.
- Embedded resources resolve: startup passes `assets::Assets.load_fonts`,
  theme/keymap loading and SQL grammar construction before exposing the SQL
  editor. The pinned assets crate uses compile-time embedding in release;
  `apps/native/src/sql.rs` links the Tree-sitter grammar and highlight query.
  The bundle has no checkout assets or grammar directory. The readable editor,
  caret and query result are verified through AX. This is not a visual font or
  highlighting audit. [AX transcript](./step00/bundle/accessibility.txt).
- `otool -L` lists only system frameworks and `/usr/lib` libraries, with no
  checkout or Homebrew dynamic dependency. The original linker ad-hoc
  signature is preserved, with no TeamIdentifier and no sealed resources.
  No signing command was run. [Libraries](./step00/bundle/linked-libraries.txt),
  [signature inspection](./step00/bundle/signature.txt).
- Exact owned fixture instance `2283820d-33ec-4c4c-ae03-7051092bd410`, checked
  against process/data-directory ownership and its SQL sentinel before launch.
  Cold startup reaches Ready; an explicit query returns `replacement verified`
  and completes terminal ACK.
- Cmd-Q to termination: **173.841 ms**. Exit 0. Queue high-water **914 bytes**,
  released bytes **0**, retained result **93 bytes**. PostgreSQL activity
  **0 → 0**. The successful disposable profile is removed. The pre-existing
  owned fixture remains running. [Cleanup](./step00/bundle/teardown.json).
- The full existing CLI AX workflow also passes after the guard change:
  editor geometry, copy values, error recovery, cancellation, layouts,
  connection loss/reconnect and close during an active query, with exit 0 and
  PostgreSQL **0 → 0**. [Transcript](./step00/cli-e2e/accessibility.txt).

## Verification

All checks in the table exited zero; raw logs are in `step00/`.

| Check | Result |
| --- | --- |
| `pnpm format`, `pnpm lint`, `pnpm typecheck` | Pass |
| `pnpm test` | 130 files, 1,488 tests pass |
| `just fmt`, `just lint` | Pass, both backend feature configurations |
| `RUST_TEST_THREADS=1 just test` | Core 634 pass / 70 ignored; Tauri 658 pass / 85 ignored |
| `RUST_TEST_THREADS=1 just check-native` | Debug and release each 30 pass / 9 live tests ignored; Clippy/build pass |
| Native backend checks within `check-native` | Isolated core 642 pass / 70 ignored, two compile-fail doctests pass; Tauri facade eight pass |
| Python fixture/packaging checks | 12 pass, including two new output-ownership refusal tests |
| `cargo build --manifest-path src-tauri/Cargo.toml --features tauri/custom-protocol` | Pass |
| Native graph proof | Exact GPUI pin, no Tauri/Wry/Tao/WebKit |
| Build-only bundle command | Pass without fixture access or a window |
| `just test-native-bundle …`, `just test-native-e2e` | Actual-window passes with baseline cleanup |

The existing `block 0.1.6` future-incompatibility warning remains. Ignored live
test cases, the 27 window-race repetitions and performance measurements were
not rerun: no native runtime or backend source changed. Remote macOS CI has not
run for this uncommitted work. No screenshot, human VoiceOver, IME, peak process
memory or packaged Keychain evidence is claimed by this step.

## Stage 07 release work

The current `.github/workflows/release.yml` creates an arm64 Tauri DMG on a
version tag, clears inherited Apple signing credentials and uploads a GitHub
prerelease. `src-tauri/tauri.conf.json` uses `codes.imran.dbunk`. The current
manifest/configuration has no updater plugin or update feed. The preflight
does not alter this release path or adopt its identity.

Retain stage 01's **5–8 person-day stage 07 release-integration allowance**:

| Remaining work | Planning allowance |
| --- | --- |
| Versioned native bundle/DMG automation, release metadata, icons, licence notices and CI artifacts | 2–3 days |
| Download/quarantine, clean-machine launch, dialogs/logging and explicit installation/replacement checks | 1–2 days |
| Profile cutover/recovery and CLI-versus-bundle credential identity verification on release artifacts | 2–3 days |

This matches the existing unsigned distribution model. Developer ID signing,
notarization and an automatic updater remain separate release decisions. If
chosen, reserve another **3–5 days** for signing/notarization and credential
access across upgrades, and **4–7 days** for update delivery, integrity checks,
failed-update recovery and rollback tests, to be re-estimated once the release
mechanism and account access are selected. No new dependency or service was
selected in this probe.

## Next gate

Step 1 is scoped backend settings/credentials with a private stage04 profile
and an injected recording credential store. Prove strict denied/corrupt reads,
all mode conversions/reset paths, cache ownership and preservation of the
default Tauri policy before using any disposable OS Keychain entry. Steps 2–5
still cover the selected shell, connections, multi-document lifecycle,
persistence and combined acceptance. The full plan is not ready for review.
