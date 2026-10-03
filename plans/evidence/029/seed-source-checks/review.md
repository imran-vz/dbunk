# Seed source review

Source review covered immutable review ownership, exact save acknowledgements,
cancellation/commit boundaries, recovered unknown outcomes, joined connection
cleanup, retained capacity bounds and column recipe disclosure.

Corrected findings:

- Generated/identity columns now skip before FK planning; illegal explicit
  overrides refuse instead of generating derived values.
- Batch SQL admission includes the INSERT prefix and actual final SQL size.
- Prepared plans account for retained vector/string capacity before publication.
- Local endpoint/count/seed and unsaved column edits invalidate Apply for the
  associated prepared attempt. Explicit Prepare creates a new exact authority.
- Tiny NULL rates survive bounded editor reconstruction without becoming zero.
- Recovery details display the recorded recipe summary and clipping flag;
  full literal/list values remain session-only and available in exact live review.
- Always-NULL actions have a neutral label, including supported FK columns whose
  explicit NULL rate is 100%.
- The isolated workspace example initializes the new seed recovery journal.
  The initial all-targets Clippy failure is retained in `isolated-clippy.txt`.

Independent native review found the two disclosure issues above and no additional
exact-save, cancellation, recovery or tab-lifetime defect. Independent backend
review found no further registry/service/budget issue. These reviews are source
inspection, not live cancellation, process-RSS or actual-window acceptance.
