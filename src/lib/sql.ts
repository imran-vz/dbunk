/**
 * Decides which SQL string to run when the user clicks Run in the query editor.
 *
 * If a non-whitespace selection is provided, the selection wins (preserving its
 * internal whitespace verbatim, since SQL semantics may rely on it). Otherwise
 * we fall back to the full editor contents.
 */
export function pickSqlToRun(
  fullText: string,
  selection: string | null,
): string {
  if (selection !== null && selection.trim().length > 0) {
    return selection;
  }
  return fullText;
}

export type SqlStatementRange = {
  sql: string;
  startOffset: number;
  endOffset: number;
  startLine: number;
  endLine: number;
};

type SqlScannerState = {
  quote: "'" | '"' | "`" | null;
  lineComment: boolean;
  blockComment: boolean;
};

const initialScannerState = (): SqlScannerState => ({
  quote: null,
  lineComment: false,
  blockComment: false,
});

const advanceScanner = (
  text: string,
  index: number,
  state: SqlScannerState,
): SqlScannerState => {
  const char = text[index];
  const next = text[index + 1];

  if (state.lineComment) {
    return char === "\n" ? { ...state, lineComment: false } : state;
  }

  if (state.blockComment) {
    return char === "*" && next === "/"
      ? { ...state, blockComment: false }
      : state;
  }

  if (state.quote) {
    if (char === state.quote) {
      if (text[index + 1] === state.quote) {
        return state;
      }
      return { ...state, quote: null };
    }
    return state;
  }

  if (char === "-" && next === "-") {
    return { ...state, lineComment: true };
  }
  if (char === "/" && next === "*") {
    return { ...state, blockComment: true };
  }
  if (char === "'" || char === '"' || char === "`") {
    return { ...state, quote: char };
  }
  return state;
};

const lineNumberAtOffset = (text: string, offset: number): number =>
  text.slice(0, offset).split("\n").length;

const trimStatementRange = (
  text: string,
  startOffset: number,
  endOffset: number,
): SqlStatementRange | null => {
  let start = startOffset;
  let end = endOffset;
  while (start < end && /\s/.test(text[start] ?? "")) {
    start += 1;
  }
  while (end > start && /[\s;]/.test(text[end - 1] ?? "")) {
    end -= 1;
  }
  const sql = text.slice(start, end);
  if (!sql.trim()) {
    return null;
  }
  return {
    sql,
    startOffset: start,
    endOffset: end,
    startLine: lineNumberAtOffset(text, start),
    endLine: lineNumberAtOffset(text, end),
  };
};

export const getSqlStatements = (text: string): SqlStatementRange[] => {
  const statements: SqlStatementRange[] = [];
  let state = initialScannerState();
  let statementStart = 0;

  for (let index = 0; index < text.length; index += 1) {
    const nextState = advanceScanner(text, index, state);
    if (
      text[index] === ";" &&
      !state.quote &&
      !state.lineComment &&
      !state.blockComment
    ) {
      const statement = trimStatementRange(text, statementStart, index + 1);
      if (statement) {
        statements.push(statement);
      }
      statementStart = index + 1;
    }
    state = nextState;
  }

  const finalStatement = trimStatementRange(text, statementStart, text.length);
  if (finalStatement) {
    statements.push(finalStatement);
  }

  return statements;
};

export const positionToOffset = (
  text: string,
  lineNumber: number,
  column: number,
): number => {
  const lines = text.split("\n");
  let offset = 0;
  for (let index = 0; index < Math.max(0, lineNumber - 1); index += 1) {
    offset += (lines[index] ?? "").length + 1;
  }
  return Math.min(text.length, offset + Math.max(0, column - 1));
};

export const getSqlStatementAtPosition = (
  text: string,
  lineNumber: number,
  column: number,
): SqlStatementRange | null => {
  const offset = positionToOffset(text, lineNumber, column);
  return (
    getSqlStatements(text).find(
      (statement) =>
        offset >= statement.startOffset && offset <= statement.endOffset,
    ) ?? null
  );
};
