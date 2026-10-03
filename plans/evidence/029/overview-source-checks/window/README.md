# Overview actual-window checks, 2026-10-03

Package: `/private/tmp/dbunk-native-package-20261003-overview/dbunk Native Preflight.app`.
Executable SHA256: `a234ec665b1b5a3ed6735b1ff98e16dbc16f5a06cb08e03f1636ac0af06af750`.
The ownership launcher used `/private/tmp/dbunk-native-auto-fit-20261003-review`.
CUA operated the native window; VoiceOver stayed off.

Observed passes:

- Disconnected restoration retained all 13 documents. Physical Return activated
  the focused Connect action. Connection did not replay a refresh.
- Database refresh captured eight metrics and 256 of 266 relations. Next returned
  the remaining ten, disabled continuation, and disclosed the older retained
  metrics interval alongside the new page interval.
- The owned schema contained two empty tables and 257 constant views. Schema
  refresh returned 256 then three rows, with full-scope totals of 259 on both
  pages. The final page contained `v_254`, `v_255`, `v_256`.
- An analyzed empty table showed zero; an unanalyzed empty table showed Unknown;
  views showed Not applicable. Known subtotal zero and unknown count one remained
  distinct in the totals and exact details.
- End selected `v_253` on the first page. Return entered its details; Tab followed
  by Return activated Administration Back. Editing the schema name disabled Next;
  restoring its exact name re-enabled continuation.
- An owned AccessExclusive lock on `t_empty` held a refresh in progress. An
  immediate Cancel settled as cancelled, disabled Cancel and retained the stale
  capture. The first slower attempt reached the configured statement timeout
  before cancellation and was a refused read, not cancellation evidence. A
  subsequent attempt after the holder ended was also not cancellation evidence.
  Both temporary holders returned normally through ROLLBACK.
- Disconnect preserved stale inspection and disabled refresh. Reconnect required
  explicit refresh.
- Exact relation capture bound `t_unknown` OID 19185. Renaming it and creating
  the same name at OID 20217 caused two refresh refusals, preserving the old
  capture. Explicit Administration > Clear captures reset the identity; a fresh
  relation request then captured OID 20217.

The screenshot exposed a visual defect: relation labels wrapped inside fixed
28-pixel rows and their second line was clipped. Full AX labels and selected
details remained available. This package does **not** pass that visual check;
the separate row-correction package must be checked before closing the defect.

The app quit normally with exit 0. All 13 persisted documents matched the before
snapshot. Cleanup verified exact OIDs, names, kinds, owners and marker comments,
dropped only the 260 recorded relations and schema with RESTRICT, and independently
verified their absence. Both owned fixtures had zero backends. See
[teardown](./teardown.json) and the [ownership receipt](../window-setup/owned-targets.json).
An initial local verification script looked for the workspace in `app_settings`
after successful cleanup; the corrected read used `ui_state` and completed the
document comparison and independent absence checks.

This is scoped keyboard/AX and workflow evidence, not complete keyboard or full
PostgreSQL acceptance. Real IME in this Tool tab remains pending. The separate
[IME record](../ime/README.md) preserves the inconclusive shortcut attempt;
VoiceOver remains deferred and non-blocking.
