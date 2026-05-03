# Ion Language Design Specification

A small, strongly-typed, embeddable scripting language implemented in Rust.

---

## 0. Design Principles — No Surprises, No Hidden Control

1. **Explicit over implicit** — No implicit type coercions, no implicit returns from ambiguous syntax, no hidden side effects.
2. **One syntax, one meaning** — `#{ }` is always a dict. `{ }` is always a block. `.field` works on dicts and host structs uniformly.
3. **No hidden panics** — Every value extraction is visible: `unwrap`, `unwrap_or`, `expect`, `?`, `match`, `if let`.
4. **Immutable by default** — `let` bindings and all collection methods return new values. Mutation requires explicit `let mut` and reassignment.
5. **Errors are values** — No exceptions. `Result` and `Option` are the only error paths. `?` is the only propagation mechanism.
6. **What you see is what runs** — No operator overloading, no implicit conversions, no magic methods, no inheritance. The behavior of code is determined by reading it top-to-bottom.
7. **Host boundary is clear** — Structs/enums come from Rust. Scripts use dicts for ad-hoc data. You always know which is which.

---

## 1. Type System — Strong, Optionally Annotated

The interpreter tracks and enforces types at runtime. No implicit coercions — `1 + "2"` is a type error.

### Primitive Types

```
int       — 64-bit signed integer
float     — 64-bit IEEE 754
bool      — true / false
string    — UTF-8, immutable
bytes     — raw byte sequence (b"..." literals)
()        — unit type
```

### Composite Types

```
list<T>       — [1, 2, 3]
dict<K, V>    — #{"key": value};  insertion-ordered, #{ } literal syntax
Option<T>     — Some(x) / None
Result<T, E>  — Ok(x) / Err(e)
tuple         — (1, "a", true);  heterogeneous, fixed-size
set           — set([1, 2, 3]);  ordered, unique elements
range         — 0..10, 0..=10;  lazy, materializes on iteration
cell          — cell(value);  shared mutable reference for closures
```

### Type Annotations (Optional)

Type annotations can be added to `let` bindings. The outer type is checked at runtime; inner/generic types are documentation-only.

```ion
let x: int = 5;
let name: string = "Ion";
let maybe: Option<int> = Some(42);
let nums: list<int> = [1, 2, 3];
```

### Host-Injected Types

Structs and enums are defined in Rust and injected into the script via `#[derive(IonType)]`. Scripts can construct, access fields, and pattern match on them but cannot declare new ones. See Section 9.

---

## 2. Variables & Mutability

```ion
let x = 10;           // immutable
let mut y = 20;       // mutable
y = 30;               // ok
x = 5;                // runtime error: cannot assign to immutable variable
```

### Destructuring

```ion
let (a, b) = (1, 2);
let [first, ...rest] = items;
```

### Scoping — Lexical, Block-Based

Every `{}` block creates a new scope. Variables are visible from their declaration to the end of their enclosing block. Inner scopes can access outer scopes. Closures capture by value.

```ion
let x = 1;
{
    let y = 2;
    x + y;  // ok — x visible from outer scope
}
y;  // ERROR: y not in scope
```

### Shadowing

A new `let` binding in the same or inner scope can shadow a previous binding. The original value is untouched — shadowing creates a new variable that hides the old one. Shadowing can change both type and mutability.

```ion
let x = 10;
let x = "now a string";  // ok — shadows previous x

{
    let x = true;   // shadows outer x within this block
    io::println(x); // true
}
io::println(x);     // "now a string" — outer x unchanged
```

Shadowing can freeze or unfreeze a binding:

```ion
let mut x = 10;
x = 20;            // ok — mutable
let x = x;         // shadows with immutable binding, "freezes" the value
x = 30;            // ERROR: x is now immutable

let y = 5;
let mut y = y;     // shadows with mutable binding
y = 10;            // ok
```

Function parameters can be shadowed:

```ion
fn process(x) {
    let x = x + 1;  // ok — shadows parameter
    x
}
```

Closures capture the binding at time of capture, not affected by later shadowing:

```ion
let x = 1;
let f = || x + 1;  // captures x = 1
let x = 100;       // shadows x
f();                // returns 2, not 101
```

---

## 3. Expressions — Everything Returns a Value

### If/Else

```ion
let status = if score > 90 { "A" } else { "B" };
```

