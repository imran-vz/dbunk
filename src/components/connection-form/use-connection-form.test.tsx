/* oxlint-disable anti-slop/no-chained-type-assertions anti-slop/require-safety-comment-for-type-assertion -- Test harness narrows a bare TanStack form to the field API used by the hook helper. */
// @vitest-environment jsdom
import { useForm } from "@tanstack/react-form";
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { syntheticReachableDiagnosis } from "@/lib/connection-diagnosis";
import {
  type Connection,
  type StoredConnection,
  useAppStore,
} from "@/lib/store";

import { EMPTY_NEW_DEFAULTS } from "./form-utils";
import {
  type ConnectionFormApi,
  resetEngineSpecificDefaults,
  useConnectionForm,
} from "./use-connection-form";

function mountForm() {
  const { result } = renderHook(
    () =>
      useForm({
        defaultValues: {
          ...EMPTY_NEW_DEFAULTS,
          tlsMode: "verify-full" as const,
          tlsRootCertPath: "/ca.pem",
          tlsClientCertPath: "/c.crt",
          tlsClientKeyPath: "/c.key",
          tlsServerName: "db.internal",
          keepaliveSeconds: 30,
        },
      }) as unknown as ConnectionFormApi,
  );
  return result.current;
}

describe("resetEngineSpecificDefaults", () => {
  it("clears the PostgreSQL TLS fields and driver knobs when leaving PostgreSQL", () => {
    const form = mountForm();
    resetEngineSpecificDefaults(form, "MySQL");
    expect(form.state.values).toMatchObject({
      ssl: true,
      tlsMode: "prefer",
      tlsRootCertPath: "",
      tlsClientCertPath: "",
      tlsClientKeyPath: "",
      tlsServerName: "",
      keepaliveSeconds: undefined,
    });
  });

  it("keeps the TLS fields when the engine stays PostgreSQL", () => {
    const form = mountForm();
    resetEngineSpecificDefaults(form, "PostgreSQL");
    expect(form.state.values).toMatchObject({
      tlsMode: "verify-full",
      tlsRootCertPath: "/ca.pem",
      keepaliveSeconds: 30,
    });
  });
});

const STORED: Connection = {
  id: "conn-1",
  name: "orders",
  engine: "PostgreSQL",
  host: "db.internal",
  database: "orders",
  port: 5432,
  user: "app",
  password: "",
  role: "read/write",
  ssl: true,
  status: "Disconnected",
  latency: "--",
};

function stubDiagnose() {
  const diagnoseConnection = vi.fn(
    async (connection: StoredConnection, _hydrateFrom?: string) => ({
      ok: true as const,
      report: syntheticReachableDiagnosis(connection),
    }),
  );
  useAppStore.setState({ diagnoseConnection });
  return diagnoseConnection;
}

describe("useConnectionForm — Test Connection", () => {
  it("hydrates the saved credential in edit mode when the password is blank", async () => {
    const diagnoseConnection = stubDiagnose();
    const { result } = renderHook(() =>
      useConnectionForm({ mode: "edit", connection: STORED }),
    );
    await act(() => result.current.runTestConnection());
    expect(diagnoseConnection).toHaveBeenCalledTimes(1);
    expect(diagnoseConnection.mock.calls[0]?.[1]).toBe("conn-1");
    expect(result.current.testStatus.state).toBe("done");
  });

  it("sends a typed password instead of hydrating", async () => {
    const diagnoseConnection = stubDiagnose();
    const { result } = renderHook(() =>
      useConnectionForm({ mode: "edit", connection: STORED }),
    );
    act(() => {
      result.current.form.setFieldValue("password", "new-secret");
    });
    await act(() => result.current.runTestConnection());
    const [connection, hydrateFrom] = diagnoseConnection.mock.calls[0] ?? [];
    expect(connection?.password).toBe("new-secret");
    expect(hydrateFrom).toBeUndefined();
  });

  it("never hydrates in new mode", async () => {
    const diagnoseConnection = stubDiagnose();
    const { result } = renderHook(() => useConnectionForm({ mode: "new" }));
    await act(() => result.current.runTestConnection());
    expect(diagnoseConnection.mock.calls[0]?.[1]).toBeUndefined();
  });

  it("surfaces the outer rejection as an error status", async () => {
    useAppStore.setState({
      diagnoseConnection: vi.fn(async () => ({
        ok: false as const,
        error: "Connection not found",
      })),
    });
    const { result } = renderHook(() =>
      useConnectionForm({ mode: "edit", connection: STORED }),
    );
    await act(() => result.current.runTestConnection());
    expect(result.current.testStatus).toEqual({
      state: "error",
      error: "Connection not found",
    });
  });
});
