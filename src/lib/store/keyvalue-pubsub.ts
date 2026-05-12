/**
 * KeyValue Pub/Sub slice — placeholder for client-side Pub/Sub
 * Session metadata. Today the backend owns the ring buffer (per
 * `redis_pubsub_drain`); component state owns active patterns and UI
 * toggles. This slice exists for cascade-contract clarity.
 *
 * Exposes `closePubSubSessionsForConnection(connectionId)` as its
 * piece of the delete-connection cleanup cascade. When the deferred
 * pub/sub auto-reconnect feature lands, this slice will track
 * per-session client state and the cleanup will tear down lingering
 * sessions. For now the method is a documented no-op — the backend's
 * session lifecycle is managed by the PubsubTab component's
 * `closePubsubSession` on unmount.
 */

import type { StateCreator } from "zustand";

import type { AppStoreState } from "./types";

export type KeyValuePubSubSlice = {
  /**
   * Cascade cleanup — see slice doc. Currently a no-op.
   */
  closePubSubSessionsForConnection: (connectionId: string) => void;
};

export const createKeyValuePubSubSlice: StateCreator<
  AppStoreState,
  [],
  [],
  KeyValuePubSubSlice
> = () => ({
  closePubSubSessionsForConnection: (_connectionId: string) => {
    // No state to clean today. Reserved for future per-session
    // pub/sub auto-reconnect metadata.
  },
});
