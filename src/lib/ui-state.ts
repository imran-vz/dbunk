/* oxlint-disable anti-slop/no-runtime-typeof -- SSR/browser boundaries and persisted-value validation live in this module by design. */
/**
 * UI-state store (DESIGN-SYSTEM §8, P8) — the single read/write point
 * for persisted UI state. In the desktop app, state lives in the app
 * SQLite database under namespaced, versioned `ui.v1.*` keys (loaded
 * once into an in-memory cache at boot; writes are debounced batches).
 * In a plain browser (dev, tests) the module passes through to
 * localStorage so behavior is unchanged there.
 *
 * Callers keep using the historical `dbunk.*` key names; this module
 * maps them onto the `ui.v1.*` namespace at the persistence boundary.
 * A one-shot migration imports existing `dbunk.*` localStorage values
 * into SQLite and removes them — after it, localStorage holds only the
 * theme/density boot-cache mirrors the pre-paint script reads.
 *
 * Corrupt-state rule: every consumer parses persisted values inside
 * try/catch and falls back to defaults; this module itself never
 * throws and never blocks launch on a storage failure.
 */

import { isTauri, tauriInvoke } from "@/lib/tauri";

type UiStateEntry = { key: string; value: string };

/** Boot-cache keys the pre-paint script reads — they stay in localStorage. */
const BOOT_CACHE_KEYS = new Set([
  "dbunk.theme",
  "dbunk.theme.preset",
  "dbunk.density",
]);

/** Dead keys removed outright during migration (P0/P4 leftovers). */
const DEAD_KEY_PATTERNS = [
  /^dbunk\.workbench\.dock\./,
  /^dbunk\.sidebar\.global/,
];

const MIGRATED_KEY = "ui.v1.migrated";
const FLUSH_DELAY_MS = 300;
const RETRY_DELAY_MS = 2000;

/**
 * Mirrors of the backend limits (storage.rs UI_STATE_MAX_*): the save
 * command rejects the whole batch when any entry exceeds them, so one
 * oversized value enqueued without validation would deterministically
 * fail — and wedge — every future flush.
 */
export const UI_STATE_MAX_VALUE_BYTES = 512 * 1024;
export const UI_STATE_MAX_KEY_BYTES = 512;

const utf8Encoder = new TextEncoder();

/**
 * True when the UTF-8 encoding of `text` exceeds `limitBytes`, with fast
 * paths that avoid encoding: bytes are always between `length` (all
 * ASCII) and `3 * length` (BMP worst case; astral pairs average 2/unit).
 */
export function exceedsUtf8Length(text: string, limitBytes: number): boolean {
  if (text.length * 3 <= limitBytes) return false;
  if (text.length > limitBytes) return true;
  return utf8Encoder.encode(text).length > limitBytes;
}

/** Validation mirror of the backend guards; oversized entries are dropped. */
function isPersistableEntry(storeKey: string, value: string): boolean {
  if (exceedsUtf8Length(storeKey, UI_STATE_MAX_KEY_BYTES)) {
    console.error(`UI-state key too long; not persisted: ${storeKey}`);
    return false;
  }
  if (exceedsUtf8Length(value, UI_STATE_MAX_VALUE_BYTES)) {
    console.error(
      `UI-state value for '${storeKey}' exceeds ${UI_STATE_MAX_VALUE_BYTES} bytes; not persisted`,
    );
    return false;
  }
  return true;
}

const toStoreKey = (key: string): string =>
  `ui.v1.${key.startsWith("dbunk.") ? key.slice("dbunk.".length) : key}`;

let cache: Map<string, string> | null = null;
let ready = false;
let initPromise: Promise<void> | null = null;

const dirtyKeys = new Map<string, string>();
const deletedKeys = new Set<string>();
const deletedPrefixes = new Set<string>();
/**
 * Legacy `dbunk.*` localStorage keys awaiting removal — they are only
 * deleted after the SQLite batch holding their migrated values commits,
 * so an exit or write failure during the debounce window can't leave
 * the values in neither storage.
 */
let pendingLegacyRemovals: string[] = [];
let flushTimer: ReturnType<typeof setTimeout> | null = null;

const usesSqlite = (): boolean => isTauri();

export function isUiStateReady(): boolean {
  return ready || !usesSqlite();
}

