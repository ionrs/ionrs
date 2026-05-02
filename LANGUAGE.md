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
- [Modules](#modules)
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
    io::println(x);
}

for (key, val) in #{a: 1, b: 2} {
    io::println(key, val);
}

for ch in "hello" {
    io::println(ch);
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
    io::println(x);
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

Tail calls are optimized (TCO) on the bytecode VM path.

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
| `set(list)` | Create a set (deduplicates) |
| `cell(value)` | Mutable reference cell for shared closure state |

### Assertions
| Function | Description |
|----------|-------------|
| `assert(cond)` | Error if `cond` is false |
| `assert(cond, msg)` | Error with custom message if `cond` is false |
| `assert_eq(a, b)` | Error if `a != b` |
| `assert_eq(a, b, msg)` | Error with custom message if `a != b` |

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

### Cell Methods

A `Cell` is a shared mutable reference cell, created with the top-level builtin `cell(value)`. It enables shared mutable state across closures.

```
let c = cell(0);
c.get()           // 0
c.set(42);
c.get()           // 42
c.update(|v| v + 1);
c.get()           // 43
```

| Method | Returns | Description |
|--------|---------|-------------|
| `.get()` | Value | Read the current value |
| `.set(v)` | Unit | Replace the stored value with `v` |
| `.update(fn)` | Unit | Apply `fn` to the current value and store the result |

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

> Native async host integration requires the `async-runtime` cargo feature.

Ion provides structured concurrency with `async` blocks, `spawn`, `.await`,
`select`, timers, and channels. In the native async runtime, Ion code stays
synchronous-looking: a script calls an async host function as a normal
function, and the runtime parks the current bytecode continuation on the
underlying Tokio future.

```rust
engine.register_async_fn("http_get", |args| async move {
    let url = args[0].as_str().unwrap_or("").to_string();
    // reqwest::get(url).await?.text().await, mapped into Value
    Ok(Value::Str(url))
});
```

```
fn load_user(id) {
    // No `await` at this call site. If `http_get` is a host async
    // function, eval_async parks and resumes this Ion function.
    json::decode(http_get(f"/users/{id}"))
}
```

### Async blocks

```
let result = async {
    let t1 = spawn compute(1);
    let t2 = spawn compute(2);
    t1.await + t2.await
};
```

All spawned tasks must complete before the `async` block returns.
Spawned tasks are Ion tasks managed by the runtime, not OS threads.

### Spawn and await

```
let task = spawn expensive_work();
// ... do other things ...
let result = task.await;
```

`spawn` is only valid inside an `async {}` block. Awaiting a task parks
the caller until the child completes. If a child fails before it is awaited,
the nursery cancels sibling tasks and propagates the error.

### Channels

```
let (tx, rx) = channel(10);    // buffered channel, capacity 10
tx.send(42);
let val = rx.recv();           // parks until value available
tx.close();
```

| Method | Description |
|--------|-------------|
| `tx.send(val)` | Send a value, parking if the bounded channel is full |
| `tx.close()` | Close the sender |
| `rx.recv()` | Receive, parking until a value arrives; returns `None` when closed |
| `rx.try_recv()` | Non-blocking receive, returns `Option` |
| `rx.recv_timeout(ms)` | Receive with timeout, returns `Option` |

Under `async-runtime`, channels are backed by Tokio channels and integrate
with the same parking mechanism as async host functions.

### Select

Race multiple async operations:

```
select {
    val = spawn http_get("/fast") => f"fast: {val}",
    _ = spawn sleep(250) => "timeout",
}
```

The first completed branch wins. Losing branch tasks are cancelled and
dropped.

### Timers and timeout

```
sleep(50);  // parks on a Tokio timer under eval_async

let maybe_body = timeout(250, || http_get("/slow"));
match maybe_body {
    Some(body) => body,
    None => "request timed out",
}
```

`timeout(ms, fn)` returns `Some(value)` if the callback completes before
the timer, or `None` if the timer wins. Callback errors still propagate.

### Tokio host callbacks

Host async functions can call back into Ion through `EngineHandle`:

```rust
let handle = engine.handle();
engine.register_async_fn("wait_for_event", move |_args| {
    let handle = handle.clone();
    async move {
        let event = Value::Str("ready".into());
        handle.call_async("on_event", vec![event]).await
    }
});
```

The callback runs on the same local async runtime and may itself call
async host functions.

---

## Modules

Modules provide namespaced access to functions and values registered by the host application.

### Module path access

Use `::` to access module members:

```
let result = math::add(1, 2);
let pi = math::PI;
```

Nested submodules:
```
let resp = net::http::get("https://example.com");
```

### Use imports

Import specific names into the current scope:

```
use math::add;             // single import
use math::{add, PI};       // multiple imports
use math::*;               // glob import (all names)
```

After importing, names can be used directly:
```
use math::add;
add(1, 2)                  // 3 (no need for math::add)
```

### Aliased imports

Use `as` to rebind an imported name to a different local name. Aliases work for
single imports and inside braced lists; glob imports cannot be aliased.

```
use io::println as say;            // single import with alias
use math::add as sum;
use math::{add as sum, PI};        // mixed: some aliased, some not
use math::{add as sum, PI as pi};  // all aliased
```

After aliasing, only the alias is in scope — the original name is not bound:
```
use math::add as sum;
sum(1, 2)                  // 3
add(1, 2)                  // error: undefined name 'add'
```

### Submodule imports

```
use net::http::get;             // import from nested module
use net::http::get as fetch;    // nested import with alias
use net::http::*;               // glob import from nested module
```

### Standard library modules

The following modules are available by default in every Engine:

#### `math`

| Name | Description |
|------|-------------|
| `math::PI` | Pi (3.14159...) |
| `math::E` | Euler's number (2.71828...) |
| `math::TAU` | Tau (2π) |
| `math::INF` | Positive infinity |
| `math::NAN` | Not-a-number |
| `math::abs(x)` | Absolute value |
| `math::min(a, b, ...)` | Minimum |
| `math::max(a, b, ...)` | Maximum |
| `math::floor(x)` | Floor |
| `math::ceil(x)` | Ceiling |
| `math::round(x)` | Round to nearest |
| `math::sqrt(x)` | Square root |
| `math::pow(base, exp)` | Exponentiation |
| `math::clamp(val, lo, hi)` | Clamp to range |
| `math::sin(x)` | Sine |
| `math::cos(x)` | Cosine |
| `math::tan(x)` | Tangent |
| `math::atan2(y, x)` | Two-argument arctangent |
| `math::log(x)` | Natural logarithm |
| `math::log2(x)` | Base-2 logarithm |
| `math::log10(x)` | Base-10 logarithm |
| `math::is_nan(x)` | Check if NaN |
| `math::is_inf(x)` | Check if infinite |

#### `json`

| Name | Description |
|------|-------------|
| `json::encode(value)` | Value to JSON string |
| `json::decode(string)` | JSON string to value |
| `json::pretty(value)` | Pretty-printed JSON string |

#### `io`

| Name | Description |
|------|-------------|
| `io::print(args...)` | Print without newline |
| `io::println(args...)` | Print with newline |
| `io::eprintln(args...)` | Print to stderr with newline |

Embedding hosts must install an `OutputHandler` on the engine before
scripts can use `io::print`, `io::println`, or `io::eprintln`.

#### `string`

| Name | Description |
|------|-------------|
| `string::join(list, sep?)` | Join list elements into a string with optional separator |

#### `semver`

Semantic version parsing, comparison, and constraint matching. Versions
cross the language boundary as dicts shaped `#{major, minor, patch, pre, build}`
so scripts can inspect fields directly. Most functions accept either a string
or a parsed dict — parse once for hot loops.

| Name | Description |
|------|-------------|
| `semver::parse(s)` | Parse a version string into `#{major, minor, patch, pre, build}`. Errors on invalid input. |
| `semver::is_valid(s)` | `true` if the string parses as a valid semantic version. |
| `semver::format(v)` | Render a version (string or dict) back to its canonical string form. |
| `semver::compare(a, b)` | Three-way ordering: returns `-1`, `0`, or `1`. |
| `semver::eq(a, b)` | `true` if `a == b` (including pre-release). |
| `semver::gt(a, b)` / `gte` / `lt` / `lte` | Boolean comparators. |
| `semver::satisfies(v, req)` | `true` if `v` matches the requirement string (e.g. `^1.0`, `~1.2`, `>=1.0, <2.0`). |
| `semver::bump_major(v)` | Increment major; zero minor and patch; clear pre-release and build. |
| `semver::bump_minor(v)` | Increment minor; zero patch; clear pre-release and build. |
| `semver::bump_patch(v)` | Increment patch — or strip pre-release if present (`1.2.3-alpha → 1.2.3`). |

```
use semver::*;

let v = parse("1.2.3-alpha.1+build.42")?;
v["major"]                                        // 1
satisfies("1.5.0", "^1.0")                        // true
compare("1.2.3", "1.2.4")                         // -1
bump_major("1.2.3-alpha")                         // "2.0.0"
```

The module is enabled by default. Embedders can opt out by depending on
`ion-core` with `default-features = false`.

#### `os`

OS / arch detection, environment variables, and process info. Pure-`std`,
no extra dependencies. Detection values are module-level constants
(read with `os::name`, no parens).

| Name | Description |
|------|-------------|
| `os::name` | Target OS — `"linux"`, `"macos"`, `"windows"`, `"freebsd"`, … |
| `os::arch` | Target architecture — `"x86_64"`, `"aarch64"`, `"arm"`, … |
| `os::family` | OS family — `"unix"` or `"windows"`. |
| `os::pointer_width` | Pointer width in bits (`32` or `64`). |
| `os::dll_extension` | Dynamic-library extension without dot — `"so"`, `"dylib"`, `"dll"`. |
| `os::exe_extension` | Executable extension without dot — `""` on Unix, `"exe"` on Windows. |
| `os::env_var(name [, default])` | Read an env var. Errors if missing; the optional 2nd arg is returned instead. |
| `os::has_env_var(name)` | `true` if the named env var is set. |
| `os::env_vars()` | Snapshot of all env vars as a `dict<string, string>`. |
| `os::cwd()` | Current working directory. |
| `os::pid()` | Current process id. |
| `os::args()` | Script arguments (host-injected via `Engine::set_args`; default `[]`). |
| `os::temp_dir()` | Platform temporary directory. |

```
use os::*;

io::println(os::name, os::arch, os::family);

let home = env_var("HOME", "/");
let port_str = env_var("PORT", "8080");

if os::family == "unix" {
    io::println("running on unix");
}

io::println("script args:", args());
```

`os::args()` reflects whatever the host passed to `Engine::set_args`. The
`ion` CLI populates it with whatever follows the script path —
`ion script.ion alpha beta` makes `os::args()` return `["alpha", "beta"]`.

The module is enabled by default. Embedders can opt out by depending on
`ion-core` with `default-features = false` (e.g. to forbid scripts from
reading environment variables).

#### `path`

Pure-string path manipulation. No I/O, no external dependencies, always
available — these are composition primitives for working with paths
before (or after) handing them to `fs::` or the host. All operations work
on strings and return strings; the platform separator is exposed as
`path::sep` so scripts can stay portable.

| Name | Description |
|------|-------------|
| `path::sep` | Platform path separator — `/` on Unix, `\` on Windows. |
| `path::join(a, b, ...)` | Variadic join using the platform separator. |
| `path::parent(p)` | Directory containing `p`. Empty string if `p` has no parent. |
| `path::basename(p)` | Final component of `p`. |
| `path::stem(p)` | Basename of `p` with the extension stripped. |
| `path::extension(p)` | Extension of `p` without the leading dot. Empty string if none. |
| `path::with_extension(p, ext)` | Replace (or add) the extension on `p`. |
| `path::is_absolute(p)` | `true` if `p` is absolute on the current platform. |
| `path::is_relative(p)` | `true` if `p` is relative on the current platform. |
| `path::components(p)` | Split `p` into its components. |
| `path::normalize(p)` | Lexically normalise — collapse `.` and `..` without touching the filesystem. |

```
use path::*;

io::println(join("src", "lib", "main.ion"));   // src/lib/main.ion (Unix)
io::println(extension("config.toml"));         // toml
io::println(stem("config.toml"));              // config
io::println(with_extension("notes.md", "txt"));// notes.txt
io::println(normalize("a/./b/../c"));          // a/c
```

#### `fs`

Filesystem I/O. The script-level surface (`fs::read`, `fs::write`, …) is
identical regardless of build mode; only the underlying implementation
differs. Ion is non-coloured: scripts call these like any other function.

* In a **sync build** (default), each `fs::*` call goes through `std::fs`
  on the calling thread.
* In an **async build** (`async-runtime` feature), each `fs::*` call goes
  through `tokio::fs` and cooperates with the executor instead of blocking.

The two modes are **mutually exclusive** at the cargo-feature level — pick
one when you build `ion-core`. The same script runs unchanged in either.

| Name | Description |
|------|-------------|
| `fs::read(path)` | Read the file as UTF-8 text. |
| `fs::read_bytes(path)` | Read the file as raw `bytes`. |
| `fs::write(path, contents)` | Write `contents` (string or bytes) to `path`, replacing the existing file. |
| `fs::append(path, contents)` | Append to (or create) `path`. |
| `fs::exists(path)` | `true` if `path` exists. |
| `fs::is_file(path)` | `true` if `path` is an existing regular file. |
| `fs::is_dir(path)` | `true` if `path` is an existing directory. |
| `fs::list_dir(path)` | List the entry names directly under `path` (non-recursive). |
| `fs::create_dir(path)` | Create one directory; errors if a parent is missing. |
| `fs::create_dir_all(path)` | Create the directory and any missing parents. |
| `fs::remove_file(path)` | Delete the file at `path`. |
| `fs::remove_dir(path)` | Delete the directory at `path`; errors if not empty. |
| `fs::remove_dir_all(path)` | Recursively delete `path` and its contents. |
| `fs::rename(from, to)` | Rename / move `from` to `to`. |
| `fs::copy(from, to)` | Copy `from` to `to`; returns bytes copied. |
| `fs::metadata(path)` | Returns `#{size, is_file, is_dir, readonly, modified}`. |
| `fs::canonicalize(path)` | Resolve symlinks against the filesystem and normalise. |

```
use fs::*;
use path::*;

let cfg_path = join(os::cwd(), "config.toml");
if exists(cfg_path) {
    let raw = read(cfg_path);
    io::println("loaded:", raw);
} else {
    write(cfg_path, "key = \"value\"");
}

for name in list_dir(os::cwd()) {
    if extension(name) == "ion" {
        io::println(name);
    }
}
```

The `fs` feature is enabled by default. Disable it with
`default-features = false` for sandboxed embedders. Note that
`Engine::eval` is removed under `async-runtime` — use `Engine::eval_async`
in async builds. The two are deliberately mutually exclusive so non-coloured
calls like `fs::read` resolve to one implementation per build.

### Registering modules (Rust side)

```rust
use ion_core::module::Module;

let mut math = Module::new("math");
math.register_fn("add", |args| { /* ... */ });
math.set("PI", Value::Float(std::f64::consts::PI));

let mut engine = Engine::new();
engine.register_module(math);
```

With the `async-runtime` feature, modules can expose native async host
functions. Ion code calls them normally, but the host must use `eval_async`:

```rust
let mut sensor = Module::new("sensor");
sensor.register_async_fn("call", |args| async move {
    Ok(Value::Int(args.len() as i64))
});

let mut engine = Engine::new();
engine.register_module(sensor);

let value = engine.eval_async(r#"sensor::call("jobs.claim", #{})"#).await?;
```

Submodules:
```rust
let mut net = Module::new("net");
let mut http = Module::new("http");
http.register_fn("get", |args| { /* ... */ });
net.register_submodule(http);
engine.register_module(net);
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

### Register modules

```rust
use ion_core::module::Module;

let mut math = Module::new("math");
math.register_fn("add", |args| {
    let (a, b) = (args[0].as_int().unwrap(), args[1].as_int().unwrap());
    Ok(Value::Int(a + b))
});
math.set("PI", Value::Float(std::f64::consts::PI));
engine.register_module(math);
```

Scripts can then use `math::add(1, 2)` or `use math::*;`.

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
| `vm` | Yes | Bytecode VM path (`vm_eval`) with peephole optimization, constant folding, DCE, and TCO |
| `derive` | Yes | `#[derive(IonType)]` proc macro |
| `async-runtime` | No | Native Tokio async evaluation (`eval_async`, async host functions, timers, channels) |
| `legacy-threaded-concurrency` | No | Legacy sync-eval backend using OS threads and crossbeam channels |
| `obfuscate` | No | String obfuscation via `obfstr` |
