# Native PostgreSQL workflow

This separate macOS development target uses Rust 1.98.1 and Zed revision
`506beb34de3f433707b7ebe8d8ad2d80f856af6c`. It links the host-neutral backend
with default features disabled. The existing Tauri application and stage01
spike keep their own launch paths.

## Run

Prerequisites: macOS Apple Silicon, Xcode command-line tools, Rust 1.98.1
with rustfmt/clippy and Python 3. Fixture setup prefers an already running
Docker daemon with Compose (`postgres:17.6`); it never starts Docker itself.
On Apple Silicon macOS, if Docker is unavailable, it instead fetches the
checksum-pinned official PostgreSQL 17.11 source archive and builds it into
`tools/native/.state/runtime/installed`. This requires Python 3.12+, Xcode
command-line tools and Make. It does not run `brew install`, create a default
cluster, or register/start a system service. The fixture build omits optional
ICU, readline and compression integrations and uses plain loopback transport.
All executable/data/share paths remain inside this task's private directories.
Other platforms need the owned Docker fixture.

From the repository root:

```sh
just native-fixture-up
just dev-native
# Close the native window when finished.
just native-fixture-down
```

With Docker, the fixture helper creates project `dbunk-native-stage03`, binds
only `127.0.0.1:15432`, and stores PostgreSQL data in container tmpfs. The database
is `dbunk_demo`, with disposable development credentials `dbunk` / `dbunk`.
The local fallback uses the same port/database/credentials and keeps all
cluster files under a UUID-marked private directory. Before connecting or
stopping it, the helper checks its exact PostgreSQL executable, PID,
postmaster data path and matching ownership marker.
Both variants load the existing many/wide/large views and `plan026.exact_values`,
`plan026.policy_probe` and an instance-specific sentinel.

Setup refuses an occupied port or existing resources without its matching
ownership record. It never adopts another fixture or invokes the broad
`db:down` command. Cleanup validates the container's Compose identity and
instance label before removing that container and its empty, labelled network.

