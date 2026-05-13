import type { DatabaseEngine } from "@/lib/store";

/**
 * Generic "coming soon" panel rendered as a temporary body for the
 * Phase 1 sub-tabs whose real content lands in later steps. Each
 * sub-tab's real component (Tables, Schemas, Query History, Details,
 * Settings) replaces this call site as that step ships.
 */
export function PlaceholderPanel({
  title,
  description,
}: {
  title: string;
  description: string;
}) {
  return (
    <section className="flex flex-col items-center justify-center gap-2 rounded-md border border-dashed border-border-subtle px-6 py-16 text-center">
      <h2 className="text-sm font-semibold text-foreground">{title}</h2>
      <p className="max-w-md text-xs text-text-muted">{description}</p>
    </section>
  );
}

/**
 * Engine-gated explainer for sub-tabs whose content is Postgres-only
 * (Schemas, Details). Rendered when the active connection's engine is
 * not PostgreSQL — keeps the tab nav consistent across engines while
 * making clear that the surface itself does not apply.
 */
export function PostgresOnlyPanel({
  engine,
  tabLabel,
}: {
  engine: DatabaseEngine;
  tabLabel: string;
}) {
  return (
    <section className="flex flex-col items-center justify-center gap-2 rounded-md border border-dashed border-border-subtle px-6 py-16 text-center">
      <h2 className="text-sm font-semibold text-foreground">
        {tabLabel} is Postgres-only
      </h2>
      <p className="max-w-md text-xs text-text-muted">
        This surface relies on Postgres-specific catalogues. {engine} coverage
        is tracked for a later phase — see{" "}
        <code className="font-mono text-[0.6875rem]">
          docs/design/PHASES.md
        </code>
        .
      </p>
    </section>
  );
}
