# Ion Testing Strategy

## Test Suite (665 tests)
- `ion-core/tests/integration.rs` — 321 integration tests (interpreter)
- `ion-core/tests/vm.rs` — 153 VM-specific tests
- `ion-core/tests/cross_validate.rs` — 125 cross-validation (tree-walk vs VM parity)
- `ion-core/tests/edge_cases.rs` — 38 adversarial/edge case tests
- `ion-core/src/` — 15 unit tests (lexer, parser)
- `ion-core/tests/concurrency.rs` — 12 concurrency tests (feature-gated)
- `ion-core/examples/embed.rs` — 1 example test

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
- `cargo test --all-features` — all tests including VM, concurrency
- `cargo test --all-features -p ion-core --test cross_validate` — just cross-validation
- `cargo clippy --all-features` — lint check
- `cargo fmt --all -- --check` — format check

## CI Pipeline (.github/workflows/ci.yml)
- test: `cargo test --workspace --all-features`
- clippy: `--all-features --all-targets -- -D warnings`
- fmt: `cargo fmt --all -- --check`
- build-lsp: `cargo build -p ion-lsp --release`

## Fuzzing
- `ion-core/fuzz/` directory (excluded from workspace)
