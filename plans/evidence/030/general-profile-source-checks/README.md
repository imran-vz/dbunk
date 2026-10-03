# Explicit general native profiles, 2026-10-03

Status: source, owned service checks and separate package pass. Window discovery
is blocked; native-window acceptance remains pending. This adds a separate
PostgreSQL endpoint capability and does not establish full Plan 030 acceptance.
Profile import, production identity/signing, distribution and native-window
acceptance remain open. VoiceOver is deferred, not passed.

## Identity and endpoint authority

`Backend::create_native_profile` requires a new private canonical directory.
`open_native_profile` requires `.dbunk-native-profile`, version 1 and kind
`general-postgres`, plus an exact matching SQLite `native.profile.identity.v1`
identity before migrations or credential construction. Path and independent
canonical profile/credential UUIDv4s are part of that identity. Foreign, mixed,
corrupt, future or missing identities refuse without adopting or repairing them.
The normal and stage04 constructors share the process profile guard and file
lock. Stage03's existing constructor is unchanged; this is not a claim that all
public constructors now share one universal process guard.

Authority is an explicit enum. The fixture branch still uses its exact immutable
manifest. The general branch admits supported direct PostgreSQL metadata with
current policy, credential recovery, document ownership and startup fences.
Other engines, SSH and unsupported options remain visible but inactive. Stored
port zero retains the baseline effective default of 5432; new typed forms still
refuse port zero.

Credentials reuse the lazy strict Native lifecycle and independently generated
UUID namespace. The existing `dbunk-native-stage04-{UUID}` service spelling is
retained for compatibility; no production or legacy Keychain identity is selected.
Focused tests use injected Keychain adapters; no actual OS Keychain check is claimed.

## Explicit native and verification entry points

The app requires `--create-native-profile PATH` or `--native-profile PATH`.
Creation and opening do not fall back to each other or to a default directory.
Both existing fixture launch modes retain their meanings. General mode refuses
`DBUNK_NATIVE_VERIFY`; the workspace lifecycle restores disconnected state.
New forms use loopback port 5432 and postgres database/user in general mode;
fixture forms retain their fixture defaults. These are editable defaults, not
automatic saves or connections.

The owned verification launcher adds `--create-general-profile` and
`--general-profile-owner RECEIPT`. A fresh profile must be absent. Reopen needs a
private receipt and creation intent matching the exact marker identities and
current owned fixture manifest. Receipts are external to the profile and contain
no credentials. A receipt proves verification ownership, not SQLite readiness or
window acceptance. This wrapper's fixture scope does not limit what endpoints
the general application capability can store.

The existing headless workspace probe now has an explicit `--general-profile`
mode. Its create/reopen processes continue to use only the fixed owned stage03
endpoint and assert the selected backend capability. The owned run passes: two
saved connections, explicit connection probes, query sessions, encrypted credential
storage and exact disconnected draft restoration in a separate process. Both
processes join backend shutdown; PostgreSQL activity returns from zero to zero.
The retained profile is `/private/tmp/dbunk-native-general-profile-probe-20261003`.
See `general-workspace-probe.txt`; no DDL or other endpoint was exercised.

## Connection input and probe cleanup

Optional certificate/key paths are capped at 4,096 UTF-8 bytes and server name at
256, with NUL refusal. Loaded metadata is checked before credential hydration;
save validates structure without DNS, certificate-file reads or connectivity.
Test shares one ten-second deadline for asynchronous connection and graceful
cleanup, then aborts and joins owned drivers before returning a timeout.

Correction to the earlier [reconnaissance](../general-profile-recon.md):
`DedicatedConnection::close` already imposed a two-second driver-join timeout.
The new change removes the independent fresh cleanup allowance after connection.
It does not prove a hard end-to-end wall-clock limit: synchronous TLS material
reads cannot be preempted by a Tokio timeout, and cooperative abort joins finish
after deadline expiry. Those existing I/O/scheduler limits are not hidden by the
new deadline. No general RSS claim is made.

## Verification

Required frontend format/lint/typecheck and Rust fmt/lint/serialized tests pass:
core 677 passed/71 ignored; Tauri 694 passed/85 ignored. Native format,
debug/release all-target Clippy, fixture-harness Clippy, debug build and
debug/release tests pass: 187 passed/13 ignored in each run. The isolated backend
passes 834 tests/82 ignored plus two doctests; Tauri facade passes 90/11 ignored.
The custom-protocol build passes. Ignored tests are not passes.

All 36 Python tooling tests pass, including exact prebuilt probe launch and
fixture recheck tests. Focused backend tests cover marker/SQLite identity,
disconnected restoration, denied Keychain lifecycle, input bounds and cleanup.
Independent review found two issues, both corrected: fixture rechecking after
probe compilation, and duplicate names exceeding the 256-byte service limit.
Duplicate names now preserve UTF-8 boundaries and remain editable/deletable.
The focused logs retain initial failures and the final passing reruns.

All 345 source hashes match after package completion. The package and native
dependency proof pass. These checks do not prove
arbitrary endpoint connectivity, OS Keychain behavior, normal application quit,
keyboard/AX, IME, profile import or full PostgreSQL parity.

## Frozen package and window attempt

Package: `/private/tmp/dbunk-native-package-20261003-general-profile/dbunk Native Preflight.app`.
Executable SHA256: `7103b11501b31c3683e9ce36724bf70a3da5f40fa439aa9957a59833085da337`.
Bundle size: 123,148,540 bytes. See `package-identity.json`, `package.txt` and
`post-package-source-proof.json`.

The guarded create-general-profile launcher created an ownership receipt for
`/private/tmp/dbunk-native-general-profile-20261003-review` and launched PID 5152.
CUA discovery by exact package path and bundle ID returned `-10005 cgWindowNotFound`;
app inventory reported it running. No UI interaction was performed. Exact
executable, hash and profile arguments were checked before SIGTERM cleanup;
the process stopped and fixture activity returned to zero. The profile remains.
This is not normal quit, Ready, keyboard/AX or IME acceptance. See the
[failed discovery](../general-profile-window-20261003/failed-discovery.json) and
[forced cleanup](../general-profile-window-20261003/forced-cleanup.json).
