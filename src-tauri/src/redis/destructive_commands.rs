//! Destructive-command list — generated from
//! `destructive-commands.toml` by `bun run generate:redis-commands`.
//!
//! DO NOT EDIT BY HAND. The companion TS module
//! (`src/lib/redis/destructive-commands.ts`) is generated from the
//! same source.
//!
//! Backend enforcement of the list happens in `redis/cli.rs` (Phase
//! 1.3) — until then this list exists so the codegen pipeline is wired
//! end-to-end and the frontend can read its TS mirror.

/// Commands that require typed-confirmation before execution. The
/// frontend renders a modal that asks the user to type the command's
/// canonical name to confirm.
pub const DESTRUCTIVE_HARD: &[&str] = &[
    "FLUSHDB",
    "FLUSHALL",
    "DEBUG",
    "SHUTDOWN",
    "CONFIG SET",
    "CONFIG RESETSTAT",
    "SCRIPT FLUSH",
    "SCRIPT KILL",
    "CLIENT KILL",
];

/// Commands that are fine in moderation but can be a performance
/// footgun. The frontend renders a softer warning before sending.
pub const DESTRUCTIVE_SOFT: &[&str] = &["KEYS"];

/// Membership check used by `cli.rs` and `key_inspector.rs` (Phase 1.3+).
/// Compares the first whitespace-delimited token of the command, plus
/// the next token for the multi-word entries (`CONFIG SET`, etc.).
#[allow(dead_code)]
pub fn is_destructive(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    let upper = trimmed.to_uppercase();
    let head = upper
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");
    DESTRUCTIVE_HARD
        .iter()
        .chain(DESTRUCTIVE_SOFT.iter())
        .any(|entry| {
            let entry_upper = entry.to_uppercase();
            head == entry_upper || upper.starts_with(&format!("{} ", entry_upper))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_single_word_commands() {
        assert!(is_destructive("FLUSHDB"));
        assert!(is_destructive("flushdb"));
        assert!(is_destructive("FLUSHDB ASYNC"));
        assert!(is_destructive("KEYS *"));
    }

    #[test]
    fn matches_two_word_commands() {
        assert!(is_destructive("CONFIG SET maxmemory 1gb"));
        assert!(is_destructive("config set maxmemory 1gb"));
        assert!(is_destructive("SCRIPT FLUSH"));
        assert!(is_destructive("CLIENT KILL ID 42"));
    }

    #[test]
    fn does_not_match_unrelated_commands() {
        assert!(!is_destructive("GET key"));
        assert!(!is_destructive("CONFIG GET maxmemory"));
        assert!(!is_destructive("SCRIPT EXISTS abc"));
        assert!(!is_destructive("CLIENT LIST"));
    }

    #[test]
    fn empty_input_is_not_destructive() {
        assert!(!is_destructive(""));
        assert!(!is_destructive("   "));
    }
}
