import { useMemo, useState } from "react";

import { cn } from "@/lib/utils";

interface RouteErrorBoundaryProps {
  error: Error;
  reset?: () => void;
}

export function RouteErrorBoundary({ error, reset }: RouteErrorBoundaryProps) {
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState(false);

  const name = error.name || "Error";
  const message = error.message || "Unknown error";
  const stack = error.stack ?? "";
  const timestamp = useMemo(() => formatTimestamp(new Date()), []);
  const location = useMemo(() => firstStackFrame(stack), [stack]);
  const stackFrameCount = useMemo(
    () =>
      stack.split("\n").filter((line) => line.trim().startsWith("at ")).length,
    [stack],
  );

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(
        [
          `TYPE      ${name}`,
          `MESSAGE   ${message}`,
          `TIME      ${timestamp}`,
          location ? `WHERE     ${location}` : null,
          "",
          "STACK",
          stack || "(unavailable)",
        ]
          .filter(Boolean)
          .join("\n"),
      );
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // Some webviews block clipboard access; fail silently.
    }
  };

  return (
    <div className="relative min-h-screen w-full overflow-hidden bg-surface-app text-foreground">
      <FadingGridBackdrop />

      <div className="relative z-10 flex min-h-screen w-full flex-col items-center justify-center px-6 py-16">
        <div className="w-full max-w-3xl">
          <div className="mb-6 flex items-center justify-between text-2xs uppercase tracking-[0.2em] text-text-muted">
            <span>dbunk · query log</span>
            <span className="flex items-center gap-2">
              <span className="size-1.5 animate-pulse rounded-full bg-danger" />
              <span>halted at {timestamp}</span>
            </span>
          </div>

          <ErrorDataGrid
            message={message}
            errorId={shortHash(message)}
            reset={reset}
          />

          <div className="mt-5 flex flex-wrap items-center gap-x-5 gap-y-2 text-xs text-text-muted">
            <span className="font-mono text-text-secondary">{name}</span>
            {location ? (
              <span className="truncate font-mono text-text-secondary">
                {location}
              </span>
            ) : null}
            <div className="ml-auto flex items-center gap-4">
              <button
                type="button"
                onClick={() => setExpanded((current) => !current)}
                aria-expanded={expanded}
                className="transition-colors hover:text-foreground"
              >
                {expanded ? "▾ hide stack" : "▸ show stack"}
                {stackFrameCount > 0 ? ` (${stackFrameCount})` : ""}
              </button>
              <button
                type="button"
                onClick={() => window.location.reload()}
                className="transition-colors hover:text-foreground"
              >
                reload app
              </button>
              <button
                type="button"
                onClick={handleCopy}
                className="transition-colors hover:text-foreground"
              >
                {copied ? "copied" : "copy report"}
              </button>
            </div>
          </div>

          {expanded ? (
            <pre className="mt-4 max-h-80 overflow-auto whitespace-pre-wrap rounded-md border border-border-subtle bg-surface-window/80 p-3 font-mono text-2xs leading-relaxed text-text-secondary backdrop-blur">
              {stack || "(no stack trace available)"}
            </pre>
          ) : null}

          <div className="mt-6 text-2xs uppercase tracking-[0.2em] text-text-muted">
            report at{" "}
            <a
              href="https://github.com/imran-vz/dbunk/issues"
              target="_blank"
              rel="noopener noreferrer"
              className="font-mono text-text-secondary normal-case tracking-normal hover:text-foreground hover:underline"
            >
              github.com/imran-vz/dbunk/issues
            </a>
          </div>
        </div>
      </div>
    </div>
  );
}

