import type { ReactNode } from "react";

export function KeyValue({
  label,
  value,
}: {
  label: string;
  value: ReactNode;
}) {
  return (
    <div className="min-w-0">
      <div className="text-[0.625rem] font-medium uppercase tracking-[0.12em] text-text-muted">
        {label}
      </div>
      <div className="mt-0.5 truncate text-foreground">{value}</div>
    </div>
  );
}
