import { describe, expect, it } from "vitest";

import {
  bastionIdsReferencedByConnection,
  connectionReferencesBastion,
} from "./bastion-references";
import type { Connection } from "./types";

const connection: Extract<Connection, { engine: "PostgreSQL" }> = {
  id: "conn-1",
  name: "Primary PG",
  engine: "PostgreSQL",
  host: "db.internal",
  port: 5432,
  database: "postgres",
  user: "postgres",
  password: "",
  role: "read/write",
  ssl: true,
  sshTunnel: {
    enabled: true,
    bastionServerId: " final ",
    jumpChain: [" jump-1 ", "jump-1", ""],
  },
  status: "Connected",
  latency: "5ms",
};

describe("Bastion reference helpers", () => {
  it("normalizes final and jump-chain Bastion references", () => {
    expect(bastionIdsReferencedByConnection(connection)).toEqual([
      "jump-1",
      "final",
    ]);
    expect(connectionReferencesBastion(connection, "final")).toBe(true);
    expect(connectionReferencesBastion(connection, "jump-1")).toBe(true);
    expect(connectionReferencesBastion(connection, "other")).toBe(false);
  });

  it("ignores SQLite and disabled tunnel configs", () => {
    const sqliteConnection: Extract<Connection, { engine: "SQLite" }> = {
      id: "sqlite-1",
      name: "Local",
      engine: "SQLite",
      host: "",
      port: 0,
      database: "/tmp/local.sqlite",
      user: "",
      password: "",
      role: "",
      status: "Disconnected",
      latency: "--",
    };

    const disabledConnection: Connection = {
      ...connection,
      sshTunnel: { ...connection.sshTunnel, enabled: false },
    };

    expect(bastionIdsReferencedByConnection(sqliteConnection)).toEqual([]);
    expect(bastionIdsReferencedByConnection(disabledConnection)).toEqual([]);
  });
});
