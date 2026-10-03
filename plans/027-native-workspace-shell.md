# Plan 027: Native PostgreSQL workspace shell

- Verification scope updated by Imran on 2026-10-03: VoiceOver is deferred and is not a blocking gate. Keyboard/AX and real IME checks remain required; see [the scope decision](./evidence/027/accessibility-scope-20261003.md).

- Priority: P1. Effort: XL. Risk: HIGH at credentials, restoration and tab lifecycle.
- Migration stage 04, PostgreSQL slice. macOS Apple Silicon only.
- Planned against `3f987c96640d6738b48ae1113ceac3ebbdc8563f`, 2026-10-02.
- Depends on Plan 026 DONE and the closed stage 01 gate. Plans 024/025 retain separate review status.
- Status: IN PROGRESS in [README.md](./README.md). Step 1 now includes typed credential/connection/draft services, durable Keychain recovery and serialized native admission, with injected-store and real-fixture headless evidence ([progress](./evidence/027/step01-services-progress.md)). The scoped real OS Keychain CLI gate passes; packaged stage04 gates remain pending. A, Persistent Navigator, selected by Imran on 2026-10-02; implementation authorized on 2026-10-02. See the [corrected packaging preflight](./evidence/027/step00-review.md). Steps 2–4 now have native implementation and focused tests in progress; Step 5 actual-window acceptance is underway.
- Decision artifact: [plan and three static shell options](./mocks/native-workspace/index.html).
- Sources: migration stage 04 in `plans/gpui-migration.html`; stage 01 remaining-work budget; ADR-0007, ADR-0024, ADR-0025 and ADR-0032; `CONTEXT.md` workspace vocabulary.

Read this plan completely. Preserve the existing Plan 026 closure changes. Imran selected A, Persistent Navigator, on 2026-10-02; the shell-selection gate is satisfied. Keep the three existing query layouts inside that shell. Do not expose raw managers, hydrate secrets in GPUI, upgrade the pinned dependencies, migrate a daily-driver profile, or deploy a build. Development and verification use explicitly owned profiles, fixture endpoints and a separate credential namespace only. No commit, push or PR is requested here.

## Outcome

Open a separate native development app, configure or unlock credential storage, manage PostgreSQL fixture connections, work in several query tabs, quit, and reopen the same isolated profile with SQL drafts and workspace geometry intact. Reopening restores documents in a disconnected state; it does not restore backend sessions, results or transactions and never replays SQL. Loading, locked, failed and empty states must remain distinguishable.

This is the next useful workspace milestone. It is not full application parity or permission to replace Tauri. Stage 04's PostgreSQL shell requirements are covered here; engine-specific forms and the wider navigator remain in stage 06. Stage 05 owns Table Browse, mutations and complete PostgreSQL editor/grid/transaction UX.

## Scope and decisions

Included: one window; native menus and discoverable keyboard commands; compact connection Navigator; PostgreSQL create/edit/duplicate/delete, folders and favorites; explicit connect/disconnect; query-tab create/rename/reorder/pin/close; credential onboarding/unlock/mode change/reset; bounded draft persistence; layout, density and Navigator width; error dialogs and focus return; redacted diagnostics; an isolated unsigned app-bundle launch probe.

Connection forms cover direct PostgreSQL host/port/database/user/password, environment/read-only/Safe Mode settings, existing TLS modes/certificate paths and supported driver options. Use existing validation; a native certificate-file picker is required. Test connection and connect are explicit actions, never validation side effects. Schema/object browsing, SSH/bastion setup, managed servers, other engines, query history/saved-query UI, full command palette/Open Anything, general themes, bound-parameter activation, signed distribution, updater and profile migration are deferred. Preserve unsupported stored metadata and identify unsupported connections instead of rewriting them. Do not treat a connection-only Navigator as schema-browser parity.

Selected shell: A, Persistent Navigator, chosen by Imran on 2026-10-02. Connections remain visible in the Navigator; query documents use the Object tab strip. B and C remain unselected design alternatives in the review artifact. Implement one shell, retaining Stacked, Side by side and Results first from Plan 026.

