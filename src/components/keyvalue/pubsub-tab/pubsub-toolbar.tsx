/**
 * Top-of-tab toolbar for the Pub/Sub workspace: pattern input,
 * subscribe button, pause/resume toggle (visible only when a
 * subscription is live), clear button, and the buffered-message
 * count.
 *
 * Extracted from `PubsubTab` to keep the parent component below
 * fallow's cognitive-complexity threshold.
 */

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

interface PubsubToolbarProps {
  patternInput: string;
  onPatternInputChange: (value: string) => void;
  onSubscribe: () => void;
  hasActivePatterns: boolean;
  paused: boolean;
  onTogglePaused: () => void;
  onClear: () => void;
  bufferedCount: number;
}

export function PubsubToolbar({
  patternInput,
  onPatternInputChange,
  onSubscribe,
  hasActivePatterns,
  paused,
  onTogglePaused,
  onClear,
  bufferedCount,
}: PubsubToolbarProps) {
  const trimmedInput = patternInput.trim();

  return (
    <header className="flex flex-wrap items-center gap-2 border-b border-border-subtle bg-surface-panel/60 px-4 py-2 text-xs">
      <Input
        value={patternInput}
        onChange={(event) => onPatternInputChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            onSubscribe();
          }
        }}
        placeholder="Channel pattern (e.g. notifications.*)"
        className="h-7 max-w-xs text-xs"
      />
      <Button
        size="sm"
        className="h-7 px-2 text-xs"
        disabled={!trimmedInput}
        onClick={onSubscribe}
      >
        Subscribe
      </Button>
      {hasActivePatterns ? (
        <Button
          size="sm"
          variant={paused ? "default" : "outline"}
          className="h-7 px-2 text-xs"
          onClick={onTogglePaused}
        >
          {paused ? "Resume" : "Pause"}
        </Button>
      ) : null}
      <Button
        size="sm"
        variant="ghost"
        className="h-7 px-2 text-xs"
        onClick={onClear}
      >
        Clear
      </Button>
      <span className="ml-auto text-text-muted">
        {bufferedCount.toLocaleString()} messages buffered
      </span>
    </header>
  );
}
