import {
  isNavigatorGroupExpanded,
  navigatorGroupId,
  type NavigatorGroupKey,
  type PgCatalogEntry,
  type PgObjectCatalog,
  type PgObjectKind,
  type PgObjectRef,
  type PgRoutineObjectKind,
  type PgSchemaObjects,
  type SchemaExplorer,
} from "@/lib/store";

export type CreatableObjectKind =
  | "schema"
  | "view"
  | "materialized-view"
  | "sequence"
  | "enum";

type SchemaGroupKey = Exclude<
  NavigatorGroupKey,
  "eventTriggers" | "roles" | "tablespaces"
>;
type DatabaseGroupKey = Extract<
  NavigatorGroupKey,
  "eventTriggers" | "roles" | "tablespaces"
>;

type SchemaGroupConfig = {
  key: SchemaGroupKey;
  label: string;
  abbreviation: string;
  objectKind: PgObjectKind;
};

type DatabaseGroupConfig = {
  key: DatabaseGroupKey;
  label: string;
  abbreviation: string;
  truncationKind: string;
};

export const SCHEMA_GROUPS: readonly SchemaGroupConfig[] = [
  { key: "tables", label: "Tables", abbreviation: "TBL", objectKind: "table" },
  { key: "views", label: "Views", abbreviation: "VIEW", objectKind: "view" },
  {
    key: "materializedViews",
    label: "Materialized Views",
    abbreviation: "MAT",
    objectKind: "materialized-view",
  },
  {
    key: "foreignTables",
    label: "Foreign Tables",
    abbreviation: "FRN",
    objectKind: "foreign-table",
  },
  {
    key: "sequences",
    label: "Sequences",
    abbreviation: "SEQ",
    objectKind: "sequence",
  },
  {
    key: "functions",
    label: "Functions",
    abbreviation: "FN",
    objectKind: "function",
  },
  {
    key: "procedures",
    label: "Procedures",
    abbreviation: "PROC",
    objectKind: "procedure",
  },
  {
    key: "aggregates",
    label: "Aggregates",
    abbreviation: "AGG",
    objectKind: "aggregate",
  },
  { key: "types", label: "Types", abbreviation: "TYPE", objectKind: "type" },
  {
    key: "domains",
    label: "Domains",
    abbreviation: "DOM",
    objectKind: "domain",
  },
  {
    key: "extensions",
    label: "Extensions",
    abbreviation: "EXT",
    objectKind: "extension",
  },
];

const DATABASE_GROUPS: readonly DatabaseGroupConfig[] = [
  {
    key: "eventTriggers",
    label: "Event Triggers",
    abbreviation: "ET",
    truncationKind: "event-trigger",
  },
  {
    key: "roles",
    label: "Roles",
    abbreviation: "ROLE",
    truncationKind: "role",
  },
  {
    key: "tablespaces",
    label: "Tablespaces",
    abbreviation: "TS",
    truncationKind: "tablespace",
  },
];

const DATABASE_SCOPE = "$database";
const INITIAL_GROUP_LIMIT = 200;

export type NavigatorRow =
  | {
      kind: "schema";
      id: string;
      name: string;
      expanded: boolean;
      count: number;
    }
  | {
      kind: "database";
      id: string;
      name: string;
      count: number;
    }
  | {
      kind: "group";
      id: string;
      name: string;
      schema: string;
      group: NavigatorGroupKey;
      abbreviation: string;
      expanded: boolean;
      count: number;
      parentId: string;
    }
  | {
      kind: "object";
      id: string;
      name: string;
      displayName: string;
      parentId: string;
      reference: PgObjectRef;
      typeClass?: PgCatalogEntry["typeClass"];
    }
  | {
      kind: "legacy-table";
      id: string;
      name: string;
      schema: string;
      parentId: string;
    }
  | {
      kind: "list-only";
      id: string;
      name: string;
      parentId: string;
      abbreviation: string;
    }
  | {
      kind: "show-more";
      id: string;
      name: string;
      parentId: string;
      remaining: number;
    }
  | {
      kind: "truncated";
      id: string;
      name: string;
      parentId: string;
    };

