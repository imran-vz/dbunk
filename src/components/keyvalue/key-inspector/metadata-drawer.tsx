import type { KeyMetadata } from "@/lib/redis/api";

interface MetadataDrawerProps {
  metadata: KeyMetadata;
}

export function MetadataDrawer({ metadata }: MetadataDrawerProps) {
  return (
    <aside className="w-72 shrink-0 border-l border-border-subtle bg-surface-panel/70 px-4 py-3 text-2xs">
      <h3 className="mb-2 font-semibold uppercase tracking-wide text-text-muted">
        Metadata
      </h3>
      <dl className="space-y-1.5">
        <MetadataRow label="Type" value={metadata.type} />
        <MetadataRow label="TTL" value={formatTtl(metadata.ttlSeconds)} />
        {metadata.encoding ? (
          <MetadataRow label="Encoding" value={metadata.encoding} />
        ) : null}
        {metadata.elementCount !== undefined ? (
          <MetadataRow
            label="Elements"
            value={metadata.elementCount.toLocaleString()}
          />
        ) : null}
        {metadata.memoryBytes !== undefined ? (
          <MetadataRow
            label="Memory"
            value={`${metadata.memoryBytes.toLocaleString()} bytes`}
          />
        ) : null}
      </dl>
      <p className="mt-4 text-text-muted">
        Editors for string/hash land in Phase 1.4. Live-refresh / keyspace
        notifications are tracked in FOLLOWUPS.md.
      </p>
    </aside>
  );
}

function MetadataRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-2 border-b border-border-subtle pb-1">
      <dt className="text-text-muted">{label}</dt>
      <dd className="font-mono text-foreground">{value}</dd>
    </div>
  );
}

function formatTtl(ttlSeconds: number): string {
  if (ttlSeconds === -1) return "never";
  if (ttlSeconds === -2) return "missing";
  return `${ttlSeconds}s`;
}
