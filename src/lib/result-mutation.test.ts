import { describe, expect, it } from "vitest";

import {
  decodeResultMutationError,
  type MutationPlan,
  supportsResultMutations,
} from "@/lib/result-mutation";
import type { DatabaseEngine } from "@/lib/store/types";

describe("result mutation protocol", () => {
  it("models tagged operations and preserves null values", () => {
    const plan = {
      operations: [
        {
          kind: "update",
          table: { schema: "public", table: "users" },
          identity: [{ column: "id", value: "1" }],
          guards: [{ column: "name", value: null }],
          set: [{ column: "name", value: "Ada" }],
        },
        {
          kind: "delete",
          table: { schema: "public", table: "users" },
          identity: [{ column: "id", value: "2" }],
          guards: [{ column: "name", value: "Grace" }],
        },
        {
          kind: "insert",
          table: { schema: "public", table: "users" },
          values: [{ column: "name", value: null }],
        },
      ],
    } satisfies MutationPlan;

    expect(plan.operations.map((operation) => operation.kind)).toEqual([
      "update",
      "delete",
      "insert",
    ]);
    expect(plan.operations).toContainEqual({
      kind: "insert",
      table: { schema: "public", table: "users" },
      values: [{ column: "name", value: null }],
    });
  });

  it("decodes every stable unit and attributed error shape", () => {
    for (const kind of [
      "unsupportedEngine",
      "analysisExpired",
      "busy",
      "superseded",
      "cancelled",
      "connectionClosing",
      "connectionLost",
    ] as const) {
      expect(decodeResultMutationError({ kind })).toEqual({ kind });
    }

    expect(
      decodeResultMutationError({ kind: "unknownColumn", column: "missing" }),
    ).toEqual({ kind: "unknownColumn", column: "missing" });
    expect(
      decodeResultMutationError({ kind: "invalidPlan", reason: "emptySet" }),
    ).toEqual({ kind: "invalidPlan", reason: "emptySet" });
    expect(decodeResultMutationError({ kind: "conflict", opIndex: 2 })).toEqual(
      { kind: "conflict", opIndex: 2 },
    );
    expect(
      decodeResultMutationError({ kind: "identityNotUnique", opIndex: 3 }),
    ).toEqual({ kind: "identityNotUnique", opIndex: 3 });
    expect(
      decodeResultMutationError({ kind: "lockTimeout", opIndex: 4 }),
    ).toEqual({ kind: "lockTimeout", opIndex: 4 });
    expect(
      decodeResultMutationError({ kind: "timeout", operation: "queueWait" }),
    ).toEqual({ kind: "timeout", operation: "queueWait" });
  });

  it("decodes not-analyzable and database detail without message parsing", () => {
    expect(
      decodeResultMutationError({
        kind: "notAnalyzable",
        reason: { kind: "possibleTempShadowing" },
      }),
    ).toEqual({
      kind: "notAnalyzable",
      reason: { kind: "possibleTempShadowing" },
    });
    expect(
      decodeResultMutationError({
        kind: "notAnalyzable",
        reason: {
          kind: "database",
          code: "42601",
          message: "syntax error",
          severity: "ERROR",
          position: 8,
        },
      }),
    ).toEqual({
      kind: "notAnalyzable",
      reason: {
        kind: "database",
        code: "42601",
        message: "syntax error",
        severity: "ERROR",
        position: 8,
      },
    });
    expect(
      decodeResultMutationError({
        kind: "database",
        code: "23505",
        message: "duplicate",
        severity: "ERROR",
        position: null,
        opIndex: 5,
      }),
    ).toEqual({
      kind: "database",
      code: "23505",
      message: "duplicate",
      severity: "ERROR",
      position: null,
      opIndex: 5,
    });
  });

  it("decodes typed policy refusals without SQL text", () => {
    expect(
      decodeResultMutationError({
        kind: "policyBlocked",
        reason: "Read-only connection",
      }),
    ).toEqual({ kind: "policyBlocked", reason: "Read-only connection" });
    expect(
      decodeResultMutationError({
        kind: "policyNeedsConfirmation",
        statements: [
          {
            index: 0,
            class: "dml",
            unbounded: true,
            destructive: true,
          },
        ],
      }),
    ).toEqual({
      kind: "policyNeedsConfirmation",
      statements: [
        {
          index: 0,
          class: "dml",
          unbounded: true,
          destructive: true,
        },
      ],
    });
  });

  it("fails closed for malformed and unknown rejection shapes", () => {
    for (const malformed of [
      new Error("socket reset"),
      "boom",
      { kind: "conflict", opIndex: "2" },
      { kind: "invalidPlan", reason: "futureReason" },
      { kind: "notAnalyzable", reason: { kind: "database" } },
      {
        kind: "database",
        code: null,
        message: "closed",
        severity: null,
        position: null,
        opIndex: "1",
      },
    ]) {
      expect(decodeResultMutationError(malformed)).toEqual({
        kind: "connectionLost",
      });
    }
  });

  it("activates result mutations only for PostgreSQL", () => {
    const engines: DatabaseEngine[] = [
      "PostgreSQL",
      "MySQL",
      "SQLite",
      "ClickHouse",
      "Redis",
    ];
    expect(engines.filter(supportsResultMutations)).toEqual(["PostgreSQL"]);
  });
});
