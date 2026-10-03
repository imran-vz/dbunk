# Native administration signals, 2026-10-03

Cancel query and terminate session now use the Administration Tool tab's captured
row. A backend-owned immutable capture mints a single-use review for its exact
document, PID, backend start, database (including NULL), and query start for
cancellation. Selection changes cannot retarget a review. A lock row resolves
only through matching session identity in the same capture; a blocker PID alone
cannot authorize a signal. The reader itself and unavailable identities refuse.

The native review displays that identity and the effect of termination. Stored
policy is rechecked before credential hydration or socket creation. Read-only
policy permits cancellation and refuses termination; configured confirmation is
bound to the same attempt and target. Control admission is separate from schema
write admission. A parameterized PostgreSQL statement rechecks observed identity
before calling the signal function. PostgreSQL cannot lock backend lifetime or
query changes through signal delivery: success means signal sent, not proof the
query or session stopped. This remaining race is disclosed in the review.

Every send and confirmation waits for its exact persisted recovery revision.
Cancellation before dispatch consumes the saving token, invalidating late save
acknowledgements. Dispatched work waits for the backend's receipt; cancellation
or disconnect cannot assert that an admitted signal was not sent. Unknown
outcomes retain the exact attempt and require explicit reconciliation, with no
automatic retry. Reopening restores disconnected read-only recovery, never an
executable token. Workspace version 8 reads versions 1–8 and rejects unknown
nested fields and future versions without overwriting original bytes. The
448 KiB/16-document workspace limits are unchanged. SQL-only export refuses an
administration journal and directs the user to complete workspace JSON.

Captures retain the existing 2 MiB display allowance without duplicating the
snapshot. A signal review reserves 32 KiB against the shared 128 MiB retained
payload budget; admission can retry after freeing another capture. When display
admission is unavailable, raw recovery stays in the bounded workspace journal,
without building review text or accepting an executable token. Dispatch reserves
16 KiB for its bounded terminal/confirmation delivery against the shared 16 MiB
queue. These are payload accounting limits, not process RSS claims.

The focused backend probe passed on owned stage03
`127.0.0.1:15432/dbunk_demo`, UUID
`2283820d-33ec-4c4c-ae03-7051092bd410`. Its only signaled target was application
`native_admin_control_873660faba074dbbbdaa6a2ef4d006f1`, PID 70166, backend start
`2026-10-03T03:56:33.241069Z`. It proved stale-start/query refusals, cancellation
SQLSTATE 57014 under read-only policy, read-only termination refusal, Strict
confirmation and target closure, one required override audit, and joined cleanup
with activity 0 → 0. No schema objects were created. The temporary isolated
profile was removed. See [live log](./live-stage03.txt).

Focused backend checks before native integration: administration 18 passed,
2 ignored; document admission 5 passed; isolated all-target Clippy passed.
An initial test setup used current-thread Tokio and was corrected to the
required multi-thread runtime; its failed log is preserved in the raw check
directory. Initial native compilation exposed a missing exhaustive reply arm,
which was corrected. Final required pnpm format/lint/typecheck and Rust fmt/lint/serialized tests pass:
core 677 passed/71 ignored; Tauri 694 passed/85 ignored. Native debug/release
Clippy and tests pass, including fixture-harness Clippy: 255 passed/13 ignored
in each test suite. The isolated suite initially found three old version-7 test
expectations; after correction it passes 933 tests with 88 ignored and 2 doc
tests. Facade tests pass 169 with 17 ignored. Isolated Clippy, Tauri
custom-protocol build, native package and dependency proof pass. The initial
failed log remains intact. Ignored tests are not passes. Python tooling was
unchanged, so its already-passing suite was not repeated.

Package: `/private/tmp/dbunk-native-package-20261003-admin-control/dbunk Native Preflight.app`,
129249855 bytes, executable SHA256
`0e128e6f21e15a6d3ef067572d935ff2951ae23bfa83baaa2220e57ca362ced1`.
All 449 source hashes matched before launch. [Scoped actual-window checks](../admin-control-window-20261003/README.md)
passed cancellation, immutable selection, Protected confirmation/cancellation,
successful termination/audit, staged reopen and explicitly injected unknown
recovery. All three app runs quit normally with fixture activity 0 → 0.
Actual transport-failure and complete keyboard/AX/IME acceptance remain open.

Independent read-only review checked exact-save cancellation, target/receipt
identity, reconnect isolation, v8 decoding and predispatch delivery reservation.
It found the export omission, keyboard review scrolling, non-retryable admission
and ambiguous recovery-clear wording; all four were corrected before packaging.
Full PostgreSQL parity, complete keyboard/AX/IME acceptance, maintenance and
broader administration workflows remain open. VoiceOver remains deferred.
