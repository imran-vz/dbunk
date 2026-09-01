import {
  IconBraces,
  IconCopy,
  IconDatabaseCog,
  IconKey,
  IconMapPin,
  IconShieldLock,
  IconSitemap,
  IconSparkles,
  IconTerminal2,
  IconTopologyStar,
} from "@tabler/icons-react";
import type * as React from "react";
import { useMemo, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { pgQuoteIdent } from "@/lib/ddl/postgres";
import {
  type ColumnInfo,
  type PgObjectOp,
  type TableStructure,
  tableStructureKey,
  useAppStore,
} from "@/lib/store";
import {
  asPgReferentialAction,
  buildAddForeignKeyOp,
  buildCreateIndexOp,
  PG_REFERENTIAL_ACTIONS,
} from "@/lib/structure-changes";
import { errorToMessage } from "@/lib/tauri";
import { cn } from "@/lib/utils";

type SpecializedEditorsProps = {
  schema: string;
  table: string;
  connectionId: string;
  structure: TableStructure | undefined;
};

const PRIVILEGES = [
  "SELECT",
  "INSERT",
  "UPDATE",
  "DELETE",
  "TRUNCATE",
] as const;
const RLS_COMMANDS = ["ALL", "SELECT", "INSERT", "UPDATE", "DELETE"] as const;
const INDEX_METHODS = ["btree", "hash", "gin", "gist", "brin"] as const;
const TRIGGER_TIMINGS = ["BEFORE", "AFTER", "INSTEAD OF"] as const;
const TRIGGER_EVENTS = ["INSERT", "UPDATE", "DELETE", "TRUNCATE"] as const;
const REFERENTIAL_ACTIONS = PG_REFERENTIAL_ACTIONS.map(
  (option) => option.label,
);

export function SpecializedEditors({
  schema,
  table,
  connectionId,
  structure,
}: SpecializedEditorsProps) {
  const openWorkspaceTab = useAppStore((state) => state.openWorkspaceTab);
  const addPendingStructureChange = useAppStore(
    (state) => state.addPendingStructureChange,
  );
  const engine = useAppStore(
    (state) =>
      state.connections.find((candidate) => candidate.id === connectionId)
        ?.engine,
  );
  // Only PostgreSQL has the typed object-DDL workflow (Plan 015). Every
  // other engine keeps this tab's original generate-SQL behaviour.
  const typedOpsAvailable = engine === "PostgreSQL";
  const columns = useMemo(() => structure?.columns ?? [], [structure]);
  const columnNames = useMemo(
    () => columns.map((column) => column.name),
    [columns],
  );
  const qualifiedTable = `${pgQuoteIdent(schema)}.${pgQuoteIdent(table)}`;
  const structureKey = tableStructureKey(connectionId, schema, table);

  const [generatedSql, setGeneratedSql] = useState("");
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">(
    "idle",
  );
  const [queueNotice, setQueueNotice] = useState<{
    panel: "index" | "fk";
    message: string;
    failed: boolean;
  } | null>(null);

  // The index and cross-table FK panels queue typed operations into the
  // shared structure-editor pending list (Plan 015); queueing fails when
  // that list still holds legacy column entries for this table.
  const queueTypedOp = (panel: "index" | "fk", op: PgObjectOp) => {
    try {
      addPendingStructureChange(structureKey, {
        schema,
        table,
        change: { kind: "pg-op", op },
      });
      setQueueNotice({
        panel,
        message:
          "Queued. Review and commit in the Structure tab's pending changes.",
        failed: false,
      });
    } catch (error) {
      setQueueNotice({ panel, message: errorToMessage(error), failed: true });
    }
  };

  const [grantState, setGrantState] = useState({
    objectType: "TABLE",
    role: "",
    withGrantOption: false,
    privileges: new Set<string>(["SELECT"]),
  });

  const [rlsState, setRlsState] = useState({
    enabled: true,
    force: false,
    policy: `${table}_access_policy`,
    command: "ALL",
    role: "PUBLIC",
    usingExpression: "true",
    checkExpression: "",
  });

  const [indexState, setIndexState] = useState({
    name: `${table}_${columnNames[0] ?? "column"}_idx`,
    method: "btree",
    columns: new Set<string>(columnNames.slice(0, 1)),
    unique: false,
    concurrently: true,
    predicate: "",
    includeColumns: new Set<string>(),
  });

  const [fkState, setFkState] = useState({
    name: `${table}_${columnNames[0] ?? "column"}_fk`,
    columns: new Set<string>(columnNames.slice(0, 1)),
    referencedSchema: schema,
    referencedTable: "referenced_table",
    referencedColumns: "id",
    onUpdate: "NO ACTION",
    onDelete: "NO ACTION",
    deferrable: false,
  });

  const [triggerState, setTriggerState] = useState({
    name: `${table}_updated_at`,
    timing: "BEFORE",
    events: new Set<string>(["UPDATE"]),
    orientation: "ROW",
    functionSchema: schema,
    functionName: `${table}_set_updated_at`,
    body: "NEW.updated_at = now();\nRETURN NEW;",
  });

  const [jsonText, setJsonText] = useState('{"example": true}');
  const [arrayText, setArrayText] = useState("alpha\nbeta\ngamma");
  const [geometryText, setGeometryText] = useState(
    "POLYGON((0 0, 80 0, 80 40, 0 40, 0 0))",
  );

  const jsonResult = useMemo(() => parseJson(jsonText), [jsonText]);
  const arrayLiteral = useMemo(() => toPgArrayLiteral(arrayText), [arrayText]);
  const geometry = useMemo(() => parseWkt(geometryText), [geometryText]);

  const copySql = async () => {
    if (!generatedSql) return;
    try {
      await navigator.clipboard.writeText(generatedSql);
      setCopyState("copied");
      window.setTimeout(() => setCopyState("idle"), 1500);
    } catch {
      setCopyState("failed");
    }
  };

  const openInSqlEditor = () => {
    if (!generatedSql) return;
    openWorkspaceTab({
      kind: "query",
      label: `${table}_specialized.sql`,
      connectionId,
      schema,
      query: generatedSql,
    });
  };

  const generateGrant = () => {
    const privileges = Array.from(grantState.privileges);
    setGeneratedSql(
      `GRANT ${privileges.length > 0 ? privileges.join(", ") : "SELECT"} ON ${grantState.objectType} ${qualifiedTable} TO ${quoteIdentOrPublic(grantState.role)}${grantState.withGrantOption ? " WITH GRANT OPTION" : ""};`,
    );
  };

  const generateRls = () => {
    const lines = [
      rlsState.enabled
        ? `ALTER TABLE ${qualifiedTable} ENABLE ROW LEVEL SECURITY;`
        : `ALTER TABLE ${qualifiedTable} DISABLE ROW LEVEL SECURITY;`,
    ];
    if (rlsState.force) {
      lines.push(`ALTER TABLE ${qualifiedTable} FORCE ROW LEVEL SECURITY;`);
    }
    const policySql = [
      `CREATE POLICY ${pgQuoteIdent(rlsState.policy || `${table}_policy`)} ON ${qualifiedTable}`,
      `  AS PERMISSIVE FOR ${rlsState.command}`,
      `  TO ${quoteIdentOrPublic(rlsState.role)}`,
      `  USING (${rlsState.usingExpression || "true"})`,
      rlsState.checkExpression
        ? `  WITH CHECK (${rlsState.checkExpression})`
        : "",
    ]
      .filter(Boolean)
      .join("\n");
    lines.push(`${policySql};`);
    setGeneratedSql(lines.join("\n"));
  };

  const indexFormColumns = () => {
    const selectedColumns = Array.from(indexState.columns);
    return selectedColumns.length > 0
      ? selectedColumns
      : columnNames.slice(0, 1);
  };

  const fkFormColumns = () => {
    const selectedColumns = Array.from(fkState.columns);
    return selectedColumns.length > 0
      ? selectedColumns
      : columnNames.slice(0, 1);
  };

  const queueIndex = () => {
    queueTypedOp(
      "index",
      buildCreateIndexOp({
        schema,
        table,
        name: indexState.name,
        unique: indexState.unique,
        method: indexState.method,
        columnExpressions: indexFormColumns(),
        include: Array.from(indexState.includeColumns),
        wherePredicate: indexState.predicate,
        concurrently: indexState.concurrently,
      }),
    );
  };

  const queueForeignKey = () => {
    queueTypedOp(
      "fk",
      buildAddForeignKeyOp({
        schema,
        table,
        name: fkState.name,
        columns: fkFormColumns(),
        referencedSchema: fkState.referencedSchema,
        referencedTable: fkState.referencedTable.trim() || "referenced_table",
        referencedColumns: splitIdentifierList(fkState.referencedColumns),
        onUpdate: asPgReferentialAction(fkState.onUpdate),
        onDelete: asPgReferentialAction(fkState.onDelete),
        deferrable: fkState.deferrable,
      }),
    );
  };

  // Pre-Plan-015 generate-only fallback for engines without the typed
  // object-DDL workflow.
  const generateIndex = () => {
    const indexColumns = indexFormColumns();
    const includeColumns = Array.from(indexState.includeColumns);
    const lines = [
      `CREATE ${indexState.unique ? "UNIQUE " : ""}INDEX ${indexState.concurrently ? "CONCURRENTLY " : ""}${pgQuoteIdent(indexState.name || `${table}_idx`)}`,
      `ON ${qualifiedTable} USING ${indexState.method} (${indexColumns.map(pgQuoteIdent).join(", ")})`,
      includeColumns.length > 0
        ? `INCLUDE (${includeColumns.map(pgQuoteIdent).join(", ")})`
        : "",
      indexState.predicate ? `WHERE ${indexState.predicate}` : "",
    ].filter(Boolean);
    setGeneratedSql(`${lines.join("\n")};`);
  };

  const generateForeignKey = () => {
    const fkColumns = fkFormColumns();
    const references = splitIdentifierList(fkState.referencedColumns);
    const foreignKeySql = [
      `ALTER TABLE ${qualifiedTable}`,
      `  ADD CONSTRAINT ${pgQuoteIdent(fkState.name || `${table}_fk`)}`,
      `  FOREIGN KEY (${fkColumns.map(pgQuoteIdent).join(", ")})`,
      `  REFERENCES ${pgQuoteIdent(fkState.referencedSchema || schema)}.${pgQuoteIdent(fkState.referencedTable || "referenced_table")} (${references.map(pgQuoteIdent).join(", ")})`,
      `  ON UPDATE ${fkState.onUpdate}`,
      `  ON DELETE ${fkState.onDelete}`,
      fkState.deferrable ? "  DEFERRABLE INITIALLY DEFERRED" : "",
    ]
      .filter(Boolean)
      .join("\n");
    setGeneratedSql(`${foreignKeySql};`);
  };

  const generateTrigger = () => {
    const events = Array.from(triggerState.events);
    const selectedEvents = events.length > 0 ? events : ["UPDATE"];
    const triggerFn = `${pgQuoteIdent(triggerState.functionSchema || schema)}.${pgQuoteIdent(triggerState.functionName || `${table}_trigger_fn`)}`;
    setGeneratedSql(
      [
        `CREATE OR REPLACE FUNCTION ${triggerFn}()`,
        "RETURNS trigger AS $$",
        "BEGIN",
        indent(triggerState.body || "RETURN NEW;", "  "),
        "END;",
        "$$ LANGUAGE plpgsql;",
        "",
        `CREATE TRIGGER ${pgQuoteIdent(triggerState.name || `${table}_trigger`)}`,
        `${triggerState.timing} ${selectedEvents.join(" OR ")} ON ${qualifiedTable}`,
        `FOR EACH ${triggerState.orientation}`,
        `EXECUTE FUNCTION ${triggerFn}();`,
      ].join("\n"),
    );
  };

  return (
    <div className="h-full overflow-auto bg-surface-canvas">
      <div className="mx-auto flex max-w-7xl flex-col gap-4 p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h2 className="text-sm font-semibold text-foreground">
              Specialized PostgreSQL editors
            </h2>
            <p className="mt-0.5 text-xs text-text-muted">
              {qualifiedTable} · {columns.length} columns
            </p>
          </div>
          <div className="flex flex-wrap gap-1.5">
            {columns.slice(0, 6).map((column) => (
              <Badge key={column.name} variant="outline">
                {column.name}:{column.dataType}
              </Badge>
            ))}
          </div>
        </div>

        <div className="grid gap-4 xl:grid-cols-2">
          <EditorPanel
            title="Permissions / GRANT"
            icon={<IconShieldLock />}
            action="Generate GRANT"
            onGenerate={generateGrant}
          >
            <FormGrid>
              <Field label="Object type">
                <NativeSelect
                  value={grantState.objectType}
                  onChange={(objectType) =>
                    setGrantState((current) => ({ ...current, objectType }))
                  }
                  options={["TABLE", "SEQUENCE"]}
                />
              </Field>
              <Field label="Role">
                <Input
                  value={grantState.role}
                  onChange={(event) =>
                    setGrantState((current) => ({
                      ...current,
                      role: event.target.value,
                    }))
                  }
                  placeholder="app_readonly"
                />
              </Field>
            </FormGrid>
            <CheckGrid
              values={PRIVILEGES}
              selected={grantState.privileges}
              onToggle={(privilege) =>
                setGrantState((current) => ({
                  ...current,
                  privileges: toggleSet(current.privileges, privilege),
                }))
              }
            />
            <SwitchRow
              label="WITH GRANT OPTION"
              checked={grantState.withGrantOption}
              onCheckedChange={(withGrantOption) =>
                setGrantState((current) => ({
                  ...current,
                  withGrantOption,
                }))
              }
            />
          </EditorPanel>

          <EditorPanel
            title="Row-Level Security"
            icon={<IconKey />}
            action="Generate policy"
            onGenerate={generateRls}
          >
            <FormGrid>
              <Field label="Policy name">
                <Input
                  value={rlsState.policy}
                  onChange={(event) =>
                    setRlsState((current) => ({
                      ...current,
                      policy: event.target.value,
                    }))
                  }
                />
              </Field>
              <Field label="Command">
                <NativeSelect
                  value={rlsState.command}
                  onChange={(command) =>
                    setRlsState((current) => ({ ...current, command }))
                  }
                  options={RLS_COMMANDS}
                />
              </Field>
              <Field label="Role">
                <Input
                  value={rlsState.role}
                  onChange={(event) =>
                    setRlsState((current) => ({
                      ...current,
                      role: event.target.value,
                    }))
                  }
                />
              </Field>
            </FormGrid>
            <SwitchRow
              label="Enable RLS"
              checked={rlsState.enabled}
              onCheckedChange={(enabled) =>
                setRlsState((current) => ({ ...current, enabled }))
              }
            />
            <SwitchRow
              label="FORCE RLS"
              checked={rlsState.force}
              onCheckedChange={(force) =>
                setRlsState((current) => ({ ...current, force }))
              }
            />
            <Field label="USING expression">
              <Textarea
                value={rlsState.usingExpression}
                onChange={(event) =>
                  setRlsState((current) => ({
                    ...current,
                    usingExpression: event.target.value,
                  }))
                }
              />
            </Field>
            <Field label="WITH CHECK expression">
              <Textarea
                value={rlsState.checkExpression}
                onChange={(event) =>
                  setRlsState((current) => ({
                    ...current,
                    checkExpression: event.target.value,
                  }))
                }
                placeholder="Optional"
              />
            </Field>
          </EditorPanel>

          <EditorPanel
            title="Index creation"
            icon={<IconDatabaseCog />}
            action={typedOpsAvailable ? "Queue index" : "Generate index"}
            onGenerate={typedOpsAvailable ? queueIndex : generateIndex}
          >
            {queueNotice?.panel === "index" ? (
              <QueueNotice notice={queueNotice} />
            ) : null}
            <FormGrid>
              <Field label="Index name">
                <Input
                  value={indexState.name}
                  onChange={(event) =>
                    setIndexState((current) => ({
                      ...current,
                      name: event.target.value,
                    }))
                  }
                />
              </Field>
              <Field label="Method">
                <NativeSelect
                  value={indexState.method}
                  onChange={(method) =>
                    setIndexState((current) => ({ ...current, method }))
                  }
                  options={INDEX_METHODS}
                />
              </Field>
            </FormGrid>
            <ColumnChecklist
              title="Indexed columns"
              columns={columns}
              selected={indexState.columns}
              onToggle={(column) =>
                setIndexState((current) => ({
                  ...current,
                  columns: toggleSet(current.columns, column),
                }))
              }
            />
            <ColumnChecklist
              title="Included columns"
              columns={columns}
              selected={indexState.includeColumns}
              onToggle={(column) =>
                setIndexState((current) => ({
                  ...current,
                  includeColumns: toggleSet(current.includeColumns, column),
                }))
              }
            />
            <FormGrid>
              <SwitchRow
                label="UNIQUE"
                checked={indexState.unique}
                onCheckedChange={(unique) =>
                  setIndexState((current) => ({ ...current, unique }))
                }
              />
              <SwitchRow
                label="CONCURRENTLY"
                checked={indexState.concurrently}
                onCheckedChange={(concurrently) =>
                  setIndexState((current) => ({
                    ...current,
                    concurrently,
                  }))
                }
              />
            </FormGrid>
            <Field label="Predicate">
              <Input
                value={indexState.predicate}
                onChange={(event) =>
                  setIndexState((current) => ({
                    ...current,
                    predicate: event.target.value,
                  }))
                }
                placeholder="deleted_at IS NULL"
              />
            </Field>
          </EditorPanel>

          <EditorPanel
            title="Cross-table foreign key"
            icon={<IconSitemap />}
            action={typedOpsAvailable ? "Queue foreign key" : "Generate FK"}
            onGenerate={
              typedOpsAvailable ? queueForeignKey : generateForeignKey
            }
          >
            {queueNotice?.panel === "fk" ? (
              <QueueNotice notice={queueNotice} />
            ) : null}
            <FormGrid>
              <Field label="Constraint name">
                <Input
                  value={fkState.name}
                  onChange={(event) =>
                    setFkState((current) => ({
                      ...current,
                      name: event.target.value,
                    }))
                  }
                />
              </Field>
              <Field label="Referenced schema">
                <Input
                  value={fkState.referencedSchema}
                  onChange={(event) =>
                    setFkState((current) => ({
                      ...current,
                      referencedSchema: event.target.value,
                    }))
                  }
                />
              </Field>
              <Field label="Referenced table">
                <Input
                  value={fkState.referencedTable}
                  onChange={(event) =>
                    setFkState((current) => ({
                      ...current,
                      referencedTable: event.target.value,
                    }))
                  }
                />
              </Field>
              <Field label="Referenced columns">
                <Input
                  value={fkState.referencedColumns}
                  onChange={(event) =>
                    setFkState((current) => ({
                      ...current,
                      referencedColumns: event.target.value,
                    }))
                  }
                  placeholder="id, tenant_id"
                />
              </Field>
              <Field label="ON UPDATE">
                <NativeSelect
                  value={fkState.onUpdate}
                  onChange={(onUpdate) =>
                    setFkState((current) => ({ ...current, onUpdate }))
                  }
                  options={REFERENTIAL_ACTIONS}
                />
              </Field>
              <Field label="ON DELETE">
                <NativeSelect
                  value={fkState.onDelete}
                  onChange={(onDelete) =>
                    setFkState((current) => ({ ...current, onDelete }))
                  }
                  options={REFERENTIAL_ACTIONS}
                />
              </Field>
            </FormGrid>
            <ColumnChecklist
              title="Local columns"
              columns={columns}
              selected={fkState.columns}
              onToggle={(column) =>
                setFkState((current) => ({
                  ...current,
                  columns: toggleSet(current.columns, column),
                }))
              }
            />
            <SwitchRow
              label="DEFERRABLE INITIALLY DEFERRED"
              checked={fkState.deferrable}
              onCheckedChange={(deferrable) =>
                setFkState((current) => ({ ...current, deferrable }))
              }
            />
          </EditorPanel>

          <EditorPanel
            title="Trigger creation"
            icon={<IconTopologyStar />}
            action="Generate trigger"
            onGenerate={generateTrigger}
          >
            <FormGrid>
              <Field label="Trigger name">
                <Input
                  value={triggerState.name}
                  onChange={(event) =>
                    setTriggerState((current) => ({
                      ...current,
                      name: event.target.value,
                    }))
                  }
                />
              </Field>
              <Field label="Timing">
                <NativeSelect
                  value={triggerState.timing}
                  onChange={(timing) =>
                    setTriggerState((current) => ({ ...current, timing }))
                  }
                  options={TRIGGER_TIMINGS}
                />
              </Field>
              <Field label="Function schema">
                <Input
                  value={triggerState.functionSchema}
                  onChange={(event) =>
                    setTriggerState((current) => ({
                      ...current,
                      functionSchema: event.target.value,
                    }))
                  }
                />
              </Field>
              <Field label="Function name">
                <Input
                  value={triggerState.functionName}
                  onChange={(event) =>
                    setTriggerState((current) => ({
                      ...current,
                      functionName: event.target.value,
                    }))
                  }
                />
              </Field>
            </FormGrid>
            <CheckGrid
              values={TRIGGER_EVENTS}
              selected={triggerState.events}
              onToggle={(event) =>
                setTriggerState((current) => ({
                  ...current,
                  events: toggleSet(current.events, event),
                }))
              }
            />
            <Field label="Function body">
              <Textarea
                value={triggerState.body}
                onChange={(event) =>
                  setTriggerState((current) => ({
                    ...current,
                    body: event.target.value,
                  }))
                }
                className="min-h-28 font-mono"
              />
            </Field>
          </EditorPanel>

          <EditorPanel
            title="JSON / jsonb cell editor"
            icon={<IconBraces />}
            action="Copy formatted JSON"
            onGenerate={() =>
              setGeneratedSql(
                jsonResult.ok
                  ? quoteLiteral(JSON.stringify(jsonResult.value))
                  : "-- JSON is invalid; fix it before copying.",
              )
            }
          >
            <Textarea
              value={jsonText}
              onChange={(event) => setJsonText(event.target.value)}
              className="min-h-32 font-mono"
            />
            <div className="flex gap-2">
              <Button
                size="sm"
                variant="outline"
                disabled={!jsonResult.ok}
                onClick={() => {
                  if (jsonResult.ok) {
                    setJsonText(JSON.stringify(jsonResult.value, null, 2));
                  }
                }}
              >
                Format
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={!jsonResult.ok}
                onClick={() => {
                  if (jsonResult.ok) {
                    setJsonText(JSON.stringify(jsonResult.value));
                  }
                }}
              >
                Minify
              </Button>
              <Badge variant={jsonResult.ok ? "success" : "destructive"}>
                {jsonResult.ok ? "valid JSON" : "invalid JSON"}
              </Badge>
            </div>
            <JsonTree result={jsonResult} />
          </EditorPanel>

          <EditorPanel
            title="Array cell editor"
            icon={<IconSparkles />}
            action="Copy array literal"
            onGenerate={() => setGeneratedSql(quoteLiteral(arrayLiteral))}
          >
            <Textarea
              value={arrayText}
              onChange={(event) => setArrayText(event.target.value)}
              className="min-h-28 font-mono"
            />
            <CodeBlock>{arrayLiteral}</CodeBlock>
          </EditorPanel>

          <EditorPanel
            title="PostGIS / geometry visualization"
            icon={<IconMapPin />}
            action="Copy EWKT"
            onGenerate={() =>
              setGeneratedSql(quoteLiteral(geometryText.trim()))
            }
          >
            <Textarea
              value={geometryText}
              onChange={(event) => setGeometryText(event.target.value)}
              className="min-h-24 font-mono"
              placeholder="POINT(10 20), LINESTRING(...), POLYGON((...))"
            />
            <GeometryPreview geometry={geometry} />
          </EditorPanel>
        </div>

        <section className="rounded-sm border border-border-subtle bg-surface-window">
          <div className="flex items-center justify-between gap-2 border-b border-border-subtle px-3 py-2">
            <div>
              <h3 className="text-xs font-semibold text-foreground">
                Generated SQL / literal
              </h3>
              <p className="text-2xs text-text-muted">
                Review before running in the SQL editor.
              </p>
            </div>
            <div className="flex items-center gap-1.5">
              <Button
                size="sm"
                variant="outline"
                disabled={!generatedSql}
                onClick={copySql}
                aria-label="Copy generated SQL"
              >
                <IconCopy />
                {copyState === "copied"
                  ? "Copied"
                  : copyState === "failed"
                    ? "Copy failed"
                    : "Copy"}
              </Button>
              <Button
                size="sm"
                disabled={!generatedSql}
                onClick={openInSqlEditor}
                aria-label="Open generated SQL in editor"
              >
                <IconTerminal2 />
                Open in SQL editor
              </Button>
            </div>
          </div>
          <pre className="max-h-80 overflow-auto whitespace-pre-wrap p-3 font-mono text-xs text-foreground">
            {generatedSql || "-- Generate SQL from one of the editors above."}
          </pre>
        </section>
      </div>
    </div>
  );
}

