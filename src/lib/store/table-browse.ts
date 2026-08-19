/**
 * Table Browse slice — owns PostgreSQL server-backed grid state keyed by
 * Workspace Tab id. I/O goes through `@/lib/table-browse-client`; this
 * file owns generation fencing, reset semantics, and prefs bookkeeping.
 */

import type { StateCreator } from "zustand";

import {
  type BrowseCursor,
  type BrowseFilter,
  type BrowsePageRequest,
  type BrowseSortKey,
  DEFAULT_TABLE_BROWSE_PAGE_SIZE,
  defaultTableGridPrefs,
  filtersForRequest,
  parseTableGridPrefs,
  pushHistory,
  type TableBrowseFilterMode,
  type TableBrowseHistoryEntry,
  type TableGridPrefs,
} from "@/lib/table-browse";
import {
  browseTable,
  cancelTableBrowse,
  closeTableBrowseForTab,
  countTableBrowseRows,
  loadTableGridPrefs,
  saveTableGridPrefs,
} from "@/lib/table-browse-client";

import type { AppStoreState, TableBrowseTabState, TableRef } from "./types";

const PREFS_DEBOUNCE_MS = 400;

type SliceSet = Parameters<
  StateCreator<AppStoreState, [], [], TableBrowseSlice>
>[0];
type SliceGet = Parameters<
  StateCreator<AppStoreState, [], [], TableBrowseSlice>
>[1];

export type TableBrowseSlice = {
  tableBrowses: Record<string, TableBrowseTabState>;
  openTableBrowse: (
    tabId: string,
    connectionId: string,
    schema: string,
    table: string,
  ) => Promise<void>;
  refreshTableBrowse: (tabId: string) => Promise<void>;
  refreshTableBrowsesForRelation: (
    connectionId: string,
    schema: string,
    table: string,
  ) => Promise<void>;
  setTableBrowseFilters: (
    tabId: string,
    filters: BrowseFilter[],
  ) => Promise<void>;
  clearTableBrowseFilters: (tabId: string) => Promise<void>;
  setTableBrowseRawFilter: (tabId: string, text: string) => Promise<void>;
  setTableBrowseFilterMode: (
    tabId: string,
    mode: TableBrowseFilterMode,
  ) => Promise<void>;
  setTableBrowseSort: (tabId: string, sort: BrowseSortKey[]) => Promise<void>;
  setTableBrowsePageSize: (tabId: string, pageSize: number) => Promise<void>;
  goToTableBrowsePage: (tabId: string, page: number) => Promise<void>;
  goToTableBrowseNextPage: (tabId: string) => Promise<void>;
  goToTableBrowsePrevPage: (tabId: string) => Promise<void>;
  goToTableBrowseFirstPage: (tabId: string) => Promise<void>;
  goToTableBrowseLastPage: (tabId: string) => Promise<void>;
  countTableBrowseRows: (tabId: string) => Promise<void>;
  cancelTableBrowse: (tabId: string) => Promise<void>;
  applyTableBrowsePreset: (tabId: string, name: string) => Promise<void>;
  saveTableBrowsePreset: (tabId: string, name: string) => Promise<void>;
  applyTableBrowseHistory: (tabId: string, index: number) => Promise<void>;
  closeTableBrowseForTab: (tabId: string) => Promise<void>;
  closeTableBrowsesForConnection: (connectionId: string) => Promise<void>;
};

const prefsTimers = new Map<string, ReturnType<typeof setTimeout>>();

const prefsKey = (connectionId: string, schema: string, table: string) =>
  `${connectionId}::${schema}::${table}`;

const initialTab = (
  tabId: string,
  connectionId: string,
  schema: string,
  table: string,
  generation: number,
): TableBrowseTabState => ({
  tabId,
  connectionId,
  schema,
  table,
  generation,
  typedFilters: [],
  rawFilterText: "",
  filterMode: "typed",
  sort: [],
  pageSize: DEFAULT_TABLE_BROWSE_PAGE_SIZE,
  page: 1,
  cursorStack: [],
  inflightRequestId: null,
  appliedRequestId: null,
  result: null,
  loadStatus: { state: "idle" },
  countStatus: { state: "idle" },
  exactCount: null,
  prefsLoaded: false,
  prefs: defaultTableGridPrefs(),
});

