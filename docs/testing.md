# Ion Testing Strategy

## Test Suite

Counts vary by feature flags; the async-runtime suite is only compiled when
`async-runtime` is enabled.

| Location | Count | Coverage |
|---|---|---|
| `ion-core/tests/integration.rs` | ~462 | Tree-walk interpreter, stdlib, host types, `register_closure` |
| `ion-core/tests/cross_validate.rs` | ~205 | Tree-walk ↔ VM parity |
| `ion-core/tests/vm.rs` | ~161 | VM-specific behavior |
| `ion-core/tests/async_runtime.rs` | ~129 | `async-runtime` native Tokio eval, host futures, timers, channels, callbacks |
| `ion-core/tests/integration_async.rs` | ~14 | End-to-end stdlib coverage under `Engine::eval_async` |
| `ion-core/tests/edge_cases.rs` | ~65 | Adversarial/edge cases |
| `ion-core/tests/legacy_threaded_concurrency.rs` | 17 | Legacy `legacy-threaded-concurrency` OS-thread sync-eval backend |
| `ion-core/src/` (unit tests) | ~28 | Lexer, parser, hash, log, names, and stdlib manifest checks |
| Doctests | 2+ | Public crate examples |

## Cross-Validation Pattern
```rust
fn assert_both_eq(src: &str, expected: Value) {
    let tw_val = Engine::new().eval(src).unwrap();
    let vm_val = Engine::new().vm_eval(src).unwrap();
    assert_eq!(tw_val, expected);
    assert_eq!(vm_val, expected);
}
```

## Running Tests

```sh
RUST_MIN_STACK=16777216 cargo test --workspace --all-features   # everything enabled
cargo test --all-features -p ion-core --test cross_validate   # just parity
cargo test -p ion-core --features async-runtime --test async_runtime   # native async runtime
cargo test -p ion-core --no-default-features --features legacy-threaded-concurrency --test legacy_threaded_concurrency
cargo test --all-features -p ion-core --test integration register_closure   # by name

cargo clippy --all-features --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Examples as tests

Example binaries are compiled by `cargo test` when their required
features are active:

- `cargo run --example embed -p ion-core` — basic embedding
- `cargo run --example tokio_host -p ion-core --features async-runtime` — native Tokio async host

## CI Pipeline (.github/workflows/ci.yml)
- test: `cargo test --workspace --all-features`
- clippy: `--all-features --all-targets -- -D warnings`
- fmt: `cargo fmt --all -- --check`
- build-lsp: `cargo build -p ion-lsp --release`

## Fuzzing
- `ion-core/fuzz/` directory (excluded from workspace)