### Match

```ion
let area = match shape {
    Shape::Circle(r) => 3.14 * r * r,
    Shape::Rect(w, h) => w * h,
    Shape::Named { width, height } => width * height,
};
```

### Match with guards

```ion
let label = match score {
    s if s >= 90 => "A",
    s if s >= 80 => "B",
    _ => "C",
};
```

### Block expressions

```ion
let x = {
    let a = compute();
    let b = transform(a);
    a + b  // last expression = return value (no semicolon)
};
```

**Rule**: A trailing semicolon makes a statement (returns `()`). No trailing semicolon = the expression is the block's value. Same as Rust.

---

## 4. Functions

```ion
fn add(a, b) {
    a + b
}

fn divide(a, b) {
    if b == 0 {
        Err("division by zero")
    } else {
        Ok(a / b)
    }
}

// lambdas
let double = |x| x * 2;
let transform = |x, y| {
    let sum = x + y;
    sum * 2
};
```

### Default arguments

```ion
fn connect(host, port = 8080) {
    // ...
}
```

### Named arguments at call site

Named arguments use `:` (not `=`) to avoid ambiguity with assignment expressions:

```ion
connect(host: "localhost", port: 9090);
```

---

## 5. Error Handling — Result, Option, `?`

### The `?` Operator

```ion
fn load_config(text) {
    let data = json::decode(text)?;  // Err propagates up
    Ok(data)
}
```

The `?` operator's behavior is determined by the **value it's applied to**, not the function's return type:

- `Result` value + `?` → unwraps `Ok`, early-returns `Err`
- `Option` value + `?` → unwraps `Some`, early-returns `None`
- Anything else + `?` → runtime error: "`?` applied to non-Result/Option"

Return type consistency is checked when the function actually returns — you cannot return `Ok(x)` in one branch and `None` in another.

### Combinators

```ion
let name = user
    .get("name")              // Option
    .unwrap_or("anonymous");  // safe — always returns a value

let result = parse(input)
    .map(|v| v * 2)
    .map_err(|e| f"parse failed: {e}");
```

### Try/Catch

```ion
try {
    let val = risky_operation()?;
    process(val)
} catch e {
    io::println(f"Error: {e}");
}
```

### Value Extraction — Explicit Handling

Extracting a value from `Result`/`Option` requires explicit handling:

- `unwrap()` — extract the value, runtime error if None/Err
- `unwrap_or(default)` — provide a fallback value
- `unwrap_or_else(|| compute())` — provide a fallback computation
- `expect("reason")` — extract the value, runtime error with message if None/Err
- `?` — propagate the error/None to caller
- `match` / `if let` — handle each case explicitly

Every value extraction is visible and intentional.

---

## 6. Loops & Iteration

### For loop

```ion
for item in list {
    io::println(item);
}

for (key, value) in my_dict {
    io::println(f"{key}: {value}");
}

for i in 0..10 {
    io::println(i);
}
```

### While loop

```ion
let mut count = 0;
while count < 10 {
    count = count + 1;
}
```

### Loop (infinite, break with value)

```ion
let result = loop {
    let input = get_input();
    if input == "quit" {
        break "done";
    }
};
```

### If-let / While-let

```ion
if let Some(user) = find_user(id) {
    io::println(user.name);
}

while let Some(item) = queue.pop() {
    process(item);
}
```

### Functional

All collection methods return **new collections**. Nothing is mutated in place.

```ion
let evens = numbers
    .filter(|x| x % 2 == 0)
    .map(|x| x * 2);          // returns new list

let sum = numbers.fold(0, |acc, x| acc + x);

let has_negative = numbers.any(|x| x < 0);

// push/pop return new collections — explicit reassignment required
let items = [1, 2, 3];
let items2 = items.push(4);    // [1, 2, 3, 4] — items is unchanged

let mut buf = [];
buf = buf.push(1);             // explicit reassignment on mut binding
buf = buf.push(2);
```

### Comprehensions

```ion
// List comprehension
let evens = [x * 2 for x in 0..10 if x % 2 == 0];

// Dict comprehension
let squares = #{str(n): n * n for n in 1..=5};
```

### Pipe operator

`a |> f(b, c)` is always `f(a, b, c)` — left side becomes the **first argument**.

