# Ion Standard Library Reference

All methods implemented in both interpreter and VM with cross-validation tests.

## String Methods (28)
len, contains, starts_with, ends_with, trim, trim_start, trim_end,
to_upper, to_lower, split, replace, chars, is_empty, repeat, find,
to_int, to_float, reverse, slice, bytes, char_len, pad_start, pad_end,
strip_prefix, strip_suffix, to_string, index

## List Methods (30)
len, push, pop, map, filter, fold, flat_map, any, all, first, last,
reverse, sort, sort_by, flatten, zip, contains, join, enumerate, is_empty,
index, count, slice, dedup, unique, min, max, sum, window, to_string

## Dict Methods (15)
len, keys, values, entries, contains_key, get, insert, remove, merge,
is_empty, map, filter, update, keys_of, to_string

## Option Methods (9)
is_some, is_none, unwrap, unwrap_or, unwrap_or_else, expect, map,
and_then, or_else

## Result Methods (10)
is_ok, is_err, unwrap, unwrap_or, unwrap_or_else, expect, map, map_err,
and_then, or_else

## Tuple Methods (3)
len, contains, to_list

## Bytes Methods (33)
len, is_empty, bytes, contains, find, count, starts_with, ends_with,
slice, split, replace, reverse, repeat, push, extend, set, pop,
to_list, to_str, to_hex, to_base64,
read_u16_le, read_u16_be, read_u32_le, read_u32_be, read_u64_le, read_u64_be,
read_i16_le, read_i16_be, read_i32_le, read_i32_be, read_i64_le, read_i64_be

## Cell Methods (3)
get, set, update

## Native Async Runtime Builtins

Available under `Engine::eval_async` with the `async-runtime` feature:

- `sleep(ms)` — park on a Tokio timer and return `()`.
- `timeout(ms, fn)` — run `fn` as a pollable callback; return `Some(value)`
  if it finishes before the timer or `None` if the timer wins.
- `channel(size)` — create a bounded Tokio-backed channel `(tx, rx)`.

Channel endpoint methods:

- `tx.send(value)` — send, parking if the channel is full.
- `tx.close()` — close the sender endpoint.
- `rx.recv()` — park until a value arrives; return `None` after close.
- `rx.try_recv()` — immediate receive attempt, returning `Option`.
- `rx.recv_timeout(ms)` — receive with a Tokio timer, returning `Option`.

## Global Builtins (30)
print, println, len, range, enumerate, join, type_of, str, int, float,
json_encode, json_decode, json_encode_pretty, bytes, bytes_from_hex,
assert, assert_eq, sort_by, channel, cell, sleep, timeout,
abs, min, max, floor, ceil, round, sqrt, pow, clamp

## Bytes Module (21)
new, zeroed, repeat, from_list, from_str, from_hex, from_base64, concat, join,
u16_le, u16_be, u32_le, u32_be, u64_le, u64_be,
i16_le, i16_be, i32_le, i32_be, i64_le, i64_be
