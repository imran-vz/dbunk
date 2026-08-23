/**
 * Redis CLI tab — REPL with type-aware result rendering and
 * destructive-command typed-confirmation modal.
 *
 * Phase 1.3 shipped the minimum-viable REPL: input box, scrolling
 * history pane, Cmd/Ctrl+Enter or Enter to execute, Up/Down through
 * history. Tier 2 adds an autocomplete dropdown driven by a static
 * command catalog (`cli-catalog.ts`) with arity hints.
 */

import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  appendRedisCliHistory,
  closeRedisCliSession,
  deleteSavedRedisCommand,
  formatValueOneLine,
  loadRedisCliHistory,
  loadSavedRedisCommands,
  type RunCommandResult,
  runRedisCommand,
  type SavedRedisCommand,
  type SerializedValue,
  saveSavedRedisCommand,
} from "@/lib/redis/api";
import {
  type CommandSpec,
  findCommand,
  suggestCommands,
  validateArgs,
} from "@/lib/redis/cli-catalog";
import { cn } from "@/lib/utils";

interface CliTabProps {
  connectionId: string;
  /** Stable workspace-tab ID. Used as the backend session ID so
   *  `MULTI ... EXEC` routes through one physical connection across
   *  multiple `run_command` calls. */
  tabId: string;
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

/* oxlint-disable react/no-array-index-key -- CLI scrollback is an append-only positional transcript whose entries have no server IDs. */
export function CliTab({ connectionId, tabId }: CliTabProps) {
  const [input, setInput] = useState("");
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  // Newest-first list of submitted command text, hydrated from SQLite
  // on mount and prepended to on each submit. Kept separate from
  // `history` (the visible scrollback) so the cross-session arrow-up
  // recall doesn't backfill the pane with old commands that have no
  // results attached.
  const [recallHistory, setRecallHistory] = useState<string[]>([]);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const [pendingConfirm, setPendingConfirm] = useState<{
    tokens: string[];
    command: string;
    severity: "hard" | "soft";
  } | null>(null);
  const [confirmText, setConfirmText] = useState("");
  const scrollRef = useRef<HTMLDivElement>(null);
  const [suggestionIndex, setSuggestionIndex] = useState(0);
  const [suggestionsOpen, setSuggestionsOpen] = useState(false);
  const [savedCommands, setSavedCommands] = useState<SavedRedisCommand[]>([]);
  const [savedPanelOpen, setSavedPanelOpen] = useState(false);
  const suggestions: CommandSpec[] = suggestionsOpen
    ? suggestCommands(input)
    : [];
  // Doc-strip: once the user has typed a recognised command we surface
  // its signature + description right above the input so they don't
  // have to commit to a suggestion or hit a separate help key. Distinct
  // from `suggestions` (the dropdown) — the strip persists after
  // accepting a suggestion as long as the typed prefix still matches.
  const docSpec: CommandSpec | null = (() => {
    const trimmed = input.trim();
    if (trimmed === "") return null;
    return findCommand(trimmed.split(/\s+/));
  })();

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, []);

  useEffect(() => {
    return () => {
      void closeRedisCliSession({ sessionId: tabId }).catch(() => {
        // Best-effort cleanup — the backend will GC orphan sessions
        // when the connection is dropped.
      });
    };
  }, [tabId]);

  useEffect(() => {
    let cancelled = false;
    loadRedisCliHistory(connectionId)
      .then((entries) => {
        if (cancelled) return;
        setRecallHistory(entries.map((entry) => entry.command));
      })
      .catch((err) => {
        // Non-fatal — arrow-up recall just won't include persisted
        // entries this session. Log so dev devs see it.
        console.warn("loadRedisCliHistory failed", err);
      });
    return () => {
      cancelled = true;
    };
  }, [connectionId]);

