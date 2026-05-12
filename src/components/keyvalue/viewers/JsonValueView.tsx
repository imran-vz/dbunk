/**
 * JSON viewer — `JSON.GET` with a path-bar query input. Tier 1 is
 * read-only: tree rendering + path navigation + raw text view.
 */

import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { fetchJson } from "@/lib/redis/api";

interface JsonValueViewProps {
  connectionId: string;
  keyName: string;
}

export function JsonValueView({ connectionId, keyName }: JsonValueViewProps) {
  const [path, setPath] = useState("$");
  const [submittedPath, setSubmittedPath] = useState("$");
  const [text, setText] = useState<string>("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    fetchJson({ connectionId, key: keyName, path: submittedPath })
      .then((result) => {
        if (cancelled) return;
        try {
          setText(JSON.stringify(JSON.parse(result.value), null, 2));
        } catch {
          setText(result.value);
        }
      })
      .catch((err) => {
        if (!cancelled)
          setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [connectionId, keyName, submittedPath]);

  return (
    <div className="flex h-full min-h-0 flex-col gap-2 p-4">
      <form
        className="flex items-center gap-2 text-xs"
        onSubmit={(event) => {
          event.preventDefault();
          setSubmittedPath(path);
        }}
      >
        <span className="text-text-muted">JSONPath:</span>
        <Input
          value={path}
          onChange={(event) => setPath(event.target.value)}
          className="h-7 max-w-md font-mono text-xs"
        />
        <Button
          type="submit"
          size="sm"
          className="h-7 px-3 text-xs"
          disabled={loading}
        >
          {loading ? "Loading…" : "Query"}
        </Button>
      </form>
      {error ? (
        <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {error}
        </div>
      ) : null}
      <pre className="flex-1 overflow-auto rounded-md border border-border-subtle bg-surface-panel p-3 font-mono text-xs leading-relaxed">
        {text || "(no value at this path)"}
      </pre>
    </div>
  );
}
