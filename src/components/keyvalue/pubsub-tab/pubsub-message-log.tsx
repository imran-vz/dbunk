/**
 * Scrolling message log for the Pub/Sub tab: renders the most recent
 * messages (capped to the tail-slice display window) for the
 * currently filtered channel set, or a "waiting" placeholder when
 * the buffer is empty.
 *
 * Extracted from `PubsubTab` to keep the parent below fallow's
 * cognitive-complexity threshold.
 */

import { type DrainedMessage, formatValueOneLine } from "@/lib/redis/api";

const DISPLAY_TAIL = 500;

interface PubsubMessageLogProps {
  messages: DrainedMessage[];
  activePatterns: string[];
}

/* oxlint-disable react/no-array-index-key -- Duplicate Pub/Sub deliveries can share every payload field, so receipt order completes their identity. */
export function PubsubMessageLog({
  messages,
  activePatterns,
}: PubsubMessageLogProps) {
  if (messages.length === 0) {
    return (
      <p className="text-text-muted">
        Waiting for messages on {activePatterns.join(", ")}…
      </p>
    );
  }

  return (
    <ul className="space-y-0.5">
      {messages.slice(-DISPLAY_TAIL).map((msg, idx) => (
        // oxlint-disable-next-line react/no-array-index-key -- Duplicate Pub/Sub deliveries can share every payload field, so receipt order completes their identity.
        <li
          key={`${msg.receivedAtMs}-${msg.channel}-${idx}`}
          className="border-b border-border-subtle py-1"
        >
          <span className="text-text-muted">
            [{new Date(msg.receivedAtMs).toISOString().slice(11, 23)}]
          </span>{" "}
          <span className="text-primary">{msg.channel}</span>{" "}
          <span className="break-all">{formatValueOneLine(msg.payload)}</span>
        </li>
      ))}
    </ul>
  );
}
/* oxlint-enable react/no-array-index-key */
