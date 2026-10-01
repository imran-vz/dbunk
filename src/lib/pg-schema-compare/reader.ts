import { createStore } from "zustand/vanilla";

import { schemaCompareClient } from "./client";
import {
  decodeSchemaCompareFailure,
  type SchemaCompareFailure,
} from "./failure";
import type {
  SchemaCompareFieldSummary,
  SchemaCompareObjectSummary,
  SchemaCompareRelationIdentity,
  SchemaCompareResultRequest,
  SchemaCompareValueRef,
} from "./protocol";

type Client = typeof schemaCompareClient;
export type SchemaCompareMetadataDetail = Awaited<
  ReturnType<Client["metadata"]>
>["detail"];
export type SchemaCompareObjectPage = Awaited<ReturnType<Client["objects"]>>;
export type SchemaCompareFieldPage = Awaited<ReturnType<Client["fields"]>>;
export type SchemaCompareEligibilityDetail = Awaited<
  ReturnType<Client["eligibility"]>
>["detail"]["eligibility"];
/** One retained chunk per side: UTF-8 byte offsets from the native contract. */
export type SchemaCompareValueChunk = {
  text: string;
  offset: number;
  nextOffset: number;
  complete: boolean;
};
export type SchemaCompareSide = "source" | "target";
type Sides<T> = Record<SchemaCompareSide, T | null>;
export type SchemaCompareLoading =
  | "metadata"
  | "objects"
  | "fields"
  | "eligibility"
  | "values";

export type SchemaCompareView = {
  request: SchemaCompareResultRequest | null;
  metadata: SchemaCompareMetadataDetail | null;
  objects: SchemaCompareObjectPage | null;
  /** Offsets of earlier object pages, so Previous follows the server's cuts. */
  previousObjectOffsets: number[];
  selectedObject: SchemaCompareObjectSummary | null;
  eligibility: Sides<SchemaCompareEligibilityDetail>;
  fields: SchemaCompareFieldPage | null;
  previousFieldOffsets: number[];
  selectedField: SchemaCompareFieldSummary | null;
  values: Sides<SchemaCompareValueChunk>;
  /** Offsets of earlier chunks per side; Previous never computes an offset. */
  previousValueOffsets: Record<SchemaCompareSide, number[]>;
  loading: SchemaCompareLoading | null;
  error: SchemaCompareFailure | null;
  /** The native result refused a read; every retained payload was dropped. */
  unavailable: boolean;
};

const EMPTY: SchemaCompareView = {
  request: null,
  metadata: null,
  objects: null,
  previousObjectOffsets: [],
  selectedObject: null,
  eligibility: { source: null, target: null },
  fields: null,
  previousFieldOffsets: [],
  selectedField: null,
  values: { source: null, target: null },
  previousValueOffsets: { source: [], target: [] },
  loading: null,
  error: null,
  unavailable: false,
};

export function objectSides(summary: SchemaCompareObjectSummary) {
  return {
    source: "source" in summary ? summary.source : null,
    target: "target" in summary ? summary.target : null,
  };
}
export function fieldSides(summary: SchemaCompareFieldSummary) {
  return {
    source: "source" in summary ? summary.source : null,
    target: "target" in summary ? summary.target : null,
  };
}
/** Stable key for one relation identity: exact kind and exact name. */
export const relationKey = (identity: SchemaCompareRelationIdentity) =>
  `${identity.kind}:${identity.name}`;
/** An object whose definition on at least one side is excluded from scope. */
export const isExcludedObject = (summary: SchemaCompareObjectSummary) =>
  summary.kind === "notComparable" &&
  (summary.reason === "excludedCounterpart" ||
    summary.reason === "excludedObject");
