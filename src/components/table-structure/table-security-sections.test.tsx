// @vitest-environment jsdom
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import {
  PrivilegesSection,
  RowLevelSecuritySection,
  TriggersSection,
} from "./table-security-sections";

const pg = (queueOp = vi.fn()) => ({
  schema: "public",
  table: "orders",
  hasPrimaryKey: true,
  queueOp,
});

describe("table security structure sections", () => {
  it("hides trigger, policy, and privilege sections when capabilities are false", () => {
    const { container } = render(
      <>
        <TriggersSection
          triggers={[]}
          supported={false}
          pg={pg()}
          functions={[]}
        />
        <RowLevelSecuritySection
          rowSecurity={null}
          policies={[]}
          supported={false}
          pg={pg()}
          roles={[]}
        />
        <PrivilegesSection
          privileges={[]}
          supported={false}
          pg={pg()}
          roles={[]}
        />
      </>,
    );
    expect(container.textContent).toBe("");
  });

  it("renders triggers and queues enable/disable and drop", () => {
    const queueOp = vi.fn();
    render(
      <TriggersSection
        supported
        triggers={[
          {
            name: "orders_touch",
            timing: "BEFORE",
            events: ["UPDATE"],
            updateColumns: [],
            level: "ROW",
            enabled: "origin",
            functionSchema: "public",
            functionName: "touch",
            definition: "CREATE TRIGGER",
          },
        ]}
        pg={pg(queueOp)}
        functions={[]}
      />,
    );
    fireEvent.click(screen.getByRole("checkbox", { name: "cascade" }));
    fireEvent.click(screen.getByRole("button", { name: "Disable" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Drop trigger orders_touch" }),
    );
    expect(queueOp.mock.calls.map(([op]) => op)).toEqual([
      {
        op: "setTriggerEnabled",
        schema: "public",
        table: "orders",
        name: "orders_touch",
        mode: "disable",
      },
      {
        op: "dropTrigger",
        schema: "public",
        table: "orders",
        name: "orders_touch",
        cascade: true,
      },
    ]);
  });

  it("retains schema for duplicate zero-argument trigger function names", () => {
    const queueOp = vi.fn();
    render(
      <TriggersSection
        supported
        triggers={[]}
        pg={pg(queueOp)}
        functions={[
          {
            schema: "public",
            name: "route_event",
            identityArgs: "",
            returns: "trigger",
          },
          {
            schema: "audit",
            name: "route_event",
            identityArgs: "",
            returns: "trigger",
          },
          {
            schema: "audit",
            name: "route_event",
            identityArgs: "integer",
            returns: "trigger",
          },
          {
            schema: "public",
            name: "calculate_total",
            identityArgs: "numeric",
            returns: "numeric",
          },
        ]}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "New trigger" }));

    const picker = screen.getByRole("combobox", { name: "Trigger function" });
    const list = document.getElementById(
      picker.getAttribute("list") ?? "missing",
    );
    expect(
      [...(list?.querySelectorAll("option") ?? [])].map(
        (option) => option.value,
      ),
    ).toEqual([
      "public.route_event() → trigger",
      "audit.route_event() → trigger",
    ]);

    fireEvent.change(screen.getByRole("textbox", { name: "Trigger name" }), {
      target: { value: "orders_route" },
    });
    fireEvent.change(picker, {
      target: { value: "audit.route_event() → trigger" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Queue trigger" }));

    expect(queueOp).toHaveBeenCalledWith(
      expect.objectContaining({
        op: "createTrigger",
        functionSchema: "audit",
        functionName: "route_event",
      }),
    );
  });

  it("allows a free-text trigger function outside the known choices", () => {
    const queueOp = vi.fn();
    render(
      <TriggersSection
        supported
        triggers={[]}
        pg={pg(queueOp)}
        functions={[]}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "New trigger" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Trigger name" }), {
      target: { value: "orders_custom" },
    });
    fireEvent.change(
      screen.getByRole("textbox", { name: "Trigger function schema" }),
      { target: { value: "custom" } },
    );
    fireEvent.change(
      screen.getByRole("combobox", { name: "Trigger function" }),
      {
        target: { value: "route_custom_event" },
      },
    );
    fireEvent.click(screen.getByRole("button", { name: "Queue trigger" }));

    expect(queueOp).toHaveBeenCalledWith(
      expect.objectContaining({
        op: "createTrigger",
        functionSchema: "custom",
        functionName: "route_custom_event",
      }),
    );
  });

  it("only queues table trigger combinations PostgreSQL accepts", () => {
    render(
      <TriggersSection supported triggers={[]} pg={pg()} functions={[]} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "New trigger" }));
    const timing = screen.getByRole<HTMLSelectElement>("combobox", {
      name: "Trigger timing",
    });
    expect([...timing.options].map((option) => option.value)).toEqual([
      "before",
      "after",
    ]);
    fireEvent.change(screen.getByRole("textbox", { name: "Trigger name" }), {
      target: { value: "orders_truncate" },
    });
    fireEvent.change(
      screen.getByRole("combobox", { name: "Trigger function" }),
      { target: { value: "handle_truncate" } },
    );
    fireEvent.click(screen.getByRole("checkbox", { name: "TRUNCATE" }));
    const queueTrigger = screen.getByRole<HTMLButtonElement>("button", {
      name: "Queue trigger",
    });
    expect(queueTrigger.disabled).toBe(true);
    fireEvent.change(screen.getByRole("combobox", { name: "Trigger level" }), {
      target: { value: "statement" },
    });
    expect(queueTrigger.disabled).toBe(false);
  });

  it("queues policy edits as drop then create", () => {
    const queueOp = vi.fn();
    render(
      <RowLevelSecuritySection
        supported
        rowSecurity={{ enabled: true, forced: false }}
        policies={[
          {
            name: "tenant",
            permissive: true,
            command: "SELECT",
            roles: ["public"],
            using: "tenant_id = 1",
            withCheck: null,
          },
        ]}
        pg={pg(queueOp)}
        roles={["reader"]}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(
      screen.getByRole("combobox", { name: "Policy role 1 kind" }),
      { target: { value: "public" } },
    );
    fireEvent.click(screen.getByRole("button", { name: "Queue policy" }));
    expect(queueOp.mock.calls.map(([op]) => op.op)).toEqual([
      "dropPolicy",
      "createPolicy",
    ]);
    expect(queueOp).toHaveBeenLastCalledWith(
      expect.objectContaining({
        op: "createPolicy",
        roles: [{ kind: "public" }],
      }),
    );
  });

  it("disables policy expressions that PostgreSQL rejects for the command", () => {
    render(
      <RowLevelSecuritySection
        supported
        rowSecurity={{ enabled: true, forced: false }}
        policies={[]}
        pg={pg()}
        roles={[]}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "New policy" }));
    const using = screen.getByRole<HTMLInputElement>("textbox", {
      name: "Policy using",
    });
    const withCheck = screen.getByRole<HTMLInputElement>("textbox", {
      name: "Policy with check",
    });

    fireEvent.change(screen.getByRole("combobox", { name: "Policy command" }), {
      target: { value: "insert" },
    });
    expect(using.disabled).toBe(true);
    expect(withCheck.disabled).toBe(false);

    fireEvent.change(screen.getByRole("combobox", { name: "Policy command" }), {
      target: { value: "select" },
    });
    expect(using.disabled).toBe(false);
    expect(withCheck.disabled).toBe(true);
  });

  it("always requires an explicit choice for an introspected public policy", () => {
    const queueOp = vi.fn();
    const policy = {
      name: "tenant",
      permissive: true,
      command: "SELECT" as const,
      roles: ["public"],
      using: "tenant_id = 1",
      withCheck: null,
    };
    render(
      <RowLevelSecuritySection
        supported
        rowSecurity={{ enabled: true, forced: false }}
        policies={[policy]}
        pg={pg(queueOp)}
        roles={[]}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const queuePolicy = screen.getByRole<HTMLButtonElement>("button", {
      name: "Queue policy",
    });
    expect(queuePolicy.disabled).toBe(true);

    fireEvent.change(
      screen.getByRole("combobox", { name: "Policy role 1 kind" }),
      { target: { value: "public" } },
    );
    expect(queuePolicy.disabled).toBe(false);
    fireEvent.click(queuePolicy);
    expect(queueOp).toHaveBeenLastCalledWith(
      expect.objectContaining({ roles: [{ kind: "public" }] }),
    );
  });

  it("resets the policy form when switching edit targets", () => {
    const queueOp = vi.fn();
    render(
      <RowLevelSecuritySection
        supported
        rowSecurity={{ enabled: true, forced: false }}
        policies={[
          {
            name: "tenant_a",
            permissive: true,
            command: "SELECT",
            roles: ["reader"],
            using: "tenant_id = 1",
            withCheck: null,
          },
          {
            name: "tenant_b",
            permissive: false,
            command: "UPDATE",
            roles: ["writer"],
            using: "tenant_id = 2",
            withCheck: "active",
          },
        ]}
        pg={pg(queueOp)}
        roles={["reader", "writer"]}
      />,
    );

    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]);
    expect(
      screen.getByRole<HTMLInputElement>("textbox", { name: "Policy name" })
        .value,
    ).toBe("tenant_a");

    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[1]);
    expect(
      screen.getByRole<HTMLInputElement>("textbox", { name: "Policy name" })
        .value,
    ).toBe("tenant_b");
    expect(
      screen.getByRole<HTMLInputElement>("combobox", {
        name: "Policy role 1",
      }).value,
    ).toBe("writer");
    expect(
      screen.getByRole<HTMLInputElement>("textbox", { name: "Policy using" })
        .value,
    ).toBe("tenant_id = 2");
    expect(
      screen.getByRole<HTMLInputElement>("textbox", {
        name: "Policy with check",
      }).value,
    ).toBe("active");

    fireEvent.click(screen.getByRole("button", { name: "Queue policy" }));
    expect(queueOp).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({ op: "dropPolicy", name: "tenant_b" }),
    );
    expect(queueOp).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({ op: "createPolicy", name: "tenant_b" }),
    );
  });

  it("keeps disabling RLS staged by clearing and locking the forced choice", () => {
    const queueOp = vi.fn();
    render(
      <RowLevelSecuritySection
        supported
        rowSecurity={{ enabled: true, forced: true }}
        policies={[]}
        pg={pg(queueOp)}
        roles={[]}
      />,
    );
    const enabled = screen.getByRole<HTMLInputElement>("checkbox", {
      name: "Enabled",
    });
    const forced = screen.getByRole<HTMLInputElement>("checkbox", {
      name: "Forced",
    });

    fireEvent.click(enabled);
    expect(enabled.checked).toBe(false);
    expect(forced.checked).toBe(false);
    expect(forced.disabled).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Queue RLS" }));

    expect(queueOp.mock.calls.map(([op]) => op)).toEqual([
      {
        op: "setRowLevelSecurity",
        schema: "public",
        table: "orders",
        enabled: false,
        force: false,
      },
    ]);
  });

  it("queues enabled and forced together from the staged RLS choice", () => {
    const queueOp = vi.fn();
    render(
      <RowLevelSecuritySection
        supported
        rowSecurity={{ enabled: false, forced: false }}
        policies={[]}
        pg={pg(queueOp)}
        roles={[]}
      />,
    );

    fireEvent.click(screen.getByRole("checkbox", { name: "Enabled" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Forced" }));
    fireEvent.click(screen.getByRole("button", { name: "Queue RLS" }));

    expect(queueOp).toHaveBeenCalledTimes(1);
    expect(queueOp).toHaveBeenCalledWith({
      op: "setRowLevelSecurity",
      schema: "public",
      table: "orders",
      enabled: true,
      force: true,
    });
  });

  it("scopes grants to relation privileges and queues revoke", () => {
    const queueOp = vi.fn();
    render(
      <PrivilegesSection
        supported
        privileges={[
          { grantee: "reader", privilege: "SELECT", grantable: false },
        ]}
        pg={pg(queueOp)}
        roles={["reader"]}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Grant" }));
    expect(screen.queryByText("EXECUTE")).toBeNull();
    expect(screen.queryByText("MAINTAIN")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Queue grant" }));
    fireEvent.click(screen.getByRole("button", { name: "Revoke" }));
    expect(queueOp.mock.calls).toEqual([
      [
        expect.objectContaining({
          op: "grantPrivileges",
          target: {
            kind: "table",
            schema: "public",
            name: "orders",
            identityArgs: null,
          },
          grantee: { kind: "public" },
          privileges: ["select"],
        }),
      ],
      [
        expect.objectContaining({
          op: "revokePrivileges",
          grantee: { kind: "role", name: "reader" },
          privileges: ["select"],
        }),
      ],
    ]);
  });

  it("disables queueing a grant when no privileges are selected", () => {
    render(
      <PrivilegesSection supported privileges={[]} pg={pg()} roles={[]} />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Grant" }));

    const queueGrant = screen.getByRole<HTMLButtonElement>("button", {
      name: "Queue grant",
    });
    expect(queueGrant.disabled).toBe(false);

    fireEvent.click(screen.getByRole("checkbox", { name: "SELECT" }));
    expect(queueGrant.disabled).toBe(true);
  });

  it("preserves manually entered lowercase public as a grant role", () => {
    const queueOp = vi.fn();
    render(
      <PrivilegesSection
        supported
        privileges={[]}
        pg={pg(queueOp)}
        roles={["public"]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Grant" }));
    fireEvent.change(
      screen.getByRole("combobox", { name: "Privilege grantee kind" }),
      { target: { value: "role" } },
    );
    fireEvent.change(
      screen.getByRole("combobox", { name: "Privilege grantee" }),
      { target: { value: "public" } },
    );
    fireEvent.click(screen.getByRole("button", { name: "Queue grant" }));

    expect(queueOp).toHaveBeenCalledWith(
      expect.objectContaining({
        op: "grantPrivileges",
        grantee: { kind: "role", name: "public" },
      }),
    );
  });

  it("keeps an uppercase PUBLIC role distinct in grant state", () => {
    const queueOp = vi.fn();
    render(
      <PrivilegesSection
        supported
        privileges={[]}
        pg={pg(queueOp)}
        roles={["PUBLIC"]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Grant" }));
    fireEvent.change(
      screen.getByRole("combobox", { name: "Privilege grantee kind" }),
      { target: { value: "role" } },
    );
    fireEvent.change(
      screen.getByRole("combobox", { name: "Privilege grantee" }),
      { target: { value: "PUBLIC" } },
    );
    fireEvent.click(screen.getByRole("button", { name: "Queue grant" }));

    expect(queueOp).toHaveBeenCalledWith(
      expect.objectContaining({
        op: "grantPrivileges",
        grantee: { kind: "role", name: "PUBLIC" },
      }),
    );
  });

  it("requires an explicit target for colliding PUBLIC privilege rows and deduplicates privileges", () => {
    const queueOp = vi.fn();
    render(
      <PrivilegesSection
        supported
        privileges={[
          { grantee: "PUBLIC", privilege: "SELECT", grantable: false },
          { grantee: "PUBLIC", privilege: "SELECT", grantable: true },
        ]}
        pg={pg(queueOp)}
        roles={["PUBLIC"]}
      />,
    );

    const revoke = screen.getByRole<HTMLButtonElement>("button", {
      name: "Revoke",
    });
    expect(revoke.disabled).toBe(true);
    fireEvent.change(
      screen.getByRole("combobox", { name: "Revoke target for PUBLIC" }),
      { target: { value: "role" } },
    );
    expect(revoke.disabled).toBe(false);
    fireEvent.click(revoke);

    expect(queueOp).toHaveBeenCalledWith(
      expect.objectContaining({
        op: "revokePrivileges",
        grantee: { kind: "role", name: "PUBLIC" },
        privileges: ["select"],
      }),
    );
  });

  it("never infers an introspected PUBLIC privilege from changing role snapshots", () => {
    const queueOp = vi.fn();
    const props = {
      supported: true,
      privileges: [
        { grantee: "PUBLIC", privilege: "SELECT", grantable: false },
      ],
      pg: pg(queueOp),
      roles: [],
    };
    const { rerender } = render(<PrivilegesSection {...props} />);
    const revoke = screen.getByRole<HTMLButtonElement>("button", {
      name: "Revoke",
    });
    expect(revoke.disabled).toBe(true);

    rerender(<PrivilegesSection {...props} roles={["reader"]} />);
    expect(revoke.disabled).toBe(true);
    fireEvent.change(
      screen.getByRole("combobox", { name: "Revoke target for PUBLIC" }),
      { target: { value: "public" } },
    );
    expect(revoke.disabled).toBe(false);
    fireEvent.click(revoke);
    expect(queueOp).toHaveBeenCalledWith(
      expect.objectContaining({ grantee: { kind: "public" } }),
    );
  });
});
