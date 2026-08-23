/**
 * Results status strip (DESIGN-SYSTEM §5.3) — the collapsed form of the
 * results pane. Always present under the editor while results are
 * collapsed: row count · runtime, plus an expand chevron. Clicking
 * anywhere on it (or `Cmd+J`) restores the pane to its previous size.
 */

import { IconChevronUp } from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";

interface ResultsStatusStripProps {
  /** e.g. "1,204 rows · 128 ms"; null when nothing has run yet. */
  summary: string | null;
  isRunning: boolean;
  onExpand: () => void;
}

export function ResultsStatusStrip({
  summary,
  isRunning,
  onExpand,
}: ResultsStatusStripProps) {
  return (
    <button
      type="button"
      data-testid="results-status-strip"
      onClick={onExpand}
      aria-label="Show results pane"
      className="flex h-(--h-statusbar) w-full shrink-0 items-center gap-2 border-t border-border-subtle bg-surface-window px-3 text-xs text-text-muted transition-colors hover:text-foreground"
    >
      <span className="truncate tabular-nums">
        {isRunning ? (
          <>
            Running · <ElapsedTime />
          </>
        ) : (
          (summary ?? "No results yet")
        )}
      </span>
      <IconChevronUp className="ml-auto size-3.5 shrink-0" />
    </button>
  );
}

/**
 * Live elapsed timer (§5.1) — ticks while mounted. Mount it when a run
 * starts; it measures from mount (or the given start) at 10Hz.
 */
export function ElapsedTime({ sinceMs }: { sinceMs?: number }) {
  const startRef = useRef(sinceMs ?? Date.now());
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 100);
    return () => clearInterval(timer);
  }, []);
  const elapsed = Math.max(0, now - startRef.current);
  return <span className="tabular-nums">{(elapsed / 1000).toFixed(1)} s</span>;
}
