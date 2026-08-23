import { describe, expect, it } from "vitest";

import { GATES, runGates } from "./check-ui-gates.mjs";

const component = (text) => [{ rel: "src/components/example.tsx", text }];
const names = (files) => runGates(files).map((f) => f.split(":")[0]);

describe("hex-color gate", () => {
  const hex = GATES["hex-color"];
  const rel = "src/components/example.tsx";

  it("flags hex colors in value positions", () => {
    expect(hex(rel, 'style={{ color: "#ff0000" }}')).toBe(true);
    expect(hex(rel, 'fill="#fff"')).toBe(true);
    expect(hex(rel, "const accent = '#a1b2c3d4';")).toBe(true);
    expect(hex(rel, "background: #abc;")).toBe(true);
  });

  it("never flags issue references or anchors", () => {
    // The exact false positives that would fail unrelated PRs.
    expect(hex(rel, "// fixes #123")).toBe(false);
    expect(hex(rel, "// see PR #202688 for context")).toBe(false);
    expect(hex(rel, '<a href="#abc">jump</a>')).toBe(false);
    expect(hex(rel, "* Ref: issue #4242 covers the rest.")).toBe(false);
  });

  it("only applies to component TSX", () => {
    expect(hex("src/lib/colors.ts", 'const c = "#ff0000";')).toBe(false);
  });
});

describe("banned-radius gate", () => {
  const radius = GATES["banned-radius"];
  const rel = "src/components/example.tsx";

  it("flags base and per-side/per-corner xl+ radii", () => {
    expect(radius(rel, 'className="rounded-xl"')).toBe(true);
    expect(radius(rel, 'className="rounded-t-2xl"')).toBe(true);
    expect(radius(rel, 'className="rounded-tl-xl"')).toBe(true);
  });

  it("allows the sanctioned radii", () => {
    expect(radius(rel, 'className="rounded-md rounded-t-sm"')).toBe(false);
    expect(radius(rel, 'className="rounded-[calc(var(--radius-sm)-2px)]"'))
      .toBe(false);
  });
});

describe("native-prompt gate", () => {
  const prompt = GATES["native-prompt"];

  it("flags window and globalThis prompts", () => {
    expect(prompt("src/lib/x.ts", "if (window.confirm('sure?')) {")).toBe(
      true,
    );
    expect(prompt("src/lib/x.ts", "globalThis.alert('hi')")).toBe(true);
  });

  it("allows the themed confirm service", () => {
    expect(prompt("src/lib/x.ts", "await requestConfirm({ title })")).toBe(
      false,
    );
  });
});

describe("runGates", () => {
  it("reports name:file:line for hits and skips token files", () => {
    const failures = runGates([
      { rel: "src/components/a.tsx", text: 'x\ncolor: "#fff"\n' },
      { rel: "src/styles.css", text: "color: #fff;" },
    ]);
    expect(failures).toHaveLength(1);
    expect(failures[0]).toContain("hex-color: src/components/a.tsx:2");
  });

  it("passes clean files", () => {
    expect(names(component('className="rounded-md bg-surface-panel"'))).toEqual(
      [],
    );
  });
});
