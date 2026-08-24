# Plan 012: TLS controls, staged connection diagnosis, and the connection-security truth pass

> **Executor instructions**: Do not start until Plan 011 is `DONE` in
> `plans/README.md`. Follow this plan step by step. Step 1 ends in a STOP
> for operator mock selection — do not write any TSX before a mock is
> selected and recorded. Run every verification command and confirm the
> expected result before moving on. Update this plan's README row after
> each step and mark `READY FOR REVIEW` after all gates. A
> reviewer/operator records `DONE: <completion SHA>` after an authorized
> commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git diff --stat <PLAN_011_COMPLETION_SHA>..HEAD -- src src-tauri plans/README.md plans/mocks/connection-security
> git status --short -- src src-tauri plans/README.md plans/mocks/connection-security
> ```
>
> Expected on a fresh run: no `src` or `src-tauri` output. A load-bearing
> mismatch with the excerpts below is a STOP condition.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MEDIUM (the backend semantics are Plan 011's; this plan's
  risk is presenting them inaccurately — e.g. a green test banner over a
  plaintext session)
- **Depends on**: Plan 011 complete
- **Category**: direction
- **Planned at**: commit `4facea1`, 2026-08-24
- **Gap**: `PAR-006` in `plans/parity-gap-register.md`

## Why this matters

Plan 011 ships contracts nobody can reach: TLS modes no form can select,
certificate paths no field accepts, a keepalive that applies but has no
control, typed `tlsFailed` errors every renderer still shows as
"connection lost", and a staged diagnosis no button invokes — while the
old Test Connection keeps reporting one string from a driver the app
does not use for queries. This plan activates all of it and ends with the
documentation truth pass the register (`PAR-017`) demands, because three
documents and two source comments currently state that keepalive cannot
be applied.

## Required Plan 011 contract

A mismatch is a STOP condition, not permission to re-implement here:

- `PgStoredConnection.tlsOptions?: PgTlsOptions` with
  `mode: "disable" | "prefer" | "require" | "verify-ca" | "verify-full"`,
  `rootCertPath?`, `clientCertPath?`, `clientKeyPath?`, `serverName?`;
  `ssl` still present and normalized backend-side to `mode !== "disable"`.
  `form-utils.ts` already threads `tlsOptions` through
  `buildStoredConnectionFromForm` / `defaultValuesFromConnection`
  (blob passthrough only — no per-field form values yet).
- `diagnose_connection(payload: { connection, hydrateCredentialFrom?: string })
  -> ConnectionDiagnosis` — `{ engine, stages: DiagnosisStage[],
  outcome, warnings }`, stages in fixed order `tunnel, dns, tcp, tls,
  authentication, database`, each `{ status: "passed", elapsedMs, detail? }
  | { status: "failed", elapsedMs, kind, message } | { status: "skipped", reason }`;
  the `tls` detail carries `encrypted`, `protocol`, `cipher`,
  `certificateVerified`, `hostnameVerified`,
  `clientCertificatePresented`, `poolHostnameVerification`; warnings
  `notEncrypted | poolHostnameVerificationCaOnly | productionWithoutVerification`.
  The outer promise rejects only for validation / credential-store
  failures.
- `keepaliveSeconds` is applied on the dedicated driver (query sessions,
  table browse, result mutation) and **not** on the sqlx metadata pool.
- The query-session, table-browse, and result-mutation error unions
  carry `{ kind: "tlsFailed"; … }` with a `TlsFailureKind` and a message.
- `test_connection` still exists and is unchanged (this plan deletes it).

## Current frontend state (verified at `4facea1`)

- `src/components/connection-form.tsx:208-226` — `showSslToggle` from
  the `host-auth` policy renders `<PgMySqlSslToggle>` (`:243-257`), which
  branches on engine to `<PgFields>` / `<MySqlFields>`; both are thin
  aliases of `<HostAuthSslField>` (`connection-form/host-auth-fields.tsx:10-24`,
  one `ToggleSwitchRow` "SSL — Negotiate TLS on the wire protocol").
  Advanced block `:228-238`: `SafetyFields`, `RoleField`, engine
  extras, `DriverOptionsFields` (PG only), `TunnelFields`.
- `src/lib/engine-policy.ts:61-77` — `host-auth` policy `{ defaultPort,
  showSslToggle, showDriverOptions }`; PG `:212-217`, MySQL `:228-233`.
  ADR-0012: gate by policy, not by `engine ===`.
- `src/components/connection-form/driver-options-fields.tsx` — header
  comment `:12-14` says keepalive is "intentionally absent … sqlx 0.8
  exposes no socket keepalive setter"; fields: statement timeout, idle
  timeout, connect timeout, search path, default role.
- `src/components/connection-form/form-utils.ts` — zod schema `:27-68`
  (`ssl` `:41`; keepalive comment `:60-64`), `EMPTY_NEW_DEFAULTS`
  `:72-108`, `driverOptionsFromForm` `:188-210`;
  `use-connection-form.ts:26-30` `TestStatus`, `:136-147`
  `runTestConnection`, `:167-196` `resetEngineSpecificDefaults`
  (`ssl: true` for PG/MySQL), `:198-205` `resetDriverOptions`.
- `src/components/connection-form/form-footer.tsx:38-50` — one-line
  success / error banners; `:52-65` the Test button renders in `new`
  mode only (edit mode holds a blanked password, so a test would fail
  auth — Plan 011's `hydrateCredentialFrom` removes that reason).
- `src/lib/store/connections.ts:185-190, 441-462` — `testConnection`
  → `test_connection`; `errorToMessage` (`src/lib/tauri.ts:146-163`).
- Select precedent: `connection-form/safety-fields.tsx:42-80`
  (`Select`/`SelectTrigger`/`SelectItem` with a description row). Path
  input precedent: `bastion-servers/bastion-form.tsx:132-140` (`TextField`
  with a placeholder path — **no file-dialog plugin is installed**;
  typed paths are the convention).
- URI: `src/lib/connection-uri.ts` reports every query parameter under
  `ignoredParams` (`:188`); `uri-import-field.tsx:73-75` renders the
  "Ignored parameters" note.
- Typed-error render sites that branch on `"connectionLost"`:
  `src/components/query-editor-panel.tsx:1707`,
  `src/components/mutation-review/model.ts:152`,
  `src/components/table-editor/use-table-session.ts:67`,
  `src/lib/query-session-channel.ts:147,187`; message formatters
  `src/lib/query-session-error.ts:100` (`formatQuerySessionError`),
  `src/lib/table-browse-error.ts:66` (`formatTableBrowseError`),
  `src/lib/safety-policy.ts:159` (`formatSharedTransportError`).
  Tests: `src/lib/result-mutation.test.ts`, `src/lib/table-browse-error.test.ts`.
- Docs that will be false once Plan 011 lands: `ROADMAP.md:25`
  ("TCP keepalive still pending"), `ROADMAP.md:38` ("sqlx 0.8 exposes no
  socket setter, so it has no control"), `docs/PENDING_TASKS.md:32`,
  `CONTEXT.md:24-28` (glossary: `ssl: boolean` is the whole TLS story),
  `src/lib/store/types.ts:284-287` (comment), plus the two source
  comments above. ADR-0013 / ADR-0021 pointers are Plan 011's.

## Decided frontend architecture

### TLS controls (PostgreSQL)

1. `engine-policy.ts`: the `host-auth` policy's `showSslToggle: boolean`
   becomes `tlsControls: "postgres-modes" | "toggle"` (PG →
   `"postgres-modes"`, MySQL → `"toggle"`); `connection-form.tsx`
   branches on the policy value, and `PgMySqlSslToggle`'s engine branch
   goes away. `<MySqlFields>` keeps `<HostAuthSslField>` unchanged.
2. New `connection-form/tls-fields.tsx` (`<TlsFields>`), rendered where
   the SSL toggle was for PG:
   - **TLS mode** `Select` with the five libpq modes and one-line
     descriptions: *Disable — plaintext*; *Prefer (default) — encrypt
     when the server offers it, no certificate check*; *Require —
     always encrypt, no certificate check*; *Verify CA — encrypt and
     verify the certificate chain*; *Verify full — verify the chain and
     that the certificate matches the host*. Selection writes
     `tlsMode` and derives `ssl` (`buildPostgres` sets
     `ssl = tlsMode !== "disable"`).
   - **CA certificate path** (shown for `verify-ca` / `verify-full`;
     helper text: "Optional — the system trust store is used when
     empty").
   - **Client certificate path** + **Client key path** (shown for every
     non-`disable` mode; zod `superRefine`: both or neither; helper
     text notes passphrase-protected keys are not supported).
   - **Certificate host name** (`serverName`, shown for `verify-full`;
     helper: "Only needed when the host is an IP address or reached
     through an SSH tunnel whose certificate names a different host").
   - Advisory under the select when `environment === "production"` and
     the mode is not `verify-ca` / `verify-full`: "Production
     connections should verify the server certificate." Non-blocking —
     the safety policy (`PAR-004`) is the enforcement boundary, this is
     copy.
3. `form-utils.ts`: schema gains `tlsMode` (enum, optional),
   `tlsRootCertPath`, `tlsClientCertPath`, `tlsClientKeyPath`,
   `tlsServerName` (trimmed strings, optional); defaults `tlsMode:
   "prefer"`; `defaultValuesFromConnection` maps `tlsOptions` → fields
   and, for a legacy record without `tlsOptions`, `tlsMode = ssl ?
   "prefer" : "disable"`; `buildPostgres` emits `tlsOptions` only when
   something differs from `{ mode: "prefer" }` with no paths (mirrors
   `driverOptionsFromForm`'s "no all-empty blob" rule) — **except** a
   legacy `ssl: false` record edited without touching TLS must still
   round-trip as `disable`. `resetEngineSpecificDefaults` resets the
   five TLS fields when leaving PostgreSQL. `resetDriverOptions` gains
   nothing new (keepalive is already there).
4. `driver-options-fields.tsx`: add **Keepalive idle (seconds)** next to
   connect timeout (1–7200, blank = OS default) with the disclosure
   line: "Applies to query, browse, and edit sessions. Metadata and
   admin queries use a pooled driver that cannot set it." Delete the
   `:12-14` comment and the `form-utils.ts:60-64` comment.

### Test Connection → staged diagnosis

1. `connections.ts`: replace `testConnection` with
   `diagnoseConnection(connection, hydrateFrom?: string): Promise<{ ok: true; report: ConnectionDiagnosis } | { ok: false; error: string }>`
   (the `ok: false` branch is the outer rejection only). Non-Tauri dev
   mode returns a synthetic all-passed report so the panel stays
   exercised, as today's stub does.
2. `use-connection-form.ts`: `TestStatus` becomes
   `idle | running | { state: "done"; report } | { state: "error"; error }`;
   `runTestConnection` passes `connection.id` as `hydrateFrom` in
   `edit` mode when the password field is empty. The button renders in
   **both** modes.
3. New `connection-form/diagnosis-panel.tsx` (`<DiagnosisPanel report>`),
   replacing the two banners in `form-footer.tsx`: one row per stage in
   fixed order with a status glyph, stage name, elapsed ms, and for a
   failed stage the `kind`-specific headline + the backend message in
   monospace; skipped stages render muted with their reason ("no tunnel
   configured", "TLS disabled", "not reached"). Below the rows, a TLS
   summary line derived only from the report: e.g. "Encrypted · TLSv1.3
   · AES_256_GCM · chain verified · host name verified · client
   certificate presented". A `notEncrypted` warning renders as a
   danger-toned line ("This connection is not encrypted"), never as
   part of a green banner; `poolHostnameVerificationCaOnly` renders as
   an info line naming the limitation; `productionWithoutVerification`
   as a warning. The `reachable` outcome shows latency; a `failed`
   outcome names the stage. Everything is token-coloured (UI gates).
4. Delete `test_connection`, `TestConnectionPayload`, and the `lib.rs`
   registration once no caller remains (`grep -rn "test_connection"
   src src-tauri` → only this plan and docs). This is the one Rust edit
   in this plan; run the `just` trio.

### Typed TLS failures where "connection lost" is shown today

`formatQuerySessionError`, `formatTableBrowseError`,
`formatSharedTransportError`, and the mutation-review model gain a
`tlsFailed` arm rendering the kind headline + message ("TLS: the server
certificate is not trusted — …"); the reconnect affordances at
`query-editor-panel.tsx:1707` and `use-table-session.ts:67` treat
`tlsFailed` like `connectionLost` for *state* (session is gone) but show
the TLS message, and `query-session-channel.ts:187` must not collapse it
into `connectionLost`. Extend the formatter tests and the two decode
tests.

### URI round-trip

`connection-uri.ts`: `buildConnectionUri` emits `?sslmode=<mode>` for
PostgreSQL when the resolved mode is not `prefer`; `parseConnectionUri`
maps a valid `sslmode` into `tlsMode` (invalid values stay ignored and
reported) and no longer lists `sslmode` under `ignoredParams`;
`sslrootcert` / `sslcert` / `sslkey` remain ignored-and-reported (paths
from a pasted URI are not trusted blindly). `uri-import-field.tsx`
prefills `tlsMode`. Tests both directions.

### Documentation truth pass (PAR-017 discipline)

Update `ROADMAP.md:25,38`, `docs/PENDING_TASKS.md:32`, `CONTEXT.md:24-28`
(glossary: `tlsOptions` + mode vocabulary; `ssl` is the legacy mirror),
`src/lib/store/types.ts:284-287`, `plans/parity-gap-register.md`
(`PAR-006` progress block → delivered scope, remaining follow-ons, stale
evidence paths), and `plans/README.md`. No claim may outrun the
implementation: the sqlx-pool keepalive and tunnel-hostname limitations
are stated where the feature is described.

## Commands you will need

```sh
pnpm format && pnpm lint && pnpm typecheck
pnpm vitest run
pnpm run check:ui-gates
pnpm run check:slice-isolation
just fmt && just lint && just test        # Step 3 deletes a command
pnpm db:postgres-tls && pnpm tauri dev    # Step 6 manual pass
```

## Scope

Expected files touched: `engine-policy.ts`, `connection-form.tsx`,
＋ `connection-form/tls-fields.tsx` (+test), `driver-options-fields.tsx`
(+test), `form-utils.ts` (+test), `use-connection-form.ts`,
`form-footer.tsx`, ＋ `connection-form/diagnosis-panel.tsx` (+test),
`connections.ts` (+ store types), `connection-uri.ts` (+test),
`uri-import-field.tsx` (+test), the four error formatters/models and
their tests, `query-editor-panel.tsx` / `use-table-session.ts` /
`query-session-channel.ts` (arm additions), `src-tauri/src/commands/connections.rs`
+ `types.rs` + `lib.rs` (deletion only), docs listed above, `plans/*`.

Out of scope: any new Tauri command or migration (belongs in a Plan 011
amendment), MySQL TLS controls, a file-picker plugin (typed paths, per
precedent), connection tags, certificate contents in the credential
store.

## Resume protocol

Each step ends with all gates green; re-run the step's verification on
resume. The selected mock is recorded in `plans/README.md` — if no
selection is recorded, you are still in Step 1 regardless of tree state.

## Git workflow

Working tree only; no commits/pushes/PRs without explicit operator
authorization.

## Steps

### Step 1: Produce and select local static UI mocks — STOP

Build three self-contained static HTML mocks under
`plans/mocks/connection-security/` (`mock-a.html`, `mock-b.html`,
`mock-c.html`), each showing: the PostgreSQL form with the TLS mode
select and its conditional fields in every mode, the keepalive control
with its disclosure, the production advisory, and the diagnosis panel in
three states (all passed with a full TLS summary; failed at `tls` with
`certificateUntrusted`; passed-but-`notEncrypted` under `prefer`).
Differentiate along real axes (e.g. A: TLS block inline under
credentials, checklist-style diagnosis rows; B: TLS as an Advanced
sub-section with a summary chip in the identity region, timeline-style
diagnosis; C: a two-column "Transport" card, compact single-line
diagnosis with an expandable failure). Every mock must render the
plaintext warning in danger tone, never inside a success banner.
**STOP: operator selects a mock; record the selection in
`plans/README.md` before any TSX.**

### Step 2: TLS fields, keepalive control, policy, form plumbing

Per the architecture. Tests (`Harness` pattern from
`driver-options-fields.test.tsx:29-50`): mode select shows/hides the
conditional fields per mode; client cert without key blocks submit with
the message; legacy `ssl: false` record opens as `disable` and
round-trips; `verify-full` + paths + server name build the expected
`tlsOptions` and `ssl: true`; `prefer` with nothing else builds no
`tlsOptions`; switching engine to MySQL clears the TLS fields and shows
the toggle; production + `prefer` shows the advisory; keepalive value
round-trips and out-of-range is rejected. Gates: the standard three +
vitest + `check:ui-gates`.

### Step 3: Diagnosis panel, store action, edit-mode test, command removal

Per the architecture. Tests: panel renders every stage status and skip
reason from a fixture report; failed-at-`tls` shows the kind headline;
`notEncrypted` renders in the danger tone and never alongside a success
banner; edit-mode test passes `hydrateFrom` only when the password is
empty; `ok: false` still renders the plain error. Delete
`test_connection`; `just` trio + the standard three + vitest.

### Step 4: `tlsFailed` rendering + URI round-trip

Per the architecture. Tests for each formatter arm, the channel
non-collapse, the mutation-review model, and both URI directions
(`sslmode` emitted only when not `prefer`; parsed into `tlsMode`;
invalid `sslmode` reported as ignored; cert params reported as ignored).
Gates as above.

### Step 5: Documentation truth pass

Per the architecture. Gate: `pnpm format` + a grep that no file still
claims keepalive is unapplied without qualification:
`grep -rn "exposes no socket" ROADMAP.md docs src` → empty.

### Step 6: End-to-end pass, mocks removal

Manual pass with `pnpm db:postgres` + `pnpm db:postgres-tls` up and
`pnpm tauri dev`: create a connection to 15433 in each mode (with and
without the fixture CA; with the client cert as `dbunk_cert`), run Test
Connection and confirm the panel matches the live outcome; wrong
password / wrong database / closed port / bad host each land on the
expected stage; open a query tab on a `verify-full` connection and run a
query; break the CA path and confirm the query tab shows the TLS message
rather than "connection lost"; edit an existing connection and Test
without retyping the password; Copy URI → Import shows `sslmode`. Delete
`plans/mocks/connection-security/`. Full gates:
`pnpm format && pnpm lint && pnpm typecheck && pnpm vitest run &&
pnpm run check:ui-gates` and the `just` trio. Mark `READY FOR REVIEW`.

## Test plan

Steps 2–5 enumerate the automated coverage; Step 6 is the manual pass
and needs the live fixtures. No automated test in this plan requires a
live database.

## Done criteria

- PostgreSQL connections expose all five TLS modes, CA / client cert /
  client key paths, and a server-name override; MySQL is unchanged.
- Keepalive has a control with an honest disclosure.
- Test Connection reports per-stage results, real encryption state, and
  the tunnel/pool limitations; it works in edit mode without retyping
  the password; the old `test_connection` command is gone.
- TLS failures in query, browse, and mutation surfaces show a TLS
  message, not "connection lost".
- `sslmode` round-trips through Copy URI / Import-from-URI.
- ROADMAP, PENDING_TASKS, CONTEXT, the register, the README, and the two
  source comments match reality.
- All gates green.

## STOP conditions

- No recorded mock selection.
- Plan 011 contract mismatch.
- Any need for a new Tauri command, migration, or backend TLS behaviour
  change (Plan 011 amendment, not here).
- The UI gates require raw hex or non-token colours to implement the
  selected mock — re-cut the mock instead.
- The diagnosis panel would need to infer encryption state from the
  selected mode rather than from the report's `pg_stat_ssl`-derived
  detail — the report is the only source of truth.

## Maintenance notes

- The mode → field visibility table lives in `tls-fields.tsx` only; the
  backend never validates presentation rules.
- A new `FailureKind` or `DiagnosisWarning` on the backend must get a
  headline in `diagnosis-panel.tsx`'s exhaustive switch (no default) —
  the typecheck fails until it does.
- `ssl` is derived, never edited directly, on the PostgreSQL form; the
  MySQL toggle still writes it.
