// Stdlib manifest instance plus re-exports of the schema/helpers from
// ion-doc.ts. Build-time Astro components import from here; client-side
// scripts that don't need the stdlib JSON should import ion-doc.ts
// directly to keep the bundle small.

import stdlibJson from "../../public/manifests/stdlib.json";
import type { IonDocManifest } from "./ion-doc";

export type {
  IonDocManifest,
  ManifestModule,
  ManifestMember,
  MemberKind,
} from "./ion-doc";
export {
  walkModules,
  findModule,
  partitionMembers,
  memberAnchor,
  validateManifest,
} from "./ion-doc";

export const stdlib = stdlibJson as IonDocManifest;
