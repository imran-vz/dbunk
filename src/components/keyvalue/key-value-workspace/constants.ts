/**
 * Width breakpoints for the Redis workspace keyspace sidebar.
 *
 * Lives next to `KeyValueWorkspace` so the shell and its extracted
 * panel both reference the same numbers without a cross-file import
 * cycle.
 */

export const KEYSPACE_MIN_WIDTH = 180;
export const KEYSPACE_MAX_WIDTH = 480;
export const KEYSPACE_DEFAULT_WIDTH = 256;
export const KEYSPACE_COMPACT_BELOW = 860;
export const PROTECTED_WORKSPACE_WIDTH = 560;
