#!/usr/bin/env node
// Copy canonical content into the site tree at build time:
//   * doc manifests (consumed by Astro pages and served at a stable URL)
//   * root markdown (LANGUAGE.md, DESIGN.md, CHANGELOG.md) — GitHub
//     remains the entry point for repo browsers; the site renders the
//     same files without duplicating them in git.
//
// Always-overwriting, idempotent. Files written here are gitignored.

import { mkdir, copyFile, readFile, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

function escapeBracesOutsideFences(src) {
  // Walk the source line by line; flip a `inFence` flag on ``` markers.
  // Inside fences and inside inline `code`, leave content alone.
  const out = [];
  let inFence = false;
  for (const line of src.split("\n")) {
    if (/^\s*```/.test(line)) {
      inFence = !inFence;
      out.push(line);
      continue;
    }
    if (inFence) {
      out.push(line);
      continue;
    }
    // Within a non-fence line: protect inline `code` spans, then escape
    // braces in the rest.
    let result = "";
    let i = 0;
    while (i < line.length) {
      if (line[i] === "`") {
        const end = line.indexOf("`", i + 1);
        if (end === -1) {
          result += line.slice(i);
          break;
        }
        result += line.slice(i, end + 1);
        i = end + 1;
      } else if (line[i] === "{" || line[i] === "}") {
        result += "\\" + line[i];
        i += 1;
      } else if (
        line[i] === "<" &&
        // Don't escape valid HTML tag starts (`<div`, `</p>`, `<!--`).
        !/[A-Za-z!\/]/.test(line[i + 1] ?? "")
      ) {
        result += "&lt;";
        i += 1;
      } else {
        result += line[i];
        i += 1;
      }
    }
    out.push(result);
  }
  return out.join("\n");
}

const here = dirname(fileURLToPath(import.meta.url));
// `repoRoot` is the ionrs source repo (canonical content). `siteRoot` is
// where the docs site lives. They're the same when site/ is nested inside
// ionrs/; they differ when the site lives in a dedicated repo.
const siteRoot = resolve(here, "..");
const repoRoot = process.env.IONRS_REPO
  ? resolve(process.env.IONRS_REPO)
  : resolve(here, "..", "..");
const siteContent = resolve(siteRoot, "src", "content", "docs");

await mkdir(resolve(siteRoot, "public", "manifests"), {
  recursive: true,
});
await mkdir(resolve(siteContent, "language"), { recursive: true });
await mkdir(resolve(siteContent, "design"), { recursive: true });
await mkdir(resolve(siteContent, "guides"), { recursive: true });
await mkdir(resolve(siteContent, "examples"), { recursive: true });

// 1. Stdlib doc manifest (single source of truth shared with the LSP).
{
  const from = resolve(repoRoot, "ion-core", "src", "stdlib-docs.json");
  const to = resolve(siteRoot, "public", "manifests", "stdlib.json");
  if (!existsSync(from)) {
    console.error(`sync: missing ${from}`);
    process.exit(1);
  }
  await copyFile(from, to);
  console.log(`sync: ${from} -> ${to}`);
}

// 2. Root markdown into Starlight content collection. Each gets a small
// frontmatter header and the source body unchanged.
const rootMarkdown = [
  {
    src: "LANGUAGE.md",
    dest: resolve(siteContent, "language", "index.mdx"),
    frontmatter: {
      title: "Language Reference",
      description:
        "Complete syntax and semantics of the Ion scripting language.",
      sidebarOrder: 1,
      stripH1: true,
    },
  },
  {
    src: "DESIGN.md",
    dest: resolve(siteContent, "design", "index.mdx"),
    frontmatter: {
      title: "Design",
      description:
        "Design principles, decisions, and rationale behind Ion's syntax and runtime.",
      sidebarOrder: 1,
      stripH1: true,
    },
  },
  {
    src: "CHANGELOG.md",
    dest: resolve(siteContent, "changelog.mdx"),
    frontmatter: {
      title: "Changelog",
      description:
        "Release notes for Ion crates and editor extensions.",
      stripH1: true,
    },
  },
];

const docsToGuides = [
  ["concurrency.md", "Concurrency"],
  ["embedding.md", "Embedding"],
  ["performance.md", "Performance"],
  ["testing.md", "Testing"],
  ["tooling.md", "Tooling"],
  ["vm-internals.md", "VM internals"],
];
for (const [filename, title] of docsToGuides) {
  rootMarkdown.push({
    src: `docs/${filename}`,
    dest: resolve(siteContent, "guides", filename.replace(/\.md$/, ".mdx")),
    frontmatter: { title, stripH1: true },
  });
}

for (const job of rootMarkdown) {
  const from = resolve(repoRoot, job.src);
  if (!existsSync(from)) {
    console.error(`sync: missing ${from}`);
    process.exit(1);
  }
  let body = await readFile(from, "utf8");
  if (job.frontmatter.stripH1) {
    // Drop the leading H1 + any blank line — Starlight renders the title
    // from frontmatter, and a duplicate H1 confuses the TOC.
    body = body.replace(/^#\s+[^\n]*\n+/, "");
  }
  // Drop any "## Table of Contents" + its bullet block (Starlight has
  // a built-in right-sidebar TOC).
  body = body.replace(
    /^##\s+Table of Contents\s*\n([-*]\s+\[[^\n]*\n)+/m,
    ""
  );

  // Normalise rustdoc-style fence attributes (` ```rust,no_run `) to a
  // plain language ID so Shiki resolves them. Fence languages are tokens
  // not comma-separated tags; the rustdoc dialect doesn't apply here.
  body = body.replace(/^```rust,[\w,]+/gm, "```rust");

  // Astro 5's content layer pipes `.md` through the MDX processor, so
  // bare `{` and `}` in prose are interpreted as JSX expression
  // delimiters. Escape them outside fenced code blocks. Backslash-
  // escapes survive Markdown rendering as literal `{` / `}`.
  body = escapeBracesOutsideFences(body);

  const fm = ["---", `title: ${JSON.stringify(job.frontmatter.title)}`];
  if (job.frontmatter.description) {
    fm.push(`description: ${JSON.stringify(job.frontmatter.description)}`);
  }
  if (job.frontmatter.sidebarOrder) {
    fm.push("sidebar:");
    fm.push(`  order: ${job.frontmatter.sidebarOrder}`);
  }
  fm.push("---");
  fm.push("");

  await writeFile(job.dest, fm.join("\n") + "\n" + body);
  console.log(`sync: ${from} -> ${job.dest}`);
}

// 3. Examples: each .ion file gets a per-example MDX page with the
// source embedded in a code block. Starlight auto-generates sidebar
// entries from the directory.
const examplesDir = resolve(repoRoot, "examples");
const exampleFiles = (await import("node:fs")).readdirSync(examplesDir);
for (const file of exampleFiles) {
  if (!file.endsWith(".ion")) continue;
  const slug = file.replace(/\.ion$/, "");
  const title = slug.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
  const source = await readFile(resolve(examplesDir, file), "utf8");
  const dest = resolve(siteContent, "examples", `${slug}.mdx`);
  const body = [
    "---",
    `title: ${JSON.stringify(title)}`,
    `description: ${JSON.stringify(`Example: ${title}`)}`,
    "---",
    "",
    `Source: \`examples/${file}\` in the repo.`,
    "",
    "```ion",
    source.replace(/\n+$/, ""),
    "```",
    "",
  ].join("\n");
  await writeFile(dest, body);
  console.log(`sync: ${file} -> ${dest}`);
}

console.log("sync: complete");
