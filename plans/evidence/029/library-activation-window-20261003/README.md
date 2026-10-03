# Library activation and Tool-tab window checks, 2026-10-03

Agent, serialized `cua_repl` AX/keyboard interaction. Executable SHA256
`a608c818fbf284d53e5f90829c3d6ed7d11ec9dbdd20e3b73102cb6c641be7d6`,
package `/private/tmp/dbunk-native-package-20261003-library-activation/dbunk Native Preflight.app`.
Reused owned profile `/private/tmp/dbunk-native-auto-fit-20261003-review`.
[Identity](./identity.json) records the exact stage03 and TLS fixture UUIDs.
Only stage03 was queried; no PostgreSQL writes were performed.

## Startup failure fixed

The same profile that previously refused a restored library page now restored
active History with its one record. Hidden Saved displayed `Open tab to load
records`. The AX assertion for those states and absence of the delivery-budget
error returned `pass: true`. Selecting Saved loaded its one starred record.
Query and table documents remained disconnected until explicit connection.
The [red observation](../../028/auto-fit-window-20261003/README.md) is preserved
against the prior package. Returning to tabs does not implicitly reload them.

## Additional scoped checks

- Saved-query search: typed `longer`, Tab to Search, Return; one matching
  record returned and controls re-enabled. This is keyboard search evidence,
  not real composition or continuation-page acceptance.
- Table: restored value/amount display and hidden id; typed `id = 59` returned
  the existing `你好`/`5.9000000000000000` row. Saved `row59` preset, cleared to
  60 rows, applied preset back to one row. Cycling browse history selected the
  earlier unfiltered entry; Apply history restored 60 rows. No row was edited.
- Browse inspector exposed executed SQL with `WHERE "id" = ($1::text)::integer`
  and `$1={"kind":"text","value":"59"}`. Applying unfiltered history removed
  the predicate and parameters. This checks generated-query AX disclosure.
- Explain draft created a disconnected query. Replaced its text with
  `EXPLAIN (FORMAT JSON) SELECT id, value FROM native_parity_20261003.rows ORDER BY id LIMIT 3;`,
  explicitly connected and ran. The two-node Limit/Index Scan tree showed
  unknown actual/planning/execution metrics for plain EXPLAIN. AX Pick plus
  Left collapsed the child; Right and Down expanded and selected Index Scan.
  This does not verify ANALYZE, large plans or all metric calculations.
- Objects required a selected Navigator connection. The bound tab loaded 29
  objects. Selected the owned parity table and Describe; metadata JSON exposed
  owner, columns and primary-key reconstruction. Definition SQL remained
  unchanged after Select All and typing `x`. Its omission notice explicitly
  named inheritance, storage options, policies, triggers and grants.
- Read-only Drop impact for that table returned no dependents and
  `truncated: false`, with its scope/authorization distinction. No DROP ran.

Some AX actions appeared only after a keyboard event. Escape used during the
first Describe attempt dismissed its result; repeating with Tab exposed the
description. No speculative rendering/focus fix was made. These scoped AX and
keyboard observations do not establish full foreground/VoiceOver acceptance.
The preceding [IME attempt and restored OS settings](../../028/auto-fit-window-20261003/README.md#ime-and-foreground-limits)
remain unchanged; no new real IME pass is claimed.

Normal Cmd-Q exited zero. [Teardown](./teardown.json) reports both fixtures
**0 → 0**; the native log reports delivery queue high-water 10,094,592 bytes and
`remaining_bytes=0`. This is delivery accounting, not retained-payload or RSS
acceptance. Profile and frozen package are preserved. Full PostgreSQL parity
and remaining Plans 027–030 gates are still incomplete.