function EditorPanel({
  title,
  icon,
  action,
  children,
  onGenerate,
}: {
  title: string;
  icon: React.ReactNode;
  action: string;
  children: React.ReactNode;
  onGenerate: () => void;
}) {
  return (
    <section className="flex min-h-0 flex-col gap-3 rounded-sm border border-border-subtle bg-surface-window p-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <span className="flex size-7 shrink-0 items-center justify-center rounded-sm border border-border-subtle bg-surface-panel text-text-secondary [&>svg]:size-3.5">
            {icon}
          </span>
          <h3 className="truncate text-sm font-semibold text-foreground">
            {title}
          </h3>
        </div>
        <Button size="sm" onClick={onGenerate}>
          {action}
        </Button>
      </div>
      {children}
    </section>
  );
}

function QueueNotice({
  notice,
}: {
  notice: { message: string; failed: boolean };
}) {
  return (
    <p
      data-testid={`specialized-queue-notice${notice.failed ? "-failed" : ""}`}
      className={cn(
        "rounded-sm border px-2 py-1.5 text-xs",
        notice.failed
          ? "border-danger/40 bg-danger/10 text-danger"
          : "border-border-subtle bg-surface-panel text-text-secondary",
      )}
    >
      {notice.message}
    </p>
  );
}

function FormGrid({ children }: { children: React.ReactNode }) {
  return <div className="grid gap-2 sm:grid-cols-2">{children}</div>;
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <span className="text-2xs font-medium uppercase tracking-normal text-text-muted">
        {label}
      </span>
      {children}
    </div>
  );
}

