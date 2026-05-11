# ADR-0007 — SQLite is primary local persistence; credentials are app-wide

**Status**: Accepted (2026-05-11)

Supersedes ADR-0005. Updates the persistence implications of ADR-0003 and
ADR-0004.

## Context

The previous local persistence model split state across JSON files and the OS
keychain:

- `connections.json` held connection metadata.
- `query_history.json` and `saved_queries.json` held recoverable workspace
  state.
- The OS keychain held all connection passwords in one JSON blob.

ADR-0005 reduced macOS prompt floods by consolidating keychain credentials
into one entry with a process cache. That still leaves the app dependent on an
OS prompt path that can ask for permission after dev rebuilds or signature
changes. Users need an app-level alternative that stores credentials under
`~/.config/dbunk/`, can encrypt passwords with a user-supplied password, and
keeps predictable startup behavior.

The app is pre-production. We do not need a compatibility migration from the
old JSON/keychain shape.

## Decision

All local app persistence moves to one SQLite database:

```
~/.config/dbunk/dbunk.sqlite
```

The database is opened once at startup through a Tauri-managed SQLite pool.
Startup runs a lightweight embedded migration runner backed by a
`schema_migrations` table. If the database cannot open or migrations fail, the
app fails loud with a blocking error rather than silently booting with empty
state.

SQLite owns these tables:

- `app_settings` — generic key/value settings.
- `connections` — connection metadata.
- `credentials` — one credential row per non-SQLite connection when a SQLite
  credential mode is active.
- `credential_verifier` — encrypted-store verifier metadata.
- `query_history` — recent query history.
- `saved_queries` — saved SQL snippets.

The old JSON files are ignored. They are not read, written, or migrated.

Credential storage is app-wide. There are exactly three modes:

1. **Encrypted SQLite** — recommended. Database passwords are encrypted at
   rest using a shared app-level credential password.
2. **OS keychain** — available, not recommended. Uses the existing single
   keychain blob and process cache to avoid per-connection prompts, but the OS
   may still ask for permission.
3. **Unencrypted SQLite** — available with an explicit warning. Database
   passwords are stored plainly in `dbunk.sqlite`.

Only database passwords are secrets. Connection metadata such as name, engine,
host, port, database, user, role, and last activity remains plaintext in the
`connections` table. SQLite database-engine connections have no credential
row.

Encrypted SQLite uses password-based authenticated encryption:

- Argon2id derives a 256-bit key from the user password and a random salt.
- AES-256-GCM encrypts each password with a random nonce.
- A dedicated verifier record is encrypted with the same scheme so unlock can
  validate the password even when no credentials exist.
- The derived key is cached only in process memory for the app session. It is
  never persisted.

If the user forgets the credential password, saved database passwords are not
recoverable. The app offers **Reset credential storage**, which deletes saved
passwords and verifier metadata while preserving connection metadata and other
app data. Onboarding and encrypted-mode setup must show this disclaimer.

The backend no longer returns stored passwords to the frontend:

- `load_connections` returns connection metadata with an empty `password`.
- New connection saves send the password once.
- Edit connection with a blank password means "keep existing password".
- Non-empty edit password replaces the stored credential.
- Backend commands hydrate passwords internally before connecting.

Onboarding is blocking. Startup order is:

1. Open SQLite and run migrations.
2. Load `app_settings`.
3. If onboarding is incomplete, show onboarding.
4. If mode is encrypted SQLite and no session key is cached, show unlock.
5. Load app data.
6. Start foreground health checks.

Onboarding is marked complete only after credential storage is successfully
configured. Encrypted SQLite is the recommended default. Keychain remains
available but is not recommended. Unencrypted SQLite requires an
acknowledgement.

The sidebar footer settings icon opens a new system-wide Settings page. The
page owns credential storage configuration and future app-wide preferences.
Switching credential modes is a focused confirmation flow: credentials are
transferred automatically when possible, the active mode changes only after
the new backend write succeeds, and concurrent credential-storage changes are
blocked.

## Consequences

- Local persistence is easier to reason about: one SQLite database plus the
  optional keychain credential backend.
- ADR-0005's "single keychain blob with process cache" remains relevant only
  inside keychain mode. It is no longer the default credential architecture.
- ADR-0004's `lastActivityAt` timestamp still lives on the connection record,
  but updates now write the `connections` table instead of
  `connections.json`.
- ADR-0003's saved-query schema remains local and sync-ready, but the storage
  medium is SQLite rather than JSON.
- No migration means existing pre-production local JSON/keychain state will not
  appear after this change. Users re-create connections and credentials.
- Encrypted SQLite protects passwords at rest but does not encrypt the entire
  SQLite file. Metadata remains visible to anyone with file access.
- A future proposal to reintroduce credential JSON files should be rejected
  unless it identifies a clear advantage over the three accepted app-wide
  modes.
