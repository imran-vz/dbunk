# Scoped CSV window acceptance, 2026-10-03

Used the ownership launcher with the frozen table-copy receipt package, SHA256
`e6b7ba18e82f298985364554858c747af132b43973fea53d0459a421e3ba4353`, and isolated
profile `/private/tmp/dbunk-native-auto-fit-20261003-review`. This evidence
precedes XLSX implementation and does not verify that later source.

The CSV Tool tab selected the saved stage03 connection. Native Save/Open dialogs
selected only `/private/tmp/dbunk-native-csv-window-20261003-files/export.csv`.
File selection did not execute a transfer; explicit inspection, loading, review
and Start were required.

Export read `native_parity_20261003.rows` without modifying it. The published
1,805-byte file contained 60 rows with id/value/amount, including the id hidden
in the grid. The receipt reported publication succeeded and cleanup complete;
its unavailable row count remained unavailable rather than inventing a count.

Import targeted the separately created owned table
`native_csv_window_20261003.destination`. Indexed mapping showed id → integer,
value → text and amount → numeric, with a bounded preview and immutable review.
The completed receipt reported 60 processed and committed rows. A two-way
`EXCEPT ALL` comparison found zero different rows. See
[exact checks](./exact-rows.json) and captured AX states under `ui/`.

The first Command-Q produced no observed change. Focusing a native schema field
and issuing Command-Q completed normal shutdown. The launcher exited zero, both
fixture client counts returned 0 → 0, and the native log reported delivery queue
high-water 8,402,487 bytes and remaining 0. This is not process RSS or complete
keyboard/AX acceptance. New-tool real IME remains open; VoiceOver is deferred.

[Guarded cleanup](./cleanup.json) removed only this window's table and schema,
checking fixture UUID plus object OID/name/owner/comment in a RESTRICT
transaction. The exported file, profile and evidence remain. No production,
daily-driver, XLSX import, cancellation race or lost-COMMIT pass is claimed.
