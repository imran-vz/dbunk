/**
 * Shared helpers for the store's lifecycle-status slots — the
 * per-key `Record<string, { state: "running" | "queued" }>` maps
 * that hold the in-flight state of async store actions. Terminal
 * results are returned as `EditOutcome` / `DDLOutcome` /
 * `QueryOutcome` from the action; the lifecycle slots exist only
 * to drive UI affordances (disabled buttons, "Running…" labels)
 * across tab unmounts.
 *
 * Lives in `src/lib/` (not `src/lib/store/`) because it is shared
 * across slices. The `src/lib/store/README.md` convention reserves
 * the `store/` directory for slice-local code only; shared helpers
 * lift out alongside `format.ts`, `tauri.ts`, and friends.
 */

import type { AppStoreState } from "./store/types";

/**
 * Names of the per-key lifecycle-status maps on `AppStoreState`.
 * Each holds `Record<string, { state: "running" } | { state:
 * "queued"; ... }>`. The hand-rolled union (rather than a derived
 * mapped type) is deliberate — `AppStoreState` also has Record-
 * shaped load-status maps (`tableLoadStatus`,
 * `tableStructureStatus`, etc.) that carry their own `state`
 * field, and a structural derivation would pick those up as false
 * positives. Adding a fourth lifecycle map means adding its key
 * here.
 */
export type LifecycleSlot =
  | "tableEditsCommitStatus"
  | "structureCommitStatus"
  | "queryStatus";

/**
 * Drop one entry from a per-key lifecycle-status map. Bails when
 * the key is absent so the inner-map spread is avoided.
 *
 * Note: the bail returns `{}` from the updater, which Zustand still
 * merges (`Object.assign({}, state, {})`) — every subscriber is
 * notified, but properly-sliced selectors return the same reference
 * and skip their re-render via the equality check. Whole-store
 * identity subscribers will re-render regardless; the bail saves
 * the allocation of a new map for the slot, not the notification.
 */
export const clearLifecycleSlot = (
  set: (fn: (state: AppStoreState) => Partial<AppStoreState>) => void,
  slot: LifecycleSlot,
  subkey: string,
) => {
  set((state) => {
    const current = state[slot] as Record<string, unknown>;
    if (!(subkey in current)) return {};
    const { [subkey]: _dropped, ...rest } = current;
    return { [slot]: rest } as Partial<AppStoreState>;
  });
};
