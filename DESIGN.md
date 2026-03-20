# Ion Language Design Specification

A small, strongly-typed, embeddable scripting language inspired by Starlark, implemented in Rust.

---

## 0. Design Principles — No Surprises, No Hidden Control

1. **Explicit over implicit** — No implicit type coercions, no implicit returns from ambiguous syntax, no hidden side effects.
2. **One syntax, one meaning** — `.field` is always a host struct access. `["key"]` is always a dict lookup. `#{ }` is always a dict. `{ }` is always a block.
3. **No hidden panics** — No `unwrap()`. Every value extraction is visible: `?`, `unwrap_or`, `expect`, `match`, `if let`.
4. **Immutable by default** — `let` bindings and all collection methods return new values. Mutation requires explicit `let mut` and reassignment.
5. **Errors are values** — No exceptions. `Result` and `Option` are the only error paths. `?` is the only propagation mechanism.
6. **What you see is what runs** — No operator overloading, no implicit conversions, no magic methods, no inheritance. The behavior of code is determined by reading it top-to-bottom.
7. **Host boundary is clear** — Structs/enums come from Rust. Scripts use dicts for ad-hoc data. You always know which is which.

---

## 1. Type System — Strong, Inferred

No type annotations in syntax. The interpreter tracks and enforces types at runtime. No implicit coercions — `1 + "2"` is a type error.

### Primitive Types

```
int       — 64-bit signed integer
float     — 64-bit IEEE 754
bool      — true / false
string    — UTF-8, immutable
()        — unit type
```

### Composite Types

```
list<T>       — [1, 2, 3];  homogeneous, all elements must be same type
dict<K, V>    — #{"key": value};  insertion-ordered, #{ } literal syntax
Option<T>     — Some(x) / None
Result<T, E>  — Ok(x) / Err(e)
tuple         — (1, "a", true);  heterogeneous, fixed-size
```

### Host-Injected Types

Structs and enums are defined in Rust and injected into the script via `#[derive(IonType)]`. Scripts can construct, access fields, and pattern match on them but cannot declare new ones. See Section 9.

Type annotations do not appear in Ion syntax. Types are always inferred by the interpreter.

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
let Point { x, y } = point;
let #{"host": h, "port": p} = config;
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
    print(x);       // true
}
print(x);           // "now a string" — outer x unchanged
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
fn load_config(path) {
    let text = fs.read(path)?;        // Err propagates up
    let data = json.parse(text)?;     // Err propagates up
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

### No `unwrap()` — Explicit Handling Only

There is no `unwrap()` method. Extracting a value from `Result`/`Option` requires explicit handling:

- `?` — propagate the error/None to caller
- `unwrap_or(default)` — provide a fallback value
- `unwrap_or_else(|| compute())` — provide a fallback computation
- `match` / `if let` — handle each case explicitly
- `expect("reason")` — crash with a documented reason (for truly impossible cases)

This prevents hidden panics. Every value extraction is visible and intentional.

---

## 6. Loops & Iteration

### For loop

```ion
for item in list {
    print(item);
}

for (key, value) in my_dict {
    print(f"{key}: {value}");
}

for i in 0..10 {
    print(i);
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
    let input = read_line();
    if input == "quit" {
        break "done";
    }
};
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
```

---

## 8. JSON / Dict — First-Class

Dict literals use `#{ }` to distinguish from block expressions. No ambiguity with `{ }`.

### Literal syntax

```ion
let config = #{
    "host": "localhost",
    "port": 8080,
    "features": ["auth", "logging"],
    "db": #{
        "url": "postgres://...",
        "pool_size": 5,
    },
};
```

### Access — brackets only, no dot sugar

Dict access always uses `["key"]`. Dot syntax (`.field`) is reserved exclusively for host-injected struct fields. This removes ambiguity — you always know what you're looking at.

```ion
config["host"];             // dict access → Option<T>
config["db"]["pool_size"];  // chained dict access

point.x;                    // host struct field — never a dict
```

### Spread & merge

```ion
let updated = #{ ...config, "port": 9090 };
let merged = #{ ...defaults, ...overrides };
```

### Dict comprehension

```ion
let squares = #{ f"{i}": i * i for i in 0..10 };
```

### List comprehension

```ion
let evens = [x * 2 for x in range(10) if x % 2 == 0];
```

### JSON interop

```ion
let text = json.encode(config);    // dict → JSON string
let data = json.decode(text)?;     // JSON string → dict (Result)
```

---

## 9. Host-Injected Types (Structs & Enums)

Scripts **cannot** define structs or enums. All typed structures are injected from the Rust host via `#[derive(IonType)]` or `register_type`. Scripts consume them — constructing, accessing fields, pattern matching — but never declare them.

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
    Ok(Some(value)) => use(value),
    Ok(None) => default(),
    Err(e) => handle(e),
};
```

### If-let

```ion
if let Some(user) = find_user(id) {
    print(user.name);
}
```

### While-let

```ion
while let Some(item) = queue.pop() {
    process(item);
}
```

---

## 10. Structured Concurrency

Inspired by Kotlin coroutines / Swift structured concurrency / Trio. All spawned tasks are scoped — they must complete before the parent scope exits. No fire-and-forget.

### Scope

```ion
let results = async {
    let a = spawn fetch("url_a");
    let b = spawn fetch("url_b");
    // both must complete before this block returns
    [a.await, b.await]
};
```

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
    a = fetch("fast") => f"got: {a}",
    _ = sleep(5000) => Err("timeout"),
};
```