const patchBrowse = (
  set: SliceSet,
  tabId: string,
  patch: Partial<TableBrowseTabState>,
) =>
  set((state) => {
    const current = state.tableBrowses[tabId];
    return current
      ? {
          tableBrowses: {
            ...state.tableBrowses,
            [tabId]: { ...current, ...patch },
          },
        }
      : {};
  });

const historySnapshot = (
  tab: TableBrowseTabState,
): TableBrowseHistoryEntry => ({
  appliedAt: new Date().toISOString(),
  typedFilters: tab.typedFilters,
  rawFilterText: tab.rawFilterText,
  filterMode: tab.filterMode,
  sort: tab.sort,
});

const firstPageRequest = (sort: BrowseSortKey[]): BrowsePageRequest =>
  sort.length === 0
    ? { kind: "keyset", cursor: null }
    : { kind: "offset", page: 1 };

const schedulePrefsSave = (get: SliceGet, tab: TableBrowseTabState) => {
  const key = prefsKey(tab.connectionId, tab.schema, tab.table);
  const existing = prefsTimers.get(key);
  if (existing) clearTimeout(existing);
  const timer = setTimeout(() => {
    prefsTimers.delete(key);
    const latest = Object.values(get().tableBrowses).find(
      (item) =>
        item.connectionId === tab.connectionId &&
        item.schema === tab.schema &&
        item.table === tab.table,
    );
    if (!latest?.prefsLoaded) return;
    const prefs: TableGridPrefs = {
      ...latest.prefs,
      pageSize: latest.pageSize,
      sort: latest.sort,
      typedFilters: latest.typedFilters,
      rawFilterText: latest.rawFilterText,
      filterMode: latest.filterMode,
    };
    void saveTableGridPrefs(
      latest.connectionId,
      latest.schema,
      latest.table,
      prefs,
    ).catch(() => undefined);
  }, PREFS_DEBOUNCE_MS);
  prefsTimers.set(key, timer);
};

const clearEditsForTab = (get: SliceGet, tab: TableBrowseTabState) => {
  get().discardTableCellEdits({
    connectionId: tab.connectionId,
    schema: tab.schema,
    table: tab.table,
  });
};

const applyGridState = (
  set: SliceSet,
  tabId: string,
  patch: Partial<
    Pick<
      TableBrowseTabState,
      "typedFilters" | "rawFilterText" | "filterMode" | "sort" | "pageSize"
    >
  >,
) => {
  patchBrowse(set, tabId, {
    ...patch,
    page: 1,
    cursorStack: [],
    exactCount: null,
  });
};

const runBrowse = async (
  set: SliceSet,
  get: SliceGet,
  tabId: string,
  pageRequest: BrowsePageRequest,
  options: {
    recordHistory: boolean;
    page: number;
    cursorStack: Array<BrowseCursor | null>;
  },
) => {
  const tab = get().tableBrowses[tabId];
  if (!tab) return;
  const generation = tab.generation;
  const pendingId = (tab.inflightRequestId ?? 0) + 1;
  patchBrowse(set, tabId, {
    loadStatus: { state: "loading" },
    inflightRequestId: pendingId,
    page: options.page,
    cursorStack: options.cursorStack,
  });
  const result = await browseTable({
    connectionId: tab.connectionId,
    tabId,
    schema: tab.schema,
    table: tab.table,
    filters: filtersForRequest(tab.typedFilters, tab.rawFilterText),
    sort: tab.sort,
    pageRequest,
    pageSize: tab.pageSize,
    countPolicy: "estimated",
    refreshStructure: false,
  });
  const current = get().tableBrowses[tabId];
  if (!current || current.generation !== generation) return;
  if (result.kind === "superseded") {
    if (current.inflightRequestId === pendingId) {
      patchBrowse(set, tabId, {
        loadStatus: current.result ? { state: "success" } : { state: "idle" },
        inflightRequestId: null,
      });
    }
    return;
  }
  if (current.inflightRequestId !== pendingId) return;
  if (result.kind === "cancelled") {
    patchBrowse(set, tabId, {
      loadStatus: { state: "idle" },
      inflightRequestId: null,
    });
    return;
  }
  if (result.kind === "error") {
    if (result.error.kind === "invalidCursor") {
      await runBrowse(set, get, tabId, firstPageRequest(current.sort), {
        recordHistory: false,
        page: 1,
        cursorStack: [],
      });
      return;
    }
    patchBrowse(set, tabId, {
      loadStatus: { state: "error", error: result.error },
      inflightRequestId: null,
    });
    return;
  }
  clearEditsForTab(get, current);
  const prefs = options.recordHistory
    ? {
        ...current.prefs,
        filterHistory: pushHistory(
          current.prefs.filterHistory,
          historySnapshot(current),
        ),
        sortHistory: pushHistory(
          current.prefs.sortHistory,
          historySnapshot(current),
        ),
        pageSize: current.pageSize,
        sort: current.sort,
        typedFilters: current.typedFilters,
        rawFilterText: current.rawFilterText,
        filterMode: current.filterMode,
      }
    : { ...current.prefs, pageSize: current.pageSize };
  patchBrowse(set, tabId, {
    result: result.value,
    appliedRequestId: result.value.requestId,
    inflightRequestId: null,
    loadStatus: { state: "success" },
    prefs,
    page: result.value.pageInfo.page ?? options.page,
  });
  if (options.recordHistory || prefs.pageSize !== current.prefs.pageSize) {
    schedulePrefsSave(get, { ...current, prefs });
  }
};

