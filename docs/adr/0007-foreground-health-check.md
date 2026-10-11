# Foreground health check on explicitly connected connections

**Status:** Accepted (2026-05-10, rewritten 2026-10-11)

Connection health is a 30-second tick that probes only the connections the
user explicitly connected, on separate probe sockets
(`apps/native/src/workspace_health.rs`). Probes never record activity
(ADR-0003).

## Considered options

- **A continuous background check in the backend.** Rejected: it kept
  connections open all the time.
- **Checking every saved connection.** Rejected: it connected engines the user
  never chose and, in keychain mode, flooded password prompts.

## Consequences

Features that need a faster liveness signal must not piggyback on this tick.
