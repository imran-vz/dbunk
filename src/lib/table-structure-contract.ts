import type {
  PgObjectDescription,
  StructureCapabilities,
  TableStructure,
} from "@/lib/store/types";

/**
 * The backend always serializes the Plan 016 table-security fields; older
 * cached payloads and non-PostgreSQL loaders predate them. Normalizing once
 * here keeps the application contract (`TableStructure`) required so no
 * consumer has to re-handle absence.
 */
export type TableStructurePayload = Omit<
  TableStructure,
  "triggers" | "policies" | "privileges" | "rowSecurity" | "capabilities"
> &
  Partial<
    Pick<TableStructure, "triggers" | "policies" | "privileges" | "rowSecurity">
  > & {
    capabilities: Omit<
      StructureCapabilities,
      "triggers" | "policies" | "privileges"
    > &
      Partial<
        Pick<StructureCapabilities, "triggers" | "policies" | "privileges">
      >;
  };

export function normalizeTableStructure(
  payload: TableStructurePayload,
): TableStructure {
  return {
    ...payload,
    triggers: payload.triggers ?? [],
    policies: payload.policies ?? [],
    privileges: payload.privileges ?? [],
    rowSecurity: payload.rowSecurity ?? null,
    capabilities: {
      ...payload.capabilities,
      triggers: payload.capabilities.triggers ?? false,
      policies: payload.capabilities.policies ?? false,
      privileges: payload.capabilities.privileges ?? false,
    },
  };
}

type RoutineFacts = Extract<PgObjectDescription["facts"], { kind: "routine" }>;

export type PgObjectDescriptionPayload = Omit<PgObjectDescription, "facts"> & {
  facts:
    | Exclude<PgObjectDescription["facts"], { kind: "routine" }>
    | (Omit<RoutineFacts, "body" | "strict" | "securityDefiner" | "parallel"> &
        Partial<
          Pick<RoutineFacts, "body" | "strict" | "securityDefiner" | "parallel">
        >);
};

/** Same boundary rule for routine source facts (Plan 016). */
export function normalizePgObjectDescription(
  payload: PgObjectDescriptionPayload,
): PgObjectDescription {
  if (payload.facts.kind !== "routine") {
    // SAFETY: only routine facts carry the optional Plan 016 fields, so a
    // payload with any other facts kind already satisfies the required type.
    return payload as PgObjectDescription;
  }
  return {
    ...payload,
    facts: {
      ...payload.facts,
      body: payload.facts.body ?? null,
      strict: payload.facts.strict ?? false,
      securityDefiner: payload.facts.securityDefiner ?? false,
      parallel: payload.facts.parallel ?? null,
    },
  };
}
