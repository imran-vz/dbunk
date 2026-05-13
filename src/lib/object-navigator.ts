export type PgObjectKind =
  | "materialized-view"
  | "function"
  | "procedure"
  | "aggregate"
  | "sequence"
  | "foreign-table"
  | "type"
  | "domain"
  | "extension"
  | "event-trigger"
  | "table-triggers"
  | "table-rules"
  | "table-policies"
  | "table-partitions"
  | "table-dependencies"
  | "table-references"
  | "role"
  | "tablespace";

export type NavigatorNode = {
  id: string;
  label: string;
  kind: PgObjectKind | "group";
  viewer: "list" | "definition" | "properties" | "table-children";
  actions: string[];
  children?: NavigatorNode[];
};

const group = (
  id: string,
  label: string,
  children: NavigatorNode[],
): NavigatorNode => ({
  id,
  label,
  kind: "group",
  viewer: "list",
  actions: [],
  children,
});

const node = (
  id: string,
  label: string,
  kind: PgObjectKind,
  actions: string[] = [],
  viewer: NavigatorNode["viewer"] = "properties",
): NavigatorNode => ({ id, label, kind, viewer, actions });

export function postgresNavigatorTemplate(): NavigatorNode[] {
  return [
    group("relations", "Relations", [
      node("materialized-views", "Materialized views", "materialized-view", [
        "refresh",
      ]),
      node("foreign-tables", "Foreign tables", "foreign-table"),
    ]),
    group("routines", "Routines", [
      node("functions", "Functions", "function", [], "definition"),
      node("procedures", "Procedures", "procedure", [], "definition"),
      node("aggregates", "Aggregate functions", "aggregate", [], "definition"),
    ]),
    group("programmability", "Programmability", [
      node("sequences", "Sequences", "sequence", [
        "edit",
        "restart",
        "next-value",
      ]),
      node("types", "Custom types", "type"),
      node("domains", "Domains", "domain"),
      node("event-triggers", "Event triggers", "event-trigger"),
    ]),
    group("database", "Database", [
      node("extensions", "Extensions", "extension", ["install", "drop"]),
      node("roles", "Roles", "role"),
      node("tablespaces", "Tablespaces", "tablespace"),
    ]),
  ];
}

export function tableChildNodes(
  schema: string,
  table: string,
): NavigatorNode[] {
  const prefix = `${schema}.${table}`;
  return [
    node(
      `${prefix}.triggers`,
      "Triggers",
      "table-triggers",
      [],
      "table-children",
    ),
    node(`${prefix}.rules`, "Rules", "table-rules", [], "table-children"),
    node(
      `${prefix}.policies`,
      "Policies",
      "table-policies",
      [],
      "table-children",
    ),
    node(
      `${prefix}.partitions`,
      "Partitions",
      "table-partitions",
      [],
      "table-children",
    ),
    node(
      `${prefix}.dependencies`,
      "Dependencies",
      "table-dependencies",
      [],
      "table-children",
    ),
    node(
      `${prefix}.references`,
      "References",
      "table-references",
      [],
      "table-children",
    ),
  ];
}

export function sequenceActionSql(params: {
  schema: string;
  sequence: string;
  action: "restart" | "next-value";
  restartWith?: number;
}): string {
  const qualified = `"${params.schema}"."${params.sequence}"`;
  if (params.action === "next-value") {
    return `SELECT nextval('${params.schema}.${params.sequence}'::regclass);`;
  }
  return `ALTER SEQUENCE ${qualified} RESTART WITH ${params.restartWith ?? 1};`;
}

export function refreshMaterializedViewSql(params: {
  schema: string;
  view: string;
  concurrently?: boolean;
}): string {
  return `REFRESH MATERIALIZED VIEW${params.concurrently ? " CONCURRENTLY" : ""} "${params.schema}"."${params.view}";`;
}
