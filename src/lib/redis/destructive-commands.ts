/**
 * Destructive Redis commands — list mirrored with the Rust backend
 * (`src-tauri/src/redis/destructive_commands.rs`).
 *
 * Authoritative source for both is
 * `src-tauri/src/redis/destructive-commands.toml`. Phase 1.1 ships
 * hand-mirrored copies in both languages; codegen wires up later if
 * drift becomes an issue.
 *
 * See ADR-0009.
 */

import { keyvaluePolicy } from "@/lib/engine-policy";

/**
 * Commands that require typed-confirmation. Backend rejects them
 * unless `confirmed: true` is passed.
 */
export const DESTRUCTIVE_HARD = keyvaluePolicy("Redis").destructiveCommandsHard;

/**
 * Commands that are fine in moderation but can be performance
 * footguns — softer warning, single Enter to confirm.
 */
export const DESTRUCTIVE_SOFT = keyvaluePolicy("Redis").destructiveCommandsSoft;

/**
 * Match a typed command against the hard + soft lists. Compares the
 * first one or two tokens (so `CONFIG SET maxmemory` is caught but
 * `CONFIG GET maxmemory` is not).
 */
export function classifyCommand(command: string): {
  kind: "hard" | "soft" | "ok";
  matched?: string;
} {
  const trimmed = command.trim().toUpperCase();
  if (!trimmed) return { kind: "ok" };

  for (const entry of DESTRUCTIVE_HARD) {
    if (matchesCommand(trimmed, entry)) {
      return { kind: "hard", matched: entry };
    }
  }
  for (const entry of DESTRUCTIVE_SOFT) {
    if (matchesCommand(trimmed, entry)) {
      return { kind: "soft", matched: entry };
    }
  }
  return { kind: "ok" };
}

function matchesCommand(typed: string, entry: string): boolean {
  if (typed === entry) return true;
  return typed.startsWith(`${entry} `);
}
