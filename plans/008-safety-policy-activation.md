# Plan 008: Safety policy activation and production identity

> **Executor instructions**: Do not start until Plan 007 is `DONE` in
> `plans/README.md`. Follow this plan step by step. Step 1 ends in a STOP for
> operator mock selection — do not write any TSX before a mock is selected
> and recorded. Run every verification command and confirm the expected
> result before moving on. Update this plan's README row after each step and
> mark `READY FOR REVIEW` after all gates. A reviewer/operator records
> `DONE: <completion SHA>` after an authorized commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git diff --stat <PLAN_007_COMPLETION_SHA>..HEAD -- src plans/README.md plans/mocks/safety-policy
> git status --short -- src plans/README.md plans/mocks/safety-policy
> ```
>
> Expected on a fresh run: no `src` output. A load-bearing mismatch with the
> excerpts below is a STOP condition.

## Status

- **Priority**: P0
- **Effort**: L
- **Risk**: MEDIUM
- **Depends on**: Plan 007 complete
- **Category**: direction
- **Planned at**: commit `4e52c8a`, 2026-08-23
- **Gap**: `PAR-004` in `plans/parity-gap-register.md`

## Why this matters

Plan 007 lands enforcement, but a policy nobody can set gates nothing: every
connection stays `development`/`inherit` forever without a form surface, and
a production connection that looks identical to a scratch database defeats
the point — the register asks for "always-visible production identity", not
just refusals. This plan activates the policy end to end: environment and
Safe Mode controls in the connection form, environment identity on every
surface where a user acts on a connection, confirmation dialogs that resend
with `confirmed: true` after a deliberate acknowledgment, read-only
affordance gating so the UI explains what the backend will refuse, and the
override audit in Settings.

## Required Plan 007 contract

This plan consumes exactly what Plan 007 shipped; a mismatch is a STOP
condition, not permission to change backend behavior here:

- `StoredConnection` fields `environment` (`development | test | staging |
  production`), `safeMode` (`inherit | disabled | protected | strict`), and
  relational `readOnly` — serde-defaulted, round-tripping through
  `save_connection`.
- `execute_query_session` and `apply_result_mutations` payloads accept
  `confirmed?: boolean`; refusals arrive as typed
  `policyBlocked { reason }` / `policyNeedsConfirmation { statements }`
  variants carrying `StatementClassSummary` lists (class labels only, never
  SQL). On the apply path the summaries are synthesized from the staged
  plan operations (one bounded-dml entry per update/delete/insert), so the
  dialog can render "3 updates, 1 delete" without any SQL.
- Legacy gated commands accept `confirmed?: boolean` and refuse with strings
  tagged `[policy:read-only]` / `[policy:confirm]`.
- `load_safety_overrides(connectionId)` returns `{ command, classes,
  occurredAt }[]`.
- Resolution rule: `inherit` maps development → disabled, test → disabled,
  staging → protected, production → strict.

## Current frontend state

- `src/lib/store/types.ts:249-260` — `ConnectionCommon`; new fields land
  here once, so `defaultValuesFromConnection`
  (`src/components/connection-form/form-utils.ts:308-392`) needs no
  per-engine branches.
- `src/components/connection-form/form-utils.ts:24-60` — zod
  `connectionSchema`; `:64-95` `EMPTY_NEW_DEFAULTS`; `:276-293`
  `buildStoredConnectionFromForm` (single construction site).
- `src/lib/engine-policy.ts:361-396` — `ConnectionFormValues` (must gain the
  new fields); `:428-480` `validateConnection`. Policy is deliberately
  engine-scoped (`:28-35`), so per-connection safety resolution goes in a
  new sibling `src/lib/safety-policy.ts`, not in `POLICIES`.
- `src/components/connection-form.tsx:108-113` — identity region
  (name/engine); `:171-215` `HostAuthSection` advanced block — the two
  insertion points. `ToggleSwitchRow`
  (`connection-form/field-helpers.tsx`) and the Redis read-only row
  (`connection-form/redis-fields.tsx:79-89`) are the control precedents.
- `src/components/sidebar.tsx:228-264` — `MANAGED_BADGE`
  (`Record<Status, {label, className}>`) is the exact badge pattern to copy;
  connection rows render at `:380-500`.
- `src/components/app-shell/workbench-header.tsx:73` — connection name in the
  workbench header, the highest-signal identity surface;
  `src/components/workspace-overview/overview-header.tsx:36-43` — name +
  status chip; `src/components/workspace-overview/health-banner.tsx` —
  banner shells to extend with a production variant;
  `src/components/connection-status.ts:5-28` — status-bar item builder.
- Tab strips: `src/components/workspace-tabs.tsx:130-181` and
  `src/components/workbench/object-tab-row.tsx` — both key on
  `tab.connectionId` but never style by connection today.
- `src/lib/store/workspace-tabs.ts:328-350` — `retargetQueryTab` already
  takes a caller-supplied confirm callback
  (`confirmDiscardStagedChanges`) — the pattern for a
  retarget-onto-production confirm.
- `src/lib/store/edit-strategies.ts:190-193` — `resolveEditContext` resolves
  the connection and is where a read-only/production refusal reason drops in
  with the connection already in hand.
- Typed-confirm precedents: Redis CLI modal
  (`src/components/keyvalue/CliTab.tsx:487-530`), keyspace bulk-op dry-run +
  typed confirm (`KeyspaceBrowser.tsx:441-462`), and the Plan 006 mutation
  review panel (`src/components/mutation-review/`).
- `src/components/workspace-overview/settings-tab.tsx` — read-only mirror
  that must show the new fields and host the override audit list.

## Decided frontend architecture

### Safety policy library

1. `src/lib/safety-policy.ts` is a pure mirror of the backend resolver:
   `resolveSafetyPolicy(connection) -> { environment, level, readOnly }`,
   environment display metadata (label, badge tone, description), and
   `parsePolicyRefusal(error: string) -> { kind: "read-only" | "confirm" } |
   null` matching the Plan 007 tags with a **strict prefix match only** —
   never a substring match, because server error text can embed
   user-controlled values that could spoof a tag mid-string and spuriously
   open the confirm dialog. It predicts nothing the backend does not
   enforce; it exists so the UI can label, pre-explain, and route
   refusals — the backend remains the boundary.
2. Environment tones are fixed, not user-picked: production = danger,
   staging = warning, test = info, development = neutral. A custom color
   field stays in the gap register as deferred `PAR-005`/`PAR-006` polish.

### Form and settings

3. `ConnectionCommon` gains `environment` and `safeMode`; relational
   variants gain `readOnly`. Zod schema, `ConnectionFormValues`,
   `EMPTY_NEW_DEFAULTS`, `commonFromForm`, and `defaultValuesFromConnection`
   carry them; `validateConnection` accepts every combination (no semantic
   rule needed in v1 — the enum types constrain the values).
4. An `EnvironmentField` (select + tone swatch) renders in the identity
   region next to `NameField`. A `SafetyFields` section (Safe Mode select
   showing the inherited resolution, plus the relational Read-only toggle)
   renders inside the advanced block for relational engines; the Redis
   read-only toggle stays where it is.
5. The Settings tab mirror gains Environment, Safe Mode (with resolved
   level), and Read-only rows, plus a "Recent safety overrides" list from
   `load_safety_overrides` (command + classes + relative time, empty-state
   copy when none).

### Production identity surfaces

6. One `EnvironmentBadge` component (label + tone, `MANAGED_BADGE` pattern)
   renders for non-development environments in: the sidebar connection row,
   the workbench header next to the connection name, the overview header
   status chip row, and the connection picker dropdown rows. Development
   renders nothing — the badge is signal, not decoration.
7. Production connections additionally get: a persistent tinted banner via a
   new `HealthBanner` production variant, a danger-tinted left border on
   active tabs whose `tab.connectionId` resolves to a production connection
   (both tab strips), and the environment appended to the status-bar
   connection item (`connectionStatusItem`).
8. `retargetQueryTab` callers pass a confirm when the **target** connection
   is production; the store signature already supports caller-supplied
   confirmation.

### Confirmation flows

9. One shared `SafetyConfirmDialog` handles every gate response. It shows:
   the connection name + environment badge, what was refused (per-statement
   class summaries on typed surfaces; the command name on legacy surfaces),
   and — for destructive or production-strict refusals — a typed-confirm
   input requiring the connection name (KeyspaceBrowser precedent).
   Confirming re-invokes the original call with `confirmed: true`; the
   dialog never pre-sets `confirmed` on first attempts.
10. Wiring: the query-session execute path handles
    `policyNeedsConfirmation` by opening the dialog and re-sending;
    `policyBlocked` renders as a non-dismissable inline refusal with
    "edit connection to unlock" copy. The mutation-review apply path feeds
    its existing review confirmation into `confirmed: true` and handles both
    new error variants. Legacy call sites (DDL apply, restore, maintenance,
    refresh, import wizard, seeding, table copy, terminate backend, legacy
    row mutations on non-PG engines) route their errors through
    `parsePolicyRefusal` and reuse the same dialog. No call site gains its
    own bespoke confirm.
11. `resolveEditContext` returns a typed refusal reason when the resolved
    policy is read-only ("<name> is a read-only connection…") so grids
    disable editing affordances up front; the backend gate remains the
    boundary and is still exercised if the UI check is bypassed.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Format | `pnpm format` | exit 0 |
| Lint | `pnpm lint` | exit 0 |
| Typecheck | `pnpm typecheck` | exit 0 |
| Unit tests | `pnpm test` | all pass |

## Scope

**In scope**:

- `plans/mocks/safety-policy/variant-{a,b,c}.html` (create; delete after
  implementation per the Plan 006 precedent)
- `src/lib/safety-policy.ts` (+ tests), `src/lib/store/types.ts`
- `src/lib/engine-policy.ts` (`ConnectionFormValues`, `validateConnection`)
- `src/components/connection-form.tsx`, `connection-form/*`
  (`form-utils.ts`, `common-fields.tsx`, new `safety-fields.tsx`,
  `use-connection-form.ts`)
- `src/components/environment-badge.tsx`, `safety-confirm-dialog.tsx`
  (create)
- `src/components/sidebar.tsx`, `app-shell/workbench-header.tsx`,
  `workspace-overview/overview-header.tsx`,
  `workspace-overview/health-banner.tsx`, `connection-status.ts`,
  `workspace-tabs.tsx`, `workbench/object-tab-row.tsx`
- `src/components/workspace-overview/settings-tab.tsx`
- `src/lib/store/edit-strategies.ts`, `store/workspace-tabs.ts` callers,
  query-session client/store and mutation-review wiring for the new error
  variants and `confirmed` re-send
- Legacy call sites listed in decision 10 (error routing only)
- `plans/README.md` status and selected-mock note only

**Out of scope**:

- Any `src-tauri` change — a needed backend change is a STOP, not a patch
- User-picked connection colors, folders, tags (`PAR-005`/`PAR-006`)
- Smart-commit/manual-transaction environment defaults (`PAR-001`
  follow-ons)
- Replacing the remaining non-policy `window.confirm` call sites
- Commits, pushes, PRs without authorization

## Resume protocol

1. Read Plan 008 status and selected-mock note in `plans/README.md`.
2. Inspect `git status --short` and `git diff -- <Scope paths>`.
3. Accept changes only when they match recorded steps; unexplained or
   out-of-order changes are a STOP. Never discard user work.

## Git workflow

- Suggested branch: `feat/safety-policy-activation`, only if the operator
  asks. No commits/pushes/PRs without authorization.

## Steps

### Step 1: Produce and select local static UI mocks

Create three static HTML variants under `plans/mocks/safety-policy/`, each
showing: the connection form with Environment + Safe Mode + Read-only
controls, a sidebar with mixed-environment connections, the workbench header
and tab strip with a production connection active (banner, badge, tab tint),
and the `SafetyConfirmDialog` in both its plain-confirm and typed-confirm
states. Vary the identity treatment across variants (e.g. A: badge + banner
emphasis; B: header-tint + minimal chrome; C: status-bar-centric with
dialog-forward emphasis).

STOP and wait for the operator to choose A, B, or C. Record the selection in
`plans/README.md`.

**Verify**: three files exist; README records the selection.

### Step 2: Types, safety library, form, and settings

Land decisions 1–5: type fields, `safety-policy.ts` with resolution/tone/
`parsePolicyRefusal` tests (tag parsing exactness, inherit resolution parity
with the backend rule), form controls with round-trip through
`buildStoredConnectionFromForm` and `defaultValuesFromConnection`, and the
Settings mirror + overrides list.

**Verify**: `pnpm format && pnpm lint && pnpm typecheck && pnpm test`; create
and edit a connection through the form in dev and confirm the fields persist
across restart.

### Step 3: Identity surfaces

Land decisions 6–8 per the selected mock: `EnvironmentBadge` and its four
render sites (sidebar row, workbench header, overview header, connection
picker rows — the Settings mirror shows the field as a row, not a badge),
the production banner variant, tab tinting in both strips, the status-bar
item, and the production retarget confirm.

**Verify**: `pnpm format && pnpm lint && pnpm typecheck && pnpm test`; in
dev, a production-flagged connection is visibly distinct in sidebar, header,
tabs, banner, and status bar simultaneously; a development connection shows
no badge.

### Step 4: Confirmation flows and refusal routing

Land decisions 9–11: the shared dialog, typed-variant handling on the
query-session and mutation-review paths with `confirmed: true` re-send,
tag-parsed routing for every legacy call site in decision 10, blocked-state
inline copy, and the `resolveEditContext` read-only reason.

Store/component tests: refusal → dialog → confirmed re-send state machine
(mocked IPC) for the query-session path; apply-path handling of both
variants; `parsePolicyRefusal` routing for a representative legacy site;
`resolveEditContext` returning the read-only reason; and a test asserting
first-attempt payloads never carry `confirmed: true`.

**Verify**: `pnpm format && pnpm lint && pnpm typecheck && pnpm test`.

### Step 5: End-to-end pass and cleanup

Against the local fixture with one production-strict and one read-only
connection: run the full flow — unconfirmed DML refusal → dialog → confirmed
success with the override appearing in Settings; unbounded `DELETE` on a
protected connection; read-only grid affordances disabled with the reason
visible; restore/maintenance/terminate confirms. Delete the mock files
(Plan 006 precedent, commit `916ffc2`).

**Verify**: all four gates green; `git status` shows only in-scope files;
mocks removed; README `READY FOR REVIEW`.

## Test plan

- `safety-policy.ts`: resolution parity, tone mapping, tag parsing —
  including a spoof case proving a tag embedded mid-string (e.g. inside a
  quoted value in a server error) does not parse as a refusal.
- Form utils: field round-trip for every engine, legacy-record defaults.
- Store: refusal/confirm state machine, retarget confirm, edit-context
  reasons.
- Components: badge render matrix by environment, dialog confirm gating
  (typed input must match), settings mirror rows.

## Done criteria

- [ ] The selected mock is recorded and implemented.
- [ ] Policy fields are editable, validated, persisted, and mirrored in
      Settings for every engine.
- [ ] Production identity is simultaneously visible in sidebar, header, tab
      strip, banner, and status bar; development connections are unmarked.
- [ ] Every gate refusal routes to one shared dialog; confirming re-sends
      with `confirmed: true`; first attempts never pre-confirm.
- [ ] Read-only connections show disabled editing affordances with reasons;
      blocked refusals explain the unlock path (edit connection).
- [ ] Confirmed overrides appear in the Settings audit list.
- [ ] No `src-tauri` file changed; mocks are deleted; all four gates pass.
- [ ] `plans/README.md` says `READY FOR REVIEW`.

## STOP conditions

Stop and report if:

- No mock is explicitly selected before TSX work.
- The Plan 007 contract mismatches the shapes listed above.
- Any flow requires a backend change or a new Tauri command.
- The confirm dialog would need to display SQL text it does not have, or a
  refusal payload unexpectedly contains SQL.
- Any gate fails twice.

## Maintenance notes

- The frontend never decides policy — `safety-policy.ts` mirrors the
  backend rule for display only, and drift between them shows up as the
  backend refusing something the UI didn't pre-explain (safe direction).
- When legacy command errors migrate to typed unions (`PAR-007`/`PAR-014`),
  `parsePolicyRefusal` and the tags retire together.
- Deferred `PAR-004` register items after this plan: environment-scoped
  smart-commit defaults, user-picked connection colors, and script
  stop/continue/prompt error policies (`PAR-001` follow-on).