/**
 * Loads the ui.v1 namespace into the cache and runs the one-shot
 * localStorage migration. Resolves (never rejects) even when storage
 * fails — the app must always launch.
 */
export function initUiState(): Promise<void> {
  if (!usesSqlite()) {
    ready = true;
    return Promise.resolve();
  }
  if (initPromise) return initPromise;
  initPromise = (async () => {
    try {
      const entries = await tauriInvoke<UiStateEntry[]>("load_ui_state");
      cache = new Map(
        (Array.isArray(entries) ? entries : []).map((entry) => [
          entry.key,
          entry.value,
        ]),
      );
      migrateLocalStorage();
    } catch (error) {
      console.error("Failed to load UI state; using defaults", error);
      cache = new Map();
    }
    ready = true;
    // Best-effort flush of the last debounce window on quit.
    if (typeof window !== "undefined") {
      window.addEventListener("beforeunload", () => {
        void flushUiState();
      });
    }
  })();
  return initPromise;
}

/** One-shot import of legacy `dbunk.*` localStorage values (P8). */
function migrateLocalStorage(): void {
  if (typeof window === "undefined" || !cache) return;
  try {
    const alreadyMigrated = cache.has(MIGRATED_KEY);
    const toRemove: string[] = [];
    for (let index = 0; index < window.localStorage.length; index += 1) {
      const key = window.localStorage.key(index);
      if (!key || !key.startsWith("dbunk.")) continue;
      if (BOOT_CACHE_KEYS.has(key)) continue;
      if (!alreadyMigrated) {
        const value = DEAD_KEY_PATTERNS.some((pattern) => pattern.test(key))
          ? null
          : window.localStorage.getItem(key);
        if (value !== null) {
          const storeKey = toStoreKey(key);
          if (!cache.has(storeKey)) {
            // An unpersistable value keeps its legacy localStorage copy —
            // never delete the only copy of something we couldn't import.
            if (!isPersistableEntry(storeKey, value)) continue;
            cache.set(storeKey, value);
            dirtyKeys.set(storeKey, value);
          }
        }
      }
      toRemove.push(key);
    }
    // Removal is deferred until the migrated values are durably in
    // SQLite (see flushUiState) — never delete the only copy first.
    pendingLegacyRemovals.push(...toRemove);
    if (!alreadyMigrated) {
      cache.set(MIGRATED_KEY, "1");
      dirtyKeys.set(MIGRATED_KEY, "1");
    }
    if (!alreadyMigrated || toRemove.length > 0) {
      scheduleFlush();
    }
  } catch (error) {
    console.error("UI-state migration failed; continuing", error);
  }
}

function scheduleFlush(delayMs: number = FLUSH_DELAY_MS): void {
  if (flushTimer !== null) return;
  flushTimer = setTimeout(() => {
    flushTimer = null;
    void flushUiState();
  }, delayMs);
}

/**
 * Serializes flushes: Tauri command handlers run concurrently and each
 * batch commits its own transaction, so two overlapping flushes could
 * commit out of order and leave a stale value durable while the cache
 * holds the newer one.
 */
let flushChain: Promise<void> = Promise.resolve();

/**
 * Persist pending writes/deletes now (also used by tests + shutdown).
 * On failure the snapshot is merged back into the pending sets (without
 * clobbering anything queued while the flush was in flight) and a retry
 * is scheduled — a transiently locked SQLite must not silently drop the
 * latest hot-exit SQL or layout changes.
 */
export function flushUiState(): Promise<void> {
  const run = flushChain.then(runFlush);
  // The chain itself must never reject, or one failure would poison it.
  flushChain = run.catch(() => {});
  return run;
}

