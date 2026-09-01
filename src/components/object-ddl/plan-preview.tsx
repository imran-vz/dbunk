import { IconCopy } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import type { DdlPlanPreview } from "@/lib/store";
import { cn } from "@/lib/utils";

/**
 * Grouped, per-statement rendering of a `DdlPlanPreview` — shared by the
 * object-DDL review dialog and the table structure editor's pending-changes
 * section so both surfaces show the identical statement breakdown.
 */
export function DdlPlanPreviewGroups({ preview }: { preview: DdlPlanPreview }) {
  return (
    <div className="divide-y divide-border-subtle border-y border-border-subtle">
      {preview.groups.map((group, groupIndex) => {
        const indexes =
          group.kind === "atomic"
            ? group.statementIndexes
            : [group.statementIndex];
        return (
          <section
            key={`${group.kind}:${indexes.join(",")}`}
            aria-label={`DDL group ${groupIndex + 1}`}
          >
            <header className="bg-surface-panel px-3 py-2 text-2xs font-semibold text-text-secondary">
              Group {groupIndex + 1} ·{" "}
              {group.kind === "atomic" ? "Atomic" : "Standalone"}
            </header>
            {group.kind === "standalone" ? (
              <div className="border-l-2 border-warning bg-warning/10 px-3 py-2 text-xs text-text-secondary">
                Runs outside a transaction. Earlier statements stay applied if
                it fails.
              </div>
            ) : null}
            <div className="divide-y divide-border-subtle">
              {indexes.map((statementIndex) => {
                const statement = preview.statements[statementIndex];
                if (!statement) return null;
                return (
                  <article
                    key={statementIndex}
                    className={cn(
                      "min-w-0 border-l-2",
                      statement.destructive
                        ? "border-danger bg-danger/5"
                        : "border-transparent",
                    )}
                  >
                    <div className="flex items-center gap-2 px-3 py-2">
                      <span className="text-2xs text-text-muted">
                        {statementIndex + 1}
                      </span>
                      <strong className="min-w-0 flex-1 text-xs font-medium text-foreground">
                        {statement.summary}
                      </strong>
                      {statement.destructive ? (
                        <span className="text-2xs font-semibold text-danger">
                          Destructive
                        </span>
                      ) : null}
                      <Button
                        type="button"
                        size="xs"
                        variant="ghost"
                        onClick={() => {
                          void navigator.clipboard.writeText(statement.sql);
                        }}
                      >
                        <IconCopy /> Copy
                      </Button>
                    </div>
                    <pre className="overflow-x-auto border-t border-border-subtle p-3 font-mono text-2xs leading-relaxed whitespace-pre-wrap text-text-secondary">
                      {statement.sql}
                    </pre>
                  </article>
                );
              })}
            </div>
          </section>
        );
      })}
    </div>
  );
}
