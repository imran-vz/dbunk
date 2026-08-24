import { describe, expect, it } from "vitest";

import { organizeConnections } from "./connection-organization";
import type { Connection } from "./store";

const conn = (
  id: string,
  name: string,
  overrides: {
    folder?: string;
    isFavorite?: boolean;
    lastActivityAt?: string;
  } = {},
): Connection => ({
  engine: "PostgreSQL",
  id,
  name,
  database: "app",
  host: "h",
  port: 5432,
  user: "u",
  password: "",
  role: "",
  ssl: true,
  status: "Disconnected",
  latency: "--",
  ...overrides,
});

describe("organizeConnections", () => {
  it("groups by folder alphabetically with ungrouped last", () => {
    const groups = organizeConnections([
      conn("1", "Loose one"),
      conn("2", "Zed", { folder: "Zulu" }),
      conn("3", "Alpha member", { folder: "Alpha" }),
    ]);
    expect(groups.map((group) => group.folder)).toEqual([
      "Alpha",
      "Zulu",
      "",
    ]);
  });

  it("orders favorites → recency → name inside a group", () => {
    const groups = organizeConnections([
      conn("stale", "A stale", {
        folder: "F",
        lastActivityAt: "2026-01-01T00:00:00Z",
      }),
      conn("recent", "Z recent", {
        folder: "F",
        lastActivityAt: "2026-08-01T00:00:00Z",
      }),
      conn("fav", "M favorite", {
        folder: "F",
        isFavorite: true,
      }),
      conn("noact-b", "B never used", { folder: "F" }),
      conn("noact-a", "A never used", { folder: "F" }),
    ]);
    expect(groups[0].connections.map((c) => c.id)).toEqual([
      "fav",
      "recent",
      "stale",
      "noact-a",
      "noact-b",
    ]);
  });

  it("treats blank and whitespace folders as ungrouped", () => {
    const groups = organizeConnections([
      conn("1", "One", { folder: "   " }),
      conn("2", "Two", { folder: "" }),
      conn("3", "Three"),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0].folder).toBe("");
    expect(groups[0].connections).toHaveLength(3);
  });

  it("ignores malformed activity timestamps rather than misordering", () => {
    const groups = organizeConnections([
      conn("good", "Good", { lastActivityAt: "2026-08-01T00:00:00Z" }),
      conn("bad", "Bad", { lastActivityAt: "not a date" }),
    ]);
    expect(groups[0].connections.map((c) => c.id)).toEqual(["good", "bad"]);
  });
});
