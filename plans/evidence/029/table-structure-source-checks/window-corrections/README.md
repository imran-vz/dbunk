# Structure actual-window checks and corrections

Initial release executable SHA256:
`371667b1f3677fc683256787120ec95a7b73591397edc93e4b6bdea6faf44db0`.
The agent used serialized `cua_repl` against the isolated profile
`/private/tmp/dbunk-native-auto-fit-20261003-review` and the checked stage03
fixture UUID `2283820d-33ec-4c4c-ae03-7051092bd410`. The TLS fixture was checked
for ownership and teardown only. No production or daily-driver target was used.

## Observed behavior

- Table-context Structure opened the same connection's Objects tab and exact
  relation. Columns distinguished identity, generated expression, empty SQL
  literal, NULL absence and a Unicode comment. Section arrows, list End/Up and
  detail-to-Catalog Tab/Return were exercised; this is scoped keyboard/AX evidence.
- Composite FK pairs preserved `y → a`, `x → b`. Expression index keys and
  INCLUDE positions, predicate, trigger UPDATE OF order, policy expressions,
  explicit PUBLIC SELECT grant and rule SQL appeared in exact selected details.
- Open related table rechecked the parent identity and opened its fixture row
  `(7, 11)`. Inbound FK and ordinary inheritance metadata were inspected.
- Disconnect retained details, marked them stale and disabled navigation.
  Reconnect alone left navigation disabled; explicit refresh recaptured metadata.
- After guarded rename of owned child OID 17075 and replacement under the same
  name at OID 17107, captured inbound navigation refused the changed identity.
  It retained the previous capture as stale and did not open the replacement.
- Partition leaf metadata displayed `FOR VALUES FROM (0) TO (10)` and the exact
  parent OID. Its parent row label incorrectly said inheritance, corrected below.

The [AX captures and screenshot](../window/foreign-key.png),
[identity refusal](../window/replaced-identity-refusal.txt) and
[independent teardown](../window/cleanup.json) identify this scope. Normal quit
returned 0; all twelve preceding documents were unchanged. One related-table
document was added. Exact recorded objects were removed with RESTRICT after
OID/owner/comment checks; the schema is absent and both fixtures have zero
remaining app backends. The profile and existing parity fixture were retained.

## Corrections

Returning to Catalog from the first direct table-context read exposed an unloaded
empty catalog. Back now requests the initial catalog load through the existing
read admission checks when no capture exists. Existing captured lists are kept.

Parent relationship labels now use the inspected child's partition flag; child
labels use the child's own flag. A partitioned parent is not necessarily itself
a partition. The focused test covers ordinary inheritance and a partition whose
parent is not a partition. Its first fixture omitted the required partition bound
and correctly failed snapshot validation; the fixture now includes that bound.
The failure log is retained, not counted as a pass.

Corrected release executable SHA256:
`e3710730062beaacc0c8e8d4a5e582232e8ee6653c3e746314117f3b9c089492`.
All 578 compiled-source manifest entries matched. Debug/release Clippy and tests
pass (313 passed, 13 ignored), fixture-harness Clippy passes, and package build
and dependency proof pass. Frontend format/lint/typecheck and diff checks pass.
The earlier required just fmt/lint/test remain applicable to the unchanged backend.

The corrected window passed detail Tab/Return to Catalog with automatic loading
of 32 objects, correct partition-parent labeling, and the app's exact-copy readback
acknowledgement. See [the corrected window records](./window/catalog-auto-load.txt)
and [cleanup](./window/cleanup.json). It quit normally with all thirteen documents
unchanged. The second owned schema was removed after exact receipt checks; both
fixtures again had zero app backends. Copy acknowledgement is not an independent
external paste test.

These findings do not prove all inspector controls, restricted-role behavior,
large-capture performance or all PostgreSQL versions. Structure editing remains
open. Tool-tab real IME remains pending; the separate OS activation attempt did
not observe composition and restored temporary settings. VoiceOver stays deferred.
