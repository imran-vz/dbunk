/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters, anti-slop/no-unsafe-dictionary-type -- Session-persisted drafts are external JSON and must be validated field by field before hydration. */
import type {
  PgCheckSpec,
  PgDefaultValue,
  PgForeignKeySpec,
  PgKeySpec,
  PgObjectOp,
  PgReferentialAction,
  TableDesignerColumnDraft,
  TableDesignerDraft,
  TableDesignerIndexDraft,
} from "@/lib/store/types";
import { buildCreateIndexOp } from "@/lib/structure-changes";

export type TableDesignerColumn = TableDesignerColumnDraft;
export type TableDesignerForm = TableDesignerDraft;
export type TableDesignerIndex = TableDesignerIndexDraft;

export type TableDesignerValidation = {
  valid: boolean;
  fields: Record<string, string>;
};

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null;

const isStringArray = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every((item) => typeof item === "string");

const isOptionalName = (value: unknown): value is string | null =>
  value === null || typeof value === "string";

const isKeySpec = (value: unknown): value is PgKeySpec =>
  isRecord(value) && isOptionalName(value.name) && isStringArray(value.columns);

const isCheckSpec = (value: unknown): value is PgCheckSpec =>
  isRecord(value) &&
  isOptionalName(value.name) &&
  typeof value.expression === "string";

const isReferentialAction = (value: unknown): value is PgReferentialAction =>
  value === "no-action" ||
  value === "restrict" ||
  value === "cascade" ||
  value === "set-null" ||
  value === "set-default";

const isForeignKeySpec = (value: unknown): value is PgForeignKeySpec =>
  isRecord(value) &&
  isOptionalName(value.name) &&
  isStringArray(value.columns) &&
  typeof value.referencedSchema === "string" &&
  typeof value.referencedTable === "string" &&
  isStringArray(value.referencedColumns) &&
  isReferentialAction(value.onUpdate) &&
  isReferentialAction(value.onDelete) &&
  typeof value.deferrable === "boolean" &&
  typeof value.initiallyDeferred === "boolean";

const isColumnDraft = (value: unknown): value is TableDesignerColumn =>
  isRecord(value) &&
  typeof value.id === "string" &&
  typeof value.name === "string" &&
  typeof value.dataType === "string" &&
  typeof value.nullable === "boolean" &&
  (value.identity === "none" ||
    value.identity === "always" ||
    value.identity === "by-default") &&
  (value.defaultKind === "none" ||
    value.defaultKind === "literal" ||
    value.defaultKind === "expression") &&
  typeof value.defaultValue === "string" &&
  typeof value.comment === "string";

const isIndexDraft = (value: unknown): value is TableDesignerIndex =>
  isRecord(value) &&
  typeof value.id === "string" &&
  typeof value.name === "string" &&
  isStringArray(value.columns) &&
  typeof value.unique === "boolean" &&
  typeof value.method === "string" &&
  isStringArray(value.include) &&
  typeof value.wherePredicate === "string" &&
  typeof value.concurrently === "boolean";

const hasDraftId = (value: unknown): value is { id: string } =>
  isRecord(value) && typeof value.id === "string";

/** Validate a session-restored designer draft before it enters store state. */
export function validatePersistedTableDesignerDraft(
  value: unknown,
  schema: string,
): TableDesignerForm | undefined {
  if (
    !isRecord(value) ||
    value.schema !== schema ||
    typeof value.name !== "string" ||
    typeof value.comment !== "string" ||
    !Array.isArray(value.columns) ||
    !value.columns.every(isColumnDraft) ||
    !(value.primaryKey === null || isKeySpec(value.primaryKey)) ||
    !Array.isArray(value.uniques) ||
    !value.uniques.every((unique) => isKeySpec(unique) && hasDraftId(unique)) ||
    !Array.isArray(value.checks) ||
    !value.checks.every((check) => isCheckSpec(check) && hasDraftId(check)) ||
    !Array.isArray(value.foreignKeys) ||
    !value.foreignKeys.every(
      (foreignKey) => isForeignKeySpec(foreignKey) && hasDraftId(foreignKey),
    ) ||
    !Array.isArray(value.indexes) ||
    !value.indexes.every(isIndexDraft) ||
    typeof value.unlogged !== "boolean"
  ) {
    return undefined;
  }
  return {
    schema: value.schema,
    name: value.name,
    comment: value.comment,
    columns: value.columns,
    primaryKey: value.primaryKey,
    uniques: value.uniques,
    checks: value.checks,
    foreignKeys: value.foreignKeys,
    indexes: value.indexes,
    unlogged: value.unlogged,
  };
}

