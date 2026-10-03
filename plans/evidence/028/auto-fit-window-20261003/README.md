# Auto-fit package window checks, 2026-10-03

Agent-driven, serialized `cua_repl` checks after Imran reported the desktop
unlocked. Package executable SHA256:
`58eea8b8090234701432ffda1c08e42f25d84e6b09bdf529955d67593315581d`.
Profile: `/private/tmp/dbunk-native-auto-fit-20261003-review`.
The launcher verified stage03 UUID `2283820d-33ec-4c4c-ae03-7051092bd410`
at `127.0.0.1:15432/dbunk_demo`, and TLS UUID
`15151cfc-5885-4066-831f-2717ed9b4587` at port 15433. UI work used only
stage03, a new plain-SQLite fixture connection, and its existing
`native_parity_20261003.rows` table. No PostgreSQL writes were performed.

## Scoped results

- Native window discovery and AX worked. Initial snapshots exposed only menus;
  keyboard activation exposed controls. Some AX actions appeared on the next
  keyboard event. A stale/mostly black capture repainted after native window
  zoom. Later AX actions again needed a keyboard event. This does not establish
  normal foreground rendering or complete focus acceptance.
- Connection Name editing, Tab to Host, masked secure password and Test
  connection worked. Test reported 6 ms; the fixture connection saved.
- Cmd-Shift-F normalized a SELECT with string, escaped multiline and Unicode
  literals. One Cmd-Z restored exact original SQL, including leading spaces;
  redo restored formatting. Cmd-Return completed the read with one returned row
  and four retained columns. F6 and arrow keys selected grid values. The Results
  menu exposed auto-fit and all retained-copy formats; selected-column auto-fit
  reported `Columns fit to retained rows`.
- The rendered query grid had different header/cell widths: narrow `id`, wider
  `label`, and first-line multiline display. Headers and cells aligned. This is
  a four-column check, not wide-grid virtualization/performance acceptance.
- The existing table loaded 60 rows. Widen then selected-column auto-fit saved
  `value: 193.0`. Moving value left, hiding id and fitting visible columns saved
  `{"version":1,"columnWidths":{"value":193.0,"amount":179.0},"columnOrder":["value","id","amount"],"hiddenColumns":["id"]}`.
  AX reported `Table preferences saved`. A read-only SQLite query confirmed the
  exact profile record. After normal quit and separate-process reopen, table and
  SQL documents started disconnected. Explicit table connect restored 60 rows,
  value/amount order and hidden id; the stored widths remained 193/179.
- History showed one successful execution and one returned row. Opening it
  created a disconnected query containing the captured statement. Save query
  created one profile-local Saved record; Toggle favorite displayed its star.
- Administration required explicit connect/Refresh, then displayed scoped
  sessions/activity and collection times. Server facts separately displayed
  PostgreSQL 17.11, UTF8, database LC_COLLATE C and Asia/Kolkata timezone. These
  reads do not verify session-control or maintenance actions.

Both launchers quit normally with exit code 0 and fixture activity **0 → 0**
for both fixtures. Their logs report queue `remaining_bytes=0`. See
[initial identity](./identity.json), [initial teardown](./teardown.json),
[reopen identity](../auto-fit-reopen-20261003/identity.json) and
[reopen teardown](../auto-fit-reopen-20261003/teardown.json).
Delivery queue observations are not retained-payload or process RSS peaks.

## Reproduced library startup failure

Restoring Saved and History together loaded Saved, but left History at
`Workspace delivery budget is full; retry after results drain`. Selecting the
failed tab preserved the error; explicit Refresh then loaded its one row.
Both library constructors dispatched immediately, each reserving an 8 MiB
maximum reply plus a request against the shared 16 MiB delivery budget.
The follow-up fix and its verification are tracked in
[library activation checks](../../029/library-activation-source-checks/README.md).
This frozen package retains the failure and is not labelled fully accepted.

## IME and foreground limits

System Settings was accessible. Baseline: ABC only, input menu off, dictation
English (United States). Temporarily added built-in Pinyin – Simplified.
Control-Space followed by individual n/i/h/a/o key events in Saved-query search
produced plain `nihao`, without marked text or candidates. A foreground
coordinate click then failed with `-10005 noWindowsAvailable`, while AX and
app-targeted keyboard events remained available. No new real composition pass
or product IME failure is claimed.

Removed Pinyin, turned off the automatically enabled input menu, removed the
automatically added Mandarin dictation language, and verified ABC only/input
menu off/English (United States) only through AX. VoiceOver was not enabled;
its checklist remains deferred and non-blocking. The prior scoped Pinyin pass
in SQL/form/cell editors remains valid only for its named earlier packages.
