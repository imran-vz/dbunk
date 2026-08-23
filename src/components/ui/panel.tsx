/**
 * Panel — the one primitive behind every sidebar and auxiliary panel
 * (DESIGN-SYSTEM §3.2–3.4). A panel is a sized, collapsible region
 * with a `Sash` on its inner edge: drag to resize (snap-close below
 * threshold), double-click to auto-fit, Alt+double-click / Enter to
 * collapse, edge-drag to restore when collapsed. Collapsed costs 0px
 * (only the sash's hit strip remains as the edge-drag affordance).
 *
 * Pressure-collapse (window too small, §3.6) and user-collapse are
 * distinct states: pressure never touches the persisted user flag, so
 * panels auto-collapsed by pressure restore themselves when space
 * returns.
 *
 * Sizes and the user-collapsed flag persist through the P8 UI-state
 * store (SQLite in the app; localStorage in a plain browser).
 */

import {
  IconChevronLeft,
  IconChevronRight,
  IconChevronUp,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Sash } from "@/components/ui/resizer-handle";
import { uiGet, uiSet } from "@/lib/ui-state";
import { cn } from "@/lib/utils";

export type PanelSide = "left" | "right" | "bottom";

export interface UsePanelStateOptions {
  /** localStorage namespace — stable per panel. */
  storageKey: string;
  defaultSize: number;
  min: number;
  /** Upper bound; pass a function for window-relative caps ("50% of window"). */
  max: number | (() => number);
  /** Dragging below this snaps the panel closed (§3.3). */
  snapThreshold: number;
  /** Initial collapsed state when nothing is persisted. */
  defaultCollapsed?: boolean;
}

export interface PanelState {
  size: number;
  min: number;
  max: number;
  snapThreshold: number;
  /** Persisted, user-driven. */
  userCollapsed: boolean;
  /** Transient, layout-pressure-driven (§3.6). */
  pressureCollapsed: boolean;
  /** What the layout renders: user OR pressure collapsed. */
  collapsed: boolean;
  setSize: (next: number) => void;
  collapse: () => void;
  expand: (size?: number) => void;
  toggle: () => void;
  setPressureCollapsed: (next: boolean) => void;
}

const readStoredNumber = (key: string, fallback: number): number => {
  const raw = uiGet(key);
  if (raw === null) return fallback;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
};

const readStoredBool = (key: string, fallback: boolean): boolean => {
  const raw = uiGet(key);
  if (raw === null) return fallback;
  return raw === "1";
};

const writeStored = (key: string, value: string) => {
  uiSet(key, value);
};

export function usePanelState({
  storageKey,
  defaultSize,
  min,
  max,
  snapThreshold,
  defaultCollapsed = false,
}: UsePanelStateOptions): PanelState {
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- `max` is a documented number-or-thunk union on this hook's own contract.
  const resolveMax = useCallback(
    () => (max instanceof Function ? max() : max),
    [max],
  );
  const resolvedMax = useMemo(() => resolveMax(), [resolveMax]);
  const clamp = useCallback(
    (next: number) => Math.max(min, Math.min(resolveMax(), next)),
    [min, resolveMax],
  );

  const [size, setSizeState] = useState<number>(() =>
    clamp(readStoredNumber(`${storageKey}.size`, defaultSize)),
  );
  const [userCollapsed, setUserCollapsed] = useState<boolean>(() =>
    readStoredBool(`${storageKey}.collapsed`, defaultCollapsed),
  );
  const [pressureCollapsed, setPressureCollapsed] = useState(false);

  useEffect(() => {
    writeStored(`${storageKey}.size`, String(size));
  }, [storageKey, size]);
  useEffect(() => {
    writeStored(`${storageKey}.collapsed`, userCollapsed ? "1" : "0");
  }, [storageKey, userCollapsed]);

  const setSize = useCallback(
    (next: number) => setSizeState(clamp(next)),
    [clamp],
  );
  const collapse = useCallback(() => setUserCollapsed(true), []);
  const expand = useCallback(
    (nextSize?: number) => {
      setUserCollapsed(false);
      setPressureCollapsed(false);
      if (nextSize !== undefined) setSizeState(clamp(nextSize));
    },
    [clamp],
  );
  const toggle = useCallback(() => setUserCollapsed((current) => !current), []);

  return {
    size,
    min,
    max: resolvedMax,
    snapThreshold,
    userCollapsed,
    pressureCollapsed,
    collapsed: userCollapsed || pressureCollapsed,
    setSize,
    collapse,
    expand,
    toggle,
    setPressureCollapsed,
  };
}