```ion
let result = data
    |> filter(|x| x > 0)
    |> map(|x| x * 2)
    |> sum();
```

### Compound assignment requires `mut`

`+=`, `-=`, `*=`, `/=` are sugar for `x = x + ...` and therefore require `let mut`:

```ion
let mut count = 0;
count += 1;          // ok — sugar for count = count + 1

let total = 0;
total += 1;          // ERROR: cannot assign to immutable variable
```

---

## 7. String Interpolation — Explicit `f"..."`

```ion
let name = "world";
let greeting = f"hello {name}";
let math = f"result = {1 + 2}";
let nested = f"user: {user.name} (id={user.id})";

// regular strings have no interpolation
let raw = "hello {name}";  // literal text "{name}"

// triple-quoted strings for multiline
let text = """
    multi-line
    string
""";
let ftext = f"""multi-line {name}""";
```

---

## 8. JSON / Dict — First-Class

Dict literals use `#{ }` to distinguish from block expressions. No ambiguity with `{ }`.

### Literal syntax

```ion
let config = #{
    host: "localhost",
    port: 8080,
    features: ["auth", "logging"],
    db: #{
        url: "postgres://...",
        pool_size: 5,
    },
};
```

### Access — dot and bracket

Dict values can be accessed with dot syntax or bracket syntax:

```ion
config.host;               // dot access — returns value or None
config["host"];            // bracket access — same behavior
config.db.pool_size;       // chained dot access
```

### Spread & merge

```ion
let updated = #{ ...config, port: 9090 };
let merged = #{ ...defaults, ...overrides };
```

### JSON interop

All JSON functions are in the `json::` namespace:

```ion
let text = json::encode(config);     // dict → JSON string
let data = json::decode(text)?;      // JSON string → dict (Result)
let pretty = json::pretty(config);   // dict → pretty-printed JSON
```

---

## 9. Host-Injected Types (Structs & Enums)

Scripts **cannot** define structs or enums. All typed structures are injected from the Rust host via `#[derive(IonType)]` or `register_struct`/`register_enum`. Scripts consume them — constructing, accessing fields, pattern matching — but never declare them.

### Why?

- Keeps the language small — dicts cover ad-hoc data needs
- Type definitions belong in Rust where they get compile-time guarantees
- Serde bridge is the single source of truth for shape
- Avoids duplicating type declarations across host and script

### Using host types in scripts

```ion
// Config is injected by host via #[derive(IonType)]
let cfg = Config {
    host: "localhost",
    port: 8080,
    debug: false,
};

// field access
cfg.host;

// functional update
let dev = Config { ...cfg, debug: true };

// methods (registered by host)
cfg.address();
```

### Pattern matching on host enums

```ion
// Command enum injected by host
let response = match cmd {
    Command::Quit => "goodbye",
    Command::Echo(msg) => f"echo: {msg}",
    Command::Move { x, y } => f"move to ({x}, {y})",
};
```

### Nested patterns

```ion
match result {
    Ok(Some(value)) => use_value(value),
    Ok(None) => default(),
    Err(e) => handle(e),
};
```

---

## 10. Modules & Imports

Ion has a namespace system with `::` path syntax and `use` imports.

### Accessing module members

```ion
math::sqrt(16)
json::encode(data)
io::println("hello")
string::join(["a", "b"], ", ")
```

### Importing names

```ion
use math::sqrt;              // import single name
use json::{encode, decode};  // import multiple names
use io::*;                   // import all names from module
```

After importing, names can be used without the namespace prefix.

### Custom modules (from Rust host)

```rust
let mut module = Module::new("mymod");
module.register_fn("hello", |args| Ok(Value::Str("hi".into())));
module.set("VERSION", Value::Int(1));
engine.register_module(module);
```

---

## 11. Structured Concurrency

> Native Tokio integration requires the `async-runtime` cargo feature.

Inspired by Kotlin coroutines / Swift structured concurrency / Trio. All spawned tasks are scoped — they must complete before the parent scope exits. No fire-and-forget.

The preferred runtime is native async: `Engine::eval_async()` returns a Rust
future, drives a pollable bytecode continuation, and parks Ion tasks on
Tokio-polled host futures, timers, channels, or child tasks. Ion source is
mostly free of function coloring. A host async function is called like an
ordinary Ion function; suspension is a runtime property of the host call.