const required = (
  errors: Record<string, string>,
  path: string,
  value: string,
  label: string,
) => {
  if (value.trim() === "") errors[path] = `${label} is required.`;
};

const validateDeclaredColumns = (
  errors: Record<string, string>,
  path: string,
  columns: string[],
  declared: Set<string>,
) => {
  if (columns.length === 0) {
    errors[path] = "Choose at least one column.";
    return;
  }
  if (columns.some((column) => column.trim() === "")) {
    errors[path] = "Complete each column name.";
    return;
  }
  const missing = columns.find((column) => !declared.has(column));
  if (missing) errors[path] = `Column ${missing} is not declared.`;
};

type SqlFragmentInspection =
  | { safe: true; topLevelIdentifiers: string[] }
  | { safe: false; reason: "boundary" | "escape" };

const isIdentifierCharacter = (character: string | undefined): boolean =>
  character !== undefined && /[A-Za-z0-9_$]/.test(character);

const scanSingleQuote = (
  fragment: string,
  start: number,
  escapes: boolean,
): number | undefined => {
  let index = start;
  while (index < fragment.length) {
    if (fragment[index] === "'" && fragment[index + 1] === "'") {
      index += 2;
      continue;
    }
    if (fragment[index] === "'") return index + 1;
    if (fragment[index] === "\\" && escapes && index + 1 < fragment.length) {
      index += 2;
      continue;
    }
    index += 1;
  }
  return undefined;
};

/** Split an index expression list only at top-level commas. Incomplete input is
 * kept as one expression so validation can report it without corrupting the
 * user's draft while they type. */
export function splitSqlExpressionList(value: string): string[] {
  if (value.trim() === "") return [];
  const parts: string[] = [];
  const delimiters: Array<"(" | "["> = [];
  let start = 0;
  const finish = (end: number) => {
    const part = value.slice(start, end).trim();
    parts.push(part);
    start = end + 1;
  };

  for (let index = 0; index < value.length; index += 1) {
    const character = value[index];
    const next = value[index + 1];
    if (character === "'") {
      const previous = value[index - 1];
      const escapes =
        (previous === "e" || previous === "E") &&
        !isIdentifierCharacter(value[index - 2]);
      const end = scanSingleQuote(value, index + 1, escapes);
      if (end === undefined) return [value.trim()];
      index = end - 1;
      continue;
    }
    if (character === '"') {
      let closed = false;
      for (index += 1; index < value.length; index += 1) {
        if (value[index] !== '"') continue;
        if (value[index + 1] === '"') {
          index += 1;
          continue;
        }
        closed = true;
        break;
      }
      if (!closed) return [value.trim()];
      continue;
    }
    if (character === "-" && next === "-") {
      const newline = value.indexOf("\n", index + 2);
      if (newline === -1) return [value.trim()];
      index = newline;
      continue;
    }
    if (character === "/" && next === "*") {
      let depth = 1;
      for (index += 2; index < value.length && depth > 0; index += 1) {
        if (value[index] === "/" && value[index + 1] === "*") {
          depth += 1;
          index += 1;
        } else if (value[index] === "*" && value[index + 1] === "/") {
          depth -= 1;
          index += 1;
        }
      }
      if (depth > 0) return [value.trim()];
      index -= 1;
      continue;
    }
    if (character === "$" && !isIdentifierCharacter(value[index - 1])) {
      const tag = value
        .slice(index)
        .match(/^\$[A-Za-z_][A-Za-z0-9_]*\$|^\$\$/)?.[0];
      if (tag) {
        const closing = value.indexOf(tag, index + tag.length);
        if (closing === -1) return [value.trim()];
        index = closing + tag.length - 1;
        continue;
      }
    }
    if (character === "(" || character === "[") {
      delimiters.push(character);
      continue;
    }
    if (character === ")" || character === "]") {
      const expected = character === ")" ? "(" : "[";
      if (delimiters.pop() !== expected) {
        return [value.trim()];
      }
      continue;
    }
    if (character === "," && delimiters.length === 0) finish(index);
  }
  if (delimiters.length > 0) return [value.trim()];
  finish(value.length);
  return parts;
}

/**
 * A deliberately small client-side preflight for renderer-embedded SQL.
 * PostgreSQL and the backend lexer remain authoritative; this only catches
 * delimiter/comment escapes and sibling clauses before a preview request.
 */
