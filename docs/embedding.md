# Ion Embedding API

> **0.9.0 breaking change.** Host-registered identifiers (function names,
> module names, struct/field names, enum/variant names) are folded to
> `u64` FNV-1a hashes at compile time via the `h!()` macro. The literal
> source text never appears in the release binary's `.rodata`. See
> [`hide-names.md`](hide-names.md) for the concise overview.
> Migration: replace each `register_fn("name", …)` with
> `register_fn(h!("name"), …)`.

## Engine (`ion-core/src/engine.rs`)
Primary public API for embedding Ion in Rust applications.

```rust
use ion_core::{h, engine::Engine};
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
engine.register_fn(h!("square"), |args| { ... });

// Register closures — can capture host state like database pools,
// shared counters, etc.
let counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
let counter_c = counter.clone();
engine.register_closure(h!("tick"), move |_args| {
    counter_c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(Value::Unit)
});

// Register native async host functions (feature: async-runtime).
engine.register_async_fn(h!("later"), |args| async move {
    let ms = args.first().and_then(Value::as_int).unwrap_or(1);
    tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
    Ok(Value::Int(ms))
});

// Register modules (namespaced functions/values)
let mut math = Module::new(h!("math"));
math.register_fn(h!("add"), |args| { ... });
math.set(h!("PI"), Value::Float(std::f64::consts::PI));
engine.register_module(math);              // scripts use math::add() or `use math::*;`

// Modules can also expose native async host functions (feature: async-runtime).
let mut sensor = Module::new(h!("sensor"));
sensor.register_async_fn(h!("call"), |args| async move {
    // async host I/O
    Ok(Value::Int(args.len() as i64))
});
engine.register_module(sensor);            // scripts use sensor::call(...) under eval_async

// Register host types
engine.register_type::<Player>();          // via #[derive(IonType)]
engine.register_struct(def);               // manual HostStructDef (uses h!() for name + fields)
engine.register_enum(def);                 // manual HostEnumDef    (uses h!() for name + variants)

// Execution limits
engine.set_limits(Limits { max_depth: 100, max_iterations: 10000 });
```

## Hash-based registration: `h!()` and `qh!()`

The `ion_core::h!("foo")` macro is `const`-evaluated FNV-1a 64-bit:
the literal `"foo"` is consumed at compile time, leaving only the `u64`
in the emitted binary. Pair it with `Module::new`, `register_fn`, etc.

```rust
use ion_core::h;
use ion_core::host_types::{HostStructDef, HostEnumDef, HostVariantDef};

engine.register_struct(HostStructDef {
    name_hash: h!("Player"),
    fields:    vec![h!("name"), h!("score")],
});

engine.register_enum(HostEnumDef {
    name_hash: h!("Color"),
    variants: vec![
        HostVariantDef { name_hash: h!("Red"),    arity: 0 },
        HostVariantDef { name_hash: h!("Custom"), arity: 3 },
    ],
});
```

`qh!("module", "fn")` exists for places that need the qualified hash
without joining the strings: `mix(h("module"), h("fn"))` precomputed.

In **debug builds** (`cfg(debug_assertions)`), each `h!()` site
auto-registers `(hash, "literal")` with `ion_core::names` exactly once
on first execution, via a per-site `Once`. Tests, dev binaries, and
`cargo run` get readable diagnostics with no extra setup. In release,
the registration block is `cfg`'d out entirely and neither the literal
nor the registration call lands in `.rodata`.

## Diagnostics: the `names` registry and sidecar workflow

`ion_core::names` is an optional runtime hash → name mapping consulted
by `Display`, `to_json`, and error rendering. Three ways to populate it:

1. **Debug builds** — automatic via the `h!()`/`qh!()` macros above.
2. **Release with hand-built table:**
   ```rust
   ion_core::names::register_many([
       (h!("Player"), "Player"),
       (h!("score"),  "score"),
   ]);
   ```
3. **Release with sidecar JSON:**
   ```rust
   let json = std::fs::read_to_string("myapp.names")?;
   ion_core::names::load_sidecar_json(&json)?;
   ```
   Generate the sidecar from a fully-populated debug build via
   `names::dump_sidecar_json()` in a `build.rs` or one-off binary.

Without any of these, `Display` of a `Value::HostEnum` renders as
`<enum#hhhh>::<v#hhhh>`, and stdlib errors look like
`runtime error at script.ion:5:3: takes 1 argument` — readable enough
to find the source location, opaque about Ion-level names.

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
| `register_closure` | `impl Fn(&[Value]) -> Result<Value, String> + Send + Sync + 'static` | Need to capture a DB pool, an `Arc<Mutex<State>>`, counters, etc. Runs synchronously. |
| `register_async_fn` | `impl Fn(Vec<Value>) -> impl Future<Output = Result<Value, IonError>> + 'static` | Native Tokio async host work under `eval_async`. |

Sync callbacks appear to Ion scripts as `builtin_fn`; async callbacks are
called with the same Ion syntax but require `eval_async`. Calling an async host
function through sync `eval` produces an explicit runtime error.

`Module` mirrors the same callback split: `Module::register_fn`,
`Module::register_closure`, and, with `async-runtime`, `Module::register_async_fn`.
Async module callbacks are useful for namespaced Tokio-backed APIs such as
`sensor::call(...)`, `sensor::upload(...)`, or `net::http::get(...)`.

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

## Embedding inside a Tokio application

For native async I/O, enable `async-runtime` and await `Engine::eval_async`.
Ion parks on Tokio futures instead of blocking an OS thread:

```rust,no_run
use ion_core::{Engine, Value};
use ion_core::error::IonError;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), IonError> {
    let mut engine = Engine::new();
    engine.register_async_fn("fetch", |args| async move {
        let path = args[0].as_str().unwrap_or("").to_string();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        Ok(Value::Str(path))
    });

    let value = engine.eval_async(r#"
        async {
            let a = spawn fetch("/a");
            let b = spawn fetch("/b");
            [a.await, b.await]
        }
    "#).await?;

    println!("{value}");
    Ok(())
}
```

See [`docs/concurrency.md`](concurrency.md#embedding-inside-a-tokio-host)
for the full model. `engine.eval(...)` remains synchronous and is still
appropriate for purely synchronous hosts.

## Cargo.toml for Embedding
```toml
[dependencies]
ion-core = "0.3"  # includes derive + optimized vm by default
# optional:
# ion-core = { version = "0.3", features = ["async-runtime", "msgpack", "rewrite"] }
```

## Examples
- `ion-core/examples/embed.rs` — basic eval, inject, pipe, register_fn, error display
- `ion-core/examples/tokio_host.rs` — running Ion inside `#[tokio::main]` with
  native async host functions (requires `async-runtime` feature)
