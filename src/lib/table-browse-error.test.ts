import { describe, expect, it } from "vitest";

import type { TableBrowseError } from "@/lib/table-browse";
import {
  decodeTableBrowseError,
  formatTableBrowseError,
  isTableBrowseError,
} from "@/lib/table-browse-error";

describe("isTableBrowseError", () => {
  it("accepts tagged browse errors and rejects other values", () => {
    expect(isTableBrowseError({ kind: "superseded" })).toBe(true);
    expect(isTableBrowseError({ kind: "timeout", operation: "browse" })).toBe(
      true,
    );
    expect(
      isTableBrowseError({
        kind: "database",
        code: "42P01",
        message: "missing",
        severity: "ERROR",
        position: 7,
      }),
    ).toBe(true);
    expect(isTableBrowseError(null)).toBe(false);
    expect(isTableBrowseError("superseded")).toBe(false);
    expect(isTableBrowseError(new Error("timeout"))).toBe(false);
    expect(isTableBrowseError({ kind: "other" })).toBe(false);
    expect(isTableBrowseError({ message: "failed" })).toBe(false);
  });
});

describe("decodeTableBrowseError", () => {
  it("passes through tagged errors and maps unknown rejections to connectionLost", () => {
    const timeout: TableBrowseError = {
      kind: "timeout",
      operation: "count",
    };
    expect(decodeTableBrowseError(timeout)).toEqual(timeout);
    expect(decodeTableBrowseError({ kind: "superseded" })).toEqual({
      kind: "superseded",
    });
    expect(decodeTableBrowseError(new Error("socket reset"))).toEqual({
      kind: "connectionLost",
    });
    expect(decodeTableBrowseError("boom")).toEqual({ kind: "connectionLost" });
  });
});

describe("formatTableBrowseError", () => {
  it("formats a database error that includes a statement position", () => {
    expect(
      formatTableBrowseError({
        kind: "database",
        code: "42601",
        message: 'syntax error at or near "SELEC"',
        severity: "ERROR",
        position: 14,
      }),
    ).toBe('42601: syntax error at or near "SELEC"');
  });

  it("formats timeout and superseded errors", () => {
    expect(
      formatTableBrowseError({ kind: "timeout", operation: "browse" }),
    ).toBe("Timed out during browse.");
    expect(formatTableBrowseError({ kind: "superseded" })).toBe(
      "This browse request was superseded.",
    );
  });
});
