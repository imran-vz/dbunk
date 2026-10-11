# Native GPUI app over a host-neutral backend library

**Status:** Accepted (2026-10-03)

dbunk is one native macOS app built on GPUI (`apps/native/`) over a Rust
library (`backend/`). The Tauri/React app was removed before reaching feature
parity rather than kept alongside: running two clients doubled every feature
and every safety fix, and the webview added a serialization boundary that
memory-bounded features had to work around.

The backend does not know its host. A host supplies a multi-threaded Tokio
runtime handle, one `EventSink` per stream (`backend/src/host.rs`), and
lifecycle inputs such as focus and shutdown. Hosts call services through the
`Backend` facade (`backend/src/backend.rs`, behind the `isolated-profile`
feature), never the managers underneath, so connection policy, safety checks
and cleanup cannot be skipped.

## Considered options

- **Split into a `crates/dbunk-core` crate.** Rejected for now: the engine,
  storage and service modules form one dependency cycle, and exposing them as
  a crate would publish operations that bypass the safety gate.
- **Keep Tauri until the native app reached parity.** Rejected: see above.

## Consequences

- New capability lands in `backend/` first and gets its UI natively.
- The backend builds bare (engine core) and with the native facade; both are
  linted and tested (`just lint`, `just test`).