export interface PanelProps {
  side: PanelSide;
  state: PanelState;
  ariaLabel: string;
  /** Double-click auto-fit target size (e.g. widest tree label). */
  onAutoFit?: () => void;
  /**
   * Visible restore control when collapsed. Panels whose restore path
   * lives elsewhere (rail item, toolbar toggle, shortcut) omit this;
   * panels with no other affordance MUST pass it — the collapsed
   * state persists across sessions and the bare 1px sash strip is
   * effectively invisible.
   */
  restoreLabel?: string;
  className?: string;
  children: React.ReactNode;
}

/**
 * Renders the panel region with its sash on the inner edge. When
 * collapsed the content unmounts and only the sash strip remains,
 * flush with the edge, so dragging it outward re-opens the panel.
 */
export function Panel({
  side,
  state,
  ariaLabel,
  onAutoFit,
  restoreLabel,
  className,
  children,
}: PanelProps) {
  const isBottom = side === "bottom";
  const sashSide =
    side === "left" ? "right" : side === "right" ? "left" : "top";
  const orientation = isBottom ? "horizontal" : "vertical";

  const sash = (
    <Sash
      value={state.size}
      min={state.min}
      max={state.max}
      collapsed={state.collapsed}
      snapThreshold={state.snapThreshold}
      onResize={state.setSize}
      onCollapse={state.collapse}
      onExpand={(size) => state.expand(size)}
      onAutoFit={onAutoFit}
      side={sashSide}
      orientation={orientation}
      ariaLabel={ariaLabel}
    />
  );

  if (state.collapsed) {
    // Near-0px cost; the sash hit strip stays as the edge-drag restore
    // path, plus a visible strip button when the panel has no other
    // restore affordance.
    if (!restoreLabel) return sash;
    const RestoreChevron =
      side === "left"
        ? IconChevronRight
        : side === "right"
          ? IconChevronLeft
          : IconChevronUp;
    return (
      <div
        data-slot="panel-collapsed"
        data-side={side}
        className={cn("relative flex shrink-0", isBottom && "flex-col")}
      >
        {(side === "right" || isBottom) && sash}
        <button
          type="button"
          aria-label={restoreLabel}
          title={restoreLabel}
          onClick={() => state.expand()}
          className={cn(
            "flex items-center justify-center border-border-subtle bg-surface-panel text-text-muted transition-colors hover:bg-surface-panel-elevated hover:text-foreground",
            side === "left" && "border-r",
            side === "right" && "border-l",
            isBottom ? "h-5 w-full border-t" : "w-5",
          )}
        >
          <RestoreChevron className="size-3.5" />
        </button>
        {side === "left" && sash}
      </div>
    );
  }

  return (
    <div
      data-slot="panel"
      data-side={side}
      className={cn(
        "relative flex min-h-0 min-w-0 shrink-0",
        isBottom ? "flex-col" : "flex-row",
        className,
      )}
      style={isBottom ? { height: state.size } : { width: state.size }}
    >
      {(side === "right" || isBottom) && sash}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        {children}
      </div>
      {side === "left" && sash}
    </div>
  );
}

/**
 * Layout-pressure watcher (§3.6): observes window width and reports
 * whether panels should yield. Yield order — right panel first, then
 * navigator — with the 560px protected workspace and fixed chrome
 * accounted for by the caller via `fixedWidth`.
 */
export function useLayoutPressure({
  fixedWidth,
  navigatorState,
  rightPanelState,
  protectedContent = 560,
}: {
  /** Rail + borders + anything else that never yields. */
  fixedWidth: number;
  navigatorState: PanelState | null;
  rightPanelState?: PanelState | null;
  protectedContent?: number;
}): void {
  const navRef = useRef(navigatorState);
  navRef.current = navigatorState;
  const rightRef = useRef(rightPanelState ?? null);
  rightRef.current = rightPanelState ?? null;

  useEffect(() => {
    const evaluate = () => {
      const nav = navRef.current;
      const right = rightRef.current;
      let available = window.innerWidth - fixedWidth - protectedContent;

      const navNeeds = nav && !nav.userCollapsed ? nav.min : 0;
      const rightNeeds = right && !right.userCollapsed ? right.min : 0;

      // Right panel yields first, then the navigator.
      if (right) {
        const fits = available >= navNeeds + rightNeeds && rightNeeds > 0;
        right.setPressureCollapsed(!right.userCollapsed && !fits);
        if (fits) available -= rightNeeds;
      }
      if (nav) {
        const fits = available >= navNeeds && navNeeds > 0;
        nav.setPressureCollapsed(!nav.userCollapsed && !fits);
      }
    };
    evaluate();
    window.addEventListener("resize", evaluate);
    return () => window.removeEventListener("resize", evaluate);
    // Re-evaluate when a user expands/collapses a panel, not just on
    // window resize — pressure must re-apply immediately.
  }, [
    fixedWidth,
    protectedContent,
    navigatorState?.userCollapsed,
    rightPanelState?.userCollapsed,
  ]);
}
