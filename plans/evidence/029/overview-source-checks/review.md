# Overview source review

Independent read-only review compared the new reader with baseline `102568b`
`postgres/admin.rs` and inspected document ownership, keyset continuation,
retained/delivery accounting, cancellation and keyboard/IME guards.

Three concrete findings were corrected and source-rechecked before release verification:

1. A cursor originally bound only connection ID and server-local OIDs. Another
   admitted document with the same connection ID could target another server
   with matching OIDs. The backend now binds the opaque cursor to the originating
   private document UUID before hydration and checks captured database name as
   well as scope/OIDs. Review rechecked the correction; live cursor rejection is
   passing in the final owned live probes.
2. Native Refresh initially dropped observed OID guards after a failed or
   cancelled read. Repeating Refresh with unchanged controls now retains those
   guards. Explicitly clearing captures in Administration allows a new target
   inspection; a transient failure must not silently rebind a capture.
3. The child exposed Cancel during connection opening or after cancellation was
   already requested, while the parent would ignore it. Its cancellation state
   now uses the parent's actual read-cancellation gate separately from general
   busy state through a typed runtime snapshot.

The reviewer found no additional concrete defect in the baseline aggregate
scope, permission/error distinctions, joined read lifetime, bounded wire pages,
incoming/retained/editor overlap accounting or composition guards. This is source
review, not actual keyboard/AX/IME or live-fixture acceptance.

Initial native integration compilation also found a mismatched render return
type, an unused context parameter and the new reply missing from the table view's
unrelated-message arm. These were corrected before the next native check. The
initial failure log is retained.
