export type GrantPrivilege =
  | "SELECT"
  | "INSERT"
  | "UPDATE"
  | "DELETE"
  | "TRUNCATE";

export type IndexSpec = {
  schema: string;
  table: string;
  name: string;
  columns: string[];
  unique?: boolean;
  method?: "btree" | "hash" | "gin" | "gist" | "brin";
};

export type ForeignKeySpec = {
  schema: string;
  table: string;
  name: string;
  columns: string[];
  referencedSchema: string;
  referencedTable: string;
  referencedColumns: string[];
  onDelete?: "CASCADE" | "RESTRICT" | "SET NULL" | "NO ACTION";
};

export type TriggerSpec = {
  schema: string;
  table: string;
  name: string;
  timing: "BEFORE" | "AFTER";
  event: "INSERT" | "UPDATE" | "DELETE";
  functionName: string;
};

const quoteIdentifier = (identifier: string): string =>
  `"${identifier.replace(/"/g, '""')}"`;

const qualified = (schema: string, name: string): string =>
  `${quoteIdentifier(schema)}.${quoteIdentifier(name)}`;

const quoteLiteral = (value: string): string =>
  `'${value.replace(/'/g, "''")}'`;

export function grantSql(params: {
  privileges: GrantPrivilege[];
  schema: string;
  table: string;
  role: string;
}): string {
  return `GRANT ${params.privileges.join(", ")} ON TABLE ${qualified(
    params.schema,
    params.table,
  )} TO ${quoteIdentifier(params.role)};`;
}

export function rlsPolicySql(params: {
  schema: string;
  table: string;
  name: string;
  command: "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "ALL";
  using?: string;
  check?: string;
}): string {
  const clauses = [
    `CREATE POLICY ${quoteIdentifier(params.name)} ON ${qualified(params.schema, params.table)}`,
    `FOR ${params.command}`,
    params.using ? `USING (${params.using})` : "",
    params.check ? `WITH CHECK (${params.check})` : "",
  ].filter(Boolean);
  return `${clauses.join(" ")};`;
}

export function createIndexSql(spec: IndexSpec): string {
  const unique = spec.unique ? "UNIQUE " : "";
  const method = spec.method ? ` USING ${spec.method}` : "";
  const columns = spec.columns.map(quoteIdentifier).join(", ");
  return `CREATE ${unique}INDEX ${quoteIdentifier(spec.name)} ON ${qualified(
    spec.schema,
    spec.table,
  )}${method} (${columns});`;
}

export function createForeignKeySql(spec: ForeignKeySpec): string {
  const columns = spec.columns.map(quoteIdentifier).join(", ");
  const referenced = spec.referencedColumns.map(quoteIdentifier).join(", ");
  const onDelete = spec.onDelete ? ` ON DELETE ${spec.onDelete}` : "";
  return `ALTER TABLE ${qualified(spec.schema, spec.table)} ADD CONSTRAINT ${quoteIdentifier(
    spec.name,
  )} FOREIGN KEY (${columns}) REFERENCES ${qualified(
    spec.referencedSchema,
    spec.referencedTable,
  )} (${referenced})${onDelete};`;
}

export function createTriggerSql(spec: TriggerSpec): string {
  return `CREATE TRIGGER ${quoteIdentifier(spec.name)} ${spec.timing} ${
    spec.event
  } ON ${qualified(spec.schema, spec.table)} FOR EACH ROW EXECUTE FUNCTION ${
    spec.functionName
  }();`;
}

export function postgresArrayLiteral(values: Array<string | null>): string {
  return `ARRAY[${values
    .map((value) => (value === null ? "NULL" : quoteLiteral(value)))
    .join(", ")}]`;
}

export function formatJsonCell(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

export function geometryPreviewSql(
  schema: string,
  table: string,
  column: string,
): string {
  return `SELECT ST_AsGeoJSON(${quoteIdentifier(column)}) AS geometry FROM ${qualified(
    schema,
    table,
  )} WHERE ${quoteIdentifier(column)} IS NOT NULL LIMIT 500;`;
}
