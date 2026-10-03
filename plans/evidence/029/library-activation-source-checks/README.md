# Deferred library activation, 2026-10-03

The [auto-fit package reopen](../../028/auto-fit-window-20261003/README.md)
reproduced a History startup error when restoring both library Tool tabs.
Each constructor immediately requested a page, reserving up to 8 MiB plus its
request against the 16 MiB shared delivery budget. One page necessarily refused
while both reservations overlapped. A later explicit Refresh succeeded.

Library views now defer their first page until the tab receives focus. Hidden
restored tabs do not allocate reply payloads. The first activation attempts the
read once; returning after an error does not automatically retry it. Explicit
Refresh/Search/paging and queued save acknowledgements retain their existing
behavior. No delivery or retained budget was increased. No SQL is replayed.

The actual-window red check inspected the restored tab label and reported
`pass: false` for `History · Workspace delivery budget is full; retry after
results drain`. The later explicit Refresh loaded the single history record,
distinguishing startup overlap from a leaked permit or failed SQLite read.
This UI construction/focus path has no existing headless GPUI test seam; a
boolean-only unit test would not reproduce the bug. Verification uses the same
profile and restored-window path, plus existing native/runtime tests.

Source change: `apps/native/src/query_library_view.rs`. All required pnpm
format/lint/typecheck and Rust fmt/lint/serialized test checks pass. Core:
677 passed/71 ignored; Tauri: 694 passed/85 ignored. Native debug and optimized
Clippy/tests pass, including fixture-harness Clippy; each native test suite has
251 passed/13 ignored. Dependency proof and separate packaging pass. No backend
facade or Python tooling changes were made for this fix, so their extra suites
were not repeated.

Package: `/private/tmp/dbunk-native-package-20261003-library-activation/dbunk Native Preflight.app`,
129071071 bytes. Executable SHA256:
`a608c818fbf284d53e5f90829c3d6ed7d11ec9dbdd20e3b73102cb6c641be7d6`.
All 440 source hashes matched immediately after packaging, before subsequent
administration implementation began. [Actual-window green verification](../library-activation-window-20261003/README.md)
passed the same-profile startup case and first activation of the hidden library.
The launcher quit normally with zero remaining fixture activity.
Full PostgreSQL parity and complete keyboard/AX/IME acceptance remain open.
