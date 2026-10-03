# Maintenance reopen, execution and cancellation, 2026-10-03

Same package/hash/profile as [the staged run](../maintenance-window-20261003/README.md),
PID 41381. Serialized CUA used only the owned stage03 fixture and disposable schema
`native_maint_window_432c769c4fe84cfe8224a15f1753e6ee` (OID 16784), table rows
(OID 16785) and materialized view summary (OID 16790). Both relations had ownership
comments and five source rows. No daily-driver or production access occurred.

- Reopened exact staged attempt `4164d6f5-cf63-42f5-b628-0e8c1135b816` while
  disconnected. Apply, Confirm and review actions were disabled. Connecting and
  refreshing catalog did not restore executable authority. The staged record
  was explicitly cleared before creating a fresh review.
- Fresh ANALYZE `811ad1ba-08b1-4c3b-99e1-8a5d434483b6` completed. Fresh REFRESH
  `e6843929-b41b-4a8a-a318-bb85df2132bb` completed. Both used the reviewed exact
  identifiers and displayed completion without claiming measured work.
- Initial helper `LOCK TABLE` on the matview was refused by PostgreSQL and its
  client exited. A finite owned source-table lock then produced a real REFRESH
  lock wait (recorded exact SQL). Slower interaction attempts hit the service's
  10-second lock cap and returned a known rollback with SQLSTATE 55P03. One later
  helper collided with the first helper and exited at its own 3-second lock cap.
  These are setup/timeout observations, not cancellation passes.
- In the short recorded sequence, REFRESH attempt
  `08b9998b-c1e2-413c-b242-f2b2cba16cc6` was pending; Back hid the review while
  Maintenance stayed enabled. Reopening preserved that exact pending attempt and
  enabled Cancel. Tab/Return activated cancellation. The terminal status reported
  an acknowledged rollback, “Maintenance interrupted.” The four AX states are
  retained in cancel-sequence-ax.txt. No unknown or partial-effect outcome is
  claimed from this window run; those remain separate backend probe evidence.
- The existing native_parity table loaded 60 retained rows with its prior hidden
  id and visible value/amount preferences. Selecting a cell and Cmd-G exposed
  “1–60 in this retained result or table page” through AX. Entering 0 then Return
  exposed “Row numbers start at 1” through AX. Escape restored row 1. This verifies
  the prior grid AX correction in this package; no table mutation ran.

The app quit normally with exit 0. The launcher then reported a **failed teardown
check** because external owned lock helper PID 50834 was still connected. It was
cancelled only after matching PID, application name, backend start, query start
and exact sleep query, and the helper process was joined. Subsequent identity-
guarded RESTRICT cleanup removed the two recorded objects and schema. Independent
checks show both fixtures at 0, the schema absent and the app PID absent. See
post-helper-teardown.json and owned-objects-cleanup.json. The failed launcher gate
is preserved; do not describe this second launch as a clean launcher pass.
The native delivery queue ended at 0 with high-water 1,771,008 bytes, not RSS.

A screenshot showed the completed receipt repeating a pre-dispatch recovery
state and redundant disclosures. The review also omitted its imposed 10-second
lock cap. Both are corrected in source after this frozen package, with separate
[receipt-correction checks](../maintenance-source-checks/receipt-correction/).
One keyboard batch reported an automation “user changed” interruption; state was
re-read before proceeding. This does not establish a product focus defect.

The temporary Pinyin retry outside GPUI was inconclusive; ABC/input-menu/dictation
settings were restored, as recorded in the accessibility scope document. Broader
keyboard/AX/IME and full PostgreSQL parity remain open. VoiceOver stays deferred.
