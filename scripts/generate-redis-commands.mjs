#!/usr/bin/env node
// @ts-check
/**
 * Generate the Rust and TypeScript destructive-command lists from
 * `src-tauri/src/redis/destructive-commands.toml`.
 *
 * Run via `pnpm run generate:redis-commands`. CI verifies the
 * generated files are up to date via `pnpm run check:redis-commands`,
 * which re-runs this script in --check mode.
 *
 * Rust: replaces the block between
 *   `// <generated:destructive-commands>` and
 *   `// </generated:destructive-commands>`
 * inside `destructive_commands.rs`, leaving the helper function +
 * tests untouched.
 *
 * TS: writes the entire `src/lib/redis/destructive-commands.ts` file
 * (it's a leaf module with no helpers — consumers import the constants
 * directly).
 *
 * The TOML shape is intentionally narrow — two top-level array-of-string
 * keys (`hard` and `soft`) — so we hand-parse instead of pulling in a
 * dependency. If the file grows tables/nesting, swap for `@iarna/toml`.
 */

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(__dirname, "..");
const TOML_PATH = join(
  REPO_ROOT,
  "src-tauri/src/redis/destructive-commands.toml",
);
const RUST_PATH = join(
  REPO_ROOT,
  "src-tauri/src/redis/destructive_commands.rs",
);
const TS_PATH = join(REPO_ROOT, "src/lib/redis/destructive-commands.ts");

const RUST_BEGIN = "// <generated:destructive-commands>";
const RUST_END = "// </generated:destructive-commands>";

const checkOnly = process.argv.includes("--check");

/**
 * Parse the constrained TOML shape we use: top-level
 *   `name = [ "ITEM", ... ]`
 * entries with `#`-comment lines. Returns a Map<string, string[]>.
 */
function parseToml(source) {
  const entries = new Map();
  const lines = source.split(/\r?\n/);
  let i = 0;
  while (i < lines.length) {
    const line = lines[i].trim();
    if (line === "" || line.startsWith("#")) {
      i++;
      continue;
    }
    const match = /^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*\[(.*)$/.exec(line);
    if (!match) {
      throw new Error(`Unrecognised line at ${i + 1}: ${lines[i]}`);
    }
    const key = match[1];
    const rest = match[2];
    const collected = [];
    const consume = (segment) => {
      const stripped = segment.replace(/#.*$/, "").trim();
      const matches = stripped.match(/"([^"]*)"/g);
      if (matches) {
        for (const m of matches) collected.push(m.slice(1, -1));
      }
    };
    if (rest.includes("]")) {
      consume(rest.slice(0, rest.indexOf("]")));
      entries.set(key, collected);
      i++;
      continue;
    }
    consume(rest);
    i++;
    while (i < lines.length) {
      const inner = lines[i];
      if (inner.includes("]")) {
        consume(inner.slice(0, inner.indexOf("]")));
        i++;
        break;
      }
      consume(inner);
      i++;
    }
    entries.set(key, collected);
  }
  return entries;
}

const tomlSource = readFileSync(TOML_PATH, "utf8");
const parsed = parseToml(tomlSource);
const hard = parsed.get("hard") ?? [];
const soft = parsed.get("soft") ?? [];
if (hard.length === 0) {
  throw new Error(
    "expected non-empty `hard` array in destructive-commands.toml",
  );
}

// Match rustfmt's behaviour: single-element arrays render on one line,
// multi-element arrays render one-item-per-line with a trailing comma.
function rustArray(items) {
  if (items.length === 0) return "&[]";
  if (items.length === 1) return `&["${items[0]}"]`;
  return `&[\n${items.map((cmd) => `    "${cmd}",`).join("\n")}\n]`;
}

const rustGenerated = `${RUST_BEGIN}
/// Commands that require typed-confirmation before execution. The
/// frontend renders a modal that asks the user to type the command's
/// canonical name to confirm.
pub const DESTRUCTIVE_HARD: &[&str] = ${rustArray(hard)};

/// Commands that are fine in moderation but can be a performance
/// footgun. The frontend renders a softer warning before sending.
pub const DESTRUCTIVE_SOFT: &[&str] = ${rustArray(soft)};
${RUST_END}`;

const existingRust = readFileSync(RUST_PATH, "utf8");
const beginIdx = existingRust.indexOf(RUST_BEGIN);
const endIdx = existingRust.indexOf(RUST_END);
if (beginIdx === -1 || endIdx === -1 || endIdx < beginIdx) {
  throw new Error(
    `Expected sentinel comments ${RUST_BEGIN} and ${RUST_END} in ${RUST_PATH}.`,
  );
}
const newRust =
  existingRust.slice(0, beginIdx) +
  rustGenerated +
  existingRust.slice(endIdx + RUST_END.length);

const HEADER_TS = `// Destructive-command list — generated from
// \`src-tauri/src/redis/destructive-commands.toml\` by
// \`pnpm run generate:redis-commands\`.
//
// DO NOT EDIT BY HAND. The companion Rust constants in
// \`src-tauri/src/redis/destructive_commands.rs\` are generated from
// the same source.
`;

// Match Oxfmt's array formatting: single-element arrays render on one
// line, multi-element arrays render one-per-line with a trailing
// comma.
function tsArray(items) {
  if (items.length === 0) return "[]";
  if (items.length === 1) return `["${items[0]}"]`;
  return `[\n${items.map((cmd) => `  "${cmd}",`).join("\n")}\n]`;
}

const tsBody = `
export const DESTRUCTIVE_HARD = ${tsArray(hard)} as const;

export const DESTRUCTIVE_SOFT = ${tsArray(soft)} as const;
`;

const tsOutput = HEADER_TS + tsBody;

if (checkOnly) {
  const actualTs = readFileSync(TS_PATH, "utf8");
  if (existingRust !== newRust || actualTs !== tsOutput) {
    console.error(
      "destructive-commands generated files are out of date. Run `pnpm run generate:redis-commands`.",
    );
    process.exit(1);
  }
  console.log("destructive-commands generated files are up to date.");
} else {
  writeFileSync(RUST_PATH, newRust);
  writeFileSync(TS_PATH, tsOutput);
  console.log("destructive-commands: wrote", RUST_PATH, "and", TS_PATH);
}
