# Hiding Host-Registered Names

Ion release builds keep host-registered identifiers out of the binary image:
module names, function names, struct and field names, enum and variant names,
and qualified `mod::fn` paths.

The goal is that `strings target/release/host_bin` does not reveal the
registered Ion API surface. This protects against static binary inspection; it
does not protect memory dumps, runtime introspection, script-source names, or
API-shape analysis.

## How It Works

- Names are folded to 64-bit FNV-1a hashes at compile time with `h!()`.
- Module member display hashes use `hash::mix(module_hash, member_hash)`.
- Host values store hashes, not strings, including host structs, enums,
  builtin functions, builtin closures, and async builtin closures.
- `Module` and `Env` keep host registrations in hash-keyed tables.
- `#[derive(IonType)]` hashes Rust type, field, and variant names during macro
  expansion.
- Release diagnostics use generic text; debug builds keep readable messages.

## Embedding API

Use `h!()` for every host registration:

```rust
use ion_core::{h, engine::Engine, module::Module};
use ion_core::value::Value;

let mut engine = Engine::new();
engine.register_fn(h!("square"), |args| {
    Ok(Value::Int(args[0].as_int().unwrap() * args[0].as_int().unwrap()))
});

let mut math = Module::new(h!("math"));
math.register_fn(h!("add"), |args| {
    Ok(Value::Int(args[0].as_int().unwrap() + args[1].as_int().unwrap()))
});
math.set(h!("PI"), Value::Float(std::f64::consts::PI));
engine.register_module(math);
```

String-taking registration APIs were removed because they force identifier
literals into `.rodata`.

## Diagnostics and Names

Debug builds auto-populate `ion_core::names` from `h!()` sites, so `Display`
and diagnostics can render readable names while developing.

Release builds start with an empty name table. Embedders that want readable
names in staging or production can load a sidecar:

```rust
let json = std::fs::read_to_string("myapp.names")?;
ion_core::names::load_sidecar_json(&json)?;
```

Without a sidecar, display falls back to opaque forms such as
`<builtin #0123456789abcdef>` or `<enum#0123456789abcdef>::<v#...>`.

## Collision Handling

FNV-1a is not cryptographic. Collisions are treated as startup-time programming
errors.

- `Module::register_*` panics on duplicate or colliding entries.
- `TypeRegistry` permits identical re-registration but panics if the same type
  hash is registered with a different shape.

## Stdlib Docs

The stdlib documentation manifest is behind the `embedded-stdlib-docs` feature.
It is off by default so embedders do not accidentally ship the full stdlib
surface as JSON. `ion-lsp` enables it for hover and completion data.

## Verification

For release binaries, use targeted probes after stripping:

```bash
cargo build --release -p ion-core --example embed
cp target/release/examples/embed /tmp/ion_embed_stripped
strip /tmp/ion_embed_stripped
strings /tmp/ion_embed_stripped | rg 'math::|json::|bytes_from_hex|type_of'
```

Expected result: no Ion-level identifier hits. Incidental libc, libm, Rust
standard library, dependency, file-path, or host-script-example strings are not
part of this guarantee.

## Caveats

- Script-source identifiers are runtime input and remain visible after parsing.
- LSP/editor tooling relies on docs and sidecar-style name data.
- The VM still hashes some script-source names at runtime; this affects
  performance, not the binary-hiding guarantee.
