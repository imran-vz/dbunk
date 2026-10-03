# Read-only Administration Tool tab, 2026-10-03

Status: implemented; required source checks, scoped live probe and isolated package pass. This increment does not complete
PostgreSQL administration or full native parity. No new actual-window,
keyboard/AX or real IME pass is recorded. VoiceOver remains deferred, not passed.

## Native behavior and ownership

The approved Tool tabs shell now opens a connection-bound Administration document.
New/restored tabs begin disconnected. Connect admits the existing owned data
worker; Refresh explicitly collects one bounded capture. There is no polling.
Sessions, locks and pending transactions have virtual rows, retained selected-row
inspection and copy, keyboard section/row navigation, and detail scrolling.

Refresh has monotonically increasing request identities. Cancellation discards a
late successful reply and preserves the previous capture; reconnect does not
reuse request identities. Disconnect marks retained readings stale. Close and
connection invalidation use the existing document fence and joined cleanup.
An oversized/refused capture leaves the earlier capture available.

The facade reuses `Backend::object_read` and its stored connection, credentials,
fixture admission, dedicated socket, cancellation and lifecycle. The legacy
process-global SQLx administration functions are not called by this tab. The
reader retains the existing 30-second operation deadline, statement/lock limits
and joined cleanup behavior.

## Meaning and bounds

The capture records its database, reader PID and UTC collection interval. It is
not an atomic historical snapshot of changing activity. Sessions and pending
transactions cover the current database and NULL-database activity. Lock rows
include current-database activity and unattributed holders; another database's
relation OID is never resolved into a current-database relation name. The blocked
lock count is explicitly cluster-wide. Pending transactions are queried
independently of the capped session list.

NULL, restricted, unavailable and numeric zero remain distinct. Restricted
activity warns that transactions may be hidden; an empty visible section is not
presented as proof of an empty server. Prepared-transaction locks can have no PID.
Missing blocker information is distinct from an empty blocker list.

Each section admits 200 rows and observes one extra row to disclose truncation.
Queries admit 500 Unicode characters with explicit clipping, blocker lists 64
PIDs with explicit clipping, and other text fields 2 KiB. Aggregate encoded
captures above 1 MiB are refused. The native model also validates vector/string
capacities against a 1 MiB owned-heap limit before acquiring a 2 MiB shared lease
for capture and derived labels/details. Row labels are bounded and details admit
64 KiB. Existing workspace 128 MiB retention and 16 MiB delivery budgets remain;
these are not process RSS limits.

## Verification and remaining scope

Six backend boundary tests and six native model tests pass. They cover exact
large integers, missing metrics, Unicode clipping, observed overflow, escaped
byte limits, blocker identity, retained-capture refusal and stale/cancelled reply
identity. Verification so far:

- Frontend format/lint/typecheck pass.
- `just fmt`, `just lint` and serialized `just test` pass: core 677 passed/71
  ignored; Tauri 694 passed/85 ignored.
- Isolated backend Clippy/tests pass: 819 passed/81 ignored and two doc tests;
  Tauri facade selection passes 81 tests/10 ignored.
- Native format, debug/release all-target Clippy, fixture-harness Clippy, debug
  build and debug/release tests pass: 163 passed/13 ignored in each test run.
- The exact opted-in stage03 administration probe passes actual reader queries,
  reader/database identity, NULL-aware metrics, bounds, collection interval,
  idle-cancel reuse, retired-document refusal and joined cleanup back to the
  prior fixture activity count. It creates no objects and does not signal a
  selected server session. In-flight cancellation/contended-lock/restricted-role
  behavior is not established by this live probe.
- Custom-protocol compatibility build, package and dependency proof pass.
  Bundle: `/private/tmp/dbunk-native-package-20261003-admin/dbunk Native Preflight.app`.
  Executable SHA256 `e8d77d51e7c0910c3f65eb148349c702e10e59b6e63f43f087e879fd3420d57d`;
  bundle 122,852,972 bytes. All 324 frozen source hashes matched after packaging.
  Ignored tests are not passes.

The [owned package window attempt](../admin-window-20261003/failed-discovery.json)
used PID 68741 and intended profile `/private/tmp/dbunk-native-admin-20261003-review`.
CUA reported the app running but exact-path and bundle-ID lookup both returned
`cgWindowNotFound`. No feature interaction, keyboard/AX or IME check was performed.
The process was stopped with SIGTERM only after matching its exact executable,
profile arguments and SHA256. This is not a normal-quit pass. Fixture backend
count was zero after cleanup. A separate Finder availability probe also failed;
these observations do not establish a systemwide outage or diagnose its cause.

Cancel/terminate backend actions, maintenance, overview/server settings and safety
audit are still absent. They require their own stored-policy and observed-target
contracts. The current Cancel read button only cancels this document's metadata
operation; it does not act on a selected server session.