export const tableKey = (schema: string, table: string): string =>
  `${schema}.${table}`;

const entryDisplayName = (entry: PgCatalogEntry): string =>
  entry.identityArgs === undefined
    ? entry.name
    : `${entry.name}(${entry.identityArgs})`;

const matchesEntry = (entry: PgCatalogEntry, needle: string): boolean =>
  needle === "" || entryDisplayName(entry).toLowerCase().includes(needle);

const totalSchemaObjects = (schema: PgSchemaObjects): number =>
  SCHEMA_GROUPS.reduce((total, group) => total + schema[group.key].length, 0);

const objectRefFor = (
  schema: string,
  group: SchemaGroupConfig,
  entry: PgCatalogEntry,
): PgObjectRef => {
  if (
    group.objectKind === "function" ||
    group.objectKind === "procedure" ||
    group.objectKind === "aggregate"
  ) {
    const kind: PgRoutineObjectKind = group.objectKind;
    return {
      kind,
      schema,
      name: entry.name,
      identityArgs: entry.identityArgs ?? "",
    };
  }
  if (group.objectKind === "schema") {
    throw new Error("Schema entries are not object groups");
  }
  return {
    kind: group.objectKind,
    schema,
    name: entry.name,
    identityArgs: null,
  };
};

export const isBrowseRelation = (kind: PgObjectKind): boolean =>
  kind === "view" || kind === "materialized-view" || kind === "foreign-table";

export const createKindForGroup = (
  group: NavigatorGroupKey,
): Exclude<CreatableObjectKind, "schema"> | null => {
  switch (group) {
    case "views":
      return "view";
    case "materializedViews":
      return "materialized-view";
    case "sequences":
      return "sequence";
    case "types":
      return "enum";
    default:
      return null;
  }
};

type BuildNavigatorRowsInput = {
  connectionId: string;
  isPostgres: boolean;
  schemas: SchemaExplorer[];
  catalog: PgObjectCatalog | undefined;
  needle: string;
  expandedSchemas: readonly string[];
  expandedNavigatorGroups: readonly string[];
  expandedLimits: ReadonlySet<string>;
};

