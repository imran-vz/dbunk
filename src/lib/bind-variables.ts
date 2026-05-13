const NAMED_BIND_PATTERN = /(^|[^:]):([A-Za-z_][A-Za-z0-9_]*)/g;

export function extractBindVariables(sql: string): string[] {
  const seen = new Set<string>();
  for (const match of sql.matchAll(NAMED_BIND_PATTERN)) {
    seen.add(match[2]);
  }
  return [...seen].sort((a, b) => a.localeCompare(b));
}

export function applyBindVariables(
  sql: string,
  values: Record<string, string>,
): string {
  return sql.replace(
    NAMED_BIND_PATTERN,
    (match, prefix: string, name: string) => {
      if (!(name in values)) {
        return match;
      }
      return `${prefix}${sqlLiteral(values[name])}`;
    },
  );
}

export function sqlLiteral(value: string): string {
  if (value.trim().toUpperCase() === "NULL") {
    return "NULL";
  }
  if (/^-?\d+(\.\d+)?$/.test(value.trim())) {
    return value.trim();
  }
  return `'${value.replaceAll("'", "''")}'`;
}
