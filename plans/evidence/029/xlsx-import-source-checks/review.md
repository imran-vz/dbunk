# XLSX import implementation review

Independent native review checked setup owner/revision fences, selected-sheet
matching, mapping/review invalidation, abandoned-owner cleanup, hidden CSV fields,
composition guards and retained/delivery accounting. It found sheet keyboard
navigation missing the setup-enabled gate; that gate is now shared with mouse
and AX activation. No further concrete native finding was reported.

Independent backend review found that an inspection whose core CSV reader failed
to join could unlink its private source. Cleanup now preserves source ownership
and admission when IO settlement fails. A held-reader test covers this lifetime.
The review found no additional concrete issue in cancellation, retirement,
accepted-job ownership, permit retention or shutdown cleanup reporting.

Root reviewed ZIP/XML preallocation admission and parser working bounds. Exact
workbook/sheets/sheet and Relationships/Relationship paths are now enforced;
malformed placements refuse. The parser author also found ZIP Unicode Path extra
fields can replace admitted central-directory names. Those extra fields are
explicitly unsupported so they cannot bypass duplicate/path checks.

These are source reviews. They are not fixture, keyboard/AX, real IME or full
parity acceptance. Final test and package receipts are recorded separately.
