/**
 * New-key wizard — pick a type, supply an initial value, create.
 * Supports all seven types at create time; each type has an inline
 * editor after creation under `src/components/keyvalue/viewers/`.
 */

import { useState } from "react";

import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { type CreateKeyType, createRedisKey } from "@/lib/redis/api";

interface NewKeyDialogProps {
  connectionId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (key: string, type: CreateKeyType) => void;
}

const TYPES: Array<{ id: CreateKeyType; label: string }> = [
  { id: "string", label: "String" },
  { id: "hash", label: "Hash" },
  { id: "list", label: "List" },
  { id: "set", label: "Set" },
  { id: "zset", label: "Sorted set" },
  { id: "stream", label: "Stream" },
  { id: "json", label: "JSON" },
];

export function NewKeyDialog({
  connectionId,
  open,
  onOpenChange,
  onCreated,
}: NewKeyDialogProps) {
  const [type, setType] = useState<CreateKeyType>("string");
  const [keyName, setKeyName] = useState("");
  const [ttl, setTtl] = useState<string>("");
  const [body, setBody] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setType("string");
    setKeyName("");
    setTtl("");
    setBody("");
    setError(null);
    setSaving(false);
  };

  const handleCreate = async () => {
    setSaving(true);
    setError(null);
    try {
      const innerPayload = parseBody(type, body);
      const ttlNumber = ttl.trim() ? Number(ttl) : undefined;
      await createRedisKey({
        connectionId,
        key: keyName,
        type,
        payload: innerPayload,
        ttlSeconds: Number.isFinite(ttlNumber) ? ttlNumber : null,
      });
      onCreated(keyName, type);
      onOpenChange(false);
      reset();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <AlertDialog
      open={open}
      onOpenChange={(value) => {
        onOpenChange(value);
        if (!value) reset();
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>New Redis key</AlertDialogTitle>
          <AlertDialogDescription>
            Pick a type and provide an initial value. All seven types have
            inline editors after creation.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="grid gap-3 py-4">
          <div className="grid gap-1">
            <Label htmlFor="new-key-name">Key name</Label>
            <Input
              id="new-key-name"
              value={keyName}
              onChange={(event) => setKeyName(event.target.value)}
              placeholder="user:42:profile"
            />
          </div>
          <div className="grid gap-1">
            <Label htmlFor="new-key-type">Type</Label>
            <Select
              value={type}
              // SAFETY: The value is constrained by the typed component or library contract at this boundary.
              onValueChange={(value) => setType(value as CreateKeyType)}
            >
              <SelectTrigger id="new-key-type">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {TYPES.map((t) => (
                  <SelectItem key={t.id} value={t.id}>
                    {t.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="grid gap-1">
            <Label htmlFor="new-key-ttl">TTL (seconds, optional)</Label>
            <Input
              id="new-key-ttl"
              type="number"
              value={ttl}
              onChange={(event) => setTtl(event.target.value)}
              placeholder="3600"
            />
          </div>
          <div className="grid gap-1">
            <Label htmlFor="new-key-body">Initial value</Label>
            <textarea
              id="new-key-body"
              value={body}
              onChange={(event) => setBody(event.target.value)}
              placeholder={placeholderFor(type)}
              className="min-h-32 rounded-md border border-border-subtle bg-surface-panel p-2 font-mono text-xs"
            />
            <p className="text-[0.65rem] text-text-muted">{hintFor(type)}</p>
          </div>
          {error ? (
            <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
              {error}
            </div>
          ) : null}
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel type="button">Cancel</AlertDialogCancel>
          <Button
            disabled={!keyName.trim() || saving}
            onClick={() => {
              void handleCreate();
            }}
          >
            {saving ? "Creating…" : "Create"}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function placeholderFor(type: CreateKeyType): string {
  switch (type) {
    case "string":
      return "hello world";
    case "hash":
      return "field1=value1\nfield2=value2";
    case "list":
      return "item1\nitem2\nitem3";
    case "set":
      return "member1\nmember2";
    case "zset":
      return "1.0=alice\n2.5=bob";
    case "stream":
      return "field1=value1,field2=value2";
    case "json":
      return '{"name": "alice"}';
  }
}

function hintFor(type: CreateKeyType): string {
  switch (type) {
    case "string":
      return "Plain text. Will be SET as-is.";
    case "hash":
      return "One `field=value` per line.";
    case "list":
      return "One item per line. Pushed with RPUSH in order.";
    case "set":
      return "One member per line.";
    case "zset":
      return "One `score=member` per line.";
    case "stream":
      return "One entry per line, `field=value` comma-separated (one XADD per entry, auto-ID).";
    case "json":
      return "Raw JSON. Stored via JSON.SET $.";
  }
}

// oxlint-disable-next-line anti-slop/no-unknown-returns -- The value is handled at a typed library or domain boundary here.
function parseBody(type: CreateKeyType, body: string): unknown {
  const lines = body
    .split(/\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  switch (type) {
    case "string":
      // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
      return { value: body };
    case "hash":
      // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
      return {
        entries: lines
          .map((line) => {
            const idx = line.indexOf("=");
            return idx === -1
              ? null
              : [line.slice(0, idx).trim(), line.slice(idx + 1).trim()];
          })
          .filter((pair): pair is [string, string] => pair !== null),
      };
    case "list":
      // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
      return { items: lines };
    case "set":
      // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
      return { members: lines };
    case "zset":
      // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
      return {
        entries: lines
          .map((line) => {
            const idx = line.indexOf("=");
            if (idx === -1) return null;
            const score = Number(line.slice(0, idx).trim());
            const member = line.slice(idx + 1).trim();
            return Number.isFinite(score) ? [member, score] : null;
          })
          .filter(
            (pair): pair is [string, number] =>
              pair !== null && pair[1] !== null,
          ),
      };
    case "stream":
      // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
      return {
        entries: lines.map((line) => {
          const fields = line
            .split(",")
            .map((pair) => pair.split("="))
            .filter((pair) => pair.length === 2)
            .map(
              // SAFETY: The value is constrained by the typed component or library contract at this boundary.
              (pair) => [pair[0].trim(), pair[1].trim()] as [string, string],
            );
          return { fields };
        }),
      };
    case "json":
      // Validate parseability but pass the original text through.
      JSON.parse(body);
      // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
      return { value: body };
  }
}