Proposed development limits: 16 open query documents, four live sessions, two concurrent executions, 128 MiB total retained encoded result data and 16 MiB total queued encoded events. Per-session 64-envelope/8 MiB mailbox and per-execution 48 MiB retention limits remain. These are explicit native admission limits, not backend changes or final parity claims; core limits remain seven sessions per connection and 24 total. Stop for review if fixture measurements require different limits. UI must explain a refusal and offer an explicit close/disconnect or result-clear action. Do not evict documents, transactions or results silently.

## Findings from the current implementation

- `backend.rs` exposes a fixture-only profile and one fixed connection ID. `controller.rs` and `workbench.rs` hold one active session/editor. Adding tabs requires ownership and lifecycle work, not copying windows.
- Connection operations already live in the host-neutral `connections` service. Credential settings and UI-state adapters still live in `commands/settings.rs`; extract only those operations needed here, preserving Tauri command names and JSON.
- `keychain.rs` hardcodes service `dbunk`, account `connection-credentials`, and a process-wide cache. `credentials.rs` also has global key/password caches; reset currently reaches Keychain regardless of the selected mode. A new profile path alone cannot establish isolation.
- Keychain read errors/corrupt blobs currently become an empty map. The new shell must not report this as a successfully loaded empty store. Distinguish NoEntry from denial/corruption before onboarding or destructive actions.
- React persistence uses `ui.v1.session`, drops SQL to fit its budget, and starts after restoration. Reuse the ordering and race tests, not silent shedding. Native persistence needs explicit bounded draft state and a failure surface.
- Stage 01 required a 2–3 person-day packaging probe before stage 04. The Plan 026 CLI launch is not that proof; complete the probe before broad shell implementation.

## Architecture and invariants

### Profile and credential boundary

Keep `open_fixture` and its existing guard unchanged. Add a separate marked stage 04 development-profile constructor with an exclusive lock, canonical private path, versioned marker, allowed fixture identities and an app-generated credential namespace. It must refuse foreign or symlinked profiles before any credential access. One profile per native process; no hot profile switching. Synthetic profile variants run in separate processes unless caches have proven profile ownership.

Thread the credential context through every read/write/reset/change-mode/cleanup path, including any shared cache. Do not add an unscoped global environment override. The default Tauri context keeps its existing production service/account. The isolated context derives a unique service/account from its validated marker and never falls back to the production entry. An injected recording credential store must prove every path is scoped before an OS Keychain test runs. Reset or fixture cleanup can remove only the owned entry. Plain/encrypted SQLite startup must not touch Keychain merely to clean it up.

Credential services own onboarding, unlock, migration and reset, with the existing global/connection fences and success ordering. Onboarding completes only after durable configuration. Wrong password, locked Keychain, denial, corruption, failed mode conversion and interrupted save remain explicit and retryable. Mode changes preserve the prior working store on failure; reset requires confirmation describing password loss while preserving connection metadata and drafts. Preserve blank edit-password means keep existing secret. Never return stored passwords to the UI or include them in logs, snapshots or persistence.

A strict isolated Keychain read path must propagate errors. If implementing it changes the ordinary Tauri failure policy, stop, document the ADR-0005 conflict and obtain an explicit correctness-fix scope decision. Default Tauri behavior must not change accidentally during extraction.

### Shell, documents and runtime

Use a Workspace entity owning query documents; each document owns its editor, result model and optional session. Share one backend and one Tokio runtime. Register one current window owner; use distinct stable tab IDs and session IDs under it. Closing a tab closes only its session and joins its work. Retire the window owner only on window replacement/shutdown, not on tab switching or reconnecting one tab. Preserve connection-generation fences and existing invalidation observers. Connection edits/deletion, disconnect, credential changes and global shutdown must settle every affected tab without stale replies reviving it.

