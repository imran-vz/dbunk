import {
  IconChevronLeft,
  IconChevronRight,
  IconChevronsLeft,
  IconChevronsRight,
} from "@tabler/icons-react";
import { useMemo } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface PaginationProps {
  page: number;
  totalPages: number | undefined;
  isLastPage: boolean;
  isLoading: boolean;
  onFirst: () => void;
  onPrev: () => void;
  onNext: () => void;
  onLast: () => void;
  onJump: (page: number) => void;
}

export function Pagination({
  page,
  totalPages,
  isLastPage,
  isLoading,
  onFirst,
  onPrev,
  onNext,
  onLast,
  onJump,
}: PaginationProps) {
  const pageButtons = useMemo(() => {
    if (!totalPages) return [page];
    const pages: Array<number | "ellipsis-left" | "ellipsis-right"> = [];
    const window = 1;
    pages.push(1);
    if (page - window > 2) pages.push("ellipsis-left");
    for (
      let p = Math.max(2, page - window);
      p <= Math.min(totalPages - 1, page + window);
      p++
    ) {
      pages.push(p);
    }
    if (page + window < totalPages - 1) pages.push("ellipsis-right");
    if (totalPages > 1) pages.push(totalPages);
    return pages;
  }, [page, totalPages]);

  return (
    <div className="flex items-center gap-1">
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="First page"
        onClick={onFirst}
        disabled={page <= 1 || isLoading}
        className="size-6"
      >
        <IconChevronsLeft className="size-3.5" />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Previous page"
        onClick={onPrev}
        disabled={page <= 1 || isLoading}
        className="size-6"
      >
        <IconChevronLeft className="size-3.5" />
      </Button>
      <div className="flex items-center gap-0.5 px-1">
        {pageButtons.map((entry, idx) =>
          typeof entry === "number" ? (
            <button
              type="button"
              key={`p-${entry}`}
              aria-label={`Go to page ${entry}`}
              aria-current={entry === page ? "page" : undefined}
              onClick={() => onJump(entry)}
              className={cn(
                "h-6 min-w-6 rounded-sm px-1.5 text-xs tabular-nums transition-colors",
                entry === page
                  ? "bg-accent-green/15 text-accent-green-hover"
                  : "text-text-muted hover:bg-surface-panel-elevated hover:text-foreground",
              )}
            >
              {entry}
            </button>
          ) : (
            <span
              // biome-ignore lint/suspicious/noArrayIndexKey: ellipsis position is stable
              key={`${entry}-${idx}`}
              className="px-1 text-text-muted"
            >
              …
            </span>
          ),
        )}
      </div>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Next page"
        onClick={onNext}
        disabled={isLastPage || isLoading}
        className="size-6"
      >
        <IconChevronRight className="size-3.5" />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Last page"
        onClick={onLast}
        disabled={isLastPage || isLoading || totalPages === undefined}
        className="size-6"
      >
        <IconChevronsRight className="size-3.5" />
      </Button>
    </div>
  );
}
