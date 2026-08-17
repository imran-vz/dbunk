/**
 * JSON viewer + editor (Tier 2). `JSON.GET` with a path-bar query
 * input. Editor commits the edited text via `JSON.SET` at the current
 * path (defaults to `$`). Delete clears the subtree via `JSON.DEL`.
 *
 * Validation is `JSON.parse` client-side before save; server errors
 * (e.g. JSONPath mismatch) surface in the error banner.
 */

import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import {
  deleteRedisJsonPath,
  fetchJson,
  setRedisJsonPath,
} from "@/lib/redis/api";

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
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const [reloadTick, setReloadTick] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    fetchJson({ connectionId, key: keyName, path: submittedPath })
      .then((result) => {
        if (cancelled) return;
        let formatted = result.value;
        try {
          formatted = JSON.stringify(JSON.parse(result.value), null, 2);
        } catch {
          // Leave as-is when the server returns non-strict JSON.
        }
        setText(formatted);
        setDraft(formatted);
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
  }, [connectionId, keyName, submittedPath, reloadTick]);

  const validate = (value: string): string | null => {
    if (value.trim() === "") return null;
    try {
      JSON.parse(value);
      return null;
    } catch (err) {
      return err instanceof Error ? err.message : "Invalid JSON";
    }
  };

  const handleSave = async () => {
    const message = validate(draft);
    if (message) {
      setError(message);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await setRedisJsonPath({
        connectionId,
        key: keyName,
        path: submittedPath,
        value: draft,
      });
      setEditing(false);
      setReloadTick((t) => t + 1);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    if (submittedPath === "$") {
      setError("Use the key-level Delete action to remove the whole document.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await deleteRedisJsonPath({
        connectionId,
        key: keyName,
        path: submittedPath,
      });
      setEditing(false);
      setSubmittedPath("$");
      setPath("$");
      setReloadTick((t) => t + 1);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

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
          disabled={loading || editing}
        >
          {loading ? "Loading…" : "Query"}
        </Button>
        <div className="ml-auto flex items-center gap-2">
          {!editing ? (
            <>
              <Button
                size="sm"
                variant="outline"
                className="h-7 px-2 text-[0.65rem]"
                onClick={() => {
                  setDraft(text);
                  setEditing(true);
                }}
              >
                Edit
              </Button>
              {submittedPath !== "$" ? (
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-7 px-2 text-[0.65rem] text-destructive hover:text-destructive"
                  onClick={() => {
                    void handleDelete();
                  }}
                  disabled={saving}
                >
                  Delete path
                </Button>
              ) : null}
            </>
          ) : (
            <>
              <Button
                size="sm"
                variant="ghost"
                className="h-7 px-2 text-[0.65rem]"
                disabled={saving}
                onClick={() => {
                  setEditing(false);
                  setDraft(text);
                }}
              >
                Cancel
              </Button>
              <Button
                size="sm"
                className="h-7 px-2 text-[0.65rem]"
                disabled={saving || draft === text}
                onClick={() => {
                  void handleSave();
                }}
              >
                {saving ? "Saving…" : "Save"}
              </Button>
            </>
          )}
        </div>
      </form>
      {error ? (
        <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {error}
        </div>
      ) : null}
      {editing ? (
        <Textarea
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          className="min-h-0 flex-1 font-mono text-xs"
          placeholder='{"key": "value"}'
        />
      ) : (
        <pre className="flex-1 overflow-auto rounded-md border border-border-subtle bg-surface-panel p-3 font-mono text-xs leading-relaxed">
          {text || "(no value at this path)"}
        </pre>
      )}
    </div>
  );
}
