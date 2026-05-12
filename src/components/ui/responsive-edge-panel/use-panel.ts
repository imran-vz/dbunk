import { useCallback, useEffect, useId, useRef, useState } from "react";

export type PanelSide = "left" | "right";

interface UsePanelArgs {
  storageKey: string;
  width: number;
  containerWidth: number;
  compactBelow: number;
  protectedWorkspaceWidth: number;
  defaultPinned: boolean;
  wideVisible: boolean;
  open?: boolean;
  defaultOpen: boolean;
  hasResizer: boolean;
  onOpenChange?: (open: boolean) => void;
}

export interface PanelState {
  panelId: string;
  panelRef: React.RefObject<HTMLDivElement | null>;
  pinned: boolean;
  hovered: boolean;
  setHovered: (next: boolean) => void;
  togglePinned: () => void;
  closePanel: () => void;
  setRequestedOpen: (next: boolean) => void;
  requestedOpen: boolean;
  isCompact: boolean;
  reserveSpace: boolean;
  overlayOpen: boolean;
  visible: boolean;
  showEdgeTarget: boolean;
  showResizer: boolean;
}

const readStoredBool = (key: string, fallback: boolean) => {
  if (typeof window === "undefined") return fallback;
  const raw = window.localStorage.getItem(key);
  if (raw === null) return fallback;
  return raw === "1";
};

export function usePanel({
  storageKey,
  width,
  containerWidth,
  compactBelow,
  protectedWorkspaceWidth,
  defaultPinned,
  wideVisible,
  open,
  defaultOpen,
  hasResizer,
  onOpenChange,
}: UsePanelArgs): PanelState {
  const panelId = useId();
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [pinned, setPinned] = useState(() =>
    readStoredBool(`${storageKey}.pinned`, defaultPinned),
  );
  const [uncontrolledOpen, setUncontrolledOpen] = useState(defaultOpen);
  const [hovered, setHovered] = useState(false);
  const requestedOpen = open ?? uncontrolledOpen;

  const isCompact = containerWidth > 0 ? containerWidth < compactBelow : false;
  const canReserve =
    containerWidth === 0 || containerWidth - width >= protectedWorkspaceWidth;
  const reserveSpace = !isCompact
    ? wideVisible
    : requestedOpen && pinned && canReserve;
  const overlayOpen = isCompact && !reserveSpace && (hovered || requestedOpen);
  const visible = reserveSpace || overlayOpen;
  const showEdgeTarget = isCompact && !reserveSpace && !overlayOpen;
  const showResizer = visible && reserveSpace && hasResizer;

  const setRequestedOpen = useCallback(
    (next: boolean) => {
      if (open === undefined) setUncontrolledOpen(next);
      onOpenChange?.(next);
    },
    [onOpenChange, open],
  );

  useEffect(() => {
    if (typeof window === "undefined") return;
    window.localStorage.setItem(`${storageKey}.pinned`, pinned ? "1" : "0");
  }, [pinned, storageKey]);

  useEffect(() => {
    if (!isCompact) {
      setHovered(false);
      setRequestedOpen(false);
    }
  }, [isCompact, setRequestedOpen]);

  useEffect(() => {
    if (!overlayOpen || canReserve) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (panelRef.current?.contains(target)) return;
      setRequestedOpen(false);
      setHovered(false);
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [canReserve, overlayOpen, setRequestedOpen]);

  const togglePinned = useCallback(() => {
    setPinned((next) => !next);
    setRequestedOpen(true);
  }, [setRequestedOpen]);

  const closePanel = useCallback(() => {
    setRequestedOpen(false);
    setHovered(false);
  }, [setRequestedOpen]);

  return {
    panelId,
    panelRef,
    pinned,
    hovered,
    setHovered,
    togglePinned,
    closePanel,
    setRequestedOpen,
    requestedOpen,
    isCompact,
    reserveSpace,
    overlayOpen,
    visible,
    showEdgeTarget,
    showResizer,
  };
}
