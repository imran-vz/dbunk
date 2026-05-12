/**
 * KeyValue Pub/Sub slice — placeholder for client-side Pub/Sub Session
 * metadata. Today the backend owns the ring buffer (per
 * `redis_pubsub_drain`); component state owns the active patterns and
 * UI toggles. This slice exists for cascade-contract clarity.
 *
 * Exposes `closePubSubSessionsForConnection(connectionId)` as its
 * piece of the delete-connection cleanup cascade. When the deferred
 * pub/sub auto-reconnect feature lands, that's where the per-session
 * client state will live.
 *
 * Phase: scaffold only.
 */

import type { StateCreator } from "zustand";

import type { AppStoreState } from "./types";

export type KeyValuePubSubSlice = Record<string, never>;

export const createKeyValuePubSubSlice: StateCreator<
  AppStoreState,
  [],
  [],
  KeyValuePubSubSlice
> = () => ({});
