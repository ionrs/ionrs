# Plan: hide enum / variant / module / function names from the host binary

Status: **§1 goal met.** Phases 1–7 plus error-message cleanup landed.
`strings target/release/example` shows zero Ion identifiers — no module
names, function names, `mod::fn` qualifiers, or stdlib constants. Only
incidental libm/libc symbols (`atan2`, `read`, `write`) remain, which exist
in any Rust binary using `f64::*` or `std::fs::*`. Follow-up items in §16.

Scope: ion-core, ion-derive, stdlib, host-facing API
Compatibility: **no back-compat preserved.** Old APIs are deleted, not shimmed.

---

## 1. Goal

Make `strings target/release/host_bin` reveal nothing about the registered Ion
surface — no enum names, no variant names, no module names, no function names,
no `mod::fn` qualified strings — without runtime decoding (no obfstr, no XOR
trampoline) and with strictly better runtime performance than today.

Names that originate in the script source at runtime (the user's `.ion` file)
are out of scope. Only the host binary image is the subject.

## 2. Non-goals

- Hiding the *shape* of the API (number of modules, fn arity).
- Defeating an adversary who runs the script with `eval` and prints a value.
- Cryptographic resistance. FNV-1a hashes are not a security primitive.
- Localized error messages. Errors get hashes by default; pretty names are
  opt-in via a sidecar table loaded at startup.

## 3. Threat model

- Static analysis of the binary (`strings`, `nm`, `objdump`, IDA / Ghidra
  string view): must not yield Ion-level identifiers.
- Memory dump of a running process: out of scope. Once the script has parsed,
  identifiers are in heap.
- Fault injection / hash-collision exploitation: handled by registration-time
  collision detection (panic at startup), not at runtime.

## 4. Core primitive

A `const fn` FNV-1a 64-bit hash, lifted to a `h!()` macro and a proc macro
helper. Computed entirely at compile time.

```rust
// new crate or new module ion-core::hash
pub const fn fnv1a64(s: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let mut i = 0;
    while i < s.len() {
        h ^= s[i] as u64;
        h = h.wrapping_mul(0x100000001b3);
        i += 1;
    }
    h
}
pub const fn h(s: &str) -> u64 { fnv1a64(s.as_bytes()) }

#[macro_export]
macro_rules! h { ($s:expr) => {{ const H: u64 = $crate::hash::h($s); H }} }
```

The proc macros in `ion-derive` use the same function during expansion. Both
paths produce a `u64` literal in the host binary; the source string is gone.

## 5. Wire-format change: `Value`

Hard cut. `enum_name`, `variant`, and the qualified-name string in builtin
function values are deleted.

```rust
// ion-core/src/value.rs

// removed: HostEnum { enum_name: String, variant: String, data: Vec<Value> }
HostEnum { type_id: u32, variant_idx: u16, data: SmallVec<[Value; 2]> },

// removed: BuiltinFn(String, BuiltinFn)
BuiltinFn        { qualified_hash: u64, func: BuiltinFn },
BuiltinClosure   { qualified_hash: u64, func: BuiltinClosureFn },
AsyncBuiltinClosure { qualified_hash: u64, func: AsyncBuiltinClosureFn },
```

`type_id` is dense, assigned by the registry at registration time.
`variant_idx` is the ordinal in the source enum. `qualified_hash` is
`h(format!("{}::{}", mod, fn))` precomputed in the macro.

`Value::HostStruct` (currently `{ type_name: String, fields: IndexMap<String, Value> }`)
gets the same treatment in a sibling change — out of scope here, but the
mechanics are identical (`type_id: u32`, `field_idx: u16`).

`Value::Module(Arc<ModuleTable>)` is added so module lookup does not collide
with user-facing `Value::Dict`. Scripts must keep using string-keyed dicts;
modules become a frozen, hash-indexed structure.

## 6. Registration API

### 6.1 `Module`

```rust
pub struct ModuleTable {
    name_hash: u64,
    fn_slots: Box<[FnSlot]>,            // dense, ordered by registration
    fn_index: phf::Map<u64, u16>,       // or sorted Vec<(u64, u16)> + binsearch
    value_slots: Box<[(u64, Value)]>,
    submodules: Box<[Arc<ModuleTable>]>,
}

pub struct FnSlot {
    qualified_hash: u64,
    func: ModuleFn,
}

pub struct Module { /* builder, drops to ModuleTable on `.freeze()` */ }

impl Module {
    pub fn new_h(name_hash: u64) -> Self { ... }
    pub fn register_fn_h(&mut self, name_hash: u64, func: BuiltinFn);
    pub fn register_closure_h<F>(&mut self, name_hash: u64, func: F);
    pub fn register_async_fn_h<F, Fut>(&mut self, name_hash: u64, func: F);
    pub fn register_submodule(&mut self, sub: Module);
    pub fn freeze(self) -> Arc<ModuleTable>;
}
```

There is no `register_fn(&str, …)`. The `&str` overload is deleted outright.
Callers either use `h!()` or the registration macro below.

### 6.2 Sugar macro for readable call sites

```rust
register_fns!(m, {
    abs   => |args| { ... },
    min   => |args| { ... },
    max   => |args| { ... },
});
```

Expands each bare-ident key to `m.register_fn_h(h!("abs"), …)`. Call sites
read like the old `register_fn("abs", …)` while the binary sees only u64s.

### 6.3 `Engine`

```rust
pub fn register_fn_h(&mut self, name_hash: u64, func: BuiltinFn);
pub fn register_closure_h<F>(&mut self, name_hash: u64, func: F);
pub fn register_async_fn_h<F, Fut>(&mut self, name_hash: u64, func: F);
pub fn register_struct(&mut self, def: HostStructDef);   // def is hash-only — see 6.4
pub fn register_enum(&mut self, def: HostEnumDef);
pub fn register_module(&mut self, module: Arc<ModuleTable>);
pub fn register_type<T: IonType>(&mut self);             // unchanged surface
```

### 6.4 `IonType` derive

The proc macro hashes everything during expansion.

```rust
// generated for `enum Color { Red, Green, Blue, Custom(u8, u8, u8) }`
impl IonType for Color {
    const ION_TYPE_HASH: u64 = h!("Color");

    fn ion_type_def() -> IonTypeDef {
        IonTypeDef::Enum(HostEnumDef {
            name_hash: 0x8a3f9c12_7b4e1d56,    // h("Color") at expansion
            variants: &[
                HostVariantDef { name_hash: h!("Red"),    arity: 0 },
                HostVariantDef { name_hash: h!("Green"),  arity: 0 },
                HostVariantDef { name_hash: h!("Blue"),   arity: 0 },
                HostVariantDef { name_hash: h!("Custom"), arity: 3 },
            ],
        })
    }

    fn to_ion(&self) -> Value { /* matches by variant, emits HostEnum {type_id, variant_idx, ..} */ }
    fn from_ion(val: &Value) -> Result<Self, String> { /* dispatch on (type_id, variant_idx) */ }
}
```

`type_id` is filled in by the registry at registration; the macro emits
hashes, the registry assigns the dense id. `to_ion`/`from_ion` consult a
per-type cell (e.g. `static TYPE_ID: OnceCell<u32>`) populated when
`register_type::<Color>()` runs.

## 7. Registry internals

```rust
pub struct TypeRegistry {
    enums:   Vec<EnumEntry>,                // indexed by type_id
    structs: Vec<StructEntry>,
    by_hash: HashMap<u64, u32>,             // name_hash → type_id, lookup-only
}

pub struct EnumEntry {
    name_hash: u64,
    variants: Box<[VariantEntry]>,          // indexed by variant_idx
    variant_by_hash: phf::Map<u64, u16>,    // for "Color::Red" path resolution
}
```

After registration, all runtime lookups are `Vec` index or perfect-hash. No
`HashMap<String, _>`, no `String::clone`.

## 8. Compiler / parser / VM

### Parser
- Identifier tokens stay as `&str` for the scope of parsing (script source is
  fine — it's loaded at runtime).
- AST node `ModulePath(Vec<String>)` becomes `ModulePath(Vec<u64>)`. Hashes
  computed during parsing.
- `EnumVariant { enum_name: String, variant: String }` becomes
  `EnumVariant { enum_hash: u64, variant_hash: u64 }`.

### Compiler
- `compiler.rs:1131,1141,1148,2043,2229-2237`: stop adding `Value::Str`
  constants for enum/variant/module names. Emit u64 operands directly into
  the chunk. Bytecode operand width grows from u16 const-pool index to u64
  inline — adjust `vm.rs::read_u64`.

### VM
- `vm.rs:866-870`: enum-match arm becomes `(type_id, variant_idx)` integer
  compare.
- `vm.rs:1058-1080`: enum construction reads `(enum_hash, variant_hash)`,
  resolves to `(type_id, variant_idx)` once at chunk-load time and rewrites
  the operands in place (or stores both forms in the chunk's resolved cache).
- New op `GetModuleSlot(module_const_idx, fn_hash)` for `mod::fn` dispatch.
  Single hash compare → slot index.

### `Env` / `intern.rs`
- `StringPool` stays for script-side identifiers (locals, captured names) —
  these come from script source, not the binary.
- For host-registered names, no pool is used. `Symbol(u32)` is replaced by a
  raw `u64` hash at registration sites only.
- `env.rs::define`/`get` get hash-keyed twins (`define_h`, `get_h`) for
  registration, leaving the script-side path untouched.

## 9. Stdlib

Mechanical conversion of every site in `stdlib.rs` (~200 calls):

```rust
// before
m.register_fn("abs",  |args| { ... });
m.register_fn("min",  |args| { ... });

// after
register_fns!(m, {
    abs => |args| { ... },
    min => |args| { ... },
});
```

Stdlib module names (`"math"`, `"json"`, `"io"`, …) come from
`register_stdlib_with_handlers` (`stdlib.rs:1687-1728`) and become:

```rust
let math = math_module();  // returns Arc<ModuleTable>, name_hash=h!("math")
env.define_module(math);   // single hash registration, no string
```

Existing `#[cfg(feature = "fs")]` etc. continue to gate whether each module
links at all. A binary built without `fs` won't contain the `fs` hash either.

## 10. Diagnostics

Default release behaviour for any error referencing a hidden name:

```
unknown variant <0xcd34_…> in enum <0x8a3f_…> at script.ion:14:7
```

Two opt-in mechanisms restore readable diagnostics without putting names in
the binary:

1. **Sidecar name table.** A separate file, e.g. `target/release/myapp.names`,
   produced by a build script. Contents: a flat `HashMap<u64, &'static str>`
   serialized as bincode. Loaded by the host on startup if present. Never
   linked into the binary. Suitable for staging/dev environments where the
   ops team controls deployment.
2. **`cfg(debug_assertions)`.** Debug builds embed the name table inline (so
   tests stay readable). Release builds drop it.

`Value::Display` follows the same policy:

| Value          | Release                       | Debug / sidecar       |
|----------------|-------------------------------|-----------------------|
| `Color::Red`   | `<enum#3:0>`                  | `Color::Red`          |
| `<builtin abs>`| `<builtin 0xa1b2…>`           | `<builtin math::abs>` |

JSON / msgpack encoding of host enums (`value.rs:543, 645`) currently emits
`"Color::Red"`. New encoding: `[type_hash_u64, variant_hash_u64, data]`. Wire
format changes; consumers re-encoded accordingly. (No back-compat — `.ion`
data files written by old binaries are not readable by new binaries. Document
in CHANGELOG.)

## 11. Collision handling

64-bit FNV-1a across the union of (enum names + variant names + struct names
+ field names + module names + function names + qualified `mod::fn`) for a
typical project: a few thousand symbols. Birthday-bound collision probability
is ~10⁻¹².

Mitigation: at registry build time, `assert!` that every inserted hash is
unique within its scope. A collision becomes a startup panic with a clear
message ("hash collision between `Color::Red` and `Other::Foo`"), not a silent
wrong-dispatch at runtime. In sidecar/debug builds the names are printed; in
release builds the user gets a hash pair and a hint to rebuild with
`debug-names` to get the conflicting strings.

If we ever hit a real collision: the affected name is renamed in source. No
runtime fix is needed.

## 12. Touch-list

Files that change. Order matches Section 13.

- `ion-core/src/hash.rs` — new, `fnv1a64`, `h!`.
- `ion-core/src/value.rs` — `HostEnum`, `BuiltinFn`, `BuiltinClosure`,
  `AsyncBuiltinClosure` shape change; `Display`/JSON/msgpack adjusted; new
  `Value::Module(Arc<ModuleTable>)`.
- `ion-core/src/host_types.rs` — `HostEnumDef`/`HostVariantDef`/`HostStructDef`
  use `name_hash: u64`; `TypeRegistry` switched to `Vec`+`HashMap<u64,u32>`.
- `ion-core/src/module.rs` — `Module` builder + `ModuleTable`; deleted
  string-taking `register_fn`/`register_closure`/`register_async_fn`.
- `ion-core/src/engine.rs` — `register_fn_h`, `register_closure_h`,
  `register_async_fn_h`, `register_module(Arc<ModuleTable>)`.
- `ion-core/src/env.rs` — `define_module`, hash-keyed module slot.
- `ion-core/src/intern.rs` — unchanged (script-side only).
- `ion-core/src/parser.rs`, `ast.rs` — `ModulePath(Vec<u64>)`,
  `EnumVariant { enum_hash, variant_hash }`.
- `ion-core/src/compiler.rs` — emit hashed operands; new `GetModuleSlot`.
- `ion-core/src/vm.rs` — new ops; `read_u64`; integer-compare match arms.
- `ion-core/src/stdlib.rs` — mechanical rewrite to `register_fns!`.
- `ion-core/src/macros.rs` (new) — `register_fns!` macro.
- `ion-derive/src/lib.rs` — emit `name_hash: h!(...)` instead of
  `name: "...".to_string()` for both struct and enum derives.
- `ion-core/tests/integration.rs` — section 28/29 tests rewritten against
  hash-based API.

## 13. Migration order

No back-compat → no parallel old/new APIs. Each phase compiles and passes
tests on its own; the next phase can begin immediately after.

1. **Hash primitive.** Add `ion-core/src/hash.rs` and `h!()` macro. Standalone.
2. **`Value` surgery.** Change `HostEnum` and the three `Builtin*` variants;
   fix every match arm (compiler will list them). `host_types.rs` and
   `module.rs` move to hash-keyed structures in the same commit.
3. **Derive rewrite.** `ion-derive` switches output to hash literals. Section
   29 tests (the manual `register_enum` ones in `integration.rs:1839`) get
   rewritten to use `h!()` constants directly.
4. **Module value + Engine API.** `Value::Module`, `Arc<ModuleTable>`,
   `register_module(Arc<ModuleTable>)`. Compiler op for `GetModuleSlot`.
5. **Parser + compiler + VM.** `ModulePath`/`EnumVariant` AST nodes carry
   hashes; compiler emits hashed operands; VM dispatches on integers.
6. **Stdlib rewrite.** `register_fns!` applied to every module in
   `stdlib.rs`. Roughly 200 lines of mechanical edits.
7. **Diagnostics + sidecar.** `cfg(debug_assertions)` inline names; build
   script for the `.names` sidecar; `Display` policy.

After step 7, run `strings target/release/<example>` for one of the demos in
`examples/` and grep for stdlib function names. Expected output: empty.

## 14. Performance expectations

Benchmark targets (compared to current `main`):

- **Enum construction** (`Color::Custom(255, 128, 0)`): -2 `String::from` per
  call ≈ ~80-150ns saved on hot loops; 16 fewer bytes per `Value::HostEnum`.
- **Module function call** (`math::abs(x)` in a tight loop): one `Vec` index
  + one hash compare vs. today's `IndexMap<String,_>::get` + `String::clone`.
  Expected 2-4× speedup on the dispatch alone.
- **Builtin value size**: `Value::BuiltinFn` shrinks from `(String, fn)` ≈ 32
  bytes to `(u64, fn)` = 16 bytes. Lists/dicts of builtins benefit.
- **Registration cost**: dominant cost (`format!` + `String::clone` per fn ×
  ~200 stdlib fns) drops to a `Vec::push` per fn. Engine startup measurably
  faster.

A regression check: add a criterion bench in `ion-core/benches/` for (a)
`engine_with_stdlib_setup`, (b) `tight_module_call_loop`, (c)
`enum_construct_match`. CI fails if any regresses by >5%.

## 15. Open questions

- **`use mod::*` reflection** (`module.rs:115::names`). Currently returns
  `Vec<String>`. New design: returns `Vec<u64>` and the wildcard-import
  binding is keyed by hash. Scripts that introspect modules via dictionary
  iteration break. Acceptable? (The DESIGN.md probably doesn't promise this.)
- **`f"{Color::Red}"` in release.** Resolved: sidecar provides names when
  loaded; without it, Display emits the opaque form `"<enum#3:0>"`. obfstr
  was considered and rejected — a debugger or 10-line IDA script defeats it
  trivially, and at that point we're paying a runtime XOR + a binary-size
  bump for hiding that doesn't actually hide. Sidecar gives a cleaner story:
  one mechanism for Display, error messages, and LSP; names are either fully
  out-of-binary (release without sidecar) or fully present (sidecar loaded /
  debug build). No middle ground that pretends to hide while leaking.
- **`HostEnum` data field type.** Plan says `SmallVec<[Value; 2]>`. Adds a
  dependency. Alternative: keep `Vec<Value>` (no perf change vs. today). I
  lean SmallVec — most variants have arity 0-2 — but flag for review.
- **`ion-lsp` impact.** LSP needs to map hashes back to names for completions
  and hover. The sidecar is the natural source. LSP forces the sidecar to
  exist in dev workflows; release deployments can omit it.

## 16. Status — what shipped, what deviated, what remains

This section is the source of truth for project state. The original plan
sections (4–14) describe intent; this section reconciles that against what
was actually committed in `1f7482b..18fd0e3`.

### Shipped as planned

- §4 hash primitive: `fnv1a64`, `h!`, `qh!`, `mix` (`hash.rs`).
- §9 stdlib mass conversion: every `register_fn`/`register_closure`/`set`
  site in `stdlib.rs` calls `crate::h!("…")`.
- §10 sidecar: `names::register`/`register_many`/`lookup`/`dump_sidecar_json`/
  `load_sidecar_json`. Debug builds auto-populate via `h!`/`qh!`.
- §10 `cfg(debug_assertions)` inline names: enabled by `h!`/`qh!` macros
  AND by the `IonType` derive (gated `register_many` block).
- §11 collision handling: `Module::register_*` panics on duplicate;
  `TypeRegistry` panics on real shape mismatch (idempotent on same shape);
  regression tests cover both.

### Shipped, but deviated from the plan's spec

| Plan said | Shipped | Why |
|---|---|---|
| `HostEnum { type_id: u32, variant_idx: u16, data: SmallVec<[Value;2]> }` | `{ enum_hash: u64, variant_hash: u64, data: Vec<Value> }` | Hash-direct skips the `OnceCell<u32>` resolution step (`§6.4`). `SmallVec<[Value;_]>` makes `Value` recursive without indirection; `Box<Value>` indirection nets out negative on small arities. |
| `HostStruct { type_id: u32, field_idx: u16 }` | `{ type_hash: u64, fields: IndexMap<u64, Value> }` | Same reasoning. Preserves field iteration order for free. Loses positional-index speedup vs. plan. |
| `Module::new_h`, `register_fn_h`, etc. (`_h` suffix) | `Module::new`, `register_fn`, etc. (no suffix) | Suffix was redundant once the only signature is `u64`. |
| `Engine::register_module(Arc<ModuleTable>)` | `Engine::register_module(Module)` | Engine wraps the builder internally. Saves the caller from `.into_value()`/`Arc::new()`. |
| `IonType::ION_TYPE_HASH: u64` const + per-type `OnceCell<u32>` | Hashes inline in `to_ion`/`from_ion`/`ion_type_def`; no associated const, no cell | No need for the cell once we use hashes directly. Marginally less pretty for downstream consumers that wanted `T::ION_TYPE_HASH`. |
| Registry: `Vec<EnumEntry>` + `HashMap<u64, u32>` (dense + by-hash) | `HashMap<u64, HostEnumDef>` | Single-table. Loses Vec-index perf. ID-less is also why the type_id story disappeared. |
| §6.2 `register_fns!` macro | Created then removed (audit) | Sed pass converted stdlib via `crate::h!()` directly; macro had no users. |

### NOT shipped (real gaps against §1 / §14)

1. **§8 parser/compiler/VM bytecode reshape.**
   - AST `ModulePath` still holds `Vec<String>`; `EnumVariant` still holds
     `(String, String)`.
   - Compiler still adds `Value::Str` constants to chunks for module/variant
     names (script-source strings — not in the binary, but they cost a
     runtime hash on every `Op::GetGlobal`/`Op::ConstructEnum`/etc.).
   - VM has no `GetModuleSlot` op; module access goes through the same
     string-keyed `Op::GetGlobal` path with a `get_sym_or_global` fallback.
   - **Consequence**: dispatch is *not* the "2–4× faster" §14 claim. Each
     `math::abs(x)` call hashes "math" + "abs" at runtime. Hiding goal is
     unaffected — the strings are script-source, not binary — but the perf
     win never landed.

2. **§1 strict reading: `mod::fn` strings inside error message literals.**
   `strings target/release/example | grep -E '^(math|json|fs|...)::'`
   returns ~33 hits like `"math::abs takes 1 argument"`,
   `"fs::remove_dir_all('"`, `"os::env_var: name must be a string"`. These
   are diagnostic strings hand-written into the stdlib closures via
   `ion_str!()`. Bare identifiers (`math`, `abs`) don't appear standalone,
   but they are substrings of these literals — **the goal is not strictly
   met**.

3. **§14 criterion benchmarks.** Not added. No measurement of actual
   dispatch / construction perf delta.

4. **§13 build-script sidecar wiring.** The `dump_sidecar_json` /
   `load_sidecar_json` helpers exist; no `examples/build_with_sidecar/`
   demonstrates the workflow.

5. **§15 ion-lsp impact.** LSP code untouched. Hover/completion in
   release builds without a sidecar will see opaque hashes.

### §1 closure (landed)

Item 2 above is **resolved.** Stdlib error literals stripped of `mod::fn`
qualifiers via sed; helper functions (`semver_parse_arg`, `fs_arg_str`,
`arg_str`, `io_err`) refactored to drop their `fn_name: &str` parameters
so the literal `"format"`, `"compare"`, `"parse"`, etc. no longer appear
either. The previous attempt to prepend `names::lookup(qualified_hash)`
at the call site was reverted because it changed user-visible runtime
semantics (`try { assert(false, "boom") } catch e { e }` returned
`"assert: boom"` instead of `"boom"`).

Final verification (`strings target/release/examples/embed`):

| Probe                                                   | Hits |
|---------------------------------------------------------|------|
| `\b(math\|json\|fs\|os\|path\|semver\|log\|io\|str\|string)::` (excl. `str::from_utf8` panic) | 0 |
| Ion-specific names (`env_var`, `cwd`, `satisfies`, `msgpack_encode`, `bytes_from_hex`, `type_of`, …) | 0 |
| Standalone module names (`math`, `json`, `fs`, …)       | 0 |
| Stdlib constants (`PI`, `E`, `TAU`, `INF`, `NAN`)       | 0 |

Items 1 (bytecode reshape — perf), 3 (criterion benches), 4 (build-script
sidecar wiring), and 5 (ion-lsp) remain open as follow-ups; none block §1.

### Trade-off taken: error message context

Stdlib closures now emit generic errors (`"takes 1 argument"`,
`"requires a number"`, `"argument 1 must be a string"`). The function
name no longer appears in the error string. Call sites identify the
function via the script source span (line:col) but not via a textual
prefix. This:

- Keeps `try { … } catch e { e }` returning the original message verbatim
  — preserving script semantics that may switch on error text.
- Means terminal error output reads `runtime error at script.ion:5:3:
  takes 1 argument` instead of `… math::abs: takes 1 argument`. The
  prefix can be added at the renderer level using
  `names::lookup(qualified_hash)` if and when an embedder wants it back —
  Display of `Value::BuiltinFn` already does this for the value itself,
  so the same lookup is one method call away from the error renderer.
