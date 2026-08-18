/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters -- Latency is display-boundary input and this helper validates it before formatting. */
/**
 * Pure display-formatting helpers. App-wide, no domain coupling.
 *
 * Grew out of the workspace-store god-store slicing — `formatLatencyMs`
 * was buried in store.ts even though its only dependencies are
 * `number → string`. Lives here so any UI component or store slice can
 * import it without pulling the full store surface.
 */

/**
 * `42` → `"42 ms"`, anything non-finite → `"--"`.
 *
 * Used by Connection.latency display in cards, lists, and status bars.
 * The `"--"` fallback is the design convention for "no measurement
 * yet" — preserved from the pre-slicing implementation.
 */
export function formatLatencyMs(latencyMs: unknown): string {
  return typeof latencyMs === "number" && Number.isFinite(latencyMs)
    ? `${latencyMs} ms`
    : "--";
}
