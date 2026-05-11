/**
 * Pending Mutation lifecycle.
 *
 * A **Pending Mutation** is a single ClickHouse `ALTER TABLE … UPDATE`
 * or `ALTER TABLE … DELETE` statement that has been accepted by the
 * server and is applying asynchronously across MergeTree parts. It is
 * identified by `mutation_id` from CH's `system.mutations` table and
 * scoped to a specific `(connection, database, table)`.
 *
 * The Tauri command `commit_cell_edits` / `delete_rows` returns
 * `state: "queued"` along with a list of mutation IDs; this module is
 * responsible for driving those mutations to completion. Consumers
 * (cell edits, row deletes, future async DDL) supply the list and
 * react to the resulting `MutationOutcome`.
 *
 * ## Interface
 *
 * - [`PendingMutation`] — the entity itself: id + connection scope.
 * - [`trackMutations`] — drives a batch to completion. Returns a
 *   discriminated `MutationOutcome` (completed / failed / timeout).
 *   The polling function is injectable for testing; the default calls
 *   the `poll_mutation_status` Tauri command.
 * - [`pendingMutationsFromResult`] — constructs the batch from a
 *   backend result payload, so callers don't repeat the
 *   "is this queued? then map mutationIds" boilerplate.
 *
 * The module deliberately doesn't know about zustand, the workspace's
 * status banners, or the table-data refresh path. Each consumer
 * translates the outcome into its own status surface.
 */

import { tauriInvoke } from "@/lib/tauri";

export type PendingMutation = {
  /** ClickHouse `system.mutations.mutation_id`. */
  id: string;
  /** Stored connection ID — needed because polling routes per-engine. */
  connectionId: string;
  /** Database (CH "schema") containing the mutated table. */
  database: string;
  /** Table the mutation applies to. */
  table: string;
};

/** Response shape from the `poll_mutation_status` Tauri command. */
export type MutationStatus = {
  mutationId: string;
  isDone: boolean;
  latestFailReason: string | null;
};

/**
 * The three end-states of a tracked mutation batch.
 *
 * - `completed` — every mutation reported `is_done = true`.
 * - `failed`    — CH surfaced a `latestFailReason` on one mutation OR
 *                 the poll call itself rejected (transport / auth).
 *                 We collapse both into "failed" with a reason string
 *                 because the consumer's response (show error) is the
 *                 same.
 * - `timeout`   — the deadline elapsed with mutations still pending.
 *                 The remaining IDs are returned so the consumer can
 *                 surface them (or resume polling) if they choose.
 */
export type MutationOutcome =
  | { kind: "completed"; runtimeMs: number }
  | { kind: "failed"; mutationId: string; reason: string }
  | { kind: "timeout"; remaining: string[] };

export type TrackMutationsOptions = {
  /** Override the poll function — tests pass a fake; default calls Tauri. */
  poll?: (pending: PendingMutation[]) => Promise<MutationStatus[]>;
  /** Total time budget before giving up. Default 60 s. */
  deadlineMs?: number;
  /** Sleep between polls. Default 1 s. */
  intervalMs?: number;
  /** Fires after each poll with the still-pending mutations. */
  onProgress?: (remaining: PendingMutation[]) => void;
};

const DEFAULT_DEADLINE_MS = 60_000;
const DEFAULT_INTERVAL_MS = 1_000;

const defaultPoll = async (
  pending: PendingMutation[],
): Promise<MutationStatus[]> => {
  // Every mutation in a single `trackMutations` call shares the same
  // (connection, database, table) — they came from one commit. We
  // read the scope off the first entry; mixed-scope batches aren't a
  // supported shape.
  const first = pending[0];
  if (!first) {
    return [];
  }
  return tauriInvoke<MutationStatus[]>("poll_mutation_status", {
    payload: {
      connectionId: first.connectionId,
      database: first.database,
      table: first.table,
      mutationIds: pending.map((mutation) => mutation.id),
    },
  });
};

const errorToReason = (error: unknown): string => {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return String(error);
};

/**
 * Drive a batch of `PendingMutation` records to completion.
 *
 * Polls on a fixed interval until every mutation reports `is_done`,
 * one fails, or the deadline elapses. Returns a `MutationOutcome` that
 * the consumer pattern-matches on.
 *
 * `trackMutations([])` resolves immediately as `completed` so callers
 * don't need to short-circuit on empty input.
 */
export async function trackMutations(
  mutations: PendingMutation[],
  options: TrackMutationsOptions = {},
): Promise<MutationOutcome> {
  if (mutations.length === 0) {
    return { kind: "completed", runtimeMs: 0 };
  }

  const poll = options.poll ?? defaultPoll;
  const deadlineMs = options.deadlineMs ?? DEFAULT_DEADLINE_MS;
  const intervalMs = options.intervalMs ?? DEFAULT_INTERVAL_MS;
  const onProgress = options.onProgress;

  const start = Date.now();
  let pending = [...mutations];

  while (pending.length > 0) {
    if (Date.now() - start > deadlineMs) {
      return {
        kind: "timeout",
        remaining: pending.map((mutation) => mutation.id),
      };
    }

    let statuses: MutationStatus[];
    try {
      statuses = await poll(pending);
    } catch (error) {
      // Treat a transport failure as a definitive end-state rather
      // than retrying — the caller can resume polling explicitly if
      // they want by issuing a fresh trackMutations call.
      return {
        kind: "failed",
        mutationId: pending[0]?.id ?? "",
        reason: errorToReason(error),
      };
    }

    const failed = statuses.find((status) => status.latestFailReason);
    if (failed) {
      return {
        kind: "failed",
        mutationId: failed.mutationId,
        reason: failed.latestFailReason ?? "unknown",
      };
    }

    const stillPendingIds = new Set(
      statuses
        .filter((status) => !status.isDone)
        .map((status) => status.mutationId),
    );
    pending = pending.filter((mutation) => stillPendingIds.has(mutation.id));

    if (pending.length === 0) {
      return { kind: "completed", runtimeMs: Date.now() - start };
    }

    onProgress?.(pending);

    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }

  return { kind: "completed", runtimeMs: Date.now() - start };
}

/**
 * Result shape that `pendingMutationsFromResult` accepts. The shared
 * structural type lets both `CommitCellEditsResult` and
 * `DeleteRowsResult` flow through without a wider import.
 */
type QueuedResultPayload = {
  state?: string;
  database?: string;
  table?: string;
  mutationIds?: string[];
};

type ResultFallback = {
  connectionId: string;
  database: string;
  table: string;
};

/**
 * Convert a backend mutation-result payload into a list of
 * `PendingMutation` records. Returns an empty array when the result is
 * not `queued` (PostgreSQL always returns `committed`).
 *
 * `fallback` supplies connection scope (which the backend doesn't
 * echo) plus default database/table for older result shapes that
 * elide them.
 */
export function pendingMutationsFromResult(
  result: QueuedResultPayload,
  fallback: ResultFallback,
): PendingMutation[] {
  if (result.state !== "queued") {
    return [];
  }
  const ids = result.mutationIds ?? [];
  if (ids.length === 0) {
    return [];
  }
  return ids.map((id) => ({
    id,
    connectionId: fallback.connectionId,
    database: result.database ?? fallback.database,
    table: result.table ?? fallback.table,
  }));
}
