import { IconAlertTriangle, IconX } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import type {
  DDLOutcome,
  PendingChange,
  StructureCommitStatus,
} from "@/lib/store";

import { describeChange, EmptyRow, Section } from "./shared";

interface PendingChangesSectionProps {
  pending: PendingChange[];
  previewSql: string;
  showPreview: boolean;
  commitStatus: StructureCommitStatus | undefined;
  lastOutcome: DDLOutcome | null;
  onTogglePreview: () => void;
  onRemove: (id: string) => void;
  onCommit: () => void;
}

export function PendingChangesSection({
  pending,
  previewSql,
  showPreview,
  commitStatus,
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
          onTogglePreview={onTogglePreview}
          onCommit={onCommit}
        />
      }
    >
      <PendingList pending={pending} onRemove={onRemove} />
      {showPreview && previewSql ? <SqlPreview sql={previewSql} /> : null}
      {errorMessage ? <ErrorBanner message={errorMessage} /> : null}
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
  onTogglePreview,
  onCommit,
}: {
  showPreview: boolean;
  isRunning: boolean;
  isEmpty: boolean;
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
        disabled={isEmpty || isRunning}
      >
        {isRunning ? "Committing..." : "Commit"}
      </Button>
    </div>
  );
}

function PendingList({
  pending,
  onRemove,
}: {
  pending: PendingChange[];
  onRemove: (id: string) => void;
}) {
  if (pending.length === 0) return <EmptyRow>No pending changes.</EmptyRow>;
  return (
    <ul className="divide-y divide-border-subtle">
      {pending.map((entry) => (
        <li
          key={entry.id}
          data-testid={`structure-pending-${entry.id}`}
          className="flex items-center gap-2 px-3 py-1.5 text-xs"
        >
          <span className="truncate text-foreground">
            {entry.change.kind === "column"
              ? describeChange(entry.change.change)
              : "Pending preview"}
          </span>
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

function ErrorBanner({ message }: { message: string }) {
  return (
    <div
      data-testid="structure-commit-error"
      role="alert"
      className="flex items-center gap-2 border-t border-danger/40 bg-danger/10 px-4 py-2 text-xs text-danger"
    >
      <IconAlertTriangle className="size-3.5" />
      <span>Commit failed: {message}</span>
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
