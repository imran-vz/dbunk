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
import { uiSet } from "@/lib/ui-state";

export const SESSION_STORAGE_KEY = "dbunk.session";
const PERSIST_DEBOUNCE_MS = 500;

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
    uiSet(
      SESSION_STORAGE_KEY,
      JSON.stringify({
        tabs: serializeTabs(state.workspaceTabs),
        activeTabId: state.activeTabId,
        expandedSchemas: state.expandedSchemas,
      }),
    );
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
}

/** Test hook. */
export function resetSessionPersistenceForTests(): void {
  started = false;
}
