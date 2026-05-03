import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import sitemap from "@astrojs/sitemap";
import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const ionGrammarRaw = JSON.parse(
  readFileSync(
    resolve(here, "..", "editors", "vscode", "syntaxes", "ion.tmLanguage.json"),
    "utf8"
  )
);
// Shiki keys grammars by `name` (the language identifier); the VS Code
// grammar's `name` is the display string "Ion" — override to "ion" so
// markdown ` ```ion ` blocks resolve.
const ionGrammar = { ...ionGrammarRaw, name: "ion" };

const gitDescribe = (() => {
  try {
    return execSync("git describe --tags --always --dirty", {
      encoding: "utf8",
    }).trim();
  } catch {
    return "unknown";
  }
})();

// Public docs site — built here and pushed to `ionrs/ionrs.github.io`,
// which Pages serves at https://ionrs.github.io/ from the `main` branch.
export default defineConfig({
  site: "https://ionrs.github.io",
  base: "/",
  trailingSlash: "always",
  integrations: [
    starlight({
      title: "Ion",
      description:
        "A fast, embeddable scripting language for Rust applications.",
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/ionrs/ionrs",
        },
      ],
      editLink: {
        baseUrl:
          "https://github.com/ionrs/ionrs/edit/main/site/",
      },
      lastUpdated: true,
      components: {
        Footer: "./src/components/Footer.astro",
      },
      customCss: ["./src/styles/site.css"],
      expressiveCode: {
        shiki: {
          langs: [ionGrammar],
        },
      },
      sidebar: [
        { label: "Introduction", link: "/" },
        {
          label: "Language",
          autogenerate: { directory: "language" },
        },
        {
          label: "Guides",
          autogenerate: { directory: "guides" },
        },
        {
          label: "Examples",
          autogenerate: { directory: "examples" },
        },
        {
          label: "Stdlib reference",
          autogenerate: { directory: "reference" },
          collapsed: false,
        },
        { label: "Design", autogenerate: { directory: "design" } },
        { label: "Changelog", link: "/changelog/" },
      ],
    }),
    sitemap(),
  ],
  vite: {
    define: {
      __ION_GIT_DESCRIBE__: JSON.stringify(gitDescribe),
    },
  },
});
