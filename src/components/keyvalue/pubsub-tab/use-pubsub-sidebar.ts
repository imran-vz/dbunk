/**
 * Resizable channels-sidebar geometry for the Pub/Sub tab:
 * persisted width, viewport-aware effective width, and the
 * auto-collapse behaviour on narrow containers.
 *
 * Extracted from `PubsubTab` to keep the component shell below
 * fallow's cognitive-complexity threshold.
 */

import { type RefObject, useEffect, useMemo, useState } from "react";

import {
  useContainerWidth,
  useResizableWidth,
} from "@/lib/use-resizable-width";

import {
  PUBSUB_AUTO_HIDE_BELOW_PX,
  PUBSUB_SIDEBAR_DEFAULT,
  PUBSUB_SIDEBAR_MAX,
  PUBSUB_SIDEBAR_MIN,
} from "./constants";

export interface PubsubSidebar {
  containerRef: RefObject<HTMLDivElement | null>;
  containerWidth: number;
  width: number;
  effectiveWidth: number;
  setWidth: (next: number) => void;
  collapsed: boolean;
  collapse: () => void;
  expand: () => void;
}

function computeEffectiveWidth(
  collapsed: boolean,
  containerWidth: number,
  storedWidth: number,
): number {
  if (collapsed) return 0;
  if (containerWidth === 0) return storedWidth;
  const maxForViewport = Math.max(
    PUBSUB_SIDEBAR_MIN,
    Math.floor(containerWidth * 0.4),
  );
  return Math.min(storedWidth, maxForViewport);
}

export function usePubsubSidebar(): PubsubSidebar {
  const {
    width,
    setWidth,
    collapsed,
    setCollapsed: setStoredCollapsed,
  } = useResizableWidth({
    storageKey: "dbunk.redis.pubsubSidebarWidth",
    defaultWidth: PUBSUB_SIDEBAR_DEFAULT,
    min: PUBSUB_SIDEBAR_MIN,
    max: PUBSUB_SIDEBAR_MAX,
  });
  const [containerRef, containerWidth] = useContainerWidth<HTMLDivElement>();
  const [autoCollapsed, setAutoCollapsed] = useState(false);

  useEffect(() => {
    if (containerWidth === 0) return;
    const narrow = containerWidth < PUBSUB_AUTO_HIDE_BELOW_PX;
    if (narrow && !collapsed) {
      setStoredCollapsed(true);
      setAutoCollapsed(true);
      return;
    }
    if (!narrow && autoCollapsed) {
      setStoredCollapsed(false);
      setAutoCollapsed(false);
    }
  }, [containerWidth, collapsed, autoCollapsed, setStoredCollapsed]);

  const effectiveWidth = useMemo(
    () => computeEffectiveWidth(collapsed, containerWidth, width),
    [collapsed, containerWidth, width],
  );

  const collapse = () => {
    setStoredCollapsed(true);
    setAutoCollapsed(false);
  };

  const expand = () => {
    setStoredCollapsed(false);
    setAutoCollapsed(false);
  };

  return {
    containerRef,
    containerWidth,
    width,
    effectiveWidth,
    setWidth,
    collapsed,
    collapse,
    expand,
  };
}
