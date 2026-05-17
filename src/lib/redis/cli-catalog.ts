/**
 * Static Redis command catalog used by the CLI's autocomplete. Each
 * entry has the command's canonical name, a short arity hint, and a
 * one-line description. We keep this hand-curated rather than auto-
 * generating from `COMMAND DOCS` because (a) the catalog needs to work
 * offline / before connect, and (b) the descriptions are dbunk-flavoured
 * (short, scannable).
 *
 * Only the v1 set ships — common reads + writes across strings,
 * hashes, lists, sets, sorted sets, streams, keys, server, pub/sub.
 * Modules (RediSearch, RedisJSON, etc.) are followups.
 */

export type CommandSpec = {
  /** Upper-cased command, possibly with a subcommand (e.g. "ACL LIST"). */
  name: string;
  /** Arity hint surfaced to the user, e.g. `"key value [EX seconds]"`. */
  args: string;
  /** One-line description. */
  description: string;
};

export const REDIS_COMMANDS: ReadonlyArray<CommandSpec> = [
  // Strings
  { name: "GET", args: "key", description: "Get the value of a key." },
  {
    name: "SET",
    args: "key value [EX seconds | PX ms | KEEPTTL | NX | XX]",
    description: "Set the value of a key.",
  },
  { name: "MGET", args: "key [key …]", description: "Get multiple values." },
  {
    name: "MSET",
    args: "key value [key value …]",
    description: "Set multiple key/value pairs atomically.",
  },
  { name: "INCR", args: "key", description: "Increment integer at key." },
  { name: "DECR", args: "key", description: "Decrement integer at key." },
  {
    name: "INCRBY",
    args: "key increment",
    description: "Increment integer by a fixed amount.",
  },
  { name: "APPEND", args: "key value", description: "Append to string value." },
  { name: "STRLEN", args: "key", description: "Length of the string value." },

  // Keys
  {
    name: "DEL",
    args: "key [key …]",
    description: "Delete one or more keys.",
  },
  {
    name: "EXISTS",
    args: "key [key …]",
    description: "Count keys that exist.",
  },
  { name: "EXPIRE", args: "key seconds", description: "Set TTL in seconds." },
  { name: "PERSIST", args: "key", description: "Remove the TTL from a key." },
  { name: "TTL", args: "key", description: "Get TTL in seconds." },
  { name: "PTTL", args: "key", description: "Get TTL in milliseconds." },
  {
    name: "RENAME",
    args: "key newkey",
    description: "Rename a key (overwrites existing).",
  },
  { name: "TYPE", args: "key", description: "Return the type of the key." },
  {
    name: "SCAN",
    args: "cursor [MATCH pattern] [COUNT count] [TYPE type]",
    description: "Iterate keys without blocking.",
  },
  {
    name: "KEYS",
    args: "pattern",
    description: "List keys (blocks the server — prefer SCAN).",
  },
  {
    name: "DBSIZE",
    args: "",
    description: "Number of keys in the current database.",
  },
  { name: "FLUSHDB", args: "[ASYNC | SYNC]", description: "Delete all keys." },

  // Hashes
  {
    name: "HSET",
    args: "key field value [field value …]",
    description: "Set hash field(s).",
  },
  { name: "HGET", args: "key field", description: "Get a hash field." },
  {
    name: "HGETALL",
    args: "key",
    description: "All field/value pairs.",
  },
  {
    name: "HDEL",
    args: "key field [field …]",
    description: "Delete hash fields.",
  },
  {
    name: "HSCAN",
    args: "key cursor [MATCH pattern] [COUNT count]",
    description: "Iterate fields without blocking.",
  },
  { name: "HEXISTS", args: "key field", description: "Field exists?" },
  { name: "HLEN", args: "key", description: "Number of fields." },

  // Lists
  {
    name: "LPUSH",
    args: "key value [value …]",
    description: "Push to head.",
  },
  {
    name: "RPUSH",
    args: "key value [value …]",
    description: "Push to tail.",
  },
  { name: "LPOP", args: "key [count]", description: "Pop from head." },
  { name: "RPOP", args: "key [count]", description: "Pop from tail." },
  {
    name: "LRANGE",
    args: "key start stop",
    description: "Range of list elements.",
  },
  { name: "LLEN", args: "key", description: "Length of the list." },
  {
    name: "LSET",
    args: "key index value",
    description: "Set element at index.",
  },
  {
    name: "LREM",
    args: "key count value",
    description: "Remove count occurrences of value.",
  },

  // Sets
  {
    name: "SADD",
    args: "key member [member …]",
    description: "Add members to a set.",
  },
  {
    name: "SREM",
    args: "key member [member …]",
    description: "Remove members.",
  },
  { name: "SMEMBERS", args: "key", description: "All set members." },
  { name: "SCARD", args: "key", description: "Cardinality of the set." },
  {
    name: "SSCAN",
    args: "key cursor [MATCH pattern] [COUNT count]",
    description: "Iterate set members.",
  },
  {
    name: "SISMEMBER",
    args: "key member",
    description: "Member exists in set?",
  },

  // Sorted sets
  {
    name: "ZADD",
    args: "key score member [score member …]",
    description: "Add members with scores.",
  },
  {
    name: "ZREM",
    args: "key member [member …]",
    description: "Remove members.",
  },
  {
    name: "ZRANGE",
    args: "key start stop [WITHSCORES]",
    description: "Range by rank.",
  },
  {
    name: "ZRANGEBYSCORE",
    args: "key min max [WITHSCORES] [LIMIT offset count]",
    description: "Range by score.",
  },
  { name: "ZCARD", args: "key", description: "Cardinality." },
  { name: "ZSCORE", args: "key member", description: "Score of a member." },
  {
    name: "ZINCRBY",
    args: "key increment member",
    description: "Add increment to member's score.",
  },

  // Streams
  {
    name: "XADD",
    args: "key [MAXLEN ~ count] id|* field value [field value …]",
    description: "Append entry to a stream.",
  },
  {
    name: "XRANGE",
    args: "key start end [COUNT count]",
    description: "Range of stream entries.",
  },
  {
    name: "XREVRANGE",
    args: "key end start [COUNT count]",
    description: "Reverse range.",
  },
  {
    name: "XLEN",
    args: "key",
    description: "Number of entries in the stream.",
  },
  {
    name: "XDEL",
    args: "key id [id …]",
    description: "Delete entries by id.",
  },
  {
    name: "XTRIM",
    args: "key MAXLEN [~] count",
    description: "Trim to length.",
  },
  {
    name: "XINFO STREAM",
    args: "key",
    description: "Stream metadata.",
  },
  {
    name: "XINFO GROUPS",
    args: "key",
    description: "Consumer groups on the stream.",
  },
  {
    name: "XINFO CONSUMERS",
    args: "key group",
    description: "Consumers within a group.",
  },

  // JSON (RedisJSON)
  {
    name: "JSON.SET",
    args: "key path value",
    description: "Set JSON value at path.",
  },
  {
    name: "JSON.GET",
    args: "key [path …]",
    description: "Get JSON value at path(s).",
  },
  {
    name: "JSON.DEL",
    args: "key [path]",
    description: "Delete JSON path.",
  },
  {
    name: "JSON.TYPE",
    args: "key [path]",
    description: "Type of JSON path.",
  },

  // Pub/Sub
  {
    name: "PUBLISH",
    args: "channel message",
    description: "Publish a message to a channel.",
  },
  {
    name: "SUBSCRIBE",
    args: "channel [channel …]",
    description: "Subscribe to channels (use the Pub/Sub tab).",
  },
  {
    name: "PSUBSCRIBE",
    args: "pattern [pattern …]",
    description: "Subscribe to channel patterns.",
  },
  {
    name: "PUBSUB CHANNELS",
    args: "[pattern]",
    description: "List active pub/sub channels.",
  },
  {
    name: "PUBSUB NUMSUB",
    args: "[channel …]",
    description: "Subscribers per channel.",
  },

  // Server
  {
    name: "PING",
    args: "[message]",
    description: "Ping the server.",
  },
  {
    name: "INFO",
    args: "[section]",
    description: "Server statistics and metadata.",
  },
  {
    name: "CONFIG GET",
    args: "pattern",
    description: "Get server configuration values.",
  },
  {
    name: "CONFIG SET",
    args: "parameter value",
    description: "Set server configuration value.",
  },
  {
    name: "CLIENT LIST",
    args: "",
    description: "List connected clients.",
  },
  {
    name: "CLIENT KILL",
    args: "ID id | ADDR addr",
    description: "Disconnect a client.",
  },
  { name: "ACL LIST", args: "", description: "List ACL users." },
  {
    name: "ACL GETUSER",
    args: "username",
    description: "Inspect a single ACL user.",
  },
  {
    name: "LATENCY LATEST",
    args: "",
    description: "Latest latency event per category.",
  },
  {
    name: "LATENCY HISTORY",
    args: "event",
    description: "Recent latency samples for an event.",
  },
  {
    name: "SLOWLOG GET",
    args: "[count]",
    description: "Recent slow-log entries.",
  },
  {
    name: "MEMORY USAGE",
    args: "key [SAMPLES count]",
    description: "Approximate memory used by a key.",
  },
  {
    name: "MODULE LIST",
    args: "",
    description: "List loaded modules.",
  },
];