/** The exact identity a result keys this object under: the reference side first. */
export function objectIdentity(
  summary: SchemaCompareObjectSummary,
): SchemaCompareRelationIdentity {
  switch (summary.kind) {
    case "equal":
    case "changed":
    case "sourceOnly":
      return summary.source;
    case "targetOnly":
      return summary.target;
    case "notComparable":
      return "source" in summary ? summary.source : summary.target;
  }
}
/** Next follows the server cursor; Previous pops the offset this page came from. */
function turnPage(
  direction: "next" | "previous",
  page: { offset: number; nextOffset: number | null },
  previous: number[],
): { offset: number; previous: number[] } | null {
  if (direction === "next") {
    return page.nextOffset === null
      ? null
      : { offset: page.nextOffset, previous: [...previous, page.offset] };
  }
  const offset = previous.at(-1);
  return offset === undefined
    ? null
    : { offset, previous: previous.slice(0, -1) };
}
/**
 * Reads one comparison result for one view. At most one native read is
 * outstanding; only the latest intent is queued. Every intent bumps an epoch,
 * so a late response finishes the client's acknowledgement but never touches
 * a newer view. Payloads are replaced, never appended: one metadata detail,
 * one object page, one field page and one chunk per side.
 */
export function createSchemaCompareReader(
  client: Client = schemaCompareClient,
) {
  const store = createStore<SchemaCompareView>(() => EMPTY);
  let epoch = 0;
  let running = false;
  let pending: { epoch: number; run: () => Promise<void> } | null = null;
  let lastIntent: (() => void) | null = null;

  const fresh = (mine: number) => mine === epoch;
  function enqueue(run: (mine: number) => Promise<void>) {
    const mine = ++epoch;
    pending = { epoch: mine, run: () => run(mine) };
    void drain();
  }
  async function drain() {
    if (running) return;
    running = true;
    try {
      while (pending) {
        const task = pending;
        pending = null;
        if (task.epoch === epoch) await task.run();
      }
    } finally {
      running = false;
    }
  }
  async function read<T>(
    mine: number,
    loading: SchemaCompareLoading,
    call: (request: SchemaCompareResultRequest) => Promise<T>,
  ): Promise<T | null> {
    const request = store.getState().request;
    if (!request || !fresh(mine)) return null;
    store.setState({ loading, error: null });
    try {
      const page = await call(request);
      return fresh(mine) ? page : null;
    } catch (error) {
      if (!fresh(mine)) return null;
      const failure = decodeSchemaCompareFailure(error);
      if (failure.kind === "unavailable") {
        store.setState({ ...EMPTY, request, unavailable: true });
      } else if (failure.kind === "transport") {
        // Transport loss: retained pages may be stale, so none survive it.
        store.setState({ ...EMPTY, request, error: failure });
      } else {
        store.setState({ loading: null, error: failure });
      }
      return null;
    }
  }
  const settle = (mine: number) => {
    if (fresh(mine)) store.setState({ loading: null });
  };
  const clearFields: Pick<
    SchemaCompareView,
    "fields" | "previousFieldOffsets" | "selectedField" | "values"
  > = {
    fields: null,
    previousFieldOffsets: [],
    selectedField: null,
    values: { source: null, target: null },
  };
  const clearObjects: typeof clearFields &
    Pick<SchemaCompareView, "selectedObject" | "eligibility"> = {
    ...clearFields,
    selectedObject: null,
    eligibility: { source: null, target: null },
  };

  async function loadObjects(mine: number, offset: number, previous: number[]) {
    const page = await read(mine, "objects", (request) =>
      client.objects(request, offset),
    );
    if (!page) return;
    store.setState({
      objects: page,
      previousObjectOffsets: previous,
      ...clearObjects,
    });
    settle(mine);
  }
  async function loadFields(
    mine: number,
    object: SchemaCompareObjectSummary,
    offset: number,
    previous: number[],
  ) {
    const page = await read(mine, "fields", (request) =>
      client.fields(request, objectIdentity(object), offset),
    );
    if (!page) return false;
    store.setState({
      fields: page,
      previousFieldOffsets: previous,
      selectedField: null,
      values: { source: null, target: null },
      previousValueOffsets: { source: [], target: [] },
    });
    return true;
  }
  async function loadChunk(
    mine: number,
    side: SchemaCompareSide,
    value: SchemaCompareValueRef,
    offset: number,
  ) {
    let chunk: SchemaCompareValueChunk | null;
    if (value.rawBytes === 0) {
      // Nothing to fetch: the captured value is the empty string.
      chunk = { text: "", offset: 0, nextOffset: 0, complete: true };
    } else {
      const page = await read(mine, "values", (request) =>
        client.value(request, value, offset),
      );
      if (!page) return false;
      chunk = {
        text: page.text,
        offset: page.offset,
        nextOffset: page.nextOffset,
        complete: page.complete,
      };
    }
    store.setState((state) => ({
      values: { ...state.values, [side]: chunk },
    }));
    return true;
  }

  function intent(run: (mine: number) => Promise<void>) {
    const start = () => enqueue(run);
    lastIntent = start;
    start();
  }
  /** Replaces every payload with a new result; earlier reads become stale. */
  function open(request: SchemaCompareResultRequest) {
    intent(async (mine) => {
      store.setState({ ...EMPTY, request });
      const metadata = await read(mine, "metadata", (r) => client.metadata(r));
      if (!metadata) return;
      store.setState({ metadata: metadata.detail });
      await loadObjects(mine, 0, []);
    });
  }

  return {
    store,
    open,
    objectPage(direction: "next" | "previous") {
      const { objects, previousObjectOffsets } = store.getState();
      const next =
        objects && turnPage(direction, objects, previousObjectOffsets);
      if (!next) return;
      intent((mine) => loadObjects(mine, next.offset, next.previous));
    },
    selectObject(object: SchemaCompareObjectSummary) {
      intent(async (mine) => {
        store.setState({ ...clearObjects, selectedObject: object });
        if (object.fieldCount > 0) {
          if (!(await loadFields(mine, object, 0, []))) return;
        }
        if (isExcludedObject(object)) {
          const sides = objectSides(object);
          for (const side of ["source", "target"] as const) {
            const identity = sides[side];
            if (!identity) continue;
            const page = await read(mine, "eligibility", (request) =>
              client.eligibility(request, identity, side),
            );
            if (!page) return;
            store.setState((state) => ({
              eligibility: {
                ...state.eligibility,
                [side]: page.detail.eligibility,
              },
            }));
          }
        }
        settle(mine);
      });
    },
    fieldPage(direction: "next" | "previous") {
      const { selectedObject, fields, previousFieldOffsets } = store.getState();
      const next =
        selectedObject &&
        fields &&
        turnPage(direction, fields, previousFieldOffsets);
      if (!next || !selectedObject) return;
      intent(async (mine) => {
        if (await loadFields(mine, selectedObject, next.offset, next.previous))
          settle(mine);
      });
    },
    /** Loads the first chunk of each present side; an absent side stays null. */
    selectField(field: SchemaCompareFieldSummary) {
      intent(async (mine) => {
        store.setState({
          selectedField: field,
          values: { source: null, target: null },
          previousValueOffsets: { source: [], target: [] },
        });
        const sides = fieldSides(field);
        for (const side of ["source", "target"] as const) {
          const value = sides[side];
          if (value && !(await loadChunk(mine, side, value, 0))) return;
        }
        settle(mine);
      });
    },
    /**
     * Moves one side to the next chunk (the native `nextOffset`) or back to
     * the offset an earlier chunk actually started at. Offsets are never
     * derived arithmetically: a computed byte offset can fall inside a
     * multibyte code point, which the native contract rejects.
     */
    valueChunk(side: SchemaCompareSide, direction: "next" | "previous") {
      const { selectedField, values, previousValueOffsets } = store.getState();
      const value = selectedField ? fieldSides(selectedField)[side] : null;
      const chunk = values[side];
      if (!value || !chunk) return;
      const next = turnPage(
        direction,
        {
          offset: chunk.offset,
          nextOffset: chunk.complete ? null : chunk.nextOffset,
        },
        previousValueOffsets[side],
      );
      if (!next) return;
      intent(async (mine) => {
        store.setState((state) => ({
          values: { ...state.values, [side]: null },
        }));
        if (await loadChunk(mine, side, value, next.offset)) {
          store.setState((state) => ({
            previousValueOffsets: {
              ...state.previousValueOffsets,
              [side]: next.previous,
            },
          }));
          settle(mine);
        }
      });
    },
    /** Repeats the last intent, or reopens the result once its pages were dropped. */
    retry() {
      const { request, metadata } = store.getState();
      if (request && !metadata) open(request);
      else lastIntent?.();
    },
    /** View unmount: stops reads and drops payloads; native jobs continue. */
    close() {
      epoch++;
      pending = null;
      lastIntent = null;
      store.setState(EMPTY);
    },
  };
}
export type SchemaCompareReader = ReturnType<typeof createSchemaCompareReader>;
