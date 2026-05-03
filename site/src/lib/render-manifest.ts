// Vanilla-DOM renderer for IonDocManifest. Mirrors the output of
// ManifestBrowser.astro + MemberList.astro so uploaded manifests look
// identical to the build-time stdlib reference. No syntax highlighting
// at runtime — kept deliberately small.

import {
  memberAnchor,
  partitionMembers,
  walkModules,
  type IonDocManifest,
  type ManifestMember,
  type ManifestModule,
} from "./ion-doc";

const escapeHtml = (s: string): string =>
  s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");

const renderInline = (text: string): string =>
  escapeHtml(text).replace(/`([^`]+)`/g, "<code>$1</code>");

const moduleAnchor = (path: string): string =>
  path.replace(/[^a-zA-Z0-9_-]/g, "_");

const codeBlock = (code: string): string =>
  `<pre class="ion-preview-code"><code>${escapeHtml(code)}</code></pre>`;

function memberHtml(m: ManifestMember, level: 4 | 5): string {
  const heading = `h${level}`;
  let html = `<article id="${escapeHtml(memberAnchor(m.name))}">`;
  html += `<${heading}><code>${escapeHtml(m.name)}</code>`;
  if (m.since) {
    html += ` <span class="ion-api-since" title="Available since">since ${escapeHtml(m.since)}</span>`;
  }
  if (m.receiver) {
    html += ` <span class="ion-api-receiver"> on ${escapeHtml(m.receiver)}</span>`;
  }
  html += `</${heading}>`;
  html += codeBlock(m.signature);
  if (m.doc) html += `<p>${renderInline(m.doc)}</p>`;
  if (m.examples && m.examples.length > 0) {
    html += `<details><summary>Examples</summary>`;
    for (const ex of m.examples) html += codeBlock(ex);
    html += `</details>`;
  }
  return html + `</article>`;
}

function memberListHtml(
  title: string,
  members: ManifestMember[],
  sectionLevel: 3 | 4 = 3
): string {
  if (members.length === 0) return "";
  const sh = `h${sectionLevel}`;
  const itemLevel = (sectionLevel + 1) as 4 | 5;
  let html = `<section class="ion-api-section"><${sh}>${escapeHtml(title)}</${sh}>`;
  for (const m of members) html += memberHtml(m, itemLevel);
  return html + `</section>`;
}

function moduleHtml(mod: ManifestModule, pathSegments: string[]): string {
  const path = pathSegments.join("::");
  const partitioned = partitionMembers(mod.members);
  let html = `<section class="ion-preview-module">`;
  html += `<h2 id="m-${escapeHtml(moduleAnchor(path))}"><code>${escapeHtml(path)}</code></h2>`;
  if (mod.summary) html += `<p>${renderInline(mod.summary)}</p>`;
  html += memberListHtml("Builtins", partitioned.builtins);
  html += memberListHtml("Constants", partitioned.constants);
  html += memberListHtml("Functions", partitioned.functions);
  html += memberListHtml("Methods", partitioned.methods);

  if (partitioned.types.length > 0) {
    html += `<section class="ion-api-section"><h3>Types</h3>`;
    for (const t of partitioned.types) {
      html += `<section><h4 id="type-${escapeHtml(memberAnchor(t.name))}"><code>${escapeHtml(t.name)}</code></h4>`;
      if (t.doc) html += `<p>${renderInline(t.doc)}</p>`;
      if (t.variants && t.variants.length > 0) {
        html += `<p>Variants: ${t.variants
          .map((v) => `<code>${escapeHtml(v)}</code>`)
          .join(", ")}</p>`;
      }
      if (t.methods && t.methods.length > 0) {
        html += memberListHtml(`${t.name} methods`, t.methods, 4);
      }
      html += `</section>`;
    }
    html += `</section>`;
  }

  if (mod.modules && mod.modules.length > 0) {
    html += `<section class="ion-api-section"><h3>Submodules</h3><ul>`;
    for (const sub of mod.modules) {
      const subPath = [...pathSegments, sub.name].join("::");
      html += `<li><a href="#m-${escapeHtml(moduleAnchor(subPath))}"><code>${escapeHtml(subPath)}</code></a>`;
      if (sub.summary) html += ` — ${renderInline(sub.summary)}`;
      html += `</li>`;
    }
    html += `</ul></section>`;
  }

  return html + `</section>`;
}

function metaHtml(manifest: IonDocManifest): string {
  const rows: Array<[string, string]> = [];
  if (manifest.profile) rows.push(["Profile", `<code>${escapeHtml(manifest.profile)}</code>`]);
  if (manifest.homepage)
    rows.push(["Homepage", `<a href="${escapeHtml(manifest.homepage)}">${escapeHtml(manifest.homepage)}</a>`]);
  if (manifest.repository)
    rows.push(["Repository", `<a href="${escapeHtml(manifest.repository)}">${escapeHtml(manifest.repository)}</a>`]);
  if (manifest.license) rows.push(["License", escapeHtml(manifest.license)]);
  if (manifest.categories && manifest.categories.length > 0) {
    rows.push([
      "Categories",
      manifest.categories.map((c) => `<code>${escapeHtml(c)}</code>`).join(", "),
    ]);
  }
  rows.push(["Schema version", `<code>${manifest.ionDocVersion}</code>`]);
  if (rows.length === 0) return "";
  return (
    `<dl class="ion-preview-meta">` +
    rows.map(([k, v]) => `<dt>${k}</dt><dd>${v}</dd>`).join("") +
    `</dl>`
  );
}

function tocHtml(manifest: IonDocManifest): string {
  const all = [...walkModules(manifest.modules)];
  if (all.length <= 1) return "";
  const links = all
    .map(({ pathSegments }) => {
      const path = pathSegments.join("::");
      return `<a href="#m-${escapeHtml(moduleAnchor(path))}"><code>${escapeHtml(path)}</code></a>`;
    })
    .join(", ");
  return `<nav class="ion-preview-toc"><strong>Modules:</strong> ${links}</nav>`;
}

export function renderManifest(
  manifest: IonDocManifest,
  target: HTMLElement
): void {
  let html = metaHtml(manifest) + tocHtml(manifest);
  for (const { pathSegments, module } of walkModules(manifest.modules)) {
    html += moduleHtml(module, pathSegments);
  }
  target.innerHTML = html;
}