/**
 * Suggest commands that match the user's first-token input. We match
 * by prefix on the first token and (when the user has typed a space)
 * also match two-token names like `XINFO GROUPS`. Sorted by exact-
 * prefix first, then by name length to push the shortest hit up.
 */
/**
 * Look up the canonical `CommandSpec` for a token list (e.g.
 * `["PUBSUB", "CHANNELS"]`). Tries the two-token name first so
 * `PUBSUB CHANNELS` wins over a hypothetical bare `PUBSUB`, then
 * falls back to the single-token name. Match is case-insensitive.
 *
 * Returns `null` when nothing in the catalog matches — callers should
 * let unknown commands through to the server unchallenged (the
 * catalog is intentionally incomplete; the server is authoritative).
 */
export function findCommand(tokens: string[]): CommandSpec | null {
  if (tokens.length === 0) return null;
  const upper = tokens.map((token) => token.toUpperCase());
  if (upper.length >= 2) {
    const twoWord = `${upper[0]} ${upper[1]}`;
    const match = REDIS_COMMANDS.find((spec) => spec.name === twoWord);
    if (match) return match;
  }
  return REDIS_COMMANDS.find((spec) => spec.name === upper[0]) ?? null;
}

/**
 * Count "required" tokens in a `CommandSpec.args` hint by ignoring
 * anything inside `[...]` brackets. Whitespace separates tokens.
 *
 * Examples:
 *   "key"                                    → 1
 *   "key value"                              → 2
 *   "key value [EX seconds | …]"             → 2 (EX/seconds bracketed)
 *   "channel [channel …]"                    → 1 (tail is variadic)
 *   "[message]"                              → 0
 */
