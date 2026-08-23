import {
  IconAlertTriangle,
  IconChevronDown,
  IconChevronRight,
  IconFlame,
  IconInfoCircle,
  IconTarget,
} from "@tabler/icons-react";
import { useMemo, useState } from "react";

import type { ExplainPlanData } from "@/components/query-editor/results-view";
import { cn } from "@/lib/utils";

import {
  type AnalyzedNode,
  type AnalyzedPlan,
  analyzePlan,
  type EstimateAccuracy,
  formatMs,
  formatNumber,
  formatPct,
  formatRatio,
  type Insight,
  type LineageSegment,
} from "./plan-analysis";

export function ExplainView({
  data,
}: {
  data: Extract<ExplainPlanData, { kind: "json" }>;
}) {
  const analyzed = useMemo(() => analyzePlan(data), [data]);
  const initialSelection = analyzed.hottest?.id ?? analyzed.root.id;
  const [selectedId, setSelectedId] = useState<string>(initialSelection);
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());

  const selected =
    analyzed.flat.find((n) => n.id === selectedId) ?? analyzed.root;

  const toggleCollapsed = (id: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <ExplainSummary analyzed={analyzed} onJumpTo={setSelectedId} />
      <div className="flex min-h-0 flex-1 overflow-hidden">
        <ExplainWaterfall
          analyzed={analyzed}
          selectedId={selectedId}
          onSelect={setSelectedId}
          collapsed={collapsed}
          onToggleCollapsed={toggleCollapsed}
        />
        <ExplainDetail node={selected} totalExecMs={analyzed.totalExecMs} />
      </div>
    </div>
  );
}

