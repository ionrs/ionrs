# Ion Embedding API

## Engine (`ion-core/src/engine.rs`)
Primary public API for embedding Ion in Rust applications.

```rust
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

// Register Rust functions
engine.register_fn("square", |args| { ... });

// Register host types
engine.register_type::<Player>();          // via #[derive(IonType)]
engine.register_struct(def);               // manual HostStructDef
engine.register_enum(def);                 // manual HostEnumDef

// Execution limits
engine.set_limits(Limits { max_depth: 100, max_iterations: 10000 });
```

## Host Types (`#[derive(IonType)]`)
- Proc macro in `ion-derive/`
- Generates `to_ion()` / `from_ion()` via serde
- Works for structs and enums
- Scripts can construct, access fields, pattern match

## Cargo.toml for Embedding
```toml
[dependencies]
ion-core = "0.1"  # includes derive + vm + optimize by default
```

## Example
See `ion-core/examples/embed.rs` — demonstrates eval, inject, pipe, register_fn, error display.
