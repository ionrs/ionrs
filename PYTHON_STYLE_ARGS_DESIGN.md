# Python-Style Argument Calling

Status: implemented in `ion-core`.
Scope: parser, AST, resolver, tree-walk interpreter, VM, async continuation runtime, host registration APIs, and focused tests.

## Goal

Ion now supports Python-style argument binding for Ion functions and host callables while keeping Ion syntax:

- Positional arguments: `f(1, 2, 3)`
- Defaults: `fn f(x = 10) { ... }`
- Keyword arguments: `f(x: 1, y: 2)`
- Variadic positional params and call spreads: `fn f(*args) { ... }`, `f(*items)`
- Variadic keyword params and call spreads: `fn f(**kwargs) { ... }`, `f(**opts)`
- Positional-only and keyword-only markers: `fn f(a, /, b, *, c) { ... }`

This applies to:

- `Value::Fn`
- `Value::BuiltinFn`
- `Value::BuiltinClosure`
- `Value::AsyncBuiltinClosure`

No `ion-derive` sugar is included here. Deriving host signatures from Rust function signatures remains a follow-up.

## Public Semantics

Definition examples:

```ion
fn collect(a, *rest, b = 10, **opts) {
    [a, rest, b, opts]
}

fn shaped(a, /, b, *, c) {
    a + b + c
}
```

Call examples:

```ion
collect(1, 2, 3, b: 4, **#{flag: true})
shaped(1, b: 2, c: 3)
```

Ordering is enforced at parse time: positional arguments and `*expr` spreads must come before keyword arguments and `**expr` spreads.

`*expr` must evaluate to `Value::List`. `**expr` must evaluate to `Value::Dict`, whose concrete representation is `IndexMap<String, Value>`.

Method calls accept positional arguments and `*expr` list spreads. Keyword arguments and `**expr` spreads are rejected for methods before method dispatch.

## AST Changes

`Param` now carries a parameter kind:

```rust
pub struct Param {
    pub name: String,
    pub default: Option<Expr>,
    pub kind: ParamKind,
}

pub enum ParamKind {
    Positional,
    PositionalOnly,
    KeywordOnly,
    VarArgs,
    VarKwargs,
}
```

Call arguments are represented as a value plus a kind:

```rust
pub struct CallArg {
    pub kind: CallArgKind,
    pub value: Expr,
}

pub enum CallArgKind {
    Positional,
    Named(String),
    SpreadPos,
    SpreadKw,
}
```

Helper constructors (`positional`, `named`, `spread_pos`, `spread_kw`) keep call-site code compact.

## Host Signatures

Host callables keep their existing Rust callback shape. Signature-aware registration only changes how Ion resolves arguments before invoking the callback.

```rust
pub struct HostSignature {
    pub params: Vec<HostParam>,
    pub has_var_args: bool,
    pub has_var_kwargs: bool,
}

pub struct HostParam {
    pub name_hash: u64,
    pub kind: ParamKind,
    pub default: Option<Value>,
}
```

Host parameter names are stored as `u64` hashes, not strings. This matches the existing host-name hiding model and avoids introducing host API names into release binaries. Debug builds can recover registered names through the name sidecar; release diagnostics fall back to an opaque hash.

The builder API:

```rust
let sig = HostSignature::builder()
    .pos_required(h!("name"))
    .pos(h!("greeting"), Value::Str("Hello".into()))
    .var_args(h!("rest"))
    .kw_only_required(h!("ctx"))
    .kw_only(h!("loud"), Value::Bool(false))
    .var_kwargs(h!("opts"))
    .build();
```

Builder methods reject duplicate host parameter hashes and duplicate variadic slots by panicking at registration/setup time.

`HostArgs<'a>` is available for name-based host access over the resolved slot layout:

```rust
let args = HostArgs::new(values, signature);
let name = args.get_str(h!("name"))?;
```

## Registration APIs

Existing positional-only host APIs remain:

```rust
engine.register_fn(h!("legacy"), legacy_fn);
engine.register_closure(h!("legacy_closure"), move |args| { ... });
engine.register_async_fn(h!("legacy_async"), |args| async move { ... });
```

Signature-aware APIs are added on both `Engine` and `Module`:

```rust
engine.register_fn_sig(h!("mix"), sig, mix_fn);
engine.register_closure_sig(h!("query"), sig, move |args| { ... });
engine.register_async_fn_sig(h!("fetch"), sig, |args| async move { ... });

module.register_fn_sig(h!("mix"), sig, mix_fn);
module.register_closure_sig(h!("query"), sig, move |args| { ... });
module.register_async_fn_sig(h!("fetch"), sig, |args| async move { ... });
```

All host `Value` variants now carry `signature: Option<Arc<HostSignature>>`.

- `None`: legacy host callable. Positional calls behave as before; keyword calls are rejected.
- `Some`: the resolver produces a slot-indexed argument vector before the callback runs.

## Resolver

