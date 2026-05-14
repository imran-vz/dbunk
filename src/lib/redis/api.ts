/**
 * Typed Tauri bindings for the Redis backend commands.
 *
 * Each function wraps `tauriInvoke<T>("redis_*", payload)` with a
 * camelCase payload shape (serde converts to snake_case for us). The
 * frontend components import these instead of touching `tauriInvoke`
 * directly so the wire shapes stay one grep away.
 */

import { tauriInvoke } from "@/lib/tauri";

// ---------------------------------------------------------------------------
// Serialised value (matches `redis/value.rs::SerializedValue`)
// ---------------------------------------------------------------------------

export type SerializedValue =
  | { kind: "nil" }
  | { kind: "int"; value: number }
  | { kind: "status"; value: string }
  | { kind: "string"; value: string; encoding: "utf8" | "hex" | string }
  | { kind: "error"; value: string }
  | { kind: "array"; value: SerializedValue[] };

// ---------------------------------------------------------------------------
// Keyspace
// ---------------------------------------------------------------------------

export type ScannedKey = {
  name: string;
  type: string;
};

export type ScanKeysPayload = {
  connectionId: string;
  pattern?: string;
  count?: number;
  typeFilter?: string;
  cursor?: string | null;
};

export type ScanKeysResult = {
  keys: ScannedKey[];
  nextCursor: string | null;
};

export function scanKeys(payload: ScanKeysPayload): Promise<ScanKeysResult> {
  return tauriInvoke<ScanKeysResult>("redis_scan_keys", { payload });
}

// ---------------------------------------------------------------------------
// Key metadata
// ---------------------------------------------------------------------------

export type KeyMetadata = {
  type: string;
  ttlSeconds: number;
  encoding?: string;
  elementCount?: number;
  memoryBytes?: number;
};

export function fetchKeyMetadata(payload: {
  connectionId: string;
  key: string;
}): Promise<KeyMetadata> {
  return tauriInvoke<KeyMetadata>("redis_fetch_key_metadata", { payload });
}

// ---------------------------------------------------------------------------
// String
// ---------------------------------------------------------------------------

export type StringValuePayload = {
  value: SerializedValue;
  totalBytes: number;
  truncated: boolean;
};

export function fetchString(payload: {
  connectionId: string;
  key: string;
  maxBytes?: number;
}): Promise<StringValuePayload> {
  return tauriInvoke<StringValuePayload>("redis_fetch_string", { payload });
}

// ---------------------------------------------------------------------------
// Hash
// ---------------------------------------------------------------------------

export type HashValuePayload = {
  entries: Array<[SerializedValue, SerializedValue]>;
  nextCursor: string | null;
};