export function requiredArgCount(spec: CommandSpec): number {
  let depth = 0;
  let count = 0;
  let inToken = false;
  for (const ch of spec.args) {
    if (ch === "[") {
      depth++;
      inToken = false;
      continue;
    }
    if (ch === "]") {
      depth = Math.max(0, depth - 1);
      inToken = false;
      continue;
    }
    if (depth > 0) continue;
    if (/\s/.test(ch)) {
      inToken = false;
      continue;
    }
    if (!inToken) {
      count++;
      inToken = true;
    }
  }
  return count;
}

/**
 * Pre-flight arity check. Tokens include the command name (and its
 * subcommand for two-word names). Returns a human-readable error
 * string when arity is short; `null` when the command can be sent —
 * either because arity is sufficient or because the command isn't in
 * the catalog (server-authoritative path).
 */
export function validateArgs(tokens: string[]): string | null {
  const spec = findCommand(tokens);
  if (!spec) return null;
  const specNameTokens = spec.name.split(" ").length;
  const supplied = tokens.length - specNameTokens;
  const required = requiredArgCount(spec);
  if (supplied < required) {
    return `${spec.name} expects ${required} argument${required === 1 ? "" : "s"} (${spec.args}); got ${supplied}.`;
  }
  return null;
}

export function suggestCommands(input: string): CommandSpec[] {
  const trimmed = input.trim().toUpperCase();
  if (trimmed === "") return [];
  const tokens = trimmed.split(/\s+/);
  const first = tokens[0];
  return REDIS_COMMANDS.filter((spec) => {
    if (tokens.length === 1) {
      return spec.name.startsWith(first);
    }
    const specHead = spec.name.split(" ")[0];
    if (specHead !== first) return false;
    const second = tokens[1] ?? "";
    if (second === "") return true;
    const specSubtoken = spec.name.split(" ")[1] ?? "";
    return specSubtoken.startsWith(second);
  })
    .slice()
    .sort((a, b) => {
      const aHit = a.name.startsWith(trimmed) ? 0 : 1;
      const bHit = b.name.startsWith(trimmed) ? 0 : 1;
      if (aHit !== bHit) return aHit - bHit;
      return a.name.length - b.name.length;
    })
    .slice(0, 12);
}
