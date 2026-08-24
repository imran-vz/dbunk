/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters, anti-slop/no-unsafe-dictionary-type -- Statement summaries arrive as unstructured invoke payloads and are decoded here. */
import type {
  ConnectionEnvironment,
  SafeMode,
  StoredConnection,
} from "@/lib/store/types";

export type ResolvedSafetyLevel = Exclude<SafeMode, "inherit">;

export type EnvironmentTone = "neutral" | "info" | "warning" | "danger";

export type StatementClassSummary = {
  index: number;
  class: "read" | "dml" | "ddl" | "transaction" | "session" | "unknown";
  unbounded: boolean;
  destructive: boolean;
};

export const ENVIRONMENT_META = {
  development: {
    label: "Development",
    shortLabel: "Dev",
    tone: "neutral",
    description: "Local and scratch data with no inherited write gate.",
  },
  test: {
    label: "Test",
    shortLabel: "Test",
    tone: "info",
    description:
      "Automated or disposable test data with no inherited write gate.",
  },
  staging: {
    label: "Staging",
    shortLabel: "Stage",
    tone: "warning",
    description: "Pre-production data with protected writes by default.",
  },
  production: {
    label: "Production",
    shortLabel: "Prod",
    tone: "danger",
    description: "Live data with strict confirmation by default.",
  },
} satisfies Record<
  ConnectionEnvironment,
  {
    label: string;
    shortLabel: string;
    tone: EnvironmentTone;
    description: string;
  }
>;

const INHERITED_LEVEL = {
  development: "disabled",
  test: "disabled",
  staging: "protected",
  production: "strict",
} satisfies Record<ConnectionEnvironment, ResolvedSafetyLevel>;

export type SafetyPolicyInput = Pick<
  StoredConnection,
  "environment" | "safeMode" | "readOnly"
>;

export type NamedSafetyPolicyInput = SafetyPolicyInput & { name: string };

/** Pure frontend mirror for labels and affordances. The backend remains authoritative. */
export type ResolvedSafetyPolicy = {
  environment: ConnectionEnvironment;
  level: ResolvedSafetyLevel;
  readOnly: boolean;
};

export function resolveSafetyPolicy(
  connection: SafetyPolicyInput,
): ResolvedSafetyPolicy {
  const environment = connection.environment ?? "development";
  const safeMode = connection.safeMode ?? "inherit";
  return {
    environment,
    level: safeMode === "inherit" ? INHERITED_LEVEL[environment] : safeMode,
    readOnly: connection.readOnly ?? false,
  };
}

export function readOnlyPolicyReason(
  connection: NamedSafetyPolicyInput,
): string | null {
  return resolveSafetyPolicy(connection).readOnly
    ? `${connection.name} is a read-only connection. Edit the connection to unlock writes.`
    : null;
}

const READ_ONLY_TAG = "[policy:read-only]";
const CONFIRM_TAG = "[policy:confirm]";

export type PolicyRefusal = { kind: "read-only" | "confirm" };

/** Route only authoritative tags at the start of the rejection string. */
export function parsePolicyRefusal(error: string): PolicyRefusal | null {
  if (hasExactPrefix(error, READ_ONLY_TAG)) return { kind: "read-only" };
  if (hasExactPrefix(error, CONFIRM_TAG)) return { kind: "confirm" };
  return null;
}

function hasExactPrefix(error: string, tag: string): boolean {
  return error === tag || error.startsWith(`${tag} `);
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === "object";

const isStatementClass = (
  value: unknown,
): value is StatementClassSummary["class"] =>
  value === "read" ||
  value === "dml" ||
  value === "ddl" ||
  value === "transaction" ||
  value === "session" ||
  value === "unknown";

/** Decode untrusted invoke payloads that claim to carry statement summaries. */
export function decodeStatementSummaries(
  value: unknown,
): StatementClassSummary[] | null {
  if (!Array.isArray(value)) return null;
  const summaries: StatementClassSummary[] = [];
  for (const statement of value) {
    if (
      !isRecord(statement) ||
      typeof statement.index !== "number" ||
      !isStatementClass(statement.class) ||
      typeof statement.unbounded !== "boolean" ||
      typeof statement.destructive !== "boolean"
    ) {
      return null;
    }
    summaries.push({
      index: statement.index,
      class: statement.class,
      unbounded: statement.unbounded,
      destructive: statement.destructive,
    });
  }
  return summaries;
}

export type SharedTransportError =
  | { kind: "connectionClosing" }
  | { kind: "connectionLost" }
  | { kind: "timeout"; operation: string }
  | { kind: "database"; code: string | null; message: string }
  | { kind: "policyBlocked"; reason: string }
  | { kind: "policyNeedsConfirmation" };

export function formatSharedTransportError(
  error: SharedTransportError,
): string {
  switch (error.kind) {
    case "connectionClosing":
      return "The connection is closing.";
    case "connectionLost":
      return "The database connection was lost.";
    case "timeout":
      return `Timed out during ${error.operation}.`;
    case "database":
      return error.code ? `${error.code}: ${error.message}` : error.message;
    case "policyBlocked":
      return `${error.reason} Edit the connection to unlock writes.`;
    case "policyNeedsConfirmation":
      return "The connection safety policy requires confirmation.";
  }
}
