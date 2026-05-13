export type ColumnShape = {
  name: string;
  dataType: string;
  nullable: boolean;
};

export type TableShape = {
  schema: string;
  name: string;
  columns: ColumnShape[];
};

export type SchemaDiff =
  | { kind: "missing-table"; table: string }
  | { kind: "extra-table"; table: string }
  | { kind: "missing-column"; table: string; column: string }
  | { kind: "extra-column"; table: string; column: string }
  | {
      kind: "changed-column";
      table: string;
      column: string;
      before: ColumnShape;
      after: ColumnShape;
    };

export type DataRow = Record<string, string | number | boolean | null>;

const keyFor = (row: DataRow, keys: string[]): string =>
  keys.map((key) => String(row[key] ?? "")).join("\u001f");

export function compareSchemas(
  source: TableShape[],
  target: TableShape[],
): SchemaDiff[] {
  const targetByName = new Map(target.map((table) => [table.name, table]));
  const sourceByName = new Map(source.map((table) => [table.name, table]));
  const diffs: SchemaDiff[] = [];

  for (const sourceTable of source) {
    const targetTable = targetByName.get(sourceTable.name);
    if (!targetTable) {
      diffs.push({ kind: "missing-table", table: sourceTable.name });
      continue;
    }
    const targetColumns = new Map(
      targetTable.columns.map((column) => [column.name, column]),
    );
    const sourceColumns = new Map(
      sourceTable.columns.map((column) => [column.name, column]),
    );
    for (const sourceColumn of sourceTable.columns) {
      const targetColumn = targetColumns.get(sourceColumn.name);
      if (!targetColumn) {
        diffs.push({
          kind: "missing-column",
          table: sourceTable.name,
          column: sourceColumn.name,
        });
      } else if (
        sourceColumn.dataType !== targetColumn.dataType ||
        sourceColumn.nullable !== targetColumn.nullable
      ) {
        diffs.push({
          kind: "changed-column",
          table: sourceTable.name,
          column: sourceColumn.name,
          before: sourceColumn,
          after: targetColumn,
        });
      }
    }
    for (const targetColumn of targetTable.columns) {
      if (!sourceColumns.has(targetColumn.name)) {
        diffs.push({
          kind: "extra-column",
          table: sourceTable.name,
          column: targetColumn.name,
        });
      }
    }
  }
  for (const targetTable of target) {
    if (!sourceByName.has(targetTable.name)) {
      diffs.push({ kind: "extra-table", table: targetTable.name });
    }
  }
  return diffs;
}

export function migrationSqlForDiff(diff: SchemaDiff): string | null {
  switch (diff.kind) {
    case "missing-table":
      return `-- create table ${diff.table}`;
    case "missing-column":
      return `ALTER TABLE "${diff.table}" ADD COLUMN "${diff.column}" text;`;
    case "changed-column":
      return `ALTER TABLE "${diff.table}" ALTER COLUMN "${diff.column}" TYPE ${diff.before.dataType};`;
    case "extra-table":
    case "extra-column":
      return null;
  }
}

export function compareData(params: {
  keys: string[];
  source: DataRow[];
  target: DataRow[];
}): {
  missing: DataRow[];
  extra: DataRow[];
  changed: Array<{ key: string; before: DataRow; after: DataRow }>;
} {
  const targetByKey = new Map(
    params.target.map((row) => [keyFor(row, params.keys), row]),
  );
  const sourceByKey = new Map(
    params.source.map((row) => [keyFor(row, params.keys), row]),
  );
  const missing: DataRow[] = [];
  const changed: Array<{ key: string; before: DataRow; after: DataRow }> = [];
  for (const sourceRow of params.source) {
    const key = keyFor(sourceRow, params.keys);
    const targetRow = targetByKey.get(key);
    if (!targetRow) {
      missing.push(sourceRow);
    } else if (JSON.stringify(sourceRow) !== JSON.stringify(targetRow)) {
      changed.push({ key, before: sourceRow, after: targetRow });
    }
  }
  const extra = params.target.filter(
    (targetRow) => !sourceByKey.has(keyFor(targetRow, params.keys)),
  );
  return { missing, extra, changed };
}

export function generateMockRows(
  columns: ColumnShape[],
  count: number,
): DataRow[] {
  return Array.from({ length: count }, (_value, rowIndex) => {
    const row: DataRow = {};
    for (const column of columns) {
      const type = column.dataType.toLowerCase();
      if (type.includes("int") || type.includes("serial")) {
        row[column.name] = rowIndex + 1;
      } else if (type.includes("bool")) {
        row[column.name] = rowIndex % 2 === 0;
      } else if (type.includes("date") || type.includes("time")) {
        row[column.name] = "2026-05-13T00:00:00.000Z";
      } else {
        row[column.name] = `${column.name}_${rowIndex + 1}`;
      }
    }
    return row;
  });
}
