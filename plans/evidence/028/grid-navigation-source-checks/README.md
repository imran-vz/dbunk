# Retained-grid navigation checks, 2026-10-03

Go to row (Cmd-G, also in Results) accepts a positive decimal row number,
clamps to the captured retained range, preserves the current displayed column,
and reveals both axes. It never queries an absent row or navigates a server page.
Invalid and zero input stays in the field with an error. Escape cancels; Tab and
Shift-Tab cycle the input, Go and Cancel. AX and keyboard activation use the same
validation. Marked composition is left to the field instead of submitting it.
Beginning a new query, replacing the table page or changing the active result
removes the transient navigation state and subscription.

The field has a 24-byte input limit, finite history and a 1 MiB reservation
against the existing shared 128 MiB retained-payload allowance. Admission refuses
before editor construction when unavailable, and closing releases it. This is
payload accounting, not a process RSS claim. The 16 MiB delivery queue, draft and
workspace persistence limits are unchanged.

Select current row (Shift-Space, also in Results) creates a rectangle across
visible columns in their current display order, using the existing copy/export
projection. Independent or noncontiguous checkbox row selection remains open.

Focused tests cover positive/invalid/overflow input, retained-range clamping,
empty results, atomic budget refusal and release. Required pnpm format/lint/
typecheck and Rust fmt/lint/serialized tests pass: core 677 passed/71 ignored;
Tauri 694 passed/85 ignored. Native debug/release Clippy and tests pass, including
fixture-harness Clippy: 257 passed/13 ignored in each suite. Ignored tests are not
passes. Backend isolated/facade and Python tooling are unchanged; their latest
passing checks were not repeated.

The separate package and dependency proof pass: 129323215 bytes, executable
SHA256 `086e274f2428ec0696e5f264f334f670f3a570ed251ecdc973956822b2c4a73e`.
[Initial window checks](../grid-navigation-window-20261003/README.md) passed the
scoped keyboard navigation/copy flows and found missing AX range/error labels.
That source correction is pending rebuilt verification and is not part of this
frozen manifest. Full
PostgreSQL parity, remaining keyboard/AX and real IME acceptance remain open.
VoiceOver stays deferred. The manifest records 450 source hashes; the only source
changes from the administration-control package are grid.rs, grid/navigation.rs
and the menu/key bindings in main.rs.
