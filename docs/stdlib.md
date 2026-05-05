# Ion Standard Library Reference

All methods implemented in both interpreter and VM with cross-validation tests.

## String Methods (28)
len, contains, starts_with, ends_with, trim, trim_start, trim_end,
to_upper, to_lower, split, replace, chars, is_empty, repeat, find,
index, count, to_int, to_float, reverse, slice, bytes, char_len,
pad_start, pad_end, strip_prefix, strip_suffix, to_string

## Scalar And Universal Methods
Every runtime value supports `to_string`; scalar docs list it on int, float,
bool, fn, and any.

## List Methods (32)
len, is_empty, contains, push, pop, first, last, reverse, sort, sort_by,
flatten, join, enumerate, zip, map, filter, fold, flat_map, any, all,
index, count, slice, dedup, unique, sum, window, chunk, reduce, min, max,
to_string

## Dict Methods (16)
len, keys, values, entries, contains_key, get, insert, remove, merge,
is_empty, map, filter, update, keys_of, zip, to_string

## Option Methods (10)
is_some, is_none, unwrap, unwrap_or, unwrap_or_else, expect, map,
and_then, or_else, to_string

## Result Methods (11)
is_ok, is_err, unwrap, unwrap_or, unwrap_or_else, expect, map, map_err,
and_then, or_else, to_string

## Tuple Methods (4)
len, contains, to_list, to_string

## Set Methods (10)
len, contains, is_empty, add, remove, union, intersection, difference,
to_list, to_string

## Range Methods (4)
len, contains, to_list, to_string

## Bytes Methods (34)
len, is_empty, bytes, contains, find, count, starts_with, ends_with,
slice, split, replace, reverse, repeat, push, extend, set, pop,
to_list, to_str, to_hex, to_base64, to_string,
read_u16_le, read_u16_be, read_u32_le, read_u32_be, read_u64_le, read_u64_be,
read_i16_le, read_i16_be, read_i32_le, read_i32_be, read_i64_le, read_i64_be

## Cell Methods (4)
get, set, update, to_string

## Task Methods (6)
await, is_finished, cancel, is_cancelled, await_timeout, to_string

## Channel Methods (6)
send, recv, try_recv, recv_timeout, close, to_string

## AsyncTask Methods (2)
await, to_string

## AsyncChannelSender Methods (3)
send, close, to_string

## AsyncChannelReceiver Methods (5)
recv, try_recv, recv_timeout, close, to_string

## Native Async Runtime Builtins

Available under `Engine::eval_async` with the `async-runtime` feature:

- `sleep(ms)` — park on a Tokio timer and return `()`.
- `timeout(ms, fn)` — run `fn` as a pollable callback; return `Some(value)`
  if it finishes before the timer or `None` if the timer wins.
- `channel(size)` — create a bounded Tokio-backed channel `(tx, rx)`.

Channel endpoint methods:

- `tx.send(value)` — send, parking if the channel is full.
- `tx.close()` — close the sender endpoint.
- `tx.to_string()` — convert the sender endpoint to a string.
- `rx.recv()` — park until a value arrives; return `None` after close.
- `rx.try_recv()` — immediate receive attempt, returning `Option`.
- `rx.recv_timeout(ms)` — receive with a Tokio timer, returning `Option`.
- `rx.close()` — close the receiver endpoint.
- `rx.to_string()` — convert the receiver endpoint to a string.

Task handle methods:

- `task.await` — wait for a task result.
- `task.await_timeout(ms)` — wait up to `ms`, returning `Option`.
- `task.is_finished()` — check whether the task has completed.
- `task.cancel()` — cancel the task.
- `task.is_cancelled()` — check whether the task was cancelled.
- `task.to_string()` — convert the task handle to a string.

## Core Builtins (16)
len, range, enumerate, type_of, str, int, float, bytes, bytes_from_hex,
assert, assert_eq, channel, set, cell, sleep, timeout

## Math Module (23)
PI, E, TAU, INF, NAN, abs, min, max, floor, ceil, round, sqrt, pow, clamp,
sin, cos, tan, atan2, log, log2, log10, is_nan, is_inf

## Json Module (5)
encode, decode, pretty, msgpack_encode, msgpack_decode

## Bytes Module (21)
new, zeroed, repeat, from_list, from_str, from_hex, from_base64, concat, join,
u16_le, u16_be, u32_le, u32_be, u64_le, u64_be,
i16_le, i16_be, i32_le, i32_be, i64_le, i64_be

## Io Module (3)
print, println, eprintln

## Rand Module (7)
int, float, bool, bytes, choice, shuffle, sample

## String Module (29)
len, char_len, is_empty, contains, starts_with, ends_with, find, index,
count, trim, trim_start, trim_end, to_upper, to_lower, split, replace,
chars, pad_start, pad_end, strip_prefix, strip_suffix, reverse, repeat,
slice, bytes, to_int, to_float, to_string, join

## Log Module (8)
trace, debug, info, warn, error, set_level, level, enabled

## Semver Module (13)
parse, is_valid, format, compare, eq, gt, gte, lt, lte, satisfies,
bump_major, bump_minor, bump_patch

## Os Module (13)
name, arch, family, pointer_width, dll_extension, exe_extension, env_var,
has_env_var, env_vars, cwd, pid, args, temp_dir

## Path Module (11)
sep, join, parent, basename, stem, extension, with_extension, is_absolute,
is_relative, components, normalize

## Fs Module (19)
read, read_bytes, write, append, append_random, pad_random,
exists, is_file, is_dir, list_dir, create_dir, create_dir_all,
remove_file, remove_dir, remove_dir_all, rename, copy, metadata, canonicalize
