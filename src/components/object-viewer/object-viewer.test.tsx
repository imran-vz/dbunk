/* oxlint-disable anti-slop/no-module-mocking, anti-slop/no-unknown-parameters -- Tauri and Monaco are replaced at the viewer's test boundaries. */
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

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

vi.mock("@/lib/invoke-with-safety-confirmation", () => ({
  invokeWithSafetyConfirmation: vi.fn(),
}));

vi.mock("@monaco-editor/react", () => ({
  default: ({
    value,
    onChange,
    options,
  }: {
    value: string;
    onChange: (value: string) => void;
    options?: { readOnly?: boolean };
  }) => (
    <textarea
      aria-label="Routine body"
      value={value}
      readOnly={options?.readOnly}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));

import {
  canRenameObjectKind,
  objectDefinitionTabLabel,
  ObjectViewer,
} from "@/components/object-viewer/object-viewer";
import { invokeWithSafetyConfirmation } from "@/lib/invoke-with-safety-confirmation";
import {
  pgObjectDescriptionKey,
  type Connection,
  type PgObjectDescription,
  type PgObjectFacts,
  type PgObjectKind,
  type PgObjectRef,
  useAppStore,
} from "@/lib/store";
import { tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);
const mockedSafetyInvoke = vi.mocked(invokeWithSafetyConfirmation);
const initialStoreState = useAppStore.getState();

const connection = (status: Connection["status"]): Connection => ({
  id: "conn-1",
  name: "warehouse_local",
  database: "postgres",
  status,
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "12 ms",
  ssl: false,
});

const referenceFor = (kind: PgObjectKind): PgObjectRef => {
  if (kind === "schema") {
    return {
      kind,
      schema: null,
      name: "lifecycle",
      identityArgs: null,
    };
  }
  if (kind === "function" || kind === "procedure" || kind === "aggregate") {
    return {
      kind,
      schema: "lifecycle",
      name: `sample_${kind}`,
      identityArgs: "integer",
    };
  }
  return {
    kind,
    schema: "lifecycle",
    name: `sample_${kind}`,
    identityArgs: null,
  };
};

const descriptionFor = (
  reference: PgObjectRef,
  facts: PgObjectFacts,
  definitionSql: string | null = "CREATE VIEW lifecycle.sample AS SELECT 1;",
): PgObjectDescription => ({
  reference,
  owner: "dbunk",
  comment: "Lifecycle fixture",
  definitionSql,
  facts,
});

beforeEach(() => {
  mockedInvoke.mockReset();
  mockedSafetyInvoke.mockReset();
  mockedSafetyInvoke.mockResolvedValue({ runtimeMs: 4 });
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({ connections: [connection("Connected")] });
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

const factCases: Array<{
  name: string;
  objectKind: PgObjectKind;
  facts: PgObjectFacts;
  expected: string;
}> = [
  {
    name: "schema",
    objectKind: "schema",
    facts: { kind: "schema" },
    expected: "Schema metadata has no additional typed facts.",
  },
  {
    name: "table",
    objectKind: "table",
    facts: { kind: "table" },
    expected: "Table structure is available in the table workspace.",
  },
  {
    name: "view",
    objectKind: "view",
    facts: { kind: "view", definition: "SELECT 1" },
    expected: "SELECT 1",
  },
  {
    name: "materialized view",
    objectKind: "materialized-view",
    facts: {
      kind: "materializedView",
      definition: "SELECT 2",
      populated: true,
    },
    expected: "SELECT 2",
  },
  {
    name: "foreign table",
    objectKind: "foreign-table",
    facts: { kind: "foreignTable", server: "warehouse_fdw" },
    expected: "warehouse_fdw",
  },
  {
    name: "sequence",
    objectKind: "sequence",
    facts: {
      kind: "sequence",
      dataType: "bigint",
      start: "10",
      increment: "5",
      minValue: "10",
      maxValue: "9999",
      cycle: false,
      cache: "20",
      lastValue: "45",
      ownedBy: "lifecycle.orders.id",
    },
    expected: "lifecycle.orders.id",
  },
  {
    name: "routine",
    objectKind: "function",
    facts: {
      kind: "routine",
      language: "plpgsql",
      returns: "integer",
      volatility: "stable",
      arguments: "value integer",
      body: null,
      strict: false,
      securityDefiner: false,
      parallel: null,
    },
    expected: "plpgsql",
  },
  {
    name: "type",
    objectKind: "type",
    facts: {
      kind: "type",
      class: "composite",
      enumLabels: null,
      attributes: [{ name: "state", dataType: "text", nullable: false }],
      subtype: null,
    },
    expected: "state text NOT NULL",
  },
  {
    name: "domain",
    objectKind: "domain",
    facts: {
      kind: "domain",
      baseType: "numeric",
      notNull: true,
      defaultValue: "0",
      checks: ["VALUE >= 0"],
    },
    expected: "VALUE >= 0",
  },
  {
    name: "extension",
    objectKind: "extension",
    facts: { kind: "extension", version: "1.3", schema: "public" },
    expected: "1.3",
  },
];

describe("ObjectViewer read-only facts", () => {
  it.each(factCases)("renders the $name facts arm", async (fixture) => {
    const reference = referenceFor(fixture.objectKind);
    mockedInvoke.mockResolvedValueOnce(
      descriptionFor(reference, fixture.facts),
    );

    render(
      <ObjectViewer
        tabId="tab-object"
        connectionId="conn-1"
        reference={reference}
      />,
    );

    fireEvent.click(await screen.findByRole("tab", { name: "facts" }));
    expect(await screen.findByText(fixture.expected)).toBeTruthy();
  });
});

describe("ObjectViewer lifecycle", () => {
  it("invokes nothing while disconnected, then describes after connect", async () => {
    const reference = referenceFor("view");
    const sql = "CREATE VIEW lifecycle.sample_view AS SELECT 1;";
    useAppStore.setState({ connections: [connection("Disconnected")] });
    mockedInvoke.mockResolvedValueOnce(
      descriptionFor(reference, { kind: "view", definition: "SELECT 1" }, sql),
    );
    render(
      <ObjectViewer
        tabId="tab-object"
        connectionId="conn-1"
        reference={reference}
      />,
    );

    expect(screen.getByRole("button", { name: "Connect & load" })).toBeTruthy();
    expect(mockedInvoke).not.toHaveBeenCalled();

    act(() => {
      useAppStore.setState({ connections: [connection("Connected")] });
    });

    expect(await screen.findByText(sql)).toBeTruthy();
    expect(mockedInvoke).toHaveBeenCalledOnce();
    expect(mockedInvoke).toHaveBeenCalledWith("describe_pg_object", {
      payload: { connectionId: "conn-1", reference },
    });
  });

  it("renders objectNotFound as an empty state with close", async () => {
    const reference = referenceFor("sequence");
    const closeTab = vi.fn(() => Promise.resolve());
    useAppStore.setState({ closeTab });
    mockedInvoke.mockRejectedValueOnce({ kind: "objectNotFound", reference });

    render(
      <ObjectViewer
        tabId="tab-object"
        connectionId="conn-1"
        reference={reference}
      />,
    );

    expect(await screen.findByText(/no longer exists/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Close tab" }));
    expect(closeTab).toHaveBeenCalledWith("tab-object");
  });

  it("revalidates a loaded viewer after focus and surfaces an external drop", async () => {
    const reference = referenceFor("view");
    mockedInvoke
      .mockResolvedValueOnce(
        descriptionFor(
          reference,
          { kind: "view", definition: "SELECT 1" },
          "SELECT 1",
        ),
      )
      .mockRejectedValueOnce({ kind: "objectNotFound", reference });

    render(
      <ObjectViewer
        tabId="tab-object"
        connectionId="conn-1"
        reference={reference}
      />,
    );

    expect(await screen.findByText("SELECT 1")).toBeTruthy();
    act(() => window.dispatchEvent(new FocusEvent("focus")));

    expect(await screen.findByText(/no longer exists/)).toBeTruthy();
    expect(mockedInvoke).toHaveBeenCalledTimes(2);
  });

  it("reloads routine source after an edit is applied", async () => {
    const reference = referenceFor("function");
    const facts = {
      kind: "routine" as const,
      language: "sql",
      returns: "integer",
      volatility: "stable",
      arguments: "value integer",
      body: "SELECT value + 1",
      strict: true,
      securityDefiner: false,
      parallel: "safe",
    };
    const initialSql = "CREATE FUNCTION lifecycle.sample_function(integer)";
    const updatedSql =
      "CREATE OR REPLACE FUNCTION lifecycle.sample_function(integer)";
    mockedInvoke
      .mockResolvedValueOnce(descriptionFor(reference, facts, initialSql))
      .mockResolvedValueOnce({
        statements: [
          {
            sql: updatedSql,
            summary: "Replace function lifecycle.sample_function",
            destructive: true,
            transactional: true,
          },
        ],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      })
      .mockResolvedValueOnce({ appliedStatements: 1, runtimeMs: 5 })
      .mockResolvedValueOnce({
        schemas: [],
        eventTriggers: [],
        roles: [],
        tablespaces: [],
        truncated: [],
      })
      .mockResolvedValueOnce(descriptionFor(reference, facts, updatedSql));

    render(
      <ObjectViewer
        tabId="tab-function"
        connectionId="conn-1"
        reference={reference}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Edit source" }));
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));

    expect(await screen.findByText(updatedSql)).toBeTruthy();
    expect(mockedInvoke).toHaveBeenCalledWith("load_pg_object_catalog", {
      payload: { connectionId: "conn-1" },
    });
    const describeCalls = mockedInvoke.mock.calls.filter(
      ([command]) => command === "describe_pg_object",
    );
    expect(describeCalls).toHaveLength(2);
  });

  it("shows routine not-found after the post-apply reload", async () => {
    const reference = referenceFor("function");
    const facts = {
      kind: "routine" as const,
      language: "sql",
      returns: "integer",
      volatility: "stable",
      arguments: "value integer",
      body: "SELECT value + 1",
      strict: true,
      securityDefiner: false,
      parallel: "safe",
    };
    mockedInvoke
      .mockResolvedValueOnce(descriptionFor(reference, facts))
      .mockResolvedValueOnce({
        statements: [
          {
            sql: "CREATE OR REPLACE FUNCTION lifecycle.sample_function(integer)",
            summary: "Replace function lifecycle.sample_function",
            destructive: true,
            transactional: true,
          },
        ],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      })
      .mockResolvedValueOnce({ appliedStatements: 1, runtimeMs: 5 })
      .mockResolvedValueOnce({
        schemas: [],
        eventTriggers: [],
        roles: [],
        tablespaces: [],
        truncated: [],
      })
      .mockRejectedValueOnce({ kind: "objectNotFound", reference });

    render(
      <ObjectViewer
        tabId="tab-function"
        connectionId="conn-1"
        reference={reference}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Edit source" }));
    fireEvent.change(screen.getByLabelText("Routine arguments"), {
      target: { value: "value bigint" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));

    expect(await screen.findByText(/no longer exists/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Close tab" })).toBeTruthy();
    expect(mockedInvoke).toHaveBeenCalledWith("preview_object_ddl", {
      payload: expect.objectContaining({
        ops: [expect.objectContaining({ arguments: "value bigint" })],
      }),
    });
  });

  it("drops lifecycle dialog state when the active object identity changes", async () => {
    const first = referenceFor("view");
    const second = { ...first, name: "other_view" };
    useAppStore.setState({
      connections: [
        connection("Connected"),
        {
          ...connection("Connected"),
          id: "conn-2",
          name: "warehouse_reporting",
        },
      ],
    });
    mockedInvoke
      .mockResolvedValueOnce(
        descriptionFor(first, { kind: "view", definition: "SELECT 1" }),
      )
      .mockResolvedValueOnce({
        statements: [
          {
            sql: "COMMENT ON VIEW lifecycle.sample_view IS 'first'",
            summary: "Comment on lifecycle.sample_view",
            destructive: false,
            transactional: true,
          },
        ],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      })
      .mockResolvedValueOnce(
        descriptionFor(
          second,
          { kind: "view", definition: "SELECT 2" },
          "SELECT 2",
        ),
      );

    const { rerender } = render(
      <ObjectViewer
        tabId="tab-first"
        connectionId="conn-1"
        reference={first}
      />,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "Edit comment" }),
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Object comment" }), {
      target: { value: "first" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    expect(
      await screen.findByRole("button", { name: "Apply DDL" }),
    ).toBeTruthy();

    rerender(
      <ObjectViewer
        tabId="tab-second"
        connectionId="conn-2"
        reference={second}
      />,
    );

    expect(await screen.findByText("SELECT 2")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Apply DDL" })).toBeNull();
    expect(screen.queryByLabelText("DDL review")).toBeNull();
  });

  it("copies a definition and hands it to a SQL editor tab", async () => {
    const reference = referenceFor("function");
    const sql =
      "CREATE OR REPLACE FUNCTION lifecycle.sample_function(integer) RETURNS integer LANGUAGE sql AS 'SELECT $1';";
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    mockedInvoke.mockResolvedValueOnce(
      descriptionFor(
        reference,
        {
          kind: "routine",
          language: "sql",
          returns: "integer",
          volatility: "immutable",
          arguments: "value integer",
          body: null,
          strict: false,
          securityDefiner: false,
          parallel: null,
        },
        sql,
      ),
    );

    render(
      <ObjectViewer
        tabId="tab-object"
        connectionId="conn-1"
        reference={reference}
      />,
    );

    expect(await screen.findByText(sql)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Edit source" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(sql));
    fireEvent.click(screen.getByRole("button", { name: "Open in SQL editor" }));

    expect(useAppStore.getState().workspaceTabs.at(-1)).toEqual(
      expect.objectContaining({
        kind: "query",
        label: "lifecycle.sample_function(integer)_function_definition.sql",
        connectionId: "conn-1",
        schema: "lifecycle",
        query: sql,
      }),
    );
  });

  it("uses overload-safe definition tab labels", () => {
    expect(
      objectDefinitionTabLabel({
        kind: "function",
        schema: "lifecycle",
        name: "add_nums",
        identityArgs: "integer, integer",
      }),
    ).not.toBe(
      objectDefinitionTabLabel({
        kind: "function",
        schema: "lifecycle",
        name: "add_nums",
        identityArgs: "numeric, numeric",
      }),
    );
  });

  it("uses honest copy when aggregate definition rendering is unsupported", async () => {
    const reference = referenceFor("aggregate");
    mockedInvoke.mockResolvedValueOnce(
      descriptionFor(
        reference,
        {
          kind: "routine",
          language: "internal",
          returns: "numeric",
          volatility: null,
          arguments: "numeric",
          body: null,
          strict: false,
          securityDefiner: false,
          parallel: null,
        },
        null,
      ),
    );

    render(
      <ObjectViewer
        tabId="tab-object"
        connectionId="conn-1"
        reference={reference}
      />,
    );

    expect(
      await screen.findByText(
        "Definition rendering is not supported for aggregates.",
      ),
    ).toBeTruthy();
  });
});

describe("ObjectViewer actions", () => {
  it.each([
    {
      kind: "type" as const,
      facts: {
        kind: "type" as const,
        class: "enum" as const,
        enumLabels: ["new", "paid"],
        attributes: null,
        subtype: null,
      },
    },
    {
      kind: "domain" as const,
      facts: {
        kind: "domain" as const,
        baseType: "numeric",
        notNull: false,
        defaultValue: null,
        checks: [],
      },
    },
    {
      kind: "function" as const,
      facts: {
        kind: "routine" as const,
        language: "sql",
        returns: "integer",
        volatility: "immutable",
        arguments: "value integer",
        body: null,
        strict: false,
        securityDefiner: false,
        parallel: null,
      },
    },
    {
      kind: "extension" as const,
      facts: {
        kind: "extension" as const,
        version: "1.0",
        schema: "public",
      },
    },
  ])("does not offer Rename for $kind viewers", async ({ kind, facts }) => {
    const reference = referenceFor(kind);
    mockedInvoke.mockResolvedValueOnce(descriptionFor(reference, facts));

    render(
      <ObjectViewer
        tabId={`tab-${kind}`}
        connectionId="conn-1"
        reference={reference}
      />,
    );

    expect(await screen.findByText("Lifecycle fixture")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Rename" })).toBeNull();
  });

  it("keeps rename capability limited to backend-supported kinds", () => {
    const renameable: PgObjectKind[] = [
      "schema",
      "table",
      "view",
      "materialized-view",
      "sequence",
    ];
    const notRenameable: PgObjectKind[] = [
      "foreign-table",
      "function",
      "procedure",
      "aggregate",
      "type",
      "domain",
      "extension",
    ];
    expect(renameable.every(canRenameObjectKind)).toBe(true);
    expect(notRenameable.some(canRenameObjectKind)).toBe(false);
  });

  it("does not offer unsupported extension drops", async () => {
    const reference = referenceFor("extension");
    mockedInvoke.mockResolvedValueOnce(
      descriptionFor(reference, {
        kind: "extension",
        version: "1.0",
        schema: "public",
      }),
    );

    render(
      <ObjectViewer
        tabId="tab-extension"
        connectionId="conn-1"
        reference={reference}
      />,
    );

    expect(await screen.findByText("Lifecycle fixture")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Drop" })).toBeNull();
  });

  it("retargets and re-describes an object tab after rename", async () => {
    const reference = referenceFor("view");
    const renamedReference = { ...reference, name: "renamed_view" };
    const initialDescription = descriptionFor(reference, {
      kind: "view",
      definition: "SELECT 1",
    });
    const renamedDescription = descriptionFor(renamedReference, {
      kind: "view",
      definition: "SELECT 1",
    });
    mockedInvoke
      .mockResolvedValueOnce(initialDescription)
      .mockResolvedValueOnce({
        statements: [
          {
            sql: "ALTER VIEW lifecycle.sample_view RENAME TO renamed_view",
            summary: "Rename lifecycle.sample_view to renamed_view",
            destructive: false,
            transactional: true,
          },
        ],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      })
      .mockResolvedValueOnce({ appliedStatements: 1, runtimeMs: 5 })
      .mockResolvedValueOnce({
        schemas: [],
        eventTriggers: [],
        roles: [],
        tablespaces: [],
        truncated: [],
      })
      .mockRejectedValueOnce({ kind: "objectNotFound", reference })
      .mockResolvedValueOnce(renamedDescription);
    useAppStore.setState({
      workspaceTabs: [
        {
          id: "tab-view",
          kind: "object",
          label: "lifecycle.sample_view",
          connectionId: "conn-1",
          schema: "lifecycle",
          objectRef: reference,
        },
      ],
      activeTabId: "tab-view",
    });

    render(
      <ObjectViewer
        tabId="tab-view"
        connectionId="conn-1"
        reference={reference}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Rename" }));
    fireEvent.change(screen.getByRole("textbox", { name: "New object name" }), {
      target: { value: "renamed_view" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));

    await waitFor(() => {
      const tab = useAppStore
        .getState()
        .workspaceTabs.find((candidate) => candidate.id === "tab-view");
      expect(tab).toEqual(
        expect.objectContaining({
          label: "lifecycle.renamed_view",
          objectRef: renamedReference,
        }),
      );
    });
    expect(mockedInvoke).toHaveBeenCalledWith("describe_pg_object", {
      payload: { connectionId: "conn-1", reference: renamedReference },
    });
  });

  it("retargets descendant object tabs and caches after a schema rename", async () => {
    const reference = referenceFor("schema");
    const renamedReference = { ...reference, name: "archive" };
    const childReference = referenceFor("view");
    const renamedChildReference = { ...childReference, schema: "archive" };
    const childDescription = descriptionFor(childReference, {
      kind: "view",
      definition: "SELECT 1",
    });
    const childKey = pgObjectDescriptionKey("conn-1", childReference);
    mockedInvoke
      .mockResolvedValueOnce(descriptionFor(reference, { kind: "schema" }))
      .mockResolvedValueOnce({
        statements: [
          {
            sql: "ALTER SCHEMA lifecycle RENAME TO archive",
            summary: "Rename lifecycle to archive",
            destructive: false,
            transactional: true,
          },
        ],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      })
      .mockResolvedValueOnce({ appliedStatements: 1, runtimeMs: 5 })
      .mockResolvedValueOnce({
        schemas: [],
        eventTriggers: [],
        roles: [],
        tablespaces: [],
        truncated: [],
      })
      .mockRejectedValueOnce({ kind: "objectNotFound", reference })
      .mockResolvedValueOnce(
        descriptionFor(renamedReference, { kind: "schema" }),
      );
    useAppStore.setState({
      workspaceTabs: [
        {
          id: "tab-schema",
          kind: "object",
          label: "lifecycle",
          connectionId: "conn-1",
          schema: "",
          objectRef: reference,
        },
        {
          id: "tab-view",
          kind: "object",
          label: "lifecycle.sample_view",
          connectionId: "conn-1",
          schema: "lifecycle",
          objectRef: childReference,
        },
      ],
      activeTabId: "tab-schema",
      pgObjectDescriptions: {
        [childKey]: {
          status: "ready",
          generation: 0,
          description: childDescription,
        },
      },
    });

    render(
      <ObjectViewer
        tabId="tab-schema"
        connectionId="conn-1"
        reference={reference}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Rename" }));
    fireEvent.change(screen.getByRole("textbox", { name: "New object name" }), {
      target: { value: "archive" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));

    await waitFor(() => {
      const tabs = useAppStore.getState().workspaceTabs;
      expect(tabs[0]).toEqual(
        expect.objectContaining({
          label: "archive",
          schema: "",
          objectRef: renamedReference,
        }),
      );
      expect(tabs[1]).toEqual(
        expect.objectContaining({
          label: "archive.sample_view",
          schema: "archive",
          objectRef: renamedChildReference,
        }),
      );
    });
    expect(
      useAppStore.getState().pgObjectDescriptions[childKey],
    ).toBeUndefined();
    expect(mockedInvoke).toHaveBeenCalledWith("describe_pg_object", {
      payload: { connectionId: "conn-1", reference: renamedReference },
    });
  });

  it("shows the terminal missing state after dropping a schema", async () => {
    const reference = referenceFor("schema");
    const childReference = referenceFor("view");
    const childKey = pgObjectDescriptionKey("conn-1", childReference);
    mockedInvoke
      .mockResolvedValueOnce(descriptionFor(reference, { kind: "schema" }))
      .mockResolvedValueOnce({ dependents: [], truncated: false })
      .mockResolvedValueOnce({
        statements: [
          {
            sql: "DROP SCHEMA lifecycle",
            summary: "Drop schema lifecycle",
            destructive: true,
            transactional: true,
          },
        ],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      })
      .mockResolvedValueOnce({ appliedStatements: 1, runtimeMs: 5 })
      .mockResolvedValueOnce({
        schemas: [],
        eventTriggers: [],
        roles: [],
        tablespaces: [],
        truncated: [],
      })
      .mockRejectedValueOnce({ kind: "objectNotFound", reference })
      .mockRejectedValueOnce({ kind: "objectNotFound", reference });
    useAppStore.setState({
      pgObjectDescriptions: {
        [childKey]: {
          status: "ready",
          generation: 0,
          description: descriptionFor(childReference, {
            kind: "view",
            definition: "SELECT 1",
          }),
        },
      },
    });

    render(
      <ObjectViewer
        tabId="tab-schema"
        connectionId="conn-1"
        reference={reference}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Drop" }));
    expect(await screen.findByText("No dependents found.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Review drop DDL" }));
    fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));

    expect(await screen.findByText(/lifecycle no longer exists/)).toBeTruthy();
    expect(screen.queryByText("Loading object…")).toBeNull();
    expect(
      useAppStore.getState().pgObjectDescriptions[childKey],
    ).toBeUndefined();
    const describeCalls = mockedInvoke.mock.calls.filter(
      ([command]) => command === "describe_pg_object",
    );
    expect(describeCalls).toHaveLength(3);
  });

  it("keeps a terminal DDL result mounted through same-object refresh", async () => {
    const reference = referenceFor("sequence");
    const description = descriptionFor(reference, {
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
    });
    let resolveDescription: (value: PgObjectDescription) => void = () =>
      undefined;
    mockedInvoke
      .mockResolvedValueOnce(description)
      .mockResolvedValueOnce({
        statements: [
          {
            sql: "COMMENT ON SEQUENCE lifecycle.sample_sequence IS 'Updated'",
            summary: "Comment on lifecycle.sample_sequence",
            destructive: false,
            transactional: true,
          },
        ],
        groups: [{ kind: "atomic", statementIndexes: [0] }],
      })
      .mockResolvedValueOnce({ appliedStatements: 1, runtimeMs: 5 })
      .mockResolvedValueOnce({
        schemas: [],
        eventTriggers: [],
        roles: [],
        tablespaces: [],
        truncated: [],
      })
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveDescription = resolve;
          }),
      );

    render(
      <ObjectViewer
        tabId="tab-sequence"
        connectionId="conn-1"
        reference={reference}
      />,
    );
    fireEvent.click(
      await screen.findByRole("button", { name: "Edit comment" }),
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Object comment" }), {
      target: { value: "Updated" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    fireEvent.click(await screen.findByRole("button", { name: "Apply DDL" }));

    expect(
      await screen.findByRole("button", { name: "Applying…" }),
    ).toBeTruthy();
    await act(async () => {
      resolveDescription({ ...description, comment: "Updated" });
    });
    expect(
      (await screen.findByRole("button", { name: "Applied" })).hasAttribute(
        "disabled",
      ),
    ).toBe(true);
  });

  it("routes materialized-view refresh through the legacy safety path", async () => {
    const reference = referenceFor("materialized-view");
    const description = descriptionFor(reference, {
      kind: "materializedView",
      definition: "SELECT count(*) FROM lifecycle.orders",
      populated: true,
    });
    mockedInvoke
      .mockResolvedValueOnce(description)
      .mockResolvedValueOnce(description);

    render(
      <ObjectViewer
        tabId="tab-matview"
        connectionId="conn-1"
        reference={reference}
      />,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(mockedSafetyInvoke).toHaveBeenCalledOnce());
    expect(mockedSafetyInvoke).toHaveBeenCalledWith(
      expect.objectContaining({
        command: "refresh_materialized_view",
        payload: {
          connectionId: "conn-1",
          schema: "lifecycle",
          view: "sample_materialized-view",
          concurrently: false,
        },
        connection: connection("Connected"),
        isConnectionCurrent: expect.any(Function),
      }),
    );
    expect(await screen.findByText("Refreshed")).toBeTruthy();
  });

  it("does not describe a materialized view in a newer generation after refresh", async () => {
    const reference = referenceFor("materialized-view");
    let resolveRefresh: (value: { runtimeMs: number }) => void = () =>
      undefined;
    mockedInvoke.mockResolvedValueOnce(
      descriptionFor(reference, {
        kind: "materializedView",
        definition: "SELECT 1",
        populated: true,
      }),
    );
    mockedSafetyInvoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveRefresh = resolve;
        }),
    );

    render(
      <ObjectViewer
        tabId="tab-matview"
        connectionId="conn-1"
        reference={reference}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Refresh" }));
    act(() => {
      useAppStore.setState({
        pgObjectCatalog: {
          "conn-1": { status: "idle", generation: 1 },
        },
      });
    });
    await act(async () => {
      resolveRefresh({ runtimeMs: 4 });
      await Promise.resolve();
      await Promise.resolve();
    });

    const describeCalls = mockedInvoke.mock.calls.filter(
      ([command]) => command === "describe_pg_object",
    );
    expect(describeCalls).toHaveLength(1);
    expect(screen.getByText("Refresh failed")).toBeTruthy();
  });

  it("reviews materialized-view edits as drop then create after impact", async () => {
    const reference = referenceFor("materialized-view");
    const sqlBody = "SELECT count(*) FROM lifecycle.orders";
    mockedInvoke
      .mockResolvedValueOnce(
        descriptionFor(reference, {
          kind: "materializedView",
          definition: sqlBody,
          populated: false,
        }),
      )
      .mockResolvedValueOnce({ dependents: [], truncated: false })
      .mockResolvedValueOnce({
        statements: [
          {
            sql: "DROP MATERIALIZED VIEW lifecycle.sample_materialized_view",
            summary: "Drop materialized view",
            destructive: true,
            transactional: true,
          },
          {
            sql: `CREATE MATERIALIZED VIEW lifecycle.sample_materialized_view AS ${sqlBody}`,
            summary: "Create materialized view",
            destructive: false,
            transactional: true,
          },
        ],
        groups: [{ kind: "atomic", statementIndexes: [0, 1] }],
      });

    render(
      <ObjectViewer
        tabId="tab-matview"
        connectionId="conn-1"
        reference={reference}
      />,
    );

    fireEvent.click(
      await screen.findByRole("button", { name: "Edit definition" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Review DDL" }));
    expect(await screen.findByText("No dependents found.")).toBeTruthy();
    expect(
      screen.getByText(
        "Recreating this materialized view drops its indexes and grants.",
      ),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Review drop DDL" }));

    const expectedOps = [
      { op: "dropObject", reference, cascade: false },
      {
        op: "createMaterializedView",
        schema: "lifecycle",
        name: "sample_materialized-view",
        sqlBody,
        withData: false,
      },
    ];
    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith("preview_object_ddl", {
        payload: { connectionId: "conn-1", ops: expectedOps },
      }),
    );
  });
});
