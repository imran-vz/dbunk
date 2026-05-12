# ADR-0002 — Health checks run on a 30 s foreground tick

**Status**: Accepted (2026-05-10, revised 2026-05-12)

## Context

The Overview health banner, the Connections screen filter chips, and the
sidebar status dot all need a live signal of whether each saved connection
is reachable. Without one, statuses stay stuck on whatever the last manual
`Connect` action wrote.

Alternatives considered:

1. **No tick** — only update on user actions. Statuses go stale; users see
   "Connected" for connections the network has long since dropped.
2. **Backend tick** — Rust spawns a background task that pings every
   connection at a fixed cadence. Reliable but burns DB connections even
   when the app is in the background or minimized.
3. **Foreground tick** — frontend `setInterval` on the App Shell, paused via
   `visibilitychange` when the tab/window is hidden.

## Decision

Option 3 — a 30-second foreground tick from the App Shell. `runHealthChecks`
fans out via the `health_check_connection` Tauri command, restricted to
connections whose status is already `Connected` or `Read only` (see Scope
revision below). Per-connection failures are local and don't block siblings.

### Scope revision (2026-05-12)

The first cut fanned out across **all** stored connections. Because
`health_check_connection` performs a real driver-level probe — `SELECT 1` for
SQL engines, `PING` for Redis — the tick auto-connected every saved engine
on launch even when the user only intended to use one. Users reported
launching the app to "use Redis" and finding their PostgreSQL connection
flipped to Connected without action.

The tick now ignores any connection whose status is `Disconnected`. A
connection joins the loop only after the user explicitly clicks Connect (or
the loop already had it as Connected and only the latency/lastSync needs
refreshing). This keeps the tick honest: it confirms reachability of
connections you are using, instead of re-establishing connections you are
not.

## Consequences

- No idle CPU/network when the app isn't visible.
- A freshly launched app sits on the saved statuses until the user takes
  action; no connection auto-opens.
- Once a user Connects, that connection's status converges to truth within
  30 s while the app stays visible.
- A connection that goes Disconnected (network drop, server restart) is
  still re-probed by the tick while its status remains Connected — only the
  user clearing it to Disconnected, or `connect_connection` failing, drops
  it from the loop.
- Any feature that needs faster signal (e.g. live transaction state)
  shouldn't piggyback on this tick — it should drive its own state via
  query-side hooks.
