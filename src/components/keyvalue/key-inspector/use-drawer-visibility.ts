import { useCallback, useEffect, useState } from "react";

/** Below this container width the metadata drawer auto-hides so the value
 *  viewer keeps a readable column. Users can still toggle it via the
 *  Details button. */
export const DETAILS_AUTO_HIDE_BELOW_PX = 640;

export interface DrawerVisibility {
  open: boolean;
  toggle: () => void;
}

/**
 * Auto-hides the metadata drawer on narrow inspector widths; remembers whether
 * we hid it automatically so we don't fight a user who's explicitly opened it
 * on a wide window and then shrunk it briefly.
 */
export function useDrawerVisibility(containerWidth: number): DrawerVisibility {
  const [open, setOpen] = useState(true);
  const [autoHid, setAutoHid] = useState(false);

  useEffect(() => {
    if (containerWidth === 0) return;
    const narrow = containerWidth < DETAILS_AUTO_HIDE_BELOW_PX;
    if (narrow && open) {
      setOpen(false);
      setAutoHid(true);
      return;
    }
    if (!narrow && autoHid) {
      setOpen(true);
      setAutoHid(false);
    }
  }, [containerWidth, open, autoHid]);

  const toggle = useCallback(() => {
    setOpen((prev) => !prev);
    setAutoHid(false);
  }, []);

  return { open, toggle };
}
