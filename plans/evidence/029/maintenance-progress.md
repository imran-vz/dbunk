# PostgreSQL maintenance integration, 2026-10-03

The native Objects Tool tab now has a maintenance review over its existing owned
worker. VACUUM, ANALYZE, REINDEX TABLE and both materialized-view refresh modes
use backend observations and opaque document-bound review/confirmation tokens.
The review identifies database, schema and relation OIDs/names/kind, generated
SQL, the inherited/configured statement timeout and a finite 300-second operation
deadline. Foreign tables remain explicitly refused and their parity stays open.

Stored policy and stale timeout checks run before credential hydration. Ordinary
REINDEX and both refresh modes use an explicit transaction and commit fence.
VACUUM, ANALYZE and partitioned REINDEX use a statement-dispatch fence and can
leave effects after interruption. The displayed contract does not equate command
completion with measured work, or transactional rollback with reversal of
sequence/external effects from user-defined functions. Notices have bounded
count and UTF-8 summary bytes, including visible overflow disclosure.

Observed object identity is revalidated, but PostgreSQL utility SQL still
resolves names. Concurrent schema/object changes can retarget an operation;
the review discloses this residual race and does not claim atomic OID authority.

Every apply and confirmation waits for its exact saved recovery revision.
Cancellation before dispatch invalidates late acknowledgements. Schema and
maintenance views share their Objects document's attempt sequence so an old
save cannot release the other tool's request. Workspace version 9 reads prior
versions and stores read-only maintenance descriptions. Lost acknowledgement
and acknowledged possible partial effects retain distinct states; reopening
never reconstructs an executable token. Reconciliation and local discard are
explicit. Mixed schema/maintenance journals in one document are refused.

The view admits 128 KiB against the shared 128 MiB retained-payload allowance.
Raw recovery remains under the existing 448 KiB workspace limit if that display
admission fails. Before dispatch, the worker reserves 64 KiB against the unchanged
16 MiB delivery queue for a bounded receipt or confirmation. These are payload
allowances, not process RSS claims. SQL-only export refuses the maintenance
journal and directs the user to complete workspace JSON.

Verification is in progress. Backend focused tests and isolated Clippy passed;
independent review found and corrected a stale-timeout check that originally
ran after credential access. Native integration Clippy passes after correcting
one private method visibility. The owned stage03 live probe passed maintenance/refresh, stale identity, Strict
policy/audit, cancellation and timeout outcomes, with joined cleanup 0 → 0.
Its first cleanup check refused string-encoded OIDs; only the probe parser was
corrected, and recorded residue was removed with identity guards before the
passing rerun. Required repository/native checks, isolated/facade suites and packaging passed.
[Scoped window/reopen checks](./maintenance-reopen-20261003/README.md) passed
recovered-token refusal, ANALYZE/REFRESH completion, pending-review reopening,
keyboard cancellation and the grid AX correction. The second launcher gate
failed on the external test blocker, then guarded cleanup verified both fixtures
at zero. Receipt/disclosure clarity fixes found during that run passed
[separate source/package checks](./maintenance-source-checks/receipt-correction/README.md)
and [narrow window verification](./maintenance-receipts-window-20261003/README.md),
including clean normal quit. Full maintenance acceptance is not yet claimed. 
Plans 027–030 remain IN PROGRESS; VoiceOver remains deferred, with its checklist
preserved. Remaining keyboard/AX and real IME checks are not marked passed.
