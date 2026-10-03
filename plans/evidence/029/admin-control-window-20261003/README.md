# Administration window verification, 2026-10-03

Package: `/private/tmp/dbunk-native-package-20261003-admin-control/dbunk Native Preflight.app`.
Executable SHA256:
`0e128e6f21e15a6d3ef067572d935ff2951ae23bfa83baaa2220e57ca362ced1`.
Bundle 129249855 bytes; all 449 source hashes matched before launch. The final
source-only changes after native tests updated three backend test expectations
from workspace version 7 to 8. The corrected isolated and facade suites pass.
[Source checks and limits](../admin-control-source-checks/README.md).

Owned profile: `/private/tmp/dbunk-native-auto-fit-20261003-review`, profile ID
`c16f87dc-e21a-4ea2-b2ed-a0fdc9aa8a42`. Its existing query/table/library drafts
were retained. Workspace storage explicitly advanced to version 8. The older
frozen packages and Plan 027 profile were not changed. Only owned stage03
`127.0.0.1:15432/dbunk_demo`, UUID
`2283820d-33ec-4c4c-ae03-7051092bd410`, supplied test targets. The TLS fixture
manifest remained attached to this profile; no TLS query was run. The existing
Development/Protected/non-read-only policy was inspected and left unchanged.

## Scoped passes

- Normal restore left query, table and Administration disconnected. Library tabs
  remained deferred until activation. Administration explicitly connected and
  collected sessions over a disclosed interval.
- Owned application `native_admin_window_cancel_4b7190e2`, PID 10172, start
  `2026-10-03T04:10:23.730042Z`, query start
  `2026-10-03T04:10:23.734332Z`: exact review attempt
  `1bb88ce9-98ae-4362-9134-d9de382699bf`. Selecting the reader row afterward
  changed row details but preserved the reviewed target. Shift-Tab, Home/End and
  Return navigated/scrolled the review and activated Send. The UI reported
  SignalSent with its limitation; the owned psql target returned SQLSTATE 57014
  and exited. No target remained. [Identity](./cancel-target.json).
- Owned application `native_admin_window_terminate_4b7190e2`, PID 14796, start
  `2026-10-03T04:12:03.235636Z`: Send reached Protected-policy confirmation.
  The target remained active. Cancelling attempt
  `64bd308f-a9aa-4158-9816-3789dd633dc5` left it active and removed confirmation
  authority. Clearing that local record required a fresh capture/review.
  Fresh attempt `fd42a07f-bff2-4fb5-aea7-61d246eae0d0` again required confirmation.
  Confirm returned SignalSent; the target returned SQLSTATE 57P01 and exited.
  [Identity](./terminate-target.json), [pre-confirmation observation](./confirmation-target-still-active.txt).
- Safety overrides explicitly refreshed one successful `terminate_pg_backend`
  record (ID 1) for this connection. The cancelled confirmation produced no
  record. Selection exposed the profile-local retention and incomplete-audit
  disclosures. This is one-row inspection, not full cursor/expiry acceptance.
- Owned application `native_admin_window_recovery_4b7190e2`, PID 19223, start
  `2026-10-03T04:13:27.795433Z`: attempt
  `18d8d2f1-13c5-4b08-bbc4-3056328ade8d` was reviewed and acknowledged Saved.
  No app signal was dispatched. Close tab refused to discard pending recovery.
  The test driver then cancelled its own psql client, which exited; activity
  returned to zero. [Exact saved journal](./saved-staged-recovery.json).
- Normal quit/reopen restored that staged attempt and target disconnected, with
  Send/Confirm disabled. Export opened a save dialog for `workspace.json`; it
  was cancelled without writing. This mixed workspace also contains a table;
  the administration-only export guarantee is covered by the focused unit test.
- After another normal quit, an explicit **offline recovery fault injection**
  changed only that record's `applyState` from `staged` to `outcomeUnknown`.
  The private profile lock and SQLite transaction were held; original bytes
  were backed up outside the profile, and every other decoded field was checked
  unchanged. [Injection evidence](./recovery-injection.json). This is not an
  actual lost-reply or uncertain-signal test.
- Unknown reopen preserved exact identity and disabled Send/Confirm. Explicit
  Connect/Refresh did not authorize another review. First Reconcile retained the
  journal and asked for inspection; the exact target was absent. A separate
  Discard reconciled recovery removed only that injected local record. It sent
  no signal. Normal quit left the profile without an administration journal.

All UI interaction used serialized CUA AX/keyboard actions. AX clicks sometimes
required a subsequent Tab event before GPUI published the visible result; no
speculative source change was made. Screenshot plus Home/End established that
the signal disclosure could be reached by keyboard. This does not establish
complete focus/AX or all window-size acceptance.

## Teardown and limits

Three launcher runs quit normally with exit 0: PID 8815
([identity](./identity.json), [teardown](./teardown.json)); PID 21291
([staged reopen](../admin-control-reopen-20261003/teardown.json)); PID 23672
([unknown reopen](../admin-control-unknown-reopen-20261003/teardown.json)).
Both fixture activity counts were 0 → 0 on each run. All three temporary target
clients exited. The primary and unknown-reopen workspace queue logs report
1705472-byte high water and zero remaining bytes. This is delivery payload
accounting, not RSS. No schema/data objects were created or modified.

This verifies the named success/confirmation/recovery cases only. Permission
failures, delivered-reply loss under real transport failure, all lifecycle races,
all window sizes, complete keyboard/AX/IME and full PostgreSQL parity remain
open. VoiceOver remains deferred, not passed.