`dev-native` runs the workspace launcher described under
[Persistent stage04 workspace](#persistent-stage04-workspace) on a fresh path
under a new private temporary directory, so every run opens the redesigned
window (sidebar, tab bar, status bar and environment frame) against the owned
fixture. The profile is retained after quit and its path is printed.

The legacy stage03 single-query window is still available as
`just dev-native-stage03`. It passes `--profile <absolute-path>` on a fresh
`.dbunk-native-stage03` profile, uses plain SQLite credentials, and removes
the profile after a successful exit. `just test-native-e2e` drives that
window.

## Default launch

Launched without arguments (`just run-native`, or opening the app bundle), the
app opens its own general profile at
`~/Library/Application Support/dbunk Native/profile` in the workspace window.
The first launch creates it; later launches only open it, so a damaged
profile is reported rather than replaced. Nothing is imported from the
removed Tauri app's data. Agents must not use this launch: it is the user's
daily-driver profile.

Do not point the executable at a daily-driver profile. A manually reused
profile must retain its marker, private permissions and loopback fixture
identity, and must not already be open in another process.

## Isolated app-bundle preflight (Plan 027, Step 0)

Rebuild bundles made before the Plan 027 Step 1 fixture-seeding correction.
That correction removes a call to ordinary credential cleanup that could
delete the normal app's Keychain entry during fresh fixture initialization.
The fresh/reopen no-Keychain guard is now tested with a recording builder in
a separate process. See `plans/evidence/027/step01-progress.md` for the
correction to the earlier isolation claim.

```sh
# Build only; no fixture, window, Keychain access or signing command.
just package-native tools/native/.state/bundle-RUN_ID

# Actual-window probe; keep the desktop available for keyboard/AX automation.
just native-fixture-up
just test-native-bundle /private/tmp/dbunk-native-bundle-RUN_ID
```

Each output directory must be new and canonical, with no symlink ancestors.
The packager builds the locked release graph and copies it into
`dbunk Native Preflight.app`, with identity
`codes.imran.dbunk.native.stage04.preflight`, a plist and the dbunk icon.
Fonts, themes, keymaps and the SQL grammar are embedded in the executable.
It preserves the linker's ad-hoc signature; it does not sign or notarize the
bundle, install into Applications, publish a build, or change the updater.
The output includes file hashes, size, linked-library and signature diagnostics.

The verification command checks fixture ownership, creates a fresh stage03
profile, and launches the bundle executable with an explicit `--profile` from
an empty temporary working directory outside the checkout. The AX probe checks
the bundle ID, executable, PID and profile before cold startup, a query and
Cmd-Q. Success requires exit zero, released queue bytes and PostgreSQL activity
back at baseline. The successful profile and temporary working directory are
removed; failures retain their marked profile and diagnostics. The bundle and
evidence stay in the named output directory for inspection. Fixture shutdown
remains the separate `just native-fixture-down` command.

This is the packaging prerequisite for the workspace shell. It uses the
unchanged stage03 `open_fixture` guard and plain SQLite credentials. The separate
stage04 profile, credential namespace, onboarding, tabs and draft restoration
use the workspace launcher described below. No Keychain behavior, Finder launch, signing compatibility,
human VoiceOver or full shell acceptance is established by this probe.

## Persistent stage04 workspace

The separate stage04 window provides the selected Navigator, PostgreSQL connection
forms, credential setup, query tabs and durable SQL drafts. Its launcher creates
or reopens a persistent marked profile, with a private verified fixture manifest.
It does not resolve the default app profile. Do not pass a stage04 directory to
`dev-native-stage03` or the old stage03 bundle probe. `just dev-native` runs this
launcher without a path, on a fresh temporary profile.

```sh
just native-fixture-up
just dev-native-workspace /private/tmp/dbunk-native-workspace-my-profile
# Quit, then run the same command to restore acknowledged drafts, disconnected.
```

The parent directory must exist and the path must be canonical. A fresh path is
created by the backend; an existing one must have its matching stage04 marker.
The launcher retains the profile after quit, records the executable hash, starts
outside the checkout, and requires fixture activity to return to its baseline.
For an already built release binary, use `python3 tools/native/workspace_launch.py
/private/tmp/dbunk-native-workspace-my-profile --no-build`. For a marked isolated
bundle, use the same script with `--bundle /absolute/path/to/the.app`.

The window limits documents to 16, open sessions to four and executions to two.
Restoration never opens a socket or replays SQL. Each tab owns its editor/undo
and results; switching tabs preserves them. The workspace drains background tabs
fairly, shares 16 MiB of event queues and 128 MiB of retained results, and joins
session work on close. Use the native menus for displayed tab shortcuts.

Under the connection list, the Navigator shows the selected connection's
schemas and objects. Load objects opens a transient data lane, reads one bounded
catalog and closes it; the tree is a retained capture until the next explicit
load. Cmd-Shift-O focuses its filter; arrows, Home/End and type-ahead move,
Right/Left expand and collapse, and Return opens relations as table documents or
describes other objects in Objects. Cmd-K opens Open Anything over commands,
tabs, connections and the Navigator capture (`>` restricts to commands); inside
the SQL editor it resolves after the editor's chord timeout. Ctrl-` toggles the
console Dock, which never opens on its own. Cmd-F, Cmd-G and Cmd-Shift-G find
in the SQL editor. The Window menu has Minimize, Zoom and Full Screen. A normal
quit saves window geometry; logs go to `~/Library/Logs/dbunk Native/`.

Draft saves debounce for 500 ms and show Saved only after the matching commit.
If a save fails or exceeds the encoded budget, keep the window open and retry,
export all current SQL to a new file, or explicitly confirm discard. Forced OS
termination can lose edits since the last acknowledged save. SQL drafts remain
plaintext even when credentials use encrypted SQLite or Keychain. Mixed query
and table drafts export as JSON so row identity, original values, NULLs and
filters survive export.

The newer Plan 028 source adds transaction controls, bound parameters and row
limits, plus table paging/filtering/sorting and staged insert/update/delete.
The bottom review uses backend SQL previews and policy confirmation. Apply
waits for the exact committed recovery snapshot; a lost response preserves an
unknown outcome and never retries automatically. Drafts can be included,
excluded or removed offline, but applying needs fresh analysis and review.
Table drafts are separately bounded to 128 changes and 4 MiB encoded intent
per document. An oversized workspace save blocks apply and offers export.
These controls are not present in the frozen Plan 027 human-check package and
still need actual-window acceptance. Specialized editors, preferences and the
remaining PostgreSQL tools are tracked in Plans 028–030.

Profile-only preparation remains available without opening a window:

```sh
# Parent directory must already exist and be canonical. Target must be new.
just native-profile-create /private/tmp/dbunk-native-stage04-my-profile
# Reopen the same profile in a new process, after re-verifying fixture ownership.
just native-profile-check /private/tmp/dbunk-native-stage04-my-profile
```

Both commands first verify the existing owned `dbunk-native-stage03` fixture.
Rust generates the profile and credential namespace, creates private files,
binds its marker to SQLite, and retains an exclusive lock while open. Checks
refuse copied/mismatched profiles, symlinks, hard links, foreign files, changed
fixture instances and profile switching within a process. Creation never adopts
an existing directory. Failed creation retains its directory for inspection;
it is not automatically retried as an empty profile.

The profile result contains only profile ID, credential mode and readiness;
the launcher also reports the verified fixture identity. These commands take no
password arguments, open no database session and perform no OS
Keychain operation. The private fixture-manifest temporary directory is removed
after the command. The profile persists. Keep it separate from daily-driver
profiles; do not delete an unknown or changed directory to retry creation.

The backend supports all three credential modes, explicit recovery of interrupted
Keychain operations, unlock, mode change and confirmed password reset. Native
Keychain failure/recovery coverage uses injected stores, and a disposable real
OS CLI probe has passed. [Packaged stage04 checks](../../plans/evidence/027/workspace-package-20261002/agent-verification.md)
also passed onboarding, secret-preserving blank edits, separate-process reopen,
encrypted conversion/unlock and confirmed reset, with both scoped Keychain
entries absent after cleanup. SQLite-only paths make no Keychain calls. Native connection metadata/password changes are atomic in SQLite modes;
Keychain changes use a scoped rollback entry and a durable recovery journal.
Blank edit passwords retain the stored secret. Unsupported or malformed stored
TLS/driver options remain visible and cannot be rewritten or opened.

Typed workspace snapshots retain at most 16 documents and 448 KiB of encoded
state. Saves require the loaded revision, preventing late writes from replacing
newer drafts. Oversized, corrupt and unsupported snapshots are never truncated
or silently replaced. Export/reset are explicit operations. SQL drafts are
plaintext regardless of credential encryption.

Run the new public service workflow against the owned fixture:

```sh
just test-native-workspace /private/tmp/dbunk-native-stage04-workspace-RUN_ID
```

The target must be new. Separate processes create and reopen the profile. The
first saves two connections, explicitly tests them, runs two distinct query
sessions, saves Unicode drafts and switches to encrypted SQLite. The second
verifies wrong-password rejection, unlock and exact restored drafts/geometry
without opening sessions or replaying SQL. The helper verifies fixture ownership
and PostgreSQL activity returning to baseline after each process. It retains the
profile for inspection; it never removes a profile on failure. The disposable
probe passphrase is fixed in the example and is not intended for personal data.

The command above is a backend/headless check. Actual stage04 window acceptance
uses `tools/measure/workspace-accessibility.swift` with the launcher's
`identity.json`; it verifies exact process/profile/executable identity before
keyboard or accessibility actions. It requires an unlocked macOS session.
[Recovery window checks](../../plans/evidence/027/workspace-recovery-20261002/agent-verification.md)
passed corrupt/future snapshot quit without rewriting, explicit reset, and
oversize save failure with exact SQL export and confirmed discard.
[Workspace performance diagnostics](../../plans/evidence/027/workspace-performance-b/summary.json)
completed one/four-session typing and scrolling, 20 open/run/close cycles and
joined teardown. These are measured diagnostics, not a universal performance
pass; the capture predates the recovery fixes and used AC power while the
Plan 026 comparison used battery power.

Plan 027 remains in progress. Imran deferred VoiceOver from the blocking gate
on 2026-10-03. Real IME composition remains pending, along with the remaining window race/recovery cases in the
[workspace progress record](../../plans/evidence/027/step02-workspace-progress.md).
The frozen preflight package for human acceptance is identified by its
[package hashes](../../plans/evidence/027/workspace-package-20261002/package.json)
and [source manifest](../../plans/evidence/027/workspace-package-20261002/source-manifest.json).
It includes the recovery fixes and predates subsequent Plan 028 backend work;
its window evidence does not validate that newer source. Full PostgreSQL
parity is still being implemented.

### Separate owned TLS fixture

```sh
just native-tls-fixture-up
python3 tools/native/tls_fixture.py matrix
python3 tools/native/workspace_launch.py /private/tmp/dbunk-native-tls-my-profile --tls
# After closing every workspace using it:
just native-tls-fixture-down
```

TLS setup uses a separate checksum-pinned PostgreSQL 17.11 SSL build, private
certificates and `127.0.0.1:15433/dbunk_tls_demo`. It does not modify the plain
fixture, install certificates into system trust, or adopt an existing listener.
`tls_fixture.py check` reports its verified paths and identity. Use the reported
CA path with Verify full in the connection form. The disposable credentials are
`dbunk` / `dbunk`. Client-certificate material is fixture-owned too.

TLS is an explicit profile opt-in. An existing plain-only profile cannot be
reopened with a wider manifest; use a separate new path. The launcher verifies
both fixture identities and records both activity baselines for TLS profiles.
Native connection tests classify certificate trust, hostname and local-material
failures without logging passwords or raw driver errors.

## Query and keyboard controls

Run / Cmd-Enter executes the selection or statement at the caret; Run script /
Cmd-Shift-Enter executes the buffer. Stop / Cmd-Period cancels the current
execution. These shortcuts work while the SQL editor has focus. Result values remain read-only.
The connection service enforces stored policy; Safe Mode refusal has no
confirmation override in this host. Reconnect opens a fresh session without
replaying SQL.

Stacked, Side by side and Results first change only pane geometry. They share
the same editor, execution and results. F6 / Shift-F6 move between SQL and
results. Tab indents SQL. F8 reaches the first enabled query control; Tab and
Shift-Tab move through controls, including layout radios and result tabs.
Enter activates a control, and Escape returns to the previous content pane.

Query errors remain above results, separate from notices. A reported source
position underlines the SQL token; hover over it for the compact PostgreSQL
message and code. At that location, Cmd-K followed by Cmd-I opens the same
hover and Escape dismisses it. The persistent error surface also exposes the
full message to VoiceOver and provides Return to SQL. Errors announce once
per failed execution without stealing focus. Editing SQL clears its old
marker; errors without a usable position remain above results.

## Verification

```sh
just fmt-native
just lint-native
just test-native
just check-native
just check-all
just test-native-live
just test-native-e2e
```

`check-native` covers debug and release Clippy/tests/build, the pinned graph,
and the prohibition on Tauri/Wry/Tao/WebKit dependencies. `check-all` adds both
backend configurations and is a macOS aggregate. Run frontend requirements
separately: `pnpm format`, `pnpm lint`, `pnpm typecheck`, and `pnpm test`.
The macOS CI job runs native compile/test checks; Linux CI retains backend and
frontend checks.

`test-native-live` verifies fixture ownership before running ignored native
Host/service tests. These exercise PostgreSQL through the native runtime but
are not UI E2E evidence.

The E2E command requires the owned running fixture and macOS Accessibility
permission for the probe. It launches the actual GPUI window using a new
profile and drives keyboard/AX actions through
`tools/measure/editor-accessibility.swift --native-fixture PID PROFILE`.
The probe verifies executable/PID/marker identity before sending input. It
retains the spike's Unicode, selection, undo, text geometry, wrap, resize and
scroll assertions; the native branch checks read-only results and toolbar
focus instead of the spike's editable cell probe. It executes a fixture query
and checks exact AX/copy values (NULL, empty text, bigint, decimal, Unicode and
quotes), a SQL error and recovery, cancellation and layout changes, externally terminates only its own identified fixture
query socket, reconnects explicitly, then closes the window during an active
query. The launcher requires a zero exit and PostgreSQL activity returning
to the pre-launch count. Forced process cleanup on probe failure is never
reported as a successful native shutdown.

Logs and teardown evidence are written under
`tools/native/.state/evidence/<profile-id>/`. Evidence is not automatically
promoted to plan completion. Queue saturation, slow consumption, stale
executions, idle/active connection loss, focus leases, delayed opens, all
closure races and performance measurements need the focused tests and additional evidence listed in Plan 026. A successful
basic AX workflow does not establish those separate gates. VoiceOver control/error listening checks are deferred follow-up work under
Imran’s 2026-10-03 scope decision; they are not counted as passed.

Run the combined window races serially, with no other UI driver:

```sh
just test-native-window-races tools/native/.state/window-races-RUN_ID
just measure-native tools/native/.state/performance-RUN_ID
```

Use a new output directory for each run. The race runner builds the opt-in
`fixture-verification` feature, then repeats nine actual-window scenarios three
times. It holds an admitted open's caller reply, pauses consumption at four
row batches, saturates a three-slot queue, and replaces the actual root view.
It checks Stop before resuming drain, new-query recovery, close and Cmd-Q,
zero queue bytes after release, zero exit status and the original PostgreSQL
backend count. The stalled TCP handshake is covered separately by the backend
test. Hooks and their keyboard bindings are absent from ordinary builds.

The performance runner rebuilds without hooks, loads real many/wide/large
results, activates accessibility, measures three 300-key typing runs using the
shared 2,000-line SQL document, 30-second idle CPU/footprint and three scroll
runs per direction. Keep the fixture window unobscured and leave the pointer
alone during scrolling. These diagnostics do not require a daily-driver launch.

## Release performance diagnostics

For an already launched release host with its matching marked profile, compile
the existing AX probe and prepare real retained data:

```sh
swiftc tools/measure/editor-accessibility.swift -o /tmp/dbunk-native-ax
/tmp/dbunk-native-ax --native-fixture PID ABSOLUTE_PROFILE --prepare-metrics=many
# Other fixed fixtures: --prepare-metrics=wide or --prepare-metrics=large
```

The helper preserves the same PID/executable/profile identity checks, activates
accessibility, executes the selected fixture through the editor's keyboard
shortcut, and waits for completion, a visible result and terminal ACK. Only
`many`, `wide` and `large` are accepted; plain `--prepare-metrics` selects many.
It prints the observed result label and any AX-exposed omission indicators.
The source fixtures contain 10,000 / 10,000 / 400 rows respectively; limits
may reduce retention, so capture actual counts and visible diagnostics. It
leaves the native window open with SQL focused, ready for the measurement
harness's footprint, typing, idle and result-scroll captures. It does not run
the ordinary editing roundtrip or close the window. Keep the launcher running
and close that window after measurements so its exit/baseline checks still run.
Preparing metrics is not a substitute for the full E2E probe.

## Failure recovery

If Docker is unavailable, the supported local fallback creates a private
runtime/cluster without resuming unrelated containers. On unsupported systems,
start an appropriate isolated Docker environment outside this helper. A
missing runtime or compiler is not a reason to contact another endpoint.
Local cleanup verifies the owned server before requesting a fast shutdown,
then removes only its marked cluster directory. The reusable private runtime
archive/binaries remain beneath `.state/runtime`.

If fixture creation fails after writing ownership state, use
`just native-fixture-down` to remove only matching resources, then retry setup.
If labels differ, stop and investigate; do not erase the state file or force
an unrelated container down. Diagnostics name only this fixture and profile.
A failed application launch retains its marked temporary profile; remove it
only after confirming the launched process has exited and the profile marker
still identifies that launch.

## Explicit general PostgreSQL profiles

The separate native app also accepts `--create-native-profile ABSOLUTE_PATH` for
a new private profile and `--native-profile ABSOLUTE_PATH` to reopen it. These
modes have a distinct marker/SQLite identity and support user-selected direct
PostgreSQL endpoints. They never convert fixture profiles; only a launch
without arguments selects the [default profile](#default-launch). Save/restore does not connect; Test and Connect are explicit.
SSH and other engines remain unsupported in this entry point. Existing fixture
launch modes are unchanged. No production identity or daily-driver cutover is
approved by this capability.

Agent verification must still use the owned launcher, named disposable profiles
and owned fixture endpoints. `tools/native/workspace_launch.py` adds
`--create-general-profile` for a new path, and `--general-profile-owner RECEIPT`
for a later reopen of that exact owned profile. The receipt is written in the
launch evidence directory with its creation intent. It establishes ownership,
not successful application initialization or window acceptance. The normal app's
endpoint capability is broader than this verification wrapper's allowed targets.

The headless service probe uses `tools/native/workspace_probe.py PATH
--general-profile` with a new disposable path. It checks owned fixture connections,
two query sessions and encrypted disconnected restoration through the general
constructor. Consult [current evidence](../../plans/evidence/030/general-profile-source-checks/README.md)
for which checks have actually passed.