const inspectEmbeddedSqlFragment = (
  fragment: string,
): SqlFragmentInspection => {
  const delimiters: Array<"(" | "["> = [];
  const topLevelIdentifiers: string[] = [];
  let previousTokenWasDot = false;

  for (let index = 0; index < fragment.length; index += 1) {
    const character = fragment[index];
    const next = fragment[index + 1];

    if (character === "'") {
      // Plain strings are safe only when standard_conforming_strings cannot
      // change their boundary. This mirrors the backend's shared SQL lexer.
      const literalEnd = scanSingleQuote(fragment, index + 1, false);
      const escapedEnd = scanSingleQuote(fragment, index + 1, true);
      if (literalEnd === undefined || literalEnd !== escapedEnd) {
        return { safe: false, reason: "escape" };
      }
      index = literalEnd - 1;
      previousTokenWasDot = false;
      continue;
    }

    if (character === '"') {
      let closed = false;
      for (index += 1; index < fragment.length; index += 1) {
        if (fragment[index] !== '"') continue;
        if (fragment[index + 1] === '"') {
          index += 1;
          continue;
        }
        closed = true;
        break;
      }
      if (!closed) return { safe: false, reason: "escape" };
      previousTokenWasDot = false;
      continue;
    }

    if (character === "-" && next === "-") {
      const newline = fragment.indexOf("\n", index + 2);
      if (newline === -1) return { safe: false, reason: "escape" };
      index = newline;
      continue;
    }

    if (character === "/" && next === "*") {
      let depth = 1;
      for (index += 2; index < fragment.length && depth > 0; index += 1) {
        if (fragment[index] === "/" && fragment[index + 1] === "*") {
          depth += 1;
          index += 1;
        } else if (fragment[index] === "*" && fragment[index + 1] === "/") {
          depth -= 1;
          index += 1;
        }
      }
      if (depth > 0) return { safe: false, reason: "escape" };
      index -= 1;
      continue;
    }

    if (character === "$" && !isIdentifierCharacter(fragment[index - 1])) {
      const tag = fragment
        .slice(index)
        .match(/^\$[A-Za-z_][A-Za-z0-9_]*\$|^\$\$/)?.[0];
      if (tag) {
        const closing = fragment.indexOf(tag, index + tag.length);
        if (closing === -1) return { safe: false, reason: "escape" };
        index = closing + tag.length - 1;
        previousTokenWasDot = false;
        continue;
      }
    }

    if (character === ";") return { safe: false, reason: "boundary" };
    if (character === "(" || character === "[") {
      delimiters.push(character);
      previousTokenWasDot = false;
      continue;
    }
    if (character === ")" || character === "]") {
      const expected = character === ")" ? "(" : "[";
      if (delimiters.pop() !== expected) {
        return { safe: false, reason: "escape" };
      }
      previousTokenWasDot = false;
      continue;
    }
    if (character === "," && delimiters.length === 0) {
      return { safe: false, reason: "escape" };
    }
    if (character === ".") {
      previousTokenWasDot = true;
      continue;
    }
    if (/[A-Za-z_]/.test(character ?? "")) {
      const start = index;
      while (isIdentifierCharacter(fragment[index + 1])) index += 1;
      const identifier = fragment.slice(start, index + 1);
      if (identifier.toLowerCase() === "e" && fragment[index + 1] === "'") {
        const literalEnd = scanSingleQuote(fragment, index + 2, true);
        if (literalEnd === undefined) {
          return { safe: false, reason: "escape" };
        }
        index = literalEnd - 1;
        previousTokenWasDot = false;
        continue;
      }
      if (delimiters.length === 0 && !previousTokenWasDot) {
        topLevelIdentifiers.push(identifier.toLowerCase());
      }
      previousTokenWasDot = false;
      continue;
    }
    if (!/\s/.test(character ?? "")) previousTokenWasDot = false;
  }

  return delimiters.length === 0
    ? { safe: true, topLevelIdentifiers }
    : { safe: false, reason: "escape" };
};

const COLUMN_OPTION_KEYWORDS = new Set([
  "not",
  "null",
  "default",
  "check",
  "constraint",
  "unique",
  "primary",
  "references",
  "generated",
  "identity",
  "collate",
  "storage",
  "compression",
  "options",
  "encoding",
  "using",
]);

