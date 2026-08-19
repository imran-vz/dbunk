/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters anti-slop/require-safety-comment-for-type-assertion -- Channel tests mock the Tauri invoke/Channel boundary. */
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const channelApi = vi.hoisted(() => {
  const instances: Array<{
    onmessage: ((event: unknown) => void) | null;
  }> = [];
  class MockChannel {
    onmessage: ((event: unknown) => void) | null = null;
    constructor() {
      instances.push(this);
    }
  }
  return { instances, MockChannel };
});

vi.mock("@tauri-apps/api/core", () => ({
  Channel: channelApi.MockChannel,
}));

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(() => Promise.resolve()),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import type { QueryEventEnvelope } from "@/lib/query-session-channel";
import {
  executeQuerySession,
  hasQuerySessionBinding,
  openQuerySession,
  resetQuerySessionChannelForTests,
} from "@/lib/query-session-channel";
import { tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);

const transaction = {
  mode: "autocommit" as const,
  status: "idle" as const,
  manualIsolation: "readCommitted" as const,
};

const lastChannel = () => channelApi.instances.at(-1);

const send = (envelope: QueryEventEnvelope) => {
  const channel = lastChannel();
  if (!channel?.onmessage) throw new Error("channel was not registered");
  channel.onmessage(envelope);
};

const baseEnvelope = (
  overrides: Partial<QueryEventEnvelope> & {
    event: QueryEventEnvelope["event"];
  },
): QueryEventEnvelope => ({
  sessionId: "session-1",
  tabId: "tab-1",
  connectionId: "conn-1",
  generation: 1,
  sequence: 1,
  executionId: "exec-1",
  requiresAck: false,
  ...overrides,
});

beforeEach(() => {
  resetQuerySessionChannelForTests();
  channelApi.instances.length = 0;
  mockedInvoke.mockReset();
  mockedInvoke.mockImplementation((command) => {
    if (command === "open_query_session") return Promise.resolve(transaction);
    return Promise.resolve(undefined);
  });
});

afterEach(() => {
  resetQuerySessionChannelForTests();
});

describe("query-session-channel", () => {
  it("coalesces acks that arrive while one is in flight and serializes them", async () => {
    let releaseAck: (() => void) | undefined;
    const ackCalls: Array<{ sequence: number; retainMoreRows: boolean }> = [];
    mockedInvoke.mockImplementation((command, payload) => {
      if (command === "open_query_session") return Promise.resolve(transaction);
      if (command === "ack_query_session_events") {
        const ack = (
          payload as {
            payload: { ackThroughSequence: number; retainMoreRows: boolean };
          }
        ).payload;
        ackCalls.push({
          sequence: ack.ackThroughSequence,
          retainMoreRows: ack.retainMoreRows,
        });
        if (ackCalls.length === 1) {
          return new Promise((resolve) => {
            releaseAck = () => resolve(undefined);
          });
        }
        return Promise.resolve(undefined);
      }
      return Promise.resolve(undefined);
    });

    const handler = vi.fn(() => ({ retainMoreRows: true }));
    await openQuerySession({
      sessionId: "session-1",
      tabId: "tab-1",
      connectionId: "conn-1",
      handler,
    });

    send(
      baseEnvelope({
        sequence: 1,
        requiresAck: true,
        event: { kind: "executionStarted" },
      }),
    );
    await Promise.resolve();
    expect(ackCalls).toHaveLength(1);
    expect(ackCalls[0]).toEqual({ sequence: 1, retainMoreRows: true });

    handler.mockReturnValueOnce({ retainMoreRows: false });
    send(
      baseEnvelope({
        sequence: 2,
        requiresAck: true,
        event: {
          kind: "executionCompleted",
          status: "completed",
          transaction,
          omittedRows: 0,
          omittedResultSets: 0,
          omittedNotices: 0,
          omittedMetadataBytes: 0,
          truncationReasons: [],
          error: null,
        },
      }),
    );
    await Promise.resolve();
    expect(ackCalls).toHaveLength(1);

    releaseAck?.();
    await Promise.resolve();
    await Promise.resolve();
    expect(ackCalls).toHaveLength(2);
    expect(ackCalls[1]).toEqual({ sequence: 2, retainMoreRows: false });
  });

  it("removes the binding and notifies the handler on a sequence gap", async () => {
    const handler = vi.fn(() => ({ retainMoreRows: true }));
    await openQuerySession({
      sessionId: "session-1",
      tabId: "tab-1",
      connectionId: "conn-1",
      handler,
    });
    expect(hasQuerySessionBinding("tab-1")).toBe(true);

    send(
      baseEnvelope({
        sequence: 4,
        event: { kind: "executionStarted" },
      }),
    );

    expect(hasQuerySessionBinding("tab-1")).toBe(false);
    expect(handler).toHaveBeenCalledWith(
      expect.objectContaining({
        event: { kind: "sessionLost", reason: "invalidSequence" },
      }),
    );
    expect(mockedInvoke).toHaveBeenCalledWith("close_query_session", {
      payload: { sessionId: "session-1" },
    });
  });

  it("settles the pending run when a terminal ack succeeds", async () => {
    await openQuerySession({
      sessionId: "session-1",
      tabId: "tab-1",
      connectionId: "conn-1",
      handler: () => ({ retainMoreRows: true }),
    });

    const settled = executeQuerySession("tab-1", "exec-1", "select 1");
    send(
      baseEnvelope({
        sequence: 1,
        requiresAck: true,
        event: {
          kind: "executionCompleted",
          status: "completed",
          transaction,
          omittedRows: 0,
          omittedResultSets: 0,
          omittedNotices: 0,
          omittedMetadataBytes: 0,
          truncationReasons: [],
          error: null,
        },
      }),
    );
    await expect(settled).resolves.toBeUndefined();
  });

  it("rejects the pending run when an ack fails", async () => {
    mockedInvoke.mockImplementation((command) => {
      if (command === "open_query_session") return Promise.resolve(transaction);
      if (command === "ack_query_session_events") {
        return Promise.reject(new Error("backend down"));
      }
      return Promise.resolve(undefined);
    });

    await openQuerySession({
      sessionId: "session-1",
      tabId: "tab-1",
      connectionId: "conn-1",
      handler: () => ({ retainMoreRows: true }),
    });

    const settled = executeQuerySession("tab-1", "exec-1", "select 1");
    send(
      baseEnvelope({
        sequence: 1,
        requiresAck: true,
        event: {
          kind: "executionCompleted",
          status: "completed",
          transaction,
          omittedRows: 0,
          omittedResultSets: 0,
          omittedNotices: 0,
          omittedMetadataBytes: 0,
          truncationReasons: [],
          error: null,
        },
      }),
    );
    await expect(settled).rejects.toEqual({ kind: "connectionLost" });
  });
});
