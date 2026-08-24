import { requestSafetyConfirmation } from "@/lib/safety-confirmation";
import { parsePolicyRefusal } from "@/lib/safety-policy";
import type { Connection } from "@/lib/store/types";
import { errorToMessage, tauriInvoke } from "@/lib/tauri";

const NON_DESTRUCTIVE_LEGACY_COMMANDS = new Set([
  "commit_cell_edits",
  "copy_table_rows",
  "import_rows",
  "insert_row",
  "refresh_materialized_view",
  "run_pg_maintenance",
  "seed_table",
]);

/**
 * Legacy refusal tags do not carry statement summaries. Classify the known
 * additive commands here and fail closed for SQL, DDL, restore, delete, and
 * future commands whose destructive intent cannot be ruled out.
 */
function isDestructiveLegacySafetyCommand(command: string): boolean {
  return !NON_DESTRUCTIVE_LEGACY_COMMANDS.has(command);
}

/**
 * Runs a legacy policy-gated command unconfirmed, then retries only after the
 * backend returns the exact confirmation tag and the shared dialog resolves.
 */
export async function invokeWithSafetyConfirmation<Result>({
  command,
  payload,
  connection,
}: {
  command: string;
  payload: object;
  connection: Connection | undefined;
}): Promise<Result> {
  try {
    return await tauriInvoke<Result>(command, { payload });
  } catch (error) {
    const refusal = parsePolicyRefusal(errorToMessage(error));
    if (refusal?.kind !== "confirm") throw error;
    if (!connection) throw new Error("Connection not found.");
    const confirmed = await requestSafetyConfirmation({
      connection,
      subject: {
        kind: "command",
        command,
        destructive: isDestructiveLegacySafetyCommand(command),
      },
    });
    if (!confirmed) throw new Error("Safety confirmation cancelled.");
    return tauriInvoke<Result>(command, {
      payload: { ...payload, confirmed: true },
    });
  }
}
