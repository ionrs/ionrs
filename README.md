# Ion

A fast, embeddable scripting language for Rust applications.

Ion is designed for embedding in Rust programs — like Lua for game engines, but with Rust-flavored syntax and strong typing. It features a tree-walk interpreter and an optimizing bytecode VM, Serde-native host integration, and a rich standard library.

## Quick Start

```rust
use ion_core::Engine;

fn main() {
    let mut engine = Engine::new();

    // Evaluate expressions
    let result = engine.eval(r#"
        let items = [1, 2, 3, 4, 5];
        items.filter(|x| x > 2).map(|x| x * 10)
    "#).unwrap();

    println!("{}", result); // [30, 40, 50]
}
```

## Features at a Glance

```
// Variables — immutable by default
let name = "Ion";
let mut counter = 0;

// Destructuring
let (x, y) = (10, 20);
let [head, ...rest] = [1, 2, 3, 4];

// String interpolation
let greeting = f"hello {name}!";

// Functions with default args
fn greet(who, greeting = "hello") {
    f"{greeting}, {who}!"
}

// Pattern matching
let result = Ok(42);
match result {
    Ok(n) if n > 0 => f"positive: {n}",
    Ok(n) => f"non-positive: {n}",
    Err(e) => f"error: {e}",
}

// First-class functions
let double = |x| x * 2;
[1, 2, 3].map(double)  // [2, 4, 6]

// Dicts (JSON-like)
let config = #{
    host: "localhost",
    port: 8080,
    tags: ["web", "api"],
};

// Error handling
fn parse(s) {
    let n = s.to_int()?;
    Ok(n * 2)
}

// Try/catch
try {
    risky_operation();
} catch e {
    f"caught: {e}"
}

// Modules and imports
use math::{add, PI};
let area = add(PI, PI);

// Loops + functional
let sum = [1, 2, 3, 4, 5].fold(0, |acc, x| acc + x);
for (i, item) in enumerate(items) {
    io::println(f"{i}: {item}");
}
```

## Embedding in Rust

```rust
use ion_core::Engine;
use ion_core::value::Value;

let mut engine = Engine::new();

// Register host functions (plain fn pointer — no captures)
engine.register_fn("fetch_score", |args: &[Value]| {
    let name = args[0].as_str().unwrap_or("unknown");
    Ok(Value::Int(match name {
        "alice" => 95,
        "bob" => 87,
        _ => 0,
    }))
});

// Register closures that capture host state (DB pool, tokio Handle,
// shared counter, etc.)
let hits = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
let hits_c = hits.clone();
engine.register_closure("record_hit", move |_args| {
    hits_c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(Value::Unit)
});

// Use from Ion scripts
let result = engine.eval(r#"
    record_hit();
    let score = fetch_score("alice");
    if score >= 90 { "A" } else { "B" }
"#).unwrap();

assert_eq!(result, Value::Str("A".to_string()));
```

### Register Modules

```rust
use ion_core::module::Module;
use ion_core::value::Value;

let mut math = Module::new("math");
math.register_fn("add", |args: &[Value]| {
    Ok(Value::Int(args[0].as_int().unwrap() + args[1].as_int().unwrap()))
});
math.set("PI", Value::Float(std::f64::consts::PI));
engine.register_module(math);

// Scripts can use: math::add(1, 2) or `use math::*;`
```

### Host Types with Derive

```rust
use ion_core::{Engine, IonType};
use ion_core::value::Value;

#[derive(IonType)]
struct Player {
    name: String,
    health: i64,
}

let mut engine = Engine::new();
engine.register_type::<Player>();

let result = engine.eval(r#"
    let p = Player { name: "hero", health: 100 };
    f"{p.name} has {p.health} HP"
"#).unwrap();
```

### Bytecode VM

```rust
let mut engine = Engine::new();

// Tree-walk interpreter (default)
let a = engine.eval("1 + 2").unwrap();

// Bytecode VM — faster for hot paths
let b = engine.vm_eval("1 + 2").unwrap();

assert_eq!(a, b);
```

## Types

| Type | Example | Description |
|------|---------|-------------|
| Int | `42`, `-1` | 64-bit signed integer |
| Float | `3.14` | 64-bit floating point |
| Bool | `true`, `false` | Boolean |
| String | `"hello"`, `f"x={x}"` | UTF-8 string |
| List | `[1, 2, 3]` | Ordered collection |
| Tuple | `(1, "a", true)` | Fixed-size heterogeneous |
| Dict | `#{key: val}` | Ordered key-value map |
| Option | `Some(42)`, `None` | Optional value |
| Result | `Ok(val)`, `Err(msg)` | Success or error |
| Bytes | `b"\x00\xFF"` | Immutable byte array |
| Unit | `()` | No value |

## Project Structure

```
ion-core/     Core library — lexer, parser, interpreter, compiler, VM
ion-derive/   #[derive(IonType)] proc macro
ion-cli/      Script runner and REPL
ion-lsp/      Language server (VSCode support)
editors/      Editor syntax highlighting
```

## Cargo Features

| Feature | Default | Description |
|---------|---------|-------------|
| `vm` | Yes | Bytecode compiler + stack VM |
| `optimize` | Yes | Peephole optimizer, constant folding, DCE, TCO |
| `derive` | Yes | `#[derive(IonType)]` proc macro |
| `concurrency` | No | Structured concurrency (`spawn` / `.await` / `select` / channels) |
| `obfuscate` | No | String obfuscation via obfstr |
| `msgpack` | No | `Value::to_msgpack()` via rmpv |
| `rewrite` | No | Source rewriter — `rewrite::replace_global(src, name, new_value)` |

## Building

```sh
cargo build                          # default features
cargo build --all-features           # everything including concurrency
cargo test --all-features            # run all tests
cargo run --bin ion-cli script.ion   # run a script
cargo run --bin ion-cli              # start REPL
```

## Documentation

- [LANGUAGE.md](LANGUAGE.md) — complete language specification
- [DESIGN.md](DESIGN.md) — design decisions and implementation phases
- [docs/embedding.md](docs/embedding.md) — embedding API reference
- [docs/concurrency.md](docs/concurrency.md) — concurrency model, cancellation,
  and the tokio embedding pattern
- [docs/stdlib.md](docs/stdlib.md) — standard library reference
- [docs/performance.md](docs/performance.md) — performance strategy
- [docs/vm-internals.md](docs/vm-internals.md) — bytecode VM internals
- [docs/testing.md](docs/testing.md) — test suite layout
- [docs/tooling.md](docs/tooling.md) — CLI, LSP, editor support

## Embedding inside a tokio application

Ion's interpreter is synchronous. Inside a tokio host, wrap
`engine.eval()` in `tokio::task::spawn_blocking` and use
`register_closure` to capture a `tokio::runtime::Handle` for
async host calls. See
[docs/concurrency.md](docs/concurrency.md#embedding-inside-a-tokio-host)
and the runnable example at
[`ion-core/examples/tokio_host.rs`](ion-core/examples/tokio_host.rs).

## License

MIT
