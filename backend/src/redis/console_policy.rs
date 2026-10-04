//! Native console command policy (Plan 031 step 4).
//!
//! The console runs arbitrary commands on a dedicated connection, so this
//! module decides, without a server round trip, which commands are reads,
//! which could write, and which would break the session's request/reply
//! pairing or block it. Unknown commands are treated as writes: a read-only
//! connection fails closed, and a protected one asks for confirmation.

/// Commands that only read data or server state. Container commands
/// (`OBJECT`, `CONFIG`, …) are listed per subcommand in [`READ_SUBCOMMANDS`].
#[rustfmt::skip]
const READ_COMMANDS: &[&str] = &[
    // Keys and strings
    "GET", "MGET", "STRLEN", "GETRANGE", "SUBSTR", "EXISTS", "TYPE", "TTL", "PTTL",
    "EXPIRETIME", "PEXPIRETIME", "KEYS", "SCAN", "RANDOMKEY", "DUMP", "LCS",
    "GETBIT", "BITCOUNT", "BITPOS", "BITFIELD_RO", "SORT_RO",
    // Hashes
    "HGET", "HMGET", "HGETALL", "HKEYS", "HVALS", "HLEN", "HEXISTS", "HSTRLEN", "HSCAN",
    "HRANDFIELD", "HTTL", "HPTTL", "HEXPIRETIME", "HPEXPIRETIME",
    // Lists
    "LRANGE", "LLEN", "LINDEX", "LPOS",
    // Sets
    "SMEMBERS", "SISMEMBER", "SMISMEMBER", "SCARD", "SRANDMEMBER", "SSCAN", "SINTER",
    "SUNION", "SDIFF", "SINTERCARD",
    // Sorted sets
    "ZRANGE", "ZREVRANGE", "ZRANGEBYSCORE", "ZREVRANGEBYSCORE", "ZRANGEBYLEX",
    "ZREVRANGEBYLEX", "ZCARD", "ZCOUNT", "ZLEXCOUNT", "ZSCORE", "ZMSCORE", "ZRANK",
    "ZREVRANK", "ZSCAN", "ZRANDMEMBER", "ZINTER", "ZUNION", "ZDIFF", "ZINTERCARD",
    // Streams (blocking forms are refused separately)
    "XRANGE", "XREVRANGE", "XLEN", "XREAD", "XPENDING",
    // Geo and HyperLogLog
    "GEOPOS", "GEODIST", "GEOHASH", "GEORADIUS_RO", "GEORADIUSBYMEMBER_RO", "GEOSEARCH",
    "PFCOUNT",
    // Read-only scripting: the server refuses writes inside these.
    "EVAL_RO", "EVALSHA_RO", "FCALL_RO",
    // Server and connection state
    "PING", "ECHO", "DBSIZE", "INFO", "TIME", "LASTSAVE", "ROLE", "SELECT", "COMMAND",
    "LOLWUT", "MULTI", "EXEC", "DISCARD", "WATCH", "UNWATCH",
    // RedisJSON reads
    "JSON.GET", "JSON.MGET", "JSON.TYPE", "JSON.STRLEN", "JSON.ARRLEN", "JSON.OBJKEYS",
    "JSON.OBJLEN", "JSON.ARRINDEX", "JSON.RESP",
];

/// Read-only subcommands of container commands, as `"CONTAINER SUB"`.
#[rustfmt::skip]
const READ_SUBCOMMANDS: &[&str] = &[
    "OBJECT ENCODING", "OBJECT FREQ", "OBJECT IDLETIME", "OBJECT REFCOUNT", "OBJECT HELP",
    "MEMORY USAGE", "MEMORY STATS", "MEMORY DOCTOR", "MEMORY HELP",
    "CONFIG GET", "CONFIG HELP",
    "CLIENT LIST", "CLIENT INFO", "CLIENT GETNAME", "CLIENT ID", "CLIENT HELP",
    "SLOWLOG GET", "SLOWLOG LEN", "SLOWLOG HELP",
    "LATENCY LATEST", "LATENCY HISTORY", "LATENCY DOCTOR", "LATENCY HELP",
    "XINFO STREAM", "XINFO GROUPS", "XINFO CONSUMERS", "XINFO HELP",
    "PUBSUB CHANNELS", "PUBSUB NUMSUB", "PUBSUB NUMPAT", "PUBSUB SHARDCHANNELS",
    "PUBSUB SHARDNUMSUB",
    "SCRIPT EXISTS", "FUNCTION LIST", "FUNCTION STATS", "FUNCTION DUMP",
    "MODULE LIST", "ACL WHOAMI", "ACL CAT", "CLUSTER INFO", "CLUSTER NODES",
    "CLUSTER SLOTS", "CLUSTER SHARDS", "CLUSTER MYID",
];

