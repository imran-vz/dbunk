# Plan 011: PostgreSQL connection security backend — TLS verification modes, client certificates, applied keepalive, and staged connection diagnosis

> **Executor instructions**: Plans 001–010 must be `DONE` in
> `plans/README.md` before starting. Follow this plan step by step. This is
> a **dark** plan: no user-visible UI change may land here — new fields,
> commands, error arms, and libraries ship unreferenced by any rendering
> path (adding optional serde/zod/TS-union members with defaults is
> invisible and allowed). Run every verification command and confirm the
> expected result before moving on. Update this plan's README row after
> each step and mark `READY FOR REVIEW` after all gates. A
> reviewer/operator records `DONE: <completion SHA>` after an authorized
> commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git status --short -- src src-tauri infrastructure plans/README.md
> git log --oneline -1
> grep -rn "tls_prefer" src-tauri/src | wc -l     # expect 14 hits at 4facea1
> grep -n "(\s*18," src-tauri/src/storage.rs        # expect no migration hit
> ```
>
> Expected on a fresh run: clean tree at or after `4facea1`. A load-bearing
> mismatch with the excerpts below is a STOP condition.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH (TLS verification semantics; a wrong default silently
  weakens every production connection)
- **Depends on**: Plans 001–010 complete
- **Category**: foundation (dark)
- **Planned at**: commit `4facea1`, 2026-08-24
- **Gap**: `PAR-006` in `plans/parity-gap-register.md`

## Review correction record

- **Step 2 (2026-08-24):** `rustls-pki-types` 1.14 ships PEM parsing
  unconditionally — there is no `pem` feature — so the direct dependency
  is `rustls-pki-types = "1"`. The fixture PEMs under
  `src-tauri/src/postgres/testdata/` were generated with a 20-year
  expiry.
- **Step 4 (2026-08-24):** the `tls_prefer` site list is 14 at
  `4facea1`, not 9 (the original plan text was corrected before
  execution). The new actor arm is `TlsFailed { tls_kind, message }`
  (`tlsKind` on the wire): the enums are internally tagged on `kind`,
  so a payload field named `kind` would collide with the tag.
- **Step 4 (2026-08-24):** the plan's "type-only" frontend touch was
  not achievable — `formatQuerySessionError`, `formatTableBrowseError`,
  `formatMutationError`, `formatSharedTransportError`, and
  `mutationClientErrorCopy` are exhaustive switches, so `typecheck`
  fails without a `tlsFailed` arm. Each gained the minimal arm (the
  shared formatter returns the backend message; the query-editor copy
  appends a one-line hint), and the three decoders
  (`decodeQuerySessionError`, `decodeTableBrowseError`,
  `decodeResultMutationError`) decode `tlsFailed` with a
  `TlsFailureKind` guard instead of collapsing it to `connectionLost`.
  No component renders differently for any error that existed before;
  Plan 012 still owns the real presentation.
- **Step 5 (2026-08-24):** the diagnosis performs the SSLRequest and the
  rustls handshake itself over the probe socket and then hands
  tokio-postgres the resulting stream with `SslMode::Disable`, so the
  TLS stage is attributable and `prefer`'s plaintext fallback is
  observed directly rather than inferred. Consequence recorded in
  ADR-0025: no SCRAM channel binding on the diagnosis path.
- **Step 5 (2026-08-24):** tokio-postgres's `Display` prints only its
  own kind ("error performing TLS handshake"); the detail ("server does
  not support TLS") lives in the source chain, so the classifier's
  message is the joined chain.
- **Step 6 (2026-08-24):** the shared `./postgres` init directory is
  mounted over `/docker-entrypoint-initdb.d`, which hid the image's
  `00-configure-tls.sh` — the pre-existing TLS fixture had never
  actually enabled SSL (`prefer` tests passed by falling back to
  plaintext). Docker also cannot create a single-file mountpoint inside
  a read-only directory mount. The fixture now builds an image whose
  init script is *sourced* (non-executable) and calls
  `docker_process_init_files /fixture-sql/*` over a side mount of the
  shared SQL, after enabling TLS and prepending the `hostssl … cert`
  rules. `003_tls_roles.sql` creates `dbunk_cert` on both fixtures.

## Why this matters

`PAR-006` names two halves: connection *organization* and connection
*security*. The organization half (folders, favorites, colors, recency,
Duplicate, secret-free Copy URI, Import-from-URI) shipped as Plans 009 and
010. What remains is the security half, and it is not cosmetic:

- dbunk cannot verify a PostgreSQL server certificate at all. Every TLS
  path in the backend is `prefer`-or-`disable`, and the dedicated
  tokio-postgres driver installs an `AcceptAllVerifier`
  (`src-tauri/src/postgres/dedicated.rs:173-229`). A connection labelled
  `production` with the SSL switch on is encrypted against a passive
  observer and nothing else — any interposed endpoint with any
  certificate is accepted.
- `require`, `verify-ca`, `verify-full`, CA files, and client
  certificates — the modes every certificate-enforced deployment (RDS
  with `rds.force_ssl`, Cloud SQL, Neon, Supabase, corporate CAs) asks
  for — do not exist in the model, the form, or any driver call.
- `keepalive_seconds` has been stored and round-tripped since ADR-0013 but
  is applied nowhere. The "sqlx 0.8 exposes no setter" rationale is now
  only half true: the dedicated driver's `tokio_postgres::Config` has
  `keepalives_idle`, and the shared connect spec is the exact place to set
  it (`src-tauri/src/postgres/connect_spec.rs:46-57`).
- Test Connection returns one string. Users cannot tell DNS from tunnel
  from TLS from authentication from a missing database, and the test
  probes sqlx-Any (`dispatch/relational.rs:790-818`), not the
  tokio-postgres driver the query editor actually uses — so a TLS setup
  can "test green" and then fail in a query tab.
- Four independent places decide TLS today (`postgres/pool.rs:35-39`,
  `postgres/dedicated.rs:65-81`, `dispatch/relational.rs:181`,
  `postgres/ddl.rs:35-38`). Any mode work that does not converge them
  creates a fifth divergence.

This plan lands every non-visual foundation the activation needs: a typed
TLS options model (migration 18), one shared TLS resolver consumed by all
four connect sites, certificate verification with native + user CA roots,
client-certificate authentication, tunnel-aware hostname verification,
applied keepalive on the dedicated driver, a typed `tls` error arm on the
actor unions, and a staged `diagnose_connection` command with a typed
per-stage report — so Plan 012 can activate the form and the diagnosis
panel against tested contracts.

## Reconciliation against the register (read before scoping questions)

**Already delivered by Plans 009/010 (do not re-plan):** connection
folders, favorites, colors, recency ordering, Duplicate, Copy URI (secret
free), Import-from-URI. The register's "groups, tags, colors, favorites,
and environment type" line is closed except **tags** (deferred below) and
**environment type**, which Plan 007/008 delivered.

**In scope here (PostgreSQL only, per ADR-0001):** TLS modes
`disable | prefer | require | verify-ca | verify-full`; CA / client cert /
client key selection as file paths with backend validation; hostname
verification that survives SSH tunnels; applied TCP keepalive on the
dedicated driver; a staged connection test that separates tunnel, DNS,
TCP, TLS, authentication, and database failures and reports whether the
session is actually encrypted.

**Deferred, staying in the register with rationale:**

- MySQL TLS modes and certificates — sqlx's MySQL options support them,
  but MySQL has no native driver, no pool, no driver options, and no
  live fixture in this repo; PostgreSQL-first rule. MySQL keeps its
  single `ssl` toggle and `ssl-mode=preferred|disabled` DSN unchanged.
- Connection **tags** (free multi-valued labels) — folders + favorites +
  colors cover the organization need; tags are a follow-on once a
  search surface would consume them.
- Encrypted (passphrase-protected) client keys — rustls' PEM loader does
  not decrypt; refused with a typed error naming the limitation.
- Certificate *contents* stored in the credential store (portable
  bundles) — paths only in v1, matching the bastion private-key-path
  precedent (`src/components/bastion-servers/bastion-form.tsx:132-140`)
  and libpq. Secure connection bundles / encrypted profile exchange stay
  deferred with `PAR-005`.
- IAM / external-secret adapters, driver manager, network profiles —
  `PAR-015` boundary.
- Keepalive on the sqlx metadata pool — sqlx 0.8 still exposes no socket
  setter; the limitation narrows to that one path and is disclosed.
- Hostname verification over an SSH tunnel on the sqlx metadata pool —
  sqlx has no `hostaddr` equivalent, so `verify-full` on that path over a
  tunnel verifies the CA chain only. Disclosed in the diagnosis report and
  ADR; the dedicated driver and `pg_dump`/`pg_restore` verify the
  hostname fully via `host` + `hostaddr` / `PGHOSTADDR`.

## Current state (verified at `4facea1`)

### Backend

- `src-tauri/src/types.rs:476-507` — `PgStoredConnection`; TLS is a single
  bool at `:496-500`:

  ```rust
  #[serde(default = "default_true")]
  pub ssl: bool,
  ```

  `PgDriverOptions` `:516-531` carries `keepalive_seconds: Option<u32>`
  (`:526`), stored, never applied. `MySqlStoredConnection` `:533-559` has
  the same `ssl: bool`. `set_network_endpoint` `:875-898` overwrites
  `host`/`port` in place. `ConnectResult` `:1031-1040`,
  `HealthCheckResult` `:1042-1049` (`tag = "state"` — the one typed enum
  in the legacy connections module), `TestConnectionPayload`
  `:1243-1247`.
- `src-tauri/src/postgres/connect_spec.rs:4-16` —
  `ResolvedPostgresConnectSpec { connection_id, host, port, database,
  user, password, tls_prefer: bool, connect_timeout, driver_options,
  safety_policy }`; `tokio_config()` `:46-57` sets host/port/dbname/
  user/password only; test `:77-107`
  `query_sessions_leave_keepalive_at_the_driver_default` asserts the
  2-hour default is untouched — **this test inverts in Step 4**.
  Consumed by `query_session/postgres.rs:35`, `table_browse/postgres.rs:20`,
  `result_mutation/postgres.rs:27`, `safety/live.rs:183`. `grep -rn
  "tls_prefer" src-tauri/src` is the authoritative site list (14 hits at
  `4facea1`: `connect_spec.rs` ×3 — struct, `from_postgres`, `Debug`;
  `dedicated.rs` ×3; `query_session/mod.rs`, `query_session/postgres.rs`
  ×2, `table_browse/executor.rs`, `table_browse/live.rs` ×2,
  `result_mutation/mod.rs`, `result_mutation/live.rs` — the non-live
  hits are spec construction/forwarding sites that must switch to
  `tls: ResolvedTls` alongside the builders).
- `src-tauri/src/postgres/dedicated.rs` — `DedicatedError` `:33-45`
  (`ConnectionLost | Timeout | Database`, **no TLS arm**); `connect`
  `:60-95` picks `SslMode::Prefer|Disable` and
  `MakeRustlsConnect::new(permissive_tls_config())`; `with_deadline`
  `:127-140` maps every connect error to `ConnectionLost`, **discarding
  the cause**; `cancel` `:142-155` reuses the permissive config;
  `AcceptAllVerifier` `:173-215`; `permissive_tls_config` `:217-229`
  (`RootCertStore::empty()` + `with_no_client_auth()` + dangerous
  verifier).
- `src-tauri/src/postgres/pool.rs:29-51` — `build_connect_options`:

  ```rust
  let ssl_mode = if connection.ssl {
      sqlx::postgres::PgSslMode::Prefer
  } else {
      sqlx::postgres::PgSslMode::Disable
  };
  ```

  No `ssl_root_cert` / `ssl_client_cert` / `ssl_client_key` (all exist
  on sqlx 0.8's `PgConnectOptions`). Connect deadline wrapper
  `:100-122`; `friendly_sqlx_error` at `:118,121`.
- `src-tauri/src/dispatch/relational.rs` — `friendly_sqlx_error`
  `:62-123` (refused / timeout / reset / unreachable / DNS-by-text /
  `Tls` / `Database` → `String`); PG DSN `:175-190` with
  `sslmode=prefer|disable`; `sqlx_connect` `:435-442`; `ping_connection`
  `:790-818` routes PostgreSQL through **sqlx-Any**, not the driver the
  app uses.
- `src-tauri/src/postgres/ddl.rs:17-42` — `pg_tool_command` sets
  `PGPASSWORD` and `PGSSLMODE=prefer|disable`; no `PGSSLROOTCERT` /
  `PGSSLCERT` / `PGSSLKEY` / `PGHOSTADDR`.
- `src-tauri/src/tunnel.rs:66-80` `resolve_connection` → `ensure_forward`
  `:139-193` (binds `local_bind_host`, default `127.0.0.1` `:26`,
  ephemeral port) → `tunnel/endpoint.rs:41-60`
  `rewrite_connection_endpoint`, whose `:58` `set_network_endpoint`
  **destroys the original hostname** — after tunnelling, every TLS path
  sees `host = 127.0.0.1`, so hostname verification would compare the
  certificate against the loopback address. `remote_endpoint` `:5-39`
  still has the original. Errors are `String` throughout.
- `src-tauri/src/commands/connections.rs:221-238` — `test_connection`:
  validates the tunnel, resolves through an ephemeral `test-<uuid>` route
  key, calls `dispatch::ping_connection`, drops the route. Returns
  `Result<ConnectResult, String>`. `save_connection_inner` `:30-46`
  already invalidates pools and tunnels on save
  (`socket_lifecycle::invalidate_connection_caches`,
  `socket_lifecycle.rs:85-96`), so a TLS edit takes effect on the next
  connect with no new work. `find_connection` (`commands/mod.rs:45-55`)
  hydrates the credential and resolves the tunnel for every command.
- `src-tauri/src/storage.rs:77` `MIGRATIONS`; migration 5 `:197-206`
  (`driver_options TEXT` JSON blob, "adding a new knob is a struct
  field, not a schema migration"); migration 17 `:410-420`; **next free
  slot is 18**. `CONNECTION_COLUMNS` `:709-716`; `driver_options` decode
  with warn-on-malformed `:802-814` — the pattern to copy. Test harness
  `test_pool` `:1997`, `test_pool_through(max_version)` `:2005`;
  `migration_17_defaults_legacy_rows_to_no_organization` `:2276` is the
  legacy-row test to copy.
- `src-tauri/Cargo.toml` — `sqlx` `runtime-tokio-rustls` (→
  `tls-rustls-ring-webpki`: Mozilla roots), `tokio-postgres = "=0.7.18"`,
  `tokio-postgres-rustls = "=0.14.0"`, `rustls 0.23` (`ring`, `std`,
  `tls12`). `rustls-native-certs 0.7.3`, `rustls-pki-types 1.14.1`, and
  `webpki-roots` are already in `Cargo.lock` transitively — promoting
  them to direct dependencies compiles nothing new.
- Crate facts relied on below (verified in the registry sources):
  `tokio_postgres::Config` has `hostaddr`, `keepalives`,
  `keepalives_idle`, `keepalives_interval`, `keepalives_retries`, and
  `connect_raw(stream, tls)`; `connect.rs:63-75` uses `host` as the TLS
  hostname and `hostaddr` for the socket. `PgSslMode` has
  `Require | VerifyCa | VerifyFull`; sqlx's rustls handshake uses
  `options.host` as the server name and, for `VerifyCa`, a
  hostname-ignoring verifier; roots = webpki **or** native store by
  feature, plus the `ssl_root_cert` file (union). `tokio-postgres`
  surfaces "server does not support TLS" via `Error::tls`
  (`connect_tls.rs:43`).
- Live fixtures: `infrastructure/test-db/compose.yml:24-43`
  `postgres-tls` on **15433** built from
  `infrastructure/test-db/postgres-tls/{Dockerfile,configure-tls.sh}` — a
  1-day **self-signed leaf** `CN=localhost`,
  `SAN=DNS:localhost,IP:127.0.0.1`, no CA, no client auth. Plain PG on
  **15432**. Ignored live tests use them: `query_session/postgres.rs:567-577`
  (`live_spec(port, tls_prefer)` at `:516-530`),
  `table_browse/live.rs:506-514`. `pnpm db:postgres-tls`
  (`package.json:32`). The `justfile` has no `--ignored` recipe.
- Typed error model to copy: `src-tauri/src/query_session/protocol.rs`
  `QuerySessionError` (`#[serde(rename_all = "camelCase",
  rename_all_fields = "camelCase", tag = "kind")]`); the same attribute
  block is on `TableBrowseError` and `ResultMutationError`. Each actor
  maps `DedicatedError::ConnectionLost → …::ConnectionLost` and
  `Timeout → Timeout` (`query_session/postgres.rs:63-64`,
  `table_browse/postgres.rs:33-34`, `result_mutation/postgres.rs:1343-1344`).

### Frontend (type-only touch points for this dark plan)

- `src/lib/store/types.ts:284-295` — `PgDriverOptions` with the stale
  comment "`connectTimeoutMs` and `keepaliveSeconds` are … not yet
  applied"; `:297-307` `PgStoredConnection { ssl, driverOptions?,
  sshTunnel? }`; `:327-340` Redis `verifyTlsCert` (the only
  verification toggle in the product); query-session error union
  `:430-442` (`{ kind: "connectionLost" }`, `{ kind: "timeout" … }`).
- `src/lib/store/connections.ts:185-190, 441-462` — `testConnection`
  returns `{ ok, latencyMs } | { ok: false, error: string }`.
- `src/components/connection-form/form-utils.ts:41` `ssl`, `:57-67`
  driver knobs (keepalive comment `:60-64`), `:72-108`
  `EMPTY_NEW_DEFAULTS`, `:188-210` `driverOptionsFromForm`;
  `use-connection-form.ts:26-30` `TestStatus`. **None of these render
  differently after this plan** — Plan 012 activates.

## Decided architecture

### 1. TLS options model (migration 18)

One nullable JSON blob column, mirroring migration 5 exactly:

```sql
ALTER TABLE connections ADD COLUMN tls_options TEXT;
```

```rust
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgTlsMode { Disable, #[default] Prefer, Require, VerifyCa, VerifyFull }

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgTlsOptions {
    #[serde(default)] pub mode: PgTlsMode,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub root_cert_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub client_cert_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub client_key_path: Option<String>,
    /// Hostname the server certificate must match when it differs from
    /// `host` (IP-literal hosts, SSH tunnels). Never persisted by the
    /// tunnel — see §4.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub server_name: Option<String>,
}
```

- `PgStoredConnection` gains
  `#[serde(default, skip_serializing_if = "Option::is_none")] pub tls_options: Option<PgTlsOptions>`.
  The wire vocabulary is libpq's (`disable`, `prefer`, `require`,
  `verify-ca`, `verify-full`) so URIs, `PGSSLMODE`, and the form share
  one set of strings.
- **`ssl` stays as the legacy on/off mirror.** Resolution lives in one
  method, `PgStoredConnection::resolved_tls_mode()`:
  `tls_options.map(|t| t.mode).unwrap_or(if ssl { Prefer } else { Disable })`.
  `storage::upsert_connection` normalizes `ssl = mode != Disable`
  whenever `tls_options` is present, so `ssl` can never disagree with
  the blob on disk. After Step 4, the only readers of `pg.ssl` in
  `src-tauri/src` are the resolver, the storage column threading, and
  the normalizer — the grep in Step 4 enforces it.
- Storage: read/decode with the `driver_options` warn-on-malformed
  pattern (`storage.rs:802-814`); NULL/empty → `None`; MySQL and other
  engines leave the column NULL. Legacy rows (no column value) resolve
  through `ssl` exactly as today — **no behaviour change for existing
  connections** (`ssl=true` → `prefer`, permissive).
- Paths are stored as typed-in strings; validation (exists, readable,
  parses) happens at connect/diagnose time, not at save time, so a
  connection can be saved on a machine where the file is not yet
  present. Client key material is a *path*: the key never enters the
  SQLite store or the credential blob.

### 2. `src-tauri/src/postgres/tls.rs` — one resolver, four renderers

```rust
pub(crate) struct ResolvedTls {
    pub mode: PgTlsMode,
    /// TLS server name for SNI + certificate matching. Equals `host`
    /// unless `PgTlsOptions.server_name` is set (user or tunnel).
    pub server_name: String,
    pub root_cert_path: Option<PathBuf>,
    pub client_cert_path: Option<PathBuf>,
    pub client_key_path: Option<PathBuf>,
}
```

- `ResolvedTls::from_postgres(pg: &PgStoredConnection) -> ResolvedTls`
  (pure; no I/O).
- **Renderer A — tokio-postgres / rustls**
  `client_config(&ResolvedTls) -> Result<rustls::ClientConfig, TlsMaterialError>`:
  - `Prefer` / `Require` → the existing accept-all verifier (renamed
    `NoVerification`) — encryption without authentication, exactly
    libpq's semantics for these modes. The permissive policy the two
    existing live TLS tests depend on is preserved for these modes only.
  - `VerifyCa` → roots = platform native store ∪ user CA file;
    `WebPkiServerVerifier` wrapped in `CaOnlyVerifier`, which passes
    through every result except
    `InvalidCertificate(NotValidForName | NotValidForNameContext { .. })`
    (mirrors sqlx's `NoHostnameTlsVerifier`).
  - `VerifyFull` → same roots, rustls' default verifier (chain +
    hostname).
  - Client auth: both `client_cert_path` and `client_key_path` →
    `with_client_auth_cert(chain, key)`; exactly one of them →
    `TlsMaterialError::ClientPairIncomplete`; PEM parsed via
    `rustls::pki_types::pem::PemObject` (`CertificateDer::pem_file_iter`,
    `PrivateKeyDer::from_pem_file`); an encrypted key →
    `TlsMaterialError::ClientKeyEncrypted`; unreadable/unparseable →
    `TlsMaterialError::Unreadable { path, detail }` /
    `Malformed { path, detail }`. Every error names the path.
  - The native root store loads once per process into a
    `std::sync::OnceLock<Arc<RootCertStore>>` (rustls-native-certs 0.7
    `load_native_certs()`); load failures `log::warn!` and yield an empty
    native set, so verification then depends on the user CA file alone
    — identical to sqlx's behaviour. `Disable` returns no config; the
    caller uses `NoTls`.
- **Renderer B — sqlx** `apply_to_pg_options(&ResolvedTls, PgConnectOptions) -> PgConnectOptions`:
  direct `PgSslMode` mapping plus `ssl_root_cert` / `ssl_client_cert` /
  `ssl_client_key` paths. **Tunnel limitation:** when `mode ==
  VerifyFull` and `server_name != host`, sqlx (no `hostaddr`) would
  verify against the loopback address and always fail; the renderer
  uses `VerifyCa` for that one path and `log::warn!`s once per
  connection id. The diagnosis report carries this as
  `poolHostnameVerification: "caOnly"` (§5) so it is never silent.
  Cargo: replace `runtime-tokio-rustls` with `runtime-tokio` +
  `tls-rustls-ring-native-roots` so the pool and the dedicated driver
  trust the **same** root set (native ∪ file); today's modes never
  verify, so this changes no existing behaviour.
- **Renderer C — DSN** `dsn_query(&ResolvedTls) -> String`:
  `sslmode=<mode>` plus `sslrootcert`/`sslcert`/`sslkey` when set
  (sqlx's URL parser accepts them). Used by `sqlx_dsn`'s PG branch so no
  PG DSN consumer is left on `prefer|disable`.
- **Renderer D — libpq subprocess** `apply_to_command(&ResolvedTls, host, &mut Command)`:
  `PGSSLMODE=<mode>`, `PGSSLROOTCERT`/`PGSSLCERT`/`PGSSLKEY` when set,
  and when `server_name != host`: `--host <server_name>` with
  `PGHOSTADDR=<host>` — libpq then verifies the certificate against the
  real hostname while connecting to the tunnel's loopback address.
  `PGPASSWORD` handling is unchanged.
- Unit tests: mode → `SslMode` / `PgSslMode` / `PGSSLMODE` tables; the
  VerifyFull-over-tunnel downgrade on the sqlx renderer only; DSN
  rendering with percent-encoded paths; client-pair-incomplete refusal;
  encrypted-key refusal; missing file names the path; committed
  test-only PEM fixtures under `src-tauri/src/postgres/testdata/`
  (a throwaway CA, leaf, client cert/key, and an encrypted key — not
  secrets, generated for the tests, expiry far in the future).

### 3. Connect spec and the four sites converge

- `ResolvedPostgresConnectSpec`: `tls_prefer: bool` → `tls: ResolvedTls`;
  new `keepalive: Option<Duration>` from `driver_options.keepalive_seconds`
  (`Some(n)` with `n > 0`). `Debug` prints `mode` and `server_name`,
  never paths' contents (paths are fine).
- `tokio_config()`: adds `keepalives(true)` + `keepalives_idle(keepalive)`
  when set (interval/retries stay OS defaults, like libpq); sets
  `host(server_name)`. When `server_name != host`, `dedicated::connect`
  resolves `host` to an `IpAddr` (parse first; else
  `tokio::net::lookup_host` under the connect deadline) and calls
  `hostaddr(ip)` — tokio-postgres then connects to the tunnel and
  verifies the certificate against the real hostname. `ssl_mode` maps
  `Disable → Disable`, `Prefer → Prefer`, everything else → `Require`
  (verification is rustls' job). Invert the `connect_spec.rs:77-107`
  test: keepalive 15 → `get_keepalives_idle() == 15s`; add a sibling
  asserting `None` leaves the driver default.
- `dedicated.rs`: `connect` and `cancel` take their `rustls::ClientConfig`
  from `tls::client_config(&spec.tls)`; `DedicatedConnection.tls: bool`
  becomes `tls: Option<Arc<rustls::ClientConfig>>` so cancel reuses the
  identical config (a `verify-full` session must not cancel over an
  unverified socket). `with_deadline` stops discarding the cause: a new
  `DedicatedError::Tls { kind: TlsFailureKind, message: String }` arm
  is produced by `classify_connect_error` (§5) for TLS-material and
  handshake failures; everything else keeps today's mapping.
  `TlsMaterialError` surfaces before any socket is opened.
- The three actors map the new arm to a new typed union member each —
  `QuerySessionError::TlsFailed { kind, message }`,
  `TableBrowseError::TlsFailed { .. }`, `ResultMutationError::TlsFailed { .. }`
  — and `src/lib/store/types.ts` gains the matching
  `{ kind: "tlsFailed"; kind…: TlsFailureKind; message: string }`
  members (type-only; no renderer reads them until Plan 012). Today
  these failures surface as `connectionLost`, which is wrong and
  undiagnosable.
- `pool.rs`: `build_connect_options` uses renderer B; keepalive is
  documented as not applied on this path (sqlx limitation) in the
  function comment.
- `dispatch/relational.rs`: PG DSN branch uses renderer C.
  `ping_connection`'s PG arm routes through `crate::postgres::connect`
  (the pooled sqlx path with full TLS options) + `SELECT 1`, so
  `connect_connection` / `health_check_connection` exercise the same
  TLS decision as every metadata query. MySQL/SQLite arms unchanged.
- `ddl.rs`: `pg_tool_command` uses renderer D.
- Every `live_spec` builder gains `tls: ResolvedTls { mode, server_name:
  "127.0.0.1", .. }` — the helper `ResolvedTls::plain(host)` /
  `ResolvedTls::prefer(host)` keeps the fixtures short.

### 4. Tunnel carries the real hostname

`tunnel/endpoint.rs::rewrite_connection_endpoint`: for
`StoredConnection::PostgreSQL`, before `set_network_endpoint`, if
`tls_options.server_name` is `None`, set it to the pre-rewrite `host`
(trimmed). This happens on the resolved **clone** inside
`resolve_connection`; the saved record is never touched (the save path
does not go through `resolve_connection`). A user-supplied
`server_name` always wins. Test: rewriting a PG connection with host
`db.internal` yields `host = 127.0.0.1`, `server_name = "db.internal"`;
a pre-set `server_name` is preserved; MySQL is untouched.

### 5. `diagnose_connection` (backend command, dark)

`diagnose_connection(payload: DiagnoseConnectionPayload) -> Result<ConnectionDiagnosis, String>`
with `DiagnoseConnectionPayload { connection: StoredConnection,
hydrate_credential_from: Option<String> }`. The outer `Err` is reserved
for validation and credential-store failures (unchanged from
`test_connection`); **every probe failure lands inside the report**.
When `hydrate_credential_from` is `Some(id)` and the payload's password
is empty, the stored credential for `id` is hydrated backend-side
(`credentials::hydrate`) — this is what lets Plan 012 offer Test
Connection in edit mode, where the form holds a blanked password. The
secret never crosses IPC.

```rust
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionDiagnosis {
    pub engine: DatabaseEngine,
    pub stages: Vec<DiagnosisStage>,          // fixed order: tunnel, dns, tcp, tls, authentication, database
    pub outcome: DiagnosisOutcome,            // tag "kind": reachable { latency_ms } | failed { stage }
    pub warnings: Vec<DiagnosisWarning>,      // notEncrypted | poolHostnameVerificationCaOnly | productionWithoutVerification
}
pub(crate) struct DiagnosisStage { pub stage: DiagnosisStageKind, pub result: StageResult }
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(crate) enum StageResult {
    Passed { elapsed_ms: u64, detail: Option<StageDetail> },
    Failed { elapsed_ms: u64, kind: FailureKind, message: String },
    Skipped { reason: SkipReason },           // noTunnel | tlsDisabled | blockedByEarlierFailure | notApplicable
}
#[serde(tag = "kind", …)]
pub(crate) enum StageDetail {
    Tunnel { local_endpoint: String },
    Dns { addresses: Vec<String> },
    Tls { encrypted: bool, protocol: Option<String>, cipher: Option<String>,
          certificate_verified: bool, hostname_verified: bool,
          client_certificate_presented: bool,
          pool_hostname_verification: PoolHostnameVerification /* full | caOnly | notApplicable */ },
    Database { server_version: String },
}
pub(crate) enum FailureKind {
    TunnelFailed, DnsUnresolvable, ConnectionRefused, TimedOut, Unreachable,
    ServerRefusedTls, CertificateUntrusted, HostnameMismatch,
    ClientCertificateRejected, InvalidLocalMaterial,
    AuthenticationFailed, DatabaseMissing, Other,
}
```

`TlsFailureKind` (used by the actor arms in §3) is the TLS subset:
`ServerRefusedTls | CertificateUntrusted | HostnameMismatch |
ClientCertificateRejected | InvalidLocalMaterial | HandshakeFailed`.

PostgreSQL stage procedure (stepwise, so each failure is attributable):

1. **tunnel** — `Skipped(noTunnel)` unless enabled; otherwise
   `tunnel::resolve_connection` under an ephemeral `diag-<uuid>` route
   key that is dropped on every exit path (the `test_connection`
   pattern). Failure kind `TunnelFailed` with the tunnel's message.
2. **dns** — `tokio::net::lookup_host((host, port))` on the resolved
   host, capped at 5 s; `Passed` with the address list; failure
   `DnsUnresolvable` (or `TimedOut`).
3. **tcp** — `TcpStream::connect(first address)` under the spec's
   connect deadline (default 10 s when unset); `ConnectionRefused` /
   `TimedOut` / `Unreachable` from `io::ErrorKind`.
4. **tls + authentication** — `tls::client_config` (material errors →
   `tls: Failed(InvalidLocalMaterial)` before any handshake), then
   `MakeRustlsConnect::new(config).make_tls_connect(server_name)` and
   `config.connect_raw(stream, tls)` over the stage-3 socket. One call
   covers the SSL request, handshake, startup, and authentication;
   `classify_connect_error` attributes it: `as_db_error()` with
   SQLSTATE `28P01`/`28000` → `authentication: Failed(AuthenticationFailed)`;
   `3D000` → `database: Failed(DatabaseMissing)`; other DB errors →
   `database: Failed(Other)` with the SQLSTATE in the message; a
   tokio-postgres TLS error → `tls: Failed` with kind from the
   `rustls::Error` in the source chain (`UnknownIssuer | Expired |
   NotValidYet | InvalidPurpose | BadSignature → CertificateUntrusted`;
   `NotValidForName* → HostnameMismatch`; `AlertReceived(_) →
   ClientCertificateRejected` when a client cert was configured, else
   `HandshakeFailed`; message "server does not support TLS" →
   `ServerRefusedTls`); anything else → `tls: Failed(Other)` when the
   handshake had begun, `tcp: Failed(Other)` otherwise. The classifier
   operates on a small extracted `ConnectErrorView { sqlstate,
   rustls: Option<rustls::Error>, io_kind, message, client_cert_configured }`
   so it is unit-testable without constructing `tokio_postgres::Error`.
   With `mode == Prefer` and a server that refuses SSL, tokio-postgres
   continues in plaintext: `tls: Passed { encrypted: false, … }` plus
   warning `notEncrypted`. With `mode == Disable`: `tls: Skipped(tlsDisabled)`,
   warning `notEncrypted`.
5. **database** — over the live client: `SELECT current_setting('server_version')`
   and `SELECT ssl, version, cipher, client_dn IS NOT NULL FROM pg_stat_ssl WHERE pid = pg_backend_pid()`
   (present on every supported server). Fills the TLS detail
   (`encrypted`, `protocol`, `cipher`, `client_certificate_presented`)
   from the server's view, which is the only honest source; the
   `certificate_verified` / `hostname_verified` flags are derived from
   `mode` (`verify-ca` → verified/not; `verify-full` → both; else
   neither), `pool_hostname_verification` from §2's rule. Outcome
   `reachable { latency_ms }` = wall time of stages 3–5.
   `productionWithoutVerification` is added when `environment ==
   Production` and `mode` is not `verify-ca`/`verify-full`.

Non-PostgreSQL engines: `tunnel` (if configured) then one `database`
stage that wraps today's `dispatch::ping_connection` (`Failed(Other)`
with its string), other stages `Skipped(notApplicable)`. This lets Plan
012 switch the form wholesale.

Registered in `lib.rs` next to `test_connection`, which stays untouched
here (Plan 012 removes it after the form switches).

### 6. Live fixture: a real CA, server cert, and client cert

`infrastructure/test-db/postgres-tls/` gains `gen-certs.sh` (host-side
`openssl`, writes to `certs/`, gitignored, idempotent):
`ca.crt`/`ca.key`; `server.crt` signed by the CA with
`SAN=DNS:localhost,IP:127.0.0.1`; `client.crt`/`client.key` with
`CN=dbunk_cert`; and `client-encrypted.key` (passphrase `dbunk`) for the
refusal test. `compose.yml` mounts `./postgres-tls/certs:/certs:ro`;
`configure-tls.sh` copies them into `$PGDATA` with the right ownership
and mode, sets `ssl_ca_file = 'ca.crt'`, and prepends to `pg_hba.conf`:
`hostssl all dbunk_cert 0.0.0.0/0 cert clientcert=verify-full` (the
existing `dbunk` password rule stays, so every current TLS test keeps
passing). `postgres/*.sql` init gains `CREATE ROLE dbunk_cert LOGIN;
GRANT CONNECT ON DATABASE dbunk_demo TO dbunk_cert;`. The `Makefile`
`postgres-tls` target depends on a `certs` target that runs
`gen-certs.sh` when `certs/ca.crt` is absent. Tests resolve paths
relative to `CARGO_MANIFEST_DIR`.

### 7. ADR-0025

`docs/adr/0025-postgres-tls-verification-and-staged-diagnosis.md`
records: the libpq mode vocabulary; `prefer` as the compatibility
default for legacy rows; native ∪ file root policy (and why union, not
replace: sqlx cannot replace); paths not contents; the `server_name`
carry-over and the sqlx-pool CA-only limitation over tunnels; keepalive
applied on the dedicated driver only; the staged diagnosis contract and
its mistakes-not-adversaries framing (a misconfigured CA or hostname is
the target, not an active attacker on the loopback interface). It
supersedes the "TLS Disable/Prefer semantics … permissive certificate
policy" sentence in ADR-0021 (`:31-34`) and amends the ADR-0013 keepalive
bullet (`:52-56`); both get a one-line pointer.

## Commands you will need

```sh
just fmt && just lint && just test
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored tls          # after pnpm db:postgres-tls
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored diagnos      # after pnpm db:postgres + db:postgres-tls
pnpm format && pnpm lint && pnpm typecheck && pnpm vitest run
grep -rn "tls_prefer" src-tauri/src                                       # must be empty after Step 4
grep -rn "\.ssl\b" src-tauri/src --include=*.rs                           # allowed sites listed in Step 4
grep -n "driver_options" src-tauri/src/storage.rs                         # column-threading site list to mirror
pnpm db:postgres-tls   # rebuilds the image; run after editing the fixture
```

## Scope

Expected files touched (creation marked ＋):

- `src-tauri/Cargo.toml` — sqlx features (`runtime-tokio`,
  `tls-rustls-ring-native-roots`), direct `rustls-native-certs = "0.7"`,
  `rustls-pki-types = { version = "1", features = ["pem", "std"] }`.
- ＋ `src-tauri/src/postgres/tls.rs` (+ inline tests),
  ＋ `src-tauri/src/postgres/testdata/*.pem`.
- `src-tauri/src/types.rs` — `PgTlsMode`, `PgTlsOptions`,
  `PgStoredConnection.tls_options`, `resolved_tls_mode`, diagnosis
  types, `DiagnoseConnectionPayload`.
- `src-tauri/src/storage.rs` — migration 18, column threading,
  normalization, tests.
- `src-tauri/src/postgres/connect_spec.rs`, `dedicated.rs`, `pool.rs`,
  `ddl.rs`, `mod.rs` (module registration).
- `src-tauri/src/dispatch/relational.rs` — DSN branch, `ping_connection`.
- `src-tauri/src/tunnel/endpoint.rs` (+ test).
- `src-tauri/src/query_session/{protocol.rs,postgres.rs}`,
  `table_browse/{protocol.rs,postgres.rs,live.rs}`,
  `result_mutation/{protocol.rs,postgres.rs,live.rs}`, `safety/live.rs`
  — `TlsFailed` arms and spec builders.
- ＋ `src-tauri/src/diagnosis.rs` (stages, classifier, tests) and
  ＋ `src-tauri/src/commands/diagnosis.rs`; `src-tauri/src/lib.rs`
  registration.
- `infrastructure/test-db/postgres-tls/*`, `compose.yml`, `Makefile`,
  `postgres/*.sql`, `.gitignore` (certs dir).
- `src/lib/store/types.ts` — `PgTlsMode`, `PgTlsOptions`,
  `PgStoredConnection.tlsOptions?`, `ConnectionDiagnosis` mirrors, the
  three `tlsFailed` union members (type-only).
- `src/components/connection-form/form-utils.ts` — thread `tlsOptions`
  through `buildStoredConnectionFromForm` / `defaultValuesFromConnection`
  so a save cannot wipe a stored blob (the same reason `keepaliveSeconds`
  is in the schema today); **no field renders**. Its test's
  exact-equality fixtures gain the field.
- ＋ `docs/adr/0025-…md`; one-line pointers in ADR-0013 and ADR-0021.
- `plans/README.md` — status row.

Out of scope (STOP if you find yourself editing them): any rendering
component (`connection-form.tsx`, `connection-form/*-fields.tsx`,
`form-footer.tsx`), `use-connection-form.ts`, `connections.ts` store
actions, `connection-uri.ts`, MySQL TLS, `managed.rs` (local Docker,
TLS off by design), ROADMAP/PENDING_TASKS/CONTEXT truth pass (Plan 012).

## Resume protocol

Each step ends with all gates green. If interrupted, re-run the step's
verification commands; a red gate means the step is not done regardless
of what the README row says. Never mark a later step done while an
earlier step's gate is red. Live (`--ignored`) tests are required for
Steps 4 and 6 — a session without Docker records `BLOCKED: live fixture
unavailable` on the README row rather than skipping them.

## Git workflow

Work on the current branch in the working tree. **No commits, pushes, or
PRs without explicit operator authorization** (repo rule). The completion
SHA is recorded by the operator after review.

## Steps

### Step 1: Record contract decisions

Confirm against the working tree: migration slot 18 is free; the
`tls_prefer` grep returns the 14 sites listed above; `dedicated.rs`
`with_deadline` still discards the error; `rewrite_connection_endpoint`
still clobbers `host` without preserving it; `ping_connection`'s PG arm
still uses sqlx-Any; `rustls-native-certs 0.7` and `rustls-pki-types`
are in `Cargo.lock`. Record deviations in this file under a
`### Review correction record` heading before proceeding.

### Step 2: TLS material module + Cargo

Land `postgres/tls.rs` per §2 with the four renderers, `NoVerification`,
`CaOnlyVerifier`, the native-root `OnceLock`, PEM loading, typed
`TlsMaterialError`, and the fixture PEMs. Switch the sqlx features and add
the direct deps. Unit tests enumerated in §2. Nothing calls the module
yet. Gate: `just fmt && just lint && just test`.

### Step 3: Migration 18 + `PgTlsOptions` round-trip

Land the SQL, the Rust types, `resolved_tls_mode`, storage decode with
warn-on-malformed, and upsert normalization of `ssl`. Tests copying the
migration-17 pattern: legacy row through `test_pool_through(17)` →
`tls_options == None` and `resolved_tls_mode()` follows `ssl`; save with
each mode + paths → identical values return; `mode: disable` saved with
`ssl: true` → `ssl` reads back `false`; malformed JSON → `None` with a
warning; MySQL row leaves the column NULL. Thread `tlsOptions` through
`form-utils.ts` (type + build + defaults; no field). Gates: `just` trio +
`pnpm format && pnpm lint && pnpm typecheck && pnpm vitest run
src/components/connection-form/form-utils.test.ts`.

### Step 4: Converge the four TLS sites, tunnel server name, keepalive, typed TLS arm

Per §3 and §4. Replace every `tls_prefer` site; `grep -rn "tls_prefer"
src-tauri/src` must be empty; `grep -rn "\.ssl\b" src-tauri/src` may hit
only `types.rs` (`resolved_tls_mode`, `set_network_endpoint` region if
untouched), `storage.rs` (column threading, normalizer), and test
literals. Invert the keepalive test. Add the endpoint test. Add the
`TlsFailed` arms and their TS mirrors. Live: with `pnpm db:postgres-tls`
up, the two existing ignored TLS tests still pass (prefer stays
permissive); add `dedicated_live_verify_full_with_ca_connects`,
`dedicated_live_verify_ca_without_ca_is_untrusted` (expects
`DedicatedError::Tls { kind: CertificateUntrusted, .. }`),
`dedicated_live_verify_full_wrong_server_name_is_mismatch`
(`server_name: "wrong.example"` → `HostnameMismatch`),
`dedicated_live_client_certificate_authenticates` (user `dbunk_cert`,
no password), `dedicated_live_prefer_against_plaintext_server_downgrades`
(15432 → `client` works, `pg_stat_ssl.ssl == false`), and
`dedicated_live_keepalive_is_applied` (asserts via the `Config` getters
before connect — socket-level assertion is not portable). Gates: `just`
trio, `cargo test … -- --ignored tls`, and the frontend trio (types
changed).

### Step 5: `diagnose_connection`

Per §5: report types, stage runner, `classify_connect_error` over
`ConnectErrorView`, non-PG fallback, credential hydration by id,
ephemeral tunnel route cleanup on every path, `lib.rs` registration, TS
mirrors. Unit tests for the classifier (every `FailureKind` branch from a
constructed view, including the `AlertReceived` split on
`client_cert_configured`) and for stage ordering/skipping (a failed
`dns` yields `tcp/tls/authentication/database` all
`Skipped(blockedByEarlierFailure)` and `outcome.failed.stage == dns`).
Gate: `just` trio + frontend trio.

### Step 6: Fixture with real CA + diagnosis live tests

Per §6. Then ignored live tests in `diagnosis.rs`: wrong password →
`authentication: Failed(AuthenticationFailed)`, everything before it
`Passed`; nonexistent database → `database: Failed(DatabaseMissing)`;
closed port (15499) → `tcp: Failed(ConnectionRefused)`; host
`nonexistent.invalid` → `dns: Failed(DnsUnresolvable)`; `verify-full` +
CA against 15433 → all `Passed`, `Tls { encrypted: true,
certificate_verified: true, hostname_verified: true }`; `prefer` against
15432 → `Passed` with `encrypted: false` and warning `notEncrypted`;
`disable` → `tls: Skipped(tlsDisabled)`; client cert as `dbunk_cert` →
`client_certificate_presented: true`; `environment: production` +
`prefer` → warning `productionWithoutVerification`; an encrypted client
key → `tls: Failed(InvalidLocalMaterial)` naming the path. Gates: all
six plus both `--ignored` runs.

### Step 7: ADR-0025 and pointers

Per §7. Gate: `pnpm format` (markdown) and a re-run of the full suites
to confirm nothing drifted. Mark `READY FOR REVIEW`.

## Test plan

Everything enumerated in Steps 2–6. Steps 2, 3, 5 need no live server.
Steps 4 and 6 need `pnpm db:postgres` (15432) and `pnpm db:postgres-tls`
(15433, rebuilt with the new fixture). The frontend suite must stay
green with type-only changes.

## Done criteria

- Migration 18 applied on a legacy-shaped database in tests; legacy rows
  behave exactly as before (`prefer`/`disable` by `ssl`).
- One `ResolvedTls` drives the dedicated driver, the sqlx pool, the DSN,
  and `pg_dump`/`pg_restore`; `tls_prefer` no longer exists.
- `verify-ca` and `verify-full` genuinely reject an untrusted chain and a
  mismatched hostname on the dedicated driver (live tests), and
  hostname verification survives SSH tunnels there and on the libpq
  tools; the sqlx-pool CA-only limitation is disclosed, not hidden.
- Client-certificate authentication works end to end against the
  fixture; incomplete or encrypted material is refused with a path.
- `keepalive_seconds` is applied on the dedicated driver; the inverted
  test proves it.
- TLS failures at session/browse/mutation open surface as typed
  `tlsFailed`, not `connectionLost`.
- `diagnose_connection` attributes every failure class to a stage and
  reports real encryption state from `pg_stat_ssl`.
- ADR-0025 written; ADR-0013/0021 pointers in place.
- All six gates green plus both `--ignored` suites; zero rendering diffs
  (`git diff --stat -- src` shows only `types.ts`, `form-utils.ts`, and
  `form-utils.test.ts`).

## STOP conditions

- Migration slot 18 is taken, or the `connections` schema differs from
  the excerpts.
- `tokio-postgres 0.7.18` or `tokio-postgres-rustls 0.14.0` is no longer
  pinned (the `hostaddr` / `connect_raw` / `make_tls_connect` contract
  is version-specific).
- Any step requires editing a rendering component to keep gates green.
- Implementing `verify-full` on the sqlx pool over a tunnel turns out to
  need a sqlx fork or a native-tls switch — record the limitation, do
  not fork.
- A live test needs the accept-all verifier for `verify-ca` /
  `verify-full` to pass — that is the bug this plan exists to fix.

## Maintenance notes

- `ResolvedTls::from_postgres` and `resolved_tls_mode` are the only
  readers of the TLS fields; a new connect site must call a renderer,
  never `pg.ssl`.
- Adding a `PgTlsMode` is a five-place change: the enum, the four
  renderers, and the diagnosis `certificate_verified`/`hostname_verified`
  derivation — the mode → semantics tests will fail until all are done.
- The native root store is loaded once per process; a CA installed
  while dbunk is running needs a restart (document in Plan 012's UI
  copy if a user hits it).
