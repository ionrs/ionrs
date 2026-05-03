// Types and helpers for IonDocManifest v2.
// The schema is owned by `ion-lsp` (Rust); see ion-lsp/src/main.rs.
// JSON Schema published in PR 2 will live at /schemas/ion-doc-v2.json.

import stdlibJson from "../../public/manifests/stdlib.json";

export type MemberKind =
  | "function"
  | "constant"
  | "method"
  | "type"
  | "builtin";

export interface ManifestMember {
  name: string;
  kind: MemberKind;
  signature: string;
  doc: string;
  receiver?: string;
  methods?: ManifestMember[];
  variants?: string[];
  examples?: string[];
  since?: string;
}

export interface ManifestModule {
  name: string;
  summary?: string;
  members?: ManifestMember[];
  modules?: ManifestModule[];
}

export interface IonDocManifest {
  ionDocVersion: 1 | 2;
  profile?: string;
  homepage?: string;
  repository?: string;
  license?: string;
  categories?: string[];
  modules?: ManifestModule[];
}

export const stdlib = stdlibJson as IonDocManifest;

/** Walk a module tree and yield (path, module) pairs depth-first. */
export function* walkModules(
  modules: ManifestModule[] | undefined,
  parentPath: string[] = []
): Generator<{ pathSegments: string[]; module: ManifestModule }> {
  if (!modules) return;
  for (const m of modules) {
    const pathSegments = [...parentPath, m.name];
    yield { pathSegments, module: m };
    yield* walkModules(m.modules, pathSegments);
  }
}

/** Resolve a slash-joined path (e.g. ["math"] or ["sensor", "session"])
 *  to the module node, or undefined. */
export function findModule(
  manifest: IonDocManifest,
  segments: string[]
): ManifestModule | undefined {
  let current: ManifestModule | undefined;
  let pool = manifest.modules ?? [];
  for (const seg of segments) {
    current = pool.find((m) => m.name === seg);
    if (!current) return undefined;
    pool = current.modules ?? [];
  }
  return current;
}

/** Group members by kind for rendering. */
export function partitionMembers(members: ManifestMember[] | undefined): {
  builtins: ManifestMember[];
  constants: ManifestMember[];
  functions: ManifestMember[];
  types: ManifestMember[];
  methods: ManifestMember[];
} {
  const out = {
    builtins: [] as ManifestMember[],
    constants: [] as ManifestMember[],
    functions: [] as ManifestMember[],
    types: [] as ManifestMember[],
    methods: [] as ManifestMember[],
  };
  for (const m of members ?? []) {
    switch (m.kind) {
      case "builtin":
        out.builtins.push(m);
        break;
      case "constant":
        out.constants.push(m);
        break;
      case "function":
        out.functions.push(m);
        break;
      case "type":
        out.types.push(m);
        break;
      case "method":
        out.methods.push(m);
        break;
    }
  }
  for (const list of Object.values(out)) {
    list.sort((a, b) => a.name.localeCompare(b.name));
  }
  return out;
}

/** Make an HTML id from a member name. */
export function memberAnchor(name: string): string {
  return name.replace(/[^a-zA-Z0-9_-]/g, "_");
}
