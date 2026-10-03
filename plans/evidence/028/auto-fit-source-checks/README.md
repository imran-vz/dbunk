# Native content-derived widths and auto-fit, 2026-10-03

Initial width estimation and explicit auto-fit are source-implemented for retained query and table results. Final required/native checks and the separate package pass. Actual-window rendering, scrolling, saved-width reopen and keyboard/AX acceptance remain pending. VoiceOver remains deferred; no full PostgreSQL parity claim is made.

## Behavior and bounds

Initial widths use the first 100 retained rows and clamp to 60–400 pixels. Auto-fit uses already-retained rows and clamps to 60–500 pixels. The formula matches the baseline grid's approximate 7.3-pixel UTF-16 unit advance, 18-pixel cell allowance and three-unit header allowance. Only the first displayed line contributes to cell sizing, with the baseline's two-unit newline allowance. Each header/value scans at most 70 UTF-16 units, because later units cannot change the clamp. No cell strings are cloned and no database read occurs.

Query geometry belongs to its exact result set. Streaming samples grow only non-explicit widths within the first 100 retained rows; a user auto-fit is not overwritten by later batches. Geometry storage is admitted with metadata before allocating its vectors. Metadata refusal does not leave an unrenderable result. Horizontal virtualization and keyboard reveal use variable offsets; narrowing clamps scroll and keeps the selected column visible without replacing the selection.

Table defaults derive from the current retained page. Saved widths take precedence and continue to follow source names through ordering and visibility changes. Results > Auto-fit selected/visible columns and the existing table column controls expose the operation. Table auto-fit preflights its bounded source-name patch before allocation, merges it into the latest profile record, and publishes only after the existing exact preference acknowledgement. Hidden widths, filters, presets and unknown fields remain intact. An oversized/invalid save refuses atomically. Query auto-fit is result-local, without inventing a new durable query-layout format.

The added table sample-width storage is charged with page admission; query geometry is charged with result metadata. The existing 128 MiB retained and 16 MiB delivery budgets remain payload budgets, not RSS claims. Apply/draft recovery and unknown-write contracts are unchanged.

## Verification scope

Focused coverage includes UTF-16/newline/clamp behavior, long values, 100-row boundaries, explicit width preservation, independent result geometry, admission-before-allocation, preference merges/refusals, and sampled versus saved widths after reordering. The initial full run passed 248 tests and failed two boundary fixtures whose allowance included only serialized metadata; the next run passed 250 and found the remaining replacement allowance in the same page fixture. These expected allowances now include geometry, preserving the exact refusal checks. The corrected page-boundary test passes. Initial failed logs remain intact.

The final source manifest covers 440 files. All required frontend format/lint/typecheck and Rust fmt/lint/serialized test checks pass. Core tests: 677 passed/71 ignored; Tauri: 694 passed/85 ignored. Pinned native format, debug/release all-target Clippy, fixture-harness Clippy, debug build, debug/release tests and dependency proof pass. Each native suite passed 251 tests with 13 ignored. Extra backend facade/custom-protocol and Python tooling suites were not repeated for this native-only increment; their sources remain unchanged from the prior frozen evidence.

Package: `/private/tmp/dbunk-native-package-20261003-auto-fit/dbunk Native Preflight.app`, 129071071 bytes. Executable SHA256: `58eea8b8090234701432ffda1c08e42f25d84e6b09bdf529955d67593315581d`. All 440 source hashes match after packaging. No window launch was attempted after the comparison package failed CUA discovery in this session; that is prior blocked evidence, not acceptance of this package. Prior window column passes are scoped to their earlier binaries and do not verify these new defaults or controls. No database fixture, production or daily-driver profile was touched by this increment.

Subsequent acceptance attempt after Imran reported the desktop unlocked:
[scoped auto-fit window evidence](../auto-fit-window-20261003/README.md).
Query formatting/undo, retained grid geometry, table preference persistence and
separate-process disconnected restoration were exercised on this exact package.
Both launchers exited zero with fixture activity 0 → 0. A library startup budget
failure was reproduced; broader keyboard/AX and Tool-tab IME acceptance remain
open. The original package/source verification above remains frozen.
