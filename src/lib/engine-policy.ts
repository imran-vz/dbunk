/**
 * Engine UI policy — frontend mirror to backend Engine Dispatch.
 *
 * The backend's `dispatch.rs` is the single place that knows "what does
 * this operation mean on each engine." This module is the frontend
 * analogue: one place that knows "how does this engine appear in the
 * UI" — connection-form shape, structure-view labels, stats-card
 * semantics, and the copy we use when a concept doesn't apply.
 *
 * ## Shape
 *
 * A pure `Record<DatabaseEngine, EnginePolicy>` table, accessed via
 * [`enginePolicy`]. TypeScript's `Record` enforces exhaustiveness — a
 * new `DatabaseEngine` variant will not compile without policies for
 * every field.
 *
 * ## Scope (deliberately narrow)
 *
 * Engine-level decisions only. Per-table mutation policy (can edit
 * cells? row identity columns? uniqueness guarantee?) lives on the
 * `TableStructure.capabilities` flag plus `pickRowIdentity`, NOT here.
 * Mixing the two would couple this module to per-table state and lose
 * the property that the policy table is a static fixture.
 *
 * ## Why not a hook
 *
 * The connection form components own engine selection in component
 * state (not the store), so a hook that reads from the store would
 * miss them. Components call `enginePolicy(engine)` directly — works
 * the same regardless of where the engine value came from.
 *
 * ## Adding an engine
 *
 * Add the variant to `DatabaseEngine` in `src/lib/store.ts`, then add
 * a `POLICIES` entry here. The compiler will refuse to build until
 * both halves agree.
 */

import type { DatabaseEngine } from "@/lib/store";

export type ConnectionFormPolicy = {
  /**
   * Whether the connection form needs host/port/user/password
   * inputs. SQLite is a file-path-only engine; the form collapses to
   * just the database-file input.
   */
  requiresHostAndAuth: boolean;
  /**
   * ClickHouse-only: surface the HTTPS toggle and URL-path input under
   * Advanced Options. Other engines leave these fields off the form.
   */
  showClickHouseHttp: boolean;
  /**
   * Default port placeholder when the user hasn't typed one. Used by
   * the form's port-field hint text; the actual default-port logic on
   * the backend (in dispatch.rs's sqlx_dsn) is independent.
   */
  defaultPort: number;
};

export type StructureLabels = {
  /**
   * The header section label. PG/MySQL/SQLite call it "Primary key";
   * ClickHouse calls it "Sorting key" — same backend field, different
   * conceptual meaning (CH's sorting key is a sparse index, not a
   * uniqueness constraint).
   */
  primaryKey: string;
  /** Short badge shown inline in the structure header. */
  primaryKeyBadge: string;
  /** "Indexes" (PG) vs "Skip indices" (CH data-skipping indices). */
  indexes: string;
  /** Empty-state copy when no primary key / sorting key is defined. */
  noPrimaryKey: string;
  /** Empty-state copy when no indexes / skip indices are defined. */
  noIndexes: string;
};

export type EnginePolicy = {
  engine: DatabaseEngine;
  /**
   * "exact" — backend reports a precise row count cheaply (CH from
   *           `system.parts.rows` sum).
   * "estimate" — backend reports a planner estimate (PG
   *           `pg_class.reltuples`); the stats card adds a "(≈)"
   *           suffix so users know.
   */
  rowCountKind: "exact" | "estimate";
  /**
   * Whether the engine has the concept of foreign keys at all. CH is
   * `false` (no FK enforcement, no FK metadata). The distinction from
   * `capabilities.foreignKeys` is *engine class* vs *per-table
   * coverage* — if `hasForeignKeys` is false the FK section's
   * "unsupported" message reads as a permanent engine property, not
   * as a coverage gap.
   */
  hasForeignKeys: boolean;
  /**
   * Copy rendered in the structure view's Foreign Keys section when
   * `capabilities.foreignKeys` is false. Engine-aware so the reason
   * reads correctly ("ClickHouse does not support foreign keys" vs
   * "Foreign keys are not supported on this engine").
   */
  foreignKeysUnsupportedCopy: string;
  /**
   * Banner shown above the schema-relationship map when the engine
   * has no foreign keys, the load returned tables, but the FK graph
   * is empty. `null` for engines where the situation doesn't arise.
   */
  schemaMapNoForeignKeysCopy: string | null;
  connectionForm: ConnectionFormPolicy;
  labels: StructureLabels;
};

const POLICIES: Record<DatabaseEngine, EnginePolicy> = {
  PostgreSQL: {
    engine: "PostgreSQL",
    rowCountKind: "estimate",
    hasForeignKeys: true,
    foreignKeysUnsupportedCopy:
      "Foreign keys are not supported on this engine.",
    schemaMapNoForeignKeysCopy: null,
    connectionForm: {
      requiresHostAndAuth: true,
      showClickHouseHttp: false,
      defaultPort: 5432,
    },
    labels: {
      primaryKey: "Primary key",
      primaryKeyBadge: "PK",
      indexes: "Indexes",
      noPrimaryKey: "This table has no primary key.",
      noIndexes: "No indexes defined.",
    },
  },
  MySQL: {
    engine: "MySQL",
    rowCountKind: "estimate",
    hasForeignKeys: true,
    foreignKeysUnsupportedCopy:
      "Foreign keys are not supported on this engine.",
    schemaMapNoForeignKeysCopy: null,
    connectionForm: {
      requiresHostAndAuth: true,
      showClickHouseHttp: false,
      defaultPort: 3306,
    },
    labels: {
      primaryKey: "Primary key",
      primaryKeyBadge: "PK",
      indexes: "Indexes",
      noPrimaryKey: "This table has no primary key.",
      noIndexes: "No indexes defined.",
    },
  },
  SQLite: {
    engine: "SQLite",
    rowCountKind: "estimate",
    hasForeignKeys: true,
    foreignKeysUnsupportedCopy:
      "Foreign keys are not supported on this engine.",
    schemaMapNoForeignKeysCopy: null,
    connectionForm: {
      requiresHostAndAuth: false,
      showClickHouseHttp: false,
      defaultPort: 0,
    },
    labels: {
      primaryKey: "Primary key",
      primaryKeyBadge: "PK",
      indexes: "Indexes",
      noPrimaryKey: "This table has no primary key.",
      noIndexes: "No indexes defined.",
    },
  },
  ClickHouse: {
    engine: "ClickHouse",
    rowCountKind: "exact",
    hasForeignKeys: false,
    foreignKeysUnsupportedCopy: "ClickHouse does not support foreign keys.",
    schemaMapNoForeignKeysCopy:
      "ClickHouse does not support foreign keys — showing tables only.",
    connectionForm: {
      requiresHostAndAuth: true,
      showClickHouseHttp: true,
      defaultPort: 8123,
    },
    labels: {
      primaryKey: "Sorting key",
      primaryKeyBadge: "ORDER BY",
      indexes: "Skip indices",
      noPrimaryKey: "This table has no sorting key.",
      noIndexes: "No skip indices defined.",
    },
  },
};

/**
 * Look up the UI policy for a given engine. Returns the policy
 * record verbatim; never null because `Record<DatabaseEngine, ...>`
 * forces every variant to have an entry.
 */
export function enginePolicy(engine: DatabaseEngine): EnginePolicy {
  return POLICIES[engine];
}
