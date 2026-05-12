interface SubTabPlaceholderProps {
  kind: "indexes" | "relations";
}

const TITLES = {
  indexes: "Indexes",
  relations: "Relations",
} as const;

const DESCRIPTIONS = {
  indexes:
    "Per-table indexes (name, columns, type, unique flag, size) will appear here.",
  relations:
    "Foreign-key relationships in and out of this table will be shown here.",
} as const;

export function SubTabPlaceholder({ kind }: SubTabPlaceholderProps) {
  return (
    <div className="flex h-full items-center justify-center p-6">
      <div className="max-w-md rounded-lg border border-dashed border-border-subtle bg-surface-panel/50 p-6 text-center">
        <div className="text-sm font-semibold text-foreground">
          {TITLES[kind]}
        </div>
        <p className="mt-1 text-xs text-text-muted">{DESCRIPTIONS[kind]}</p>
        <p className="mt-3 text-[0.625rem] uppercase tracking-[0.12em] text-text-muted">
          Coming soon
        </p>
      </div>
    </div>
  );
}
