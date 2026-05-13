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
 * Default-value quoting rule (v1):
 *   - All-numeric-or-decimal values (matches /^-?[0-9]+(\.[0-9]+)?$/) are
 *     emitted as raw literals.
 *   - Values that end with `()` are treated as function calls and emitted raw.
 *   - Anything else is single-quoted; embedded `'` characters are escaped to
 *     `''`.
 *
 * This keeps the common cases (`42`, `12.50`, `now()`, `CURRENT_TIMESTAMP()`)
 * working without forcing the caller to think about quoting, while still
 * being safe for arbitrary text. Users who need an unquoted bareword like
 * `CURRENT_TIMESTAMP` (without parens) can append `()` or write a function
 * form. A richer expression model is a deliberate follow-up.
 */
export const formatDefault = (raw: string): string => {
  if (/^-?[0-9]+(\.[0-9]+)?$/.test(raw)) return raw;
  if (raw.endsWith("()")) return raw;
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
