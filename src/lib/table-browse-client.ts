/* oxlint-disable anti-slop/no-unknown-parameters -- Invoke rejections are decoded at this command boundary. */
import {
  type BrowseExactCountResult,
  type BrowseTableDataPayload,
  type BrowseTableResult,
  type CancelTableBrowseResult,
  type CountTableBrowseRowsPayload,
  type TableBrowseError,
  type TableGridPrefs,
} from "@/lib/table-browse";
import { decodeTableBrowseError } from "@/lib/table-browse-error";
import { isTauri, tauriInvoke } from "@/lib/tauri";

export type TableBrowseClientResult<T> =
  | { kind: "ok"; value: T }
  | { kind: "superseded" }
  | { kind: "cancelled" }
  | { kind: "error"; error: TableBrowseError };

type TabRequestState = {
  latestIssued: number;
};

const tabs = new Map<string, TabRequestState>();

const tabState = (tabId: string): TabRequestState => {
  const existing = tabs.get(tabId);
  if (existing) return existing;
  const created = { latestIssued: 0 };
  tabs.set(tabId, created);
  return created;
};

const issueRequestId = (tabId: string): number => {
  const state = tabState(tabId);
  state.latestIssued += 1;
  return state.latestIssued;
};

const isStale = (tabId: string, requestId: number): boolean =>
  requestId < tabState(tabId).latestIssued;

const toResult = <T extends { requestId: number }>(
  tabId: string,
  value: T,
): TableBrowseClientResult<T> => {
  if (isStale(tabId, value.requestId)) return { kind: "superseded" };
  return { kind: "ok", value };
};

const fromError = (error: unknown): TableBrowseClientResult<never> => {
  const decoded = decodeTableBrowseError(error);
  if (decoded.kind === "superseded" || decoded.kind === "cancelled") {
    return { kind: decoded.kind };
  }
  return { kind: "error", error: decoded };
};

export const resetTableBrowseClientForTab = (tabId: string): void => {
  tabs.delete(tabId);
};

export async function browseTable(
  payload: Omit<BrowseTableDataPayload, "requestId">,
): Promise<TableBrowseClientResult<BrowseTableResult>> {
  if (!isTauri()) {
    return { kind: "error", error: { kind: "connectionLost" } };
  }
  const requestId = issueRequestId(payload.tabId);
  try {
    const value = await tauriInvoke<BrowseTableResult>("browse_table_data", {
      payload: { ...payload, requestId },
    });
    return toResult(payload.tabId, value);
  } catch (error) {
    if (isStale(payload.tabId, requestId)) return { kind: "superseded" };
    return fromError(error);
  }
}

export async function countTableBrowseRows(
  payload: Omit<CountTableBrowseRowsPayload, "requestId">,
): Promise<TableBrowseClientResult<BrowseExactCountResult>> {
  if (!isTauri()) {
    return { kind: "error", error: { kind: "connectionLost" } };
  }
  const requestId = issueRequestId(payload.tabId);
  try {
    const value = await tauriInvoke<BrowseExactCountResult>(
      "count_table_browse_rows",
      { payload: { ...payload, requestId } },
    );
    return toResult(payload.tabId, value);
  } catch (error) {
    if (isStale(payload.tabId, requestId)) return { kind: "superseded" };
    return fromError(error);
  }
}

export async function cancelTableBrowse(
  connectionId: string,
  tabId: string,
): Promise<CancelTableBrowseResult> {
  if (!isTauri()) return { cancelRequested: false };
  try {
    return await tauriInvoke<CancelTableBrowseResult>("cancel_table_browse", {
      payload: { connectionId, tabId },
    });
  } catch {
    return { cancelRequested: false };
  }
}

export async function closeTableBrowseForTab(
  connectionId: string,
  tabId: string,
): Promise<void> {
  resetTableBrowseClientForTab(tabId);
  if (!isTauri()) return;
  try {
    await tauriInvoke("close_table_browse_for_tab", {
      payload: { connectionId, tabId },
    });
  } catch {
    return;
  }
}

export async function loadTableGridPrefs(
  connectionId: string,
  schema: string,
  table: string,
): Promise<TableGridPrefs | null> {
  if (!isTauri()) return null;
  return tauriInvoke<TableGridPrefs | null>("load_table_grid_prefs", {
    payload: { connectionId, schema, table },
  });
}

export async function saveTableGridPrefs(
  connectionId: string,
  schema: string,
  table: string,
  prefs: TableGridPrefs,
): Promise<void> {
  if (!isTauri()) return;
  await tauriInvoke("save_table_grid_prefs", {
    payload: { connectionId, schema, table, prefs },
  });
}
