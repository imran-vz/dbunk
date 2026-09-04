/* oxlint-disable anti-slop/no-module-mocking -- The review tests hold the typed preview/apply boundary under deterministic control. */
// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const clientMocks = vi.hoisted(() => ({
  preview: vi.fn(),
  apply: vi.fn(),
}));
const tauriMocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@/lib/object-ddl", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/object-ddl")>();
  return {
    ...actual,
    previewObjectDdl: clientMocks.preview,
    applyObjectDdlWithSafetyConfirmation: clientMocks.apply,
  };
});

vi.mock("@/lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/tauri")>();
  return { ...actual, tauriInvoke: tauriMocks.invoke };
});

import { DdlReviewDialog } from "@/components/object-ddl/ddl-review-dialog";
import {
  pgObjectDescriptionKey,
  type Connection,
  type DdlPlanPreview,
  type PgObjectDescription,
  type PgObjectOp,
  type PgObjectRef,
  useAppStore,
} from "@/lib/store";

const initialStoreState = useAppStore.getState();

const connection: Connection = {
  id: "conn-1",
  name: "Staging",
  database: "app",
  status: "Connected",
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "8 ms",
  ssl: false,
  environment: "staging",
  safeMode: "protected",
  readOnly: false,
};

const sequenceRef: PgObjectRef = {
  kind: "sequence",
  schema: "public",
  name: "invoice_number",
  identityArgs: null,
};

const alterSequence: PgObjectOp = {
  op: "alterSequence",
  schema: "public",
  name: "invoice_number",
  restartWith: null,
  incrementBy: "10",
  minValue: null,
  maxValue: null,
  cycle: null,
  cache: null,
};

const multiGroupPreview: DdlPlanPreview = {
  statements: [
    {
      sql: "ALTER SEQUENCE public.invoice_number INCREMENT BY 10",
      summary: "Alter sequence public.invoice_number",
      destructive: false,
      transactional: true,
    },
    {
      sql: "DROP VIEW public.old_view",
      summary: "Drop view public.old_view",
      destructive: true,
      transactional: true,
    },
    {
      sql: "CREATE INDEX CONCURRENTLY users_email_idx ON public.users (email)",
      summary: "Create index concurrently users_email_idx",
      destructive: false,
      transactional: false,
    },
  ],
  groups: [
    { kind: "atomic", statementIndexes: [0, 1] },
    { kind: "standalone", statementIndex: 2 },
  ],
};

