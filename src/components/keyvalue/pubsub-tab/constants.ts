/**
 * Layout + lifecycle constants for the Redis Pub/Sub workspace tab.
 *
 * Extracted alongside `PubsubTab` so the sibling hooks/components can
 * share them without re-importing the parent shell.
 */

export const PUBSUB_SIDEBAR_MIN = 160;
export const PUBSUB_SIDEBAR_MAX = 360;
export const PUBSUB_SIDEBAR_DEFAULT = 224;
export const PUBSUB_AUTO_HIDE_BELOW_PX = 560;

export const MAX_BUFFER = 10_000;