```rust
engine.register_async_fn("fetch", |args| async move {
    let url = args[0].as_str().unwrap_or("").to_string();
    // reqwest::get(url).await?.text().await
    Ok(Value::Str(url))
});
```

### Scope

```ion
let results = async {
    let a = spawn fetch("url_a");
    let b = spawn fetch("url_b");
    // both must complete before this block returns
    [a.await, b.await]
};
```

`spawn` creates another Ion task in the runtime, not a Tokio task and not
an OS thread. `.await` parks the parent until the child completes.

### Cancellation

If the parent scope is cancelled (or errors), all child tasks are cancelled.

```ion
let result = async {
    let a = spawn do_work();
    let b = spawn do_other();
    // if a fails, b is cancelled automatically
    Ok((a.await?, b.await?))
};
```

### Select / race

```ion
let winner = select {
    a = spawn fetch("fast") => f"got: {a}",
    _ = spawn sleep(5000) => Err("timeout"),
};
```

`select` races branch tasks. The first completion wins; losing branch tasks
are cancelled and dropped.

### Channels (bounded)

```ion
let (tx, rx) = channel(10);

fn produce(tx, items) {
    for item in items {
        tx.send(item);
    }
    tx.close();
}

async {
    spawn produce(tx, items);

    let mut val = rx.recv();
    while val != None {
        process(val.unwrap());
        val = rx.recv();
    }
};
```

`spawn` is only valid inside `async {}` blocks — no exceptions.

Under `async-runtime`, `channel(size)` returns native async sender and
receiver endpoints backed by `tokio::sync::mpsc`. `send`, `recv`, and
`recv_timeout` park the Ion task; `try_recv` is immediate; `close` closes
the sender endpoint. See [`docs/concurrency.md`](docs/concurrency.md) for
the runtime model, cancellation semantics, and Tokio embedding pattern.

---

## 12. Rust Embedding API

### Evaluation — running scripts

```rust
use ion_core::Engine;
use ion_core::Value;

let mut engine = Engine::new();

// Run script, get return value
let result = engine.eval("1 + 2")?;
assert_eq!(result, Value::Int(3));

// Run script with side effects
engine.eval("let x = 10;")?;
```

### Getting values out — script → Rust

A script's last expression (without trailing semicolon) is its return value, consistent with how blocks work. The host can also read any top-level variable by name.

```rust
// Read specific variable by name (returns Option<Value>)
engine.eval("let x = 42;")?;
let val = engine.get("x");  // Some(Value::Int(42))

// Get with typed deserialization (requires IonType)
let x: i64 = engine.get_typed::<i64>("x")?;

// Get all top-level bindings
let all: HashMap<String, Value> = engine.get_all();
```

No special export syntax in the script. The script computes values; the host decides what to extract.

### Setting values in — Rust → script

```rust
// Inject values into script scope
engine.set("threshold", Value::Int(80));
engine.set("name", Value::Str("alice".into()));

// Inject typed values (requires IonType)
engine.set_typed("config", &my_config)?;
```

### Registering Rust functions

Two methods, differing only in whether the callback can capture state:

```rust
// Plain fn pointer — stateless, zero overhead
engine.register_fn("fetch_url", |args: &[Value]| -> Result<Value, String> {
    let url = args[0].as_str().ok_or("expected string")?;
    // ... fetch logic ...
    Ok(Value::Str(body))
});

// Closure — can capture host state (DB pool, counters, etc.)
let pool = db_pool.clone();
engine.register_closure("lookup", move |args| {
    let id = args[0].as_int().ok_or("id must be int")?;
    let row = pool.query_one(id).map_err(|e| e.to_string())?;
    Ok(Value::Str(row.name))
});
```

Both produce values with `type_of(f) == "builtin_fn"` and satisfy
`let f: fn = ...;` annotations identically.

### Serde integration — automatic bridging

```rust
#[derive(Serialize, Deserialize, IonType)]
struct Config {
    host: String,
    port: u16,
    debug: bool,
}

// Inject Rust value → Ion value
engine.set_typed("config", &my_config)?;

// Extract Ion value → Rust value
let cfg: Config = engine.get_typed("config")?;
```

`#[derive(IonType)]` generates:
- Field access (so Ion scripts can do `config.host`)
- Constructor (so Ion scripts can do `Config { host: "...", ... }`)
- Pattern matching support for enums
- Serde round-trip for host ↔ script boundary

