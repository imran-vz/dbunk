# Tool tabs and retained-selection copy, 2026-10-03

This variant implements the selected History/Saved queries Tool tabs, execution
history capture, and bounded retained-selection copy formats. It preserves the
existing uncommitted migration. `source-sha256.json` identifies its Rust source,
examples and Cargo files. The column package remains a separate earlier variant.

Implemented behavior: cursor-aware bounded pages (including empty continuations),
connection/search/outcome filters, exact draft opening, SQL copy, favorite/delete,
confirmed history clear, durable saved-query IDs and explicit save/edit. Saved edits
preserve current organization and creation time transactionally. History excludes
cancelled execution, measures monotonic duration, and uses completed-set row counts
without adding omitted rows twice. Incomplete result sets report unknown counts.
A focused red test caught a noncanonical engine name; the constructor now uses
`PostgreSQL`, and its storage round trip passes.

The library's workers and storage acknowledgements are owned and joined. Library
pages share the existing 128 MiB retained-payload budget; requests/replies/history
capture share the 16 MiB delivery budget. These are encoded-payload limits, not RSS.
Copy borrows selected source rows with display-column projections, bounds input
and escaped output at 8 MiB, preserves NULL and numeric strings, and reports partial
retained values. Clipboard actions provide TSV/CSV/JSON/INSERT/Markdown/HTML/TXT.
UTF-16LE is formatter-only preparation. File export, gzip, XLSX and saved export
configurations remain pending.

| Check | Result |
| --- | --- |
| `pnpm format`, `pnpm lint`, `pnpm typecheck` | PASS |
| `just fmt`, `just lint`, serialized `just test` | PASS; core 675, Tauri 692; fixture tests ignored |
| Isolated-profile all-target Clippy and tests, Tauri backend subset | PASS after updating two example document constructors; 747/75 ignored, 53/4 ignored, two compile-fail doc tests |
| Tauri custom-protocol build | PASS |
| Native debug all-target/harness Clippy, tests/build | PASS; 94 passed, 12 ignored |
| Native release all-target Clippy | PASS |
| Native release tests/package | PASS; 94 passed, 12 ignored; separate marked package built |
| Source dependency proof | PASS in `native-dependencies.txt`; package proof also passes |
| Actual-window Tool tabs/copy/IME | PENDING; two window-discovery attempts failed for native app and System Settings; see [attempt record](../tool-tabs-window-20261003.md) |

The pinned native toolchain is Rust 1.98.1. The default backend is Rust 1.97.1.
The debug linker reports a compact-unwind size warning; the pinned `block` crate
also reports an existing future-compatibility warning. Neither is a failed check,
and no dependency graph changes were made to suppress them.

`history-engine-red.txt` and `native-backend-red.txt` preserve the failed attempts;
subsequent focused/integrated logs show the corrections. Ignored fixture tests
are not passes. These source checks do not establish complete PostgreSQL parity.

Frozen package: `/private/tmp/dbunk-native-package-20261003-tools/dbunk Native Preflight.app`.
Executable SHA256: `6abd0abebf535d980d8c8f6340d9edba123d6e8cba56525340d800f91f5a28ca`.
All 269 recorded source hashes matched after packaging. Subsequent table-filter
work is another variant and is not covered by these tests or this binary.
