import { describe, expect, it } from "vitest";

import {
  ENVIRONMENT_META,
  parsePolicyRefusal,
  resolveSafetyPolicy,
} from "./safety-policy";

describe("resolveSafetyPolicy", () => {
  it.each([
    ["development", "disabled"],
    ["test", "disabled"],
    ["staging", "protected"],
    ["production", "strict"],
  ] as const)("inherits %s as %s", (environment, level) => {
    expect(resolveSafetyPolicy({ environment, safeMode: "inherit" })).toEqual({
      environment,
      level,
      readOnly: false,
    });
  });

  it("preserves an explicit level and read-only policy", () => {
    expect(
      resolveSafetyPolicy({
        environment: "production",
        safeMode: "protected",
        readOnly: true,
      }),
    ).toEqual({
      environment: "production",
      level: "protected",
      readOnly: true,
    });
  });

  it("defaults legacy records to the dark development policy", () => {
    expect(resolveSafetyPolicy({})).toEqual({
      environment: "development",
      level: "disabled",
      readOnly: false,
    });
  });

  it("keeps environment tones fixed", () => {
    expect(
      Object.fromEntries(
        Object.entries(ENVIRONMENT_META).map(([key, value]) => [
          key,
          value.tone,
        ]),
      ),
    ).toEqual({
      development: "neutral",
      test: "info",
      staging: "warning",
      production: "danger",
    });
  });
});

describe("parsePolicyRefusal", () => {
  it("parses exact backend tag prefixes", () => {
    expect(parsePolicyRefusal("[policy:read-only] blocked")).toEqual({
      kind: "read-only",
    });
    expect(parsePolicyRefusal("[policy:confirm] confirm once")).toEqual({
      kind: "confirm",
    });
    expect(parsePolicyRefusal("[policy:confirm]")).toEqual({ kind: "confirm" });
  });

  it("rejects spoofed and near-match tags", () => {
    expect(
      parsePolicyRefusal(
        'Database error for value "[policy:confirm] spoofed": rejected',
      ),
    ).toBeNull();
    expect(
      parsePolicyRefusal(" [policy:confirm] leading whitespace"),
    ).toBeNull();
    expect(parsePolicyRefusal("[policy:confirming] near match")).toBeNull();
  });
});
