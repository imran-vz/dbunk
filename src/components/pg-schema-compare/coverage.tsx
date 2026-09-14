import {
  DIFFERENCE_LABEL,
  DIFFERENCE_TONE,
  type DifferenceKind,
  EXCLUDED_CATEGORY,
  formatExcludedCount,
  formatServerVersion,
} from "@/lib/pg-schema-compare/presentation";
import type { SchemaCompareMetadataDetail } from "@/lib/pg-schema-compare/reader";
import { cn } from "@/lib/utils";

export const TONE_CLASS = {
  success: "text-success",
  warning: "text-warning",
  info: "text-info",
  danger: "text-danger",
  neutral: "text-text-secondary",
} as const;

export function DifferenceBadge({
  kind,
  className,
}: {
  kind: DifferenceKind;
  className?: string;
}) {
  return (
    <span className={cn(TONE_CLASS[DIFFERENCE_TONE[kind]], className)}>
      {DIFFERENCE_LABEL[kind]}
    </span>
  );
}

function ExcludedCounts({
  label,
  counts,
}: {
  label: string;
  counts: SchemaCompareMetadataDetail["sourceExcludedCounts"];
}) {
  if (counts.length === 0) return null;
  return (
    <div>
      <div className="text-2xs font-semibold tracking-wide text-text-muted uppercase">
        {label} exclusions
      </div>
      <dl className="mt-1 grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-0.5">
        {counts.map((entry) => (
          <div key={entry.category} className="contents">
            <dt className="text-text-secondary">
              {EXCLUDED_CATEGORY[entry.category]}
            </dt>
            <dd className="text-right font-mono tabular-nums">
              {formatExcludedCount(entry.count, entry.complete)}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

/**
 * Scope line above the result: overall kind, the named projection and a
 * route to exclusions, capture times, server versions and consistency.
 */
export function CoverageSummary({
  detail,
}: {
  detail: SchemaCompareMetadataDetail;
}) {
  const { metadata } = detail;
  const { coverage } = metadata;
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-border-subtle px-3 py-1.5 text-xs">
      <DifferenceBadge kind={detail.kind} className="font-medium" />
      <span className="text-text-secondary">
        PostgreSQL 16 · Ordinary tables · {detail.objectCount}{" "}
        {detail.objectCount === 1 ? "object" : "objects"}
      </span>
      {coverage.incomparableFields > 0 ? (
        <span className="text-warning">
          {coverage.incomparableFields}{" "}
          {coverage.incomparableFields === 1 ? "field" : "fields"} not
          comparable
        </span>
      ) : null}
      {coverage.excludedRelations > 0 ? (
        <span className="text-text-secondary">
          {coverage.excludedRelations} excluded{" "}
          {coverage.excludedRelations === 1 ? "relation" : "relations"}
        </span>
      ) : null}
      <details className="min-w-0 basis-full">
        <summary className="cursor-default text-text-secondary select-none">
          Coverage &amp; capture
        </summary>
        <div className="mt-1.5 grid gap-3 border-l-2 border-border-subtle pl-3 @xl:grid-cols-2">
          <div className="space-y-1.5">
            <p>
              Columns, constraints, indexes, table persistence and comments of
              ordinary tables. Type and collation references are compared by
              name only.
            </p>
            <p className="text-text-secondary">
              Scope <span className="font-mono">{coverage.scope}</span>,
              normalization {coverage.normalizationVersion}. Partitioned,
              inherited, foreign and extension-owned tables are excluded.
            </p>
            {coverage.excludedCategories.length ? (
              <p className="text-text-secondary">
                Not compared:{" "}
                {coverage.excludedCategories
                  .map((category) => EXCLUDED_CATEGORY[category])
                  .join("; ")}
                .
              </p>
            ) : null}
            <p className="text-text-secondary">
              Source: {formatServerVersion(metadata.source.serverVersion)} ·{" "}
              <span className="font-mono">{metadata.source.capturedAt}</span>
              <br />
              Target: {formatServerVersion(
                metadata.target.serverVersion,
              )} ·{" "}
              <span className="font-mono">{metadata.target.capturedAt}</span>
            </p>
            <p className="text-text-secondary">
              {metadata.consistency === "sharedTransaction"
                ? "Both schemas were read in one transaction on the same connection."
                : "Independent captures. This is not a single cross-database snapshot."}
            </p>
          </div>
          <div className="space-y-3">
            <ExcludedCounts
              label="Source"
              counts={detail.sourceExcludedCounts}
            />
            <ExcludedCounts
              label="Target"
              counts={detail.targetExcludedCounts}
            />
          </div>
        </div>
      </details>
    </div>
  );
}