/// Commands that wait server-side for data or replicas. The console has one
/// bounded request at a time, so these would only end in a timeout.
#[rustfmt::skip]
const BLOCKING_COMMANDS: &[&str] = &[
    "BLPOP", "BRPOP", "BLMOVE", "BRPOPLPUSH", "BLMPOP", "BZPOPMIN", "BZPOPMAX", "BZMPOP",
    "WAIT", "WAITAOF",
];

/// Commands that change the connection's protocol or reply stream.
#[rustfmt::skip]
const SESSION_BREAKING: &[&str] = &[
    "HELLO", "RESET", "QUIT", "SYNC", "PSYNC", "SSUBSCRIBE", "SUNSUBSCRIBE",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleCommand {
    /// Reads only; runs under every policy.
    Read,
    /// May write (or is unknown). Blocked on read-only connections and
    /// confirmed first on protected ones.
    Write,
    /// Never runs from the console.
    Refused(String),
}

/// Classifies one tokenized command. Pub/Sub and the destructive list are
/// checked by [`super::cli::guard`]; this adds what an owned console needs.
pub fn classify(tokens: &[String]) -> ConsoleCommand {
    let Some(first) = tokens.first() else {
        return ConsoleCommand::Refused("Empty command".into());
    };
    let head = first.to_uppercase();
    if BLOCKING_COMMANDS.contains(&head.as_str())
        || (matches!(head.as_str(), "XREAD" | "XREADGROUP")
            && tokens
                .iter()
                .skip(1)
                .any(|token| token.eq_ignore_ascii_case("BLOCK")))
    {
        return ConsoleCommand::Refused(format!(
            "{head} blocks the connection and is not available in the console"
        ));
    }
    let sub = tokens.get(1).map(|token| token.to_uppercase());
    if SESSION_BREAKING.contains(&head.as_str())
        || (head == "CLIENT" && sub.as_deref() == Some("REPLY"))
    {
        return ConsoleCommand::Refused(format!(
            "{head} would change this console's connection and is not available"
        ));
    }
    if READ_COMMANDS.contains(&head.as_str()) {
        return ConsoleCommand::Read;
    }
    if let Some(sub) = sub {
        let pair = format!("{head} {sub}");
        if READ_SUBCOMMANDS.contains(&pair.as_str()) {
            return ConsoleCommand::Read;
        }
    }
    ConsoleCommand::Write
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn reads_are_case_insensitive_and_subcommand_aware() {
        assert_eq!(classify(&tokens("get user:1")), ConsoleCommand::Read);
        assert_eq!(classify(&tokens("hScan h 0")), ConsoleCommand::Read);
        assert_eq!(classify(&tokens("object encoding k")), ConsoleCommand::Read);
        assert_eq!(
            classify(&tokens("CONFIG GET maxmemory")),
            ConsoleCommand::Read
        );
        assert_eq!(
            classify(&tokens("XREAD COUNT 5 STREAMS s 0")),
            ConsoleCommand::Read
        );
    }

    #[test]
    fn writes_and_unknown_commands_fail_closed() {
        assert_eq!(classify(&tokens("SET k v")), ConsoleCommand::Write);
        assert_eq!(
            classify(&tokens("CONFIG SET maxmemory 1")),
            ConsoleCommand::Write
        );
        assert_eq!(classify(&tokens("OBJECT")), ConsoleCommand::Write);
        assert_eq!(classify(&tokens("EVAL return 1 0")), ConsoleCommand::Write);
        assert_eq!(classify(&tokens("FUTURECMD x")), ConsoleCommand::Write);
    }

    #[test]
    fn blocking_and_session_breaking_commands_are_refused() {
        for text in [
            "BLPOP q 0",
            "xread block 0 streams s $",
            "XREADGROUP GROUP g c BLOCK 10 STREAMS s >",
            "WAIT 1 0",
            "HELLO 3",
            "QUIT",
            "client reply off",
        ] {
            assert!(
                matches!(classify(&tokens(text)), ConsoleCommand::Refused(_)),
                "{text}"
            );
        }
        assert!(matches!(classify(&[]), ConsoleCommand::Refused(_)));
    }
}
