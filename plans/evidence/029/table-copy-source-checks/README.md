# Table copy source, fixture and window checks

Scope: native PostgreSQL table-to-table append through the approved Tool tab and
Bottom review. Full PostgreSQL parity and complete keyboard/AX/IME acceptance
remain open. No production or daily-driver target was used.

## Source verification

`backend-checks.json` records zero exits for required pnpm format/lint/typecheck,
just fmt/lint/serialized test, isolated all-target Clippy/tests, facade tests and
Tauri custom-protocol build. The isolated suite passed 982 tests with 89 ignored
and two doctests; the facade filter passed 194 with 18 ignored. Ignored tests are
not passes. Required just test configurations passed 677/71 ignored and 694/85
ignored.

The initial native package preceded the borrowed connection-choice admission
correction. `final-native-checks.json` covers that correction with 280 passed and
13 ignored in debug and release, both Clippy configurations, harness Clippy,
packaging and dependency proof. Initial failed compile/test attempts are retained;
final successful logs supersede those attempts only for their named checks.

The window then found stale admitted-status wording after terminal completion.
The receipt wording now directs the reader to the attempt outcome and cleanup
status. `receipt-native-checks.json` and `receipt-*` logs cover that final source;
packaging and the scoped final-window recovery checks passed. No backend source
changed after its recorded matrix. `source-sha256.json` names the final source;
preceding manifests preserve the earlier package scopes.

## Owned live backend probe

[Probe output](./live/probe.txt), [identity](./live/identity.json) and
[teardown](./live/teardown.json) record the owned stage03 UUID
`2283820d-33ec-4c4c-ae03-7051092bd410`, private disposable profile and unique
schema. Exact decimal/bigint values, SQL NULL versus text/empty, identity/default/
generated mapping, finite self-copy, late-row rollback, stale target/authority,
in-flight cancellation and success-only audit passed. Object OID/owner/comment
guards and RESTRICT cleanup removed only this probe's objects. Joined shutdown
returned fixture activity 0 → 0. The first probe stopped at canonical-profile
validation before creating its profile/schema; its output remains separately
under `live-initial-profile`.

## Initial native window

[Identity](./window-initial/identity.json): executable SHA256
`2b291e7131892b59d97b9e4a132aaec9a479094344a77804400bf4e1b5196ec8`, isolated
profile `/private/tmp/dbunk-native-auto-fit-20261003-review`. This package does
not contain the later connection-choice admission or receipt-wording correction.

- Prepared owned `native_copy_window_20261003.source` → `destination`. Review
  exposed three copied columns, a defaulted column and generated column. Apply
  displayed exact-recovery saving before admission. The terminal receipt and
  persisted journal both recorded three rows. [Database rows](./window-initial/rows.json)
  retained the large decimal, SQL NULL, literal NULL text, Chinese text, default
  and generated values exactly.
- A second copy to `slow_destination` was Running before Command-W closed its
  Tool tab. Reopening found the same running attempt; it later completed with
  three rows. Both terminal receipts persisted after normal quit. This verifies
  app-owned job lifetime across tab disposal.
- The rebuilt diagnosis report exposed both plaintext and SCRAM-channel-binding
  warnings in AX. Connection edits were cancelled without saving.
- AX actions sometimes appeared only after a subsequent keyboard event. This
  is an unresolved activation observation, not complete AX acceptance. The prior
  diagnosis cleanup had closed Objects as well as its disposable query; Objects
  and its original connection binding were restored, with nine documents now
  including the new Table copy Tool tab.
- Normal quit returned zero and both fixture counts 0 → 0. Real IME retry in
  System Settings produced only plain nihao. Temporary sources/preferences were
  restored; new-tool composition remains unverified. VoiceOver stays deferred.

## Final package and recovery window

The frozen receipt package is
`/private/tmp/dbunk-native-package-20261003-table-copy-receipt/dbunk Native Preflight.app`,
executable SHA256
`e6b7ba18e82f298985364554858c747af132b43973fea53d0459a421e3ba4353`.
All 491 recorded source hashes and bundle file hashes match; see
[source match](./source-match.json) and [package identity](./package-identity.json).
Final debug/release tests each passed 280 with 13 ignored.

- Reopen restored the two completed receipts, with documents disconnected.
  Connection choices were selected using arrows and Enter; Tab traversed all
  four endpoint fields, each retaining its intended text.
- Applied an explicit slow copy, observed Running/Pending and Saved Applying,
  then quit normally. Joined shutdown left zero destination rows. The saved
  journal remained Applying. Reopening showed Unknown, disabled Apply, and
  refused a new preparation to that destination. No SQL replay occurred.
- Independently checked destination row count and zero fixture clients. The
  two-step reconciliation changed only the record to Reconciled. A new explicit
  Prepare/Review/Apply then completed three rows. Exact values and both the
  reconciliation and completion persisted after normal quit.
- Both final launchers exited zero with stage03 and stage04-TLS clients 0 → 0.
  [Window evidence](./window-final/) includes identities, teardown, AX snapshots,
  database checks and journal states.
- [Guarded cleanup](./window-final/cleanup.json) removed only the owned three
  tables, trigger function and schema in one RESTRICT transaction, after checking
  OID/name/owner/comment and fixture UUID. Profiles, receipts and evidence remain.

These are scoped correctness/recovery and keyboard checks, not complete visual,
AX, real-IME or PostgreSQL acceptance. Lost COMMIT acknowledgement is covered by
source failure semantics, not an induced network failure in this window run.
The app was quit normally, not force-killed. VoiceOver remains deferred.

