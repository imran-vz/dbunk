import { IconAlertTriangle, IconX } from "@tabler/icons-react";

import { DdlPlanPreviewGroups } from "@/components/object-ddl/plan-preview";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type {
  DDLOutcome,
  PendingChange,
  StructureCommitStatus,
} from "@/lib/store";

import { describeChange, EmptyRow, Section } from "./shared";
import type { StructurePgPreview } from "./use-structure";

interface PendingChangesSectionProps {
  pending: PendingChange[];
  previewSql: string;
  pgPreview: StructurePgPreview;
  showPreview: boolean;
  commitStatus: StructureCommitStatus | undefined;
  commitDisabled: boolean;
  lastOutcome: DDLOutcome | null;
  onTogglePreview: () => void;
  onRemove: (id: string) => void;
  onCommit: () => void;
}

export function PendingChangesSection({
  pending,
  previewSql,
  pgPreview,
  showPreview,
  commitStatus,
  commitDisabled,
  lastOutcome,
  onTogglePreview,
  onRemove,
  onCommit,
}: PendingChangesSectionProps) {
  const isRunning = commitStatus?.state === "running";
  const errorMessage =
    lastOutcome?.kind === "failed" ? lastOutcome.reason : null;
  const successRuntime =
    lastOutcome?.kind === "completed" ? lastOutcome.runtimeMs : null;
  const isEmpty = pending.length === 0;

  return (
    <Section
      title="Pending changes"
      testId="structure-pending-section"
      action={
        <SectionActions
          showPreview={showPreview}
          isRunning={isRunning}
          isEmpty={isEmpty}
          commitDisabled={commitDisabled}
          onTogglePreview={onTogglePreview}
          onCommit={onCommit}
        />
      }
    >
      <PendingList
        pending={pending}
        pgPreview={pgPreview}
        onRemove={onRemove}
      />
      {pgPreview.state === "loading" && !isEmpty ? (
        <div
          data-testid="structure-preview-loading"
          className="border-t border-border-subtle px-3 py-2 text-xs text-muted-foreground"
        >
          Generating DDL preview…
        </div>
      ) : null}
      {pgPreview.state === "error" ? (
        <ErrorBanner
          testId="structure-preview-error"
          message={`DDL preview failed: ${pgPreview.message}`}
        />
      ) : null}
      {showPreview && pgPreview.state === "ready" ? (
        <div data-testid="structure-ddl-preview">
          <DdlPlanPreviewGroups preview={pgPreview.preview} />
        </div>
      ) : null}
      {showPreview && previewSql ? <SqlPreview sql={previewSql} /> : null}
      {errorMessage ? (
        <ErrorBanner
          testId="structure-commit-error"
          message={`Commit failed: ${errorMessage}`}
        />
      ) : null}
      {successRuntime !== null && successRuntime !== undefined ? (
        <SuccessBanner runtimeMs={successRuntime} />
      ) : null}
    </Section>
  );
}

function SectionActions({
  showPreview,
  isRunning,
  isEmpty,
  commitDisabled,
  onTogglePreview,
  onCommit,
}: {
  showPreview: boolean;
  isRunning: boolean;
  isEmpty: boolean;
  commitDisabled: boolean;
  onTogglePreview: () => void;
  onCommit: () => void;
}) {
  return (
    <div className="flex items-center gap-2">
      <Button
        data-testid="structure-preview-sql"
        variant="outline"
        size="sm"
        onClick={onTogglePreview}
        disabled={isEmpty}
      >
        {showPreview ? "Hide SQL" : "Preview SQL"}
      </Button>
      <Button
        data-testid="structure-commit"
        variant="default"
        size="sm"
        onClick={onCommit}
        disabled={commitDisabled}
      >
        {isRunning ? "Committing..." : "Commit"}
      </Button>
    </div>
  );
}

/** PG row summary and destructiveness come from the same aligned backend
 * statement. A count mismatch falls back to neutral UI rather than guessing. */
const pendingPreviewStatement = (
  entry: PendingChange,
  index: number,
  pending: PendingChange[],
  pgPreview: StructurePgPreview,
) => {
  if (entry.change.kind === "column") return null;
  if (
    pgPreview.state === "ready" &&
    pgPreview.preview.statements.length === pending.length
  ) {
    return pgPreview.preview.statements[index] ?? null;
  }
  return null;
};

const pendingLabel = (
  entry: PendingChange,
  index: number,
  pending: PendingChange[],
  pgPreview: StructurePgPreview,
): string => {
  if (entry.change.kind === "column") {
    return describeChange(entry.change.change);
  }
  const statement = pendingPreviewStatement(entry, index, pending, pgPreview);
  if (statement) return statement.summary;
  return "Pending preview";
};

function PendingList({
  pending,
  pgPreview,
  onRemove,
}: {
  pending: PendingChange[];
  pgPreview: StructurePgPreview;
  onRemove: (id: string) => void;
}) {
  if (pending.length === 0) return <EmptyRow>No pending changes.</EmptyRow>;
  return (
    <ul className="divide-y divide-border-subtle">
      {pending.map((entry, index) => (
        <li
          key={entry.id}
          data-testid={`structure-pending-${entry.id}`}
          className="flex items-center gap-2 px-3 py-1.5 text-xs"
        >
          <span className="truncate text-foreground">
            {pendingLabel(entry, index, pending, pgPreview)}
          </span>
          {pendingPreviewStatement(entry, index, pending, pgPreview)
            ?.destructive ? (
            <Badge
              data-testid={`structure-pending-destructive-${entry.id}`}
              variant="destructive"
              className="shrink-0 text-2xs"
            >
              Destructive
            </Badge>
          ) : null}
          <Button
            data-testid={`structure-remove-pending-${entry.id}`}
            variant="ghost"
            size="sm"
            className="ml-auto px-1.5 text-muted-foreground"
            aria-label="Remove pending change"
            onClick={() => onRemove(entry.id)}
          >
            <IconX />
          </Button>
        </li>
      ))}
    </ul>
  );
}

function SqlPreview({ sql }: { sql: string }) {
  return (
    <pre
      data-testid="structure-sql-preview"
      className="overflow-x-auto whitespace-pre-wrap border-t border-border-subtle bg-surface-window px-3 py-2 font-mono text-xs text-foreground"
    >
      {sql}
    </pre>
  );
}

function ErrorBanner({ testId, message }: { testId: string; message: string }) {
  return (
    <div
      data-testid={testId}
      role="alert"
      className="flex items-center gap-2 border-t border-danger/40 bg-danger/10 px-4 py-2 text-xs text-danger"
    >
      <IconAlertTriangle className="size-3.5" />
      <span>{message}</span>
    </div>
  );
}

function SuccessBanner({ runtimeMs }: { runtimeMs: number }) {
  return (
    <div
      data-testid="structure-commit-success"
      className="border-t border-border-subtle bg-success/10 px-4 py-2 text-xs text-success"
    >
      Committed in {runtimeMs} ms.
    </div>
  );
}
