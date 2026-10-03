# Native seed source and fixture verification

Scope: the approved table-context seed Tool tab, column recipes, exact Bottom
review, app-owned PostgreSQL execution and format-11 recovery. Full PostgreSQL
parity and complete keyboard/AX/IME acceptance are not claimed.

The [source manifest](./source-sha256.json) identifies 564 files including the
example harnesses. [Source review](./review.md) records corrected findings.
Initial failures remain in their original logs; final receipts supersede only the
named checks. Ignored fixture tests are not passes.

The [owned fixture probe](./live/probe.txt) passed one explicitly opted-in test on
stage03 `2283820d-33ec-4c4c-ae03-7051092bd410`. It verified unsupported-type recipe
inspection without executable authority, exact composite FK pairing for 25 rows,
constant/default/generated behavior, full rollback after a late 600-row batch
failure and stale-catalog refusal. The probe joined shutdown and removed only its
captured objects. [Independent teardown](./live/teardown.json) confirms its exact
schema absent and zero fixture connections.

The [IME precheck](./ime/README.md) remains inconclusive outside GPUI. Temporary
settings were restored. VoiceOver stays deferred and was not enabled.

Required pnpm format/lint/typecheck and just fmt/lint/serialized test pass. Required
backend configurations pass 677/71 ignored and 694/85 ignored. Isolated all-targets
Clippy passes after the recorded example fix; isolated tests pass 1,022 with 91
ignored and two doctests. Facade tests pass 212 with 20 ignored. Native debug and
release tests each pass 300 with 13 ignored; debug, fixture-harness and release
Clippy pass. Native debug build, 36 Python tooling tests, dependency proof and the
Tauri custom-protocol build pass. Test processes use `CARGO_INCREMENTAL=0` and
`RUST_TEST_THREADS=1`; default test debug settings were retained.

The initial package/window probe passed exact review and three-row commit, then
exposed two keyboard defects. The [corrected release](./window-corrections/README.md)
passes scoped Tab traversal, persistent receipt focus, actual interrupted-write
recovery/reconciliation and a new explicit one-row commit. Its final native
Clippy/tests pass. All ten original documents remain unchanged. Guarded RESTRICT
teardown removed the captured window schema and returned both fixtures to zero.
See [implementation limits and progress](../seed-progress.md).
