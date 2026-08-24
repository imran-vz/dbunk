/**
 * Closed vocabulary for the user-picked connection color (Plan 009,
 * PAR-005). The backend stores the value as an opaque string; this
 * module is the single frontend authority on what counts as a color.
 * The token → CSS mapping ships with the Plan 010 UI activation so the
 * palette stays theme-token-driven (no raw hex).
 */

export const CONNECTION_COLORS = [
  "red",
  "orange",
  "amber",
  "green",
  "teal",
  "blue",
  "violet",
  "pink",
  "gray",
] as const;

export type ConnectionColor = (typeof CONNECTION_COLORS)[number];

/** Narrow an opaque stored string; unknown values mean "no color". */
export function isConnectionColor(
  value: string | undefined,
): value is ConnectionColor {
  return CONNECTION_COLORS.some((color) => color === value);
}

/**
 * Token → CSS variable. The variables are defined once in
 * `styles.css` `:root` (theme-invariant identity colors); components
 * consume them via inline `style` so no raw hex ever lands in TSX.
 */
export function connectionColorVar(color: ConnectionColor): string {
  return `var(--connection-color-${color})`;
}
