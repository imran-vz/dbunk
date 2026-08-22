import type { MutationDraftChange } from "@/lib/store";
import { cn } from "@/lib/utils";

import {
  displayMutationValue,
  type MutationChangeGroup,
  mutationChangeGuardCopy,
  mutationChangeTitle,
  mutationFailureMessage,
} from "./model";

type MutationChangeListProps = {
  groups: MutationChangeGroup[];
  disabled: boolean;
  onIncludedChange: (changeId: string, included: boolean) => void;
  onRevert: (changeId: string) => void;
};

const ChangeValues = ({ change }: { change: MutationDraftChange }) => {
  if (change.kind === "updateRow") {
    return (
      <dl className="mt-1.5 grid grid-cols-[5rem_minmax(0,1fr)] gap-x-2 gap-y-1 pl-6 text-[0.6875rem]">
        {change.cellOrder.map((column) => {
          const cell = change.cells[column];
          return cell ? (
            <div key={column} className="contents">
              <dt className="truncate text-text-muted" title={column}>
                {column}
              </dt>
              <dd className="min-w-0 break-words text-foreground">
                <span className="text-danger line-through">
                  {displayMutationValue(cell.original)}
                </span>
                <span className="px-1.5 text-text-muted" aria-hidden="true">
                  →
                </span>
                <span className="text-accent-hover">
                  {displayMutationValue(cell.value)}
                </span>
              </dd>
            </div>
          ) : null;
        })}
      </dl>
    );
  }

  const values = change.kind === "deleteRow" ? change.originals : change.values;
  return (
    <dl className="mt-1.5 grid grid-cols-[5rem_minmax(0,1fr)] gap-x-2 gap-y-1 pl-6 text-[0.6875rem]">
      {values.map(({ column, value }) => (
        <div key={column} className="contents">
          <dt className="truncate text-text-muted" title={column}>
            {column}
          </dt>
          <dd className="min-w-0 break-words text-foreground">
            {change.kind === "deleteRow" ? (
              <>
                <span className="text-danger line-through">
                  {displayMutationValue(value)}
                </span>
                <span className="px-1.5 text-text-muted" aria-hidden="true">
                  →
                </span>
                <span className="text-text-muted">deleted</span>
              </>
            ) : (
              <>
                <span className="text-text-muted">default</span>
                <span className="px-1.5 text-text-muted" aria-hidden="true">
                  →
                </span>
                <span className="text-accent-hover">
                  {displayMutationValue(value)}
                </span>
              </>
            )}
          </dd>
        </div>
      ))}
    </dl>
  );
};

export function MutationChangeList({
  groups,
  disabled,
  onIncludedChange,
  onRevert,
}: MutationChangeListProps) {
  return groups.map((group) => (
    <section key={group.key} className="border-b border-border-subtle">
      <h2 className="sticky top-0 z-10 border-b border-border-subtle bg-[#0b0b0b] px-3 py-1.5 text-[0.625rem] font-semibold uppercase tracking-[0.08em] text-text-secondary">
        {group.label} · {group.target}
      </h2>
      {group.changes.map((change) => {
        const title = mutationChangeTitle(change);
        const failure = mutationFailureMessage(change);
        const failureId = `${change.changeId.replaceAll(":", "-")}-failure`;
        return (
          <article
            key={change.changeId}
            data-change-id={change.changeId}
            data-failed={failure ? "true" : "false"}
            className={cn(
              "border-t border-border-subtle px-3 py-2 first:border-t-0",
              !change.included && "opacity-45",
              failure && "bg-danger/10 shadow-[inset_3px_0_0_var(--danger)]",
            )}
          >
            <div className="grid grid-cols-[1rem_minmax(0,1fr)_auto] items-start gap-2">
              <input
                type="checkbox"
                checked={change.included}
                disabled={disabled}
                onChange={(event) =>
                  onIncludedChange(change.changeId, event.target.checked)
                }
                aria-label={`Include ${title}`}
                aria-describedby={failure ? failureId : undefined}
                className="mt-0.5 size-3.5 accent-primary"
              />
              <div className="min-w-0">
                <div className="flex flex-wrap items-baseline gap-x-2 text-xs font-medium text-foreground">
                  <span className="break-words">{title}</span>
                  {change.kind !== "insertRow" && change.rowIndex === null ? (
                    <span className="text-[0.625rem] font-normal text-warning">
                      Off page
                    </span>
                  ) : null}
                </div>
                <div className="text-[0.625rem] text-text-muted">
                  {mutationChangeGuardCopy(change)}
                </div>
              </div>
              <button
                type="button"
                disabled={disabled}
                onClick={() => onRevert(change.changeId)}
                className="text-[0.625rem] text-text-muted hover:text-foreground disabled:opacity-50"
                aria-label={`Revert ${title}`}
              >
                Revert
              </button>
            </div>
            <ChangeValues change={change} />
            {failure ? (
              <p
                id={failureId}
                role="alert"
                className="mb-0 mt-1.5 pl-6 text-[0.6875rem] text-danger"
              >
                {failure}
              </p>
            ) : null}
          </article>
        );
      })}
    </section>
  ));
}
