#!/usr/bin/env node
// Extract every fenced ```ion code block from the docs (synced and
// canonical) and pipe each through `ion --check`. Failures fail CI.
//
// Run:
//   node site/scripts/check-ion-snippets.mjs <ion-binary>
// Defaults to `target/release/ion`.

import { spawnSync } from "node:child_process";
import { readFileSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve, relative } from "node:path";
import { readdirSync } from "node:fs";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..", "..");
const ionBin =
  process.argv[2] ?? resolve(repoRoot, "target", "release", "ion");

if (!exists(ionBin)) {
  console.error(`check-ion-snippets: missing binary: ${ionBin}`);
  console.error("Build it with: cargo build -p ionlang-cli --release");
  process.exit(2);
}

const roots = [
  "site/src/content/docs",
  "docs",
  "LANGUAGE.md",
  "DESIGN.md",
  "README.md",
];

const files = [];
for (const r of roots) {
  const abs = resolve(repoRoot, r);
  if (!exists(abs)) continue;
  if (statSync(abs).isDirectory()) {
    walk(abs, (p) => {
      if (/\.(md|mdx)$/.test(p)) files.push(p);
    });
  } else {
    files.push(abs);
  }
}

let total = 0;
let failed = 0;
for (const file of files) {
  const text = readFileSync(file, "utf8");
  const blocks = extractIonBlocks(text);
  for (let i = 0; i < blocks.length; i++) {
    total++;
    const block = blocks[i];
    const r = spawnSync(ionBin, ["--check", "-"], {
      input: block.code,
      encoding: "utf8",
    });
    if (r.status !== 0) {
      failed++;
      const rel = relative(repoRoot, file);
      console.error(
        `::error file=${rel},line=${block.line}::ion snippet #${i + 1} failed to parse`
      );
      if (r.stderr) console.error(r.stderr);
      if (r.stdout) console.error(r.stdout);
    }
  }
}

console.log(
  `check-ion-snippets: ${total} snippet(s) across ${files.length} file(s); ${failed} failed`
);
process.exit(failed === 0 ? 0 : 1);

function exists(p) {
  try {
    statSync(p);
    return true;
  } catch {
    return false;
  }
}

function walk(dir, visit) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, visit);
    else visit(p);
  }
}

function extractIonBlocks(src) {
  // Match fenced blocks introduced by ```ion (no info string after, or
  // attribute-only). Captures the body and the 1-based line of the opener.
  const blocks = [];
  const lines = src.split("\n");
  let i = 0;
  while (i < lines.length) {
    const m = /^\s*```ion(\s|$)/.exec(lines[i]);
    if (m) {
      const opener = i + 1;
      i++;
      const buf = [];
      while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) {
        buf.push(lines[i]);
        i++;
      }
      blocks.push({ line: opener, code: buf.join("\n") });
    }
    i++;
  }
  return blocks;
}
