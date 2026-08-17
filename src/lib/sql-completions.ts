import {
  type SchemaExplorer,
  type TableStructure,
  tableStructureKey,
} from "@/lib/store";

export type SqlCompletionContext = {
  connectionId?: string;
  schemas: SchemaExplorer[];
  currentSchema?: string;
  tableStructure?: Record<string, TableStructure>;
};

export type SqlCompletionKind =
  | "column"
  | "keyword"
  | "schema"
  | "table"
  | "view";

export type SqlCompletion = {
  label: string;
  insertText: string;
  kind: SqlCompletionKind;
  detail: string;
  sortText: string;
};

export const SQL_KEYWORDS = [
  "select",
  "from",
  "where",
  "join",
  "left join",
  "right join",
  "inner join",
  "outer join",
  "cross join",
  "on",
  "group by",
  "order by",
  "having",
  "limit",
  "offset",
  "insert into",
  "values",
  "update",
  "set",
  "delete from",
  "create table",
  "alter table",
  "drop table",
  "with",
  "as",
  "distinct",
  "and",
  "or",
  "not",
  "is null",
  "is not null",
  "between",
  "like",
  "in",
  "exists",
] as const;

const TABLE_CONTEXT_PATTERN =
  /(?:^|[\s(;])(?:from|join|update|into|describe|desc|table|truncate\s+table|delete\s+from|merge\s+into)\s+(?:(?:"[^"]+"|`[^`]+`|\[[^\]]+\]|[\w$]+)\.)?(?:"[^"]*"?|`[^`]*`?|\[[^\]]*\]?|[\w$]*)$/i;

const SCHEMA_QUALIFIER_PATTERN =
  /(?:^|[\s(,])(?<schema>"[^"]+"|`[^`]+`|\[[^\]]+\]|[\w$]+)\.(?:"[^"]*"?|`[^`]*`?|\[[^\]]*\]?|[\w$]*)$/i;

const PREDICATE_CONTEXT_PATTERN =
  /(?:^|[\s)])(?:where|and|or|on|having)\s+(?:"[^"]*"?|`[^`]*`?|\[[^\]]*\]?|[\w$]*)$/i;

