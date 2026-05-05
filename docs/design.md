# Ion Language — Design Decisions

## Core Decisions
1. Strong typing, interpreter-inferred, with optional type annotations on `let` bindings
2. Semicolons required
3. Explicit string interpolation `f"..."`
4. Structured concurrency (native Tokio runtime under `async-runtime`; legacy OS-thread backend under `legacy-threaded-concurrency`)
5. Flat module system (single-file scripts + host-provided globals)
6. Both loops (`for`, `while`) and functional (`map`/`filter`/`fold`)
7. **No struct/enum/impl in scripts** — host-injected only via `#[derive(IonType)]`
8. Embedded language (like Lua for game logic)
9. Serde-native host integration
10. JSON/dict as first-class citizens (`#{}` syntax)
11. Rust-flavored: pattern matching, Result/Option, `?` operator, immutable-by-default

## No-Surprises Rules
- `#{ }` for dict literals — no ambiguity with `{ }` blocks
- `.field` = host struct only, `["key"]` = dict only — no dot sugar for dicts
- `?` behavior based on VALUE type (Result/Option), not function return type
- `unwrap()` allowed on Option/Result (updated — was previously forbidden)
- All collection methods return NEW collections, no mutation
- `spawn` only inside `async {}` — strictly enforced
- Async host functions are uncolored in Ion source; `eval_async` parks VM continuations on Tokio futures
- `+=` etc. are sugar for `x = x + ...` — require `let mut`
- Pipe `|>` always passes as first argument: `a |> f(b)` = `f(a, b)`
- Closures capture by value at time of creation

## Type Philosophy
- Scripts use dicts (`#{}`) for ad-hoc data
- Typed structs/enums come from Rust host via `#[derive(IonType)]` or `register_type`
- Scripts can construct, access fields, pattern match on host types — but never declare them
- Optional type annotations on `let` bindings: `let x: int = 42;`
  - Supported types: `int`, `float`, `bool`, `string`, `bytes`, `list`, `dict`, `tuple`, `set`, `fn`, `any`
  - Generic forms: `Option<T>`, `Result<T, E>`, `list<T>`, `dict<K, V>`
  - Only the outer type is checked at runtime; inner/generic types are documentation-only hints
  - `any` accepts all types; unknown type names also pass (forward compatibility)

## Error Redaction and Name Hiding
- Public Rust errors use `redacted-error` through crate-local helpers; release
  `Display` and `Debug` must not include runtime detail.
- The workspace does not depend directly on a string-obfuscation crate; do not
  add one back.
- `ion_str!()` returns a `String` for static public messages. In release it
  collapses diagnostics to a generic public error unless the call site uses the
  explicit public-string facade.
- `ion_format!()` is for dynamic diagnostic detail. It formats in debug builds
  and skips evaluating format arguments in release builds.
- `ion_static_str!()` is for `&'static str` contexts such as type names; release
  builds collapse these diagnostics to a generic public word.
- Function, method, module, type, field, and variant names are protected with
  compile-time hashes and debug-only name registration rather than runtime
  string obfuscation.
