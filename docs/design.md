# Ion Language — Design Decisions

## Core Decisions
1. Strong typing, interpreter-inferred (no explicit annotations)
2. Semicolons required
3. Explicit string interpolation `f"..."`
4. Structured concurrency (feature-gated)
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
- `+=` etc. are sugar for `x = x + ...` — require `let mut`
- Pipe `|>` always passes as first argument: `a |> f(b)` = `f(a, b)`
- Closures capture by value at time of creation

## Type Philosophy
- Scripts use dicts (`#{}`) for ad-hoc data
- Typed structs/enums come from Rust host via `#[derive(IonType)]` or `register_type`
- Scripts can construct, access fields, pattern match on host types — but never declare them

## Security Feature
- `obfstr` crate integration: cargo feature `obfuscate`
- `ion_str!()` macro returns `String`, wraps `obfstr::obfstr!()` when feature enabled
- `ion_static_str!()` for `&'static str` contexts (pass-through, not obfuscated)
- When using `ion_str!()` with `str::contains()`, use `&*ion_str!(...)` for `&str` coercion
- All error messages, format strings in interpreter/vm use `ion_str!()`
