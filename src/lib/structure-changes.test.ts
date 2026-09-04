import { describe, expect, it } from "vitest";

import {
  asPgPolicyCommand,
  asPgTriggerEvent,
  buildGrantOp,
  buildPolicyOps,
  buildRevokeOp,
  buildTriggerOps,
} from "./structure-changes";

describe("table security operation builders", () => {
  it("shares case-insensitive policy and trigger input conversion", () => {
    expect(asPgPolicyCommand("SELECT")).toBe("select");
    expect(() => asPgPolicyCommand("unknown")).toThrow(
      "Unsupported PostgreSQL policy command: unknown",
    );
    expect(asPgTriggerEvent("INSERT")).toEqual({ kind: "insert" });
    expect(() => asPgTriggerEvent("unknown")).toThrow(
      "Unsupported PostgreSQL trigger event: unknown",
    );
  });

  it("normalizes PUBLIC and preserves relation privileges", () => {
    expect(
      buildGrantOp({
        schema: "public",
        table: "orders",
        grantee: { kind: "public" },
        privileges: ["select", "maintain"],
        withGrantOption: true,
      }),
    ).toMatchObject({
      op: "grantPrivileges",
      target: { kind: "table", schema: "public", name: "orders" },
      grantee: { kind: "public" },
      privileges: ["select", "maintain"],
      withGrantOption: true,
    });
    expect(
      buildRevokeOp({
        schema: "public",
        table: "orders",
        grantee: { kind: "role", name: " reader " },
        privileges: ["select"],
        cascade: true,
      }),
    ).toMatchObject({
      op: "revokePrivileges",
      grantee: { kind: "role", name: "reader" },
      cascade: true,
    });
  });

  it("keeps PUBLIC and identically named quoted roles distinct", () => {
    expect(
      buildGrantOp({
        schema: "public",
        table: "orders",
        grantee: { kind: "role", name: " PUBLIC " },
        privileges: ["select"],
        withGrantOption: false,
      }),
    ).toMatchObject({ grantee: { kind: "role", name: "PUBLIC" } });
  });

  it("orders policy replacement and inline trigger function creation", () => {
    expect(
      buildPolicyOps({
        schema: "public",
        table: "orders",
        dropExisting: "tenant_policy",
        name: "tenant_policy",
        permissive: true,
        command: "select",
        roles: [{ kind: "public" }],
        using: "tenant_id = current_setting('app.tenant')::int",
        withCheck: "",
      }).map((op) => op.op),
    ).toEqual(["dropPolicy", "createPolicy"]);

    expect(
      buildTriggerOps({
        schema: "public",
        table: "orders",
        name: "orders_touch",
        timing: "before",
        events: [{ kind: "update", columns: [] }],
        forEach: "row",
        when: "",
        functionSchema: "public",
        functionName: "touch_order",
        createFunction: {
          language: "plpgsql",
          body: "BEGIN RETURN NEW; END;",
        },
      }).map((op) => op.op),
    ).toEqual(["createFunction", "createTrigger"]);
  });

  it("never builds FORCE RLS while RLS is disabled", () => {
    expect(
      buildPolicyOps({
        schema: "public",
        table: "orders",
        enabled: false,
        force: true,
        name: "tenant_policy",
        permissive: true,
        command: "all",
        roles: [{ kind: "public" }],
        using: "true",
        withCheck: "",
      })[0],
    ).toMatchObject({
      op: "setRowLevelSecurity",
      enabled: false,
      force: false,
    });
  });

  it.each([
    { command: "insert" as const, using: null, withCheck: "tenant_id = 7" },
    { command: "select" as const, using: "tenant_id = 7", withCheck: null },
    { command: "delete" as const, using: "tenant_id = 7", withCheck: null },
    {
      command: "update" as const,
      using: "tenant_id = 7",
      withCheck: "tenant_id = 7",
    },
  ])(
    "normalizes expressions accepted by $command policies",
    ({ command, using, withCheck }) => {
      expect(
        buildPolicyOps({
          schema: "public",
          table: "orders",
          name: "tenant_policy",
          permissive: true,
          command,
          roles: [{ kind: "public" }],
          using: "tenant_id = 7",
          withCheck: "tenant_id = 7",
        })[0],
      ).toMatchObject({ using, withCheck });
    },
  );
});
