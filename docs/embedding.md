# Ion Embedding API

## Engine (`ion-core/src/engine.rs`)
Primary public API for embedding Ion in Rust applications.

```rust
use ion_core::engine::Engine;
use ion_core::value::Value;

let mut engine = Engine::new();

// Evaluate scripts
engine.eval("let x = 42;")?;              // tree-walk
engine.vm_eval("let x = 42;")?;           // bytecode VM (feature: vm)

// Inject/read values
engine.set("name", Value::Str("Ion".into()));
engine.get("name");                        // Option<Value>
engine.get_all();                          // HashMap<String, Value>

// Typed host values (feature: derive)
engine.set_typed("player", &player);       // T: IonType → Value
engine.get_typed::<Player>("player")?;     // Value → T: IonType

// Register Rust functions (plain fn pointer — no captures)
engine.register_fn("square", |args| { ... });

// Register closures — can capture host state like database pools,
// tokio Handles, shared counters, etc.
let counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
let counter_c = counter.clone();
engine.register_closure("tick", move |_args| {
    counter_c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(Value::Unit)
});

// Register modules (namespaced functions/values)
let mut math = Module::new("math");
math.register_fn("add", |args| { ... });
math.set("PI", Value::Float(std::f64::consts::PI));
engine.register_module(math);              // scripts use math::add() or `use math::*;`

// Register host types
engine.register_type::<Player>();          // via #[derive(IonType)]
engine.register_struct(def);               // manual HostStructDef
engine.register_enum(def);                 // manual HostEnumDef

// Execution limits
engine.set_limits(Limits { max_depth: 100, max_iterations: 10000 });
```

## Handling `io::print*` output

Ion does not write script output directly to the host process. To use
`io::print`, `io::println`, or `io::eprintln`, install an output handler:

```rust
use ion_core::engine::Engine;
use ion_core::stdlib::{OutputHandler, OutputStream};

struct MyOutput;

impl OutputHandler for MyOutput {
    fn write(&self, stream: OutputStream, text: &str) -> Result<(), String> {
        match stream {
            OutputStream::Stdout => {
                // send `text` to your app's stdout, log buffer, UI, etc.
            }
            OutputStream::Stderr => {
                // send `text` to your app's stderr/error channel.
            }
        }
        Ok(())
    }
}

let mut engine = Engine::with_output(MyOutput);
engine.eval(r#"io::println("hello")"#)?;
```

The CLI uses `StdOutput`; embedded hosts can capture, redirect, reject,
or otherwise handle output however they need.

## Registering Rust callbacks

Two registration methods, picked by whether the callback needs to
capture host state.

| Method | Signature | Use when |
|---|---|---|
| `register_fn` | `fn(&[Value]) -> Result<Value, String>` | Stateless builtins. Plain function pointer, zero overhead. |
| `register_closure` | `impl Fn(&[Value]) -> Result<Value, String> + Send + Sync + 'static` | Need to capture a `tokio::runtime::Handle`, a DB pool, an `Arc<Mutex<State>>`, etc. |

Both appear to Ion scripts identically — `type_of(f) == "builtin_fn"`
for both, and both satisfy `let f: fn = ...;` annotations.

## Host Types (`#[derive(IonType)]`)
- Proc macro in `ion-derive/`
- Generates `to_ion()` / `from_ion()` via serde
- Works for structs and enums
- Scripts can construct, access fields, pattern match

## Source rewriting (feature: `rewrite`)

The `ion_core::rewrite` module lets hosts swap the value of a
top-level `let` binding without running the script:

```rust
use ion_core::rewrite::replace_global;

let src = "let threshold = 10;\nfn check(x) { x > threshold }\n";
let out = replace_global(src, "threshold", "42").unwrap();
// out == "let threshold = 42;\nfn check(x) { x > threshold }\n"
```

The replacement fragment is re-parsed; invalid Ion returns
`RewriteError::InvalidReplacement`. Use cases: config surgery before
eval, A/B swapping constants, build-time rewrites.

## Embedding inside a tokio application

Ion's interpreter is synchronous. The conventional pattern:

1. Wrap `engine.eval(...)` in `tokio::task::spawn_blocking` to
   run off the async worker pool.
2. Register tokio-backed host functions via `register_closure`,
   capturing a `tokio::runtime::Handle` and using
   `handle.block_on(fut)` inside the closure.

See [`docs/concurrency.md`](concurrency.md#embedding-inside-a-tokio-host)
for the full discussion and [`ion-core/examples/tokio_host.rs`](../ion-core/examples/tokio_host.rs)
for a runnable end-to-end example.

## Cargo.toml for Embedding
```toml
[dependencies]
ion-core = "0.2"  # includes derive + vm + optimize by default
# optional:
# ion-core = { version = "0.2", features = ["concurrency", "msgpack", "rewrite"] }
```

## Examples
- `ion-core/examples/embed.rs` — basic eval, inject, pipe, register_fn, error display
- `ion-core/examples/tokio_host.rs` — running Ion inside `#[tokio::main]` with
  tokio-backed async builtins (requires `concurrency` feature)