function NativeSelect({
  value,
  options,
  onChange,
}: {
  value: string;
  options: readonly string[];
  onChange: (value: string) => void;
}) {
  return (
    <select
      value={value}
      onChange={(event) => onChange(event.target.value)}
      className="h-8 w-full rounded-sm border border-border-subtle bg-surface-input px-2 text-xs text-foreground outline-none transition-colors focus:border-ring"
    >
      {options.map((option) => (
        <option key={option} value={option}>
          {option}
        </option>
      ))}
    </select>
  );
}

function SwitchRow({
  label,
  checked,
  onCheckedChange,
}: {
  label: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-2 rounded-sm border border-border-subtle bg-surface-panel px-2 py-1.5">
      <span className="text-xs text-foreground">{label}</span>
      <Switch checked={checked} onCheckedChange={onCheckedChange} />
    </div>
  );
}

function CheckGrid<T extends string>({
  values,
  selected,
  onToggle,
}: {
  values: readonly T[];
  selected: Set<string>;
  onToggle: (value: T) => void;
}) {
  return (
    <div className="grid gap-1.5 sm:grid-cols-2 lg:grid-cols-3">
      {values.map((value) => (
        <button
          key={value}
          type="button"
          aria-pressed={selected.has(value)}
          onClick={() => onToggle(value)}
          className={cn(
            "h-7 rounded-sm border px-2 text-left text-xs transition-colors",
            selected.has(value)
              ? "border-accent/50 bg-accent/10 text-accent"
              : "border-border-subtle bg-surface-panel text-text-secondary hover:text-foreground",
          )}
        >
          {value}
        </button>
      ))}
    </div>
  );
}

