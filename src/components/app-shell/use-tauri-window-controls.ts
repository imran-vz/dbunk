import {
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  tauriOnWindowFullscreenChange,
  tauriPrepareWindowZoomTransition,
  tauriRestoreWindowTrafficLightPosition,
  tauriStartDragging,
  tauriToggleWindowZoom,
} from "@/lib/tauri";
import {
  useWindowViewportZoom,
  type WindowViewportZoomState,
} from "./use-window-viewport-zoom";
import { shouldStartTopBarDrag } from "./window-drag";

const WINDOW_TRAFFIC_LIGHT_RESTORE_DELAYS_MS = [120, 340] as const;

function nextAnimationFrame() {
  return new Promise<void>((resolve) => {
    if (typeof window === "undefined") {
      resolve();
      return;
    }
    window.requestAnimationFrame(() => resolve());
  });
}

export interface TauriWindowControls {
  isWindowFullscreen: boolean;
  windowViewportZoom: WindowViewportZoomState | null;
  onTopBarPointerDown: (event: ReactPointerEvent<HTMLElement>) => void;
  onTopBarDoubleClick: (event: ReactMouseEvent<HTMLElement>) => void;
}

/**
 * Hook owning the Tauri-window integration: drag start, native zoom
 * orchestration (paired with the JS viewport animation), fullscreen
 * tracking, and the macOS traffic-light restore dance after exiting
 * fullscreen.
 */
export function useTauriWindowControls(): TauriWindowControls {
  const [isWindowFullscreen, setIsWindowFullscreen] = useState(false);
  const wasWindowFullscreen = useRef(false);
  const trafficLightRestoreTimeouts = useRef<number[]>([]);
  const viewportZoom = useWindowViewportZoom();

  const clearTrafficLightRestoreTimers = useCallback(() => {
    for (const timeout of trafficLightRestoreTimeouts.current) {
      window.clearTimeout(timeout);
    }
    trafficLightRestoreTimeouts.current = [];
  }, []);

  const restoreTrafficLightPosition = useCallback(() => {
    clearTrafficLightRestoreTimers();
    void tauriRestoreWindowTrafficLightPosition().catch(() => undefined);
    for (const delay of WINDOW_TRAFFIC_LIGHT_RESTORE_DELAYS_MS) {
      const timeout = window.setTimeout(() => {
        trafficLightRestoreTimeouts.current =
          trafficLightRestoreTimeouts.current.filter((id) => id !== timeout);
        void tauriRestoreWindowTrafficLightPosition().catch(() => undefined);
      }, delay);
      trafficLightRestoreTimeouts.current.push(timeout);
    }
  }, [clearTrafficLightRestoreTimers]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void tauriOnWindowFullscreenChange((fullscreen) => {
      if (disposed) return;
      const didExitFullscreen = wasWindowFullscreen.current && !fullscreen;
      wasWindowFullscreen.current = fullscreen;
      setIsWindowFullscreen(fullscreen);
      if (didExitFullscreen) {
        restoreTrafficLightPosition();
      } else if (fullscreen) {
        clearTrafficLightRestoreTimers();
      }
    })
      .then((unsubscribe) => {
        if (disposed) {
          unsubscribe();
          return;
        }
        unlisten = unsubscribe;
      })
      .catch(() => undefined);

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [clearTrafficLightRestoreTimers, restoreTrafficLightPosition]);

  useEffect(() => {
    return () => {
      clearTrafficLightRestoreTimers();
    };
  }, [clearTrafficLightRestoreTimers]);

  const startViewportZoom = viewportZoom.start;
  const handleNativeWindowZoom = useCallback(async () => {
    const transition = await tauriPrepareWindowZoomTransition();
    const willAnimateViewport = startViewportZoom(transition);
    if (willAnimateViewport) {
      await nextAnimationFrame();
    }
    await tauriToggleWindowZoom();
  }, [startViewportZoom]);

  const onTopBarPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLElement>) => {
      if (event.detail > 1 || !shouldStartTopBarDrag(event)) {
        return;
      }
      event.preventDefault();
      void tauriStartDragging().catch(() => undefined);
    },
    [],
  );

  const onTopBarDoubleClick = useCallback(
    (event: ReactMouseEvent<HTMLElement>) => {
      if (!shouldStartTopBarDrag(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      void handleNativeWindowZoom().catch(() => undefined);
    },
    [handleNativeWindowZoom],
  );

  return {
    isWindowFullscreen,
    windowViewportZoom: viewportZoom.state,
    onTopBarPointerDown,
    onTopBarDoubleClick,
  };
}
