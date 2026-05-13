export type ExplainMode = "explain" | "analyze";

export type PlanNode = {
  nodeType: string;
  relationName?: string;
  totalCost?: number;
  actualTotalTime?: number;
  plans: PlanNode[];
};

export type SqlSnippet = {
  id: string;
  name: string;
  sql: string;
  tags: string[];
};

export function explainSql(sql: string, mode: ExplainMode = "explain"): string {
  const options =
    mode === "analyze"
      ? "ANALYZE, BUFFERS, VERBOSE, FORMAT JSON"
      : "VERBOSE, FORMAT JSON";
  return `EXPLAIN (${options}) ${sql.trim().replace(/;$/, "")};`;
}

export function normalizePlanNode(raw: Record<string, unknown>): PlanNode {
  const children = Array.isArray(raw.Plans) ? raw.Plans : [];
  return {
    nodeType: String(raw["Node Type"] ?? "Unknown"),
    relationName:
      typeof raw["Relation Name"] === "string"
        ? raw["Relation Name"]
        : undefined,
    totalCost:
      typeof raw["Total Cost"] === "number" ? raw["Total Cost"] : undefined,
    actualTotalTime:
      typeof raw["Actual Total Time"] === "number"
        ? raw["Actual Total Time"]
        : undefined,
    plans: children.map((child) =>
      normalizePlanNode(child as Record<string, unknown>),
    ),
  };
}

export const DEFAULT_SQL_SNIPPETS: SqlSnippet[] = [
  {
    id: "select-limit",
    name: "Select with limit",
    sql: "SELECT * FROM {{schema}}.{{table}} LIMIT {{limit}};",
    tags: ["select", "table"],
  },
  {
    id: "count-table",
    name: "Count table",
    sql: "SELECT COUNT(*) FROM {{schema}}.{{table}};",
    tags: ["select", "aggregate"],
  },
  {
    id: "active-sessions",
    name: "Active sessions",
    sql: "SELECT * FROM pg_stat_activity WHERE state = 'active';",
    tags: ["admin", "postgres"],
  },
];

export function applyBindVariables(
  sql: string,
  values: Record<string, string | number | boolean | null>,
): string {
  return sql.replace(/:(\w+)/g, (_match, name) => {
    if (!(name in values)) {
      throw new Error(`Missing bind variable :${name}`);
    }
    const value = values[name];
    if (value === null) {
      return "NULL";
    }
    if (typeof value === "number") {
      return String(value);
    }
    if (typeof value === "boolean") {
      return value ? "TRUE" : "FALSE";
    }
    return `'${value.replace(/'/g, "''")}'`;
  });
}

export function renderSnippet(
  snippet: SqlSnippet,
  variables: Record<string, string | number>,
): string {
  return snippet.sql.replace(/\{\{(\w+)}}/g, (match, name) => {
    if (!(name in variables)) {
      return match;
    }
    return String(variables[name]);
  });
}
