# Table and column window verification, 2026-10-03

Tester: agent, serialized `cua_repl` native keyboard/AX interaction. The launcher
verified package hashes, canonical marked profile and owned fixture before each
launch. No personal profile, production endpoint or daily-driver package was used.

Profile: `/private/tmp/dbunk-native-tables-20261003-review`.
Fixture: `dbunk-native-stage03`, `127.0.0.1:15432/dbunk_demo`, UUID
`2283820d-33ec-4c4c-ae03-7051092bd410`.
New named test schema/table: `native_parity_20261003.rows`, 60 rows with integer
primary key, text value and exact numeric amount. It remains for subsequent owned
acceptance. Existing fixture rows were not modified.

## Table-source package

Executable `fde9c80d5bee18566974c1d4616cd5a4db7d48918c34c3d75bbca34a90d8fa23`.
Identities/teardown: `table-window-20261003/`, `table-window-reopen-20261003/`,
`table-ime-20261003/`. The first launch ended cleanly before functional checks;
it is cleanup evidence only. App discovery initially failed until the CUA app
inventory was refreshed. Raising the window refreshed stale background imagery;
the actual raised window was inspected before visual conclusions.

- New plain-SQLite profile; saved fixture connection with masked AX password.
  SQL `SELECT 42 AS answer` returned 42.
- Manual mode, division by zero: explicit PostgreSQL error, failed transaction,
  Commit disabled, Rollback enabled. Rollback settled to manual idle; switching
  to autocommit then executing the named fixture setup script succeeded.
- Table page size 25; Next returned row 26 first. Raw filter `id >= 58` returned
  rows 58–60; Count reported exactly 3 matching rows. Exact decimal strings were
  present in AX despite shortened visual cell display.
- Edit row 58 value to `checked é😀`; staging retained the old grid value. Bottom
  review showed qualified UPDATE, new value, key 58 and original `row 58` guard.
  Apply refreshed to the exact new value and removed the resolved draft.
- A second staged value `review recovery` survived normal quit and reopen;
  the table restored disconnected with fresh-analysis requirement and raw filter.
  The real Pinyin cell draft described below also survived a later process.
- Found a reproducible AX defect: review container existed but contained no SQL
  or bound-value text. Explicit tree assertions returned SQL=false/value=false.
  The visual SQL was present. This was fixed in the next source/package variant.

## Column package

Executable `edce1936c62c739d5dd6ab63fd61aa3349d2e81520c4f647cbf1ad6f39a765a4`.
Identities/teardown: `column-window-20261003/`, `column-reopen-20261003/`.

- SQL comment `-- 你你` restored exactly, disconnected; two recovered table
  changes restored, with no automatic apply or reconnection.
- Fresh table review exposed semantic AX text for each complete SQL statement
  and its bound JSON values. Explicit assertions found SQL, `review recovery`,
  and composed `你好`: PASS. No VoiceOver listening test is claimed.
- Selected `value`, moved it left, widened it from 160 to 192 logical pixels,
  then hid `id`. Saved status followed service acknowledgement. Visible order
  became value/amount; selection retained the original value-column identity.
- Copied the reordered row-60 value into its editor: exact `row 60`. Editing
  that displayed cell opened `Value for value`. Review of the new value
  `column identity verified` still bound hidden key 60 and original `row 60`.
- Applied the three recovered/new changes; visible values became
  `review recovery`, `你好`, and `column identity verified` on rows 58–60.
- From grid focus, four Shift-Tab actions then Return activated Show all columns.
  AX order was value/id/amount. Raised-window inspection confirmed the widened
  value column and aligned headers/cells. Hiding id again saved successfully.
- Normal quit and separate-process reopen started disconnected. Explicit connect
  restored value/amount order and hidden id with the committed row values.

All five completed launchers exited zero and reported fixture activity **0 → 0**.
Each native log reports queue `remaining_bytes=0`. These are cleanup results,
not aggregate retained-byte peaks or performance acceptance.

## Real IME, scoped result

Input method: macOS built-in **Pinyin – Simplified**, temporarily added through
System Settings. Control-Space selected it. All composing letters were individual
physical `pressKey` events, not pasted Unicode or `typeText` substitution.

SQL: pasted only ASCII comment prefix `-- `, then keys n/i/h/a/o produced visibly
underlined `ni hao`; Space committed `你好`. A second z/h/o/n/g composition
appeared as `zhong`; Escape removed only that marked text. Undo first restored
the cancelled intermediate `zhong`, next removed it, and next undid the earlier
group (including prefix). Redo restored the committed comment. Moving left,
selecting one character and composing n/i/Space produced `-- 你你`; switching
documents retained it. After Saved and normal quit, the column package restored
exact `-- 你你` disconnected. No Run was triggered by composition.

Connection Name: selected old name, physically composed nihao then Space;
observed `你好`. Started zhong, Escape retained `你好` and kept the form open.
Caret/selection replacement produced `你你`; Cmd-Z restored `你好`. Cancelled
the form without creating a new connection or changing an endpoint.

Cell editor: row 59/value, selected old text, physically composed nihao, observed
marked `ni hao`, committed with Space, then cancelled marked zhong with Escape.
Selection replacement followed by undo retained `你好`. Stage change retained
the original grid row. After Saved, quit/reopen and fresh analysis, review showed
the exact composed `你好` with key 59 and original `row 59`; apply succeeded.

Temporary configuration was restored and verified in System Settings: ABC only,
Show Input menu off, dictation languages English (United States) only. macOS had
automatically added Mandarin dictation when adding Pinyin; it was removed.
VoiceOver was not enabled or tested and remains deferred.

This passes composition/commit/cancel/selection/undo/tab-switch/reopen for the
tested SQL/form/cell controls and named binaries. It does not cover future Tool
tabs, all dialog/navigation controls, every input method, or broader Plan 027–030
acceptance. Repeated race, forced-termination and injected-save window gates,
wide-grid/performance acceptance, and remaining PostgreSQL features stay open.