function ErrorDataGrid({
  message,
  errorId,
  reset,
}: {
  message: string;
  errorId: string;
  reset?: () => void;
}) {
  return (
    <div className="overflow-hidden rounded-md border border-border-subtle bg-surface-window/90 shadow-2xl shadow-black/20 backdrop-blur">
      <div className="grid grid-cols-[2.5rem_5rem_minmax(0,1fr)_6rem] border-b border-border-subtle bg-surface-panel text-2xs uppercase tracking-[0.2em] text-text-muted">
        <Cell header>#</Cell>
        <Cell header>level</Cell>
        <Cell header>event</Cell>
        <Cell header className="text-right">
          status
        </Cell>
      </div>

      <FillerRow id="001" level="info" event="session opened" status="ok" />
      <FillerRow
        id="002"
        level="info"
        event="load schema relationships"
        status="ok"
      />
      <FillerRow id="003" level="info" event="build schema graph" status="ok" />

      <div className="relative grid grid-cols-[2.5rem_5rem_minmax(0,1fr)_6rem] border-y border-danger/40 bg-danger/[0.08]">
        <span className="pointer-events-none absolute -left-px top-0 h-full w-0.5 bg-danger" />
        <Cell>
          <span className="font-mono text-danger">{errorId}</span>
        </Cell>
        <Cell>
          <span className="inline-flex items-center rounded-sm border border-danger/40 bg-danger/15 px-1.5 py-0.5 font-mono text-2xs font-semibold uppercase tracking-[0.15em] text-danger">
            error
          </span>
        </Cell>
        <Cell className="!whitespace-normal !py-3 text-foreground">
          <span className="break-words text-sm leading-snug">{message}</span>
        </Cell>
        <Cell className="text-right">
          {reset ? (
            <button
              type="button"
              onClick={reset}
              className="rounded-sm border border-accent/40 bg-accent/10 px-2 py-1 text-2xs font-medium text-accent transition-colors hover:bg-accent/20 hover:text-accent-hover"
            >
              retry
            </button>
          ) : (
            <span className="text-2xs text-text-muted">—</span>
          )}
        </Cell>
      </div>

      <FillerRow id="005" level="—" event="—" status="—" dim />
      <FillerRow id="006" level="—" event="—" status="—" dim />
    </div>
  );
}

function FillerRow({
  id,
  level,
  event,
  status,
  dim,
}: {
  id: string;
  level: string;
  event: string;
  status: string;
  dim?: boolean;
}) {
  const muted = dim ? "text-text-muted/60" : "text-text-muted";
  return (
    <div
      className={cn(
        "grid grid-cols-[2.5rem_5rem_minmax(0,1fr)_6rem] border-b border-border-subtle/60 last:border-b-0",
        muted,
      )}
    >
      <Cell>
        <span className="font-mono text-2xs">{id}</span>
      </Cell>
      <Cell>
        <span className="font-mono text-2xs uppercase tracking-wide">
          {level}
        </span>
      </Cell>
      <Cell>
        <span className="font-mono text-2xs">{event}</span>
      </Cell>
      <Cell className="text-right">
        <span className="font-mono text-2xs">{status}</span>
      </Cell>
    </div>
  );
}

function Cell({
  children,
  header,
  className,
}: {
  children: React.ReactNode;
  header?: boolean;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "truncate border-r border-border-subtle/60 px-3 py-2 last:border-r-0",
        header && "py-2",
        className,
      )}
    >
      {children}
    </div>
  );
}

/**
 * Infinite-feeling table grid that fades to the background near the
 * edges (10% inset on every side). Pure CSS — uses `background-image`
 * for the lines and intersected `mask-image` linear gradients for the
 * edge fade. Responsive: cell size is a ratio of the viewport so it
 * looks the same at any resolution.
 */
function FadingGridBackdrop() {
  const cellSize = "min(7vw, 80px)";
  const gridLine =
    "linear-gradient(to right, var(--border) 1px, transparent 1px), linear-gradient(to bottom, var(--border) 1px, transparent 1px)";
  const fadeMask =
    "linear-gradient(to right, transparent 0%, #000 10%, #000 90%, transparent 100%), linear-gradient(to bottom, transparent 0%, #000 10%, #000 90%, transparent 100%)";

  return (
    <div
      aria-hidden
      className="pointer-events-none absolute inset-0 opacity-70"
      style={{
        backgroundImage: gridLine,
        backgroundSize: `${cellSize} ${cellSize}`,
        backgroundPosition: "center",
        maskImage: fadeMask,
        WebkitMaskImage: fadeMask,
        maskComposite: "intersect",
        WebkitMaskComposite: "source-in",
      }}
    />
  );
}

function firstStackFrame(stack: string): string | null {
  if (!stack) return null;
  const lines = stack.split("\n");
  for (const raw of lines) {
    const line = raw.trim();
    if (!line.startsWith("at ")) continue;
    return line.replace(/^at\s+/, "");
  }
  return null;
}

function formatTimestamp(date: Date): string {
  const pad = (value: number) => value.toString().padStart(2, "0");
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

/**
 * Stable short hash for the error-row "id" cell — derives a 3-digit
 * pseudo row number from the message so the displayed id is
 * deterministic across reloads of the same error.
 */
function shortHash(input: string): string {
  let hash = 0;
  for (let i = 0; i < input.length; i++) {
    hash = (hash * 31 + input.charCodeAt(i)) | 0;
  }
  const positive = Math.abs(hash) % 900;
  return String(positive + 100).padStart(3, "0");
}
