import { describe, expect, it } from "vitest";

import { REDIS_COMMANDS, suggestCommands } from "./cli-catalog";

describe("suggestCommands", () => {
  it("returns nothing for empty input", () => {
    expect(suggestCommands("")).toEqual([]);
    expect(suggestCommands("   ")).toEqual([]);
  });

  it("prefix-matches simple commands", () => {
    const result = suggestCommands("GE").map((spec) => spec.name);
    expect(result).toContain("GET");
  });

  it("is case-insensitive", () => {
    expect(suggestCommands("get").map((spec) => spec.name)).toContain("GET");
  });

  it("matches subcommands when the user types two tokens", () => {
    const result = suggestCommands("XINFO G").map((spec) => spec.name);
    expect(result).toContain("XINFO GROUPS");
    expect(result).not.toContain("XINFO STREAM");
  });

  it("returns all subcommands when only the head is typed", () => {
    const result = suggestCommands("XINFO").map((spec) => spec.name);
    expect(result).toEqual(
      expect.arrayContaining([
        "XINFO STREAM",
        "XINFO GROUPS",
        "XINFO CONSUMERS",
      ]),
    );
  });

  it("ranks exact-prefix hits above looser matches", () => {
    const [first] = suggestCommands("SET");
    expect(first.name).toBe("SET");
  });

  it("caps suggestions at 12 results", () => {
    expect(suggestCommands("X").length).toBeLessThanOrEqual(12);
  });

  it("every spec has a non-empty name and description", () => {
    for (const spec of REDIS_COMMANDS) {
      expect(spec.name.length).toBeGreaterThan(0);
      expect(spec.description.length).toBeGreaterThan(0);
    }
  });
});