/** Pure, bounded row-model builder shared by rendering and keyboard behavior. */
export function buildNavigatorRows({
  connectionId,
  isPostgres,
  schemas,
  catalog,
  needle,
  expandedSchemas,
  expandedNavigatorGroups,
  expandedLimits,
}: BuildNavigatorRowsInput): NavigatorRow[] {
  const filtering = needle !== "";
  if (!isPostgres) {
    const rows: NavigatorRow[] = [];
    for (const schema of schemas) {
      const tables = schema.tables.filter(
        (table) => needle === "" || table.toLowerCase().includes(needle),
      );
      if (filtering && tables.length === 0) continue;
      const schemaId = `${connectionId}:${schema.name}`;
      const expanded = filtering || expandedSchemas.includes(schemaId);
      rows.push({
        kind: "schema",
        id: schemaId,
        name: schema.name,
        expanded,
        count: tables.length,
      });
      if (!expanded) continue;
      for (const table of tables) {
        rows.push({
          kind: "legacy-table",
          id: `${schemaId}:table:${table}`,
          name: table,
          schema: schema.name,
          parentId: schemaId,
        });
      }
    }
    return rows;
  }
  if (!catalog) return [];

  const rows: NavigatorRow[] = [];
  for (const schema of catalog.schemas) {
    const schemaMatches = schema.name.toLowerCase().includes(needle);
    const groupEntries = SCHEMA_GROUPS.map((group) => ({
      group,
      entries: schema[group.key].filter((entry) =>
        schemaMatches ? true : matchesEntry(entry, needle),
      ),
    })).filter(({ entries }) => entries.length > 0);
    if (filtering && !schemaMatches && groupEntries.length === 0) continue;

    const schemaId = `${connectionId}:${schema.name}`;
    const expanded = filtering || expandedSchemas.includes(schemaId);
    rows.push({
      kind: "schema",
      id: schemaId,
      name: schema.name,
      expanded,
      count: totalSchemaObjects(schema),
    });
    if (!expanded) continue;

    for (const { group, entries } of groupEntries) {
      const groupId = navigatorGroupId(connectionId, schema.name, group.key);
      const groupExpanded =
        filtering ||
        isNavigatorGroupExpanded(expandedNavigatorGroups, groupId, group.key);
      rows.push({
        kind: "group",
        id: groupId,
        name: group.label,
        schema: schema.name,
        group: group.key,
        abbreviation: group.abbreviation,
        expanded: groupExpanded,
        count: entries.length,
        parentId: schemaId,
      });
      if (!groupExpanded) continue;

      const visibleEntries = expandedLimits.has(groupId)
        ? entries
        : entries.slice(0, INITIAL_GROUP_LIMIT);
      for (const entry of visibleEntries) {
        const reference = objectRefFor(schema.name, group, entry);
        const displayName = entryDisplayName(entry);
        rows.push({
          kind: "object",
          id: `${groupId}:${JSON.stringify([entry.name, entry.identityArgs ?? ""])}`,
          name: displayName,
          displayName,
          parentId: groupId,
          reference,
          typeClass: entry.typeClass,
        });
      }
      if (entries.length > visibleEntries.length) {
        const remaining = entries.length - visibleEntries.length;
        rows.push({
          kind: "show-more",
          id: `${groupId}:show-more`,
          name: `Show ${remaining} more`,
          parentId: groupId,
          remaining,
        });
      }
      if (
        catalog.truncated.some(
          (item) =>
            item.schema === schema.name && item.kind === group.objectKind,
        )
      ) {
        rows.push({
          kind: "truncated",
          id: `${groupId}:truncated`,
          name: `${group.label} list cut at 2000 on the server`,
          parentId: groupId,
        });
      }
    }
  }

  const databaseGroups = DATABASE_GROUPS.map((group) => ({
    group,
    entries: catalog[group.key].filter((entry) => matchesEntry(entry, needle)),
  })).filter(({ entries }) => entries.length > 0);
  if (databaseGroups.length > 0) {
    const databaseId = `${connectionId}:${DATABASE_SCOPE}`;
    rows.push({
      kind: "database",
      id: databaseId,
      name: "Database objects",
      count: databaseGroups.reduce(
        (total, current) => total + current.entries.length,
        0,
      ),
    });
    for (const { group, entries } of databaseGroups) {
      const groupId = navigatorGroupId(connectionId, DATABASE_SCOPE, group.key);
      const groupExpanded =
        filtering ||
        isNavigatorGroupExpanded(expandedNavigatorGroups, groupId, group.key);
      rows.push({
        kind: "group",
        id: groupId,
        name: group.label,
        schema: DATABASE_SCOPE,
        group: group.key,
        abbreviation: group.abbreviation,
        expanded: groupExpanded,
        count: entries.length,
        parentId: databaseId,
      });
      if (!groupExpanded) continue;
      const visibleEntries = expandedLimits.has(groupId)
        ? entries
        : entries.slice(0, INITIAL_GROUP_LIMIT);
      for (const entry of visibleEntries) {
        rows.push({
          kind: "list-only",
          id: `${groupId}:${entry.name}`,
          name: entry.name,
          parentId: groupId,
          abbreviation: group.abbreviation,
        });
      }
      if (entries.length > visibleEntries.length) {
        const remaining = entries.length - visibleEntries.length;
        rows.push({
          kind: "show-more",
          id: `${groupId}:show-more`,
          name: `Show ${remaining} more`,
          parentId: groupId,
          remaining,
        });
      }
      if (
        catalog.truncated.some(
          (item) => item.schema === null && item.kind === group.truncationKind,
        )
      ) {
        rows.push({
          kind: "truncated",
          id: `${groupId}:truncated`,
          name: `${group.label} list cut at 2000 on the server`,
          parentId: groupId,
        });
      }
    }
  }
  if (
    catalog.truncated.some(
      (item) => item.schema === null && item.kind === "schema",
    )
  ) {
    rows.push({
      kind: "truncated",
      id: `${connectionId}:schemas:truncated`,
      name: "Schema list cut at 2000 on the server",
      parentId: "",
    });
  }
  return rows;
}
