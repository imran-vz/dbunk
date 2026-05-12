/**
 * Set viewer — same two-mode pattern as Hash, but for unordered
 * `SMEMBERS` / `SSCAN`.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  fetchSet,
  formatValueOneLine,
  type SerializedValue,
} from "@/lib/redis/api";

interface SetValueViewProps {
  connectionId: string;
  keyName: string;
  elementCount?: number;
}

const FULL_FETCH_THRESHOLD = 500;

export function SetValueView({
  connectionId,
  keyName,
  elementCount,
}: SetValueViewProps) {
  const mode = useMemo<"full" | "scan">(
    () =>
      elementCount !== undefined && elementCount > FULL_FETCH_THRESHOLD
        ? "scan"
        : "full",
    [elementCount],
  );
  const [members, setMembers] = useState<SerializedValue[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [pattern, setPattern] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestSeq = useRef(0);

  useEffect(() => {
    let cancelled = false;
    const seq = ++requestSeq.current;
    setLoading(true);
    setError(null);
    setMembers([]);
    setCursor(null);
    fetchSet({
      connectionId,
      key: keyName,
      mode,
      count: 200,
      pattern: pattern.trim() ? `*${pattern.trim()}*` : null,
    })
      .then((result) => {
        if (cancelled || requestSeq.current !== seq) return;
        setMembers(result.members);
        setCursor(result.nextCursor);
      })
      .catch((err) => {
        if (cancelled || requestSeq.current !== seq) return;
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled && requestSeq.current === seq) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [connectionId, keyName, mode, pattern]);

  const loadMore = async () => {
    if (!cursor) return;
    setLoading(true);
    try {
      const result = await fetchSet({
        connectionId,
        key: keyName,
        mode: "scan",
        count: 200,
        cursor,
        pattern: pattern.trim() ? `*${pattern.trim()}*` : null,
      });
      setMembers((prev) => [...prev, ...result.members]);
      setCursor(result.nextCursor);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-2 p-4">
      <div className="flex items-center gap-2 text-[0.65rem] text-text-muted">
        <Input
          placeholder={
            mode === "scan"
              ? "Filter members (server-side MATCH)…"
              : "Filter loaded members…"
          }
          value={pattern}
          onChange={(event) => setPattern(event.target.value)}
          className="h-7 max-w-sm text-xs"
        />
        <span>
          {members.length.toLocaleString()}{" "}
          {elementCount !== undefined
            ? `of ${elementCount.toLocaleString()}`
            : ""}{" "}
          members · mode: {mode}
        </span>
      </div>
      {error ? (
        <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {error}
        </div>
      ) : null}
      <div className="flex-1 overflow-auto rounded-md border border-border-subtle">
        <ul className="divide-y divide-border-subtle font-mono text-xs">
          {members.map((member) => (
            <li
              key={formatValueOneLine(member)}
              className="break-all px-3 py-1 hover:bg-white/5"
            >
              {formatValueOneLine(member)}
            </li>
          ))}
        </ul>
      </div>
      <div className="flex items-center justify-between text-[0.65rem] text-text-muted">
        <span>
          {mode === "scan"
            ? "Filtering all members (server-side SCAN)"
            : "Filtering loaded page only"}
        </span>
        {cursor ? (
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-2 text-[0.65rem]"
            onClick={() => {
              void loadMore();
            }}
            disabled={loading}
          >
            {loading ? "Loading…" : "Load more"}
          </Button>
        ) : null}
      </div>
    </div>
  );
}
