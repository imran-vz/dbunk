/**
 * String viewer + editor (Phase 1.4). Read-only display with an
 * optional "Edit" toggle that swaps in a textarea. TTL is set
 * separately via the key-header Expire modal.
 */

import { useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  fetchString,
  type StringValuePayload,
  setRedisString,
} from "@/lib/redis/api";
import { cn } from "@/lib/utils";

interface StringValueViewProps {
  connectionId: string;
  keyName: string;
}

const DEFAULT_MAX_BYTES = 1024 * 1024;
const FULL_LOAD_MAX_BYTES = 64 * 1024 * 1024;

export function StringValueView({
  connectionId,
  keyName,
}: StringValueViewProps) {
  const [data, setData] = useState<StringValuePayload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [maxBytes, setMaxBytes] = useState(DEFAULT_MAX_BYTES);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const [reloadTick, setReloadTick] = useState(0);
  const [bitmapMode, setBitmapMode] = useState(false);

  // biome-ignore lint/correctness/useExhaustiveDependencies: reloadTick is intentional
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    fetchString({ connectionId, key: keyName, maxBytes })
      .then((result) => {
        if (cancelled) return;
        setData(result);
        if (!editing && result.value.kind === "string") {
          setDraft(result.value.value);
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
  }, [connectionId, keyName, maxBytes, reloadTick]);

  if (loading && !data) {
    return <p className="p-4 text-xs text-text-muted">Loading value…</p>;
  }
  if (error) {
    return (
      <div
        role="alert"
        className="m-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
      >
        {error}
      </div>
    );
  }
  if (!data) return null;

  const value = data.value.kind === "string" ? data.value.value : "";
  const encoding = data.value.kind === "string" ? data.value.encoding : "utf8";

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      await setRedisString({
        connectionId,
        key: keyName,
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

  return (
    <div className="flex h-full min-h-0 flex-col gap-2 p-4">
      <div className="flex items-center justify-between text-[0.65rem] text-text-muted">
        <span>
          {data.totalBytes.toLocaleString()} bytes · encoding: {encoding}
        </span>
        <div className="flex items-center gap-2">
          {data.truncated && !editing ? (
            <Button
              size="sm"
              variant="outline"
              className="h-6 px-2 text-[0.65rem]"
              onClick={() => setMaxBytes(FULL_LOAD_MAX_BYTES)}
            >
              Load full ({(data.totalBytes / (1024 * 1024)).toFixed(2)} MB)
            </Button>
          ) : null}
          {!editing ? (
            <Button
              size="sm"
              variant="outline"
              className="h-6 px-2 text-[0.65rem]"
              onClick={() => {
                setDraft(value);
                setEditing(true);
              }}
            >
              Edit
            </Button>
          ) : (
            <>
              <Button
                size="sm"
                variant="ghost"
                className="h-6 px-2 text-[0.65rem]"
                onClick={() => {
                  setDraft(value);
                  setEditing(false);
                }}
                disabled={saving}
              >
                Cancel
              </Button>
              <Button
                size="sm"
                className="h-6 px-2 text-[0.65rem]"
                disabled={saving || draft === value}
                onClick={() => {
                  void handleSave();
                }}
              >
                {saving ? "Saving…" : "Save"}
              </Button>
            </>
          )}
        </div>
      </div>
      <div className="flex items-center gap-2 text-[0.65rem]">
        <Button
          size="sm"
          variant={bitmapMode ? "default" : "outline"}
          className="h-6 px-2 text-[0.65rem]"
          onClick={() => setBitmapMode((value) => !value)}
          disabled={editing}
          title="Treat the value as a bitmap and render each byte as 8 bits"
        >
          {bitmapMode ? "Text view" : "Bitmap view"}
        </Button>
      </div>
      {bitmapMode && !editing ? (
        <BitmapGrid value={value} encoding={encoding} />
      ) : (
        <textarea
          readOnly={!editing}
          value={editing ? draft : value}
          onChange={(event) => setDraft(event.target.value)}
          className="flex-1 resize-none rounded-md border border-border-subtle bg-surface-panel p-3 font-mono text-xs leading-relaxed"
        />
      )}
      {data.truncated && !editing ? (
        <p className="text-[0.65rem] text-amber-400">
          Showing first {(maxBytes / 1024).toFixed(0)} KB of{" "}
          {(data.totalBytes / 1024).toFixed(0)} KB.
        </p>
      ) : null}
    </div>
  );
}

const MAX_BITS_RENDERED = 1024;

/**
 * Render a string value as a grid of bits. Read-only view-mode for
 * inspecting bitmap-typed Redis strings. Editing per-bit (`SETBIT`)
 * is a separate follow-up — for now users can switch back to Text
 * view, edit there, and save.
 *
 * `encoding === "hex"` means the value is a hex-string of the raw
 * bytes; we parse pairs back into bytes. Otherwise we treat each
 * char's code point as a byte (truncating to 8 bits) — accurate for
 * ASCII-shaped data; lossy for higher-codepoint strings but
 * acceptable for a view-only mode.
 */
function BitmapGrid({ value, encoding }: { value: string; encoding: string }) {
  const bits = useMemo(() => bytesToBits(value, encoding), [value, encoding]);
  const totalBits = bits.length;
  const visibleBits = bits.slice(0, MAX_BITS_RENDERED);
  return (
    <div className="flex-1 overflow-auto rounded-md border border-border-subtle bg-surface-panel p-3 font-mono text-[0.6rem]">
      <div className="grid grid-cols-[repeat(64,_minmax(0,_1fr))] gap-0.5">
        {visibleBits.map((bit, idx) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: bit position IS the identity
            key={idx}
            title={`bit ${idx} = ${bit}`}
            className={cn(
              "aspect-square w-full rounded-[2px] text-center leading-none",
              bit === 1
                ? "bg-accent-green/80 text-surface-window"
                : "bg-surface-panel-elevated text-text-muted",
            )}
          >
            {bit}
          </span>
        ))}
      </div>
      {totalBits > MAX_BITS_RENDERED ? (
        <p className="mt-2 text-[0.6rem] text-amber-400">
          Showing first {MAX_BITS_RENDERED.toLocaleString()} of{" "}
          {totalBits.toLocaleString()} bits.
        </p>
      ) : (
        <p className="mt-2 text-[0.6rem] text-text-muted">
          {totalBits.toLocaleString()} bits
        </p>
      )}
    </div>
  );
}

function bytesToBits(value: string, encoding: string): (0 | 1)[] {
  const bytes: number[] = [];
  if (encoding === "hex") {
    for (let i = 0; i + 1 < value.length; i += 2) {
      const byte = Number.parseInt(value.slice(i, i + 2), 16);
      if (Number.isFinite(byte)) bytes.push(byte & 0xff);
    }
  } else {
    for (let i = 0; i < value.length; i++) {
      bytes.push(value.charCodeAt(i) & 0xff);
    }
  }
  const bits: (0 | 1)[] = [];
  for (const byte of bytes) {
    for (let bit = 7; bit >= 0; bit--) {
      bits.push(((byte >> bit) & 1) as 0 | 1);
    }
  }
  return bits;
}
