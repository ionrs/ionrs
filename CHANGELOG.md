# Changelog

All notable changes to the Ion language and its tooling are recorded here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
project follows [SemVer](https://semver.org/) for the published crates
(`ion-derive`, `ion-core`, `ionlang-cli`, `ion-lsp`).

Editor extensions track their own version numbers under each entry.

## [Unreleased]

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

[Unreleased]: https://github.com/chutuananh2k/ion-lang/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/chutuananh2k/ion-lang/compare/v0.3.0...v0.4.0
[0.3.2]: https://github.com/chutuananh2k/ion-lang/compare/v0.3.0...v0.3.2
[0.3.1]: https://github.com/chutuananh2k/ion-lang/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/chutuananh2k/ion-lang/compare/v0.2.0...v0.3.0
[0.2.3]: https://github.com/chutuananh2k/ion-lang/compare/v0.2.0...v0.2.3
[0.2.2]: https://github.com/chutuananh2k/ion-lang/compare/v0.2.0...v0.2.2
[0.2.0]: https://github.com/chutuananh2k/ion-lang/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/chutuananh2k/ion-lang/releases/tag/v0.1.0
