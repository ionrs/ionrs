# Ion Testing Strategy

## Test Suite (854 tests)

| Location | Count | Coverage |
|---|---|---|
| `ion-core/tests/integration.rs` | 395 | Tree-walk interpreter, stdlib, host types, `register_closure` |
| `ion-core/tests/cross_validate.rs` | 205 | Tree-walk ↔ VM parity |
| `ion-core/tests/vm.rs` | 153 | VM-specific behavior |
| `ion-core/tests/edge_cases.rs` | 56 | Adversarial/edge cases |
| `ion-core/src/` (unit tests) | 26 | Lexer, parser, `rewrite` module |
| `ion-core/tests/concurrency.rs` | 17 | `concurrency` feature (async/spawn/await/select/channels/cancel) |
| Doctests | 2 | `lib.rs` Quick Start, `rewrite.rs` example |

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
cargo test --workspace --all-features           # everything — 854 tests
cargo test --all-features -p ion-core --test cross_validate   # just parity
cargo test --all-features -p ion-core --test concurrency      # concurrency only
cargo test --all-features -p ion-core --test integration register_closure   # by name

cargo clippy --all-features --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Examples as tests

Example binaries are compiled by `cargo test` when their required
features are active:

- `cargo run --example embed -p ion-core` — basic embedding
- `cargo run --example tokio_host -p ion-core --features concurrency` — tokio host

## CI Pipeline (.github/workflows/ci.yml)
- test: `cargo test --workspace --all-features`
- clippy: `--all-features --all-targets -- -D warnings`
- fmt: `cargo fmt --all -- --check`
- build-lsp: `cargo build -p ion-lsp --release`

## Fuzzing
- `ion-core/fuzz/` directory (excluded from workspace)
