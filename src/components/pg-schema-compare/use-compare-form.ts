import { useCallback, useEffect } from "react";
import { useStore } from "zustand";
import { createStore } from "zustand/vanilla";

import { createSchemaCompareRequest } from "@/lib/pg-schema-compare/client";
import { decodeSchemaCompareFailure } from "@/lib/pg-schema-compare/failure";
import type { SchemaCompareObserver } from "@/lib/pg-schema-compare/observer";
import { formatSchemaCompareFailure } from "@/lib/pg-schema-compare/presentation";
import {
  type SchemaCompareEndpoint,
  type SchemaCompareStartRequest,
  type SchemaCompareStatus,
  schemaCompareEndpoint,
} from "@/lib/pg-schema-compare/protocol";
import type { SchemaCompareSide } from "@/lib/pg-schema-compare/reader";
import type { Connection } from "@/lib/store";

export type EndpointDraft = { connectionId: string; schema: string };

/**
 * Draft endpoints and the selected job survive leaving the rail; nothing here
 * is persisted. Draft edits never retarget an accepted job: its endpoints
 * live on the native status record.
 */
export type SchemaCompareFormState = {
  source: EndpointDraft;
  target: EndpointDraft;
  seeded: boolean;
  selectedJobId: string | null;
  /** The exact request in flight or awaiting admission reconciliation. */
  pendingRequest: SchemaCompareStartRequest | null;
  submitting: boolean;
  /** Formatted start failure or validation problem for the draft. */
  error: string | null;
};
const EMPTY_ENDPOINT: EndpointDraft = { connectionId: "", schema: "" };
export const schemaCompareForm = createStore<SchemaCompareFormState>(() => ({
  source: EMPTY_ENDPOINT,
  target: EMPTY_ENDPOINT,
  seeded: false,
  selectedJobId: null,
  pendingRequest: null,
  submitting: false,
  error: null,
}));

export function validateEndpoint(
  draft: EndpointDraft,
  label: string,
): { endpoint: SchemaCompareEndpoint } | { problem: string } {
  if (!draft.connectionId) return { problem: `Choose a ${label} connection.` };
  if (draft.schema.length === 0)
    return { problem: `Enter the ${label} schema name.` };
  // Never trim or rewrite: the typed identifier is compared exactly.
  const parsed = schemaCompareEndpoint.safeParse(draft);
  return parsed.success
    ? { endpoint: parsed.data }
    : {
        problem: `The ${label} schema name must be 1–63 bytes without NUL characters.`,
      };
}

export function useSchemaCompareForm(
  connection: Connection,
  observer: SchemaCompareObserver,
) {
  const form = useStore(schemaCompareForm);
  const observation = useStore(observer.store);

  // The active connection seeds Source on first open only.
  useEffect(() => {
    const state = schemaCompareForm.getState();
    if (state.seeded) return;
    schemaCompareForm.setState({
      seeded: true,
      source:
        connection.engine === "PostgreSQL"
          ? { connectionId: connection.id, schema: "" }
          : EMPTY_ENDPOINT,
    });
  }, [connection.engine, connection.id]);

  // A lost start response resolves only through a later successful list.
  const pending = form.pendingRequest;
  useEffect(() => {
    if (!pending || form.submitting) return;
    if (observation.uncertainRequestId === pending.requestId) return;
    const admitted = observation.jobs.find(
      (job) => job.requestId === pending.requestId,
    );
    schemaCompareForm.setState({
      pendingRequest: null,
      selectedJobId: admitted?.jobId ?? null,
      error: admitted
        ? null
        : "The start response was lost and no job with this request exists. Compare again to start a new comparison.",
    });
  }, [
    form.submitting,
    observation.jobs,
    observation.uncertainRequestId,
    pending,
  ]);

  const setEndpoint = useCallback(
    (side: SchemaCompareSide, next: Partial<EndpointDraft>) => {
      schemaCompareForm.setState((state) => {
        const current = state[side];
        const changedConnection =
          next.connectionId !== undefined &&
          next.connectionId !== current.connectionId;
        return {
          error: null,
          [side]: {
            connectionId: next.connectionId ?? current.connectionId,
            // Editing the connection clears that side's schema.
            schema: changedConnection ? "" : (next.schema ?? current.schema),
          },
        };
      });
    },
    [],
  );

  const submit = useCallback(async () => {
    const state = schemaCompareForm.getState();
    if (state.submitting || state.pendingRequest) return;
    const source = validateEndpoint(state.source, "source");
    const target = validateEndpoint(state.target, "target");
    if ("problem" in source) {
      schemaCompareForm.setState({ error: source.problem });
      return;
    }
    if ("problem" in target) {
      schemaCompareForm.setState({ error: target.problem });
      return;
    }
    const payload = createSchemaCompareRequest(
      source.endpoint,
      target.endpoint,
    );
    schemaCompareForm.setState({
      submitting: true,
      pendingRequest: payload,
      error: null,
    });
    try {
      const status = await observer.start(payload);
      schemaCompareForm.setState({
        pendingRequest: null,
        selectedJobId: status.jobId,
      });
    } catch (cause) {
      const failure = decodeSchemaCompareFailure(cause);
      if (failure.kind !== "transport" && failure.kind !== "invalidResponse") {
        schemaCompareForm.setState({
          pendingRequest: null,
          error: formatSchemaCompareFailure(failure),
        });
      }
      // Otherwise the request stays pending until observation resolves it.
    } finally {
      schemaCompareForm.setState({ submitting: false });
    }
  }, [observer]);

  /** A deliberate fresh request for the exact endpoints of an earlier job. */
  const rerun = useCallback(
    (job: SchemaCompareStatus) => {
      schemaCompareForm.setState({
        source: { ...job.source },
        target: { ...job.target },
        error: null,
      });
      return submit();
    },
    [submit],
  );

  const selectJob = useCallback((jobId: string | null) => {
    schemaCompareForm.setState({ selectedJobId: jobId });
  }, []);

  return { ...form, observation, setEndpoint, submit, rerun, selectJob };
}