### Resource limits

```rust
use ion_core::interpreter::Limits;

engine.set_limits(Limits {
    max_call_depth: 256,
    max_loop_iters: 1_000_000,
});
```

---

## 13. Standard Library

Namespaced modules, accessed via `::` syntax. Auto-registered in every Engine.

### `math::` — Mathematics

| Function | Description |
|----------|-------------|
| `abs(x)` | Absolute value |
| `min(a, b)` | Minimum of two values |
| `max(a, b)` | Maximum of two values |
| `floor(x)` | Floor (float → int) |
| `ceil(x)` | Ceiling (float → int) |
| `round(x)` | Round to nearest int |
| `sqrt(x)` | Square root |
| `pow(base, exp)` | Exponentiation |
| `clamp(x, lo, hi)` | Clamp to range |
| `sin(x)`, `cos(x)`, `tan(x)` | Trigonometric functions |
| `atan2(y, x)` | Two-argument arctangent |
| `log(x)`, `log2(x)`, `log10(x)` | Logarithms |
| `is_nan(x)`, `is_inf(x)` | Float classification |

Constants: `PI`, `E`, `TAU`, `INF`, `NAN`

### `json::` — JSON serialization

| Function | Description |
|----------|-------------|
| `encode(value)` | Value → JSON string |
| `decode(text)` | JSON string → Value (returns Result) |
| `pretty(value)` | Value → pretty-printed JSON string |
| `msgpack_encode(value)` | Value → MessagePack bytes (feature `msgpack`) |
| `msgpack_decode(bytes)` | MessagePack bytes → Value (feature `msgpack`) |

### `io::` — Output

| Function | Description |
|----------|-------------|
| `print(value)` | Print without newline |
| `println(value)` | Print with newline |
| `eprintln(value)` | Print to stderr with newline |

### `string::` — String utilities

| Function | Description |
|----------|-------------|
| `join(list, sep)` | Join list elements with separator |

> String methods like `split`, `trim`, `contains`, `to_upper`, etc. are available as methods on string values directly (e.g., `"hello".to_upper()`), not as module functions.

### Top-level builtins (not in modules)

| Function | Description |
|----------|-------------|
| `len(x)` | Length of list, string, dict, bytes |
| `range(n)` / `range(a, b)` | Create a range |
| `set(list)` | Create a set from a list |
| `cell(value)` | Create a shared mutable cell |
| `type_of(x)` | Type name as string |
| `str(x)` | Convert to string |
| `int(x)` | Convert to int |
| `float(x)` | Convert to float |
| `enumerate(list)` | List of (index, value) tuples |
| `bytes(n)` / `bytes(list)` | Create bytes |
| `bytes_from_hex(s)` | Decode hex string to bytes |
| `assert(cond)` | Assert condition is true |
| `assert_eq(a, b)` | Assert equality |
| `sleep(ms)` | Sleep for milliseconds |
| `timeout(ms, fn)` | Run with timeout, returns Option |
| `channel(size)` | Create bounded channel (`async-runtime` native async, `legacy-threaded-concurrency` legacy OS-thread backend) |

### Methods on values

List, dict, string, Option, and Result methods are called directly on values, not through modules:

```ion
[1, 2, 3].map(|x| x * 2)        // list methods
#{ a: 1 }.keys()                  // dict methods
"hello".split(" ")                // string methods
Some(42).unwrap_or(0)             // Option methods
Ok(1).map(|x| x + 1)             // Result methods
```

---

## 14. Operator Summary

| Category    | Operators |
|-------------|-----------|
| Arithmetic  | `+`, `-`, `*`, `/`, `%` |
| Comparison  | `==`, `!=`, `<`, `>`, `<=`, `>=` |
| Logical     | `&&`, `\|\|`, `!` |
| Bitwise     | `&`, `\|`, `^`, `<<`, `>>` |
| Assignment  | `=`, `+=`, `-=`, `*=`, `/=` |
| Range       | `..`, `..=` |
| Pipe        | `\|>` |
| Error prop  | `?` |
| Spread      | `...` |

---

## 15. Keywords

```
let mut fn match if else for while loop
break continue return in as true false
None Some Ok Err
async spawn await select
try catch use
```

