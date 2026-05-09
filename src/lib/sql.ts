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
