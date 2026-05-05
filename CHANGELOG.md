# Changelog

All notable changes to the Ion language and its tooling are recorded here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
project follows [SemVer](https://semver.org/) for the published crates
(`ion-derive`, `ion-core`, `ionrs-cli`, `ion-lsp`).

Editor extensions track their own version numbers under each entry.

## [Unreleased]

## [0.9.3] — 2026-05-05

### Added

- Python-style argument resolution for Ion and host callables:
  `*args`, `**kwargs`, `*expr`, `**expr`, positional-only `/`, and
  keyword-only `*`.
- Signature-aware host registration APIs:
  `Engine::{register_fn_sig, register_closure_sig, register_async_fn_sig}`
  and matching `Module::*_sig` variants.

### Changed

- Keyword calls to legacy unsigned host callables now error instead of dropping
  keyword names path-dependently.
- Extra positional arguments and duplicate positional/keyword bindings now
  error consistently across tree-walk, VM, and async-runtime paths.
- Duplicate Ion parameter names are now rejected at parse time.
- VM call handling now includes `CallResolved`, `TailCallNamed`,
  `TailCallResolved`, `MethodCallResolved`, `SpawnCallResolved`, `KwInsert`,
  and `KwMerge` for named and spread-containing calls.

### Fixed

- Method calls now expand `*expr` list spreads consistently across tree-walk,
  VM, and async-runtime paths, while rejecting keyword arguments before method
  dispatch.
- Named and spread tail calls now preserve VM tail-call optimization.
- External `EngineHandle::call_async` callbacks into signed host callables now
  apply signature defaults and slot resolution.

## [0.9.2] — 2026-05-05

This release expands the standard library around binary data, randomness, and
file-padding workflows, and publishes the public crates in lockstep:
`ion-derive`, `ion-core`, `ionrs-cli`, and `ion-lsp`.

### Added

- **`bytes::` stdlib module** with constructors and helpers for binary data:
  `new`, `zeroed`, `repeat`, `from_list`, `from_str`, `from_hex`,
  `from_base64`, `concat`, and `join`.
- **Endian byte helpers** on both bytes values and the `bytes::` module:
  `read_u16_le` / `read_u16_be`, `read_u32_le` / `read_u32_be`,
  `read_u64_le` / `read_u64_be`, signed `read_i*` variants, and packing
  helpers `bytes::u16_*`, `bytes::u32_*`, `bytes::u64_*`, `bytes::i16_*`,
  `bytes::i32_*`, and `bytes::i64_*`.
- **`rand::` stdlib module** with `int`, `float`, `bool`, `bytes`, `choice`,
  `shuffle`, and `sample`. Bounded numeric helpers use half-open ranges;
  `choice` returns `None` for empty inputs; `sample` samples without
  replacement.
- **Random file helpers in `fs::`**:
  `fs::append_random(path, count) -> int` and
  `fs::pad_random(path, target_size) -> int`. Both stream random bytes in
  bounded chunks and are available in sync and `async-runtime` builds.

### Changed

- The embedded stdlib documentation manifest now includes the `bytes::`,
  `rand::`, and new `fs::` helper entries for editor completion and hover
  surfaces.
- Language and stdlib reference docs now cover the new binary, random, and
  file-padding APIs.
- Test documentation was refreshed to match current default and async-runtime
  suite sizes.

## [0.9.0] — 2026-05-03

This release implements host-name hiding: every host-registered identifier
(enum names, variant names, struct/field names, module names, function names,
qualified `mod::fn` paths) is folded to a 64-bit FNV-1a hash at compile time
and never appears in the release binary's `.rodata`. See
`docs/hide-names.md`.
`strings target/release/host_bin` now reveals zero Ion-level identifiers
from registered types or stdlib functions.

This is a **major breaking-change release** for embedders. Any host that
calls `Engine::register_fn`, `Engine::register_module`, `Module::new`,
`Module::register_*`, `Module::set`, or constructs `HostStructDef` /
`HostEnumDef` directly will need to migrate. Scripts themselves do not
change. Test counts: 1100+ across default, async-runtime, and
legacy-threaded-concurrency feature combinations.

### Changed (breaking)

- **Module / Engine registration APIs take `u64` name hashes** instead of
  `&str`. Use the new `ion_core::h!("name")` macro (compile-time FNV-1a)
  at the call site:
  ```rust
  // before
  engine.register_fn("square", |args| { ... });
  let mut math = Module::new("math");
  math.register_fn("add", |args| { ... });
  math.set("PI", Value::Float(std::f64::consts::PI));

  // after
  use ion_core::h;
  engine.register_fn(h!("square"), |args| { ... });
  let mut math = Module::new(h!("math"));
  math.register_fn(h!("add"), |args| { ... });
  math.set(h!("PI"), Value::Float(std::f64::consts::PI));
  ```
  The literal `"square"` / `"math"` / `"add"` / `"PI"` is consumed by
  `h!()`'s const evaluation at compile time and folded to a `u64`. No
  string survives in the release binary. (`Module::register_fn`,
  `register_closure`, `register_async_fn`, `set`; `Engine::register_fn`,
  `register_closure`, `register_async_fn`.)
- **`Value::HostEnum`, `Value::HostStruct`, `Value::BuiltinFn`,
  `Value::BuiltinClosure`, `Value::AsyncBuiltinClosure` reshaped** to
  carry only u64 hashes — no name strings:
  ```rust
  // before
  Value::HostEnum { enum_name: String, variant: String, data: Vec<Value> }
  Value::HostStruct { type_name: String, fields: IndexMap<String, Value> }
  Value::BuiltinFn(String, BuiltinFn)

  // after
  Value::HostEnum { enum_hash: u64, variant_hash: u64, data: Vec<Value> }
  Value::HostStruct { type_hash: u64, fields: IndexMap<u64, Value> }
  Value::BuiltinFn { qualified_hash: u64, func: BuiltinFn }
  ```
  `Value::BuiltinFn` shrinks from ~32 bytes to 16 bytes and saves a
  `String::clone` per construction.
- **`HostStructDef`, `HostEnumDef`, `HostVariantDef` carry hashes:**
  `name: String` → `name_hash: u64`; `HostStructDef.fields:
  Vec<String>` → `Vec<u64>`. Use `h!("Color")` etc. at construction.
- **`Value::Module(Arc<ModuleTable>)` is a new variant** distinct from
  `Value::Dict`. Modules registered through `Engine::register_module`
  produce `Value::Module`; `Value::Dict` remains for script-side
  user-built dicts. `mod::fn` and `use mod::*` now hit the typed module
  path; the legacy `Value::Dict`-as-module behaviour still works for
  back-compat with hosts that injected dicts directly.
- **`Module::to_value` → `Module::into_value`**, returning
  `Value::Module(Arc<ModuleTable>)` directly. `Module::name` field
  removed; use `Module::name_hash()`.
- **`#[derive(IonType)]` emits `u64` literals** instead of
  `name.to_string()`. Field names, variant names, and the type name are
  hashed at proc-macro expansion; the source identifier never reaches
  the generated `TokenStream`. Generated `to_ion` and `ion_type_def`
  also include a `cfg(debug_assertions)` block that registers the
  hashes back to their source-form names with `ion_core::names`, so
  Display and diagnostic output render readably in dev builds.
- **`TypeRegistry::construct_struct` now takes `IndexMap<u64, Value>`**
  (pre-hashed field names); callers (interpreter, VM, async runtime)
  hash script-source identifiers at the boundary.
- **JSON / MessagePack output of `Value::HostEnum` / `Value::HostStruct`
  changes:** field/variant keys come from the optional `names` registry
  when populated; otherwise hex-formatted hashes
  (`"#0123456789abcdef"`) are emitted. Round-trips losslessly within
  the same registry.

### Added

- **`ion_core::hash` module** with `const fn fnv1a64` (matches canonical
  FNV-1a test vectors), `const fn h(&str) -> u64`, `const fn mix(u64,
  u64) -> u64`, plus `h!()` and `qh!()` macros that fold their inputs
  to compile-time `u64` constants.
- **`ion_core::names` module** — optional runtime hash → name mapping
  used by `Display`, `to_json`, and error rendering. Populated three
  ways:
  - Debug builds (`cfg(debug_assertions)`): every `h!()` and `qh!()`
    site auto-registers its source literal exactly once on first
    execution. Tests, dev binaries, and `cargo run` all see readable
    names with no extra setup.
  - Release builds: empty by default. Hosts can call
    `names::register` / `register_many`, or load a sidecar JSON with
    `names::load_sidecar_json`.
  - Sidecar workflow: a build script can call `names::dump_sidecar_json`
    on a fully-populated debug build to emit `myapp.names`, which the
    release host loads at startup.
- **`embedded-stdlib-docs` cargo feature on `ion-core`.** Gates the
  `STDLIB_DOCS_JSON` constant (the `ionDocManifest` for stdlib hover /
  completion / docs-site rendering). Off by default — release embedders
  who don't ship a docs surface skip the JSON blob entirely. `ion-lsp`
  enables it.
- **`Module::new(u64)`, `register_fn(u64, fn)`, `register_closure(u64,
  fn)`, `register_async_fn(u64, fn)`, `set(u64, value)`,
  `register_submodule(Module)`, `into_value() -> Value`,
  `name_hash() -> u64`, `name_hashes() -> Vec<u64>`** — the hash-only
  builder API.
- **`Env::define_h(u64, Value)`, `get_h(u64) -> Option<&Value>`,
  `get_sym_or_global(Symbol) -> Option<&Value>`** for host-registered
  globals that bypass the `StringPool`.
- **Hash collision detection**: `Module::register_fn` and friends panic
  on duplicate insertion at startup. `TypeRegistry::register_struct` /
  `register_enum` panic on real shape mismatch (different fields or
  variants under the same name hash) but are idempotent on identical
  re-registrations.

### Removed

- **`Module::name: String` field** — replaced by `name_hash()`.
- **`Module::names() -> Vec<String>`** — replaced by `name_hashes() ->
  Vec<u64>`.
- **String-taking `register_fn(&str, …)` / `set(&str, …)` overloads on
  `Module` and `Engine`.** These would have leaked the literal into
  `.rodata`; they are removed outright (no shim).

### Additional hardening (release builds only)

- **`ion_str!` / `ion_static_str!` macros are gated on
  `cfg(debug_assertions)`.** Release builds replace every diagnostic
  literal with the generic placeholder `"runtime error"` /
  `"value"`. Combined with the `mod::fn` strip above, this means
  `strings target/release/host_bin` shows none of the stdlib's
  human-readable error text either — only the generic placeholders.
- **Derive-emitted error messages are `cfg`-split.** Debug builds
  format `"expected Player, got …"`; release builds format
  `"expected host struct #abcd…, got …"`, dropping the literal
  type name from `.rodata`.
- **`Module::register_*` lazily registers the qualified name** in
  `ion_core::names` when both the module and item names are
  individually known, so debug builds get readable
  `<builtin math::abs>` Display without an extra `qh!()` site.

### Performance side-effects

- `Value::BuiltinFn` shrunk from `(String, BuiltinFn)` ≈ 32 bytes to
  `{ u64, BuiltinFn }` = 16 bytes; lists/dicts of builtins benefit.
- Builtin registration cost dropped: no `format!("{}::{}", …)` and no
  `String::clone` per fn — it's a `Vec::push` of an integer-keyed slot.
- Stdlib error message strings stripped of `mod::fn` qualifiers; helper
  functions (`semver_parse_arg`, `fs_arg_str`, `arg_str`, `io_err`)
  refactored to drop `fn_name` parameters. Generic error strings dedup
  across the whole stdlib.
- The aggressive bytecode-level dispatch reshape (u64 operands inline in
  bytecode, new `GetModuleSlot` op) is **deferred** as a follow-up.
  `mod::fn` calls still hash the script-source string at runtime in the VM.
  Hiding goal is unaffected.

### Trade-offs

- **Diagnostic context**: stdlib closures emit generic errors
  (`"takes 1 argument"`, `"requires a number"`) — the `mod::fn` prefix
  no longer appears in the error string. The script source span
  (line:col) identifies the call site. The renderer can re-prepend the
  resolved name from `names::lookup(qualified_hash)` if an embedder
  wants it. Preserves runtime semantics of `try { … } catch e { e }`.
- **Module reflection**: `use mod::*` now binds entries by hash on the
  glob path. Scripts that introspect modules via dictionary iteration
  break (DESIGN.md does not promise this surface).

### Editor extensions

- **`zed-ion`** bumped to `0.9.0` for parity with the language version.

### Documentation

- **`README.md`** code samples updated to the `h!()` form.
- **`docs/embedding.md`** updated with the new registration APIs and
  a section on the `names` sidecar workflow.
- **`docs/hide-names.md`** added as the concise reference for the shipped
  host-name hiding behavior.

## [0.8.0] — 2026-05-03

### Added
- **Canonical stdlib doc manifest in `ion-core`.** New
  `ion_core::STDLIB_DOCS_JSON` constant — a complete `IonDocManifest`
  (schema v2) describing every global builtin, built-in type, type-method,
  and stdlib-module function/constant. Embedded via `include_str!` so it
  ships with every build. Single source of truth for the LSP and the
  forthcoming documentation site; eliminates drift between editor tooltips
  and published docs.
- **`IonDocManifest` schema v2** in `ion-lsp`. Adds member kinds
  `method`, `type`, and `builtin` (alongside the existing `function` and
  `constant`); per-member optional `receiver`, `methods`, `variants`,
  `examples`, and `since` fields; and top-level optional `homepage`,
  `repository`, `license`, and `categories` for package metadata. v1
  manifests continue to load unchanged. v3+ rejected.
- **`ion --check <file|->` parse-only mode** in `ionrs-cli`. Lex and
  parse a script (or stdin) without evaluating; exits non-zero with all
  parse errors on stderr. Used by the docs-site CI to verify `.ion` code
  blocks compile.

### Changed
- **`ion-lsp` `DocCatalog::builtins()` is now manifest-driven.** The
  hardcoded `BUILTINS` / `METHODS` / `TYPES` / per-module member tables
  (~1000 lines) have been replaced by a single call that parses
  `ion_core::STDLIB_DOCS_JSON`. Hover, completion, and module overviews
  see the same data they always did — but adding or fixing a stdlib doc
  string is now a one-file change.

### Editor extensions
- No editor extension changes in this release.

## [0.7.7] — 2026-05-03

### Fixed
- **`ion-lsp` initialize response no longer double-nests `capabilities`.** The
  server passed the full `InitializeResult` to `Connection::initialize()`, but
  `lsp-server` itself wraps that argument in `{ "capabilities": ... }`. Strict
  clients (VS Code) parsed `result.capabilities.hoverProvider` as `undefined`,
  concluded the server didn't advertise hover, and never sent a single
  `textDocument/hover` request — hover, completion, and goto-definition all
  silently no-op'd while the LSP process looked healthy. Switched to
  `initialize_start` / `initialize_finish` so the result shape is correct.
- **`ion-lsp` document symbols pass `selectionRange ⊆ range` validation.**
  `def_to_symbol` returned a zero-length `range` (`(line, 0)..(line, 0)`)
  alongside a `selectionRange` that extended out to `(line, col + name_len)`,
  causing VS Code to reject every outline entry with
  `selectionRange must be contained in fullRange`. Range now spans the full
  declaration line.
- **VS Code extension VSIX now bundles `vscode-languageclient`.**
  `editors/vscode/.vscodeignore` excluded `node_modules/**` while the
  extension was unbundled (`tsc` only), so the published 9 KB VSIX was missing
  every runtime dep and crashed on activation with
  `Cannot find module 'vscode-languageclient/node'`. Highlighting (a static
  contribution) survived; everything LSP-driven died. Trimmed `.vscodeignore`
  to keep prod deps and added a `npm run package` script that produces the
  full ~280 KB VSIX.

### Editor extensions
- VS Code 0.7.2 → 0.7.7

## [0.7.6] — 2026-05-02 (JetBrains extension only)

### Fixed
- JetBrains plugin now provides native syntax highlighting for the registered
  `Ion` file type, restoring colors after `.ion` stopped being highlighted by
  TextMate-only association.
- Ion LSP startup now logs the resolved command in IDE logs, making attachment
  issues visible.

### Editor extensions
- JetBrains 0.7.5 → 0.7.6

## [0.7.5] — 2026-05-02 (JetBrains extension only)

### Fixed
- JetBrains plugin now registers `.ion` as a native `Ion` file type and maps
  that file type to LSP4IJ, so the Ion language server attaches reliably in
  RustRover instead of relying only on TextMate filename matching.

### Editor extensions
- JetBrains 0.7.4 → 0.7.5

## [0.7.4] — 2026-05-02 (JetBrains extension only)

### Fixed
- JetBrains plugin now normalizes custom Windows + WSL `ion-lsp` commands
  through a shell with `$HOME/.cargo/bin` on `PATH`, fixing LSP startup and
  hover documentation when users configured commands such as
  `wsl.exe -d Ubuntu --cd /project ion-lsp`.

### Editor extensions
- JetBrains 0.7.3 → 0.7.4

## [0.7.3] — 2026-05-02 (JetBrains extension only)

### Fixed
- JetBrains plugin now targets 2024.2+ and uses LSP4IJ 0.14.2, which restores
  LSP hover routing for TextMate-backed `.ion` files in current JetBrains IDEs.

### Editor extensions
- JetBrains 0.7.2 → 0.7.3

## [0.7.2] — 2026-05-02 (editor extensions only)

### Fixed
- VS Code and JetBrains editor launchers now handle Windows + WSL projects when
  spawning `ion-lsp`, so hover/completion features work when the server is
  installed inside the distro.
- Zed now honors `[lsp.ion-lsp.binary]` settings, detects WSL UNC worktrees, and
  falls back to an executable `$HOME/.cargo/bin/ion-lsp` when Zed's PATH misses
  Cargo-installed tools.
- Zed extension builds now follow current Zed docs with `zed_extension_api`
  0.7.0, requiring Zed 0.205.x or newer for dev-extension installs.

### Editor extensions
- VS Code 0.7.0 → 0.7.2
- Zed 0.7.0 → 0.7.2
- JetBrains 0.7.0 → 0.7.2

## [0.7.1] — 2026-05-02 (`ion-lsp` only)

### Added
- **Workspace-provided Ion docs for `ion-lsp`** — host runtimes can provide
  `.json` doc manifests for modules such as `sensor`, `host`, `ipc`, and
  `win32` without forking editor plugins or hard-coding runtime-specific docs
  into generic Ion.
- Manifests are versioned with `ionDocVersion: 1` and load from
  `<workspace>/.ion/ion-docs/*.json`, `<workspace>/ion-docs/*.json`, and
  `ION_LSP_DOCS` paths.
- External docs support module overview hover, member hover, completion after
  `module::`, nested module completion such as `sensor::session::`, and
  function/constant completion kinds.

### Changed
- LSP hover/completion docs now flow through a shared documentation catalog.
  Built-in stdlib docs load first; external docs may add modules or override
  built-in module/member keys.

### Fixed
- Invalid or missing external doc manifests no longer risk crashing the LSP;
  load failures are reported as diagnostic-safe stderr warnings.

## [0.7.0] — 2026-05-02

### Added
- **`os::` stdlib module** — OS / arch detection (`os::name`, `os::arch`,
  `os::family`, `os::pointer_width`, `os::dll_extension`, `os::exe_extension`),
  env vars (`env_var`, `has_env_var`, `env_vars`), and process info (`cwd`,
  `pid`, `args`, `temp_dir`). Pure-`std`, no new dependencies. Enabled by
  default; embedders can opt out with `default-features = false` on `ion-core`.
- `Engine::set_args` / `Engine::with_args` / `Engine::args` to inject script
  arguments reachable from Ion as `os::args()`. The `ion` CLI now passes
  positional args after the script path through to `os::args()`.
- **`path::` stdlib module** — pure-string path manipulation: `sep`, `join`,
  `parent`, `basename`, `stem`, `extension`, `with_extension`, `is_absolute`,
  `is_relative`, `components`, `normalize`. No I/O, always-on, no feature gate.
- **`fs::` stdlib module** — filesystem I/O with `read`, `read_bytes`, `write`,
  `append`, `exists`, `is_file`, `is_dir`, `list_dir`, `create_dir`,
  `create_dir_all`, `remove_file`, `remove_dir`, `remove_dir_all`, `rename`,
  `copy`, `metadata`, `canonicalize`. Single non-coloured surface — same names
  in sync and async builds. New `fs` cargo feature (in default).
- LSP hover/completion learn the `os::`, `path::`, and `fs::` namespaces.
- Tree-sitter, VS Code, JetBrains, and Zed grammars recognise `os`, `path`,
  and `fs` as builtin module names.
- `ion` CLI gains an `async-runtime` cargo feature that sets up a
  current-thread Tokio runtime and drives `Engine::eval_async` for both
  `run_file` and the REPL.

### Changed
- **`io::print*` no longer blocks the executor under `async-runtime`.** The
  `io::` module is now registered with async builtins that dispatch the
  underlying `OutputHandler::write` call onto Tokio's blocking thread pool
  via `spawn_blocking`. Sync builds keep the old direct-call path. The
  `OutputHandler` trait is unchanged; embedder code is unaffected.
- **`Engine::eval` and `Engine::vm_eval` are removed under `async-runtime`.**
  The sync and async runtimes are now mutually exclusive at the cargo-feature
  level — async builds must use `Engine::eval_async`. This guarantees that
  non-coloured stdlib functions (`fs::read`, `io::println`, …) resolve to one
  implementation per build.

### Editor extensions
- VS Code 0.6.0 → 0.7.0
- Zed 0.6.0 → 0.7.0
- JetBrains 0.6.0 → 0.7.0

## [0.6.0] — 2026-05-02

### Added
- **`semver::` stdlib module** — `parse`, `is_valid`, `format`, `compare`,
  `eq`/`gt`/`gte`/`lt`/`lte`, `satisfies`, `bump_major`/`bump_minor`/`bump_patch`.
  Backed by the `semver` crate. Versions round-trip through dicts shaped
  `#{major, minor, patch, pre, build}`. Enabled by default; embedders can opt
  out with `default-features = false` on `ion-core`.
- New `semver` cargo feature on `ion-core` (in `default`).
- LSP hover/completion learn the `semver::` namespace.
- Tree-sitter, VS Code, JetBrains, and Zed grammars recognise `semver` as a
  builtin module name.

### Editor extensions
- VS Code 0.5.0 → 0.6.0
- Zed 0.5.0 → 0.6.0
- JetBrains 0.5.0 → 0.6.0

## [0.5.0] — 2026-05-02

### Added
- **Aliased `use` imports** — `use io::println as say;`, `use math::{add as sum, PI};`.
  Both single and braced-list forms accept an optional `as <ident>` clause; the
  original name is used for module lookup, the alias becomes the local binding.
  Glob imports (`use m::*`) cannot be aliased. Supported by the tree-walking
  interpreter, the bytecode VM, the LSP (hover shows `use m::name as alias`),
  and tree-sitter / TextMate highlighting.
- `as` recognized as a keyword across all editor grammars (tree-sitter, VS Code
  TextMate, JetBrains TextMate bundle).
- Tree-sitter grammar gains an `import_item` node with `name`/`alias` fields.

### Editor extensions
- VS Code 0.4.0 → 0.5.0
- Zed 0.4.0 → 0.5.0
- JetBrains 0.4.0 → 0.5.0

## [0.4.0] — 2026-05-01

### Added
- **`log::` stdlib module** with `trace`, `debug`, `info`, `warn`, `error`,
  plus `set_level`, `level`, `enabled`. Each level function takes
  `(message, fields?)`; the optional dict argument is passed through to the
  handler as structured fields.
- **Compile-time level cap** in both the bytecode compiler and the tree-walk
  interpreter — `log::<level>(...)` callsites whose level is above
  `COMPILE_LOG_CAP` are stripped (args and all). Mirrors `tracing`'s
  `release_max_level_*` semantics.
- **Cargo features** on `ion-core` to set the cap:
  `log_max_level_off|error|warn|info|debug|trace`. With none enabled the cap
  defaults to `Trace` under `debug_assertions` and `Info` otherwise.
- **`LogHandler` trait** + default `StdLogHandler` (stderr, `LEVEL message
  [k=v ...]` format) honouring an engine-wide threshold.
- **`tracing` feature** that exposes `TracingLogHandler`, forwarding each
  level to `tracing::event!` so embedders inherit subscriber filtering,
  spans, and structured fields.
- `Engine::with_handlers`, `Engine::set_log_handler`,
  `Engine::set_log_handler_arc`, `Engine::set_log_level`,
  `Engine::log_level`, `register_stdlib_with_handlers`,
  `register_builtins_with_handlers`, `Interpreter::with_handlers`.
- LSP hover/completion learn the `log::` namespace.
- VS Code, JetBrains, Zed, and tree-sitter grammars recognize `log` as a
  builtin module name.

### Editor extensions
- VS Code 0.3.2 → 0.4.0
- Zed 0.3.2 → 0.4.0
- JetBrains 0.3.2 → 0.4.0

## [0.3.2] — 2026-05-01 (`ion-lsp` only)

### Added
- **Hover overhaul** in `ion-lsp`:
  - Method hover (`xs.push`, `s.to_upper`, `task.await`, …).
  - Module-member hover (`math::sqrt`, `math::PI`, `io::println`, `json::decode`).
  - Module-name hover (cursor on `math` shows the module overview).
  - Type-name hover for `Option`, `Result`, `bool`, `dict`, `list`, `tuple`,
    `set`, `cell`, `any`, `fn`.
  - Function parameters tracked as definitions so they hover.
  - `let` hover now shows the full source initializer with `mut` and type
    annotation (e.g. `let mut total: int = 0`).
- All hover responses include a token range so editors highlight the
  hovered identifier.
- 13 new hover tests in `ion-lsp`.

### Changed
- The method, module-member, and type-name tables are lifted into shared
  module-level statics so hover and completion stay in sync.

### Editor extensions
- VS Code 0.3.1 → 0.3.2
- Zed 0.3.1 → 0.3.2
- JetBrains 0.3.1 → 0.3.2

## [0.3.1] — 2026-05-01 (editor extensions only)

### Added
- Distinct scope (`storage.type.string.ion` /
  `@string.special.symbol`) for the f-string `f` prefix so themes can color
  it apart from the string body.
- Explicit `punctuation.definition.string.{begin,end}` captures on string
  delimiters in the TextMate grammars.

### Editor extensions
- VS Code 0.2.0 → 0.3.1
- Zed 0.2.0 → 0.3.1
- JetBrains 0.2.1 → 0.3.1

## [0.3.0] — 2026-04-30

### Added
- **Native async runtime** (`async-runtime` Cargo feature) with structured
  concurrency, `spawn` / `.await`, `select`, channels, timers, and
  cooperative scheduling. Replaces the legacy threaded backend for new
  embedders.
- `Engine::eval_async` entry point and async host-function registration
  (`register_async_fn`). Tokio embedding documented in
  `docs/concurrency.md`.
- Async module host functions in stdlib.

### Changed
- VM optimizations folded into the `vm` feature flag (peephole, constant
  folding, dead-code elimination, tail-call optimization).
- Legacy threaded concurrency renamed to `legacy-threaded-concurrency`
  feature (off by default).
- Editor extensions updated for Ion 0.3 syntax — async / `select` / spawn /
  `.await` keywords, nested module paths, loop labels, cell type
  annotations, named arguments, expanded standard module APIs.

### Fixed
- JetBrains Ion file highlighting (TextMate file-type registration).

## [0.2.3] — earlier

### Added
- `references` and `rename` request handlers in `ion-lsp`.
- Labeled `break` and `continue` (`'outer: for ... { break 'outer; }`).

### Fixed
- Friendlier "ion-lsp missing binary" error from the editor extensions.
- JetBrains plugin TextMate registration.

## [0.2.2] — earlier

### Added
- JetBrains IDE plugin (TextMate-based, optional LSP4IJ integration).
- MessagePack byte encoding round-trips (`Value::to_msgpack` /
  `from_msgpack`, `msgpack` Cargo feature).

## [0.2.0] — earlier

### Added
- **Module/namespace system** with `use` imports and `::` path syntax.
- **Built-in stdlib modules**: `math`, `json`, `io`, `string`.
- `rewrite` feature for replacing top-level global values in source.
- Tokio embedding via closure-backed builtins (`Engine::register_closure`).
- Tier A cooperative-scheduler concurrency runtime; Tier C plan documented.
- Zed editor support with tree-sitter grammar and WASM build pipeline.
- Comprehensive parser error recovery (multiple diagnostics per parse).
- Lazy `Value::Range` to avoid allocating large lists for integer ranges.
- 196 cross-validation tests covering sets, spread, match, closures.

### Changed
- Removed backward-compatible top-level builtins; stdlib access is now
  namespaced (`math::abs`, `io::println`, …).
- Optimized VM: cloning avoided in conditionals, faster string ops, broader
  constant deduplication.

### Fixed
- Comprehensive audit: guarded panics, removed dead code, broadened test
  coverage.

## [0.1.0] — initial release

- Tree-walk interpreter with a Starlark-influenced syntax.
- Optional bytecode VM (`vm` feature).
- `#[derive(IonType)]` for host struct/enum injection.
- VS Code extension (TextMate grammar + LSP client).
- Initial `ion-lsp` with definitions, document symbols, completion, hover,
  diagnostics.

[Unreleased]: https://github.com/ionrs/ionrs/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/ionrs/ionrs/compare/v0.3.0...v0.4.0
[0.3.2]: https://github.com/ionrs/ionrs/compare/v0.3.0...v0.3.2
[0.3.1]: https://github.com/ionrs/ionrs/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/ionrs/ionrs/compare/v0.2.0...v0.3.0
[0.2.3]: https://github.com/ionrs/ionrs/compare/v0.2.0...v0.2.3
[0.2.2]: https://github.com/ionrs/ionrs/compare/v0.2.0...v0.2.2
[0.2.0]: https://github.com/ionrs/ionrs/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ionrs/ionrs/releases/tag/v0.1.0
