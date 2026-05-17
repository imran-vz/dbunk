import { describe, expect, it } from "vitest";

import type { ExplainPlanNode } from "@/components/query-editor/results-view";

import { analyzePlan, formatRatio } from "./plan-analysis";

const node = (overrides: Partial<ExplainPlanNode> = {}): ExplainPlanNode => ({
  nodeType: "Seq Scan",
  buffers: [],
  children: [],
  ...overrides,
});

describe("analyzePlan", () => {
  it("computes inclusive time as actualTotalTime × loops", () => {
    const root = node({
      nodeType: "Nested Loop",
      actualTotalTime: 1.5,
      actualLoops: 4,
    });
    const result = analyzePlan({
      runtimeMs: 6,
      planningMs: 0.1,
      executionMs: 6,
      root,
    });
    expect(result.root.inclusiveMs).toBe(6);
  });

  it("computes self time as inclusive minus child inclusive sum", () => {
    const child = node({
      nodeType: "Index Scan",
      actualTotalTime: 0.4,
      actualLoops: 1,
    });
    const root = node({
      nodeType: "Limit",
      actualTotalTime: 1,
      actualLoops: 1,
      children: [child],
    });
    const result = analyzePlan({
      runtimeMs: 1,
      planningMs: null,
      executionMs: 1,
      root,
    });
    expect(result.root.selfMs).toBeCloseTo(0.6, 5);
    expect(result.root.children[0].selfMs).toBeCloseTo(0.4, 5);
  });

  it("falls back to inclusive self when a child time is unknown", () => {
    const root = node({
      nodeType: "Append",
      actualTotalTime: 1,
      actualLoops: 1,
      children: [node({ nodeType: "Seq Scan" })],
    });
    const result = analyzePlan({
      runtimeMs: 1,
      planningMs: null,
      executionMs: 1,
      root,
    });
    expect(result.root.selfMs).toBe(1);
  });

  it("flags estimate as 'under' when actual rows exceed planned by ≥2×", () => {
    const root = node({ planRows: 1, actualRows: 100 });
    const result = analyzePlan({
      runtimeMs: 0,
      planningMs: null,
      executionMs: null,
      root,
    });
    expect(result.root.estimate.kind).toBe("under");
    if (result.root.estimate.kind === "under") {
      expect(result.root.estimate.ratio).toBe(100);
    }
  });

  it("flags estimate as 'over' when planned rows exceed actual by ≥2×", () => {
    const root = node({ planRows: 580, actualRows: 3 });
    const result = analyzePlan({
      runtimeMs: 0,
      planningMs: null,
      executionMs: null,
      root,
    });
    expect(result.root.estimate.kind).toBe("over");
    if (result.root.estimate.kind === "over") {
      expect(result.root.estimate.ratio).toBeCloseTo(580 / 3, 3);
    }
  });

  it("treats sub-2× drift as accurate", () => {
    const root = node({ planRows: 100, actualRows: 150 });
    const result = analyzePlan({
      runtimeMs: 0,
      planningMs: null,
      executionMs: null,
      root,
    });
    expect(result.root.estimate.kind).toBe("accurate");
  });

  it("derives a planning-vs-execution skew insight when planning ≥ 10× exec", () => {
    const result = analyzePlan({
      runtimeMs: 3,
      planningMs: 2.7,
      executionMs: 0.05,
      root: node({ actualTotalTime: 0.05, actualLoops: 1 }),
    });
    expect(result.insights.some((i) => i.id === "planning-skew")).toBe(true);
  });

  it("surfaces the worst estimate miss in the insights ribbon", () => {
    const root = node({
      nodeType: "Append",
      actualTotalTime: 0.05,
      actualLoops: 1,
      planRows: 1740,
      actualRows: 5,
      children: [
        node({
          nodeType: "Seq Scan",
          actualTotalTime: 0.005,
          actualLoops: 1,
          planRows: 580,
          actualRows: 1,
        }),
      ],
    });
    const result = analyzePlan({
      runtimeMs: 1,
      planningMs: null,
      executionMs: 0.05,
      root,
    });
    const miss = result.insights.find((i) => i.id === "estimate-miss");
    expect(miss).toBeDefined();
    expect(miss?.message).toMatch(/580×/);
    expect(miss?.message).toContain("Seq Scan");
  });

  it("warns when only sequential scans are present", () => {
    const root = node({
      nodeType: "Seq Scan",
      actualRows: 1,
      planRows: 1,
    });
    const result = analyzePlan({
      runtimeMs: 0,
      planningMs: null,
      executionMs: null,
      root,
    });
    const seq = result.insights.find((i) => i.id === "seq-scans");
    expect(seq?.severity).toBe("warn");
  });

  it("picks the hottest node by self time", () => {
    const cheap = node({
      nodeType: "Index Scan",
      actualTotalTime: 0.001,
      actualLoops: 1,
    });
    const hot = node({
      nodeType: "Seq Scan",
      actualTotalTime: 0.5,
      actualLoops: 1,
    });
    const root = node({
      nodeType: "Nested Loop",
      actualTotalTime: 0.6,
      actualLoops: 1,
      children: [cheap, hot],
    });
    const result = analyzePlan({
      runtimeMs: 1,
      planningMs: null,
      executionMs: 0.6,
      root,
    });
    expect(result.hottest?.node.nodeType).toBe("Seq Scan");
  });
});

describe("formatRatio", () => {
  it("formats small ratios with one decimal", () => {
    expect(formatRatio(2.5)).toBe("2.5×");
  });

  it("rounds ratios ≥ 10× to integer", () => {
    expect(formatRatio(348)).toBe("348×");
  });

  it("compacts ratios ≥ 1000× with k", () => {
    expect(formatRatio(12500)).toBe("12.5k×");
  });
});
