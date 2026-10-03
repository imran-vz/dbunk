# Native table copy progress, 2026-10-03

Implementation and verification are in progress. This is not full table-copy or
PostgreSQL acceptance. The selected Tool tab and Bottom review layouts apply.

The backend registers an app-owned attempt before preparation, checks both
endpoint authorities and destination write policy before credential hydration,
and returns an opaque exact review. Name-based mapping matches the old app's
setup, while explicitly disclosing missing/defaulted, generated and identity
columns. Ordinary/partitioned tables are supported; destination row-level
security and unsupported targets are refused. Identity values are preserved
without advancing destination sequence counters. The source repeatable-read
snapshot begins at execution, not at review. A streaming CSV COPY path preserves
SQL NULL versus text and exact numeric values, including finite self-copy; one
destination transaction avoids the baseline's independently committed pages.

The native store owns observations, reviews and a global recovery journal across
tab disposal. Workspace format 10 persists at most 16 bounded copy descriptions
under the existing 448 KiB cap. Apply and policy confirmation each wait for the
exact persisted Applying revision. Cancellation revokes late save acknowledgements.
Restored Applying becomes Unknown, without rebuilding execution authority or
replaying SQL. Unknown writes require explicit destination reconciliation; the
store refuses another copy to that destination while uncertainty remains.

The native store reserves 7 MiB from the shared 128 MiB retained allowance for
one review, observations and journals. Facade calls are synchronous bounded
registry operations; there is no native cross-thread payload delivery lane for
these calls. Database work is owned and joined by the backend. Backend execution
buffers have separate limits of 64 MiB per job and 128 MiB aggregate, at most four
active jobs, 1 MiB fields and 8 MiB records. These are payload bounds, not RSS.

Review found and corrected cancellation dropping admitted data retirement and
fixed timeouts widening shorter explicit connection settings. Another correction
prevents late save failure from overwriting a terminal journal receipt. Concurrent
namespace rename/replacement remains a qualified-name targeting limitation,
despite relation locks and OID/column revalidation. Sequence and external trigger
side effects are not made reversible by transaction rollback. Unknown COMMIT
outcomes retain recovery and are never automatically retried.

Initial backend compile and 13 focused tests passed. Initial native compile
exposed pinned-GPUI call signatures and an exhaustive dispatch arm, corrected in
source. Required backend/repository checks and the owned live probe passed. Native
debug/release checks passed with 280 tests and 13 ignored. Initial actual-window
review/apply and job survival across tab closure passed, including exact-value
database checks and persisted receipts. Window inspection corrected stale
admission wording; final package/recovery checks passed in the owned window. Interrupted Applying
restored as Unknown, refused another copy, and required two-step reconciliation
after independent destination inspection. A new explicit attempt completed and
persisted; all window-only fixture objects were removed with ownership guards. See
[source and window evidence](./table-copy-source-checks/README.md). VoiceOver stays deferred; keyboard/AX
and real IME remain required. No daily-driver or production target is authorized.
