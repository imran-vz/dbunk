import { useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import type { Connection, SchemaExplorer } from "@/lib/store";
import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";

type TableDataResult = {
  columns: string[];
  rows: string[][];
};

export function CompareTab({
  connection,
  schemas,
}: {
  connection: Connection;
  schemas: SchemaExplorer[];
}) {
  const [leftSchema, setLeftSchema] = useState(schemas[0]?.name ?? "public");
  const [rightSchema, setRightSchema] = useState(
    schemas[1]?.name ?? leftSchema,
  );
  const [leftTable, setLeftTable] = useState("");
  const [rightTable, setRightTable] = useState("");
  const [dataDiff, setDataDiff] = useState<string[]>([]);
  const [mockTable, setMockTable] = useState("");
  const [mockSql, setMockSql] = useState("");
  const [error, setError] = useState<string | null>(null);

  const schemaDiff = useMemo(
    () => compareSchemas(schemas, leftSchema, rightSchema),
    [schemas, leftSchema, rightSchema],
  );

  const runDataCompare = async () => {
    if (!isTauri()) {
      setError("Data compare requires the desktop runtime.");
      return;
    }
    setError(null);
    try {
      const [left, right] = await Promise.all([
        loadPreview(connection.id, leftSchema, leftTable),
        loadPreview(connection.id, rightSchema, rightTable),
      ]);
      setDataDiff(compareRows(left, right));
    } catch (error) {
      setError(errorToMessage(error));
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle>Schema compare</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3 text-xs">
          <div className="grid gap-2 md:grid-cols-2">
            <SchemaSelect
              value={leftSchema}
              schemas={schemas}
              onChange={setLeftSchema}
            />
            <SchemaSelect
              value={rightSchema}
              schemas={schemas}
              onChange={setRightSchema}
            />
          </div>
          <DiffList
            rows={schemaDiff}
            empty="Schemas have the same visible objects."
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Data compare</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3 text-xs">
          <div className="grid gap-2 md:grid-cols-4">
            <Input
              value={leftSchema}
              onChange={(event) => setLeftSchema(event.target.value)}
            />
            <Input
              value={leftTable}
              onChange={(event) => setLeftTable(event.target.value)}
              placeholder="Left table"
            />
            <Input
              value={rightSchema}
              onChange={(event) => setRightSchema(event.target.value)}
            />
            <Input
              value={rightTable}
              onChange={(event) => setRightTable(event.target.value)}
              placeholder="Right table"
            />
          </div>
          <Button size="sm" onClick={() => void runDataCompare()}>
            Compare first 100 rows
          </Button>
          <DiffList rows={dataDiff} empty="No sampled row differences." />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Mock data generator</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3 text-xs">
          <div className="grid gap-2 md:grid-cols-[1fr_auto]">
            <Input
              value={mockTable}
              onChange={(event) => setMockTable(event.target.value)}
              placeholder="schema.table"
            />
            <Button
              size="sm"
              onClick={() =>
                setMockSql(
                  generateMockSql(
                    mockTable || `${leftSchema}.${leftTable || "table_name"}`,
                  ),
                )
              }
            >
              Generate INSERTs
            </Button>
          </div>
          <pre className="max-h-56 overflow-auto rounded-md border border-border-subtle bg-surface-panel p-3 font-mono text-[0.6875rem]">
            {mockSql || "-- generated mock INSERTs appear here"}
          </pre>
        </CardContent>
      </Card>

      {error ? <div className="text-xs text-danger">{error}</div> : null}
    </div>
  );
}

function SchemaSelect({
  value,
  schemas,
  onChange,
}: {
  value: string;
  schemas: SchemaExplorer[];
  onChange: (value: string) => void;
}) {
  return (
    <select
      value={value}
      onChange={(event) => onChange(event.target.value)}
      className="h-8 rounded-sm border border-border-subtle bg-surface-input px-2"
    >
      {schemas.map((schema) => (
        <option key={schema.name} value={schema.name}>
          {schema.name}
        </option>
      ))}
    </select>
  );
}

function DiffList({ rows, empty }: { rows: string[]; empty: string }) {
  return rows.length > 0 ? (
    <ul className="rounded-md border border-border-subtle">
      {rows.map((row) => (
        <li
          key={row}
          className="border-b border-border-subtle px-2 py-1 last:border-b-0"
        >
          {row}
        </li>
      ))}
    </ul>
  ) : (
    <div className="rounded-md border border-dashed border-border-subtle px-3 py-6 text-center text-text-muted">
      {empty}
    </div>
  );
}

function compareSchemas(
  schemas: SchemaExplorer[],
  leftName: string,
  rightName: string,
): string[] {
  const left = schemas.find((schema) => schema.name === leftName);
  const right = schemas.find((schema) => schema.name === rightName);
  const leftObjects = objectSet(left);
  const rightObjects = objectSet(right);
  return [
    ...[...leftObjects]
      .filter((item) => !rightObjects.has(item))
      .map((item) => `Only in ${leftName}: ${item}`),
    ...[...rightObjects]
      .filter((item) => !leftObjects.has(item))
      .map((item) => `Only in ${rightName}: ${item}`),
  ];
}

function objectSet(schema: SchemaExplorer | undefined): Set<string> {
  if (!schema) return new Set();
  return new Set([
    ...schema.tables.map((name) => `table ${name}`),
    ...(schema.views ?? []).map((name) => `view ${name}`),
    ...(schema.materializedViews ?? []).map(
      (name) => `materialized view ${name}`,
    ),
    ...(schema.sequences ?? []).map((name) => `sequence ${name}`),
    ...(schema.functions ?? []).map((name) => `function ${name}`),
  ]);
}

async function loadPreview(
  connectionId: string,
  schema: string,
  table: string,
): Promise<TableDataResult> {
  return tauriInvoke<TableDataResult>("load_table_data", {
    payload: { connectionId, schema, table, page: 1, pageSize: 100 },
  });
}

function compareRows(left: TableDataResult, right: TableDataResult): string[] {
  const rows = [];
  if (left.columns.join("|") !== right.columns.join("|")) {
    rows.push(
      `Columns differ: ${left.columns.join(", ")} vs ${right.columns.join(", ")}`,
    );
  }
  const max = Math.max(left.rows.length, right.rows.length);
  for (let index = 0; index < max; index += 1) {
    if (
      (left.rows[index] ?? []).join("|") !== (right.rows[index] ?? []).join("|")
    ) {
      rows.push(`Row ${index + 1} differs`);
    }
  }
  return rows.slice(0, 100);
}

function generateMockSql(table: string): string {
  return Array.from({ length: 5 }, (_, index) => {
    const id = index + 1;
    return `insert into ${table} (id, name, created_at) values (${id}, 'Sample ${id}', now());`;
  }).join("\n");
}
