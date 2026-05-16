/**
 * Identifier-quoting + literal-formatting helpers shared by the PG and
 * ClickHouse DDL builders. Per-engine modules instantiate a quoter via
 * `createIdentQuoter(quoteChar)` and import `formatDefault` directly.
 */

export function createIdentQuoter(quote: string): {
  quoteIdent: (identifier: string) => string;
  qualifiedTable: (schema: string, table: string) => string;
} {
  const escaped = quote + quote;
  const quoteIdent = (identifier: string): string =>
    `${quote}${identifier.replaceAll(quote, escaped)}${quote}`;
  const qualifiedTable = (schema: string, table: string): string =>
    `${quoteIdent(schema)}.${quoteIdent(table)}`;
  return { quoteIdent, qualifiedTable };
}

/**
 * SQL keyword barewords that should pass through as raw expressions rather
 * than be quoted as string literals. Matched case-insensitively so users can
 * type `current_timestamp` or `CURRENT_TIMESTAMP` and get the same result.
 *
 * Anything not on this list is quoted — including unrecognised barewords —
 * so an opt-in-only allowlist keeps the "safe for arbitrary text" guarantee.
 */
const RAW_DEFAULT_KEYWORDS = new Set([
  "CURRENT_TIMESTAMP",
  "CURRENT_DATE",
  "CURRENT_TIME",
  "LOCALTIMESTAMP",
  "LOCALTIME",
  "CURRENT_USER",
  "SESSION_USER",
  "USER",
  "NULL",
  "TRUE",
  "FALSE",
]);

/**
 * Default-value quoting rule:
 *   - All-numeric-or-decimal values (matches /^-?[0-9]+(\.[0-9]+)?$/) are
 *     emitted as raw literals.
 *   - Values that end with `()` are treated as function calls and emitted raw.
 *   - Barewords matching `RAW_DEFAULT_KEYWORDS` (case-insensitive) are emitted
 *     raw — covers `CURRENT_TIMESTAMP`, `NULL`, `TRUE`, etc.
 *   - Anything else is single-quoted; embedded `'` characters are escaped to
 *     `''`.
 *
 * A tagged-union "literal vs expression" model is a deliberate follow-up —
 * for arbitrary SQL expressions the user can append `()` to a function name
 * or write the full function form (e.g., `now()`, `gen_random_uuid()`).
 */
export const formatDefault = (raw: string): string => {
  if (/^-?[0-9]+(\.[0-9]+)?$/.test(raw)) return raw;
  if (raw.endsWith("()")) return raw;
  if (RAW_DEFAULT_KEYWORDS.has(raw.toUpperCase())) return raw;
  return `'${raw.replace(/'/g, "''")}'`;
};

/**
 * Engine-agnostic case renderers — the `ADD COLUMN`, `DROP COLUMN`,
 * and `RENAME COLUMN` syntaxes are identical across PostgreSQL and
 * ClickHouse, so they share a renderer. Engine-specific cases
 * (`set_type`, `set_nullable`, `set_default`) stay in each builder
 * because the SQL keywords differ.
 */
export function renderAddColumn(
  prefix: string,
  columnDefinition: string,
): string {
  return `${prefix} ADD COLUMN ${columnDefinition};`;
}

export function renderDropColumn(prefix: string, quotedColumn: string): string {
  return `${prefix} DROP COLUMN ${quotedColumn};`;
}

export function renderRenameColumn(
  prefix: string,
  quotedOld: string,
  quotedNew: string,
): string {
  return `${prefix} RENAME COLUMN ${quotedOld} TO ${quotedNew};`;
}
