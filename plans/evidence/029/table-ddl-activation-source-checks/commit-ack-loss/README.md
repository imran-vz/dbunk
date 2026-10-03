# Real commit with a lost runner acknowledgement

The explicit owned stage03 probe passes. The test-only transport waits for a
real PostgreSQL COMMIT acknowledgement, then returns a connection failure to the
DDL runner instead of delivering it. This is deterministic injection at the
runner boundary, not a dropped packet, socket proxy or native-window test.

The runner returns `OutcomeUnknown { Connection }` with the exact attempt,
connection, observed target and intent. COMMIT runs once. An independent fixture
connection confirms the new comment remains committed. A fresh explicit
observation also succeeds after joined cleanup. The operation does not become a
successful safety audit, and nothing automatically retries the intent.

The same run retains existing real comment/rename, empty/NULL, namespace swap and
ABA, cancellation-before-COMMIT rollback and replacement-OID checks. Its two fresh
UUID-named schemas are recorded in `live.txt`; cleanup verifies owners, markers
and OIDs and uses RESTRICT. Fixture activity is 0 before and after. No production,
daily-driver, global event trigger or role was touched. The independently created
window fixture was already removed before this run.

`checks.json` records formatting, isolated-profile Clippy and nine focused tests
(one ignored live test). `live-check.json` records the separately opted-in live
pass. `required-checks.json` records passing `just lint` and serialized `just
test` (core 677 passed/71 ignored; Tauri 694/85). `just fmt`, `pnpm format`,
`pnpm lint`, `pnpm typecheck` and `git diff --check` also pass.
Ignored tests are not passes. Controlled server-hook injection and wire-level
lost-reply verification remain open.

Only two `cfg(test)` files changed after the corrected DDL package was frozen;
`package-source-delta.json` lists both. Its 689-file package proof describes the
earlier exact source, and no rebuilt executable is claimed for this test-only
increment. Native DDL window/recovery, keyboard/AX and real Tool-tab IME remain
pending. VoiceOver remains deferred.