Opening or restoring a document does not allocate a socket. Explicit connect opens a session for the selected document, within admission limits. Two sessions on the same connection remain transaction-independent. Tabs that are not selected continue consuming/ACKing streams and receive truthful status changes. A fair workspace drain budget is at most eight envelopes or about 2 ms total per UI turn; do not multiply it by tab count. Shared byte permits cover all queues and retained models. Keep independent failure notification, cumulative ACK coalescing, terminal-ACK readiness, and bounded control paths. Window focus is a window property; changing tabs must not fake background leases. Use one owned heartbeat task for the window.

Switching tabs or pane layout preserves each editor's in-memory selection, undo, focus target and result scroll anchors. Native menus expose new/close/next/previous tab, connections, settings and quit with displayed shortcuts. Preserve Cmd-Enter, Cmd-Shift-Enter, Cmd-Period, F6 and F8 where their current context applies. Forms and dialogs require labels, validation announcements, secure password inputs, keyboard traversal and deterministic Escape/submit focus return. New UI uses true black, white primary text, existing density metrics, separators and no continuous repaint animation.

### Durable restoration

Use a typed versioned native workspace snapshot under a separate `ui.v1.native.*` namespace; do not overwrite the React session key. Store IDs, connection binding, SQL draft text, active tab, order/pinning, primary selection/caret, layout, density and Navigator geometry. No passwords, result rows, actor IDs, transaction state or running flags. Undo history survives tab switches but is not persisted. Keep SQL drafts plaintext like the current app, and state that credential encryption does not encrypt query text.

Initial snapshot budget: 448 KiB encoded, below the existing 512 KiB value limit. Coalesce saves through one owned writer, debounce 500 ms and serialize revisions. Only mark Saved after the corresponding SQLite commit; failed older writes cannot overwrite newer drafts. Above the limit, retain the last valid durable snapshot and all current in-memory text, show Drafts not saved, and prevent an ordinary close from silently discarding it. Offer retry, export affected SQL through a native save dialog, or explicitly confirmed discard. No silent truncation, no per-keystroke task backlog.

Load settings/credentials and connection metadata before restoration; start persistence only after the load outcome is known. Corrupt or unsupported-version snapshots remain untouched and show a recoverable error, with explicit reset/export. A missing connection leaves the SQL document recoverable and disconnected. Geometry/caret values are validated and clamped. A crash can lose edits since the last acknowledged save; test and document that window instead of claiming crash-proof autosave.

Quit first resolves any unsaved/failed draft persistence without blocking the event loop. If saving fails, keep the window open unless the user explicitly chooses discard. Once quit is admitted, stop new work and use one shared three-second graceful/five-second total runtime cleanup deadline, not five seconds per tab. Join/abort-and-join all session/driver/monitor/writer tasks and close SQLite before runtime termination. A timeout is a failed cleanup gate. OS forced termination is not a successful quit.

## Implementation sequence and gates

### Step 0: Design and packaging preflight (2–3 person-days)

Shell A is selected. The [packaging preflight passes on 2026-10-02](./evidence/027/step00-review.md). Its contract: build an isolated unsigned `.app` from the pinned release binary with embedded fonts/grammar, correct resources and separate bundle identity. Launch from a working directory outside the repo against a marked fixture profile. Record size, resource resolution, AX identity and clean quit. Inspect the existing signing/update approach and cost the stage 07 integration; do not sign, publish or change the updater. Stop if the bundle cannot run or the pinned graph must change.

### Step 1: Scoped backend services and credentials (5–8 person-days)

Extract typed settings/credential/workspace operations and adapt existing Tauri commands. Add the isolated credential context and strict error handling, preserve service-level safety and secret redaction, then expose the narrow facade. Verify all three credential modes with injected stores and separate-process profile isolation before touching a unique disposable OS Keychain entry. Gate: denied/corrupt reads never look empty; failed conversion/reset is truthful; no production keychain call; both backend feature configurations and compile-fail bypass checks pass.

### Step 2: Shell and connection workflow (5–8 person-days)

