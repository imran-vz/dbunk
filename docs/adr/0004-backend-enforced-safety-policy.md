# Backend-enforced safety policy

**Status:** Accepted (Plan 007, rewritten 2026-10-11)

Every write path checks one policy in the backend (`backend/src/safety/`),
built from three stored connection fields:

- **Environment**: Development, Test, Staging or Production.
- **Safe mode**: Disabled, Protected, Strict, or Inherit. Inherit resolves by
  environment: Development and Test → Disabled, Staging → Protected,
  Production → Strict (`resolve_policy`).
- **Read-only**, which refuses writes regardless of safe mode.

A statement classifier decides what counts as a write and fails closed: SQL it
cannot classify counts as a write (`backend/src/postgres/sql_class.rs`, with
MySQL and SQLite dialect rules). Protected asks for confirmation of
destructive or unbounded statements and of row mutations; Strict asks for any
write. The only unlock is a request re-sent with `confirmed: true` for the
exact reviewed operation. Confirmation never unlocks read-only, and every
confirmed override is audited without its SQL text.

The UI mirrors the policy for presentation (environment frame and tint,
review dialogs, typed `confirm` for Production) but the backend stays
authoritative. Each engine applies the same policy to its own statement
shapes: ClickHouse heads, Redis commands (ADR-0005).

The threat model is user mistakes, not attackers; database privileges remain
the real security boundary.
