# ADR-0002 — Health checks run on a 30 s foreground tick

**Status**: Accepted (2026-05-10)

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
fans out across all stored connections in parallel via the
`health_check_connection` Tauri command. Per-connection failures are local
and don't block siblings.

## Consequences

- No idle CPU/network when the app isn't visible.
- Connection statuses converge to truth within 30 s of the user looking at
  the app.
- Any feature that needs faster signal (e.g. live transaction state)
  shouldn't piggyback on this tick — it should drive its own state via
  query-side hooks.
- The tick runs once immediately on app boot so the UI doesn't sit on stale
  cached statuses for 30 s after launch.
