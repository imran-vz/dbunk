import {
  type MutationDraftScope,
  useAppStore,
  isConnectedStatus,
  tableStructureKey,
} from "@/lib/store";

import type { PgToolJob } from "./client";

/** Completion is irreversible; metadata refresh failures never relabel job success. */
export async function refreshAfterPgRestore(job: PgToolJob): Promise<void> {
  const connectionId = job.connectionId;
  const state = useAppStore.getState();
  const browsesToRefresh = Object.values(state.tableBrowses)
    .filter((browse) => browse.connectionId === connectionId)
    .map((browse) => ({
      tabId: browse.tabId,
      connectionId: browse.connectionId,
      schema: browse.schema,
      table: browse.table,
      generation: browse.generation + 1,
      refreshExactCount:
        browse.exactCount !== null || browse.countStatus.state !== "idle",
    }));
  state.markPgObjectDdlApplied();
  state.dropPgObjectCachesForConnection(connectionId);
  const prefix = `${connectionId}::`;
  useAppStore.setState((current) => ({
    relationStats: Object.fromEntries(
      Object.entries(current.relationStats).filter(
        ([id]) => id !== connectionId,
      ),
    ),
    relationStatsStatus: Object.fromEntries(
      Object.entries(current.relationStatsStatus).filter(
        ([id]) => id !== connectionId,
      ),
    ),
    databaseOverviewStats: Object.fromEntries(
      Object.entries(current.databaseOverviewStats).filter(
        ([id]) => id !== connectionId,
      ),
    ),
    databaseOverviewStatsStatus: Object.fromEntries(
      Object.entries(current.databaseOverviewStatsStatus).filter(
        ([id]) => id !== connectionId,
      ),
    ),
    tableStructure: Object.fromEntries(
      Object.entries(current.tableStructure).filter(
        ([key]) => !key.startsWith(prefix),
      ),
    ),
    schemaRelationships: Object.fromEntries(
      Object.entries(current.schemaRelationships).filter(
        ([key]) => !key.startsWith(prefix),
      ),
    ),
    tableStructureStatus: Object.fromEntries(
      Object.entries(current.tableStructureStatus).filter(
        ([key]) => !key.startsWith(prefix),
      ),
    ),
    schemaRelationshipsStatus: Object.fromEntries(
      Object.entries(current.schemaRelationshipsStatus).filter(
        ([key]) => !key.startsWith(prefix),
      ),
    ),
    tableBrowses: Object.fromEntries(
      Object.entries(current.tableBrowses).map(([tabId, browse]) => [
        tabId,
        browse.connectionId === connectionId
          ? {
              ...browse,
              generation: browse.generation + 1,
              inflightRequestId: null,
              exactCount: null,
              countStatus: { state: "idle" as const },
            }
          : browse,
      ]),
    ),
    ...invalidateMutationDraftSources(current, connectionId),
  }));
  const connection = useAppStore
    .getState()
    .connections.find((c) => c.id === connectionId);
  if (!connection || !isConnectedStatus(connection.status)) return;
  const latest = useAppStore.getState();
  const refreshes = [
    latest.loadPgObjectCatalog(connectionId).then(() => {}),
    latest.loadRelationStats(connectionId),
    latest.loadDatabaseOverviewStats(connectionId),
  ];
  for (const browse of browsesToRefresh) {
    refreshes.push(
      latest.refreshTableBrowse(browse.tabId).then(async () => {
        const refreshedState = useAppStore.getState();
        const currentBrowse = refreshedState.tableBrowses[browse.tabId];
        if (!browseStillMatches(currentBrowse, browse)) return;
        if (currentBrowse.loadStatus.state === "error") {
          throw new Error("Browse refresh failed");
        }
        if (
          currentBrowse.loadStatus.state !== "success" ||
          currentBrowse.inflightRequestId !== null
        ) {
          return;
        }
        const currentConnection = refreshedState.connections.find(
          (candidate) => candidate.id === connectionId,
        );
        if (
          browse.refreshExactCount &&
          isConnectedStatus(currentConnection?.status)
        ) {
          await refreshedState.countTableBrowseRows(browse.tabId);
          const countedBrowse =
            useAppStore.getState().tableBrowses[browse.tabId];
          if (!browseStillMatches(countedBrowse, browse)) return;
          const countStatus = countedBrowse.countStatus;
          if (countStatus?.state === "error") {
            throw new Error("Browse count refresh failed");
          }
        }
      }),
    );
    refreshes.push(
      latest.loadTableStructure(connectionId, browse.schema, browse.table),
    );
  }
  const settled = await Promise.allSettled(refreshes);
  const result = useAppStore.getState();
  const failed =
    settled.some((r) => r.status === "rejected") ||
    result.databaseOverviewStatsStatus[connectionId]?.state === "error" ||
    result.pgObjectCatalog[connectionId]?.status === "error" ||
    result.relationStatsStatus[connectionId]?.state === "error" ||
    Object.values(result.tableBrowses).some(
      (b) =>
        b.connectionId === connectionId &&
        (b.loadStatus.state === "error" ||
          result.tableStructureStatus[
            tableStructureKey(connectionId, b.schema, b.table)
          ]?.state === "error"),
    );
  if (failed)
    result.appendConsoleEvent({
      severity: "warning",
      source: "connection",
      connectionId,
      message:
        "Restore completed. Some database metadata could not refresh; refresh the affected view.",
    });
}

const invalidateMutationDraftSources = (
  state: ReturnType<typeof useAppStore.getState>,
  connectionId: string,
) => {
  const mutationDrafts = { ...state.mutationDrafts };
  const mutationDraftGenerations = { ...state.mutationDraftGenerations };
  for (const rawScope of Object.keys(mutationDrafts)) {
    // SAFETY: mutationDrafts is declared with MutationDraftScope keys.
    const scope = rawScope as MutationDraftScope;
    const draft = mutationDrafts[scope];
    if (!draft || draft.connectionId !== connectionId) {
      continue;
    }
    if (draft.changeOrder.length === 0) {
      mutationDraftGenerations[scope] =
        Math.max(draft.generation, mutationDraftGenerations[scope] ?? 0) + 1;
      delete mutationDrafts[scope];
      continue;
    }
    if (draft.sourceInvalidated) continue;
    const generation =
      Math.max(draft.generation, mutationDraftGenerations[scope] ?? 0) + 1;
    mutationDraftGenerations[scope] = generation;
    mutationDrafts[scope] = {
      ...draft,
      generation,
      analysis: null,
      sourceInvalidated: true,
      preview: { state: "idle" },
    };
  }
  return { mutationDrafts, mutationDraftGenerations };
};

type BrowseRefreshTarget = {
  tabId: string;
  connectionId: string;
  schema: string;
  table: string;
  generation: number;
};

const browseStillMatches = (
  browse:
    | ReturnType<typeof useAppStore.getState>["tableBrowses"][string]
    | undefined,
  target: BrowseRefreshTarget,
): boolean =>
  browse?.generation === target.generation &&
  browse.connectionId === target.connectionId &&
  browse.schema === target.schema &&
  browse.table === target.table;
