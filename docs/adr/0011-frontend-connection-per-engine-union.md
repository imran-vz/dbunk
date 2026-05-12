# ADR-0011 — Frontend `Connection` is a per-engine tagged union

**Status**: Accepted (2026-05-12)

Follows ADR-0010. ADR-0012 builds on this for the unified
`ConnectionForm` component.

## Context

ADR-0010 made the backend `StoredConnection` a serde-tagged enum,
preserving the existing flat-JSON wire shape via internally-tagged
serialization (`#[serde(tag = "engine")]`). The frontend continued to
parse the same wire shape into a flat `Connection` record whose
engine-specific fields were optional: `useHttps?`, `urlPath?`,
`dbNumber?`, `useTls?`, `verifyTlsCert?`.

The flat shape had the same class of bug the Rust side just closed: a
caller could read `connection.useHttps` on a PG connection and get
`undefined`, with no type-system warning. After Slice 4 lifts SSL to a
real UI affordance, that hole would surface as quiet, engine-specific
form bugs.

## Decision

`Connection` becomes a per-engine tagged union discriminated by
`engine`:

```ts
type Connection =
  | PgConnection         // engine: "PostgreSQL"; ssl: boolean
  | MySqlConnection      // engine: "MySQL";      ssl: boolean
  | SqliteConnection     // engine: "SQLite"
  | ClickHouseConnection // engine: "ClickHouse"; useHttps, urlPath
  | RedisConnection;     // engine: "Redis";      dbNumber, useTls, verifyTlsCert
```

Each variant is `StoredConnectionVariant & ConnectionRuntimeFields`
where `ConnectionRuntimeFields = { status, latency, lastSync,
errorMessage? }`. The same structural relationship the Rust side has
between `StoredConnection` variants and their concrete data — minus
the runtime fields the frontend tracks separately.

PG and MySQL stay separate variants even though their field shapes
are identical today. The strict per-engine split was the explicit
choice in the design conversation that produced this slice: future
divergence (PG-only `sslmode=verify-full` vs MySQL socket paths)
slots in without restructuring; the cost is two variants in match
patterns where one would have done.

Variant-specific field reads narrow on `connection.engine`:

```ts
if (connection.engine === "ClickHouse") {
  connection.useHttps;  // typed access
}
```

Code that handles "any connection" reads common fields (`id`, `host`,
`port`, `user`, `password`, `database`, `role`, `lastActivityAt`)
without narrowing — TypeScript's discriminated-union semantics
preserve those accesses on the union.

At workspace-fork boundaries (`workspace-view.tsx`), the
storage-class check (ADR-0008) pairs with an engine-tag narrow so
that `KeyValueWorkspace` receives a typed `RedisConnection`:

```tsx
if (
  storageClassFor(activeConnection.engine) === "keyvalue" &&
  activeConnection.engine === "Redis"
) {
  return <KeyValueWorkspace activeConnection={activeConnection} />;
}
```

When a second keyvalue engine joins the union, the extra
discriminator-check disappears.

`testConnection` in the store slice now takes `StoredConnection`
directly instead of a `Pick<>`-based subset; the call sites
(`new-connection-form`, `edit-connection-dialog`) build the variant
from form values and pass it through. The build step is concentrated
in helper functions (`buildStoredConnectionFromForm`,
`buildConnectionFromForm`) that switch on engine and emit the right
shape. Slice 4 (ADR-0012) lifts those helpers into the unified
`ConnectionForm` component alongside the per-kind validator.

The connections-slice `applyConnectionUpdate` helper is restricted
to runtime-field updates (`status`, `latency`, `lastSync`,
`errorMessage`, `lastActivityAt`). Variant-specific fields are part
of the connection's engine identity and aren't touched by the
runtime-only paths.

## Alternatives considered

1. **Keep `Connection` flat, just add `ssl?: boolean`**. Cheapest:
   one new optional field. Rejected — the silent-default bug class
   stays and grows (`ssl` joins the others). The whole point of
   ADR-0010's variant-level safety is to push it through to the
   consumers.
2. **Two-level union: `Connection = RelationalConnection |
   KeyValueConnection`, with engine-discriminated subtypes inside**.
   Aligns the record's union with the storage-class fork (ADR-0008).
   Rejected: that asymmetry is right. Workspace shells and dispatch
   routers care about the class; the record carries per-engine
   fields and discriminates on engine.
3. **Group PG + MySQL under one variant** (`HostAuthConnection`
   discriminated on engine). Saves duplication of identical fields.
   Rejected: explicit choice in the design conversation. The
   variants stay separate so future per-engine fields slot in
   without restructuring.

## Implications

- The wire contract between Rust and TS is now mirror-shaped on
  both sides. JSON round-trips through internally-tagged serde on
  Rust and TypeScript discriminated-union narrowing on the
  frontend; no adapter sits between them.
- `KeyValueWorkspace` takes a `RedisConnection` directly, not a
  generic `Connection`. Inside the component, `dbNumber`, `useTls`,
  `verifyTlsCert` reads no longer need `??` fallbacks — the fields
  are required on the variant. Existing fallbacks are kept for
  diff-minimisation; they're now dead but harmless.
- `edit-connection-dialog` and `new-connection-form` build the
  variant per engine via a `switch (engine)` block before calling
  `addConnection` / `updateConnection`. This pattern repeats in
  Slice 4's unified form (#16) where it becomes the canonical
  construction path.
- The Connection-record paragraph in CONTEXT.md still describes the
  flat shape with optional engine-specific fields. ADR-0012 (Slice 4)
  is the right moment for the CONTEXT.md rewrite — by then the form
  layer also uses the tagged-policy shape and the picture is
  complete.

## Related

- ADR-0010 — backend `StoredConnection` enum (the precondition).
- ADR-0008 — storage-class fork at the dispatcher layer (sits
  above this per-engine fork on the record).
- ADR-0012 (forthcoming) — unified `ConnectionForm` + tagged
  `ConnectionFormPolicy`.