  const runTokens = async (tokens: string[], confirmed: boolean) => {
    const commandText = tokens.join(" ");
    setHistory((prev) => [...prev, { kind: "command", text: commandText }]);
    setRecallHistory((prev) => [commandText, ...prev]);
    const arityError = validateArgs(tokens);
    if (arityError) {
      setHistory((prev) => [...prev, { kind: "rejected", reason: arityError }]);
      return;
    }
    void appendRedisCliHistory({
      id:
        globalThis.crypto?.randomUUID?.() ??
        `${Date.now()}-${Math.random().toString(36).slice(2)}`,
      connectionId,
      command: commandText,
      submittedAt: new Date().toISOString(),
    }).catch((err) => {
      console.warn("appendRedisCliHistory failed", err);
    });
    try {
      const result = await runRedisCommand({
        connectionId,
        tokens,
        confirmed,
        sessionId: tabId,
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
    setSuggestionsOpen(false);
    await runTokens(tokens, false);
  };

  const acceptSuggestion = (spec: CommandSpec) => {
    setInput(`${spec.name} `);
    setSuggestionsOpen(false);
  };

  const navHistory = (direction: 1 | -1) => {
    if (recallHistory.length === 0) return;
    // `recallHistory` is newest-first; index 0 = most recent.
    const nextIdx = Math.max(
      -1,
      Math.min(recallHistory.length - 1, historyIndex + direction),
    );
    setHistoryIndex(nextIdx);
    setInput(nextIdx < 0 ? "" : (recallHistory[nextIdx] ?? ""));
  };

  useEffect(() => {
    let cancelled = false;
    loadSavedRedisCommands()
      .then((list) => {
        if (!cancelled) setSavedCommands(list);
      })
      .catch(() => {
        // Non-fatal — saved-commands panel just stays empty.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleSaveCurrent = async () => {
    const trimmed = input.trim();
    if (!trimmed) {
      toast.info("Type a command before saving");
      return;
    }
    const name = window.prompt("Name for this command:", trimmed.slice(0, 40));
    if (!name?.trim()) return;
    try {
      const now = new Date().toISOString();
      const updated = await saveSavedRedisCommand({
        id:
          globalThis.crypto?.randomUUID?.() ??
          `${Date.now()}-${Math.random().toString(36).slice(2)}`,
        name: name.trim(),
        body: trimmed,
        connectionId,
        isFavorite: false,
        createdAt: now,
        updatedAt: now,
      });
      setSavedCommands(updated);
      toast.success(`Saved as "${name.trim()}"`);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : String(err));
    }
  };

  const handleLoadSaved = (cmd: SavedRedisCommand) => {
    setInput(cmd.body);
    setSavedPanelOpen(false);
  };

  const handleDeleteSaved = async (cmd: SavedRedisCommand) => {
    if (!window.confirm(`Delete saved command "${cmd.name}"?`)) return;
    try {
      const updated = await deleteSavedRedisCommand({ id: cmd.id });
      setSavedCommands(updated);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-1 border-b border-border-subtle bg-surface-panel/40 px-3 py-1 text-2xs">
        <Button
          size="sm"
          variant="ghost"
          className="h-6 px-2 text-2xs"
          onClick={() => {
            void handleSaveCurrent();
          }}
        >
          Save
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="h-6 px-2 text-2xs"
          onClick={() => setSavedPanelOpen((value) => !value)}
        >
          Saved ({savedCommands.length})
        </Button>
      </div>
      {savedPanelOpen ? (
        <div className="max-h-48 overflow-auto border-b border-border-subtle bg-surface-panel/60 text-2xs">
          {savedCommands.length === 0 ? (
            <div className="px-3 py-2 text-text-muted">
              No saved commands yet. Use Save to pin the current input.
            </div>
          ) : (
            <ul className="divide-y divide-border-subtle">
              {savedCommands.map((cmd) => (
                <li
                  key={cmd.id}
                  className="flex items-center gap-2 px-3 py-1 hover:bg-white/5"
                >
                  <button
                    type="button"
                    onClick={() => handleLoadSaved(cmd)}
                    className="flex-1 text-left"
                  >
                    <div className="font-semibold text-foreground">
                      {cmd.name}
                    </div>
                    <div className="truncate font-mono text-text-muted">
                      {cmd.body}
                    </div>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      void handleDeleteSaved(cmd);
                    }}
                    className="px-2 text-danger hover:underline"
                  >
                    delete
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}
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
          // oxlint-disable-next-line react/no-array-index-key -- CLI history is an append-only positional transcript and entries have no server IDs.
          <div
            key={`${index}-${entry.kind}`}
            className={cn(
              "border-b border-border-subtle py-1",
              entry.kind === "command" && "text-foreground",
              entry.kind === "rejected" && "text-danger",
            )}
          >
            {entry.kind === "command" ? (
              <div>
                <span className="text-text-muted">{">"}</span> {entry.text}
              </div>
            ) : entry.kind === "rejected" ? (
              <div className="text-danger">(error) {entry.reason}</div>
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
      <div className="relative shrink-0 border-t border-border-subtle bg-surface-panel/60">
        {docSpec ? (
          <div className="flex items-baseline gap-2 border-b border-border-subtle bg-surface-window/40 px-3 py-1 font-mono text-2xs">
            <span className="font-semibold text-foreground">
              {docSpec.name}
            </span>
            <span className="text-text-muted">{docSpec.args}</span>
            <span className="ml-auto truncate text-text-muted">
              {docSpec.description}
            </span>
          </div>
        ) : null}
        {suggestionsOpen && suggestions.length > 0 ? (
          <div className="absolute bottom-full left-0 right-0 z-10 max-h-64 overflow-auto border-t border-border-subtle bg-surface-window shadow-lg">
            {suggestions.map((spec, idx) => (
              <button
                type="button"
                key={spec.name}
                onMouseDown={(event) => {
                  event.preventDefault();
                  acceptSuggestion(spec);
                }}
                onMouseEnter={() => setSuggestionIndex(idx)}
                className={cn(
                  "flex w-full items-baseline gap-2 px-3 py-1 text-left font-mono text-2xs",
                  idx === suggestionIndex
                    ? "bg-primary/15 text-foreground"
                    : "text-text-secondary",
                )}
              >
                <span className="font-semibold text-foreground">
                  {spec.name}
                </span>
                <span className="text-text-muted">{spec.args}</span>
                <span className="ml-auto truncate text-2xs text-text-muted">
                  {spec.description}
                </span>
              </button>
            ))}
          </div>
        ) : null}
        <div className="flex items-center gap-2 px-3 py-2">
          <span className="font-mono text-xs text-text-muted">{">"}</span>
          <input
            value={input}
            onChange={(event) => {
              setInput(event.target.value);
              setSuggestionsOpen(event.target.value.trim().length > 0);
              setSuggestionIndex(0);
            }}
            onFocus={() => {
              if (input.trim().length > 0) setSuggestionsOpen(true);
            }}
            onBlur={() => {
              // Defer so a mouse-down on a suggestion can still fire.
              window.setTimeout(() => setSuggestionsOpen(false), 80);
            }}
            onKeyDown={(event) => {
              // Enter only accepts a suggestion when the user has typed
              // a prefix of it. An exact-name match should submit so
              // `GET foo` doesn't first rewrite the box to `GET `
              // (review 2026-05-14 P1-3).
              const focused = suggestions[suggestionIndex];
              const exactMatch =
                focused?.name.toUpperCase() === input.trim().toUpperCase();
              if (
                suggestionsOpen &&
                suggestions.length > 0 &&
                focused &&
                (event.key === "Tab" ||
                  (event.key === "Enter" &&
                    !input.endsWith(" ") &&
                    !exactMatch))
              ) {
                event.preventDefault();
                acceptSuggestion(focused);
                return;
              }
              if (suggestionsOpen && event.key === "ArrowDown") {
                event.preventDefault();
                setSuggestionIndex((idx) =>
                  Math.min(idx + 1, suggestions.length - 1),
                );
                return;
              }
              if (suggestionsOpen && event.key === "ArrowUp") {
                event.preventDefault();
                setSuggestionIndex((idx) => Math.max(0, idx - 1));
                return;
              }
              if (event.key === "Escape" && suggestionsOpen) {
                event.preventDefault();
                setSuggestionsOpen(false);
                return;
              }
              if (event.key === "Enter") {
                event.preventDefault();
                void handleSubmit();
              } else if (event.key === "ArrowUp" && !suggestionsOpen) {
                event.preventDefault();
                navHistory(1);
              } else if (event.key === "ArrowDown" && !suggestionsOpen) {
                event.preventDefault();
                navHistory(-1);
              }
            }}
            placeholder="GET key, HGETALL hash, INFO server, …"
            className="flex-1 bg-transparent font-mono text-xs outline-none placeholder:text-text-muted/60"
          />
        </div>
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
              <p className="mb-2 text-2xs text-text-secondary">
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
              // oxlint-disable-next-line jsx-a11y/no-autofocus -- typed-confirm modal expects focus
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

/* oxlint-enable react/no-array-index-key */
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

/* oxlint-disable react/no-array-index-key -- Redis arrays have positional identity and provide no item IDs. */
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
      return <span className="text-danger">(error) {value.value}</span>;
    case "array":
      return (
        <ol className="ml-3 list-decimal text-text-muted">
          {value.value.map((item, idx) => (
            // oxlint-disable-next-line react/no-array-index-key -- Redis array items have positional identity and no stable IDs.
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
/* oxlint-enable react/no-array-index-key */
function scanReplacementFor(tokens: string[]): string[] | null {
  if (tokens.length < 2) return null;
  if (tokens[0].toUpperCase() !== "KEYS") return null;
  const pattern = tokens[1];
  if (!pattern) return null;
  return ["SCAN", "0", "MATCH", pattern, "COUNT", "100"];
}
