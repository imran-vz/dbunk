import { describe, expect, it } from "vitest";

import {
  schemaCompareCaptureMetadata,
  schemaCompareChunk,
  schemaCompareError,
  schemaCompareFieldPath,
  schemaCompareFieldSummary,
  schemaCompareObjectPage,
  schemaCompareObjectSummary,
  schemaCompareStatus,
  schemaCompareValueRef,
} from "./protocol";

describe("schema comparison wire contract", () => {
  const value = {
    side: "source",
    valueId: 0,
    rawBytes: 256 * 1024,
    valueKind: "text",
  };
  const identity = { jobId: "job", resultId: "result" };

  it("accepts one worst-case escaped chunk without flattening value identity", () => {
    const chunk = {
      responseId: "response",
      identity,
      value,
      offset: 0,
      text: "\u0001".repeat(64 * 1024),
      nextOffset: 64 * 1024,
      complete: false,
    };
    expect(schemaCompareChunk.safeParse(chunk).success).toBe(true);
    expect(new TextEncoder().encode(JSON.stringify(chunk)).length).toBeLessThan(
      1024 * 1024,
    );
    expect(
      schemaCompareChunk.safeParse({ ...chunk, text: chunk.text + "x" })
        .success,
    ).toBe(false);
  });

  it("checks UTF-8 byte offsets rather than JavaScript string lengths", () => {
    const chunk = {
      responseId: "response",
      identity,
      value,
      offset: 0,
      text: "€".repeat(21_845),
      nextOffset: 65_535,
      complete: false,
    };
    expect(schemaCompareChunk.safeParse(chunk).success).toBe(true);
    expect(
      schemaCompareChunk.safeParse({ ...chunk, nextOffset: 21_845 }).success,
    ).toBe(false);
    expect(
      schemaCompareChunk.safeParse({ ...chunk, complete: true }).success,
    ).toBe(false);
  });

  it("keeps source and target references independent and rejects oversized metadata", () => {
    expect(schemaCompareValueRef.parse({ ...value, side: "target" }).side).toBe(
      "target",
    );
    expect(
      schemaCompareFieldPath.safeParse({
        kind: "column",
        name: "€".repeat(22),
        field: "default",
      }).success,
    ).toBe(false);
    expect(
      schemaCompareFieldPath.safeParse({
        kind: "index",
        name: "idx",
        owner: "unique",
        field: "nullsNotDistinct",
      }).success,
    ).toBe(true);
    expect(
      schemaCompareFieldPath.safeParse({
        kind: "indexKey",
        name: "idx",
        owner: null,
        position: 1,
        field: "expression",
      }).success,
    ).toBe(true);
    expect(
      schemaCompareFieldPath.safeParse({
        kind: "indexKey",
        name: "idx",
        owner: null,
        position: 1.5,
        field: "expression",
      }).success,
    ).toBe(false);
  });

  it("does not decode expired or unsupported results as equality", () => {
    expect(schemaCompareError.parse({ kind: "unavailable" }).kind).toBe(
      "unavailable",
    );
    expect(
      schemaCompareError.safeParse({
        kind: "unsupportedVersion",
        side: "target",
        version: "17.0",
      }).success,
    ).toBe(true);
    expect(schemaCompareError.safeParse({ kind: "equal" }).success).toBe(false);
    const item = {
      source: { kind: "table", name: "orders" },
      target: { kind: "view", name: "orders" },
      kind: "notComparable",
      reason: "excludedCounterpart",
      fieldCount: 0,
      changedFields: 0,
      incomparableFields: 0,
    };
    const page = {
      responseId: "response",
      identity,
      offset: 0,
      nextOffset: null,
      items: Array.from({ length: 100 }, () => item),
    };
    expect(schemaCompareObjectPage.safeParse(page).success).toBe(true);
    expect(
      schemaCompareObjectPage.safeParse({
        ...page,
        items: [...page.items, item],
      }).success,
    ).toBe(false);
  });

  it("rejects impossible field and object summary states", () => {
    const path = { kind: "column", name: "id", field: "default" };
    const source = { ...value, side: "source" };
    const target = { ...value, side: "target" };
    expect(
      schemaCompareFieldSummary.safeParse({
        path,
        kind: "changed",
        source,
        target,
      }).success,
    ).toBe(true);
    expect(
      schemaCompareFieldSummary.safeParse({
        path,
        kind: "equal",
        source: null,
        target: null,
      }).success,
    ).toBe(false);
    expect(
      schemaCompareFieldSummary.safeParse({
        path,
        kind: "sourceOnly",
        source,
        target,
      }).success,
    ).toBe(false);
    expect(
      schemaCompareFieldSummary.safeParse({
        path,
        kind: "notComparable",
        source,
      }).success,
    ).toBe(false);
    expect(
      schemaCompareFieldSummary.safeParse({
        path,
        kind: "notComparable",
        reason: "expressionOutsideSubset",
      }).success,
    ).toBe(false);
    expect(
      schemaCompareFieldSummary.safeParse({
        path,
        kind: "notComparable",
        reason: "expressionOutsideSubset",
        source,
      }).success,
    ).toBe(true);

    const sourceObject = { kind: "table", name: "orders" };
    const targetObject = { kind: "table", name: "orders" };
    const counts = {
      fieldCount: 2,
      changedFields: 1,
      incomparableFields: 1,
    };
    expect(
      schemaCompareObjectSummary.safeParse({
        ...counts,
        kind: "changed",
        source: sourceObject,
        target: targetObject,
      }).success,
    ).toBe(true);
    expect(
      schemaCompareObjectSummary.safeParse({
        ...counts,
        kind: "targetOnly",
        source: sourceObject,
        target: targetObject,
      }).success,
    ).toBe(false);
    expect(
      schemaCompareObjectSummary.safeParse({
        ...counts,
        kind: "notComparable",
        source: sourceObject,
      }).success,
    ).toBe(false);
  });

  it("binds terminal status fields to their phases", () => {
    const base = {
      jobId: "job",
      requestId: "request",
      source: { connectionId: "source", schema: "public" },
      target: { connectionId: "target", schema: "public" },
      sourceObjects: 1,
      targetObjects: 1,
    };
    expect(
      schemaCompareStatus.safeParse({
        ...base,
        phase: "completed",
        resultId: "result",
      }).success,
    ).toBe(true);
    expect(
      schemaCompareStatus.safeParse({
        ...base,
        phase: "failed",
        failure: { kind: "captureChanged" },
      }).success,
    ).toBe(true);
    for (const status of [
      { ...base, phase: "completed" },
      {
        ...base,
        phase: "completed",
        resultId: "result",
        failure: { kind: "captureChanged" },
      },
      { ...base, phase: "failed", resultId: "result" },
      { ...base, phase: "comparing", resultId: "result" },
      {
        ...base,
        phase: "readingBoth",
        failure: { kind: "captureChanged" },
      },
    ]) {
      expect(schemaCompareStatus.safeParse(status).success).toBe(false);
    }
  });

  it("retains display and integral numeric server versions", () => {
    const metadata = {
      endpoint: { connectionId: "source", schema: "public" },
      serverVersion: "16.15",
      serverVersionNum: 160_015,
      capturedAt: "2026-09-06T00:00:00Z",
    };
    expect(schemaCompareCaptureMetadata.safeParse(metadata).success).toBe(true);
    expect(
      schemaCompareCaptureMetadata.safeParse({
        ...metadata,
        serverVersionNum: 160_015.5,
      }).success,
    ).toBe(false);
  });
});
