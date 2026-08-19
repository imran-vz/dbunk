import { Button } from "@/components/ui/button";
import { confirmCloseQuerySession } from "@/lib/query-session-close";
import type {
  QueryTransactionIsolation,
  QueryTransactionMode,
} from "@/lib/store";
import { useAppStore } from "@/lib/store";

function isTransactionMode(value: string): value is QueryTransactionMode {
  return value === "autocommit" || value === "manual";
}

function isTransactionIsolation(
  value: string,
): value is QueryTransactionIsolation {
  return (
    value === "readCommitted" ||
    value === "repeatableRead" ||
    value === "serializable"
  );
}

export function TransactionControls({ tabId }: { tabId: string }) {
  const session = useAppStore((state) => state.querySessions[tabId]);
  const queryStatus = useAppStore((state) => state.queryStatus[tabId]);
  const applyQueryTransactionCommand = useAppStore(
    (state) => state.applyQueryTransactionCommand,
  );
  const closeQuerySessionForTab = useAppStore(
    (state) => state.closeQuerySessionForTab,
  );
  const transaction = session?.transaction;
  if (!transaction) return null;

  const isRunning =
    queryStatus?.state === "running" || queryStatus?.state === "cancelling";

  return (
    <>
      <select
        aria-label="Transaction mode"
        value={transaction.mode}
        disabled={transaction.status !== "idle" || isRunning}
        onChange={(event) => {
          const mode = event.target.value;
          if (!isTransactionMode(mode)) return;
          void applyQueryTransactionCommand(tabId, { kind: "setMode", mode });
        }}
        className="h-7 border border-border-subtle bg-surface-panel px-1 text-[0.6875rem]"
      >
        <option value="autocommit">Autocommit</option>
        <option value="manual">Manual</option>
      </select>
      <select
        aria-label="Next manual transaction isolation"
        title="Applies to the next manual transaction"
        value={transaction.manualIsolation}
        disabled={
          transaction.mode === "autocommit" ||
          transaction.status !== "idle" ||
          isRunning
        }
        onChange={(event) => {
          const isolation = event.target.value;
          if (!isTransactionIsolation(isolation)) return;
          void applyQueryTransactionCommand(tabId, {
            kind: "setIsolation",
            isolation,
          });
        }}
        className="h-7 border border-border-subtle bg-surface-panel px-1 text-[0.6875rem]"
      >
        <option value="readCommitted">Read committed</option>
        <option value="repeatableRead">Repeatable read</option>
        <option value="serializable">Serializable</option>
      </select>
      {transaction.status === "unknown" ? (
        <>
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              void applyQueryTransactionCommand(tabId, { kind: "refresh" })
            }
          >
            Recheck
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              if (!confirmCloseQuerySession(transaction.status)) return;
              void closeQuerySessionForTab(tabId);
            }}
          >
            Close
          </Button>
        </>
      ) : null}
      <Button
        size="sm"
        variant="outline"
        disabled={transaction.status !== "active" || isRunning}
        onClick={() =>
          void applyQueryTransactionCommand(tabId, { kind: "commit" })
        }
      >
        Commit
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={transaction.status === "idle" || isRunning}
        onClick={() =>
          void applyQueryTransactionCommand(tabId, { kind: "rollback" })
        }
      >
        Rollback
      </Button>
    </>
  );
}