function ExplainSummary({
  analyzed,
  onJumpTo,
}: {
  analyzed: AnalyzedPlan;
  onJumpTo: (id: string) => void;
}) {
  const { planningMs, executionMs, runtimeMs, insights } = analyzed;
  const total =
    planningMs != null && executionMs != null ? planningMs + executionMs : null;
  const planFrac =
    total != null && total > 0 && planningMs != null
      ? planningMs / total
      : null;
  const execFrac = planFrac == null ? null : 1 - planFrac;

  return (
    <div className="shrink-0 border-b border-border-subtle bg-surface-window">
      <div className="flex flex-wrap items-center gap-3 px-3 py-2">
        <div className="flex items-baseline gap-1.5">
          <span className="text-2xs uppercase tracking-wide text-text-muted">
            Total
          </span>
          <span className="font-mono text-sm font-semibold text-foreground">
            {formatMs(runtimeMs)}
          </span>
        </div>
        {planFrac != null && execFrac != null ? (
          <div className="flex min-w-[280px] flex-1 items-center gap-2">
            <SplitBar planFrac={planFrac} execFrac={execFrac} />
            <div className="flex shrink-0 items-baseline gap-2 font-mono text-2xs text-text-secondary">
              <span>
                <span className="text-text-muted">plan</span>{" "}
                {formatMs(planningMs)}
              </span>
              <span className="text-text-muted">·</span>
              <span>
                <span className="text-text-muted">exec</span>{" "}
                {formatMs(executionMs)}
              </span>
            </div>
          </div>
        ) : null}
      </div>
      {insights.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1.5 border-t border-border-subtle/60 px-3 py-2">
          {insights.map((insight) => (
            <InsightChip
              key={insight.id}
              insight={insight}
              onClick={() =>
                insight.nodeId ? onJumpTo(insight.nodeId) : undefined
              }
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}

/* oxlint-disable jsx-a11y/prefer-tag-over-role -- The styled bar is a graphical summary, not an image resource. */
function SplitBar({
  planFrac,
  execFrac,
}: {
  planFrac: number;
  execFrac: number;
}) {
  return (
    /* oxlint-disable-next-line jsx-a11y/prefer-tag-over-role -- This styled bar is a graphical summary, not an image resource. */
    <div
      role="img"
      className="flex h-1.5 flex-1 overflow-hidden rounded-full bg-surface-panel"
      aria-label={`Planning ${(planFrac * 100).toFixed(0)}%, execution ${(
        execFrac * 100
      ).toFixed(0)}%`}
    >
      <div
        className="h-full bg-text-muted/60"
        style={{ width: `${planFrac * 100}%` }}
      />
      <div
        className="h-full bg-accent"
        style={{ width: `${execFrac * 100}%` }}
      />
    </div>
  );
}

/* oxlint-enable jsx-a11y/prefer-tag-over-role */
function InsightChip({
  insight,
  onClick,
}: {
  insight: Insight;
  onClick: () => void;
}) {
  const Icon =
    insight.severity === "danger"
      ? IconFlame
      : insight.severity === "warn"
        ? IconAlertTriangle
        : IconInfoCircle;
  const styles =
    insight.severity === "danger"
      ? "bg-danger/10 text-danger border-danger/30"
      : insight.severity === "warn"
        ? "bg-warning/15 text-warning border-warning/30"
        : "bg-surface-panel text-text-secondary border-border-subtle";
  const clickable = Boolean(insight.nodeId);
  return (
    <button
      type="button"
      onClick={clickable ? onClick : undefined}
      disabled={!clickable}
      className={cn(
        "flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-2xs font-medium transition-colors",
        styles,
        clickable
          ? "cursor-pointer hover:brightness-110"
          : "cursor-default opacity-90",
      )}
    >
      <Icon className="size-3" />
      <span>{insight.message}</span>
    </button>
  );
}

function ExplainWaterfall({
  analyzed,
  selectedId,
  onSelect,
  collapsed,
  onToggleCollapsed,
}: {
  analyzed: AnalyzedPlan;
  selectedId: string;
  onSelect: (id: string) => void;
  collapsed: Set<string>;
  onToggleCollapsed: (id: string) => void;
}) {
  const visible: AnalyzedNode[] = [];
  const pushVisible = (node: AnalyzedNode) => {
    visible.push(node);
    if (collapsed.has(node.id)) return;
    for (const child of node.children) pushVisible(child);
  };
  pushVisible(analyzed.root);

  return (
    <div className="min-h-0 flex-1 overflow-auto bg-surface-app">
      <div className="grid grid-cols-[1fr_120px_110px_60px] items-center gap-2 border-b border-border-subtle bg-surface-window px-3 py-1.5 text-2xs font-semibold uppercase tracking-wide text-text-muted">
        <div>Plan</div>
        <div>Self time</div>
        <div>Rows act / plan</div>
        <div className="text-right">Loops</div>
      </div>
      <div className="divide-y divide-border-subtle/50" role="tree">
        {visible.map((n) => (
          <ExplainRow
            key={n.id}
            node={n}
            maxSelfMs={analyzed.maxSelfMs}
            selected={selectedId === n.id}
            collapsed={collapsed.has(n.id)}
            onSelect={() => onSelect(n.id)}
            onToggle={() => onToggleCollapsed(n.id)}
          />
        ))}
      </div>
    </div>
  );
}

/* oxlint-disable jsx-a11y/prefer-tag-over-role -- The styled bar is a graphical summary, not an image resource. */
function ExplainRow({
  node,
  maxSelfMs,
  selected,
  collapsed,
  onSelect,
  onToggle,
}: {
  node: AnalyzedNode;
  maxSelfMs: number;
  selected: boolean;
  collapsed: boolean;
  onSelect: () => void;
  onToggle: () => void;
}) {
  const hasChildren = node.children.length > 0;
  const fillPct =
    node.selfMs != null && maxSelfMs > 0
      ? Math.max(2, (node.selfMs / maxSelfMs) * 100)
      : 0;
  const heatClass = heatColor(node.selfPct);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onSelect();
    }
  };

  return (
    <div
      className={cn(
        "grid grid-cols-[1fr_120px_110px_60px] items-center gap-2 px-3 py-1.5 text-xs transition-colors",
        selected
          ? "bg-accent/10"
          : "hover:bg-surface-row-hover/60 cursor-pointer",
      )}
      role="treeitem"
      aria-selected={selected}
      aria-expanded={hasChildren ? !collapsed : undefined}
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={handleKeyDown}
    >
      <div className="flex min-w-0 items-center">
        <TreeConnectors lineage={node.parentLineage} depth={node.depth} />
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            if (hasChildren) onToggle();
          }}
          aria-label={
            hasChildren
              ? collapsed
                ? "Expand subtree"
                : "Collapse subtree"
              : undefined
          }
          className={cn(
            "mr-1 flex size-4 shrink-0 items-center justify-center rounded-sm text-text-muted",
            hasChildren ? "hover:bg-surface-panel-elevated" : "invisible",
          )}
        >
          {hasChildren ? (
            collapsed ? (
              <IconChevronRight className="size-3" />
            ) : (
              <IconChevronDown className="size-3" />
            )
          ) : null}
        </button>
        <span className="truncate text-sm font-medium text-foreground">
          {node.node.nodeType}
        </span>
        {node.node.relation ? (
          <span className="ml-1.5 truncate rounded-sm bg-surface-panel px-1.5 py-0.5 font-mono text-2xs text-text-secondary">
            {node.node.relation}
          </span>
        ) : null}
        {node.node.alias && node.node.alias !== node.node.relation ? (
          <span className="ml-1.5 truncate font-mono text-2xs text-text-muted">
            {node.node.alias}
          </span>
        ) : null}
        {selected ? (
          <IconTarget className="ml-1.5 size-3 shrink-0 text-accent" />
        ) : null}
        <div className="ml-auto flex shrink-0 items-center gap-1 pl-2">
          {node.node.buffers.slice(0, 2).map((buffer) => (
            <span
              key={buffer}
              className="hidden rounded-sm border border-border-subtle/60 bg-surface-panel/60 px-1 py-0.5 font-mono text-2xs text-text-muted md:inline"
            >
              {buffer}
            </span>
          ))}
        </div>
      </div>

      <div className="flex items-center gap-2">
        {/* oxlint-disable-next-line jsx-a11y/prefer-tag-over-role -- This styled bar is a graphical summary, not an image resource. */}
        <div
          role="img"
          className="relative h-1.5 flex-1 overflow-hidden rounded-full bg-surface-panel"
          aria-label={`self time ${formatMs(node.selfMs)}`}
        >
          <div
            className={cn("h-full transition-all", heatClass)}
            style={{ width: `${fillPct}%` }}
          />
        </div>
        <span className="shrink-0 font-mono text-2xs tabular-nums text-text-secondary">
          {formatMs(node.selfMs)}
        </span>
      </div>

      <div className="flex items-center gap-1 font-mono text-2xs tabular-nums text-text-secondary">
        <span>
          {formatNumber(node.node.actualRows)} /{" "}
          <span className="text-text-muted">
            {formatNumber(node.node.planRows)}
          </span>
        </span>
        <EstimateChip estimate={node.estimate} />
      </div>

      <div className="text-right font-mono text-2xs tabular-nums text-text-muted">
        {node.node.actualLoops != null
          ? node.node.actualLoops.toLocaleString()
          : "—"}
      </div>
    </div>
  );
}

/* oxlint-enable jsx-a11y/prefer-tag-over-role */
function TreeConnectors({
  lineage,
  depth,
}: {
  lineage: LineageSegment[];
  depth: number;
}) {
  if (depth === 0) return null;
  return (
    <div className="mr-1 flex h-5 shrink-0 items-stretch self-stretch">
      {lineage.map((seg) => (
        <div
          key={seg.ancestorId}
          className={cn(
            "w-3",
            seg.hasMore && "border-l border-border-subtle/70",
          )}
        />
      ))}
    </div>
  );
}

function EstimateChip({ estimate }: { estimate: EstimateAccuracy }) {
  if (estimate.kind === "unknown") return null;
  if (estimate.kind === "accurate") return null;
  const label = formatRatio(estimate.ratio);
  const direction = estimate.kind === "over" ? "over" : "under";
  return (
    <span
      className="ml-0.5 inline-flex items-center gap-0.5 rounded-sm bg-warning/15 px-1 py-0.5 text-2xs font-medium text-warning"
      title={`Planner ${direction}estimated rows by ${label}`}
    >
      <IconAlertTriangle className="size-2.5" />
      {label}
    </span>
  );
}

function ExplainDetail({
  node,
  totalExecMs,
}: {
  node: AnalyzedNode;
  totalExecMs: number | null;
}) {
  return (
    <aside className="hidden w-72 shrink-0 flex-col overflow-auto border-l border-border-subtle bg-surface-window md:flex">
      <div className="border-b border-border-subtle px-3 py-2">
        <div className="text-2xs uppercase tracking-wide text-text-muted">
          Selected node
        </div>
        <div className="mt-0.5 text-sm font-semibold text-foreground">
          {node.node.nodeType}
        </div>
        {node.node.relation ? (
          <div className="mt-1 font-mono text-2xs text-text-secondary">
            {node.node.relation}
            {node.node.alias && node.node.alias !== node.node.relation ? (
              <span className="ml-1 text-text-muted">
                · alias {node.node.alias}
              </span>
            ) : null}
          </div>
        ) : null}
      </div>

      <dl className="grid grid-cols-2 gap-x-3 gap-y-2 px-3 py-3 text-2xs">
        <DetailMetric label="Self time" value={formatMs(node.selfMs)} />
        <DetailMetric label="% of exec" value={formatPct(node.selfPct)} />
        <DetailMetric
          label="Inclusive"
          value={formatMs(node.inclusiveMs)}
          help={
            totalExecMs && node.inclusiveMs
              ? `${formatPct(node.inclusiveMs / totalExecMs)} of total`
              : undefined
          }
        />
        <DetailMetric
          label="Loops"
          value={
            node.node.actualLoops != null
              ? node.node.actualLoops.toLocaleString()
              : "—"
          }
        />
        <DetailMetric
          label="Rows actual"
          value={formatNumber(node.node.actualRows)}
        />
        <DetailMetric
          label="Rows planned"
          value={formatNumber(node.node.planRows)}
        />
        <DetailMetric
          label="Cost"
          value={formatCostRange(node.node.startupCost, node.node.totalCost)}
          span
        />
        <DetailMetric
          label="Actual time"
          value={formatTimeRange(
            node.node.actualStartupTime,
            node.node.actualTotalTime,
          )}
          span
        />
      </dl>

      <EstimateBlock estimate={node.estimate} />

      {node.node.buffers.length > 0 ? (
        <div className="border-t border-border-subtle px-3 py-3">
          <div className="text-2xs uppercase tracking-wide text-text-muted">
            Buffers
          </div>
          <div className="mt-1.5 flex flex-wrap gap-1">
            {node.node.buffers.map((buffer) => (
              <span
                key={buffer}
                className="rounded-sm border border-border-subtle bg-surface-panel px-1.5 py-0.5 font-mono text-2xs text-text-secondary"
              >
                {buffer}
              </span>
            ))}
          </div>
        </div>
      ) : null}
    </aside>
  );
}

function DetailMetric({
  label,
  value,
  help,
  span,
}: {
  label: string;
  value: string;
  help?: string;
  span?: boolean;
}) {
  return (
    <div className={cn("min-w-0", span && "col-span-2")}>
      <dt className="truncate text-2xs uppercase tracking-wide text-text-muted">
        {label}
      </dt>
      <dd className="truncate font-mono text-xs text-foreground">{value}</dd>
      {help ? (
        <div className="mt-0.5 truncate text-2xs text-text-muted">{help}</div>
      ) : null}
    </div>
  );
}

function EstimateBlock({ estimate }: { estimate: EstimateAccuracy }) {
  if (estimate.kind === "unknown") return null;
  if (estimate.kind === "accurate") {
    return (
      <div className="border-t border-border-subtle px-3 py-3">
        <div className="text-2xs uppercase tracking-wide text-text-muted">
          Estimate
        </div>
        <div className="mt-1 flex items-center gap-1.5 text-xs text-accent">
          <span className="inline-block size-1.5 rounded-full bg-accent" />
          <span>Within {formatRatio(estimate.ratio)} of actual</span>
        </div>
      </div>
    );
  }
  const direction = estimate.kind === "over" ? "overestimate" : "underestimate";
  const hint =
    estimate.kind === "over"
      ? "Planner expected more rows than arrived. ANALYZE the table or check filter selectivity."
      : "Planner expected fewer rows than arrived. ANALYZE the table or refine statistics target.";
  return (
    <div className="border-t border-border-subtle px-3 py-3">
      <div className="flex items-center gap-1.5 text-2xs uppercase tracking-wide text-warning">
        <IconAlertTriangle className="size-3" />
        Estimate miss
      </div>
      <div className="mt-1 font-mono text-xs text-foreground">
        {formatRatio(estimate.ratio)} {direction}
      </div>
      <p className="mt-1.5 text-2xs leading-relaxed text-text-muted">{hint}</p>
    </div>
  );
}

function heatColor(selfPct: number | null): string {
  if (selfPct == null) return "bg-text-muted/40";
  if (selfPct >= 0.5) return "bg-danger";
  if (selfPct >= 0.25) return "bg-warning";
  if (selfPct >= 0.05) return "bg-accent";
  return "bg-accent/70";
}

function formatCostRange(
  start: number | undefined,
  end: number | undefined,
): string {
  if (start == null && end == null) return "—";
  if (start == null) return formatNumber(end ?? null);
  if (end == null) return formatNumber(start);
  return `${formatNumber(start)} .. ${formatNumber(end)}`;
}

function formatTimeRange(
  start: number | undefined,
  end: number | undefined,
): string {
  if (start == null && end == null) return "—";
  if (start == null) return formatMs(end ?? null);
  if (end == null) return formatMs(start);
  return `${formatMs(start)} → ${formatMs(end)}`;
}
