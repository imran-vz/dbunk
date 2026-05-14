// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  applyTheme,
  isPresetIntrinsicallyDark,
  isThemeMode,
  isThemePreset,
  LOCAL_STORAGE_KEY,
  LOCAL_STORAGE_PRESET_KEY,
  readStoredMode,
  readStoredPreset,
  resolveMode,
  subscribeSystem,
  writeStoredMode,
  writeStoredPreset,
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
    document.documentElement.removeAttribute("data-theme");
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

  it("omits the data-theme attribute when preset is 'default'", () => {
    applyTheme("light", "default");
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
  });

  it("sets data-theme for non-default presets", () => {
    applyTheme("light", "github");
    expect(document.documentElement.getAttribute("data-theme")).toBe("github");
    applyTheme("dark", "gruvbox");
    expect(document.documentElement.getAttribute("data-theme")).toBe("gruvbox");
  });

  it("forces `.dark` for intrinsically-dark presets regardless of mode", () => {
    mockMatchMedia(false);
    applyTheme("light", "dracula");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(document.documentElement.getAttribute("data-theme")).toBe("dracula");
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

describe("isThemeMode / isThemePreset / isPresetIntrinsicallyDark", () => {
  it("accepts only the valid modes", () => {
    expect(isThemeMode("system")).toBe(true);
    expect(isThemeMode("light")).toBe(true);
    expect(isThemeMode("dark")).toBe(true);
    expect(isThemeMode("auto")).toBe(false);
    expect(isThemeMode(null)).toBe(false);
    expect(isThemeMode(undefined)).toBe(false);
  });

  it("accepts only the valid presets", () => {
    expect(isThemePreset("default")).toBe(true);
    expect(isThemePreset("dracula")).toBe(true);
    expect(isThemePreset("github")).toBe(true);
    expect(isThemePreset("gruvbox")).toBe(true);
    expect(isThemePreset("monokai")).toBe(false);
    expect(isThemePreset(null)).toBe(false);
  });

  it("flags Dracula as intrinsically dark and others as not", () => {
    expect(isPresetIntrinsicallyDark("dracula")).toBe(true);
    expect(isPresetIntrinsicallyDark("default")).toBe(false);
    expect(isPresetIntrinsicallyDark("github")).toBe(false);
    expect(isPresetIntrinsicallyDark("gruvbox")).toBe(false);
  });
});

describe("read/writeStoredMode + Preset", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("round-trips a valid mode", () => {
    writeStoredMode("light");
    expect(readStoredMode()).toBe("light");
    expect(window.localStorage.getItem(LOCAL_STORAGE_KEY)).toBe("light");
  });

  it("round-trips a valid preset", () => {
    writeStoredPreset("dracula");
    expect(readStoredPreset()).toBe("dracula");
    expect(window.localStorage.getItem(LOCAL_STORAGE_PRESET_KEY)).toBe(
      "dracula",
    );
  });

  it("falls back to 'system' when the stored mode is missing or invalid", () => {
    expect(readStoredMode()).toBe("system");
    window.localStorage.setItem(LOCAL_STORAGE_KEY, "garbage");
    expect(readStoredMode()).toBe("system");
  });

  it("falls back to 'default' when the stored preset is missing or invalid", () => {
    expect(readStoredPreset()).toBe("default");
    window.localStorage.setItem(LOCAL_STORAGE_PRESET_KEY, "monokai");
    expect(readStoredPreset()).toBe("default");
  });
});
