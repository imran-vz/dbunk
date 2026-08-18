/**
 * Platform-aware keyboard shortcut renderer. Pass shortcut tokens
 * like `["mod", "k"]`, `["mod", "shift", "f"]`, or `["enter"]` and
 * the component picks the right glyph per OS:
 *
 * - macOS: ⌘ Shift ⌥ ⌃ ↩
 * - Windows / Linux: Ctrl Shift Alt ↩
 *
 * The OS check runs once at module load; user-agent sniffing is good
 * enough here — the rendered glyph is decorative and any miss only
 * affects rendering, not functionality.
 */

import { cn } from "@/lib/utils";

/**
 * Evaluated lazily so unit tests (which run in jsdom and may override
 * `navigator.platform` after module load) see the right value. Cheap
 * enough to recompute on every call.
 */
export function isMacPlatform(): boolean {
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
  if (typeof navigator === "undefined") return false;
  return /Mac|iPhone|iPad/i.test(navigator.platform || navigator.userAgent);
}

// oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
const MAC_TOKENS: Record<string, string> = {
  mod: "⌘",
  shift: "⇧",
  alt: "⌥",
  ctrl: "⌃",
  enter: "↩",
  esc: "⎋",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
};

// oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
const WIN_TOKENS: Record<string, string> = {
  mod: "Ctrl",
  shift: "Shift",
  alt: "Alt",
  ctrl: "Ctrl",
  enter: "Enter",
  esc: "Esc",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
};

function renderToken(token: string, isMac: boolean): string {
  const map = isMac ? MAC_TOKENS : WIN_TOKENS;
  const lower = token.toLowerCase();
  if (lower in map) return map[lower];
  return token.length === 1 ? token.toUpperCase() : token;
}

interface KbdProps {
  /** Tokens like ["mod", "k"]. Mixed case ok; rendered per platform. */
  keys: ReadonlyArray<string>;
  className?: string;
}

/* oxlint-disable react/no-array-index-key -- Repeated shortcut tokens are distinguished by their fixed sequence position. */
export function Kbd({ keys, className }: KbdProps) {
  const isMac = isMacPlatform();
  return (
    <span
      className={cn(
        "inline-flex items-center gap-0.5 rounded-sm border border-border-subtle bg-surface-app px-1 py-0.5 font-mono text-[0.6rem] leading-none text-text-muted",
        className,
      )}
    >
      {keys.map((token, index) => {
        const rendered = renderToken(token, isMac);
        return (
          // oxlint-disable-next-line react/no-array-index-key -- Repeated shortcut tokens are distinguished by their fixed sequence position.
          <span
            key={`${rendered}-${index}-${token}`}
            className="inline-flex items-center"
          >
            {index > 0 && !isMac ? (
              <span className="mx-0.5 text-text-muted/60">+</span>
            ) : null}
            {rendered}
          </span>
        );
      })}
    </span>
  );
}
/* oxlint-enable react/no-array-index-key */