beforeEach(() => {
  clientMocks.preview.mockReset();
  clientMocks.apply.mockReset();
  tauriMocks.invoke.mockReset();
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({ connections: [connection] });
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("DdlReviewDialog", () => {
  it("does not preview or apply DDL for a disconnected connection", async () => {
    useAppStore.setState({
      connections: [{ ...connection, status: "Disconnected" }],
    });

    render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[alterSequence]}
        onOpenChange={() => undefined}
      />,
    );

    await waitFor(() => {
      expect(clientMocks.preview).not.toHaveBeenCalled();
      expect(
        screen
          .getByRole("button", { name: "Apply DDL" })
          .hasAttribute("disabled"),
      ).toBe(true);
    });
    expect(
      screen.getByText(
        "Connect to the PostgreSQL database before reviewing DDL.",
      ),
    ).toBeTruthy();
    expect(clientMocks.apply).not.toHaveBeenCalled();
  });

  it("renders grouped statements, destructive tone, and the standalone warning", async () => {
    let resolvePreview: (result: {
      kind: "ok";
      value: DdlPlanPreview;
    }) => void = () => undefined;
    clientMocks.preview.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolvePreview = resolve;
        }),
    );

    render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[alterSequence]}
        onOpenChange={() => undefined}
      />,
    );

    expect(
      screen
        .getByRole("button", { name: "Apply DDL" })
        .hasAttribute("disabled"),
    ).toBe(true);
    act(() => resolvePreview({ kind: "ok", value: multiGroupPreview }));

    expect(await screen.findByText("Group 1 · Atomic")).toBeTruthy();
    expect(screen.getByText("Group 2 · Standalone")).toBeTruthy();
    expect(screen.getByText("Drop view public.old_view")).toBeTruthy();
    expect(screen.getByText("Destructive")).toBeTruthy();
    expect(
      screen.getByText(
        "Runs outside a transaction. Earlier statements stay applied if it fails.",
      ),
    ).toBeTruthy();
    expect(
      screen
        .getByRole("button", { name: "Apply DDL" })
        .hasAttribute("disabled"),
    ).toBe(false);
  });

  it("blocks a second apply while the first is in flight", async () => {
    clientMocks.preview.mockResolvedValue({
      kind: "ok",
      value: multiGroupPreview,
    });
    let resolveApply: (result: {
      kind: "ok";
      value: { appliedStatements: number; runtimeMs: number };
    }) => void = () => undefined;
    clientMocks.apply.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveApply = resolve;
        }),
    );

    const onOpenChange = vi.fn();
    const onApplyingChange = vi.fn();
    render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[alterSequence]}
        onOpenChange={onOpenChange}
        onApplyingChange={onApplyingChange}
      />,
    );

    const apply = await screen.findByRole("button", { name: "Apply DDL" });
    fireEvent.click(apply);
    fireEvent.click(apply);
    expect(clientMocks.apply).toHaveBeenCalledOnce();
    expect(onApplyingChange).toHaveBeenLastCalledWith(true);
    expect(
      screen
        .getByRole("button", { name: "Applying…" })
        .hasAttribute("disabled"),
    ).toBe(true);
    const close = screen.getByRole("button", { name: "Close" });
    expect(close.hasAttribute("disabled")).toBe(true);
    fireEvent.click(close);
    fireEvent.click(screen.getByRole("button", { name: "Close DDL review" }));
    expect(onOpenChange).not.toHaveBeenCalled();

    await act(async () => {
      resolveApply({
        kind: "ok",
        value: { appliedStatements: 3, runtimeMs: 9 },
      });
    });
    expect(await screen.findByText("Applied in 9 ms.")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Applied" }).hasAttribute("disabled"),
    ).toBe(true);
    expect(onApplyingChange).toHaveBeenLastCalledWith(false);
  });

  it("clears the applying lifecycle after the dialog unmounts", async () => {
    clientMocks.preview.mockResolvedValue({
      kind: "ok",
      value: multiGroupPreview,
    });
    let resolveApply: (result: {
      kind: "error";
      error: {
        kind: "database";
        statementIndex: number;
        code: string;
        message: string;
        position: null;
        appliedStatements: number;
      };
    }) => void = () => undefined;
    clientMocks.apply.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveApply = resolve;
        }),
    );
    const onApplyingChange = vi.fn();
    const rendered = render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[alterSequence]}
        onOpenChange={() => undefined}
        onApplyingChange={onApplyingChange}
      />,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));
    expect(onApplyingChange).toHaveBeenLastCalledWith(true);
    rendered.unmount();
    await act(async () => {
      resolveApply({
        kind: "error",
        error: {
          kind: "database",
          statementIndex: 0,
          code: "XX000",
          message: "apply failed",
          position: null,
          appliedStatements: 0,
        },
      });
      await Promise.resolve();
    });

    expect(onApplyingChange).toHaveBeenLastCalledWith(false);
  });

  it("reconciles applied identity when a newer metadata refresh wins", async () => {
    clientMocks.preview.mockResolvedValue({
      kind: "ok",
      value: {
        statements: [
          {
            sql: 'ALTER SCHEMA "public" RENAME TO "app"',
            summary: "Rename schema public to app",
            destructive: false,
            transactional: true,
          },
        ],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      },
    });
    clientMocks.apply.mockResolvedValue({
      kind: "ok",
      value: { appliedStatements: 1, runtimeMs: 4 },
    });
    useAppStore.setState({
      loadPgObjectCatalog: vi.fn(() => Promise.resolve("stale" as const)),
    });
    const onApplied = vi.fn();
    const schemaRef: PgObjectRef = {
      kind: "schema",
      schema: null,
      name: "public",
      identityArgs: null,
    };

    render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[{ op: "renameObject", reference: schemaRef, newName: "app" }]}
        onApplied={onApplied}
        onOpenChange={() => undefined}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));

    await waitFor(() =>
      expect(onApplied).toHaveBeenCalledWith(
        { appliedStatements: 1, runtimeMs: 4 },
        0,
      ),
    );
  });

  it("discloses index and grant loss for materialized-view recreation", async () => {
    clientMocks.preview.mockResolvedValue({
      kind: "ok",
      value: multiGroupPreview,
    });
    const materializedView = {
      kind: "materialized-view" as const,
      schema: "public",
      name: "sales_rollup",
      identityArgs: null,
    };

    render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[
          {
            op: "dropObject",
            reference: materializedView,
            cascade: false,
          },
          {
            op: "createMaterializedView",
            schema: "public",
            name: "sales_rollup",
            sqlBody: "SELECT 1",
            withData: true,
          },
        ]}
        onOpenChange={() => undefined}
      />,
    );

    expect(
      await screen.findByText(
        "Recreating this materialized view drops its indexes and grants.",
      ),
    ).toBeTruthy();
  });

  it("restores a terminal review without previewing or enabling replay", () => {
    render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[alterSequence]}
        initialTerminal={{
          result: "partial",
          outcome: "One statement was applied before the failure.",
          preview: multiGroupPreview,
          reviewedDdlVersion: 0,
          reviewedConnectionEpoch: 0,
        }}
        onOpenChange={() => undefined}
      />,
    );

    expect(
      screen.getByText("One statement was applied before the failure."),
    ).toBeTruthy();
    expect(
      screen
        .getByRole("button", { name: "Partially applied" })
        .hasAttribute("disabled"),
    ).toBe(true);
    expect(clientMocks.preview).not.toHaveBeenCalled();
    expect(clientMocks.apply).not.toHaveBeenCalled();
  });

  it.each([
    {
      name: "success",
      result: {
        kind: "ok" as const,
        value: { appliedStatements: 1, runtimeMs: 4 },
      },
      outcome: "Applied in 4 ms.",
    },
    {
      name: "partial database failure",
      result: {
        kind: "error" as const,
        error: {
          kind: "database" as const,
          statementIndex: 0,
          code: "XX000",
          message: "apply failed",
          position: null,
          appliedStatements: 1,
        },
      },
      outcome:
        "Statement 1 (Alter sequence public.invoice_number) failed: apply failed [XX000]. 1 earlier statement was applied.",
    },
  ])(
    "refreshes described objects after $name without reloading the catalog",
    async ({ result, outcome }) => {
      clientMocks.preview.mockResolvedValue({
        kind: "ok",
        value: {
          statements: [multiGroupPreview.statements[0]],
          groups: [{ kind: "atomic", statementIndexes: [0] }],
        },
      });
      clientMocks.apply.mockResolvedValue(result);
      const loadCatalog = vi.fn(() => Promise.resolve("ready" as const));
      const loadDescription = vi.fn(() => Promise.resolve("ready" as const));
      const descriptionKey = pgObjectDescriptionKey(connection.id, sequenceRef);
      useAppStore.setState({
        loadPgObjectCatalog: loadCatalog,
        loadPgObjectDescription: loadDescription,
        pgObjectDescriptions: {
          [descriptionKey]: {
            status: "ready",
            generation: 0,
            description: {
              reference: sequenceRef,
              owner: "postgres",
              comment: null,
              definitionSql: "CREATE SEQUENCE public.invoice_number",
              facts: {
                kind: "sequence",
                dataType: "bigint",
                start: "1",
                increment: "1",
                minValue: "1",
                maxValue: "9223372036854775807",
                cycle: false,
                cache: "1",
                lastValue: null,
                ownedBy: null,
              },
            },
          },
        },
      });
      const onRefresh = vi.fn();
      const onPartiallyApplied = vi.fn();

      render(
        <DdlReviewDialog
          open
          variant="inline"
          connectionId={connection.id}
          ops={[alterSequence]}
          onOpenChange={() => undefined}
          onRefresh={onRefresh}
          onPartiallyApplied={onPartiallyApplied}
        />,
      );
      fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));

      expect(await screen.findByText(outcome)).toBeTruthy();
      await waitFor(() => expect(loadDescription).toHaveBeenCalledOnce());
      expect(loadDescription).toHaveBeenCalledWith(
        connection.id,
        sequenceRef,
        0,
      );
      expect(loadCatalog).not.toHaveBeenCalled();
      expect(onRefresh).toHaveBeenCalledOnce();
      if (result.kind === "ok") {
        expect(onPartiallyApplied).not.toHaveBeenCalled();
        expect(
          screen
            .getByRole("button", { name: "Applied" })
            .hasAttribute("disabled"),
        ).toBe(true);
      } else {
        expect(onPartiallyApplied).toHaveBeenCalledWith(result.error, 0);
        expect(clientMocks.preview).toHaveBeenCalledOnce();
        expect(
          screen
            .getByRole("button", { name: "Partially applied" })
            .hasAttribute("disabled"),
        ).toBe(true);
      }
    },
  );

  it("revalidates cached dependents after a cascading drop", async () => {
    const dependentRef: PgObjectRef = {
      kind: "materialized-view",
      schema: "public",
      name: "dependent_rollup",
      identityArgs: null,
    };
    const targetRef: PgObjectRef = {
      kind: "view",
      schema: "public",
      name: "source_view",
      identityArgs: null,
    };
    clientMocks.preview.mockResolvedValue({
      kind: "ok",
      value: {
        statements: [
          {
            sql: "DROP VIEW public.source_view CASCADE",
            summary: "Drop view public.source_view (CASCADE)",
            destructive: true,
            transactional: true,
          },
        ],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      },
    });
    clientMocks.apply.mockResolvedValue({
      kind: "ok",
      value: { appliedStatements: 1, runtimeMs: 3 },
    });
    const loadCatalog = vi.fn(() => Promise.resolve("ready" as const));
    const loadDescription = vi.fn(() => Promise.resolve("error" as const));
    useAppStore.setState({
      loadPgObjectCatalog: loadCatalog,
      loadPgObjectDescription: loadDescription,
      pgObjectDescriptions: {
        [pgObjectDescriptionKey(connection.id, targetRef)]: {
          status: "ready",
          generation: 0,
          description: {
            reference: targetRef,
            owner: "postgres",
            comment: null,
            definitionSql: "CREATE VIEW public.source_view AS SELECT 1",
            facts: { kind: "view", definition: "SELECT 1" },
          },
        },
        [pgObjectDescriptionKey(connection.id, dependentRef)]: {
          status: "ready",
          generation: 0,
          description: {
            reference: dependentRef,
            owner: "postgres",
            comment: null,
            definitionSql:
              "CREATE MATERIALIZED VIEW public.dependent_rollup AS SELECT 1 WITH DATA",
            facts: {
              kind: "materializedView",
              definition: "SELECT 1",
              populated: true,
            },
          },
        },
      },
    });

    render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[{ op: "dropObject", reference: targetRef, cascade: true }]}
        onOpenChange={() => undefined}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));

    expect(await screen.findByText("Applied in 3 ms.")).toBeTruthy();
    await waitFor(() => expect(loadDescription).toHaveBeenCalledTimes(2));
    expect(loadCatalog).toHaveBeenCalledWith(connection.id, 0);
    expect(loadDescription).toHaveBeenCalledWith(connection.id, targetRef, 0);
    expect(loadDescription).toHaveBeenCalledWith(
      connection.id,
      dependentRef,
      0,
    );
  });

  it("supersedes a loading dependent description after a cascading drop", async () => {
    const dependentRef: PgObjectRef = {
      kind: "view",
      schema: "public",
      name: "dependent_view",
      identityArgs: null,
    };
    const targetRef: PgObjectRef = {
      kind: "table",
      schema: "public",
      name: "source_table",
      identityArgs: null,
    };
    const preview: DdlPlanPreview = {
      statements: [
        {
          sql: "DROP TABLE public.source_table CASCADE",
          summary: "Drop table public.source_table (CASCADE)",
          destructive: true,
          transactional: true,
        },
      ],
      groups: [{ kind: "atomic", statementIndexes: [0] }],
    };
    clientMocks.preview.mockResolvedValue({ kind: "ok", value: preview });
    clientMocks.apply.mockResolvedValue({
      kind: "ok",
      value: { appliedStatements: 1, runtimeMs: 3 },
    });
    let resolvePreDrop: (description: PgObjectDescription) => void = () =>
      undefined;
    tauriMocks.invoke
      .mockImplementationOnce(
        () =>
          new Promise<PgObjectDescription>((resolve) => {
            resolvePreDrop = resolve;
          }),
      )
      .mockRejectedValueOnce({
        kind: "objectNotFound",
        reference: dependentRef,
      });
    useAppStore.setState({
      loadPgObjectCatalog: vi.fn(() => Promise.resolve("ready" as const)),
    });

    const preDropLoad = useAppStore
      .getState()
      .loadPgObjectDescription(connection.id, dependentRef, 0);
    const dependentKey = pgObjectDescriptionKey(connection.id, dependentRef);
    expect(useAppStore.getState().pgObjectDescriptions[dependentKey]).toEqual({
      status: "loading",
      generation: 0,
    });

    render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[{ op: "dropObject", reference: targetRef, cascade: true }]}
        onOpenChange={() => undefined}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));

    await waitFor(() => expect(tauriMocks.invoke).toHaveBeenCalledTimes(2));
    expect(tauriMocks.invoke).toHaveBeenNthCalledWith(2, "describe_pg_object", {
      payload: { connectionId: connection.id, reference: dependentRef },
    });
    expect(
      useAppStore.getState().pgObjectDescriptionRequestIds[dependentKey],
    ).toBe(2);
    await waitFor(() =>
      expect(useAppStore.getState().pgObjectDescriptions[dependentKey]).toEqual(
        {
          status: "error",
          generation: 0,
          error: { kind: "objectNotFound", reference: dependentRef },
        },
      ),
    );
    resolvePreDrop({
      reference: dependentRef,
      owner: "postgres",
      comment: null,
      definitionSql: "CREATE VIEW public.dependent_view AS SELECT 1",
      facts: { kind: "view", definition: "SELECT 1" },
    });

    await expect(preDropLoad).resolves.toBe("stale");
    expect(useAppStore.getState().pgObjectDescriptions[dependentKey]).toEqual({
      status: "error",
      generation: 0,
      error: { kind: "objectNotFound", reference: dependentRef },
    });
  });

  it("does not refresh metadata into a newer connection generation", async () => {
    let resolveApply: (result: {
      kind: "ok";
      value: { appliedStatements: number; runtimeMs: number };
    }) => void = () => undefined;
    clientMocks.preview.mockResolvedValue({
      kind: "ok",
      value: {
        statements: [multiGroupPreview.statements[0]],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      },
    });
    clientMocks.apply.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveApply = resolve;
        }),
    );
    const loadCatalog = vi.fn(() => Promise.resolve("ready" as const));
    const loadDescription = vi.fn(() => Promise.resolve("ready" as const));
    const onApplied = vi.fn();
    useAppStore.setState({
      loadPgObjectCatalog: loadCatalog,
      loadPgObjectDescription: loadDescription,
      pgObjectCatalog: {
        [connection.id]: { status: "ready", generation: 4 },
      },
      pgObjectDescriptions: {
        [pgObjectDescriptionKey(connection.id, sequenceRef)]: {
          status: "loading",
          generation: 4,
        },
      },
    });

    render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[alterSequence]}
        onApplied={onApplied}
        onOpenChange={() => undefined}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));
    useAppStore.setState({
      pgObjectCatalog: {
        [connection.id]: { status: "idle", generation: 5 },
      },
      pgObjectDescriptions: {},
    });
    act(() =>
      resolveApply({
        kind: "ok",
        value: { appliedStatements: 1, runtimeMs: 3 },
      }),
    );

    expect(await screen.findByText("Applied in 3 ms.")).toBeTruthy();
    expect(loadCatalog).not.toHaveBeenCalled();
    expect(loadDescription).not.toHaveBeenCalled();
    expect(onApplied).not.toHaveBeenCalled();
  });

  it("does not regenerate a partial-apply preview in a newer generation", async () => {
    let resolveApply: (result: {
      kind: "error";
      error: {
        kind: "database";
        statementIndex: number;
        code: string;
        message: string;
        position: null;
        appliedStatements: number;
      };
    }) => void = () => undefined;
    clientMocks.preview.mockResolvedValue({
      kind: "ok",
      value: {
        statements: [multiGroupPreview.statements[0]],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      },
    });
    clientMocks.apply.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveApply = resolve;
        }),
    );
    const loadCatalog = vi.fn(() => Promise.resolve("ready" as const));
    const loadDescription = vi.fn(() => Promise.resolve("ready" as const));
    useAppStore.setState({
      loadPgObjectCatalog: loadCatalog,
      loadPgObjectDescription: loadDescription,
      pgObjectCatalog: {
        [connection.id]: { status: "ready", generation: 4 },
      },
    });

    render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[alterSequence]}
        onOpenChange={() => undefined}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));
    act(() => {
      useAppStore.setState({
        pgObjectCatalog: {
          [connection.id]: { status: "idle", generation: 5 },
        },
      });
      resolveApply({
        kind: "error",
        error: {
          kind: "database",
          statementIndex: 0,
          code: "XX000",
          message: "apply failed",
          position: null,
          appliedStatements: 1,
        },
      });
    });

    expect(
      await screen.findByText(
        "Statement 1 (Alter sequence public.invoice_number) failed: apply failed [XX000]. 1 earlier statement was applied.",
      ),
    ).toBeTruthy();
    expect(clientMocks.preview).toHaveBeenCalledOnce();
    expect(loadCatalog).not.toHaveBeenCalled();
    expect(loadDescription).not.toHaveBeenCalled();
    expect(
      screen
        .getByRole("button", { name: "Partially applied" })
        .hasAttribute("disabled"),
    ).toBe(true);
  });

  it("requires a new preview after the connection lifetime changes", async () => {
    clientMocks.preview.mockResolvedValue({
      kind: "ok",
      value: {
        statements: [multiGroupPreview.statements[0]],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      },
    });

    render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[alterSequence]}
        onOpenChange={() => undefined}
      />,
    );
    await screen.findByRole("button", { name: "Apply DDL" });

    act(() => {
      useAppStore.setState({ connectionEpochs: { [connection.id]: 1 } });
    });
    const regenerate = await screen.findByRole("button", {
      name: "Regenerate preview",
    });
    fireEvent.click(regenerate);

    await waitFor(() => expect(clientMocks.preview).toHaveBeenCalledTimes(2));
    expect(
      screen
        .getByRole("button", { name: "Apply DDL" })
        .hasAttribute("disabled"),
    ).toBe(false);
    expect(clientMocks.apply).not.toHaveBeenCalled();
  });

  it("keeps a connection apply locked across viewers and different plans", async () => {
    let resolveApply: (result: {
      kind: "ok";
      value: { appliedStatements: number; runtimeMs: number };
    }) => void = () => undefined;
    clientMocks.preview.mockResolvedValue({
      kind: "ok",
      value: {
        statements: [multiGroupPreview.statements[0]],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      },
    });
    clientMocks.apply.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveApply = resolve;
        }),
    );

    const first = render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[
          {
            op: "setComment",
            target: { kind: "object", reference: sequenceRef },
            comment: "Locked while another plan applies",
          },
        ]}
        onOpenChange={() => undefined}
      />,
    );
    render(
      <DdlReviewDialog
        open
        variant="inline"
        connectionId={connection.id}
        ops={[alterSequence]}
        onOpenChange={() => undefined}
      />,
    );
    const applyButtons = await screen.findAllByRole("button", {
      name: "Apply DDL",
    });
    const firstApply = applyButtons[0];
    if (!firstApply) throw new Error("first apply button is missing");
    fireEvent.click(firstApply);
    await waitFor(() =>
      expect(useAppStore.getState().pgObjectDdlApplying).not.toEqual({}),
    );

    first.unmount();
    expect(
      screen
        .getByRole("button", { name: "Applying…" })
        .hasAttribute("disabled"),
    ).toBe(true);

    act(() =>
      resolveApply({
        kind: "ok",
        value: { appliedStatements: 1, runtimeMs: 2 },
      }),
    );
    await waitFor(() =>
      expect(
        screen
          .getByRole("button", { name: "Regenerate preview" })
          .hasAttribute("disabled"),
      ).toBe(false),
    );
    expect(clientMocks.apply).toHaveBeenCalledOnce();
  });
});
