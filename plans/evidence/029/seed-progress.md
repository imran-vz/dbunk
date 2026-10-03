# Native table seed progress, 2026-10-03

Status: implementation and verification in progress. This carries baseline
column-aware seeding into the approved table-context Tool tab and Bottom review.
No full PostgreSQL parity or complete keyboard/AX/IME acceptance is claimed.

The table action opens a destination-bound seed tab. Setup supports 1–1,000,000
rows (default 100), an exact optional unsigned 64-bit seed, Auto/DEFAULT/constant/
value-list sources, the 26 baseline generator overrides and nullable NULL rates.
One virtualized column list and one selected-column editor bound retained setup;
no generated rows are sent to the UI. Invalid recipes still expose bounded column
metadata through NeedsRecipe; they never expose an executable review. Replacing
setup explicitly retires only a proven unstarted preparation.

The backend owns tracked inspection and execution connections. A reviewed recipe
freezes exact target/catalog identity, seed and clock. FK samples and integer
maxima are read inside the eventual write transaction; the same seed alone does
not promise identical rows after database data changes. Generated/identity/default
behavior, bounded batches and cancellation/commit boundaries have focused source
and model coverage; scoped window verification is recorded below.
Constraints and triggers can reject generated rows, and sequence/external trigger
effects are not rollback guarantees.

Workspace format 11 adds a bounded seed recovery journal. Earlier formats 1–10
remain readable; older-version seed fields and future fields refuse. The exact
persisted attempt/description must be acknowledged before dispatch; cancellation
invalidates late acknowledgements. Interrupted Applying restores as Unknown,
never a replay. Unknown outcomes require explicit reconciliation. The complete
workspace still visibly refuses saves beyond 448 KiB and at most 16 documents.

A 12 MiB native store allowance and bounded view setup/choices share the existing
128 MiB retained allowance. Bounded synchronous registry calls do not introduce a
native delivery queue. Backend execution reservations are separately bounded;
none of these are process RSS claims. Recipe literals are session-only; recovery
retains a bounded human-readable summary with clipping disclosure and exact digest.

Focused backend, workspace and native checks pass. Source review corrected
generated/identity FK precedence, SQL-prefix budgeting, actual retained recipe
capacity accounting, dirty-setup review invalidation and tiny NULL-rate round trips.
Recovery details now show the saved summary with clipping disclosure; resolved
100% NULL columns use the accurate label “Always NULL”.

The [owned PostgreSQL probe](./seed-source-checks/live/probe.txt) passed unsupported
type inspection without an executable review, 25 inserted rows with exact
composite FK pairing, constant/default/generated behavior, complete rollback after
a late batch constraint failure, and catalog-change refusal. This does not claim
live cancellation or fixed-clock reproducibility. Joined cleanup removed only the
captured owned objects; an independent recheck confirms the schema absent and zero
fixture connections. Required frozen checks and the initial package pass. The [initial window](./seed-source-checks/window/README.md)
passed exact review/invalidation, three committed rows and format-11 preservation
of ten original documents with clean quit. It found multiline Tab indentation and
focus loss when Apply retired review controls. The [correction](./seed-source-checks/window-corrections/README.md)
passes debug/release Tab traversal and receipt focus. An actual interrupted Applying
write restored Unknown, refused preparation until explicit reconciliation, and
never replayed; a new reviewed attempt then committed one row. Normal quit, exact
row checks, original-document preservation and guarded fixture cleanup pass.
Complete keyboard/AX and Tool-tab IME remain open.
VoiceOver stays deferred.
