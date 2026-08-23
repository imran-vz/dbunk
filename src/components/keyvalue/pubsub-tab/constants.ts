/**
 * Lifecycle constants for the Redis Pub/Sub workspace tab.
 *
 * Extracted alongside `PubsubTab` so the sibling hooks/components can
 * share them without re-importing the parent shell. Sidebar geometry
 * moved to the `Panel` primitive (`usePanelState`).
 */

export const MAX_BUFFER = 10_000;