const validateDataTypeFragment = (value: string): string | undefined => {
  const inspection = inspectEmbeddedSqlFragment(value);
  if (!inspection.safe) {
    return inspection.reason === "boundary"
      ? "Data type cannot contain a statement boundary."
      : "Data type escapes its field.";
  }
  return inspection.topLevelIdentifiers.some((identifier) =>
    COLUMN_OPTION_KEYWORDS.has(identifier),
  )
    ? "Data type cannot contain column options."
    : undefined;
};

const validateExpressionFragment = (
  value: string,
  label: string,
): string | undefined => {
  const inspection = inspectEmbeddedSqlFragment(value);
  if (inspection.safe) return undefined;
  return inspection.reason === "boundary"
    ? `${label} cannot contain a statement boundary.`
    : `${label} escapes its field.`;
};

/** Fast local validation; the backend preview remains authoritative. */
export function validateTableDesignerForm(
  form: TableDesignerForm,
): TableDesignerValidation {
  const fields: Record<string, string> = {};
  required(fields, "schema", form.schema, "Schema");
  required(fields, "name", form.name, "Table name");
  if (form.columns.length === 0) {
    fields.columns = "Add at least one column.";
  }
  const declared = new Set<string>();
  form.columns.forEach((column, index) => {
    const prefix = `columns.${index}`;
    required(fields, `${prefix}.name`, column.name, "Column name");
    required(fields, `${prefix}.dataType`, column.dataType, "Data type");
    if (column.dataType.trim() !== "") {
      const dataTypeError = validateDataTypeFragment(column.dataType);
      if (dataTypeError) fields[`${prefix}.dataType`] = dataTypeError;
    }
    if (column.identity === "none" && column.defaultKind === "expression") {
      const path = `${prefix}.defaultValue`;
      required(fields, path, column.defaultValue, "Default expression");
      if (column.defaultValue.trim() !== "") {
        const defaultError = validateExpressionFragment(
          column.defaultValue,
          "Default expression",
        );
        if (defaultError) fields[path] = defaultError;
      }
    }
    const name = column.name.trim();
    if (name !== "" && declared.has(name)) {
      fields[`${prefix}.name`] = `Column ${name} is declared more than once.`;
    }
    declared.add(name);
  });
  if (form.primaryKey) {
    validateDeclaredColumns(
      fields,
      "primaryKey.columns",
      form.primaryKey.columns,
      declared,
    );
  }
  form.uniques.forEach((unique, index) =>
    validateDeclaredColumns(
      fields,
      `uniques.${index}.columns`,
      unique.columns,
      declared,
    ),
  );
  form.checks.forEach((check, index) => {
    const path = `checks.${index}.expression`;
    required(fields, path, check.expression, "Check expression");
    if (check.expression.trim() !== "") {
      const checkError = validateExpressionFragment(
        check.expression,
        "Check expression",
      );
      if (checkError) fields[path] = checkError;
    }
  });
  form.foreignKeys.forEach((foreignKey, index) => {
    const prefix = `foreignKeys.${index}`;
    validateDeclaredColumns(
      fields,
      `${prefix}.columns`,
      foreignKey.columns,
      declared,
    );
    required(
      fields,
      `${prefix}.referencedSchema`,
      foreignKey.referencedSchema,
      "Referenced schema",
    );
    required(
      fields,
      `${prefix}.referencedTable`,
      foreignKey.referencedTable,
      "Referenced table",
    );
    if (foreignKey.referencedColumns.length === 0) {
      fields[`${prefix}.referencedColumns`] =
        "Choose at least one referenced column.";
    } else if (
      foreignKey.referencedColumns.some((column) => column.trim() === "")
    ) {
      fields[`${prefix}.referencedColumns`] =
        "Complete each referenced column name.";
    } else if (
      foreignKey.columns.length !== foreignKey.referencedColumns.length
    ) {
      fields[`${prefix}.referencedColumns`] =
        "Local and referenced column counts must match.";
    }
    if (foreignKey.initiallyDeferred && !foreignKey.deferrable) {
      fields[`${prefix}.deferrable`] =
        "Initially deferred requires a deferrable constraint.";
    }
  });
  form.indexes.forEach((index, position) => {
    const columnsPath = `indexes.${position}.columns`;
    if (index.columns.length === 0) {
      fields[columnsPath] = "Choose at least one index column.";
    } else if (index.columns.some((expression) => expression.trim() === "")) {
      fields[columnsPath] = "Complete each index expression.";
    } else {
      const expressionError = index.columns
        .map((expression) =>
          validateExpressionFragment(expression, "Index expression"),
        )
        .find((error) => error !== undefined);
      if (expressionError) fields[columnsPath] = expressionError;
    }
    required(
      fields,
      `indexes.${position}.method`,
      index.method,
      "Index method",
    );
    if (index.wherePredicate.trim() !== "") {
      const predicateError = validateExpressionFragment(
        index.wherePredicate,
        "Index predicate",
      );
      if (predicateError) {
        fields[`indexes.${position}.wherePredicate`] = predicateError;
      }
    }
  });
  return { valid: Object.keys(fields).length === 0, fields };
}

