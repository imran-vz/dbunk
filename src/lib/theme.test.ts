// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  applyTheme,
  isThemeMode,
  LOCAL_STORAGE_KEY,
  readStoredMode,
  resolveMode,
  subscribeSystem,
  writeStoredMode,
} from "@/lib/theme";

type MatchMediaResult = {
  matches: boolean;
  addEventListener: ReturnType<typeof vi.fn>;
  removeEventListener: ReturnType<typeof vi.fn>;
};

function mockMatchMedia(matches: boolean): MatchMediaResult {
  const handle: MatchMediaResult = {
    matches,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  };
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: () => handle,
  });
  return handle;
}

describe("resolveMode", () => {
  afterEach(() => {
    document.documentElement.classList.remove("dark");
  });

  it("returns the explicit mode unchanged", () => {
    expect(resolveMode("light")).toBe("light");
    expect(resolveMode("dark")).toBe("dark");
  });

  it("returns 'dark' when system prefers dark", () => {
    mockMatchMedia(true);
    expect(resolveMode("system")).toBe("dark");
  });

  it("returns 'light' when system prefers light", () => {
    mockMatchMedia(false);
    expect(resolveMode("system")).toBe("light");
  });
});

describe("applyTheme", () => {
  afterEach(() => {
    document.documentElement.classList.remove("dark");
  });

  it("adds `.dark` for dark mode", () => {
    applyTheme("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("removes `.dark` for light mode", () => {
    document.documentElement.classList.add("dark");
    applyTheme("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("resolves 'system' via matchMedia", () => {
    mockMatchMedia(true);
    applyTheme("system");
    expect(document.documentElement.classList.contains("dark")).toBe(true);

    mockMatchMedia(false);
    applyTheme("system");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});

describe("subscribeSystem", () => {
  it("registers and unregisters a change listener on the media query", () => {
    const mql = mockMatchMedia(false);
    const handler = vi.fn();
    const unsubscribe = subscribeSystem(handler);
    expect(mql.addEventListener).toHaveBeenCalledWith("change", handler);

    unsubscribe();
    expect(mql.removeEventListener).toHaveBeenCalledWith("change", handler);
  });
});

describe("isThemeMode", () => {
  it("accepts the three valid modes only", () => {
    expect(isThemeMode("system")).toBe(true);
    expect(isThemeMode("light")).toBe(true);
    expect(isThemeMode("dark")).toBe(true);
    expect(isThemeMode("auto")).toBe(false);
    expect(isThemeMode(null)).toBe(false);
    expect(isThemeMode(undefined)).toBe(false);
  });
});

describe("read/writeStoredMode", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("round-trips a valid mode", () => {
    writeStoredMode("light");
    expect(readStoredMode()).toBe("light");
    expect(window.localStorage.getItem(LOCAL_STORAGE_KEY)).toBe("light");
  });

  it("falls back to 'system' when the stored value is missing or invalid", () => {
    expect(readStoredMode()).toBe("system");
    window.localStorage.setItem(LOCAL_STORAGE_KEY, "garbage");
    expect(readStoredMode()).toBe("system");
  });
});