export function fetchHash(payload: {
  connectionId: string;
  key: string;
  mode?: "full" | "scan";
  count?: number;
  cursor?: string | null;
  pattern?: string | null;
}): Promise<HashValuePayload> {
  return tauriInvoke<HashValuePayload>("redis_fetch_hash", { payload });
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

export type ListValuePayload = {
  items: SerializedValue[];
};

export function fetchList(payload: {
  connectionId: string;
  key: string;
  start?: number;
  stop?: number;
  reverse?: boolean;
}): Promise<ListValuePayload> {
  return tauriInvoke<ListValuePayload>("redis_fetch_list", { payload });
}

// ---------------------------------------------------------------------------
// Set
// ---------------------------------------------------------------------------

export type SetValuePayload = {
  members: SerializedValue[];
  nextCursor: string | null;
};

export function fetchSet(payload: {
  connectionId: string;
  key: string;
  mode?: "full" | "scan";
  count?: number;
  cursor?: string | null;
  pattern?: string | null;
}): Promise<SetValuePayload> {
  return tauriInvoke<SetValuePayload>("redis_fetch_set", { payload });
}

// ---------------------------------------------------------------------------
// Sorted set
// ---------------------------------------------------------------------------

export type SortedSetValuePayload = {
  entries: Array<[SerializedValue, number]>;
};

export function fetchSortedSet(payload: {
  connectionId: string;
  key: string;
  mode?: "rank" | "byscore";
  start?: number;
  stop?: number;
  reverse?: boolean;
  scoreMin?: string;
  scoreMax?: string;
}): Promise<SortedSetValuePayload> {
  return tauriInvoke<SortedSetValuePayload>("redis_fetch_sorted_set", {
    payload,
  });
}

// ---------------------------------------------------------------------------
// Stream
// ---------------------------------------------------------------------------

export type StreamEntry = {
  id: string;
  fields: Array<[SerializedValue, SerializedValue]>;
};

export type StreamValuePayload = {
  entries: StreamEntry[];
};

export function fetchStream(payload: {
  connectionId: string;
  key: string;
  start?: string;
  end?: string;
  count?: number;
  reverse?: boolean;
}): Promise<StreamValuePayload> {
  return tauriInvoke<StreamValuePayload>("redis_fetch_stream", { payload });
}

export type StreamGroupInfo = {
  name: string;
  consumers: number;
  pending: number;
  lastDeliveredId: string;
};

export type StreamConsumerInfo = {
  group: string;
  name: string;
  pending: number;
  idleMs: number;
};

export type StreamGroupsPayload = {
  groups: StreamGroupInfo[];
  consumers: StreamConsumerInfo[];
};

export function fetchStreamGroups(payload: {
  connectionId: string;
  key: string;
}): Promise<StreamGroupsPayload> {
  return tauriInvoke<StreamGroupsPayload>("redis_fetch_stream_groups", {
    payload,
  });
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

export type JsonValuePayload = {
  value: string;
};

export function fetchJson(payload: {
  connectionId: string;
  key: string;
  path?: string;
}): Promise<JsonValuePayload> {
  return tauriInvoke<JsonValuePayload>("redis_fetch_json", { payload });
}

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Writes (Phase 1.4)
// ---------------------------------------------------------------------------

export function setRedisString(payload: {
  connectionId: string;
  key: string;
  value: string;
  ttlSeconds?: number | null;
  persist?: boolean;
}): Promise<{ ok: boolean }> {
  return tauriInvoke<{ ok: boolean }>("redis_set_string", { payload });
}

export function setRedisHashFields(payload: {
  connectionId: string;
  key: string;
  entries: Array<[string, string]>;
}): Promise<void> {
  return tauriInvoke<void>("redis_set_hash_fields", { payload });
}

export function deleteRedisHashFields(payload: {
  connectionId: string;
  key: string;
  fields: string[];
}): Promise<void> {
  return tauriInvoke<void>("redis_delete_hash_fields", { payload });
}

export function delRedisKeys(payload: {
  connectionId: string;
  keys: string[];
}): Promise<{ deleted: number }> {
  return tauriInvoke<{ deleted: number }>("redis_del_keys", { payload });
}

export function setRedisExpire(payload: {
  connectionId: string;
  key: string;
  ttlSeconds?: number | null;
}): Promise<void> {
  return tauriInvoke<void>("redis_set_expire", { payload });
}

export function renameRedisKey(payload: {
  connectionId: string;
  from: string;
  to: string;
}): Promise<void> {
  return tauriInvoke<void>("redis_rename_key", { payload });
}

export type CreateKeyType =
  | "string"
  | "hash"
  | "list"
  | "set"
  | "zset"
  | "stream"
  | "json";

export function createRedisKey(payload: {
  connectionId: string;
  key: string;
  type: CreateKeyType;
  payload: unknown;
  ttlSeconds?: number | null;
}): Promise<void> {
  return tauriInvoke<void>("redis_create_key", { payload });
}

// ---------------------------------------------------------------------------
// List / Set / ZSet / Stream / JSON write ops (Tier 2)
// ---------------------------------------------------------------------------

export type ListEdit = {
  index: number;
  value: string;
};

export function applyRedisListEdits(payload: {
  connectionId: string;
  key: string;
  sets?: ListEdit[];
  deletes?: number[];
  appends?: string[];
}): Promise<void> {
  return tauriInvoke<void>("redis_apply_list_edits", { payload });
}

export function applyRedisSetEdits(payload: {
  connectionId: string;
  key: string;
  adds?: string[];
  removes?: string[];
}): Promise<void> {
  return tauriInvoke<void>("redis_apply_set_edits", { payload });
}

export type SortedSetUpsert = {
  member: string;
  score: number;
};

export function applyRedisSortedSetEdits(payload: {
  connectionId: string;
  key: string;
  upserts?: SortedSetUpsert[];
  removes?: string[];
}): Promise<void> {
  return tauriInvoke<void>("redis_apply_sorted_set_edits", { payload });
}

export type StreamAppendEntry = {
  id?: string | null;
  fields: Array<[string, string]>;
};

export function applyRedisStreamEdits(payload: {
  connectionId: string;
  key: string;
  appends?: StreamAppendEntry[];
  deletes?: string[];
  trimMaxlen?: number | null;
}): Promise<void> {
  return tauriInvoke<void>("redis_apply_stream_edits", { payload });
}

export function setRedisJsonPath(payload: {
  connectionId: string;
  key: string;
  path?: string;
  value: string;
}): Promise<void> {
  return tauriInvoke<void>("redis_set_json_path", { payload });
}

export function deleteRedisJsonPath(payload: {
  connectionId: string;
  key: string;
  path: string;
}): Promise<void> {
  return tauriInvoke<void>("redis_delete_json_path", { payload });
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

export type RunCommandResult =
  | { kind: "ok"; value: SerializedValue; runtimeMs: number }
  | { kind: "needs-confirmation"; command: string; severity: "hard" | "soft" }
  | { kind: "rejected"; reason: string };

export function runRedisCommand(payload: {
  connectionId: string;
  tokens: string[];
  confirmed?: boolean;
}): Promise<RunCommandResult> {
  return tauriInvoke<RunCommandResult>("redis_run_command", { payload });
}

// ---------------------------------------------------------------------------
// Server overview
// ---------------------------------------------------------------------------

export type KeyValueOverviewStats = {
  identity: {
    version?: string;
    mode?: string;
    uptimeSeconds?: number;
    os?: string;
    arch?: string;
  };
  keyspace: Array<{ dbNumber: number; keys: number; expires: number }>;
  memory?: {
    usedMemory?: number;
    usedMemoryRss?: number;
    usedMemoryPeak?: number;
    fragmentationRatio?: number;
    maxmemory?: number;
    maxmemoryPolicy?: string;
  };
  clients?: {
    connectedClients: number;
    maxclients?: number;
    blockedClients: number;
    trackingClients?: number;
  };
  replication?: {
    role: string;
    connectedSlaves?: number;
    masterLinkStatus?: string;
    masterHost?: string;
    masterPort?: number;
  };
  modules?: Array<{ name: string; version: string }>;
  slowLog?: Array<{
    id: number;
    timestamp: number;
    durationUs: number;
    command: string;
    clientAddress?: string;
    clientName?: string;
  }>;
  persistence?: {
    rdbLastSaveTime?: number;
    rdbChangesSinceLastSave?: number;
    aofEnabled?: boolean;
    aofLastRewriteTime?: number;
  };
};

export function fetchKeyValueOverview(payload: {
  connectionId: string;
}): Promise<KeyValueOverviewStats> {
  return tauriInvoke<KeyValueOverviewStats>("redis_fetch_overview", {
    payload,
  });
}

// ---------------------------------------------------------------------------
// Server tab extras (Tier 2 cards)
// ---------------------------------------------------------------------------

export type ClientListEntry = {
  id: number;
  addr: string;
  name: string;
  ageSeconds: number;
  idleSeconds: number;
  flags: string;
  db: number;
  command: string;
};

export function fetchRedisClientList(payload: {
  connectionId: string;
}): Promise<{ entries: ClientListEntry[] }> {
  return tauriInvoke<{ entries: ClientListEntry[] }>(
    "redis_fetch_client_list",
    {
      payload,
    },
  );
}

export type AclListEntry = {
  username: string;
  rules: string;
};

export function fetchRedisAclList(payload: {
  connectionId: string;
}): Promise<{ entries: AclListEntry[] }> {
  return tauriInvoke<{ entries: AclListEntry[] }>("redis_fetch_acl_list", {
    payload,
  });
}

export type RedisConfigEntry = {
  key: string;
  value: string;
};

export function fetchRedisConfig(payload: {
  connectionId: string;
  pattern?: string;
}): Promise<{ entries: RedisConfigEntry[] }> {
  return tauriInvoke<{ entries: RedisConfigEntry[] }>("redis_fetch_config", {
    payload,
  });
}

export type LatencyEntry = {
  event: string;
  timestamp: number;
  latestLatencyMs: number;
  maxLatencyMs: number;
};

export function fetchRedisLatency(payload: {
  connectionId: string;
}): Promise<{ entries: LatencyEntry[] }> {
  return tauriInvoke<{ entries: LatencyEntry[] }>("redis_fetch_latency", {
    payload,
  });
}

// ---------------------------------------------------------------------------
// Pub/Sub
// ---------------------------------------------------------------------------

export function startPubsubSession(payload: {
  connectionId: string;
  sessionId: string;
  patterns: string[];
}): Promise<{ sessionId: string }> {
  return tauriInvoke<{ sessionId: string }>("redis_pubsub_start", { payload });
}

export type DrainedMessage = {
  receivedAtMs: number;
  channel: string;
  patternMatched: string;
  payload: SerializedValue;
};

export type DrainResult = {
  messages: DrainedMessage[];
  channels: Array<{ channel: string; count: number }>;
  dropped: number;
  patterns: string[];
};

export function drainPubsub(payload: {
  sessionId: string;
}): Promise<DrainResult> {
  return tauriInvoke<DrainResult>("redis_pubsub_drain", { payload });
}

export function closePubsubSession(payload: {
  sessionId: string;
}): Promise<void> {
  return tauriInvoke<void>("redis_pubsub_close", { payload });
}

export type DiscoveredChannel = {
  channel: string;
  subscribers: number;
};

export function discoverPubsubChannels(payload: {
  connectionId: string;
  pattern?: string;
}): Promise<{ channels: DiscoveredChannel[] }> {
  return tauriInvoke<{ channels: DiscoveredChannel[] }>(
    "redis_pubsub_discover",
    { payload },
  );
}

/** Render a `SerializedValue` as a single line — used by row gutters
 * in the hash / list / set / zset viewers when the cell isn't being
 * inspected in detail. */
export function formatValueOneLine(value: SerializedValue): string {
  switch (value.kind) {
    case "nil":
      return "(nil)";
    case "int":
      return value.value.toString();
    case "status":
      return value.value;
    case "string":
      return value.value;
    case "error":
      return `(error) ${value.value}`;
    case "array":
      return `[${value.value.length} items]`;
  }
}
