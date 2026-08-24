/**
 * Session persistence (P8) — continuously (debounced) mirrors the
 * restorable session state into the UI-state store: open query/table
 * tabs (including hot-exit SQL text, which lives on the tab), tab
 * order and pinning, the active tab, and expanded navigator nodes.
 *
 * Started once after `restoreSession()` so an empty boot state can't
 * clobber a stored session before restore runs.
 */

import { useAppStore, type WorkspaceTab } from "@/lib/store";
import {
  exceedsUtf8Length,
  registerUiStatePreCloseHook,
  UI_STATE_MAX_VALUE_BYTES,
  uiSet,
} from "@/lib/ui-state";

export const SESSION_STORAGE_KEY = "dbunk.session";
const PERSIST_DEBOUNCE_MS = 500;

/**
 * Keep the serialized session comfortably under the backend's per-value
 * limit — an over-limit value is rejected and would leave the whole
 * session unpersisted (typical culprit: a huge pasted SQL script).
 */
const SESSION_BUDGET_BYTES = UI_STATE_MAX_VALUE_BYTES - 64 * 1024;

/** Only query/table tabs restore cleanly; strip runtime-only fields. */
const serializeTabs = (tabs: WorkspaceTab[]) =>
  tabs.flatMap((tab) => {
    if (tab.kind !== "query" && tab.kind !== "table") return [];
    const serialized: Partial<WorkspaceTab> = {
      id: tab.id,
      kind: tab.kind,
      label: tab.label,
      connectionId: tab.connectionId,
      schema: tab.schema,
    };
    if (tab.table !== undefined) serialized.table = tab.table;
    if (tab.query !== undefined) serialized.query = tab.query;
    if (tab.pinned) serialized.pinned = true;
    if (tab.isDirty) serialized.isDirty = true;
    if (tab.kind === "query" && tab.caret !== undefined) {
      serialized.caret = tab.caret;
    }
    return [serialized];
  });

let started = false;

export function startSessionPersistence(): void {
  if (started) return;
  started = true;

  let timer: ReturnType<typeof setTimeout> | null = null;
  let lastTabs = useAppStore.getState().workspaceTabs;
  let lastActive = useAppStore.getState().activeTabId;
  let lastExpanded = useAppStore.getState().expandedSchemas;

  const persist = () => {
    timer = null;
    const state = useAppStore.getState();
    const tabs = serializeTabs(state.workspaceTabs);
    let payload = JSON.stringify({
      tabs,
      activeTabId: state.activeTabId,
      expandedSchemas: state.expandedSchemas,
    });
    // Over budget: shed hot-exit SQL from the largest tabs first so the
    // tab set itself (and every other tab's SQL) still persists.
    if (exceedsUtf8Length(payload, SESSION_BUDGET_BYTES)) {
      const byQuerySize = tabs
        .filter((tab) => tab.query !== undefined)
        .sort((a, b) => (b.query?.length ?? 0) - (a.query?.length ?? 0));
      for (const tab of byQuerySize) {
        console.warn(
          `Session too large to persist; dropping hot-exit SQL for tab ${tab.id}`,
        );
        delete tab.query;
        // The caret and dirty flag describe the shed SQL — restoring
        // them against an empty editor would be an orphaned position
        // and a false "unsaved changes" claim.
        delete tab.caret;
        delete tab.isDirty;
        payload = JSON.stringify({
          tabs,
          activeTabId: state.activeTabId,
          expandedSchemas: state.expandedSchemas,
        });
        if (!exceedsUtf8Length(payload, SESSION_BUDGET_BYTES)) break;
      }
    }
    uiSet(SESSION_STORAGE_KEY, payload);
  };

  useAppStore.subscribe((state) => {
    if (
      state.workspaceTabs === lastTabs &&
      state.activeTabId === lastActive &&
      state.expandedSchemas === lastExpanded
    ) {
      return;
    }
    lastTabs = state.workspaceTabs;
    lastActive = state.activeTabId;
    lastExpanded = state.expandedSchemas;
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(persist, PERSIST_DEBOUNCE_MS);
  });

  // On shutdown the last debounce window (the newest tab/SQL state —
  // the feature's whole point) must reach the queue before the final
  // flush snapshots it.
  registerUiStatePreCloseHook(() => {
    if (timer !== null) {
      clearTimeout(timer);
      persist();
    }
  });
}

/** Test hook. */
export function resetSessionPersistenceForTests(): void {
  started = false;
}