const ensurePrefs = async (set: SliceSet, get: SliceGet, tabId: string) => {
  const tab = get().tableBrowses[tabId];
  if (!tab || tab.prefsLoaded) return;
  try {
    const loaded = await loadTableGridPrefs(
      tab.connectionId,
      tab.schema,
      tab.table,
    );
    const current = get().tableBrowses[tabId];
    if (!current || current.prefsLoaded) return;
    const prefs = parseTableGridPrefs(loaded);
    patchBrowse(set, tabId, {
      prefsLoaded: true,
      prefs,
      pageSize: prefs.pageSize,
      sort: prefs.sort,
      typedFilters: prefs.typedFilters,
      rawFilterText: prefs.rawFilterText,
      filterMode: prefs.filterMode,
    });
  } catch {
    const current = get().tableBrowses[tabId];
    if (!current || current.prefsLoaded) return;
    patchBrowse(set, tabId, { prefsLoaded: true });
  }
};

export const createTableBrowseSlice: StateCreator<
  AppStoreState,
  [],
  [],
  TableBrowseSlice
> = (set, get) => ({
  tableBrowses: {},

  openTableBrowse: async (tabId, connectionId, schema, table) => {
    const existing = get().tableBrowses[tabId];
    const sameTarget =
      existing &&
      existing.connectionId === connectionId &&
      existing.schema === schema &&
      existing.table === table;
    if (!sameTarget) {
      const generation = (existing?.generation ?? 0) + 1;
      set((state) => ({
        tableBrowses: {
          ...state.tableBrowses,
          [tabId]: initialTab(tabId, connectionId, schema, table, generation),
        },
      }));
      if (existing) {
        get().discardTableCellEdits({
          connectionId: existing.connectionId,
          schema: existing.schema,
          table: existing.table,
        });
      }
    }
    await ensurePrefs(set, get, tabId);
    const tab = get().tableBrowses[tabId];
    if (!tab) return;
    if (sameTarget && tab.result && tab.loadStatus.state === "success") return;
    await runBrowse(set, get, tabId, firstPageRequest(tab.sort), {
      recordHistory: false,
      page: 1,
      cursorStack: [],
    });
  },

  refreshTableBrowse: async (tabId) => {
    const tab = get().tableBrowses[tabId];
    if (!tab) return;
    await runBrowse(set, get, tabId, firstPageRequest(tab.sort), {
      recordHistory: false,
      page: 1,
      cursorStack: [],
    });
  },

  refreshTableBrowsesForRelation: async (connectionId, schema, table) => {
    const tabIds = Object.values(get().tableBrowses)
      .filter(
        (tab) =>
          tab.connectionId === connectionId &&
          tab.schema === schema &&
          tab.table === table,
      )
      .map((tab) => tab.tabId);
    await Promise.all(tabIds.map((tabId) => get().refreshTableBrowse(tabId)));
  },

  setTableBrowseFilters: async (tabId, filters) => {
    applyGridState(set, tabId, { typedFilters: filters });
    const tab = get().tableBrowses[tabId];
    if (!tab) return;
    await runBrowse(set, get, tabId, firstPageRequest(tab.sort), {
      recordHistory: true,
      page: 1,
      cursorStack: [],
    });
  },

  clearTableBrowseFilters: async (tabId) => {
    applyGridState(set, tabId, { typedFilters: [], rawFilterText: "" });
    const tab = get().tableBrowses[tabId];
    if (!tab) return;
    await runBrowse(set, get, tabId, firstPageRequest(tab.sort), {
      recordHistory: true,
      page: 1,
      cursorStack: [],
    });
  },

  setTableBrowseRawFilter: async (tabId, text) => {
    applyGridState(set, tabId, { rawFilterText: text });
    const tab = get().tableBrowses[tabId];
    if (!tab) return;
    await runBrowse(set, get, tabId, firstPageRequest(tab.sort), {
      recordHistory: true,
      page: 1,
      cursorStack: [],
    });
  },

  setTableBrowseFilterMode: async (tabId, mode) => {
    patchBrowse(set, tabId, { filterMode: mode });
    const tab = get().tableBrowses[tabId];
    if (!tab) return;
    schedulePrefsSave(get, { ...tab, filterMode: mode });
  },

  setTableBrowseSort: async (tabId, sort) => {
    applyGridState(set, tabId, { sort });
    await runBrowse(set, get, tabId, firstPageRequest(sort), {
      recordHistory: true,
      page: 1,
      cursorStack: [],
    });
  },

  setTableBrowsePageSize: async (tabId, pageSize) => {
    applyGridState(set, tabId, { pageSize });
    const tab = get().tableBrowses[tabId];
    if (!tab) return;
    await runBrowse(set, get, tabId, firstPageRequest(tab.sort), {
      recordHistory: false,
      page: 1,
      cursorStack: [],
    });
  },

  goToTableBrowsePage: async (tabId, page) => {
    const tab = get().tableBrowses[tabId];
    if (!tab) return;
    const target = Math.max(1, page);
    if (target === tab.page) return;
    await runBrowse(
      set,
      get,
      tabId,
      { kind: "offset", page: target },
      {
        recordHistory: false,
        page: target,
        cursorStack: [],
      },
    );
  },

  goToTableBrowseNextPage: async (tabId) => {
    const tab = get().tableBrowses[tabId];
    if (!tab?.result?.pageInfo.hasMore) return;
    const nextPage = tab.page + 1;
    const nextCursor = tab.result.pageInfo.nextCursor;
    if (tab.result.pageInfo.mode === "keyset" && nextCursor) {
      await runBrowse(
        set,
        get,
        tabId,
        { kind: "keyset", cursor: nextCursor },
        {
          recordHistory: false,
          page: nextPage,
          cursorStack: [...tab.cursorStack, nextCursor],
        },
      );
      return;
    }
    await runBrowse(
      set,
      get,
      tabId,
      { kind: "offset", page: nextPage },
      {
        recordHistory: false,
        page: nextPage,
        cursorStack: [],
      },
    );
  },

  goToTableBrowsePrevPage: async (tabId) => {
    const tab = get().tableBrowses[tabId];
    if (!tab || tab.page <= 1) return;
    const nextPage = tab.page - 1;
    if (tab.cursorStack.length > 0) {
      const stack = tab.cursorStack.slice(0, -1);
      const cursor = stack[stack.length - 1] ?? null;
      const pageRequest: BrowsePageRequest =
        cursor === null
          ? firstPageRequest(tab.sort)
          : { kind: "keyset", cursor };
      await runBrowse(set, get, tabId, pageRequest, {
        recordHistory: false,
        page: nextPage,
        cursorStack: stack,
      });
      return;
    }
    await runBrowse(
      set,
      get,
      tabId,
      { kind: "offset", page: nextPage },
      {
        recordHistory: false,
        page: nextPage,
        cursorStack: [],
      },
    );
  },

  goToTableBrowseFirstPage: async (tabId) => {
    const tab = get().tableBrowses[tabId];
    if (!tab) return;
    await runBrowse(set, get, tabId, firstPageRequest(tab.sort), {
      recordHistory: false,
      page: 1,
      cursorStack: [],
    });
  },

  goToTableBrowseLastPage: async (tabId) => {
    const tab = get().tableBrowses[tabId];
    if (!tab) return;
    const count = tab.exactCount?.value ?? tab.result?.count.value;
    const kind = tab.exactCount?.kind ?? tab.result?.count.kind;
    if (count === null || count === undefined || kind === "unknown") return;
    const totalPages = Math.max(1, Math.ceil(count / tab.pageSize));
    await runBrowse(
      set,
      get,
      tabId,
      { kind: "offset", page: totalPages },
      {
        recordHistory: false,
        page: totalPages,
        cursorStack: [],
      },
    );
  },

  countTableBrowseRows: async (tabId) => {
    const tab = get().tableBrowses[tabId];
    if (!tab) return;
    const generation = tab.generation;
    patchBrowse(set, tabId, { countStatus: { state: "loading" } });
    const result = await countTableBrowseRows({
      connectionId: tab.connectionId,
      tabId,
      schema: tab.schema,
      table: tab.table,
      filters: filtersForRequest(tab.typedFilters, tab.rawFilterText),
    });
    const current = get().tableBrowses[tabId];
    if (!current || current.generation !== generation) return;
    if (result.kind === "superseded") {
      patchBrowse(set, tabId, { countStatus: { state: "idle" } });
      return;
    }
    if (result.kind === "cancelled") {
      patchBrowse(set, tabId, { countStatus: { state: "idle" } });
      return;
    }
    if (result.kind === "error") {
      patchBrowse(set, tabId, {
        countStatus: { state: "error", error: result.error },
      });
      return;
    }
    patchBrowse(set, tabId, {
      exactCount: result.value,
      countStatus: { state: "success" },
    });
  },

  cancelTableBrowse: async (tabId) => {
    const tab = get().tableBrowses[tabId];
    if (!tab) return;
    await cancelTableBrowse(tab.connectionId, tabId);
  },

  applyTableBrowsePreset: async (tabId, name) => {
    const tab = get().tableBrowses[tabId];
    const preset = tab?.prefs.presets.find((item) => item.name === name);
    if (!tab || !preset) return;
    applyGridState(set, tabId, {
      typedFilters: preset.typedFilters,
      rawFilterText: preset.rawFilterText,
      filterMode: preset.filterMode,
      sort: preset.sort,
      pageSize: preset.pageSize,
    });
    await runBrowse(set, get, tabId, firstPageRequest(preset.sort), {
      recordHistory: false,
      page: 1,
      cursorStack: [],
    });
  },

  saveTableBrowsePreset: async (tabId, name) => {
    const tab = get().tableBrowses[tabId];
    if (!tab || name.trim().length === 0) return;
    const preset = {
      name: name.trim(),
      typedFilters: tab.typedFilters,
      rawFilterText: tab.rawFilterText,
      filterMode: tab.filterMode,
      sort: tab.sort,
      pageSize: tab.pageSize,
    };
    const presets = [
      preset,
      ...tab.prefs.presets.filter((item) => item.name !== preset.name),
    ];
    const prefs = { ...tab.prefs, presets };
    patchBrowse(set, tabId, { prefs });
    schedulePrefsSave(get, { ...tab, prefs });
  },

  applyTableBrowseHistory: async (tabId, index) => {
    const tab = get().tableBrowses[tabId];
    const entry = tab?.prefs.filterHistory[index];
    if (!tab || !entry) return;
    applyGridState(set, tabId, {
      typedFilters: entry.typedFilters,
      rawFilterText: entry.rawFilterText,
      filterMode: entry.filterMode,
      sort: entry.sort,
    });
    await runBrowse(set, get, tabId, firstPageRequest(entry.sort), {
      recordHistory: false,
      page: 1,
      cursorStack: [],
    });
  },

  closeTableBrowseForTab: async (tabId) => {
    const tab = get().tableBrowses[tabId];
    if (tab) {
      await closeTableBrowseForTab(tab.connectionId, tabId).catch(
        () => undefined,
      );
    }
    set((state) => {
      const { [tabId]: _dropped, ...tableBrowses } = state.tableBrowses;
      return { tableBrowses };
    });
  },

  closeTableBrowsesForConnection: async (connectionId) => {
    const tabIds = Object.values(get().tableBrowses)
      .filter((tab) => tab.connectionId === connectionId)
      .map((tab) => tab.tabId);
    await Promise.all(
      tabIds.map((tabId) => get().closeTableBrowseForTab(tabId)),
    );
  },
});

export const tableBrowseRef = (tab: TableBrowseTabState): TableRef => ({
  connectionId: tab.connectionId,
  schema: tab.schema,
  table: tab.table,
});
