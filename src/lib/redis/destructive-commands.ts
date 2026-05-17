// Destructive-command list — generated from
// `src-tauri/src/redis/destructive-commands.toml` by
// `pnpm run generate:redis-commands`.
//
// DO NOT EDIT BY HAND. The companion Rust constants in
// `src-tauri/src/redis/destructive_commands.rs` are generated from
// the same source.

export const DESTRUCTIVE_HARD = [
  "FLUSHDB",
  "FLUSHALL",
  "DEBUG",
  "SHUTDOWN",
  "CONFIG SET",
  "CONFIG RESETSTAT",
  "SCRIPT FLUSH",
  "SCRIPT KILL",
  "CLIENT KILL",
] as const;

export const DESTRUCTIVE_SOFT = ["KEYS"] as const;
