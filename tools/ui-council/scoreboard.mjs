#!/usr/bin/env node
// Builds per-round score tables from a council workspace.
//
//   node scoreboard.mjs <council dir> [--app "My App"]
//
// Reads <council>/round-<N>/scores-<reviewer>.json, each shaped like
//   { "reviewer": "reviewer-a", "round": 2, "overall": { "My App": 8.5, "Ref A": 9.0, ... } }
// and prints one Markdown table per reviewer plus the average:
//   | App | Round 0 | Round 1 | ... |
// The app named by --app (default: the first app in the first file) is listed first.
import fs from "node:fs";
import path from "node:path";

const argv = process.argv.slice(2);
const appFlagIndex = argv.indexOf("--app");
const dir = argv.find((a, i) => !a.startsWith("--") && i !== appFlagIndex + 1);
if (!dir) {
  console.error("usage: node scoreboard.mjs <council dir> [--app <name>]");
  process.exit(1);
}
let focus = appFlagIndex >= 0 ? argv[appFlagIndex + 1] : undefined;

const byReviewer = new Map(); // reviewer -> Map(round -> overall)
const rounds = new Set();
const apps = [];
for (const entry of fs.readdirSync(dir)) {
  const m = /^round-(\d+)$/.exec(entry);
  if (!m) continue;
  const roundDir = path.join(dir, entry);
  for (const file of fs.readdirSync(roundDir)) {
    const s = /^scores-(.+)\.json$/.exec(file);
    if (!s) continue;
    const data = JSON.parse(fs.readFileSync(path.join(roundDir, file), "utf8"));
    const reviewer = data.reviewer ?? s[1];
    const round = Number(data.round ?? m[1]);
    rounds.add(round);
    if (!byReviewer.has(reviewer)) byReviewer.set(reviewer, new Map());
    byReviewer.get(reviewer).set(round, data.overall ?? {});
    for (const app of Object.keys(data.overall ?? {})) {
      if (!apps.includes(app)) apps.push(app);
    }
  }
}
if (!byReviewer.size) {
  console.error(`No round-*/scores-*.json files under ${dir}`);
  process.exit(1);
}
focus ??= apps[0];
const ordered = [focus, ...apps.filter((a) => a !== focus)];
const roundList = [...rounds].sort((a, b) => a - b);

const fmt = (n, digits) => (n === undefined ? "–" : n.toFixed(digits));
function table(title, get, digits) {
  const head = `| App | ${roundList.map((r) => `Round ${r}`).join(" | ")} |`;
  const sep = `|---|${roundList.map(() => "---:").join("|")}|`;
  const rows = ordered.map((app) => `| ${app} | ${roundList.map((r) => fmt(get(app, r), digits)).join(" | ")} |`);
  return [`**${title}**`, "", head, sep, ...rows, ""].join("\n");
}

const out = [];
for (const [reviewer, rs] of byReviewer) {
  out.push(table(reviewer, (app, r) => rs.get(r)?.[app], 1));
}
if (byReviewer.size > 1) {
  out.push(
    table(
      "Average",
      (app, r) => {
        const vals = [...byReviewer.values()].map((rs) => rs.get(r)?.[app]).filter((v) => typeof v === "number");
        return vals.length === byReviewer.size ? vals.reduce((a, b) => a + b, 0) / vals.length : undefined;
      },
      2,
    ),
  );
}
console.log(out.join("\n"));
