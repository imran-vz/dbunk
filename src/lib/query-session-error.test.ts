import { describe, expect, it } from "vitest";

import {
  decodeQuerySessionError,
  formatQuerySessionError,
  isQuerySessionError,
} from "@/lib/query-session-error";
import type { QuerySessionError } from "@/lib/store/types";

const errors: QuerySessionError[] = [
  { kind: "unsupportedEngine" },
  { kind: "connectionClosing" },
  { kind: "sessionLimitReached", limit: "8" },
  { kind: "sessionNotFound" },
  { kind: "ownerMismatch" },
  { kind: "executionInProgress" },
  { kind: "invalidSequence" },
  {
    kind: "invalidTransactionTransition",
    status: "active",
    attemptedAction: "setMode",
    allowedActions: ["commit", "rollback"],
  },
  { kind: "transactionStateUnknown", canRecheck: true },
  { kind: "transactionStateUnknown", canRecheck: false },
  { kind: "transactionObserverUnavailable" },
  { kind: "connectionLost" },
  { kind: "timeout", operation: "execute" },
  {
    kind: "database",
    code: "42P01",
    message: "relation does not exist",
    severity: "ERROR",
    position: 15,
  },
  {
    kind: "database",
    code: null,
    message: "syntax error",
    severity: null,
    position: null,
  },
];

describe("formatQuerySessionError", () => {
  it("formats every QuerySessionError variant", () => {
    const messages = errors.map(formatQuerySessionError);
    expect(messages).toEqual([
      "Query sessions are only available for PostgreSQL.",
      "The connection is closing.",
      "Query session limit reached (8).",
      "Query session not found.",
      "Query session owner mismatch.",
      "A query is already running on this session.",
      "Query session lost a protocol event.",
      "Cannot setMode while the transaction is active.",
      "Transaction state is unknown. Recheck and try again.",
      "Transaction state is unknown.",
      "Transaction observer is unavailable.",
      "The database connection was lost.",
      "Timed out during execute.",
      "42P01: relation does not exist",
      "syntax error",
    ]);
    for (const message of messages) {
      expect(message.length).toBeGreaterThan(0);
    }
  });

  it("detects session errors by kind", () => {
    expect(isQuerySessionError({ kind: "connectionLost" })).toBe(true);
    expect(isQuerySessionError(new Error("nope"))).toBe(false);
    expect(isQuerySessionError({ kind: "other" })).toBe(false);
    expect(isQuerySessionError({ kind: "timeout" })).toBe(false);
    expect(decodeQuerySessionError({ kind: "timeout" })).toEqual({
      kind: "connectionLost",
    });
  });

  it("rejects malformed safety-policy errors", () => {
    expect(isQuerySessionError({ kind: "policyBlocked", reason: 42 })).toBe(
      false,
    );
    expect(
      decodeQuerySessionError({
        kind: "policyNeedsConfirmation",
        statements: [{ destructive: true }],
      }),
    ).toEqual({ kind: "connectionLost" });
    expect(
      isQuerySessionError({
        kind: "policyNeedsConfirmation",
        statements: [
          {
            index: 0,
            class: "drop-everything",
            unbounded: true,
            destructive: true,
          },
        ],
      }),
    ).toBe(false);
    expect(
      isQuerySessionError({
        kind: "policyNeedsConfirmation",
        statements: "delete from users",
      }),
    ).toBe(false);
  });

  it("accepts structurally valid safety-policy errors", () => {
    expect(
      isQuerySessionError({
        kind: "policyBlocked",
        reason: "This connection is read-only.",
      }),
    ).toBe(true);
    expect(
      isQuerySessionError({
        kind: "policyNeedsConfirmation",
        statements: [
          {
            index: 0,
            class: "ddl",
            unbounded: false,
            destructive: true,
          },
        ],
      }),
    ).toBe(true);
  });
});