function ColumnChecklist({
  title,
  columns,
  selected,
  onToggle,
}: {
  title: string;
  columns: ColumnInfo[];
  selected: Set<string>;
  onToggle: (column: string) => void;
}) {
  return (
    <div className="rounded-sm border border-border-subtle bg-surface-panel p-2">
      <div className="mb-2 text-2xs font-medium uppercase tracking-normal text-text-muted">
        {title}
      </div>
      <div className="grid max-h-36 gap-1 overflow-auto sm:grid-cols-2">
        {columns.map((column) => (
          <button
            key={column.name}
            type="button"
            aria-pressed={selected.has(column.name)}
            onClick={() => onToggle(column.name)}
            className={cn(
              "flex min-w-0 items-center justify-between gap-2 rounded-sm border px-2 py-1 text-left text-xs transition-colors",
              selected.has(column.name)
                ? "border-accent/50 bg-accent/10 text-accent"
                : "border-border-subtle bg-surface-window text-text-secondary hover:text-foreground",
            )}
          >
            <span className="truncate">{column.name}</span>
            <span className="truncate text-2xs text-text-muted">
              {column.dataType}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

function JsonTree({ result }: { result: JsonParseResult }) {
  if (!result.ok) {
    return (
      <div className="rounded-sm border border-danger/30 bg-danger/10 p-2 font-mono text-xs text-danger">
        {result.message}
      </div>
    );
  }
  return (
    <div className="max-h-48 overflow-auto rounded-sm border border-border-subtle bg-surface-panel p-2 font-mono text-xs">
      <JsonNode value={result.value} depth={0} />
    </div>
  );
}

function JsonNode({ value, depth }: { value: unknown; depth: number }) {
  if (Array.isArray(value)) {
    return (
      <div style={{ paddingLeft: depth * 12 }}>
        <span className="text-text-muted">Array[{value.length}]</span>
        {value.map((item, index) => (
          <div key={`${depth}-${JSON.stringify(item)}`} className="mt-1">
            <span className="text-accent">{index}: </span>
            <JsonNode value={item} depth={depth + 1} />
          </div>
        ))}
      </div>
    );
  }
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
  if (value !== null && typeof value === "object") {
    return (
      <div style={{ paddingLeft: depth * 12 }}>
        {Object.entries(value).map(([key, child]) => (
          <div key={key} className="mt-1">
            <span className="text-accent">{key}: </span>
            <JsonNode value={child} depth={depth + 1} />
          </div>
        ))}
      </div>
    );
  }
  return <span className="text-foreground">{JSON.stringify(value)}</span>;
}

function CodeBlock({ children }: { children: string }) {
  return (
    <pre className="max-h-36 overflow-auto whitespace-pre-wrap rounded-sm border border-border-subtle bg-surface-panel p-2 font-mono text-xs text-foreground">
      {children}
    </pre>
  );
}

/* oxlint-disable jsx-a11y/prefer-tag-over-role -- SVG is the correct semantic element for this generated geometry graphic. */
function GeometryPreview({ geometry }: { geometry: GeometryParseResult }) {
  if (!geometry.ok) {
    return (
      <div className="rounded-sm border border-warning/30 bg-warning/10 p-2 text-xs text-warning">
        {geometry.message}
      </div>
    );
  }
  return (
    <div className="rounded-sm border border-border-subtle bg-surface-panel p-2">
      {/* oxlint-disable-next-line jsx-a11y/prefer-tag-over-role -- SVG is the correct semantic element for this generated geometry graphic. */}
      <svg
        role="img"
        aria-label="Geometry preview"
        viewBox="0 0 320 180"
        className="h-44 w-full rounded-sm bg-surface-window"
      >
        {geometry.kind === "POINT" ? (
          <circle
            cx={geometry.points[0]?.x ?? 160}
            cy={geometry.points[0]?.y ?? 90}
            r="5"
            className="fill-accent"
          />
        ) : geometry.kind === "POLYGON" ? (
          <polygon
            points={geometry.points.map(({ x, y }) => `${x},${y}`).join(" ")}
            className="fill-accent/20 stroke-accent"
            strokeWidth="2"
          />
        ) : (
          <polyline
            points={geometry.points.map(({ x, y }) => `${x},${y}`).join(" ")}
            className="fill-none stroke-accent"
            strokeWidth="2"
          />
        )}
      </svg>
      <div className="mt-2 text-2xs text-text-muted">
        {geometry.kind} · {geometry.points.length} points · bounds{" "}
        {geometry.bounds}
      </div>
    </div>
  );
}

type JsonParseResult =
  | { ok: true; value: unknown }
  | { ok: false; message: string };

/* oxlint-enable jsx-a11y/prefer-tag-over-role */
function parseJson(value: string): JsonParseResult {
  try {
    return { ok: true, value: JSON.parse(value) };
  } catch (error) {
    return {
      ok: false,
      message: error instanceof Error ? error.message : "Invalid JSON",
    };
  }
}

type GeometryParseResult =
  | {
      ok: true;
      kind: "POINT" | "LINESTRING" | "POLYGON";
      points: Array<{ x: number; y: number }>;
      bounds: string;
    }
  | { ok: false; message: string };

function parseWkt(value: string): GeometryParseResult {
  const trimmed = value.trim();
  const match = /^(POINT|LINESTRING|POLYGON)\s*\((.*)\)$/i.exec(trimmed);
  if (!match) {
    return {
      ok: false,
      message: "Supports POINT, LINESTRING, and POLYGON WKT.",
    };
  }

  // SAFETY: The value is constrained by the typed component or library contract at this boundary.
  const kind = match[1].toUpperCase() as "POINT" | "LINESTRING" | "POLYGON";
  const body = kind === "POLYGON" ? match[2].replace(/^\(|\)$/g, "") : match[2];
  const rawPoints = body.split(",").map((point) => {
    const [x, y] = point.trim().split(/\s+/).map(Number);
    return { x, y };
  });

  if (
    rawPoints.some(
      (point) => !Number.isFinite(point.x) || !Number.isFinite(point.y),
    )
  ) {
    return { ok: false, message: "Coordinates must be numeric x y pairs." };
  }

  const xs = rawPoints.map((point) => point.x);
  const ys = rawPoints.map((point) => point.y);
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);
  const width = maxX - minX || 1;
  const height = maxY - minY || 1;
  const points = rawPoints.map((point) => ({
    x: 16 + ((point.x - minX) / width) * 288,
    y: 164 - ((point.y - minY) / height) * 148,
  }));

  return {
    ok: true,
    kind,
    points,
    bounds: `${minX},${minY} → ${maxX},${maxY}`,
  };
}

function toggleSet(values: Set<string>, value: string): Set<string> {
  const next = new Set(values);
  if (next.has(value)) {
    next.delete(value);
  } else {
    next.add(value);
  }
  return next;
}

function splitIdentifierList(value: string): string[] {
  const identifiers = value
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean);
  return identifiers.length > 0 ? identifiers : ["id"];
}

function toPgArrayLiteral(value: string): string {
  const items = value
    .split(/\r?\n|,/)
    .map((item) => item.trim())
    .filter(Boolean)
    .map((item) => `"${item.replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`);
  return `{${items.join(",")}}`;
}

function quoteIdentOrPublic(value: string): string {
  const trimmed = value.trim();
  if (!trimmed || trimmed.toUpperCase() === "PUBLIC") {
    return "PUBLIC";
  }
  return pgQuoteIdent(trimmed);
}

function quoteLiteral(value: string): string {
  return `'${value.replaceAll("'", "''")}'`;
}

function indent(value: string, prefix: string): string {
  return value
    .split("\n")
    .map((line) => `${prefix}${line}`)
    .join("\n");
}
