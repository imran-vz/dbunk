import type { ExplainPlanNode } from "@/components/query-editor/results-view";

export type EstimateAccuracy =
  | { kind: "unknown" }
  | { kind: "accurate"; ratio: number }
  | { kind: "over"; ratio: number }
  | { kind: "under"; ratio: number };

export type LineageSegment = { ancestorId: string; hasMore: boolean };

export type AnalyzedNode = {
  node: ExplainPlanNode;
  id: string;
  depth: number;
  isLastChild: boolean;
  parentLineage: LineageSegment[];
  inclusiveMs: number | null;
  selfMs: number | null;
  selfPct: number | null;
  estimate: EstimateAccuracy;
  children: AnalyzedNode[];
};

export type Insight = {
  id: string;
  severity: "info" | "warn" | "danger";
  message: string;
  nodeId?: string;
};

export type AnalyzedPlan = {
  root: AnalyzedNode;
  totalExecMs: number | null;
  planningMs: number | null;
  executionMs: number | null;
  runtimeMs: number;
  maxSelfMs: number;
  hottest: AnalyzedNode | null;
  flat: AnalyzedNode[];
  insights: Insight[];
};

const ESTIMATE_WARN_RATIO = 2;
const PLANNING_SKEW_RATIO = 10;
const SEQ_SCAN_TYPES = new Set(["Seq Scan"]);

export function analyzePlan(data: {
  runtimeMs: number;
  planningMs: number | null;
  executionMs: number | null;
  root: ExplainPlanNode;
}): AnalyzedPlan {
  const root = analyzeNode(data.root, "0", 0, true, []);
  const flat: AnalyzedNode[] = [];
  walk(root, (n) => flat.push(n));

  const totalExecMs = root.inclusiveMs;
  if (totalExecMs != null && totalExecMs > 0) {
    for (const n of flat) {
      if (n.selfMs != null) {
        n.selfPct = n.selfMs / totalExecMs;
      }
    }
  }

  const maxSelfMs = flat.reduce((max, n) => Math.max(max, n.selfMs ?? 0), 0);

  let hottest: AnalyzedNode | null = null;
  for (const n of flat) {
    if (n.selfMs == null) continue;
    if (!hottest || (hottest.selfMs ?? 0) < n.selfMs) hottest = n;
  }

  return {
    root,
    totalExecMs,
    planningMs: data.planningMs,
    executionMs: data.executionMs,
    runtimeMs: data.runtimeMs,
    maxSelfMs,
    hottest,
    flat,
    insights: deriveInsights({
      flat,
      planningMs: data.planningMs,
      executionMs: data.executionMs ?? totalExecMs,
    }),
  };
}

function analyzeNode(
  node: ExplainPlanNode,
  id: string,
  depth: number,
  isLastChild: boolean,
  parentLineage: LineageSegment[],
): AnalyzedNode {
  const inclusive = computeInclusiveMs(node);
  const children = node.children.map((child, i, arr) => {
    const childLineage =
      depth === 0
        ? []
        : [...parentLineage, { ancestorId: id, hasMore: !isLastChild }];
    return analyzeNode(
      child,
      `${id}.${i}`,
      depth + 1,
      i === arr.length - 1,
      childLineage,
    );
  });

  let selfMs: number | null = null;
  if (inclusive != null) {
    const anyChildUnknown = children.some((c) => c.inclusiveMs == null);
    if (anyChildUnknown) {
      selfMs = inclusive;
    } else {
      const childSum = children.reduce(
        (sum, c) => sum + (c.inclusiveMs ?? 0),
        0,
      );
      selfMs = Math.max(0, inclusive - childSum);
    }
  }

  return {
    node,
    id,
    depth,
    isLastChild,
    parentLineage,
    inclusiveMs: inclusive,
    selfMs,
    selfPct: null,
    estimate: computeEstimate(node),
    children,
  };
}

function computeInclusiveMs(node: ExplainPlanNode): number | null {
  if (node.actualTotalTime == null) return null;
  const loops = node.actualLoops ?? 1;
  return node.actualTotalTime * loops;
}

