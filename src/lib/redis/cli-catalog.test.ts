import { describe, expect, it } from "vitest";

import {
  findCommand,
  REDIS_COMMANDS,
  requiredArgCount,
  suggestCommands,
  validateArgs,
} from "./cli-catalog";

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

describe("findCommand", () => {
  it("matches a single-word command case-insensitively", () => {
    expect(findCommand(["get", "foo"])?.name).toBe("GET");
  });

  it("prefers a two-word match over the single-word head", () => {
    expect(findCommand(["PUBSUB", "CHANNELS"])?.name).toBe("PUBSUB CHANNELS");
  });

  it("returns null for unknown commands", () => {
    expect(findCommand(["SOMETHING_WEIRD"])).toBeNull();
  });
});

describe("requiredArgCount", () => {
  it("counts plain tokens", () => {
    expect(requiredArgCount({ name: "X", args: "key", description: "" })).toBe(
      1,
    );
    expect(
      requiredArgCount({ name: "X", args: "key value", description: "" }),
    ).toBe(2);
  });

  it("ignores bracketed optional segments", () => {
    expect(
      requiredArgCount({
        name: "X",
        args: "key value [EX seconds | PX ms | KEEPTTL | NX | XX]",
        description: "",
      }),
    ).toBe(2);
  });

  it("treats variadic tails as zero required", () => {
    expect(
      requiredArgCount({
        name: "X",
        args: "channel [channel …]",
        description: "",
      }),
    ).toBe(1);
  });

  it("returns 0 when everything is optional", () => {
    expect(
      requiredArgCount({ name: "X", args: "[message]", description: "" }),
    ).toBe(0);
  });
});

describe("validateArgs", () => {
  it("returns null for unknown commands (server-authoritative)", () => {
    expect(validateArgs(["MAYBE_NEW_CMD", "foo"])).toBeNull();
  });

  it("returns null when arity is sufficient", () => {
    expect(validateArgs(["GET", "foo"])).toBeNull();
    expect(validateArgs(["SET", "foo", "bar"])).toBeNull();
    expect(validateArgs(["SET", "foo", "bar", "EX", "60"])).toBeNull();
  });

  it("returns an error string when a required arg is missing", () => {
    expect(validateArgs(["GET"])).toMatch(/GET expects 1 argument/);
    expect(validateArgs(["SET", "foo"])).toMatch(/SET expects 2 arguments/);
  });

  it("counts the two-word command name as zero supplied args", () => {
    // "CONFIG SET" expects `parameter value` → 2 args after the
    // two-token name.
    expect(validateArgs(["CONFIG", "SET", "maxmemory"])).toMatch(
      /CONFIG SET expects 2 arguments/,
    );
    expect(validateArgs(["CONFIG", "SET", "maxmemory", "100mb"])).toBeNull();
  });
});