Shared resolver code lives in `ion-core/src/call.rs`.

Main entry points:

- `resolve_ion_slots(params, function_name, positional, named, line, col)`
- `resolve_host_call(signature, positional, named, line, col)`
- `keyword_args_from_dict(map)`

Ion and host signatures use a shared positional binding path through a `ParamView` abstraction. Ion keyword matching uses exact script parameter strings. Host keyword matching uses `h(source_name)` against the stored host parameter hash.

Resolution order:

1. Bind positional values left-to-right until `*args`, keyword-only, or `**kwargs`.
2. Bind keyword values to eligible params.
3. Store unmatched keyword values in `**kwargs` if present.
4. Error on duplicates, unknown keywords without `**kwargs`, or too many positionals without `*args`.
5. Finalize defaults in declaration order.

Ion defaults are evaluated lazily, left-to-right, after prior params have been bound into the evaluator environment. This preserves existing behavior:

```ion
fn f(a, b = a) { b }
f(7)  # 7
```

Host defaults are eager `Value` clones supplied in `HostSignature`.

## Bytecode And Compiler

Fast paths remain intact:

- Pure positional calls use `Call` or `TailCall`.
- Named calls without spreads use `CallNamed` or `TailCallNamed`.

Calls involving spreads use the resolved-call path:

- `CallResolved`
- `TailCallResolved`
- `SpawnCallResolved`
- `MethodCallResolved`
- `KwInsert`
- `KwMerge`

The compiler lowers:

```ion
f(*items, c: 3, **opts)
```

into:

1. Build a positional `Value::List`, using `ListAppend` and `ListExtend`.
2. Build a keyword-pair `Value::List`, using `KwInsert` and `KwMerge`.
3. Emit `CallResolved`.

Keyword pair entries are stored as `Value::Tuple(vec![Value::Str(name), value])`.

Method calls with `*expr` use the same positional-list lowering and emit `MethodCallResolved`; any keyword pairs on that path produce `methods do not support keyword arguments`.

The synchronous VM decodes the new opcodes and rejects async-only `SpawnCallResolved` with the same error shape as other spawn opcodes.

## Runtime Paths

Tree-walk interpreter:

- Evaluates `CallArgKind` into `Vec<Value>` plus `Vec<KeywordArg>`.
- Calls the shared resolver for Ion and signed host callables.
- Rejects keyword calls to legacy host callables.

Synchronous VM:

- `CallNamed` now preserves keyword metadata for all callable kinds.
- `CallResolved` consumes the prebuilt positional and keyword lists.
- `MethodCallResolved` expands method `*expr` spreads and rejects method keywords.
- Ion named/resolved calls preserve bytecode tail-call behavior.

Async continuation runtime:

- Handles `CallResolved`, `TailCallResolved`, `MethodCallResolved`, `SpawnCallResolved`, `KwInsert`, and `KwMerge`.
- Resolves signed sync host, async host, and Ion calls through the same resolver.
- Resolves signatures for `EngineHandle::call_async` callbacks into signed host callables.
- The historical async tree-walk bridge also supports keyword and spread calls for parity.

## Compatibility

Intentional behavior changes:

| Surface | Before | Now |
|---|---|---|
| Legacy host callable with keywords | names could be dropped path-dependently | runtime error |
| Extra positional args to Ion functions | ignored in some paths | runtime error |
| Duplicate bindings, e.g. `f(1, a: 2)` | overwritten or path-dependent | runtime error |
| Duplicate Ion parameter names | accepted and overwritten | parse error |
| Method call with keyword args | accepted path-dependently | runtime error |
| Positional-only param called by name | not expressible | runtime error |
| `*expr` with non-list | not expressible | type error |
| `**expr` with non-dict | not expressible | type error |

No migration flag was added. The stricter behavior is the default and is covered by integration and cross-validation tests.

## Tests Added

Coverage includes:

- Ion `*args`
- Ion `**kwargs`
- Keyword-only params
- Positional-only params
- `*` and `**` call spreads
- Duplicate binding errors
- Duplicate parameter errors
- Too-many-positional errors
- Method `*` spreads and method keyword rejection
- Unsigned host keyword rejection
- Engine-level signed host calls
- Module-level signed host calls
- VM resolved-call opcodes
- Named/resolved VM entry preserving tail calls, including deep recursion
- Async signed host keyword calls
- External async callbacks into signed host callables
- Async Ion keyword-only calls

## Verification

These passed after implementation:

```sh
cargo check -p ion-core
cargo check -p ion-core --features async-runtime
cargo test -p ion-core
cargo test -p ion-core --features async-runtime
```

The full default run covered unit tests, integration tests, VM tests, cross-validation tests, and doc tests. The async run covered unit tests, async-runtime tests, async integration tests, and doc tests.

## Follow-Ups

- Add `ion-derive` sugar for deriving `HostSignature` from Rust functions.
- Consider dynamic host defaults (`fn() -> Value`) only if a real use case appears.
