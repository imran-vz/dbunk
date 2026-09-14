import { IconChevronLeft, IconChevronRight } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import { formatPagePosition } from "@/lib/pg-schema-compare/presentation";
import type { SchemaCompareObjectSummary } from "@/lib/pg-schema-compare/protocol";
import {
  objectIdentity,
  relationKey,
  type SchemaCompareObjectPage,
} from "@/lib/pg-schema-compare/reader";
import { cn } from "@/lib/utils";

import { DifferenceBadge } from "./coverage";

export function PagerControls({
  label,
  position,
  hasPrevious,
  hasNext,
  onPrevious,
  onNext,
}: {
  label: string;
  position: string;
  hasPrevious: boolean;
  hasNext: boolean;
  onPrevious: () => void;
  onNext: () => void;
}) {
  return (
    <div className="flex items-center gap-1">
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={`Previous ${label} page`}
        disabled={!hasPrevious}
        onClick={onPrevious}
      >
        <IconChevronLeft />
      </Button>
      <span className="text-2xs text-text-muted tabular-nums">{position}</span>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={`Next ${label} page`}
        disabled={!hasNext}
        onClick={onNext}
      >
        <IconChevronRight />
      </Button>
    </div>
  );
}

const sameObject = (
  a: SchemaCompareObjectSummary | null,
  b: SchemaCompareObjectSummary,
) =>
  a !== null &&
  relationKey(objectIdentity(a)) === relationKey(objectIdentity(b));

/** One explicit server page of object summaries; nothing is derived globally. */
export function ObjectList({
  page,
  total,
  selected,
  onSelect,
  onPage,
}: {
  page: SchemaCompareObjectPage;
  total: number;
  selected: SchemaCompareObjectSummary | null;
  onSelect: (object: SchemaCompareObjectSummary) => void;
  onPage: (direction: "next" | "previous") => void;
}) {
  return (
    <section
      aria-label="Compared objects"
      className="flex min-h-0 min-w-0 flex-col"
    >
      <div className="flex h-(--h-toolbar) shrink-0 items-center justify-between gap-2 border-b border-border-subtle px-(--pad-panel)">
        <h2 className="text-xs font-semibold">Objects</h2>
        <PagerControls
          label="object"
          position={formatPagePosition(page.offset, page.items.length, total)}
          hasPrevious={page.offset > 0}
          hasNext={page.nextOffset !== null}
          onPrevious={() => onPage("previous")}
          onNext={() => onPage("next")}
        />
      </div>
      {page.items.length === 0 ? (
        <p className="p-(--pad-panel) text-xs text-text-muted">
          This page has no objects.
        </p>
      ) : (
        <ul className="min-h-0 flex-1 overflow-auto">
          {page.items.map((object) => {
            const identity = objectIdentity(object);
            const active = sameObject(selected, object);
            return (
              <li key={relationKey(identity)}>
                <button
                  type="button"
                  aria-pressed={active}
                  onClick={() => onSelect(object)}
                  className={cn(
                    "flex h-(--row-tree) w-full items-center gap-2 px-(--pad-panel) text-left text-xs outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-accent",
                    active
                      ? "bg-accent-subdued text-accent"
                      : "hover:bg-surface-row-hover",
                  )}
                >
                  <span className="min-w-0 flex-1 truncate font-mono">
                    {identity.name}
                  </span>
                  <DifferenceBadge
                    kind={object.kind}
                    className="shrink-0 text-2xs"
                  />
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
