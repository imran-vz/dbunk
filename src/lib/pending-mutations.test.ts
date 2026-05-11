import { describe, expect, it, vi } from "vitest";

import {
  type MutationStatus,
  type PendingMutation,
  pendingMutationsFromResult,
  trackMutations,
} from "./pending-mutations";

/** Tiny builder so tests don't repeat the same `(connection, db, table)` triple. */
const m = (id: string): PendingMutation => ({
  id,
  connectionId: "conn-1",
  database: "analytics",
  table: "events",
});

describe("trackMutations", () => {
  it("completes immediately with zero runtime when the batch is empty", async () => {
    const outcome = await trackMutations([]);
    expect(outcome).toEqual({ kind: "completed", runtimeMs: 0 });
  });

  it("completes on the first poll when every mutation is done", async () => {
    const poll = vi.fn().mockResolvedValue([
      { mutationId: "a", isDone: true, latestFailReason: null },
      { mutationId: "b", isDone: true, latestFailReason: null },
    ] satisfies MutationStatus[]);

    const outcome = await trackMutations([m("a"), m("b")], {
      poll,
      intervalMs: 0,
    });

    expect(outcome.kind).toBe("completed");
    expect(poll).toHaveBeenCalledTimes(1);
  });

  it("polls again, narrowing the pending set, until everything is done", async () => {
    const poll = vi
      .fn()
      .mockResolvedValueOnce([
        { mutationId: "a", isDone: false, latestFailReason: null },
        { mutationId: "b", isDone: true, latestFailReason: null },
      ] satisfies MutationStatus[])
      .mockResolvedValueOnce([
        { mutationId: "a", isDone: true, latestFailReason: null },
      ] satisfies MutationStatus[]);

    const onProgress = vi.fn();
    const outcome = await trackMutations([m("a"), m("b")], {
      poll,
      intervalMs: 0,
      onProgress,
    });

    expect(outcome.kind).toBe("completed");
    expect(poll).toHaveBeenCalledTimes(2);
    // Second-call payload narrows to only the still-pending mutation.
    expect(poll.mock.calls[1][0]).toEqual([m("a")]);
    // Progress callback fires once between polls, with the narrowed set.
    expect(onProgress).toHaveBeenCalledTimes(1);
    expect(onProgress).toHaveBeenCalledWith([m("a")]);
  });

  it("returns `failed` when CH surfaces latestFailReason on any mutation", async () => {
    const poll = vi.fn().mockResolvedValue([
      {
        mutationId: "a",
        isDone: false,
        latestFailReason: "DB::Exception: Cannot mutate",
      },
    ] satisfies MutationStatus[]);

    const outcome = await trackMutations([m("a")], {
      poll,
      intervalMs: 0,
    });

    expect(outcome).toEqual({
      kind: "failed",
      mutationId: "a",
      reason: "DB::Exception: Cannot mutate",
    });
  });

  it("treats a poll rejection as a `failed` outcome with the error message", async () => {
    const poll = vi.fn().mockRejectedValue(new Error("network down"));
    const outcome = await trackMutations([m("a")], { poll, intervalMs: 0 });

    expect(outcome.kind).toBe("failed");
    if (outcome.kind !== "failed") {
      throw new Error("unreachable");
    }
    expect(outcome.reason).toBe("network down");
  });

  it("returns `timeout` with the still-pending IDs when the deadline elapses", async () => {
    const poll = vi.fn().mockResolvedValue([
      { mutationId: "a", isDone: false, latestFailReason: null },
      { mutationId: "b", isDone: true, latestFailReason: null },
    ] satisfies MutationStatus[]);

    // Deadline already in the past — the very first iteration bails.
    const outcome = await trackMutations([m("a"), m("b")], {
      poll,
      intervalMs: 0,
      deadlineMs: -1,
    });

    expect(outcome).toEqual({ kind: "timeout", remaining: ["a", "b"] });
  });
});

describe("pendingMutationsFromResult", () => {
  const fallback = {
    connectionId: "conn-1",
    database: "analytics",
    table: "events",
  };

  it("returns empty when the result state is not 'queued'", () => {
    expect(
      pendingMutationsFromResult(
        { state: "committed", mutationIds: ["a"] },
        fallback,
      ),
    ).toEqual([]);
  });

  it("returns empty when no mutation IDs were supplied", () => {
    expect(
      pendingMutationsFromResult(
        { state: "queued", mutationIds: [] },
        fallback,
      ),
    ).toEqual([]);
  });

  it("constructs PendingMutation records using the result's db/table when present", () => {
    const result = pendingMutationsFromResult(
      {
        state: "queued",
        database: "analytics",
        table: "events",
        mutationIds: ["m1", "m2"],
      },
      fallback,
    );
    expect(result).toEqual([
      {
        id: "m1",
        connectionId: "conn-1",
        database: "analytics",
        table: "events",
      },
      {
        id: "m2",
        connectionId: "conn-1",
        database: "analytics",
        table: "events",
      },
    ]);
  });

  it("falls back to the caller-supplied database/table when result omits them", () => {
    const result = pendingMutationsFromResult(
      { state: "queued", mutationIds: ["m1"] },
      fallback,
    );
    expect(result).toEqual([
      {
        id: "m1",
        connectionId: "conn-1",
        database: "analytics",
        table: "events",
      },
    ]);
  });
});
