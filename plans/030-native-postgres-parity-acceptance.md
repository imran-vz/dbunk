# Plan 030: Native PostgreSQL parity and compatibility acceptance

- Verification scope updated by Imran on 2026-10-03: VoiceOver is deferred and is not a blocking gate. Keyboard/AX and real IME checks remain required; see [the scope decision](./evidence/027/accessibility-scope-20261003.md).

- Migration stage 07, PostgreSQL acceptance/profile/package subset; macOS Apple Silicon.
- Written on 2026-10-02 against `3f987c96640d6738b48ae1113ceac3ebbdc8563f`, with Plan 027's uncommitted implementation/evidence tracked separately.
- Behavioral baseline: Tauri `102568b`; architecture is the approved migration reason and no performance improvement is required.
- Latest profile evidence: [separate general PostgreSQL profile capability](./evidence/030/general-profile-source-checks/README.md); source checks, owned service probe and separate package pass; native-window discovery failed and acceptance remains pending.
- Earlier window limitation: [capture comparison](./evidence/028/pinning-capture-comparison-20261003/README.md) reproduced missing unchanged content on both the pinning package and its predecessor. Scoped AX results remain distinct from visual acceptance; wheel input also reported noWindowsAvailable. The cause is unresolved. Later [owned export/settings window checks](./evidence/029/whole-table-export-source-checks/window/README.md) can capture and interact with their named package; they do not close the earlier wheel/capture cases.
- Status: IN PROGRESS, capability ledger and explicit profile capability. The [action-level ledger](./evidence/030/capability-ledger.md) maps the baseline and existing scoped evidence. Combined acceptance and compatibility remain pending; the ledger is not evidence that parity already exists.
- Depends on the scoped gates in [027](./027-native-workspace-shell.md), [028](./028-native-postgres-data-workflow.md) and [029](./029-native-postgres-tools.md).
- Design: [A, Tool tabs](./mocks/native-postgres-tools/index.html#a), selected by Imran on 2026-10-02. Existing workspace A, table Bottom review A and historical selections remain valid.

## Outcome and exclusions

Demonstrate every PostgreSQL baseline workflow in the separate native release
app, preserve synthetic old-format profiles without losing acknowledged work,
and prove an isolated macOS package can launch, use its owned credentials,
recover and quit outside the repository. Produce a concrete gap/evidence ledger
and a rollback rehearsal suitable for a later cutover decision.

Passing this plan permits the claim **PostgreSQL parity on the verified macOS
configuration**. It does not establish other-engine parity, Windows/Linux or
Intel support, signed/notarized compatibility, an updater, or permission to
replace the daily driver. Do not remove Tauri/React/Monaco/Vite, alter published
releases, migrate a personal profile, select a production Keychain namespace,
or switch build/update channels under this plan. Full migration stage 07 retains
those separate obligations.

## Acceptance ledger, without missing families

Create `plans/evidence/030/capability-ledger.md` before running acceptance. Every
applicable row of the [baseline checklist](./evidence/024/parity-checklist.md)
gets a source reference at `102568b`, current service/view owner, expected
behavior/limit, test scenario, exact artifact and state: not implemented,
implemented/unverified, passed, or blocked. Expand grouped rows into their
observable actions. The 75-item all-engine source checklist is not a count of
75 PostgreSQL features. Mark non-PostgreSQL rows excluded with a reason; never
mark an absent required capability passed by relabelling it unsupported.

| Required family | Complete acceptance obligation and service ownership |
| --- | --- |
| Shell and navigation | Window/titlebar/traffic lights, Activity rail/Navigator/Object tab strip/Results pane/Dock/Status bar, new/rename/pin/reorder/dirty/close tabs, view switching, keyboard shortcuts, menus, notifications, command palette and Open Anything with connection/object/saved-work context. Plan 027 owns workspace state; finish native navigation models from Plan 010 before claiming parity. |
| Preferences and persistence | Baseline theme choices and density, sash/pane/Navigator settings, table preferences, window size/position, relevant app settings, release file logging and native file dialogs. Audit existing settings services and frontend-only `ui.v1.*` storage. Design gate applies to substantial unselected settings UI. No screenshot substitutes for saved/reopened behavior. |
| Credentials and direct connections | Onboarding/unlock, plain/encrypted SQLite and OS Keychain, mode change/reset/recovery, CRUD/duplicate/folders/favorites/colors/recency/health/activity, environment/read-only/Safe Mode, direct TLS/certificates, driver settings and staged diagnosis. Use Plan 027 services plus Plan 029 T14; verify errors never masquerade as Ready or empty storage. |
| Advanced PostgreSQL connections | URI import/secret-free copy, SSH/proxy/bastions/fingerprint changes and managed Docker PostgreSQL lifecycle. Plan 029 T14–T15. Full diagnosis includes observed encryption; successful direct TLS alone does not pass this row. |
| SQL editing and execution | Selection/undo/multi-cursor/wrap/find, highlighting/completion/format/snippets, current/selection/all, errors with Unicode offsets, notices, multiple result sets, partial/truncated outcomes, independent sessions, execution/cancel/ACK/heartbeat and explicit reconnect. Plans 026/028 and Plan 029 T01. Driver-bound parameters/row limits are an explicitly requested native addition; unsupported combinations must be disclosed, never changed to unsafe interpolation. |
| Transactions and results | Mode/isolation/recheck/commit/rollback, failed and unknown states, rows/columns virtualization, sizing/ordering/visibility, selection/copy formats, NULL/empty/binary/large values and JSON/array/geometry editors. Plan 028 plus existing Query Session service. No automatic transaction or SQL replay after restore/reconnect. |
| Table Browse and mutations | Server filter/raw predicate/sort/page/count semantics, preferences/history/presets/query inspection, identity and virtual keys, FK drill-down, insert/update/delete and baseline duplicate/bulk actions where source confirms them, exact DML review, inclusion/exclusion/revert, conflicts, confirmation and unknown outcomes. Plan 028 services. Resolve register items marked unknown from baseline code and a fixture, rather than inventing a delivered feature. |
| Saved SQL, EXPLAIN and result exports | All T01–T04 actions in Plan 029, including history filters/counts, saved queries as drafts, EXPLAIN ANALYZE policy, malformed/partial plans, every baseline export format/settings and saved export configurations. Storage services, query sessions and native file/format adapters. |
| Catalog, Structure and DDL | All T05–T07 actions, supported object kinds, table designer, routines/security, sequence actions, dependencies/drop impact, reviewed grouped apply and relation-oriented DDL export limitations. Shared catalog/object-DDL services, not a raw SQL bypass. |
| Schema map | All T08 metadata/display/layout/persistence/routing/PNG/SVG actions, keyboard/AX access and bounded large-schema behavior. Actual relationship services and native graph model, not the synthetic spike. |
| Jobs, transfers and comparison | All T09–T11 formats/settings, native dialogs, owned sources/destinations, progress/cancel/release, tab-close survival, uncertain-start reconciliation, explicit unknown commit, response leases/ACKs, PG16 comparison coverage and expiry. Tool Job, Transfer Job and Schema Comparison services retain their distinct semantics. |
| Overview, administration and safety | All T12–T13 statistics/details/health/settings/audit actions, sessions/locks/pending transactions, correct cancel/terminate target, maintenance and matview refresh. Every write surface uses stored policy; read-only/Strict refuse before dispatch and confirmed Protected writes audit only according to their service contract. |
| Lifecycle and recovery | Close/reopen, document replacement, concurrent views/jobs, connection/bastion/credential edits, sleep/refocus, socket loss and quit fence admission and retain/join the correct owners. Existing jobs survive setup-view closure where promised. Recovery preserves durable drafts and unfinished user intent without restoring sockets, results or jobs as running. |

Inherited missing capabilities stay missing: debugger, visual query builder,
advanced diagram editing, data comparison, schema migration SQL, broader PG
comparison, new transfer formats/streaming, scheduling and advanced monitoring.
A newly found correctness defect becomes a tracked fix with focused evidence;
"the baseline did it" does not justify unsafe behavior.

## Sequence and deliverables

### 1. Freeze expected behavior and finish shared shell gaps

Populate the ledger from the baseline and Plans 027–029. Inventory native menu
commands, shortcuts, Open Anything targets, settings, theme/density variants,
notifications and release-log behavior. Expose only missing typed storage or
navigation services; keep secrets out of searchable metadata and diagnostics.
Implement uncovered UI only after its design selection. Reuse existing selected
layouts where they apply; tools A is selected. The broad parity request does not
authorize a substantial departure from those selections.

Baseline `src/lib/theme.ts` supports system/light/dark and presets. Imran's
explicit standing native design instruction requires true black with white
primary text, so that appearance takes precedence. Preserve imported theme
settings without deleting or rewriting them; record the native appearance
difference in the ledger. Do not add an approval gate merely to follow this
existing instruction. Any later request for other appearances needs its own
design selection.

Record concrete baseline discrepancies such as history counts or a legacy buffered operation before setting expected outputs. Preserve feature
intent while documenting any deliberate correctness fix. This is a coverage
review, not another open-ended redesign or a reason to expand engine scope.

### 2. Build disposable compatibility profiles

Create fixtures from baseline migrations, storage schemas and synthetic values,
not a copy of a personal profile. Include stable connection/object/tab IDs,
connection order/folders/favorites/colors, TLS/SSH/driver/policy fields, bastions,
managed metadata, history, saved queries, export configurations, map positions,
grid preferences, settings, query/table/object/designer drafts and selections.
Keep unsupported engine records/unknown fields intact without activating them.
Record which items the baseline actually persisted; do not invent restoration
of temporary edits or undo history.

Create one fixture per credential mode plus empty, locked, wrong-passphrase,
missing/denied Keychain, corrupt verifier/record, pending credential recovery,
unsupported future version, near-budget and over-budget cases. Baseline browser
storage leftovers need an explicit inventory and one-time export/import path
where needed; SQLite-only success cannot prove those values were preserved.

Profile preparation may use pure storage/injected stores immediately. Real OS
Keychain tests first name the exact disposable service and primary/backup
accounts and use unique synthetic credentials. Never read ordinary daily-driver
entries. Secret values stay off argv, diagnostics and evidence; a mode reset may
remove only the test's own entries.

### 3. Implement and prove copy-based migration and rollback

Prepare a consistent SQLite snapshot with the backup API or an equivalently
verified checkpoint-and-copy procedure, including WAL content. A plain copy of
an open main database file is insufficient. Retain the immutable source and
checksums. Map baseline state into versioned native keys without overwriting
React keys or changing original IDs. Native-only state stays in its own
namespace; unknown future native data is preserved for export/reset.

Use a new destination with canonical private path, exclusive lock and marker.
An explicit importer validates the synthetic legacy source schema and manifest
read-only, then writes into a destination created with its own native identity.
Do not pass an unmarked legacy database to the ordinary native opener or bypass
its identity checks. Verify destination identity before migrations or credential
access, and keep the source immutable throughout.
Choose one process/profile; refuse concurrent hosts and hot switching. Reopening
must restore exact acknowledged text and valid caret/selection boundaries, tab
order/bindings and persisted settings, while leaving sessions disconnected.
Missing connection records must not silently delete recoverable SQL.

On every failure preserve source and last valid destination state. Exercise
interruption before/after SQLite commit and each cross-store credential step;
reopen in a fresh process and reconcile any journal before returning Ready.
Repeat migration or reconciliation without duplicating IDs/history or losing
secret ownership. Record rollback as launching the previous host against a
separate verified source snapshot after the native process fully exits, never
as opening a possibly upgraded mutable database with both hosts.

Synthetic profile compatibility is within scope. Actual production path and
Keychain identity/signature behavior remains a later release decision. Provide
a compatibility assessment that names the intended production identity and
unverified signing assumptions without opening those entries. Do not mistake a
successful CLI Keychain probe for packaged-app or production-signature evidence.

### 4. Validate an isolated package

The normal native product also needs a profile entry point that can save and
explicitly connect to user-selected PostgreSQL endpoints. Fixture-only
hard-coded endpoint admission is a development boundary, not complete connection
parity. Implement this as a separate, explicit native profile capability with
its own validated identity and scoped credential lifecycle. Preserve the
existing immutable fixture constructors and manifests unchanged. Never turn a
fixture profile into a general profile on reopen or fall back to the Tauri
daily-driver path/Keychain entry. A fresh normal native profile and a copy-based
import must share the same service policy and lifecycle guarantees.

Verify that capability using only newly created disposable profiles populated
with owned fixture endpoints. Tests must establish that save/restore alone opens
no sockets, invalid endpoints fail truthfully, and Test/connect are deliberate
actions. Implementing general endpoint support does not authorize this agent to
contact a live database, read personal credentials or launch a daily-driver
profile. Record the explicit launch path and capability distinction so the
delivered app is usable beyond the test fixture without weakening test guards.

Build a separately identified macOS arm64 `.app` and local DMG using the pinned
release graph. Include fonts/SQL grammar/icons/resources, bundle metadata,
licenses, file dialogs, geometry and secret-safe release logging. Preserve the
existing isolated launcher/profile capability; do not broaden endpoint admission
or let ordinary app defaults select a user's profile during package testing.

Use a new named package/profile and exact executable/resource hashes. Launch
from outside the repository and from the mounted local DMG/copy-to-disposable-
Applications location. Verify keyboard/AX identity, dialogs, read-only-volume
behavior, first-run failure, reopen, actual PostgreSQL query and joined quit.
Run owned credentials through the packaged executable in every mode, including
Keychain access refusal/recovery and final owned-entry cleanup. The unsigned or
ad-hoc development package is identified as such; signing/notarization and
updater metadata are not silently introduced to make a check pass.

Rehearse retaining the previous package and source profile snapshot, then a
failed-start rollback on disposable paths. Package preparation and local tests
are reversible; publication, release-channel changes and daily-driver migration
remain separately authorized actions after this plan, not implicit final steps.

### 5. Run the combined release acceptance matrix

Use owned fixtures with immutable endpoint/profile/process/container identities
and manifests. Name new listeners, schemas, restore targets, output directories,
containers and disposable Keychain entries before touching them. Reuse Plan 029's
PG16 comparison, direct TLS, SSH/proxy and managed-container guards. No arbitrary
saved endpoint is eligible because it happens to be reachable.

Run representative complete workflows for every ledger row with exact expected
results. Include two independent transactions, browsing and mutation conflict,
reviewed partial DDL, EXPLAIN ANALYZE refusal, file import/export, restore,
comparison reads, admin target validation and advanced connection recovery.
Run the critical reconnect/close/credential-change/job races three times, using
barriers or observed protocol states rather than sleeps as race proof. Add
focused service/model tests for new findings, not endless broad smoke repeats.

Measure release startup, key-to-frame latency, scrolling, process-tree footprint
and idle CPU with the existing external harness on matched hardware/display,
power, thermal state, window geometry, AX state and workloads. Retain raw samples
and discard foreground-interrupted runs. The stage 00 architecture decision
waives a required 30% improvement; the stated no-metric-over-10%-worse-than-Tauri
criterion remains. Revalidate calibration/noise if it cannot resolve that margin.
Do not substitute a Plan 026-versus-027 diagnostic or differently powered run for
this Tauri comparison. Investigate failures or seek an explicit changed criterion;
never redefine a threshold after measuring.

For one and four sessions, heavy results/maps/comparison/jobs and at least 20
open/run/close or release cycles, record current and peak queue/model/process
memory, worker/admission counts and settle traces. Per-document encoded rows,
aggregate retained bytes and physical footprint are different measurements.
Where aggregate accounting is not observable, add a narrow test/debug observation
or leave that gate pending; do not derive a claimed peak from screenshots.
Confirm no idle continuous frame requests and no orphan subprocesses or sockets.

Run automated AX/keyboard tests and real IME composition through SQL, cells,
forms, dialogs, Navigator and the new tools. Record tester/date/build, actual
input method, actions, expected result and actual result. Agent-driven real
composition is acceptable under Imran’s instruction to run the checks. Prior
spike or shell acceptance does not cover new controls. VoiceOver is explicitly
deferred by Imran on 2026-10-03; it is not a pass and does not block acceptance.

### 6. Review the evidence and state the supported boundary

Reconcile every ledger row and unresolved defect. No mandatory PostgreSQL row
may remain implemented/unverified or blocked for full PostgreSQL acceptance.
Keep non-PostgreSQL and later distribution requirements visibly excluded.
Produce a completion review naming platform, package identity, fixture versions,
profile formats, inherited limits, measured outcomes and rollback result.
READY FOR REVIEW requires the scoped gates, including keyboard/AX and real IME acceptance. DONE
requires reviewed committed evidence and a completion SHA. Do not mark the full
migration or retirement of Tauri complete from this PostgreSQL subset.

## Verification and evidence discipline

During implementation run `pnpm format`, `pnpm lint`, `pnpm typecheck`, relevant
`pnpm test`, `just fmt`, `just lint`, `just test`, native debug/release
Clippy/tests/build, custom-protocol Tauri build and independent core dependency
proof. Keep both hosts' meaningful tests and gates until separately authorized
retirement. Add stable noninteractive profile/package tests to macOS CI; document
interactive tests and required host permissions without pretending they ran in CI.

Store evidence under `plans/evidence/030/`: capability ledger; commands and
OS/toolchains; commit and dirty/source hashes; exact binaries/package/resources;
fixture and profile identities; redacted migration manifests; source/destination
checksums; credential failure matrix; runtime/window/human results; performance
raw samples and comparisons; memory/worker/queue peaks; and timed cleanup counts.
Record a source change after a build as a different variant. A passing test of a
previous binary does not verify a later recovery fix.

Stop for source-profile modification without a safe snapshot, foreign resource
access, a policy bypass, secret leakage, false-success/unknown-write collapse,
unsupported destructive downgrade, non-reproducible cleanup, a failed gate or
missing design approval. Preserve the failing artifacts and continue independent
preparation where possible. No production access or daily-driver changes are
needed to produce a concrete, reviewable acceptance result.

Latest actual-window evidence: [auto-fit package and disconnected reopen](./evidence/028/auto-fit-window-20261003/README.md). Scoped formatting, table geometry and library/administration checks ran; full acceptance remains open. The restored-library budget failure is tracked in the [activation fix](./evidence/029/library-activation-source-checks/README.md). VoiceOver remains deferred.

Native administration cancel/terminate now has an immutable captured-target
review, stored-policy confirmation, exact-save dispatch barrier and version-8
read-only recovery. Its owned stage03 backend probe passed with activity 0 → 0;
[source and verification scope](./evidence/029/admin-control-source-checks/README.md)
records the PostgreSQL signal-identity race. Required/native/backend checks and
the isolated package pass; [scoped window checks](./evidence/029/admin-control-window-20261003/README.md)
pass cancel, policy confirmation/termination, staged reopen and explicitly
injected unknown recovery, with normal quit and activity 0 → 0.
Full parity remains IN PROGRESS; VoiceOver stays deferred.
