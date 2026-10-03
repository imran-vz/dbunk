# Scoped table-copy review

Independent source review covered backend registration/retirement/commit,
workspace recovery, native store and view bounds. Findings corrected before
final checks:

- Do not drop an admitted data-retirement future on copy cancellation. Await
  retirement, then observe cancellation.
- Copy timeouts must not widen shorter configured or inherited PostgreSQL
  statement/lock limits.
- Late save acknowledgement/failure must not replace an already terminal
  durable receipt; cancellation revokes only its exact save request.
- Borrow connection choices and reserve replacement overlap before cloning;
  reject over-budget input without entering the cloning pass.
- Window inspection found admission text still saying it awaited an outcome
  after completion. It now directs readers to the per-attempt receipt.

Native window AX observation was separately traced read-only through the pinned
GPUI source. AX Click invokes the listener without an immediate draw; ordinary
keyboard dispatch draws dirty state before dispatching Tab. Thus a later Tab can
publish a mutation already applied by AX. macOS normal redraw depends on the
display link. Existing Workspace activation and changed paths already notify.
No confirmed app-handler defect or display-link failure was established, so no
speculative notification or dependency patch was made. The evidence cannot
separate frame publication, foreground scheduling and tool delivery; complete AX
acceptance remains open. Do not issue a second destructive/closing action just
because the immediate AX snapshot is unchanged.

Remaining explicit limits: concurrent schema rename/replacement is not excluded
by relation locks and qualified-name/OID revalidation. Source snapshot begins at
execution, not review. Identity sequences are not advanced; sequence and external
trigger side effects are not generally reversible. Unknown COMMIT outcomes must
be independently reconciled and cannot be automatically retried. This review is
not full PostgreSQL parity or keyboard/IME acceptance.