### Channels (bounded)

```ion
let (tx, rx) = channel(10);

async {
    spawn {
        for item in items {
            tx.send(item).await;
        }
    };

    for msg in rx {
        process(msg);
    }
};
```

`spawn` is only valid inside `async {}` blocks — no exceptions.

---

## 11. Rust Embedding API

### Evaluation — running scripts

```rust
use ion_core::Engine;

let mut engine = Engine::new();

// Run script, discard return value
engine.eval("let x = 1 + 2;")?;

// Run script, deserialize last expression as return value
// (script without trailing semicolon = returns that expression, like blocks)
let result: i64 = engine.eval_as("x * 10")?;
assert_eq!(result, 30);
```

### Getting values out — script → Rust

A script's last expression (without trailing semicolon) is its return value, consistent with how blocks work. The host can also read any top-level variable by name.

```rust
// Method 1: Last expression as return value
let script = r#"
    let data = json.decode(input)?;
    let filtered = data.filter(|x| x["score"] > 80);
    filtered
"#;
let results: Vec<Record> = engine.eval_as(script)?;

// Method 2: Read specific variables by name
engine.eval(script)?;
let filtered: Vec<Record> = engine.get("filtered")?;
let data: Vec<Record> = engine.get("data")?;

// Method 3: Try-get (None if variable doesn't exist, Err if wrong type)
let maybe: Option<Vec<Record>> = engine.try_get("filtered")?;

// Method 4: Get all top-level bindings
let all: HashMap<String, ion_core::Value> = engine.get_all()?;
```

No special export syntax in the script. The script computes values; the host decides what to extract.

### Setting values in — Rust → script

```rust
// Inject Rust values into script scope
engine.set("config", &my_config)?;
engine.set("threshold", &80)?;
engine.set("names", &vec!["alice", "bob"])?;
```

### Registering Rust functions

```rust
engine.register_fn("fetch_url", |url: String| -> Result<String, IonError> {
    reqwest::blocking::get(&url)?.text().map_err(IonError::from)
});
```

### Serde integration — automatic bridging

```rust
#[derive(Serialize, Deserialize, IonType)]
struct Config {
    host: String,
    port: u16,
    debug: bool,
}

// Inject Rust value → Ion value
engine.set("config", &my_config)?;

// Extract Ion value → Rust value
let cfg: Config = engine.get("config")?;
```

`#[derive(IonType)]` generates:
- Field access (so Ion scripts can do `config.host`)
- Constructor (so Ion scripts can do `Config { host: "...", ... }`)
- Pattern matching support for enums
- Serde round-trip for host ↔ script boundary

### Registering custom types without derive

```rust
engine.register_type::<Config>()
    .field("host", |c| &c.host)
    .field("port", |c| &c.port)
    .method("address", |c| format!("{}:{}", c.host, c.port));
```

### Sandboxing