Implement the selected shell and reusable form/dialog/focus primitives. Wire credentials, connection CRUD/organization, TLS/options, explicit test/connect/disconnect and shell settings. Use two owned fixture connection records with distinct IDs. Add a dedicated owned TLS fixture for the direct TLS validation matrix, never reuse an unidentified listener. Gate: cold onboarding, wrong-password retry, successful reopen, failed connection save and service safety refusal work through the actual window. Measure keyboard/AX forms and dialogs; VoiceOver is deferred as follow-up work.

### Step 3: Multiple document/session lifecycle (4–6 person-days)

Split workspace ownership from query-document state, add admission counters/shared byte budgets and fair background draining. Preserve existing query behavior and layouts. Gate: two simultaneous streams can be cancelled independently; switch/close/reconnect/edit/delete races cannot cross tabs or connections; background credit drains; global quit returns PostgreSQL activity to baseline within one cleanup deadline. Saturation must not starve Stop or another tab.

### Step 4: Persistence and recovery (3–5 person-days)

Implement validated snapshots, the single writer, revision ordering and close-save handling. Gate: fresh/reopen/corrupt/unsupported/deleted-connection cases, SQLite failure/retry, oversized Unicode SQL, rapid edits during writes, quit during pending save and forced termination all preserve their documented outcomes. Reopen restores exact acknowledged SQL and geometry without connection or query side effects. Export/discard confirmation is keyboard and AX accessible.

### Step 5: Combined acceptance (4–6 person-days)

Drive the real release window through configure/unlock → connection management → two tabs → concurrent SQL → switch/stop → edit draft → quit/reopen → explicit reconnect → close. Repeat the critical close/reconnect/mode-change races three times. Verify real IME composition in connection fields and SQL, recording the actual input method. VoiceOver for new forms/dialogs is deferred. Run release AX-active typing/idle/scroll diagnostics with one and four sessions; compare like-for-like with Plan 026 and investigate regressions. Verify peak queue/model/process memory and settle after at least 20 open/run/close cycles. No claim of parity from screenshots or headless tests.

Estimate: 23–36 person-days, sequential planning allowance, not a delivery promise. Includes stage 01's applicable form/control allowance; do not add it again. Credential failure-policy conflicts or pinned packaging failures require re-estimation before continuing.

## Verification contract

Each implemented step updates the README status with its gate evidence. Run `pnpm format`, `pnpm lint`, `pnpm typecheck`, `pnpm test`, `just fmt`, `just lint`, and `RUST_TEST_THREADS=1 just test` (documented credential-test workaround). Run `just check-native`, the custom-protocol Tauri build, and native dependency proof. Keep focused safety/credential/persistence tests in both relevant backend configurations and add the new noninteractive checks to macOS CI. Do not run interactive AX/Keychain tests on an unattended CI account.

Retain Plan 026's owned-fixture live, release AX and window-race coverage while adapting exact identity guards for the new profile/bundle. Deliver documented fixture-only stage 04 setup, launch, reopen, E2E and cleanup commands; existing stage 03 commands remain valid. All cleanup checks include observers and all opened sessions. No test may contact arbitrary saved endpoints. Launcher-supplied verification manifests allow only owned loopback fixture identities; backend admission rejects endpoints outside that manifest, including TLS tests. Never enable arbitrary real endpoints just to demonstrate a form.

Store commands, toolchain/OS, commit and dirty-tree hashes, build variant, fixture/profile identity, test counts, AX/screenshots, recorded human results, memory/queue peaks, cleanup durations and database counts under `plans/evidence/027/`. Include the failure matrix above and distinguish injected/headless, actual-window and human evidence. Redact passwords and credential blobs. Verify the packaged app's isolated Keychain behavior independently from the CLI process; production signing/keychain compatibility remains stage 07.

## STOP conditions

Stop for unresolved keychain identity/cache cross-contamination, secret exposure, incompatible storage migration, safety bypass, unintended Tauri/wire changes, missing profile/fixture ownership, unbounded retained work, draft loss disguised as success, failed AX geometry/focus, unjoined cleanup or dependency/license changes. A layout choice authorizes the design only; implementation needs the subsequent instruction. READY FOR REVIEW requires all scoped gates. DONE requires reviewed committed evidence and its completion SHA.
