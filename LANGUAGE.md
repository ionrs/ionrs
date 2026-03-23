# Ion Language Reference

Ion is an embeddable scripting language for Rust applications. It has Rust-flavored syntax with pattern matching, `Result`/`Option` types, immutable-by-default variables, and first-class JSON/dict support.

## Table of Contents

- [Types](#types)
- [Variables](#variables)
- [Operators](#operators)
- [Control Flow](#control-flow)
- [Functions](#functions)
- [Pattern Matching](#pattern-matching)
- [Collections](#collections)
- [Strings](#strings)
- [Error Handling](#error-handling)
- [Builtin Functions](#builtin-functions)
- [Methods](#methods)
- [Bytes](#bytes)
- [Concurrency](#concurrency)
- [Embedding API](#embedding-api)

---

## Types

| Type | Literal | Example |
|------|---------|---------|
| Int | `42`, `-1`, `0` | `let x = 42;` |
| Float | `3.14`, `-0.5` | `let pi = 3.14;` |
| Bool | `true`, `false` | `let ok = true;` |
| String | `"hello"` | `let s = "hello";` |
| Unit | `()` | `let u = ();` |
| List | `[a, b, c]` | `let xs = [1, 2, 3];` |
| Tuple | `(a, b, c)` | `let t = (1, "hi", true);` |
| Dict | `#{key: val}` | `let d = #{name: "ion"};` |
| Option | `Some(x)`, `None` | `let opt = Some(42);` |
| Result | `Ok(x)`, `Err(e)` | `let res = Ok(1);` |
| Bytes | `b"hello"`, `b"\x00\xff"` | `let data = b"\x48\x49";` |
| Function | `fn`, `\|x\| x` | `let f = \|x\| x * 2;` |

All values support `.to_string()` for conversion to string. Use `type_of(value)` to inspect types at runtime:

```
type_of(42)       // "int"
type_of("hello")  // "string"
type_of(true)     // "bool"
type_of([])       // "list"
type_of(#{})      // "dict"
type_of(())       // "unit"
type_of(3.14)     // "float"
```

---

## Variables

Variables are **immutable by default**. Use `mut` for mutable bindings.

```
let x = 10;          // immutable
let mut y = 0;       // mutable
y = 5;               // OK
y += 1;              // compound assignment
// x = 1;            // ERROR: x is not mutable
```

### Compound assignment operators

`+=`, `-=`, `*=`, `/=`

### Destructuring

```
let (a, b) = (1, 2);
let [x, y, z] = [10, 20, 30];
```

---

## Operators

### Arithmetic
`+`, `-`, `*`, `/`, `%` (modulo)

### Comparison
`==`, `!=`, `<`, `>`, `<=`, `>=`

### Logical
`&&` (and), `||` (or), `!` (not)

### Bitwise
`&` (and), `|` (or), `^` (xor), `<<` (left shift), `>>` (right shift)

### Unary
`-` (negate), `!` (not)

### Pipe
```
5 |> double        // equivalent to double(5)
```

### Try (`?`)
Unwraps `Ok`/`Some`, propagates `Err`/`None` (see [Error Handling](#error-handling)).

---

## Control Flow

### If / else

`if`/`else` is an expression — it returns a value.

```
let result = if x > 0 { "positive" } else { "non-positive" };
```

Chained:
```
if x < 0 {
    -1
} else if x == 0 {
    0
} else {
    1
}
```

### If-let

Destructure and branch on `Option`/`Result`:

```
let opt = Some(10);
if let Some(v) = opt {
    v + 1
} else {
    0
}
```

### While

```
let mut x = 0;
while x < 10 {
    x += 1;
}
```

### While-let

```
let mut items = Some(1);
while let Some(v) = items {
    // process v
    items = None;
}
```

### For

Iterates over lists, tuples, dicts, strings, bytes, and ranges.

```
for x in [1, 2, 3] {
    println(x);
}

for (key, val) in #{a: 1, b: 2} {
    println(key, val);
}

for ch in "hello" {
    println(ch);
}
```

### Loop

Infinite loop. Use `break` to exit, optionally with a value.

```
let result = loop {
    if done { break 42; }
};
```

### Break / Continue

```
for x in [1, 2, 3, 4, 5] {
    if x == 3 { continue; }
    if x == 5 { break; }
    println(x);
}
```

### Return

```
fn find_first(items) {
    for item in items {
        if item > 10 { return item; }
    }
    return None;
}
```

---

## Functions

### Declaration

```
fn add(a, b) {
    a + b
}
```

The last expression is the return value (no semicolon needed).

### Default parameters

```
fn greet(name = "world") {
    f"hello {name}"
}
greet()         // "hello world"
greet("ion")    // "hello ion"
```

### Named arguments

Call functions with arguments specified by name:

```
fn connect(host, port, timeout = 30) {
    f"{host}:{port} (timeout={timeout})"
}
connect(port: 443, host: "example.com")
connect("localhost", timeout: 5, port: 8080)
```

Positional and named arguments can be mixed. Named arguments are resolved to parameter positions at call time.

### Lambdas (closures)

```
let double = |x| x * 2;
let add = |a, b| a + b;
```

Lambdas capture variables from their enclosing scope:

```
let x = 10;
let add_x = |y| x + y;
add_x(5)  // 15
```

### Higher-order functions

```
fn make_adder(x) {
    |y| x + y
}
let add5 = make_adder(5);
add5(10)  // 15
```

### Recursion

```
fn fib(n) {
    if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
}
fib(10)  // 55
```

Tail calls are optimized (TCO) when the `optimize` feature is enabled.

---

## Pattern Matching

### Match expression

```
match value {
    pattern => result,
    pattern => result,
}
```

### Pattern types

```
// Literal patterns
match x {
    0 => "zero",
    1 => "one",
    _ => "other",       // wildcard
}

// Variable binding
match x {
    n => n + 1,         // binds n to x
}

// Option patterns
match opt {
    Some(v) => v * 2,
    None => 0,
}

// Result patterns
match res {
    Ok(v) => v,
    Err(e) => e,
}

// Tuple patterns
match (1, 2) {
    (a, b) => a + b,
}

// List patterns
match items {
    [] => "empty",
    [a] => f"one: {a}",
    [a, b, ...rest] => f"first: {a}",  // rest pattern
}
```

### Guards

```
match x {
    n if n > 0 => "positive",
    n if n < 0 => "negative",
    _ => "zero",
}
```

---

## Collections

### Lists

```
let xs = [1, 2, 3];
xs[0]                   // 1 (0-indexed)
xs[-1]                  // 3 (negative indexing)
xs[1..3]                // [2, 3] (slicing)
xs[..2]                 // [1, 2]
xs[1..]                 // [2, 3]
```

See [List Methods](#list-methods) for the full API.

### Tuples

```
let t = (1, "hello", true);
t.0                     // 1 (field access by index)
t.1                     // "hello"
```

### Dicts

```
let d = #{name: "ion", version: 1};
d.name                  // "ion" (field access)
d["name"]               // "ion" (bracket access)
d.missing               // None (missing key returns None)
```

String keys and shorthand identifier keys are equivalent:
```
#{name: 1}              // same as #{"name": 1}
```

Computed keys use expressions:
```
let key = "x";
#{(key): 42}            // #{"x": 42}
```

Spread operator:
```
let base = #{a: 1, b: 2};
let extended = #{...base, c: 3};  // #{a: 1, b: 2, c: 3}
```

See [Dict Methods](#dict-methods) for the full API.

### Comprehensions

```
// List comprehension
[x * 2 for x in [1, 2, 3]]              // [2, 4, 6]

// Filtered comprehension
[x for x in [1, 2, 3, 4, 5] if x > 3]  // [4, 5]

// Dict comprehension
#{k: v * 10 for (k, v) in #{a: 1, b: 2}}
```

---

## Strings

### Literals

```
"hello world"
"line one\nline two"    // escape sequences: \n, \t, \r, \\, \"
"\u{1F600}"             // Unicode escape: 😀
```

### Triple-quoted strings

Multi-line strings using triple quotes:

```
let text = """
This is a multi-line
string literal.
""";
```

A leading newline immediately after `"""` is stripped. Escape sequences work inside triple-quoted strings.

Triple-quoted f-strings: `f"""value: {x}"""`

### F-strings (interpolation)

```
let name = "ion";
f"hello {name}"                   // "hello ion"
f"2 + 2 = {2 + 2}"               // "2 + 2 = 4"
f"result: {foo("bar")}"          // nested quotes in interpolation
```

### Indexing

```
"hello"[0]                        // "h"
"hello"[4]                        // "o"
"hello"[-1]                       // "o" (negative indexing)
```

### Concatenation and repetition

```
"hello" + " " + "world"          // "hello world"
"ha" * 3                          // "hahaha"
3 * "ab"                          // "ababab"
```

See [String Methods](#string-methods) for the full API.

---

## Error Handling

### Option

Represents an optional value: `Some(value)` or `None`.

```
let opt = Some(42);
let empty = None;
```

### Result

Represents success or failure: `Ok(value)` or `Err(error)`.

```
let ok = Ok(42);
let err = Err("something failed");
```

### The `?` operator

`?` unwraps `Ok`/`Some` or early-returns `Err`/`None`:

```
fn parse_and_double(s) {
    let n = s.to_int()?;   // propagates Err if parse fails
    Ok(n * 2)
}
parse_and_double("5")    // Ok(10)
parse_and_double("abc")  // Err("invalid digit found in string")
```

```
fn first_item(items) {
    let v = items.first()?;  // propagates None if empty
    Some(v + 1)
}
```

At the top level (outside a function), `?` on `Err`/`None` returns the error/none as a value rather than crashing.

### Try / Catch

Catch runtime errors with `try`/`catch` blocks:

```
let result = try {
    let x = 10 / 0;
    x
} catch e {
    f"caught: {e}"
};
// result is the error message string
```

`try`/`catch` is an expression — the last value of whichever branch executes is returned. Control flow signals (`return`, `break`, `continue`) pass through and are not caught.

```
// Nested try/catch
try {
    try {
        error_fn()
    } catch inner {
        f"inner: {inner}"
    }
} catch outer {
    f"outer: {outer}"
}
```

### Method chains

Both `Option` and `Result` support functional chaining:

```
Some(5).unwrap()                  // 5
Some(5).map(|x| x * 2)           // Some(10)
None.unwrap_or(0)                 // 0
Ok(5).unwrap()                    // 5
Ok(5).map(|x| x + 1)             // Ok(6)
Err("fail").unwrap_or(0)          // 0
```

See [Option Methods](#option-methods) and [Result Methods](#result-methods).

---

## Builtin Functions

### I/O
| Function | Description |
|----------|-------------|
| `print(args...)` | Print without newline |
| `println(args...)` | Print with newline |

### Type conversion
| Function | Description |
|----------|-------------|
| `str(x)` | Convert to string |
| `int(x)` | Convert to int (from float, string, bool) |
| `float(x)` | Convert to float (from int, string) |
| `type_of(x)` | Returns type name as string |

### Collections
| Function | Description |
|----------|-------------|
| `len(x)` | Length of list, string, dict, or bytes |
| `range(n)` | `[0, 1, ..., n-1]` |
| `range(start, end)` | `[start, start+1, ..., end-1]` |
| `enumerate(val)` | `[(0, a), (1, b), ...]` (list, string, or dict) |
| `join(list, sep)` | Join list elements with separator |

### Math
| Function | Description |
|----------|-------------|
| `abs(x)` | Absolute value |
| `min(a, b, ...)` | Minimum of arguments |
| `max(a, b, ...)` | Maximum of arguments |
| `floor(x)` | Floor (rounds down) |
| `ceil(x)` | Ceiling (rounds up) |
| `round(x)` | Round to nearest |
| `sqrt(x)` | Square root |
| `pow(base, exp)` | Exponentiation |
| `clamp(val, min, max)` | Clamp value to range |

### Assertions
| Function | Description |
|----------|-------------|
| `assert(cond)` | Error if `cond` is false |
| `assert(cond, msg)` | Error with custom message if `cond` is false |
| `assert_eq(a, b)` | Error if `a != b` |
| `assert_eq(a, b, msg)` | Error with custom message if `a != b` |

### JSON
| Function | Description |
|----------|-------------|
| `json_encode(value)` | Value to JSON string |
| `json_encode_pretty(value)` | Pretty-printed JSON |
| `json_decode(string)` | JSON string to value |

### Bytes
| Function | Description |
|----------|-------------|
| `bytes()` | Empty bytes |
| `bytes(list)` | Bytes from list of ints (0-255) |
| `bytes(string)` | Bytes from UTF-8 string |
| `bytes(n)` | Zero-filled bytes of length n |
| `bytes_from_hex(string)` | Bytes from hex string |

---

## Methods

### String Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `.len()` | Int | Byte length |
| `.char_len()` | Int | Character count (for Unicode) |
| `.is_empty()` | Bool | True if empty |
| `.contains(sub)` | Bool | Contains substring (string or char code int) |
| `.starts_with(pre)` | Bool | Starts with prefix |
| `.ends_with(suf)` | Bool | Ends with suffix |
| `.find(sub)` | Option(Int) | Char index of first occurrence |
| `.trim()` | String | Strip leading/trailing whitespace |
| `.trim_start()` | String | Strip leading whitespace |
| `.trim_end()` | String | Strip trailing whitespace |
| `.to_upper()` | String | Uppercase |
| `.to_lower()` | String | Lowercase |
| `.split(delim)` | List | Split by delimiter |
| `.replace(from, to)` | String | Replace all occurrences |
| `.chars()` | List | List of single-char strings |
| `.pad_start(n, ch?)` | String | Left-pad to width n (default space) |
| `.pad_end(n, ch?)` | String | Right-pad to width n (default space) |
| `.strip_prefix(s)` | String | Remove prefix if present |
| `.strip_suffix(s)` | String | Remove suffix if present |
| `.reverse()` | String | Reversed string |
| `.repeat(n)` | String | Repeat n times |
| `.slice(start, end)` | String | Substring by char index |
| `.bytes()` | List | List of byte values (ints) |
| `.to_int()` | Result | Parse as integer |
| `.to_float()` | Result | Parse as float |

### List Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `.len()` | Int | Number of elements |
| `.is_empty()` | Bool | True if empty |
| `.first()` | Option | First element |
| `.last()` | Option | Last element |
| `.contains(val)` | Bool | Contains value |
| `.push(val)` | List | New list with value appended |
| `.pop()` | (List, Option) | New list and removed last element |
| `.sort()` | List | Sorted copy |
| `.reverse()` | List | Reversed copy |
| `.flatten()` | List | Flatten one level of nesting |
| `.join(sep)` | String | Join elements with separator |
| `.enumerate()` | List | `[(0, a), (1, b), ...]` |
| `.zip(other)` | List | `[(a1, b1), (a2, b2), ...]` |
| `.map(fn)` | List | Apply function to each element |
| `.filter(fn)` | List | Keep elements where fn returns true |
| `.fold(init, fn)` | Value | Reduce with accumulator |
| `.flat_map(fn)` | List | Map then flatten results |
| `.any(fn)` | Bool | True if fn returns true for any element |
| `.all(fn)` | Bool | True if fn returns true for all elements |
| `.index(val)` | Option(Int) | Index of first occurrence |
| `.count(val)` | Int | Number of occurrences |
| `.slice(start, end?)` | List | Sublist by index |
| `.dedup()` | List | Remove consecutive duplicates |
| `.unique()` | List | Remove all duplicates (preserves order) |
| `.min()` | Option | Minimum element |
| `.max()` | Option | Maximum element |
| `.sum()` | Int/Float | Sum of numeric elements |
| `.window(n)` | List | Sliding windows of size n |
| `.sort_by(fn)` | List | Sort with custom comparator (fn returns int: neg/0/pos) |

### Tuple Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `.len()` | Int | Number of elements |
| `.contains(val)` | Bool | Contains value |
| `.to_list()` | List | Convert to list |

### Dict Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `.len()` | Int | Number of entries |
| `.is_empty()` | Bool | True if empty |
| `.keys()` | List | List of keys |
| `.values()` | List | List of values |
| `.entries()` | List | List of (key, value) tuples |
| `.contains_key(key)` | Bool | Key exists |
| `.get(key)` | Option | Get value by key |
| `.update(other)` | Dict | Merge other dict (overwrites existing keys) |
| `.keys_of(val)` | List | Keys with the given value |
| `.insert(key, val)` | Dict | New dict with entry added |
| `.remove(key)` | Dict | New dict with entry removed |
| `.merge(other)` | Dict | New dict merging two dicts |
| `.map(fn)` | Dict | Apply fn(key, value) to each entry, keep keys |
| `.filter(fn)` | Dict | Keep entries where fn(key, value) is truthy |
| `.zip(other)` | Dict | Merge matching keys into tuples |

### Option Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `.is_some()` | Bool | True if Some |
| `.is_none()` | Bool | True if None |
| `.unwrap()` | Value | Extract value, error if None |
| `.unwrap_or(default)` | Value | Unwrap or return default |
| `.expect(msg)` | Value | Unwrap or error with message |
| `.map(fn)` | Option | Apply fn to inner value |
| `.and_then(fn)` | Value | Flat-map (fn should return Option) |
| `.or_else(fn)` | Value | Call fn if None |
| `.unwrap_or_else(fn)` | Value | Unwrap or call fn |

### Result Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `.is_ok()` | Bool | True if Ok |
| `.is_err()` | Bool | True if Err |
| `.unwrap()` | Value | Extract value, error if Err |
| `.unwrap_or(default)` | Value | Unwrap or return default |
| `.expect(msg)` | Value | Unwrap or error with message |
| `.map(fn)` | Result | Apply fn to Ok value |
| `.map_err(fn)` | Result | Apply fn to Err value |
| `.and_then(fn)` | Value | Flat-map on Ok |
| `.or_else(fn)` | Value | Flat-map on Err |
| `.unwrap_or_else(fn)` | Value | Unwrap or call fn with error |

### Bytes Methods

| Method | Returns | Description |
|--------|---------|-------------|
| `.len()` | Int | Number of bytes |
| `.is_empty()` | Bool | True if empty |
| `.contains(byte)` | Bool | Contains byte value |
| `.find(byte)` | Option(Int) | Index of first occurrence |
| `.slice(start, end)` | Bytes | Sub-slice |
| `.reverse()` | Bytes | Reversed copy |
| `.push(byte)` | Bytes | New bytes with byte appended |
| `.to_list()` | List | List of int values |
| `.to_str()` | Result | Decode as UTF-8 |
| `.to_hex()` | String | Hex-encoded string |

---

## Bytes

Binary data type for raw byte manipulation.

```
let data = b"hello";           // bytes literal
let hex = b"\x48\x49";         // hex escape
let empty = bytes();           // empty bytes
let from_list = bytes([72, 73]); // from ints

data[0]                        // 72 (int)
data[0..3]                     // b"hel" (slicing)
data.len()                     // 5
data.to_hex()                  // "68656c6c6f"
data.to_str()                  // Ok("hello")
```

---

## Concurrency

> Requires the `concurrency` cargo feature.

Ion provides structured concurrency with `async` blocks, `spawn`, and channels.

### Async blocks

```
let result = async {
    let t1 = spawn compute(1);
    let t2 = spawn compute(2);
    t1.await + t2.await
};
```

All spawned tasks must complete before the `async` block returns.

### Spawn and await

```
let task = spawn expensive_work();
// ... do other things ...
let result = task.await;
```

### Task methods

| Method | Description |
|--------|-------------|
| `.await` | Wait for task to complete |
| `.await_timeout(ms)` | Wait with timeout (returns Result) |
| `.is_finished()` | Check if task is done |
| `.cancel()` | Request cancellation |
| `.is_cancelled()` | Check if cancelled |

### Channels

```
let (tx, rx) = channel(10);    // buffered channel, capacity 10
tx.send(42);
let val = rx.recv();           // blocks until value available
tx.close();
```

| Method | Description |
|--------|-------------|
| `tx.send(val)` | Send a value |
| `tx.close()` | Close the sender |
| `rx.recv()` | Receive (blocks), returns `None` when closed |
| `rx.try_recv()` | Non-blocking receive, returns `Option` |
| `rx.recv_timeout(ms)` | Receive with timeout, returns `Option` |

### Select

Race multiple async operations:

```
select {
    val = task1 => f"task1 finished: {val}",
    val = task2 => f"task2 finished: {val}",
}
```

---

## Embedding API

Ion is designed to be embedded in Rust applications.

### Basic usage

```rust
use ion_core::engine::Engine;
use ion_core::value::Value;

let mut engine = Engine::new();

// Evaluate a script
let result = engine.eval("2 + 2").unwrap();
assert_eq!(result, Value::Int(4));

// Inject values
engine.set("player_hp", Value::Int(100));
engine.eval("player_hp > 50").unwrap(); // Value::Bool(true)

// Read values back
let hp = engine.get("player_hp");
```

### Register custom functions

```rust
engine.register_fn("roll_dice", |args| {
    let sides = args[0].as_int().ok_or("expected int")?;
    Ok(Value::Int(rand::random::<i64>() % sides + 1))
});
```

### Host types with `#[derive(IonType)]`

```rust
use ion_derive::IonType;

#[derive(IonType)]
struct Player {
    name: String,
    hp: i64,
    alive: bool,
}

let mut engine = Engine::new();
engine.register_type::<Player>();

// Scripts can now construct and match on Player
engine.eval(r#"
    let p = Player { name: "Alice", hp: 100, alive: true };
    p.name
"#);
```

### Host enums

```rust
#[derive(IonType)]
enum Status {
    Active,
    Dead,
    Poisoned(i64),
}

engine.register_type::<Status>();
engine.eval(r#"
    match status {
        Status::Active => "fine",
        Status::Poisoned(dmg) => f"poisoned for {dmg}",
        Status::Dead => "dead",
    }
"#);
```

### Typed data exchange

```rust
let player = Player { name: "Bob".into(), hp: 50, alive: true };
engine.set_typed("player", &player);

engine.eval("player.hp += 10;");

let updated: Player = engine.get_typed("player").unwrap();
```

### Execution limits

```rust
use ion_core::interpreter::Limits;

engine.set_limits(Limits {
    max_call_depth: 256,
    max_loop_iters: 100_000,
});
```

### Bytecode VM

```rust
// Use the bytecode VM for better performance (requires "vm" feature)
let result = engine.vm_eval("fib(30)").unwrap();
```

The VM automatically falls back to tree-walk interpretation for unsupported features.

### Cargo features

| Feature | Default | Description |
|---------|---------|-------------|
| `vm` | Yes | Bytecode VM path (`vm_eval`) |
| `optimize` | Yes | Peephole optimizer, constant folding, DCE, TCO |
| `derive` | Yes | `#[derive(IonType)]` proc macro |
| `concurrency` | No | Structured concurrency (`async`, `spawn`, channels) |
| `obfuscate` | No | String obfuscation via `obfstr` |