const TABLE_REFERENCE_PATTERN =
  /(?:^|[\s(;])(?:from|join|update|into|delete\s+from)\s+(?<table>(?:(?:"[^"]+"|`[^`]+`|\[[^\]]+\]|[\w$]+)\.)?(?:"[^"]+"|`[^`]+`|\[[^\]]+\]|[\w$]+))(?:\s+(?:as\s+)?(?!"?where\b|"?join\b|"?left\b|"?right\b|"?inner\b|"?outer\b|"?cross\b|"?on\b|"?group\b|"?order\b|"?having\b|"?limit\b|"?offset\b)(?:"[^"]+"|`[^`]+`|\[[^\]]+\]|[\w$]+))?/gi;

const RESERVED_WORDS = new Set(
  SQL_KEYWORDS.flatMap((keyword) => keyword.split(/\s+/)),
);

const unquoteIdentifier = (value: string): string =>
  value
    .replace(/^"(.+)"$/, "$1")
    .replace(/^`(.+)`$/, "$1")
    .replace(/^\[(.+)\]$/, "$1");

const quoteIdentifier = (value: string): string => {
  if (
    /^[A-Za-z_][A-Za-z0-9_$]*$/.test(value) &&
    !RESERVED_WORDS.has(value.toLowerCase())
  ) {
    return value;
  }
  return `"${value.replaceAll('"', '""')}"`;
};

export type SqlTableReference = {
  schema: string;
  table: string;
};

const splitTableReference = (
  value: string,
  fallbackSchema: string,
): SqlTableReference => {
  const parts = value.split(".").map(unquoteIdentifier);
  if (parts.length >= 2) {
    return {
      schema: parts.at(-2) ?? fallbackSchema,
      table: parts.at(-1) ?? "",
    };
  }
  return {
    schema: fallbackSchema,
    table: unquoteIdentifier(value),
  };
};

export const getSqlPredicateTableReference = (
  textBeforeCursor: string,
  context: Pick<SqlCompletionContext, "currentSchema" | "schemas">,
): SqlTableReference | null => {
  if (!PREDICATE_CONTEXT_PATTERN.test(textBeforeCursor)) {
    return null;
  }

  const fallbackSchema =
    context.currentSchema ?? context.schemas[0]?.name ?? "public";
  const matches = [...textBeforeCursor.matchAll(TABLE_REFERENCE_PATTERN)];
  const tableReference = matches.at(-1)?.groups?.table;

  return tableReference
    ? splitTableReference(tableReference, fallbackSchema)
    : null;
};

const tableCompletionsForSchema = (
  schema: SchemaExplorer,
  options: { qualified: boolean; sortPrefix: string },
): SqlCompletion[] => [
  ...schema.tables.map((table): SqlCompletion => ({
    label: options.qualified ? `${schema.name}.${table}` : table,
    insertText: options.qualified
      ? `${quoteIdentifier(schema.name)}.${quoteIdentifier(table)}`
      : quoteIdentifier(table),
    kind: "table",
    detail: `Table in ${schema.name}`,
    sortText: `${options.sortPrefix}${table}`,
  })),
  ...(schema.views ?? []).map((view): SqlCompletion => ({
    label: options.qualified ? `${schema.name}.${view}` : view,
    insertText: options.qualified
      ? `${quoteIdentifier(schema.name)}.${quoteIdentifier(view)}`
      : quoteIdentifier(view),
    kind: "view",
    detail: `View in ${schema.name}`,
    sortText: `${options.sortPrefix}${view}`,
  })),
];

const schemaCompletions = (schemas: SchemaExplorer[]): SqlCompletion[] =>
  schemas.map((schema): SqlCompletion => ({
    label: schema.name,
    insertText: quoteIdentifier(schema.name),
    kind: "schema",
    detail: "Schema",
    sortText: `1${schema.name}`,
  }));

const keywordCompletions = (): SqlCompletion[] =>
  SQL_KEYWORDS.map((keyword): SqlCompletion => ({
    label: keyword,
    insertText: keyword,
    kind: "keyword",
    detail: "SQL keyword",
    sortText: `9${keyword}`,
  }));

const columnCompletions = (
  structure: TableStructure,
  table: SqlTableReference,
): SqlCompletion[] =>
  structure.columns.map((column): SqlCompletion => ({
    label: column.name,
    insertText: quoteIdentifier(column.name),
    kind: "column",
    detail: `${column.dataType}${column.isPrimaryKey ? " primary key" : ""} in ${table.schema}.${table.table}`,
    sortText: `0${String(column.ordinalPosition).padStart(4, "0")}${column.name}`,
  }));

const uniqueByLabel = (items: SqlCompletion[]): SqlCompletion[] => {
  const seen = new Set<string>();
  return items.filter((item) => {
    const key = `${item.kind}:${item.label}`;
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
};

export const getSqlCompletions = (
  textBeforeCursor: string,
  context: SqlCompletionContext,
): SqlCompletion[] => {
  const schemas = context.schemas;
  const predicateTable = getSqlPredicateTableReference(
    textBeforeCursor,
    context,
  );
  const structure =
    predicateTable && context.connectionId
      ? context.tableStructure?.[
          tableStructureKey(
            context.connectionId,
            predicateTable.schema,
            predicateTable.table,
          )
        ]
      : null;

  if (predicateTable && structure) {
    return uniqueByLabel([
      ...columnCompletions(structure, predicateTable),
      ...keywordCompletions(),
    ]);
  }

  const schemaQualifier = textBeforeCursor.match(SCHEMA_QUALIFIER_PATTERN);
  const qualifiedSchemaName = schemaQualifier?.groups?.schema
    ? unquoteIdentifier(schemaQualifier.groups.schema)
    : "";

  if (qualifiedSchemaName) {
    const schema = schemas.find(
      (item) => item.name.toLowerCase() === qualifiedSchemaName.toLowerCase(),
    );
    return schema
      ? tableCompletionsForSchema(schema, { qualified: false, sortPrefix: "0" })
      : [];
  }

  const currentSchema =
    schemas.find((schema) => schema.name === context.currentSchema) ??
    schemas[0];
  const otherSchemas = schemas.filter((schema) => schema !== currentSchema);
  const tableSuggestions = [
    ...(currentSchema
      ? tableCompletionsForSchema(currentSchema, {
          qualified: false,
          sortPrefix: "0",
        })
      : []),
    ...otherSchemas.flatMap((schema) =>
      tableCompletionsForSchema(schema, {
        qualified: true,
        sortPrefix: "2",
      }),
    ),
  ];

  if (TABLE_CONTEXT_PATTERN.test(textBeforeCursor)) {
    return uniqueByLabel([
      ...tableSuggestions,
      ...schemaCompletions(schemas),
      ...keywordCompletions(),
    ]);
  }

  return uniqueByLabel([
    ...keywordCompletions(),
    ...schemaCompletions(schemas),
    ...tableSuggestions,
  ]);
};
