# ADR-0012 — Unified `ConnectionForm` + tagged `ConnectionFormPolicy`

**Status**: Accepted (2026-05-12)

Closes the Group A/B/C arc started by ADR-0010 (backend
`StoredConnection` enum) and ADR-0011 (frontend `Connection` per-engine
union).

## Context

Two near-identical form components existed: `new-connection-form.tsx`
(660 lines) for create, and the inline form inside
`edit-connection-dialog.tsx` (560 lines) for edit. Each had its own
copy of the Zod schema, its own per-engine field-zeroing logic at
submit, its own port-placeholder ternary, and its own (slightly
drifted) set of `selectedEngine === "..."` branches throughout the
JSX. The `ConnectionFormPolicy` abstraction existed in
`engine-policy.ts` but neither form read it — both re-derived
everything from string comparisons on the engine.

Concrete consequences:

1. The two Zod schemas had silently diverged (the new-form required
   password, the edit-form didn't; the new-form had an `ssl` field
   that the edit-form lacked).
2. The `ssl` toggle was dead UI: it rendered in the new form,
   defaulted to `true`, but was dropped at submit (`Connection`
   didn't even have an `ssl` field until ADR-0010).
3. Adding an engine meant touching two forms plus the policy table —
   three sources of truth.
4. The edit dialog let users change the engine on an existing
   connection. Combined with the per-engine `Connection` union from
   ADR-0011, that "edit" was actually a transform between variants —
   not a property edit.

ADR-0010 and ADR-0011 set up the types; this ADR is where the form
layer finally reads them.

## Decision

One `<ConnectionForm mode="new" | "edit">` component
(`src/components/connection-form.tsx`) renders every engine. Both
existing dialog wrappers shrink to AlertDialog chrome plus a single
render of the component.

**Mode-aware behavior:**

- `mode: "new"` — engine picker editable; common fields
  (`name`/`host`/`port`/`user`/`password`/`role`) carry across
  engine-switches via `form.setFieldValue` resets that touch only
  engine-specific fields; footer shows Test Connection + the
  credential-storage hint.
- `mode: "edit"` — engine picker disabled. Footer shows Cancel +
  Save changes. The "blank password keeps existing credential" rule
  rides on the validator (mode-aware via `validateConnection`) and
  the backend's existing `save_connection` substitution (ADR-0010
  §1).

**Policy-driven field rendering.** The form reads
`connectionFormPolicy(selectedEngine)` and switches on `policy.kind`:

| kind             | Visible fields beyond common                                  |
| ---------------- | ------------------------------------------------------------- |
| `host-auth`      | `ssl` toggle (when `showSslToggle`); host/port/user/password  |
| `clickhouse-http`| `useHttps` toggle, `urlPath` (advanced)                       |
| `redis`          | `dbNumber` (replaces database), `useTls`, `verifyTlsCert`     |
| `file`           | `database` (as file path); no host/auth                       |

The scattered `selectedEngine === "..."` checks vanish. Adding an
engine means filling in a `ConnectionFormPolicy` variant — the form
inherits its rendering automatically.

**Construction of the wire shape.** Two helpers in the same file:

- `buildStoredConnectionFromForm(value, id)` — switches on `engine`,
  emits the right `StoredConnection` variant with engine-specific
  fields filled. Used by Test Connection (passes the variant
  straight to `testConnection`).
- `buildConnectionFromForm(value, id, runtime)` — wraps the above
  with runtime fields (`status`, `latency`, `lastSync`,
  `errorMessage`, `lastActivityAt`). Used by `addConnection`
  (`mode: "new"`) and `updateConnection` (`mode: "edit"`).

The pre-Slice-1 dead `ssl` toggle is now real: it lives on the
`host-auth` rendering branch, reads/writes `Connection.ssl`, and
round-trips through the SQLite `ssl` column added in ADR-0010's
migration 3.

## Alternatives considered

1. **Keep two form components, share the validator + helpers only.**
   Lower-touch refactor. Rejected as the explicit choice in the
   design conversation that produced this slice — leaves the two
   components free to re-drift, and doesn't address the engine-
   picker-in-edit bug.
2. **Extract `<ConnectionFields>` only; keep two outer components.**
   Middle ground. Rejected for the same reason: the form's
   `useForm` hook, validator wiring, and engine-switch state all
   want to live in one place; splitting at the field-row level
   leaves them duplicated.
3. **Keep engine picker editable in edit mode.** Today's behavior.
   Rejected: changing engine on a tagged-union record is a delete-
   and-recreate, not an edit. Disabling the picker is an
   intentional behavior change recorded here so future readers don't
   re-enable it without thinking.

## Implications

- The `ConnectionFormPolicy` tagged union becomes the canonical
  per-engine knob list for the form. Adding fields means adding
  them to a variant; the type system tells the form to render them.
- Field rendering inside `ConnectionForm` is still procedural (a
  `if (policy.kind === "host-auth") ...` ladder, not a fully
  data-driven renderer). That ceiling is fine for four kinds; the
  pattern only starts to feel weak when half the variants share
  rendering and the form has to dedupe via composition. Worth
  revisiting if a fifth or sixth kind lands.
- The Engine UI Policy entry in `CONTEXT.md` is updated alongside
  this ADR — the previous description ("flat shared field with
  per-engine feature flags") no longer matches reality.
- The Connection entry in `CONTEXT.md` is also updated — Slice 2
  deferred that rewrite to this ADR, since by now the form layer
  and the type layer agree on shape and the doc can describe both
  in one pass.

## Related

- ADR-0010 — backend `StoredConnection` enum + `ssl` column. The
  precondition this form makes user-visible.
- ADR-0011 — frontend `Connection` per-engine union. The shape the
  form constructs and consumes.
- ADR-0008 — storage-class fork at the dispatcher layer. The
  `ConnectionFormPolicy.kind` discriminator names form shape, which
  is finer than storage class (PG/MySQL both `host-auth`; SQLite is
  `file` even though relational).
- ADR-0009 — Redis read-only-by-default. The form does not
  surface a write-toggle for Redis; gating happens at the editor
  render site.
