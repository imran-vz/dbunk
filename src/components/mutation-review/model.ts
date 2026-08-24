import {
  type ResultMutationError,
  usesProjectedRowGuards,
} from "@/lib/result-mutation";
import { formatSharedTransportError } from "@/lib/safety-policy";
import type { MutationDraft, MutationDraftChange } from "@/lib/store";

export type MutationChangeGroup = {
  key: string;
  label: string;
  target: string;
  changes: MutationDraftChange[];
};

const kindLabel = (kind: MutationDraftChange["kind"]): string => {
  switch (kind) {
    case "updateRow":
      return "Updates";
    case "deleteRow":
      return "Deletes";
    case "insertRow":
      return "Inserts";
  }
};

export const qualifiedTarget = (change: MutationDraftChange): string =>
  `${change.table.schema}.${change.table.table}`;

export const groupMutationChanges = (
  draft: MutationDraft,
): MutationChangeGroup[] => {
  const groups = new Map<string, MutationChangeGroup>();
  for (const changeId of draft.changeOrder) {
    const change = draft.changes[changeId];
    if (!change) continue;
    const target = qualifiedTarget(change);
    const label = kindLabel(change.kind);
    const key = `${change.kind}:${target}`;
    const existing = groups.get(key);
    if (existing) {
      existing.changes.push(change);
    } else {
      groups.set(key, { key, label, target, changes: [change] });
    }
  }
  return [...groups.values()];
};

export const mutationTargets = (draft: MutationDraft): string[] => {
  const targets = new Set<string>();
  for (const changeId of draft.changeOrder) {
    const change = draft.changes[changeId];
    if (change) targets.add(qualifiedTarget(change));
  }
  return [...targets];
};

export const displayMutationValue = (value: string | null): string =>
  value === null ? "NULL" : value;

const identitySummary = (change: MutationDraftChange): string => {
  if (change.kind === "insertRow") return "New row";
  if (change.identity.length === 0) return "Row";
  return change.identity
    .map(({ column, value }) => `${column} = ${displayMutationValue(value)}`)
    .join(", ");
};

export const mutationChangeTitle = (change: MutationDraftChange): string => {
  const identity = identitySummary(change);
  switch (change.kind) {
    case "updateRow":
      return `Update row ${identity}`;
    case "deleteRow":
      return `Delete row ${identity}`;
    case "insertRow":
      return identity;
  }
};

export const mutationChangeGuardCopy = (
  change: MutationDraftChange,
): string => {
  if (change.kind === "insertRow") return "Values included in this insert";
  if (change.kind === "deleteRow") return "All projected values guarded";
  if (usesProjectedRowGuards(change.identityKind)) {
    return "Full projected row guarded";
  }
  return "Edited columns guarded";
};

export const mutationFailureMessage = (
  change: MutationDraftChange,
): string | null => {
  const error = change.failure?.error;
  if (!error) return null;
  switch (error.kind) {
    case "conflict":
      return change.kind === "updateRow" &&
        !usesProjectedRowGuards(change.identityKind)
        ? "Conflict on this change. An edited-column guard no longer matched. Nothing was applied."
        : "Conflict on this change. A projected-row guard no longer matched. Nothing was applied.";
    case "identityNotUnique":
      return "This identity matched more than one row. Nothing was applied.";
    case "lockTimeout":
      return "This operation timed out waiting for a lock. Nothing was applied.";
    case "database":
      return error.code
        ? `Database error ${error.code}: ${error.message}`
        : `Database error: ${error.message}`;
  }
};

export const formatMutationError = (error: ResultMutationError): string => {
  switch (error.kind) {
    case "unsupportedEngine":
      return "Result mutation is unavailable for this database engine.";
    case "notAnalyzable":
      switch (error.reason.kind) {
        case "multiStatement":
          return "Only a single statement can be analyzed for editing.";
        case "noProjectedColumns":
          return "The result has no projected columns to edit.";
        case "noTableOrigins":
          return "The result has no writable table columns.";
        case "possibleTempShadowing":
          return "Editing is blocked because a temporary table may shadow the resolved target.";
        case "database":
          return error.reason.code
            ? `${error.reason.code}: ${error.reason.message}`
            : error.reason.message;
      }
    case "unknownColumn":
      return `Unknown column ${error.column}.`;
    case "invalidPlan":
      return `The mutation plan is invalid (${error.reason}).`;
    case "analysisExpired":
      return "The analysis expired. Refresh the preview and try again.";
    case "conflict":
      return "A row changed after staging. The transaction was rolled back.";
    case "identityNotUnique":
      return "An identity matched multiple rows. The transaction was rolled back.";
    case "lockTimeout":
      return "A database lock timed out. The transaction was rolled back.";
    case "busy":
      return "Another mutation is already applying on this connection.";
    case "superseded":
      return "The analysis was superseded by a newer request.";
    case "cancelled":
      return "The mutation was cancelled. Nothing was applied.";
    case "connectionClosing":
    case "connectionLost":
    case "policyBlocked":
    case "policyNeedsConfirmation":
    case "timeout":
    case "database":
      return formatSharedTransportError(error);
  }
};
