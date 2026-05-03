// Types and helpers for IonDocManifest v2. Schema is owned by ion-lsp;
// this module is pure data + functions so it can be imported from both
// build-time Astro components and client-side scripts without dragging
// in the stdlib JSON.

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

export function memberAnchor(name: string): string {
  return name.replace(/[^a-zA-Z0-9_-]/g, "_");
}

export function validateManifest(
  data: unknown
):
  | { ok: true; manifest: IonDocManifest }
  | { ok: false; error: string } {
  if (!data || typeof data !== "object" || Array.isArray(data)) {
    return { ok: false, error: "expected a JSON object" };
  }
  const obj = data as Record<string, unknown>;
  if (obj.ionDocVersion !== 1 && obj.ionDocVersion !== 2) {
    return {
      ok: false,
      error: `unsupported ionDocVersion ${JSON.stringify(obj.ionDocVersion)}; expected 1 or 2`,
    };
  }
  if (obj.modules !== undefined && !Array.isArray(obj.modules)) {
    return { ok: false, error: "`modules` must be an array" };
  }
  return { ok: true, manifest: obj as unknown as IonDocManifest };
}
