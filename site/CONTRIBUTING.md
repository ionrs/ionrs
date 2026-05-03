# Contributing to the Ion docs site

The site is an [Astro] + [Starlight] static build. Most contributors will
be touching Markdown content; the manifest browser and a few Astro
components are the only non-prose pieces.

## Local development

```bash
cd site
npm install
npm run dev
```

Open <http://localhost:4321/ion-lang/>. The dev server hot-reloads on
file changes.

`npm run build` produces a static site in `site/dist/`. `npm run preview`
serves the built output for a final check.

Both `dev` and `build` first run `npm run sync-manifests`, which copies
canonical content into the site:

- `ion-core/src/stdlib-docs.json` → `site/public/manifests/stdlib.json`
- `LANGUAGE.md` → `site/src/content/docs/language/index.mdx`
- `DESIGN.md` → `site/src/content/docs/design/index.mdx`
- `CHANGELOG.md` → `site/src/content/docs/changelog.mdx`
- `docs/<topic>.md` → `site/src/content/docs/guides/<topic>.mdx`
- `examples/<name>.ion` → `site/src/content/docs/examples/<name>.mdx`

Those synced files are gitignored. **Edit the canonical sources at the
repo root, not the synced copies** — your changes will be overwritten
otherwise.

## Where to put a new page

| Adding…                          | Where                                             |
|----------------------------------|---------------------------------------------------|
| Long-form prose for users        | `docs/<topic>.md` at repo root (synced to `/guides/<topic>/`) |
| New language feature reference   | Append to `LANGUAGE.md`                           |
| Example program                  | `examples/<name>.ion` (auto-renders as a docs page) |
| New stdlib function/module       | Add to `ion-core/src/stdlib-docs.json` (then run `cargo test -p ion-core`) — the LSP and the docs site both pick it up |
| Site-only page (announcements, FAQs) | New `.mdx` under `site/src/content/docs/` (commit it; it isn't gitignored) |
| New top-level section            | Add to `sidebar` in `site/astro.config.mjs`       |

## The manifest browser

`site/src/lib/manifest.ts` defines the `IonDocManifest` v2 types and a
walker. `site/src/components/{ManifestBrowser,MemberList}.astro` render
a single module. The dynamic route at
`site/src/pages/reference/[...slug].astro` enumerates every module path
in the stdlib manifest at build time.

To add a new manifest (e.g. a third-party Ion package, once the
external-docs pipeline lands in PR 2): drop a `*.json` file in
`site/public/manifests/` and add a route or page that imports it. The
schema is identical to the stdlib manifest.

## Why prose lives in `.mdx`, not `.md`

Astro 5 routes both `.md` and `.mdx` through the MDX processor in
content collections, so `{` / `}` / `<…>` in prose are interpreted as
JSX. The sync script escapes braces and `<` (outside fenced code blocks
and inline `code` spans) when copying canonical sources, but if you
write content directly under `site/src/content/docs/`, escape with
`\{`, `\}`, or `&lt;` yourself.

## Code samples

Use ` ```ion ` for Ion code. Highlighting comes from the VS Code
extension's TextMate grammar (`editors/vscode/syntaxes/ion.tmLanguage.json`)
loaded into Shiki via `astro.config.mjs`.

CI runs `node site/scripts/check-ion-snippets.mjs` against every
` ```ion ` block on every PR. If your snippet doesn't parse with
`ion --check`, the build fails. To check locally:

```bash
cargo build -p ionlang-cli --release
node site/scripts/check-ion-snippets.mjs
```

## Debugging a build failure

- **Vite resolution error mentioning `astro:content-layer-deferred-module`**:
  stale Astro cache. `rm -rf site/.astro site/dist` and rerun.
- **MDX brace error in a synced file**: the sync script's escape pass
  missed a pattern. Add a regression test snippet to the canonical
  source and adjust `escapeBracesOutsideFences` in
  `site/scripts/sync-manifests.mjs`.
- **`<h2>On this page</h2>` is empty**: the page has only one `##`
  heading. Starlight hides the TOC when there's only one entry. Add a
  second `##` or set `tableOfContents: false` in frontmatter.
- **Pagefind search returns nothing**: rebuild — Pagefind indexes from
  `dist/`, not from sources. `npm run build` regenerates it.

## Deployment

`.github/workflows/site.yml` builds, link-checks, snippet-checks, and
deploys to GitHub Pages on every push to `main`. PRs run all but the
deploy step.

[Astro]: https://astro.build
[Starlight]: https://starlight.astro.build
