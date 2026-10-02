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

`dev-native` verifies the fixture, builds the locked release target, creates a
canonical private temporary directory, writes its `.dbunk-native-stage03`
marker, and passes `--profile <absolute-path>` explicitly. It prints fixture
identity, profile path, PID and diagnostic output location. Startup configures
plain SQLite credentials through backend services. It does not resolve a
normal profile, migrate credentials, or use Keychain.

Every ordinary launch gets a fresh profile. Layout preference survives
reopening the same marked profile; an ordinary successful launcher exit
removes its own disposable profile. On failure the launcher retains its
profile and diagnostics rather than pretending cleanup passed.

Do not point the executable at a daily-driver profile. A manually reused
profile must retain its marker, private permissions and loopback fixture
identity, and must not already be open in another process.

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
basic AX workflow does not establish those separate gates. New VoiceOver
control/error announcements require an explicit human listening check.

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
