# Content-derived native grid widths, 2026-10-03

Query and table grids now derive initial widths from at most 100 retained rows and offer auto-fit for a selected column or all visible columns. Query geometry is result-local and budgeted; table widths preserve the existing save acknowledgement and profile merge behavior. See [source and verification scope](./auto-fit-source-checks/README.md). Required/native checks and the separate package pass against 440 source hashes; both native suites passed 251 tests with 13 ignored. Actual-window geometry, persistence and keyboard/AX checks remain required; real IME remains open for the broader migration, and VoiceOver is deferred. Plans 027–030 remain IN PROGRESS.

The later [window/reopen check](./auto-fit-window-20261003/README.md) exercised
query sizing and table auto-fit persistence through reorder/hide/reopen. It also
found a restored-library delivery-budget refusal, now tracked in the
[activation fix](../029/library-activation-source-checks/README.md). Foreground
coordinate input became unavailable; no new real IME pass is claimed.
