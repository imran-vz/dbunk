/**
 * Redis CLI tab — REPL with type-aware result rendering and
 * destructive-command typed-confirmation modal.
 *
 * Phase 1.3 ships the minimum-viable REPL: input box, scrolling
 * history pane, Cmd/Ctrl+Enter or Enter to execute, Up/Down through
 * history. Command autocomplete catalog is deferred.
 */

import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  formatValueOneLine,
  type RunCommandResult,
  runRedisCommand,
  type SerializedValue,
} from "@/lib/redis/api";
import { cn } from "@/lib/utils";

interface CliTabProps {
  connectionId: string;
}

type HistoryEntry =
  | { kind: "command"; text: string }
  | {
      kind: "ok";
      value: SerializedValue;
      runtimeMs: number;
    }
  | { kind: "rejected"; reason: string }
  | { kind: "needs-confirmation"; command: string; severity: "hard" | "soft" };

export function CliTab({ connectionId }: CliTabProps) {
  const [input, setInput] = useState("");
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const [pendingConfirm, setPendingConfirm] = useState<{
    tokens: string[];
    command: string;
    severity: "hard" | "soft";
  } | null>(null);
  const [confirmText, setConfirmText] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, []);

  const submittedCommands = history
    .filter((entry) => entry.kind === "command")
    .map((entry) => (entry as { text: string }).text);

  const runTokens = async (tokens: string[], confirmed: boolean) => {
    setHistory((prev) => [
      ...prev,
      { kind: "command", text: tokens.join(" ") },
    ]);
    try {
      const result = await runRedisCommand({
        connectionId,
        tokens,
        confirmed,
      });
      handleResult(result, tokens);
    } catch (err) {
      setHistory((prev) => [
        ...prev,
        {
          kind: "rejected",
          reason: err instanceof Error ? err.message : String(err),
        },
      ]);
    }
  };

  const handleResult = (result: RunCommandResult, tokens: string[]) => {
    switch (result.kind) {
      case "ok":
        setHistory((prev) => [
          ...prev,
          { kind: "ok", value: result.value, runtimeMs: result.runtimeMs },
        ]);
        break;
      case "rejected":
        setHistory((prev) => [
          ...prev,
          { kind: "rejected", reason: result.reason },
        ]);
        break;
      case "needs-confirmation":
        setPendingConfirm({
          tokens,
          command: result.command,
          severity: result.severity,
        });
        setConfirmText("");
        break;
    }
  };

  const handleSubmit = async () => {
    const trimmed = input.trim();
    if (!trimmed) return;
    const tokens = trimmed.split(/\s+/);
    setInput("");
    setHistoryIndex(-1);
    await runTokens(tokens, false);
  };

  const navHistory = (direction: 1 | -1) => {
    if (submittedCommands.length === 0) return;
    const nextIdx = Math.max(
      -1,
      Math.min(submittedCommands.length - 1, historyIndex + direction),
    );
    setHistoryIndex(nextIdx);
    setInput(
      nextIdx < 0
        ? ""
        : (submittedCommands[submittedCommands.length - 1 - nextIdx] ?? ""),
    );
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div
        ref={scrollRef}
        className="flex-1 overflow-auto bg-surface-window p-3 font-mono text-xs"
      >
        {history.length === 0 ? (
          <p className="text-text-muted">
            Redis CLI — type a command and press Enter. Pub/Sub commands open in
            the Pub/Sub tab. Destructive commands ask for typed confirmation.
          </p>
        ) : null}
        {history.map((entry, index) => (
          <div
            key={`${index}-${entry.kind}`}
            className={cn(
              "border-b border-border-subtle py-1",
              entry.kind === "command" && "text-foreground",
              entry.kind === "rejected" && "text-destructive",
            )}
          >
            {entry.kind === "command" ? (
              <div>
                <span className="text-text-muted">{">"}</span> {entry.text}
              </div>
            ) : entry.kind === "rejected" ? (
              <div className="text-destructive">(error) {entry.reason}</div>
            ) : entry.kind === "needs-confirmation" ? (
              <div className="text-amber-400">
                {entry.command} requires confirmation ({entry.severity})
              </div>
            ) : (
              <RenderValue value={entry.value} runtimeMs={entry.runtimeMs} />
            )}
          </div>
        ))}
      </div>
      <div className="flex shrink-0 items-center gap-2 border-t border-border-subtle bg-surface-panel/60 px-3 py-2">
        <span className="font-mono text-xs text-text-muted">{">"}</span>
        <input
          value={input}
          onChange={(event) => setInput(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void handleSubmit();
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              navHistory(1);
            } else if (event.key === "ArrowDown") {
              event.preventDefault();
              navHistory(-1);
            }
          }}
          placeholder="GET key, HGETALL hash, INFO server, …"
          className="flex-1 bg-transparent font-mono text-xs outline-none placeholder:text-text-muted/60"
        />
      </div>
      {pendingConfirm ? (
        <div className="border-t border-amber-400/30 bg-amber-400/10 px-4 py-3 text-xs">
          <p className="mb-2">
            <strong>
              {pendingConfirm.severity === "soft"
                ? "Slow command."
                : "Destructive command."}
            </strong>{" "}
            Type{" "}
            <code className="rounded bg-surface-panel px-1 py-0.5 font-mono">
              {pendingConfirm.command}
            </code>{" "}
            to confirm:
          </p>
          {(() => {
            const replacement = scanReplacementFor(pendingConfirm.tokens);
            if (!replacement) return null;
            return (
              <p className="mb-2 text-[0.6875rem] text-text-secondary">
                <code className="rounded bg-surface-panel px-1 py-0.5 font-mono">
                  KEYS
                </code>{" "}
                blocks the server while it scans every key. Consider{" "}
                <code className="rounded bg-surface-panel px-1 py-0.5 font-mono">
                  {replacement.join(" ")}
                </code>{" "}
                instead — iterates in batches without blocking other clients.{" "}
                <button
                  type="button"
                  className="underline underline-offset-2 hover:text-foreground"
                  onClick={() => {
                    setPendingConfirm(null);
                    setConfirmText("");
                    void runTokens(replacement, false);
                  }}
                >
                  Use SCAN instead
                </button>
              </p>
            );
          })()}
          <div className="flex items-center gap-2">
            <input
              value={confirmText}
              onChange={(event) => setConfirmText(event.target.value)}
              className="flex-1 rounded border border-border-subtle bg-surface-panel px-2 py-1 font-mono text-xs"
              // biome-ignore lint/a11y/noAutofocus: typed-confirm modal expects focus
              autoFocus
            />
            <Button
              size="sm"
              variant="destructive"
              className="h-7 text-xs"
              disabled={confirmText.toUpperCase() !== pendingConfirm.command}
              onClick={async () => {
                const tokens = pendingConfirm.tokens;
                setPendingConfirm(null);
                setConfirmText("");
                await runTokens(tokens, true);
              }}
            >
              Run
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-7 text-xs"
              onClick={() => {
                setPendingConfirm(null);
                setConfirmText("");
              }}
            >
              Cancel
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function RenderValue({
  value,
  runtimeMs,
}: {
  value: SerializedValue;
  runtimeMs: number;
}) {
  return (
    <div className="space-y-1">
      <div className="text-text-muted">[{runtimeMs} ms]</div>
      {renderBody(value)}
    </div>
  );
}

function renderBody(value: SerializedValue): React.ReactNode {
  switch (value.kind) {
    case "nil":
      return <span className="italic text-text-muted">(nil)</span>;
    case "int":
      return (
        <span className="inline-block rounded bg-primary/15 px-1.5 py-0.5 text-primary">
          {value.value}
        </span>
      );
    case "status":
      return (
        <span className="inline-block rounded bg-emerald-400/20 px-1.5 py-0.5 text-emerald-400">
          {value.value}
        </span>
      );
    case "string":
      return (
        <pre className="overflow-x-auto whitespace-pre-wrap break-all rounded bg-surface-panel px-2 py-1">
          {value.value}
        </pre>
      );
    case "error":
      return <span className="text-destructive">(error) {value.value}</span>;
    case "array":
      return (
        <ol className="ml-3 list-decimal text-text-muted">
          {value.value.map((item, idx) => (
            <li key={`${idx}-${formatValueOneLine(item)}`}>
              {renderBody(item)}
            </li>
          ))}
        </ol>
      );
  }
}

/**
 * Suggest a non-blocking replacement for `KEYS <pattern>`. Returns
 * `SCAN 0 MATCH <pattern> COUNT 100` — one batch of results, not the
 * full iteration. Users continue with subsequent SCAN cursor calls if
 * they need to walk the whole keyspace.
 *
 * Returns `null` when the tokens aren't a `KEYS` command we can
 * rewrite (no pattern given, etc.).
 */
function scanReplacementFor(tokens: string[]): string[] | null {
  if (tokens.length < 2) return null;
  if (tokens[0].toUpperCase() !== "KEYS") return null;
  const pattern = tokens[1];
  if (!pattern) return null;
  return ["SCAN", "0", "MATCH", pattern, "COUNT", "100"];
}