async function runFlush(): Promise<void> {
  if (!usesSqlite()) return;
  if (flushTimer !== null) {
    clearTimeout(flushTimer);
    flushTimer = null;
  }
  const writes = [...dirtyKeys];
  dirtyKeys.clear();
  const keys = [...deletedKeys];
  const prefixes = [...deletedPrefixes];
  deletedKeys.clear();
  deletedPrefixes.clear();
  try {
    if (keys.length > 0 || prefixes.length > 0) {
      await tauriInvoke("delete_ui_state", { payload: { keys, prefixes } });
    }
    if (writes.length > 0) {
      await tauriInvoke("save_ui_state", {
        payload: { entries: writes.map(([key, value]) => ({ key, value })) },
      });
    }
    commitLegacyRemovals();
  } catch (error) {
    console.error("Failed to persist UI state; retry scheduled", error);
    for (const prefix of prefixes) {
      deletedPrefixes.add(prefix);
    }
    for (const key of keys) {
      if (!dirtyKeys.has(key)) deletedKeys.add(key);
    }
    for (const [key, value] of writes) {
      // Keep newer values / deletes queued during the failed flush.
      if (dirtyKeys.has(key) || deletedKeys.has(key)) continue;
      // Belt-and-braces: never requeue an entry the backend will always
      // reject, or the retry loop would fail deterministically forever.
      if (!isPersistableEntry(key, value)) continue;
      let coveredByPrefix = false;
      for (const prefix of deletedPrefixes) {
        if (key.startsWith(prefix)) {
          coveredByPrefix = true;
          break;
        }
      }
      if (!coveredByPrefix) dirtyKeys.set(key, value);
    }
    scheduleFlush(RETRY_DELAY_MS);
  }
}

/** The migrated values are durable — drop the legacy localStorage copies. */
function commitLegacyRemovals(): void {
  if (pendingLegacyRemovals.length === 0 || typeof window === "undefined") {
    return;
  }
  const doomed = pendingLegacyRemovals;
  pendingLegacyRemovals = [];
  try {
    for (const key of doomed) {
      window.localStorage.removeItem(key);
    }
  } catch {
    // Best-effort cleanup; stale legacy keys are re-collected next boot.
  }
}

export function uiGet(key: string): string | null {
  if (!usesSqlite()) {
    try {
      return typeof window === "undefined"
        ? null
        : window.localStorage.getItem(key);
    } catch {
      return null;
    }
  }
  return cache?.get(toStoreKey(key)) ?? null;
}

export function uiSet(key: string, value: string): void {
  if (!usesSqlite()) {
    try {
      window.localStorage.setItem(key, value);
    } catch {
      // Best-effort persistence.
    }
    return;
  }
  const storeKey = toStoreKey(key);
  if (cache?.get(storeKey) === value) return;
  // Dropped (not truncated): the backend rejects the whole batch on any
  // oversized entry, so enqueueing it would wedge every future flush.
  if (!isPersistableEntry(storeKey, value)) return;
  cache?.set(storeKey, value);
  deletedKeys.delete(storeKey);
  dirtyKeys.set(storeKey, value);
  scheduleFlush();
}

export function uiRemove(key: string): void {
  if (!usesSqlite()) {
    try {
      window.localStorage.removeItem(key);
    } catch {
      // Best-effort persistence.
    }
    return;
  }
  const storeKey = toStoreKey(key);
  cache?.delete(storeKey);
  dirtyKeys.delete(storeKey);
  deletedKeys.add(storeKey);
  scheduleFlush();
}

/** Removes every key under a prefix — connection GC uses this. */
export function uiRemovePrefix(prefix: string): void {
  if (!usesSqlite()) {
    try {
      if (typeof window === "undefined") return;
      const doomed: string[] = [];
      for (let index = 0; index < window.localStorage.length; index += 1) {
        const key = window.localStorage.key(index);
        if (key?.startsWith(prefix)) doomed.push(key);
      }
      for (const key of doomed) window.localStorage.removeItem(key);
    } catch {
      // Best-effort persistence.
    }
    return;
  }
  const storePrefix = toStoreKey(prefix);
  if (cache) {
    const doomed: string[] = [];
    for (const key of cache.keys()) {
      if (key.startsWith(storePrefix)) doomed.push(key);
    }
    for (const key of doomed) {
      cache.delete(key);
      dirtyKeys.delete(key);
    }
  }
  deletedPrefixes.add(storePrefix);
  scheduleFlush();
}

/** Test hook: drop module state so each test starts cold. */
export function resetUiStateForTests(): void {
  cache = null;
  ready = false;
  initPromise = null;
  dirtyKeys.clear();
  deletedKeys.clear();
  deletedPrefixes.clear();
  pendingLegacyRemovals = [];
  flushChain = Promise.resolve();
  if (flushTimer !== null) {
    clearTimeout(flushTimer);
    flushTimer = null;
  }
}
