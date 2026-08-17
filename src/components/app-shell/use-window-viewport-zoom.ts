import {
  type CSSProperties,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { flushSync } from "react-dom";

import type { WindowViewportZoomTransition } from "@/lib/tauri";

export const WINDOW_VIEWPORT_ZOOM_MS = 280;
const WINDOW_VIEWPORT_ZOOM_CLEAR_MS = WINDOW_VIEWPORT_ZOOM_MS + 140;
const SCALE_NEUTRAL_EPSILON = 0.015;

export type WindowViewportZoomState = WindowViewportZoomTransition & {
  id: number;
  active: boolean;
  scaleX: number;
  scaleY: number;
};

export type WindowViewportZoomTransform =
  | { skip: true }
  | { skip: false; scaleX: number; scaleY: number };

/**
 * Pure helper: decide whether a transition should animate, and compute the
 * initial scale. Returns `{ skip: true }` when the transition is missing,
 * shrinks the window, produces non-finite or non-positive scales, or is
 * within the neutral epsilon (effectively no movement).
 */
export function computeViewportZoomTransform(
  transition: WindowViewportZoomTransition | null,
  reducedMotion: boolean,
): WindowViewportZoomTransform {
  if (!transition || reducedMotion) {
    return { skip: true };
  }
  const scaleX = transition.fromWidth / transition.toWidth;
  const scaleY = transition.fromHeight / transition.toHeight;
  if (
    transition.toWidth < transition.fromWidth ||
    transition.toHeight < transition.fromHeight ||
    !Number.isFinite(scaleX) ||
    !Number.isFinite(scaleY) ||
    scaleX <= 0 ||
    scaleY <= 0 ||
    (Math.abs(scaleX - 1) < SCALE_NEUTRAL_EPSILON &&
      Math.abs(scaleY - 1) < SCALE_NEUTRAL_EPSILON)
  ) {
    return { skip: true };
  }
  return { skip: false, scaleX, scaleY };
}

export function windowViewportZoomStyle(
  state: WindowViewportZoomState | null,
): CSSProperties | undefined {
  if (!state) {
    return undefined;
  }
  return {
    width: `${state.toWidth}px`,
    height: `${state.toHeight}px`,
    transform: state.active
      ? "scale(1)"
      : `scale(${state.scaleX}, ${state.scaleY})`,
    transitionDuration: `${WINDOW_VIEWPORT_ZOOM_MS}ms`,
  };
}

export function prefersReducedMotion(): boolean {
  if (typeof window === "undefined") {
    return false;
  }
  return Boolean(
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches,
  );
}

export interface UseWindowViewportZoom {
  state: WindowViewportZoomState | null;
  start: (transition: WindowViewportZoomTransition | null) => boolean;
}

/**
 * Hook owning the window viewport zoom animation state machine. Driven by
 * the native window zoom: we briefly render the shell at the *target* size
 * but visually scale it back to the *source* size, then animate to 1.0 so
 * the user sees a continuous zoom even though Tauri snaps the OS window.
 */
export function useWindowViewportZoom(): UseWindowViewportZoom {
  const [state, setState] = useState<WindowViewportZoomState | null>(null);
  const idRef = useRef(0);
  const timeoutRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (timeoutRef.current) {
        window.clearTimeout(timeoutRef.current);
      }
    };
  }, []);

  const start = useCallback(
    (transition: WindowViewportZoomTransition | null) => {
      const transform = computeViewportZoomTransform(
        transition,
        prefersReducedMotion(),
      );
      if (transform.skip || !transition) {
        return false;
      }

      const id = idRef.current + 1;
      idRef.current = id;
      if (timeoutRef.current) {
        window.clearTimeout(timeoutRef.current);
      }

      flushSync(() => {
        setState({
          ...transition,
          id,
          active: false,
          scaleX: transform.scaleX,
          scaleY: transform.scaleY,
        });
      });

      window.requestAnimationFrame(() => {
        setState((current) =>
          current?.id === id ? { ...current, active: true } : current,
        );
      });
      timeoutRef.current = window.setTimeout(() => {
        setState((current) => (current?.id === id ? null : current));
      }, WINDOW_VIEWPORT_ZOOM_CLEAR_MS);
      return true;
    },
    [],
  );

  return { state, start };
}