function computeEstimate(node: ExplainPlanNode): EstimateAccuracy {
  if (node.actualRows == null || node.planRows == null) {
    return { kind: "unknown" };
  }
  const actual = Math.max(node.actualRows, 0);
  const plan = Math.max(node.planRows, 0);
  if (actual === plan) return { kind: "accurate", ratio: 1 };

  if (actual > plan) {
    const ratio = actual / Math.max(plan, 1);
    return ratio < ESTIMATE_WARN_RATIO
      ? { kind: "accurate", ratio }
      : { kind: "under", ratio };
  }
  const ratio = plan / Math.max(actual, 1);
  return ratio < ESTIMATE_WARN_RATIO
    ? { kind: "accurate", ratio }
    : { kind: "over", ratio };
}

function walk(node: AnalyzedNode, visit: (n: AnalyzedNode) => void): void {
  visit(node);
  for (const child of node.children) walk(child, visit);
}

function ratioOf(est: EstimateAccuracy): number {
  return est.kind === "over" || est.kind === "under" ? est.ratio : 0;
}

function deriveInsights(args: {
  flat: AnalyzedNode[];
  planningMs: number | null;
  executionMs: number | null;
}): Insight[] {
  const insights: Insight[] = [];
  const { flat, planningMs, executionMs } = args;

  if (
    planningMs != null &&
    executionMs != null &&
    executionMs > 0 &&
    planningMs / executionMs >= PLANNING_SKEW_RATIO
  ) {
    const ratio = (planningMs / executionMs).toFixed(0);
    insights.push({
      id: "planning-skew",
      severity: "info",
      message: `Planning took ${ratio}× execution — consider prepared statements`,
    });
  }

  const worstMiss = flat.reduce<AnalyzedNode | null>((worst, n) => {
    if (n.estimate.kind !== "over" && n.estimate.kind !== "under") return worst;
    if (!worst) return n;
    return ratioOf(n.estimate) > ratioOf(worst.estimate) ? n : worst;
  }, null);
  if (
    worstMiss &&
    (worstMiss.estimate.kind === "over" || worstMiss.estimate.kind === "under")
  ) {
    const direction =
      worstMiss.estimate.kind === "over" ? "overestimate" : "underestimate";
    insights.push({
      id: "estimate-miss",
      severity: "warn",
      message: `Plan-vs-actual ${direction} of ${formatRatio(
        worstMiss.estimate.ratio,
      )} on ${worstMiss.node.nodeType}`,
      nodeId: worstMiss.id,
    });
  }

  const seqScans = flat.filter((n) => SEQ_SCAN_TYPES.has(n.node.nodeType));
  if (seqScans.length > 0) {
    const hasIndexScan = flat.some((n) => /Index/.test(n.node.nodeType));
    insights.push({
      id: "seq-scans",
      severity: hasIndexScan ? "info" : "warn",
      message: hasIndexScan
        ? `${seqScans.length} sequential scan${seqScans.length > 1 ? "s" : ""}`
        : `${seqScans.length} sequential scan${seqScans.length > 1 ? "s" : ""}, no indexes used`,
    });
  }

  const spill = flat.find((n) =>
    n.node.buffers.some((b) => /temp written/i.test(b)),
  );
  if (spill) {
    insights.push({
      id: "spill",
      severity: "danger",
      message: `${spill.node.nodeType} spilled to disk`,
      nodeId: spill.id,
    });
  }

  return insights;
}

export function formatRatio(ratio: number): string {
  if (ratio >= 1000) return `${(ratio / 1000).toFixed(1)}k×`;
  if (ratio >= 10) return `${ratio.toFixed(0)}×`;
  return `${ratio.toFixed(1)}×`;
}

export function formatMs(value: number | null | undefined): string {
  if (value == null) return "—";
  if (value < 0.001) return "<0.001 ms";
  if (value < 1) return `${value.toFixed(3)} ms`;
  if (value < 100) return `${value.toFixed(2)} ms`;
  return `${value.toFixed(1)} ms`;
}

export function formatNumber(value: number | null | undefined): string {
  return value == null ? "—" : value.toLocaleString();
}

export function formatPct(value: number | null | undefined): string {
  if (value == null) return "—";
  if (value < 0.001) return "<0.1%";
  return `${(value * 100).toFixed(1)}%`;
}