Note: `channel`, `sleep`, `timeout`, `cell`, `set`, etc. are builtin functions, not keywords.

---

## 16. Implementation Phases

All phases are complete.

### Phase 1 — Core (Tree-Walk Interpreter) ✓
1. Hand-written lexer
2. Recursive descent parser (Pratt for expressions)
3. AST with spans
4. Tree-walk interpreter
5. Core types: int, float, bool, string, list, dict, Option, Result, tuple
6. Variables, mutability, destructuring
7. Control flow: if/else, match, for, while, loop, if-let, while-let
8. Functions, closures, lambdas, default args, named args
9. `?` operator
10. String interpolation (`f"..."`, `f"""..."""`)

### Phase 2 — Embedding ✓
11. Engine API (eval, set/get, register_fn, register_module)
12. Serde bridge
13. `#[derive(IonType)]` proc macro
14. Resource limits (max_call_depth, max_loop_iters)

### Phase 3 — Ergonomics ✓
15. Pipe operator (`|>`)
16. Comprehensions (list, dict)
17. Spread syntax (`...` in lists and dicts)
18. Namespaced standard library (math, json, io, string)
19. Module system with `use` imports
20. Additional types: bytes, set, range, cell
21. Type annotations (`let x: int = 5`)

### Phase 4 — Concurrency ✓
22. Async runtime (structured scopes, native Tokio continuation runtime)
23. Spawn / await
24. Select
25. Channels
26. Cooperative cancellation (nursery cancellation; select cancels losers)

### Phase 5 — Performance ✓
27. Bytecode compiler
28. Stack-based VM
29. Peephole optimizer, constant folding, dead code elimination
30. Tail call optimization
31. String interning

---

## 17. Project Structure

```
ionlang/
├── Cargo.toml               # workspace
├── ion-core/
│   └── src/
│       ├── lexer.rs          # tokenization
│       ├── token.rs          # token types
│       ├── parser.rs         # recursive descent
│       ├── ast.rs            # AST node types
│       ├── interpreter.rs    # tree-walk evaluator
│       ├── compiler.rs       # bytecode compiler
│       ├── bytecode.rs       # opcodes and chunks
│       ├── vm.rs             # stack-based VM
│       ├── value.rs          # runtime value representation
│       ├── env.rs            # variable environment / scopes
│       ├── intern.rs         # string interning
│       ├── error.rs          # IonError types
│       ├── engine.rs         # public embedding API
│       ├── module.rs         # module system
│       ├── stdlib.rs         # standard library (math, json, io, string)
│       ├── host_types.rs     # host struct/enum injection
│       ├── async_runtime.rs  # native Tokio async eval + bytecode continuations
│       ├── async_rt.rs       # legacy sync-eval OS-thread concurrency traits
│       ├── async_rt_std.rs   # legacy std::thread + crossbeam-channel backend
│       ├── rewrite.rs        # source rewriter (feature: rewrite)
│       └── lib.rs
├── ion-derive/
│   └── src/lib.rs            # #[derive(IonType)] proc macro
├── ion-cli/
│   └── src/main.rs           # script runner + REPL
├── ion-lsp/
│   └── src/main.rs           # LSP server (diagnostics, hover, completion, go-to-def)
├── tree-sitter-ion/
│   └── grammar.js            # tree-sitter grammar for editor support
├── editors/
│   ├── vscode/               # VSCode extension (tmLanguage + LSP client)
│   └── zed/                  # Zed extension (tree-sitter + LSP client)
├── examples/                  # .ion example scripts
└── tests/
    └── scripts/              # .ion test scripts
```

---

## 18. Example: Complete Script

```ion
// Todo struct and its methods are injected by the Rust host:
//   #[derive(Serialize, Deserialize, IonType)]
//   struct Todo { id: i64, title: String, done: bool }
//   with methods: new(id, title), complete(), to_json()

fn find_todo(todos, id) {
    todos.filter(|t| t.id == id).first()  // returns Option
}

let mut todos = [
    Todo::new(1, "Design Ion"),
    Todo::new(2, "Implement lexer"),
    Todo::new(3, "Write tests"),
];

let todo = find_todo(todos, 2)?;
let updated = todo.complete();

let output = todos
    .map(|t| if t.id == updated.id { updated } else { t })
    .map(|t| t.to_json());

io::println(json::pretty(output));
```
