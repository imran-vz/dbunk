#!/usr/bin/env node
/**
 * UI consistency gates (UI_REFRESH_PLAN.md §3, DESIGN-SYSTEM.md).
 * Run via `pnpm run check:ui-gates` (wired into CI).
 *
 * Enabled gates:
 *   1. No arbitrary text sizes  — `text-[` forbidden in src TSX.
 *   2. No banned radii          — `rounded-(xl|2xl|3xl|4xl)` forbidden.
 *   3. No native prompts        — `window.confirm/alert/prompt(` forbidden
 *                                 (the themed confirm service replaces them).
 *   4. No raw colors            — Tailwind palette literals and hex colors
 *                                 forbidden in components (tokens only).
 *                                 Allowed: the `bg-black/60` dialog scrim
 *                                 (the ui primitives' own convention) and
 *                                 CSS keyword colors.
 *   5. No retired primitives    — imports of components deleted by the
 *                                 refresh fail the build conversation, not
 *                                 just typecheck.
 *
 * Documented as pending (see UI_REFRESH_PLAN.md P9/P10 notes):
 *   - `title=` ban: the virtualized grid keeps native titles for
 *     read-only-reason/dirty cells (per-cell Tooltip components are too
 *     heavy at 100k-cell scale); app toolbars still carry ~90 uses.
 *   - raw `<button>`/`<select>` ban: remaining raw elements are dense
 *     list/tree/grid rows where the Button primitive's chrome is wrong;
 *     needs a scoped rule or an unstyled row-button primitive first.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;

/** Files never scanned: token/theme sources and generated output. */
const TOKEN_FILES = new Set(["src/styles.css", "src/lib/theme.ts"]);

const collect = (dir, out = []) => {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    const stats = statSync(path);
    if (stats.isDirectory()) collect(path, out);
    else if (/\.(tsx?|css)$/.test(entry)) out.push(path);
  }
  return out;
};

const files = collect(join(ROOT, "src")).map((path) => ({
  rel: path.slice(ROOT.length),
  text: readFileSync(path, "utf8"),
}));

const failures = [];
const check = (name, predicate) => {
  for (const file of files) {
    if (TOKEN_FILES.has(file.rel)) continue;
    const lines = file.text.split("\n");
    lines.forEach((line, index) => {
      const hit = predicate(file.rel, line);
      if (hit) {
        failures.push(
          `${name}: ${file.rel}:${index + 1}: ${line.trim().slice(0, 120)}`,
        );
      }
    });
  }
};

const isTsx = (rel) => rel.endsWith(".tsx");
const inComponents = (rel) => rel.startsWith("src/components/");

// 1. No arbitrary text sizes.
check(
  "arbitrary-text-size",
  (rel, line) => isTsx(rel) && /\btext-\[/.test(line),
);

// 2. No banned radii.
check(
  "banned-radius",
  (rel, line) => isTsx(rel) && /\brounded-(xl|2xl|3xl|4xl)\b/.test(line),
);

// 3. No native prompts.
check("native-prompt", (_rel, line) =>
  /window\.(confirm|alert|prompt)\s*\(/.test(line),
);

// 4a. No Tailwind palette literals in components.
const PALETTE =
  /\b(bg|text|border|divide|ring|from|to|via|fill|stroke)-(red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose|slate|gray|zinc|neutral|stone)-\d/;
check(
  "palette-literal",
  (rel, line) => inComponents(rel) && PALETTE.test(line),
);

// 4b. No bg-white/bg-black surfaces in components (dialog scrim allowed).
check("raw-surface-color", (rel, line) => {
  if (!inComponents(rel)) return false;
  const stripped = line.replaceAll("bg-black/60", "");
  return (
    /\bbg-(white|black)\b/.test(stripped) ||
    /\bbg-(white|black)\//.test(stripped)
  );
});

// 4c. No hex colors in component TSX.
check("hex-color", (rel, line) => {
  if (!inComponents(rel) || !isTsx(rel)) return false;
  // Ignore ids/anchors and non-color hashes: require a color-position hint.
  return /(#[0-9a-fA-F]{3}\b|#[0-9a-fA-F]{6}\b|#[0-9a-fA-F]{8}\b)/.test(line);
});

// 5. No retired primitives.
check("retired-primitive", (_rel, line) =>
  /\b(ResponsiveEdgePanel|WorkbenchDock)\b|use-resizable-width|use-pubsub-sidebar/.test(
    line,
  ),
);

if (failures.length > 0) {
  console.error(`UI gates failed (${failures.length}):`);
  for (const failure of failures) console.error(`  ${failure}`);
  process.exit(1);
}
console.log(`UI gates passed across ${files.length} files.`);
