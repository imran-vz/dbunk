/* oxlint-disable anti-slop/no-chained-type-assertions anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  computeViewportZoomTransform,
  prefersReducedMotion,
  useWindowViewportZoom,
  WINDOW_VIEWPORT_ZOOM_MS,
  windowViewportZoomStyle,
} from "./use-window-viewport-zoom";

describe("computeViewportZoomTransform", () => {
  it("skips when transition is null", () => {
    expect(computeViewportZoomTransform(null, false)).toEqual({ skip: true });
  });

  it("skips when the user prefers reduced motion", () => {
    expect(
      computeViewportZoomTransform(
        { fromWidth: 800, fromHeight: 600, toWidth: 1440, toHeight: 875 },
        true,
      ),
    ).toEqual({ skip: true });
  });

  it("skips when the window is shrinking (zoom-out / restore)", () => {
    expect(
      computeViewportZoomTransform(
        { fromWidth: 1440, fromHeight: 875, toWidth: 800, toHeight: 600 },
        false,
      ),
    ).toEqual({ skip: true });
  });

  it("skips when one axis shrinks even if the other grows", () => {
    expect(
      computeViewportZoomTransform(
        { fromWidth: 800, fromHeight: 900, toWidth: 1200, toHeight: 600 },
        false,
      ),
    ).toEqual({ skip: true });
  });

  it("skips when target width is zero (non-finite scale)", () => {
    expect(
      computeViewportZoomTransform(
        { fromWidth: 800, fromHeight: 600, toWidth: 0, toHeight: 875 },
        false,
      ),
    ).toEqual({ skip: true });
  });

  it("skips when source width is zero (non-positive scale)", () => {
    expect(
      computeViewportZoomTransform(
        { fromWidth: 0, fromHeight: 600, toWidth: 1440, toHeight: 875 },
        false,
      ),
    ).toEqual({ skip: true });
  });

  it("skips when the scale change is below the neutral epsilon", () => {
    expect(
      computeViewportZoomTransform(
        { fromWidth: 999, fromHeight: 999, toWidth: 1000, toHeight: 1000 },
        false,
      ),
    ).toEqual({ skip: true });
  });

  it("returns the scale pair when the window is growing", () => {
    expect(
      computeViewportZoomTransform(
        { fromWidth: 900, fromHeight: 650, toWidth: 1440, toHeight: 875 },
        false,
      ),
    ).toEqual({
      skip: false,
      scaleX: 900 / 1440,
      scaleY: 650 / 875,
    });
  });
});

describe("windowViewportZoomStyle", () => {
  it("returns undefined when there is no state", () => {
    expect(windowViewportZoomStyle(null)).toBeUndefined();
  });

  it("emits the source-size transform while idle", () => {
    expect(
      windowViewportZoomStyle({
        id: 1,
        active: false,
        scaleX: 0.5,
        scaleY: 0.7,
        fromWidth: 720,
        fromHeight: 610,
        toWidth: 1440,
        toHeight: 875,
      }),
    ).toEqual({
      width: "1440px",
      height: "875px",
      transform: "scale(0.5, 0.7)",
      transitionDuration: `${WINDOW_VIEWPORT_ZOOM_MS}ms`,
    });
  });

  it("emits a unit transform once the animation is active", () => {
    expect(
      windowViewportZoomStyle({
        id: 1,
        active: true,
        scaleX: 0.5,
        scaleY: 0.7,
        fromWidth: 720,
        fromHeight: 610,
        toWidth: 1440,
        toHeight: 875,
      })?.transform,
    ).toBe("scale(1)");
  });
});

describe("prefersReducedMotion", () => {
  const originalMatchMedia = window.matchMedia;

  afterEach(() => {
    window.matchMedia = originalMatchMedia;
  });

  it("returns false when matchMedia is missing", () => {
    // @ts-expect-error — intentionally removing for the test
    window.matchMedia = undefined;
    expect(prefersReducedMotion()).toBe(false);
  });

  it("reflects the underlying media query", () => {
    window.matchMedia = vi.fn().mockReturnValue({
      matches: true,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }) as unknown as typeof window.matchMedia;
    expect(prefersReducedMotion()).toBe(true);
  });
});

describe("useWindowViewportZoom", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns false and leaves state alone when the transition is skipped", () => {
    const { result } = renderHook(() => useWindowViewportZoom());
    let started: boolean | undefined;
    act(() => {
      started = result.current.start(null);
    });
    expect(started).toBe(false);
    expect(result.current.state).toBeNull();
  });

  it("drives the active flag on the next animation frame and clears after the timeout", () => {
    const { result } = renderHook(() => useWindowViewportZoom());

    let started: boolean | undefined;
    act(() => {
      started = result.current.start({
        fromWidth: 900,
        fromHeight: 650,
        toWidth: 1440,
        toHeight: 875,
      });
    });

    expect(started).toBe(true);
    expect(result.current.state).not.toBeNull();
    expect(result.current.state?.active).toBe(false);
    expect(result.current.state?.scaleX).toBeCloseTo(900 / 1440);

    act(() => {
      vi.advanceTimersByTime(16);
    });
    expect(result.current.state?.active).toBe(true);

    act(() => {
      vi.advanceTimersByTime(WINDOW_VIEWPORT_ZOOM_MS + 200);
    });
    expect(result.current.state).toBeNull();
  });

  it("supersedes an in-flight animation when start is called again", () => {
    const { result } = renderHook(() => useWindowViewportZoom());

    act(() => {
      result.current.start({
        fromWidth: 900,
        fromHeight: 650,
        toWidth: 1440,
        toHeight: 875,
      });
    });
    const firstId = result.current.state?.id;

    act(() => {
      result.current.start({
        fromWidth: 1000,
        fromHeight: 700,
        toWidth: 1600,
        toHeight: 1000,
      });
    });
    expect(result.current.state?.id).not.toBe(firstId);
  });
});
