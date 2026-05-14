import { format, type SqlLanguage } from "sql-formatter";

import type { DatabaseEngine } from "@/lib/store";

/**
 * Pick a `sql-formatter` dialect for a dbunk engine. We default to
 * generic `sql` for anything that doesn't have a dedicated dialect
 * (Redis isn't relational; nothing else lands here today).
 */
export function dialectForEngine(engine: DatabaseEngine): SqlLanguage {
  switch (engine) {
    case "PostgreSQL":
      return "postgresql";
    case "MySQL":
      return "mysql";
    case "SQLite":
      return "sqlite";
    case "ClickHouse":
      return "clickhouse";
    default:
      return "sql";
  }
}

export type FormatSqlResult =
  | { kind: "formatted"; sql: string }
  | { kind: "unchanged" }
  | { kind: "empty" }
  | { kind: "failed"; reason: string };

/**
 * Format a SQL string with the appropriate dialect. Returns one of:
 * - `formatted` — the formatter produced output that differs from the
 *   input.
 * - `unchanged` — the formatter ran but the output matches the input.
 * - `empty` — the input is whitespace-only.
 * - `failed` — the formatter threw (typically a parse error on a
 *   half-typed query). The caller can keep the user's text intact
 *   and surface the reason.
 */
export function formatSql(
  sql: string,
  engine: DatabaseEngine,
): FormatSqlResult {
  if (!sql.trim()) return { kind: "empty" };
  try {
    const formatted = format(sql, {
      language: dialectForEngine(engine),
      keywordCase: "upper",
      tabWidth: 2,
      useTabs: false,
      linesBetweenQueries: 2,
    });
    if (formatted === sql) return { kind: "unchanged" };
    return { kind: "formatted", sql: formatted };
  } catch (error) {
    return {
      kind: "failed",
      reason: error instanceof Error ? error.message : String(error),
    };
  }
}