```rust
let engine = Engine::builder()
    .max_execution_time(Duration::from_secs(5))
    .max_memory(64 * 1024 * 1024)  // 64MB
    .max_stack_depth(256)
    .allow_fn("print")
    .allow_fn("json.*")
    .deny_fn("fs.*")
    .build();
```

---

## 12. Standard Library (ion-std)

Minimal, focused on embedding use cases.

| Module   | Functions |
|----------|-----------|
| `string` | `len`, `split`, `join`, `trim`, `contains`, `replace`, `starts_with`, `ends_with`, `to_upper`, `to_lower` |
| `list`   | `len`, `push`, `pop`, `map`, `filter`, `fold`, `any`, `all`, `sort`, `reverse`, `flatten`, `zip` (all return new lists) |
| `dict`   | `len`, `keys`, `values`, `entries`, `contains_key`, `get`, `insert`, `remove`, `merge` (all return new dicts) |
| `json`   | `encode`, `decode`, `encode_pretty` |
| `math`   | `abs`, `min`, `max`, `floor`, `ceil`, `round`, `pow`, `sqrt` |
| `io`     | `print`, `println`, `eprint` (host can override/redirect) |
| `option` | `Some`, `None`, `is_some`, `is_none`, `unwrap_or`, `unwrap_or_else`, `expect`, `map`, `and_then`, `or_else` |
| `result` | `Ok`, `Err`, `is_ok`, `is_err`, `unwrap_or`, `unwrap_or_else`, `expect`, `map`, `map_err`, `and_then`, `or_else` |

All modules are host-configurable — the embedder chooses what to expose.

---

## 13. Operator Summary

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

## 14. Keywords

```
let mut fn match if else for while loop
break continue return true false None Some Ok Err
in as spawn await async select channel
```

---

## 15. Implementation Phases

### Phase 1 — Core (Tree-Walk Interpreter)
1. Lexer (hand-written, zero-copy with logos or manual)
2. Parser (recursive descent / Pratt for expressions)
3. AST (arena-allocated)
4. Tree-walk interpreter
5. Core types: int, float, bool, string, list, dict, Option, Result
6. Variables, mutability, destructuring
7. Control flow: if/else, match, for, while, loop
8. Functions, closures, lambdas
9. `?` operator
10. String interpolation

### Phase 2 — Embedding
12. Engine API (eval, set/get, register_fn)
13. Serde bridge
14. `#[derive(IonType)]` proc macro
15. Sandboxing (time, memory, stack limits)

### Phase 3 — Ergonomics
16. Pipe operator
17. Comprehensions (list, dict)
18. Spread syntax
19. Standard library modules

### Phase 4 — Concurrency
20. Async runtime (structured scopes)
21. Spawn / await
22. Select
23. Channels

### Phase 5 — Performance (Optional)
24. Bytecode compiler
25. Stack-based VM
26. Optimizations (constant folding, interned strings)

---

## 16. Project Structure

```
ion-lang/
├── Cargo.toml              # workspace
├── ion-core/
│   ├── src/
│   │   ├── lexer.rs        # tokenization
│   │   ├── token.rs        # token types
│   │   ├── parser.rs       # recursive descent
│   │   ├── ast.rs          # AST node types
│   │   ├── interpreter.rs  # tree-walk evaluator
│   │   ├── value.rs        # runtime value representation
│   │   ├── env.rs          # variable environment / scopes
│   │   ├── types.rs        # type checking logic
│   │   ├── error.rs        # IonError types
│   │   ├── engine.rs       # public embedding API
│   │   └── lib.rs
├── ion-derive/
│   └── src/lib.rs          # #[derive(IonType)] proc macro
├── ion-std/
│   └── src/                # standard library modules
├── ion-cli/
│   └── src/main.rs         # REPL + file runner
└── tests/
    └── scripts/            # .ion test scripts
```

---

## 17. Example: Complete Script

```ion
// Todo struct and its methods are injected by the Rust host:
//   #[derive(Serialize, Deserialize, IonType)]
//   struct Todo { id: i64, title: String, done: bool }
//   with methods: new(id, title), complete(), to_json()

fn find_todo(todos, id) {
    todos.filter(|t| t.id == id).first()  // returns Option
}

fn main() {
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

    println(json.encode_pretty(output));
}
```
