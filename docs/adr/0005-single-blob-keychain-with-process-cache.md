# ADR-0005 — Connection passwords live in one keychain blob, cached in process

**Status**: Accepted (2026-05-10) — supersedes the per-connection-entry shape
that briefly shipped earlier the same day.

## Context

Connection passwords must not sit in `connections.json` as plaintext (the
file is world-readable inside the user's home dir, and shows up in cloud
backups). The OS keychain is the right home.

The first cut wrote one keychain entry per connection
(`service=dbunk, account=<connection_id>`). That looks neat, but macOS
prompts the user once per entry per session — and dev rebuilds change the
binary signature, invalidating "Always Allow". Users with 4 saved
connections were seeing 4+ unlock prompts every 30 seconds because:

- `find_connection` was called on every backend command (`run_query`,
  `health_check_connection`, `load_table_data`, `load_table_structure`, …).
- Each call resolved every password from its own keychain entry.
- The 30 s health-check tick fanned out across all connections.

Multiplied: N connections × every command + tick = a flood of prompts.

## Decision

All connection passwords live in **one** keychain entry:

- Service: `"dbunk"`
- Account: `"connection-credentials"`
- Password: a serialized JSON map `{ connectionId: password }`

The decoded map is held in process memory behind a `OnceLock<Mutex<Option<HashMap>>>`
(see `password_cache()` in `src-tauri/src/lib.rs`). The first password
lookup of the session reads the keychain (one OS prompt); every subsequent
lookup hits the cache. Writes mutate the cache and persist the whole blob
back.

An earlier draft also carried a one-time migration that folded surviving
per-connection entries from the previous shape into the blob. That code was
removed once we confirmed the per-connection-entry shape never reached an
external user — the only affected machine re-saved its credentials manually.
The plaintext-from-JSON migration (older still, predating any keychain
storage) remains because it can plausibly run on any developer's machine.

## Consequences

- One macOS unlock prompt per session, regardless of connection count or
  command load.
- Granular per-connection deletes still work — they remove the key from the
  in-memory map, then write the whole blob.
- If the keychain blob is ever corrupted, all stored passwords are lost
  together. The user re-enters via Edit Connection. Acceptable: the
  alternative (per-connection blast radius) imposed dramatically worse UX
  every day.
- A future architecture proposal that suggests "one keychain entry per
  connection for cleaner invalidation" should be rejected without re-
  litigating: the prompt-flood pain is the load-bearing reason for the
  current shape.
