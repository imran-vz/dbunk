export type DdlExportScope =
  | { kind: "table"; schema: string; table: string }
  | { kind: "schema"; schema: string }
  | { kind: "database" };

export type PgDumpFormat = "plain" | "custom";

export type PgToolConnection = {
  host: string;
  port: number;
  database: string;
  user: string;
};

const quoteIdentifier = (identifier: string): string =>
  `"${identifier.replace(/"/g, '""')}"`;

export function ddlExportStatement(scope: DdlExportScope): string {
  switch (scope.kind) {
    case "table":
      return `SELECT pg_get_tabledef('${scope.schema}.${scope.table}'::regclass);`;
    case "schema":
      return `SELECT pg_get_ddl_for_schema(${JSON.stringify(scope.schema)});`;
    case "database":
      return "SELECT pg_get_ddl_for_database();";
  }
}

const basePgArgs = (connection: PgToolConnection): string[] => [
  "--host",
  connection.host,
  "--port",
  String(connection.port || 5432),
  "--username",
  connection.user,
  "--dbname",
  connection.database,
];

export function buildPgDumpArgs(params: {
  connection: PgToolConnection;
  scope: DdlExportScope;
  format: PgDumpFormat;
  file: string;
}): string[] {
  const args = [...basePgArgs(params.connection), "--file", params.file];
  args.push("--format", params.format === "custom" ? "custom" : "plain");
  if (params.scope.kind === "schema") {
    args.push("--schema", params.scope.schema);
  }
  if (params.scope.kind === "table") {
    args.push("--table", `${params.scope.schema}.${params.scope.table}`);
  }
  return args;
}

export function buildPgRestoreArgs(params: {
  connection: PgToolConnection;
  file: string;
  clean?: boolean;
  dataOnly?: boolean;
}): string[] {
  const args = [...basePgArgs(params.connection), params.file];
  if (params.clean) {
    args.push("--clean");
  }
  if (params.dataOnly) {
    args.push("--data-only");
  }
  return args;
}

export function buildCrossConnectionCopySql(params: {
  sourceSchema: string;
  sourceTable: string;
  targetSchema: string;
  targetTable: string;
  columns: string[];
}): { exportSql: string; importSql: string } {
  const columns = params.columns.map(quoteIdentifier).join(", ");
  const source = `${quoteIdentifier(params.sourceSchema)}.${quoteIdentifier(
    params.sourceTable,
  )}`;
  const target = `${quoteIdentifier(params.targetSchema)}.${quoteIdentifier(
    params.targetTable,
  )}`;
  return {
    exportSql: `COPY (SELECT ${columns} FROM ${source}) TO STDOUT WITH (FORMAT csv, HEADER true)`,
    importSql: `COPY ${target} (${columns}) FROM STDIN WITH (FORMAT csv, HEADER true)`,
  };
}
