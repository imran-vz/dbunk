# SQLite profile store and app-wide credential storage

**Status:** Accepted (2026-05-11, rewritten 2026-10-11)

A profile's stored records (connections, bastions, managed servers, saved
queries) live in one SQLite file, `dbunk.sqlite`
(`backend/src/storage.rs`), with versioned migrations. The default profile is
under `~/Library/Application Support/dbunk Native`. If the store cannot open
or migrate, the app stops with an error rather than running without it.

Database passwords use one app-wide mode (`CredentialBackend`,
`backend/src/credentials.rs`), chosen at first launch:

- **Encrypted SQLite** (recommended): Argon2id key derivation and AES-256-GCM,
  unlocked once per session with an app password.
- **OS keychain**: every password in a single keychain entry, cached in
  process, so macOS prompts once per session rather than once per connection
  (`backend/src/keychain.rs`).
- **Unencrypted SQLite**, with an explicit acknowledgement.

The backend never sends a stored password back to the UI. A forgotten app
password cannot be recovered; the only way forward is a confirmed reset that
deletes stored passwords.

Records carry two fields that look unused but are deliberate: saved queries
have `owner_id`, `created_at` and `updated_at` so sync can be added without a
format change, and connections store `last_activity_at` on the record (updated
after a successful connect or query) rather than deriving it from query
history, which users can clear.

## Considered options

- **One keychain entry per connection.** Rejected: it caused a prompt flood.
- **Migrating the old JSON files.** Rejected: there is no migration from the
  pre-SQLite format.