const defaultValue = (column: TableDesignerColumn): PgDefaultValue | null => {
  if (column.identity !== "none" || column.defaultKind === "none") return null;
  return column.defaultKind === "literal"
    ? { kind: "literal", value: column.defaultValue }
    : { kind: "expression", sql: column.defaultValue };
};

/** Stable mapping used to place a backend invalidOp message near its field. */
export function tableDesignerFieldForOpIndex(
  form: TableDesignerForm,
  opIndex: number,
): string {
  if (opIndex === 0) return "table";
  let cursor = 1;
  if (form.comment.trim() !== "") {
    if (opIndex === cursor) return "comment";
    cursor += 1;
  }
  for (let index = 0; index < form.columns.length; index += 1) {
    if (form.columns[index]?.comment.trim() === "") continue;
    if (opIndex === cursor) return `columns.${index}.comment`;
    cursor += 1;
  }
  const indexPosition = opIndex - cursor;
  return indexPosition >= 0 && indexPosition < form.indexes.length
    ? `indexes.${indexPosition}`
    : "table";
}

/** Pure form-to-operation mapping for the table designer. */
export function buildTableDesignerOps(form: TableDesignerForm): PgObjectOp[] {
  const schema = form.schema.trim();
  const name = form.name.trim();
  const ops: PgObjectOp[] = [
    {
      op: "createTable",
      schema,
      name,
      columns: form.columns.map((column) => ({
        name: column.name.trim(),
        dataType: column.dataType.trim(),
        nullable: column.identity === "none" ? column.nullable : false,
        default: defaultValue(column),
        identity: column.identity === "none" ? null : column.identity,
      })),
      primaryKey: form.primaryKey,
      uniques: form.uniques.map(({ name: constraintName, columns }) => ({
        name: constraintName,
        columns,
      })),
      checks: form.checks.map(({ name: constraintName, expression }) => ({
        name: constraintName,
        expression,
      })),
      foreignKeys: form.foreignKeys.map(
        ({
          name: constraintName,
          columns,
          referencedSchema,
          referencedTable,
          referencedColumns,
          onUpdate,
          onDelete,
          deferrable,
          initiallyDeferred,
        }) => ({
          name: constraintName,
          columns,
          referencedSchema: referencedSchema.trim(),
          referencedTable: referencedTable.trim(),
          referencedColumns,
          onUpdate,
          onDelete,
          deferrable,
          initiallyDeferred,
        }),
      ),
      unlogged: form.unlogged,
      // Conditional creation is unsafe in this multi-op designer: comments
      // and indexes would otherwise mutate an already-existing table.
      ifNotExists: false,
    },
  ];
  if (form.comment.trim() !== "") {
    ops.push({
      op: "setComment",
      target: {
        kind: "object",
        reference: { kind: "table", schema, name, identityArgs: null },
      },
      comment: form.comment.trim(),
    });
  }
  form.columns.forEach((column) => {
    if (column.comment.trim() === "") return;
    ops.push({
      op: "setComment",
      target: {
        kind: "column",
        schema,
        table: name,
        column: column.name.trim(),
      },
      comment: column.comment.trim(),
    });
  });
  form.indexes.forEach((index) =>
    ops.push(
      buildCreateIndexOp({
        schema,
        table: name,
        name: index.name,
        unique: index.unique,
        method: index.method,
        columnExpressions: index.columns,
        include: index.include,
        wherePredicate: index.wherePredicate,
        concurrently: index.concurrently,
      }),
    ),
  );
  return ops;
}

export const newTableDesignerForm = (schema: string): TableDesignerForm => ({
  schema,
  name: "",
  comment: "",
  columns: [
    {
      id: crypto.randomUUID(),
      name: "id",
      dataType: "bigint",
      nullable: false,
      identity: "by-default",
      defaultKind: "none",
      defaultValue: "",
      comment: "",
    },
  ],
  primaryKey: { name: null, columns: ["id"] },
  uniques: [],
  checks: [],
  foreignKeys: [],
  indexes: [],
  unlogged: false,
});
