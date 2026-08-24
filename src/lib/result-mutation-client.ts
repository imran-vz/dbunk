/* oxlint-disable anti-slop/no-unknown-parameters -- Invoke rejections are decoded at this command boundary. */
import {
  type AnalyzeResultSetPayload,
  type AnalyzeResultSetResult,
  type ApplyResult,
  type ApplyResultMutationsPayload,
  type CancelResultMutationPayload,
  type CancelResultMutationResult,
  type ClearVirtualKeyPayload,
  type CloseResultMutationPayload,
  decodeResultMutationError,
  type LoadVirtualKeyPayload,
  type PreviewResult,
  type PreviewResultMutationsPayload,
  type ResultMutationError,
  type SaveVirtualKeyPayload,
  type VirtualKey,
} from "@/lib/result-mutation";
import { isTauri, tauriInvoke } from "@/lib/tauri";

export type ResultMutationClientResult<T> =
  | { kind: "ok"; value: T }
  | { kind: "superseded" }
  | { kind: "cancelled" }
  | { kind: "error"; error: ResultMutationError };

type TabRequestState = {
  connectionId: string;
  nextAnalysisId: number;
  latestAnalysisId: number;
  nextApplyId: number;
};

const tabs = new Map<string, TabRequestState>();

const tabState = (tabId: string, connectionId: string): TabRequestState => {
  const existing = tabs.get(tabId);
  if (existing?.connectionId === connectionId) return existing;
  const created = {
    connectionId,
    nextAnalysisId: 0,
    latestAnalysisId: 0,
    nextApplyId: 0,
  };
  tabs.set(tabId, created);
  return created;
};

const issueAnalysisRequestId = (
  tabId: string,
  connectionId: string,
): number => {
  const state = tabState(tabId, connectionId);
  state.nextAnalysisId += 1;
  state.latestAnalysisId = state.nextAnalysisId;
  return state.nextAnalysisId;
};

const issueApplyRequestId = (tabId: string, connectionId: string): number => {
  const state = tabState(tabId, connectionId);
  state.nextApplyId += 1;
  return state.nextApplyId;
};

const isStaleAnalysis = (
  tabId: string,
  connectionId: string,
  requestId: number,
): boolean => {
  const state = tabs.get(tabId);
  return (
    !state ||
    state.connectionId !== connectionId ||
    requestId < state.latestAnalysisId
  );
};

const fromError = (error: unknown): ResultMutationClientResult<never> => {
  const decoded = decodeResultMutationError(error);
  if (decoded.kind === "superseded" || decoded.kind === "cancelled") {
    return { kind: decoded.kind };
  }
  return { kind: "error", error: decoded };
};

type MutationCommandPayload =
  | PreviewResultMutationsPayload
  | ApplyResultMutationsPayload
  | CancelResultMutationPayload
  | LoadVirtualKeyPayload
  | SaveVirtualKeyPayload
  | ClearVirtualKeyPayload
  | CloseResultMutationPayload;

const invokeMutation = async <T, Payload extends MutationCommandPayload>(
  command: string,
  payload: Payload,
): Promise<ResultMutationClientResult<T>> => {
  if (!isTauri()) {
    return { kind: "error", error: { kind: "connectionLost" } };
  }
  try {
    const value = await tauriInvoke<T>(command, { payload });
    return { kind: "ok", value };
  } catch (error) {
    return fromError(error);
  }
};

export const resetResultMutationClientForTab = (tabId: string): void => {
  tabs.delete(tabId);
};

export async function analyzeResultSet(
  payload: Omit<AnalyzeResultSetPayload, "requestId">,
): Promise<ResultMutationClientResult<AnalyzeResultSetResult>> {
  if (!isTauri()) {
    return { kind: "error", error: { kind: "connectionLost" } };
  }
  const requestId = issueAnalysisRequestId(payload.tabId, payload.connectionId);
  try {
    const value = await tauriInvoke<AnalyzeResultSetResult>(
      "analyze_result_set",
      { payload: { ...payload, requestId } },
    );
    if (isStaleAnalysis(payload.tabId, payload.connectionId, requestId)) {
      return { kind: "superseded" };
    }
    return { kind: "ok", value };
  } catch (error) {
    if (isStaleAnalysis(payload.tabId, payload.connectionId, requestId)) {
      return { kind: "superseded" };
    }
    return fromError(error);
  }
}

export function previewResultMutations(
  payload: PreviewResultMutationsPayload,
): Promise<ResultMutationClientResult<PreviewResult>> {
  return invokeMutation("preview_result_mutations", payload);
}

export function applyResultMutations(
  payload: Omit<ApplyResultMutationsPayload, "requestId">,
): Promise<ResultMutationClientResult<ApplyResult>> {
  const requestId = issueApplyRequestId(payload.tabId, payload.connectionId);
  return invokeMutation("apply_result_mutations", { ...payload, requestId });
}

export function cancelResultMutation(
  payload: CancelResultMutationPayload,
): Promise<ResultMutationClientResult<CancelResultMutationResult>> {
  return invokeMutation("cancel_result_mutation", payload);
}

export function loadVirtualKey(
  payload: LoadVirtualKeyPayload,
): Promise<ResultMutationClientResult<VirtualKey | null>> {
  return invokeMutation("load_virtual_key", payload);
}

export function saveVirtualKey(
  payload: SaveVirtualKeyPayload,
): Promise<ResultMutationClientResult<void>> {
  return invokeMutation("save_virtual_key", payload);
}

export function clearVirtualKey(
  payload: ClearVirtualKeyPayload,
): Promise<ResultMutationClientResult<void>> {
  return invokeMutation("clear_virtual_key", payload);
}

export async function closeResultMutationForConnection(
  connectionId: string,
): Promise<ResultMutationClientResult<void>> {
  for (const [tabId, state] of tabs) {
    if (state.connectionId === connectionId) tabs.delete(tabId);
  }
  return invokeMutation("close_result_mutation_for_connection", {
    connectionId,
  });
}
