//! Built-in standard library modules.
//!
//! These modules are automatically registered in every Engine instance
//! and provide namespaced access to common functions and constants.
//! The same functions remain available as top-level builtins for
//! backwards compatibility.

use std::sync::Arc;

use crate::module::Module;
use crate::value::Value;

#[cfg(feature = "semver")]
use semver::{BuildMetadata, Prerelease, Version, VersionReq};

const MAX_BYTES_LEN: usize = 10_000_000;
#[cfg(feature = "fs")]
const FS_RANDOM_CHUNK_LEN: usize = 8 * 1024 * 1024;
const BASE64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

#[derive(Debug, Clone, Copy)]
pub(crate) enum ByteOrder {
    Little,
    Big,
}

/// Output stream requested by Ion's `io` stdlib module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// Host-side output handler for Ion's `io::print*` functions.
///
/// Embedders install an implementation with `Engine::set_output` or
/// `Engine::with_output`. The core runtime never writes directly to process
/// stdout/stderr through `io::print*`.
pub trait OutputHandler: Send + Sync {
    fn write(&self, stream: OutputStream, text: &str) -> Result<(), String>;
}

/// Output handler that writes to the process stdout/stderr streams.
#[derive(Debug, Default, Clone, Copy)]
pub struct StdOutput;

impl OutputHandler for StdOutput {
    fn write(&self, stream: OutputStream, text: &str) -> Result<(), String> {
        use std::io::Write;

        match stream {
            OutputStream::Stdout => {
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(text.as_bytes()).map_err(|e| e.to_string())
            }
            OutputStream::Stderr => {
                let mut stderr = std::io::stderr().lock();
                stderr.write_all(text.as_bytes()).map_err(|e| e.to_string())
            }
        }
    }
}

struct MissingOutputHandler;

impl OutputHandler for MissingOutputHandler {
    fn write(&self, _stream: OutputStream, _text: &str) -> Result<(), String> {
        Err(ion_str!(
            "io output handler is not configured; call Engine::set_output"
        ))
    }
}

pub(crate) fn missing_output_handler() -> Arc<dyn OutputHandler> {
    Arc::new(MissingOutputHandler)
}

fn ok_value(value: Value) -> Value {
    Value::Result(Ok(Box::new(value)))
}

fn err_value(message: String) -> Value {
    Value::Result(Err(Box::new(Value::Str(message))))
}

fn result_bytes(result: Result<Vec<u8>, String>) -> Value {
    match result {
        Ok(bytes) => ok_value(Value::Bytes(bytes)),
        Err(message) => err_value(message),
    }
}

fn result_int(result: Result<i64, String>) -> Value {
    match result {
        Ok(value) => ok_value(Value::Int(value)),
        Err(message) => err_value(message),
    }
}

fn check_bytes_len(len: usize, context: &str) -> Result<(), String> {
    if len > MAX_BYTES_LEN {
        return Err(format!(
            "{}{}{}{}",
            context,
            ion_str!(" would create "),
            len,
            ion_str!(" bytes")
        ));
    }
    Ok(())
}

pub(crate) fn byte_from_int(value: i64, context: &str) -> Result<u8, String> {
    if !(0..=255).contains(&value) {
        return Err(format!(
            "{}{}{}",
            context,
            ion_str!(" byte value out of range: "),
            value
        ));
    }
    Ok(value as u8)
}

fn byte_from_value(value: &Value, context: &str) -> Result<u8, String> {
    let Some(value) = value.as_int() else {
        return Err(format!("{}{}", context, ion_str!(" requires an int byte")));
    };
    byte_from_int(value, context)
}

fn bytes_from_list(items: &[Value], context: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(items.len());
    for item in items {
        bytes.push(byte_from_value(item, context)?);
    }
    Ok(bytes)
}

fn required_int_arg(args: &[Value], index: usize, context: &str) -> Result<i64, String> {
    args.get(index)
        .and_then(Value::as_int)
        .ok_or_else(|| format!("{}{}", context, ion_str!(" requires an int argument")))
}

fn optional_int_arg(
    args: &[Value],
    index: usize,
    default: i64,
    context: &str,
) -> Result<i64, String> {
    match args.get(index) {
        Some(value) => value
            .as_int()
            .ok_or_else(|| format!("{}{}", context, ion_str!(" requires an int argument"))),
        None => Ok(default),
    }
}

fn required_bytes_arg<'a>(
    args: &'a [Value],
    index: usize,
    context: &str,
) -> Result<&'a [u8], String> {
    match args.get(index) {
        Some(Value::Bytes(bytes)) => Ok(bytes),
        Some(other) => Err(format!(
            "{}{}{}",
            context,
            ion_str!(" requires bytes, got "),
            other.type_name()
        )),
        None => Err(format!("{}{}", context, ion_str!(" requires bytes"))),
    }
}

fn byte_pattern(value: &Value, context: &str) -> Result<Vec<u8>, String> {
    match value {
        Value::Int(value) => byte_from_int(*value, context).map(|byte| vec![byte]),
        Value::Bytes(bytes) => Ok(bytes.clone()),
        other => Err(format!(
            "{}{}{}",
            context,
            ion_str!(" requires int or bytes, got "),
            other.type_name()
        )),
    }
}

fn bytes_constructor(args: &[Value], context: &str) -> Result<Value, String> {
    match args.len() {
        0 => Ok(Value::Bytes(Vec::new())),
        1 => match &args[0] {
            Value::List(items) => bytes_from_list(items, context).map(Value::Bytes),
            Value::Str(value) => Ok(Value::Bytes(value.as_bytes().to_vec())),
            Value::Int(value) if *value >= 0 => {
                let len = *value as usize;
                if len > MAX_BYTES_LEN {
                    return Err(format!("{}{}", ion_str!("invalid byte count: "), value));
                }
                check_bytes_len(len, context)?;
                Ok(Value::Bytes(vec![0u8; len]))
            }
            Value::Int(value) => Err(format!("{}{}", ion_str!("invalid byte count: "), value)),
            other => Err(format!(
                "{}{}",
                ion_str!("bytes() not supported for "),
                other.type_name()
            )),
        },
        _ => Err(format!("{}{}", context, ion_str!(" takes 0 or 1 argument"))),
    }
}

pub(crate) fn bytes_builtin(args: &[Value]) -> Result<Value, String> {
    bytes_constructor(args, "bytes")
}

pub(crate) fn call_stdlib_module(
    table: &crate::module::ModuleTable,
    args: &[Value],
) -> Option<Result<Value, String>> {
    if table.name_hash == crate::h!("bytes") {
        Some(bytes_builtin(args))
    } else {
        None
    }
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn bytes_from_hex_string(value: &str) -> Result<Vec<u8>, String> {
    if !value.is_ascii() {
        return Err(ion_str!("hex string must be ASCII"));
    }
    let raw = value.as_bytes();
    if raw.len() % 2 != 0 {
        return Err(ion_str!("hex string must have even length"));
    }
    let mut bytes = Vec::with_capacity(raw.len() / 2);
    for chunk in raw.chunks_exact(2) {
        let hi = hex_nibble(chunk[0]).ok_or_else(|| {
            format!(
                "{}{}{}",
                ion_str!("invalid hex byte: "),
                chunk[0] as char,
                chunk[1] as char
            )
        })?;
        let lo = hex_nibble(chunk[1]).ok_or_else(|| {
            format!(
                "{}{}{}",
                ion_str!("invalid hex byte: "),
                chunk[0] as char,
                chunk[1] as char
            )
        })?;
        bytes.push((hi << 4) | lo);
    }
    Ok(bytes)
}

pub(crate) fn bytes_from_hex_builtin(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(ion_str!("bytes_from_hex takes 1 argument"));
    }
    let value = args[0]
        .as_str()
        .ok_or_else(|| ion_str!("bytes_from_hex requires a string"))?;
    bytes_from_hex_string(value).map(Value::Bytes)
}

pub(crate) fn bytes_to_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let triple = ((b0 as u32) << 16) | ((b1 as u32) << 8) | b2 as u32;
        out.push(BASE64_TABLE[((triple >> 18) & 0x3f) as usize] as char);
        out.push(BASE64_TABLE[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(BASE64_TABLE[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(BASE64_TABLE[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

pub(crate) fn bytes_from_base64_string(value: &str) -> Result<Vec<u8>, String> {
    if !value.is_ascii() {
        return Err(ion_str!("base64 string must be ASCII"));
    }
    let input: Vec<u8> = value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if input.len() % 4 == 1 {
        return Err(ion_str!("invalid base64 length"));
    }

    let mut out = Vec::with_capacity((input.len() / 4) * 3);
    let mut index = 0;
    while index < input.len() {
        let remaining = input.len() - index;
        let take = remaining.min(4);
        let chunk = &input[index..index + take];
        let final_chunk = index + take == input.len();
        let mut values = [0u8; 4];
        let mut padding = 0usize;

        for pos in 0..take {
            match chunk[pos] {
                b'=' => {
                    if !final_chunk || take != 4 || pos < 2 {
                        return Err(ion_str!("invalid base64 padding"));
                    }
                    padding += 1;
                    values[pos] = 0;
                }
                byte => {
                    if padding > 0 {
                        return Err(ion_str!("invalid base64 padding"));
                    }
                    values[pos] = base64_value(byte).ok_or_else(|| {
                        format!("{}{}", ion_str!("invalid base64 character: "), byte as char)
                    })?;
                }
            }
        }

        if padding > 0 {
            if padding > 2 || (padding == 2 && chunk[2] != b'=') {
                return Err(ion_str!("invalid base64 padding"));
            }
        } else if take < 4 && !final_chunk {
            return Err(ion_str!("invalid base64 length"));
        }

        let triple = ((values[0] as u32) << 18)
            | ((values[1] as u32) << 12)
            | ((values[2] as u32) << 6)
            | values[3] as u32;
        out.push(((triple >> 16) & 0xff) as u8);
        if take > 2 && padding < 2 {
            out.push(((triple >> 8) & 0xff) as u8);
        }
        if take > 3 && padding < 1 {
            out.push((triple & 0xff) as u8);
        }

        index += take;
    }

    Ok(out)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn count_subslice(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() {
        return haystack.len() + 1;
    }
    let mut count = 0;
    let mut index = 0;
    while index <= haystack.len() {
        let Some(pos) = find_subslice(&haystack[index..], needle) else {
            break;
        };
        count += 1;
        index += pos + needle.len();
    }
    count
}

fn split_subslice(haystack: &[u8], sep: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    if sep.is_empty() {
        return Err(ion_str!("bytes.split() separator must not be empty"));
    }
    let mut parts = Vec::new();
    let mut index = 0;
    while index <= haystack.len() {
        let Some(pos) = find_subslice(&haystack[index..], sep) else {
            parts.push(haystack[index..].to_vec());
            break;
        };
        parts.push(haystack[index..index + pos].to_vec());
        index += pos + sep.len();
    }
    Ok(parts)
}

fn replace_subslice(haystack: &[u8], from: &[u8], to: &[u8]) -> Result<Vec<u8>, String> {
    if from.is_empty() {
        return Err(ion_str!("bytes.replace() needle must not be empty"));
    }
    let mut out = Vec::with_capacity(haystack.len());
    let mut index = 0;
    while index <= haystack.len() {
        let Some(pos) = find_subslice(&haystack[index..], from) else {
            let new_len = out.len() + haystack.len() - index;
            check_bytes_len(new_len, "bytes.replace()")?;
            out.extend_from_slice(&haystack[index..]);
            break;
        };
        let new_len = out.len() + pos + to.len();
        check_bytes_len(new_len, "bytes.replace()")?;
        out.extend_from_slice(&haystack[index..index + pos]);
        out.extend_from_slice(to);
        index += pos + from.len();
    }
    Ok(out)
}

fn normalize_index(index: i64, len: usize) -> Option<usize> {
    let index = if index < 0 { len as i64 + index } else { index };
    if index < 0 || index >= len as i64 {
        None
    } else {
        Some(index as usize)
    }
}

fn normalize_slice_bound(index: i64, len: usize) -> usize {
    let index = if index < 0 { len as i64 + index } else { index };
    index.max(0).min(len as i64) as usize
}

fn read_offset(args: &[Value], context: &str) -> Result<usize, String> {
    let offset = required_int_arg(args, 0, context)?;
    if offset < 0 {
        return Err(format!(
            "{}{}",
            context,
            ion_str!(" offset must be non-negative")
        ));
    }
    Ok(offset as usize)
}

fn read_bytes_window<'a>(
    bytes: &'a [u8],
    args: &[Value],
    width: usize,
    context: &str,
) -> Result<&'a [u8], String> {
    let offset = read_offset(args, context)?;
    let end = offset
        .checked_add(width)
        .ok_or_else(|| format!("{}{}", context, ion_str!(" offset overflow")))?;
    if end > bytes.len() {
        return Err(format!(
            "{}{}",
            context,
            ion_str!(" read past end of bytes")
        ));
    }
    Ok(&bytes[offset..end])
}

pub(crate) fn read_unsigned(
    bytes: &[u8],
    args: &[Value],
    width: usize,
    order: ByteOrder,
    context: &str,
) -> Result<i64, String> {
    let window = read_bytes_window(bytes, args, width, context)?;
    match (width, order) {
        (2, ByteOrder::Little) => Ok(u16::from_le_bytes([window[0], window[1]]) as i64),
        (2, ByteOrder::Big) => Ok(u16::from_be_bytes([window[0], window[1]]) as i64),
        (4, ByteOrder::Little) => {
            Ok(u32::from_le_bytes([window[0], window[1], window[2], window[3]]) as i64)
        }
        (4, ByteOrder::Big) => {
            Ok(u32::from_be_bytes([window[0], window[1], window[2], window[3]]) as i64)
        }
        (8, ByteOrder::Little) => {
            let value = u64::from_le_bytes([
                window[0], window[1], window[2], window[3], window[4], window[5], window[6],
                window[7],
            ]);
            if value > i64::MAX as u64 {
                Err(format!(
                    "{}{}",
                    context,
                    ion_str!(" u64 value does not fit in int")
                ))
            } else {
                Ok(value as i64)
            }
        }
        (8, ByteOrder::Big) => {
            let value = u64::from_be_bytes([
                window[0], window[1], window[2], window[3], window[4], window[5], window[6],
                window[7],
            ]);
            if value > i64::MAX as u64 {
                Err(format!(
                    "{}{}",
                    context,
                    ion_str!(" u64 value does not fit in int")
                ))
            } else {
                Ok(value as i64)
            }
        }
        _ => Err(ion_str!("unsupported integer width")),
    }
}

pub(crate) fn read_signed(
    bytes: &[u8],
    args: &[Value],
    width: usize,
    order: ByteOrder,
    context: &str,
) -> Result<i64, String> {
    let window = read_bytes_window(bytes, args, width, context)?;
    match (width, order) {
        (2, ByteOrder::Little) => Ok(i16::from_le_bytes([window[0], window[1]]) as i64),
        (2, ByteOrder::Big) => Ok(i16::from_be_bytes([window[0], window[1]]) as i64),
        (4, ByteOrder::Little) => {
            Ok(i32::from_le_bytes([window[0], window[1], window[2], window[3]]) as i64)
        }
        (4, ByteOrder::Big) => {
            Ok(i32::from_be_bytes([window[0], window[1], window[2], window[3]]) as i64)
        }
        (8, ByteOrder::Little) => Ok(i64::from_le_bytes([
            window[0], window[1], window[2], window[3], window[4], window[5], window[6], window[7],
        ])),
        (8, ByteOrder::Big) => Ok(i64::from_be_bytes([
            window[0], window[1], window[2], window[3], window[4], window[5], window[6], window[7],
        ])),
        _ => Err(ion_str!("unsupported integer width")),
    }
}

fn unsigned_bounds(width: usize) -> (i64, i64) {
    match width {
        2 => (0, u16::MAX as i64),
        4 => (0, u32::MAX as i64),
        8 => (0, i64::MAX),
        _ => (0, 0),
    }
}

fn signed_bounds(width: usize) -> (i64, i64) {
    match width {
        2 => (i16::MIN as i64, i16::MAX as i64),
        4 => (i32::MIN as i64, i32::MAX as i64),
        8 => (i64::MIN, i64::MAX),
        _ => (0, 0),
    }
}

fn pack_unsigned(
    args: &[Value],
    width: usize,
    order: ByteOrder,
    context: &str,
) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!("{}{}", context, ion_str!(" takes 1 argument")));
    }
    let value = required_int_arg(args, 0, context)?;
    let (min, max) = unsigned_bounds(width);
    if value < min || value > max {
        return Err(format!(
            "{}{}{}{}{}",
            context,
            ion_str!(" requires value in "),
            min,
            ion_str!("..="),
            max
        ));
    }
    let bytes = match (width, order) {
        (2, ByteOrder::Little) => (value as u16).to_le_bytes().to_vec(),
        (2, ByteOrder::Big) => (value as u16).to_be_bytes().to_vec(),
        (4, ByteOrder::Little) => (value as u32).to_le_bytes().to_vec(),
        (4, ByteOrder::Big) => (value as u32).to_be_bytes().to_vec(),
        (8, ByteOrder::Little) => (value as u64).to_le_bytes().to_vec(),
        (8, ByteOrder::Big) => (value as u64).to_be_bytes().to_vec(),
        _ => return Err(ion_str!("unsupported integer width")),
    };
    Ok(Value::Bytes(bytes))
}

fn pack_signed(
    args: &[Value],
    width: usize,
    order: ByteOrder,
    context: &str,
) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!("{}{}", context, ion_str!(" takes 1 argument")));
    }
    let value = required_int_arg(args, 0, context)?;
    let (min, max) = signed_bounds(width);
    if value < min || value > max {
        return Err(format!(
            "{}{}{}{}{}",
            context,
            ion_str!(" requires value in "),
            min,
            ion_str!("..="),
            max
        ));
    }
    let bytes = match (width, order) {
        (2, ByteOrder::Little) => (value as i16).to_le_bytes().to_vec(),
        (2, ByteOrder::Big) => (value as i16).to_be_bytes().to_vec(),
        (4, ByteOrder::Little) => (value as i32).to_le_bytes().to_vec(),
        (4, ByteOrder::Big) => (value as i32).to_be_bytes().to_vec(),
        (8, ByteOrder::Little) => value.to_le_bytes().to_vec(),
        (8, ByteOrder::Big) => value.to_be_bytes().to_vec(),
        _ => return Err(ion_str!("unsupported integer width")),
    };
    Ok(Value::Bytes(bytes))
}

pub(crate) fn bytes_method_value(
    bytes: &[u8],
    method_hash: u64,
    args: &[Value],
) -> Result<Option<Value>, String> {
    let value = match method_hash {
        h if h == crate::h!("len") => Value::Int(bytes.len() as i64),
        h if h == crate::h!("is_empty") => Value::Bool(bytes.is_empty()),
        h if h == crate::h!("bytes") => Value::Bytes(bytes.to_vec()),
        h if h == crate::h!("contains") => {
            let needle = args
                .first()
                .ok_or_else(|| ion_str!("bytes.contains() requires an argument"))
                .and_then(|value| byte_pattern(value, "bytes.contains()"))?;
            Value::Bool(if needle.is_empty() {
                true
            } else {
                find_subslice(bytes, &needle).is_some()
            })
        }
        h if h == crate::h!("find") => {
            let needle = args
                .first()
                .ok_or_else(|| ion_str!("bytes.find() requires an argument"))
                .and_then(|value| byte_pattern(value, "bytes.find()"))?;
            match find_subslice(bytes, &needle) {
                Some(index) => Value::Option(Some(Box::new(Value::Int(index as i64)))),
                None => Value::Option(None),
            }
        }
        h if h == crate::h!("count") => {
            let needle = args
                .first()
                .ok_or_else(|| ion_str!("bytes.count() requires an argument"))
                .and_then(|value| byte_pattern(value, "bytes.count()"))?;
            Value::Int(count_subslice(bytes, &needle) as i64)
        }
        h if h == crate::h!("starts_with") => {
            let needle = args
                .first()
                .ok_or_else(|| ion_str!("bytes.starts_with() requires an argument"))
                .and_then(|value| byte_pattern(value, "bytes.starts_with()"))?;
            Value::Bool(bytes.starts_with(&needle))
        }
        h if h == crate::h!("ends_with") => {
            let needle = args
                .first()
                .ok_or_else(|| ion_str!("bytes.ends_with() requires an argument"))
                .and_then(|value| byte_pattern(value, "bytes.ends_with()"))?;
            Value::Bool(bytes.ends_with(&needle))
        }
        h if h == crate::h!("slice") => {
            let start = optional_int_arg(args, 0, 0, "bytes.slice()")?;
            let end = optional_int_arg(args, 1, bytes.len() as i64, "bytes.slice()")?;
            let start = normalize_slice_bound(start, bytes.len());
            let end = normalize_slice_bound(end, bytes.len());
            if start > end {
                Value::Bytes(Vec::new())
            } else {
                Value::Bytes(bytes[start..end].to_vec())
            }
        }
        h if h == crate::h!("split") => {
            let sep = args
                .first()
                .ok_or_else(|| ion_str!("bytes.split() requires an argument"))
                .and_then(|value| byte_pattern(value, "bytes.split()"))?;
            Value::List(
                split_subslice(bytes, &sep)?
                    .into_iter()
                    .map(Value::Bytes)
                    .collect(),
            )
        }
        h if h == crate::h!("replace") => {
            if args.len() < 2 {
                return Err(ion_str!("bytes.replace() requires from and to"));
            }
            let from = byte_pattern(&args[0], "bytes.replace()")?;
            let to = byte_pattern(&args[1], "bytes.replace()")?;
            Value::Bytes(replace_subslice(bytes, &from, &to)?)
        }
        h if h == crate::h!("reverse") => {
            let mut reversed = bytes.to_vec();
            reversed.reverse();
            Value::Bytes(reversed)
        }
        h if h == crate::h!("repeat") => {
            let n = required_int_arg(args, 0, "bytes.repeat()")?;
            if n < 0 {
                return Err(ion_str!("bytes.repeat() count must be non-negative"));
            }
            let len = bytes
                .len()
                .checked_mul(n as usize)
                .ok_or_else(|| ion_str!("bytes.repeat() length overflow"))?;
            check_bytes_len(len, "bytes.repeat()")?;
            Value::Bytes(bytes.repeat(n as usize))
        }
        h if h == crate::h!("push") => {
            let byte = args
                .first()
                .ok_or_else(|| ion_str!("bytes.push() requires a byte"))
                .and_then(|value| byte_from_value(value, "bytes.push()"))?;
            let new_len = bytes.len() + 1;
            check_bytes_len(new_len, "bytes.push()")?;
            let mut out = bytes.to_vec();
            out.push(byte);
            Value::Bytes(out)
        }
        h if h == crate::h!("extend") => {
            let other = required_bytes_arg(args, 0, "bytes.extend()")?;
            let new_len = bytes.len() + other.len();
            check_bytes_len(new_len, "bytes.extend()")?;
            let mut out = Vec::with_capacity(new_len);
            out.extend_from_slice(bytes);
            out.extend_from_slice(other);
            Value::Bytes(out)
        }
        h if h == crate::h!("set") => {
            if args.len() < 2 {
                return Err(ion_str!("bytes.set() requires index and byte"));
            }
            let index = required_int_arg(args, 0, "bytes.set()")?;
            let byte = byte_from_value(&args[1], "bytes.set()")?;
            let Some(index) = normalize_index(index, bytes.len()) else {
                return Err(ion_str!("bytes.set() index out of bounds"));
            };
            let mut out = bytes.to_vec();
            out[index] = byte;
            Value::Bytes(out)
        }
        h if h == crate::h!("pop") => {
            if bytes.is_empty() {
                Value::Tuple(vec![Value::Bytes(Vec::new()), Value::Option(None)])
            } else {
                let mut out = bytes.to_vec();
                let byte = out.pop().unwrap();
                Value::Tuple(vec![
                    Value::Bytes(out),
                    Value::Option(Some(Box::new(Value::Int(byte as i64)))),
                ])
            }
        }
        h if h == crate::h!("to_list") => Value::List(
            bytes
                .iter()
                .copied()
                .map(|byte| Value::Int(byte as i64))
                .collect(),
        ),
        h if h == crate::h!("to_str") => match std::str::from_utf8(bytes) {
            Ok(value) => ok_value(Value::Str(value.to_string())),
            Err(err) => err_value(err.to_string()),
        },
        h if h == crate::h!("to_hex") => {
            Value::Str(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
        }
        h if h == crate::h!("to_base64") => Value::Str(bytes_to_base64(bytes)),
        h if h == crate::h!("read_u16_le") => result_int(read_unsigned(
            bytes,
            args,
            2,
            ByteOrder::Little,
            "read_u16_le",
        )),
        h if h == crate::h!("read_u16_be") => {
            result_int(read_unsigned(bytes, args, 2, ByteOrder::Big, "read_u16_be"))
        }
        h if h == crate::h!("read_u32_le") => result_int(read_unsigned(
            bytes,
            args,
            4,
            ByteOrder::Little,
            "read_u32_le",
        )),
        h if h == crate::h!("read_u32_be") => {
            result_int(read_unsigned(bytes, args, 4, ByteOrder::Big, "read_u32_be"))
        }
        h if h == crate::h!("read_u64_le") => result_int(read_unsigned(
            bytes,
            args,
            8,
            ByteOrder::Little,
            "read_u64_le",
        )),
        h if h == crate::h!("read_u64_be") => {
            result_int(read_unsigned(bytes, args, 8, ByteOrder::Big, "read_u64_be"))
        }
        h if h == crate::h!("read_i16_le") => result_int(read_signed(
            bytes,
            args,
            2,
            ByteOrder::Little,
            "read_i16_le",
        )),
        h if h == crate::h!("read_i16_be") => {
            result_int(read_signed(bytes, args, 2, ByteOrder::Big, "read_i16_be"))
        }
        h if h == crate::h!("read_i32_le") => result_int(read_signed(
            bytes,
            args,
            4,
            ByteOrder::Little,
            "read_i32_le",
        )),
        h if h == crate::h!("read_i32_be") => {
            result_int(read_signed(bytes, args, 4, ByteOrder::Big, "read_i32_be"))
        }
        h if h == crate::h!("read_i64_le") => result_int(read_signed(
            bytes,
            args,
            8,
            ByteOrder::Little,
            "read_i64_le",
        )),
        h if h == crate::h!("read_i64_be") => {
            result_int(read_signed(bytes, args, 8, ByteOrder::Big, "read_i64_be"))
        }
        _ => return Ok(None),
    };
    Ok(Some(value))
}

struct RandomSource {
    state: u64,
    #[cfg(unix)]
    file: Option<std::fs::File>,
}

impl RandomSource {
    fn new() -> Self {
        Self {
            state: fallback_random_seed(),
            #[cfg(unix)]
            file: std::fs::File::open("/dev/urandom").ok(),
        }
    }

    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        #[cfg(unix)]
        {
            use std::io::Read;

            if let Some(file) = self.file.as_mut() {
                if file.read_exact(bytes).is_ok() {
                    return;
                }
            }
            self.file = None;
        }

        for chunk in bytes.chunks_mut(8) {
            let random = splitmix64_next(&mut self.state).to_le_bytes();
            chunk.copy_from_slice(&random[..chunk.len()]);
        }
    }

    fn u64(&mut self) -> u64 {
        let mut bytes = [0u8; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    fn below(&mut self, upper: u64) -> Result<u64, String> {
        if upper == 0 {
            return Err(ion_str!("upper bound must be positive"));
        }
        if upper == 1 {
            return Ok(0);
        }
        let zone = u64::MAX - (u64::MAX % upper);
        loop {
            let value = self.u64();
            if value < zone {
                return Ok(value % upper);
            }
        }
    }

    fn unit_float(&mut self) -> f64 {
        const SCALE: f64 = 1.0 / ((1u64 << 53) as f64);
        ((self.u64() >> 11) as f64) * SCALE
    }
}

fn fallback_random_seed() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let marker = 0u8;
    let addr = (&marker as *const u8 as usize) as u64;
    let seed = (now as u64)
        ^ ((now >> 64) as u64)
        ^ addr.rotate_left(17)
        ^ (std::process::id() as u64).rotate_left(31);
    if seed == 0 {
        0x9e37_79b9_7f4a_7c15
    } else {
        seed
    }
}

fn splitmix64_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn rand_int_range(rng: &mut RandomSource, min: i64, max: i64) -> Result<i64, String> {
    if min >= max {
        return Err(ion_str!("rand::int requires min < max"));
    }
    let span = (max as i128 - min as i128) as u128;
    let offset = rng.below(span as u64)? as i128;
    Ok((min as i128 + offset) as i64)
}

fn rand_int(args: &[Value]) -> Result<Value, String> {
    let mut rng = RandomSource::new();
    match args.len() {
        0 => Ok(Value::Int(rng.u64() as i64)),
        1 => {
            let max = required_int_arg(args, 0, "rand::int")?;
            if max <= 0 {
                return Err(ion_str!("rand::int max must be positive"));
            }
            rand_int_range(&mut rng, 0, max).map(Value::Int)
        }
        2 => {
            let min = required_int_arg(args, 0, "rand::int")?;
            let max = required_int_arg(args, 1, "rand::int")?;
            rand_int_range(&mut rng, min, max).map(Value::Int)
        }
        _ => Err(ion_str!("rand::int takes 0, 1, or 2 arguments")),
    }
}

fn rand_float(args: &[Value]) -> Result<Value, String> {
    let mut rng = RandomSource::new();
    let unit = rng.unit_float();
    match args.len() {
        0 => Ok(Value::Float(unit)),
        1 => {
            let max = args[0]
                .as_float()
                .ok_or_else(|| ion_str!("rand::float requires numeric arguments"))?;
            if !max.is_finite() || max <= 0.0 {
                return Err(ion_str!("rand::float max must be finite and positive"));
            }
            Ok(Value::Float(unit * max))
        }
        2 => {
            let min = args[0]
                .as_float()
                .ok_or_else(|| ion_str!("rand::float requires numeric arguments"))?;
            let max = args[1]
                .as_float()
                .ok_or_else(|| ion_str!("rand::float requires numeric arguments"))?;
            if !min.is_finite() || !max.is_finite() || min >= max {
                return Err(ion_str!("rand::float requires finite min < max"));
            }
            Ok(Value::Float(min + unit * (max - min)))
        }
        _ => Err(ion_str!("rand::float takes 0, 1, or 2 arguments")),
    }
}

fn rand_bool(args: &[Value]) -> Result<Value, String> {
    let mut rng = RandomSource::new();
    match args.len() {
        0 => Ok(Value::Bool((rng.u64() & 1) == 1)),
        1 => {
            let probability = args[0]
                .as_float()
                .ok_or_else(|| ion_str!("rand::bool requires numeric probability"))?;
            if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
                return Err(ion_str!("rand::bool probability must be in 0.0..=1.0"));
            }
            Ok(Value::Bool(rng.unit_float() < probability))
        }
        _ => Err(ion_str!("rand::bool takes 0 or 1 arguments")),
    }
}

fn rand_bytes(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(ion_str!("rand::bytes takes 1 argument"));
    }
    let len = required_int_arg(args, 0, "rand::bytes")?;
    if len < 0 {
        return Err(ion_str!("rand::bytes length must be non-negative"));
    }
    let len = len as usize;
    check_bytes_len(len, "rand::bytes")?;
    let mut bytes = vec![0u8; len];
    RandomSource::new().fill_bytes(&mut bytes);
    Ok(Value::Bytes(bytes))
}

fn rand_choice(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(ion_str!("rand::choice takes 1 argument"));
    }
    let mut rng = RandomSource::new();
    match &args[0] {
        Value::List(items) => {
            if items.is_empty() {
                Ok(Value::Option(None))
            } else {
                let index = rng.below(items.len() as u64)? as usize;
                Ok(Value::Option(Some(Box::new(items[index].clone()))))
            }
        }
        Value::Str(value) => {
            let chars: Vec<char> = value.chars().collect();
            if chars.is_empty() {
                Ok(Value::Option(None))
            } else {
                let index = rng.below(chars.len() as u64)? as usize;
                Ok(Value::Option(Some(Box::new(Value::Str(
                    chars[index].to_string(),
                )))))
            }
        }
        Value::Bytes(bytes) => {
            if bytes.is_empty() {
                Ok(Value::Option(None))
            } else {
                let index = rng.below(bytes.len() as u64)? as usize;
                Ok(Value::Option(Some(Box::new(Value::Int(
                    bytes[index] as i64,
                )))))
            }
        }
        other => Err(format!(
            "{}{}",
            ion_str!("rand::choice not supported for "),
            other.type_name()
        )),
    }
}

fn shuffle_slice<T>(rng: &mut RandomSource, items: &mut [T]) -> Result<(), String> {
    for index in (1..items.len()).rev() {
        let other = rng.below((index + 1) as u64)? as usize;
        items.swap(index, other);
    }
    Ok(())
}

fn rand_shuffle(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(ion_str!("rand::shuffle takes 1 argument"));
    }
    let mut rng = RandomSource::new();
    match &args[0] {
        Value::List(items) => {
            let mut shuffled = items.clone();
            shuffle_slice(&mut rng, &mut shuffled)?;
            Ok(Value::List(shuffled))
        }
        Value::Bytes(bytes) => {
            let mut shuffled = bytes.clone();
            shuffle_slice(&mut rng, &mut shuffled)?;
            Ok(Value::Bytes(shuffled))
        }
        other => Err(format!(
            "{}{}",
            ion_str!("rand::shuffle not supported for "),
            other.type_name()
        )),
    }
}

fn sample_len(args: &[Value], len: usize) -> Result<usize, String> {
    let n = required_int_arg(args, 1, "rand::sample")?;
    if n < 0 {
        return Err(ion_str!("rand::sample count must be non-negative"));
    }
    let n = n as usize;
    if n > len {
        return Err(ion_str!("rand::sample count exceeds population length"));
    }
    Ok(n)
}

fn sample_slice<T>(rng: &mut RandomSource, items: &mut [T], n: usize) -> Result<(), String> {
    for index in 0..n {
        let other = index + rng.below((items.len() - index) as u64)? as usize;
        items.swap(index, other);
    }
    Ok(())
}

fn rand_sample(args: &[Value]) -> Result<Value, String> {
    if args.len() != 2 {
        return Err(ion_str!("rand::sample takes 2 arguments"));
    }
    let mut rng = RandomSource::new();
    match &args[0] {
        Value::List(items) => {
            let n = sample_len(args, items.len())?;
            let mut sampled = items.clone();
            sample_slice(&mut rng, &mut sampled, n)?;
            sampled.truncate(n);
            Ok(Value::List(sampled))
        }
        Value::Bytes(bytes) => {
            let n = sample_len(args, bytes.len())?;
            let mut sampled = bytes.clone();
            sample_slice(&mut rng, &mut sampled, n)?;
            sampled.truncate(n);
            Ok(Value::Bytes(sampled))
        }
        other => Err(format!(
            "{}{}",
            ion_str!("rand::sample not supported for "),
            other.type_name()
        )),
    }
}

/// Build the `math` stdlib module.
///
/// Functions: abs, min, max, floor, ceil, round, sqrt, pow, clamp, log, log2, log10, sin, cos, tan, atan2
/// Constants: PI, E, INF, NAN, TAU
pub fn math_module() -> Module {
    let mut m = Module::new(crate::h!("math"));

    // Constants
    m.set(crate::h!("PI"), Value::Float(std::f64::consts::PI));
    m.set(crate::h!("E"), Value::Float(std::f64::consts::E));
    m.set(crate::h!("TAU"), Value::Float(std::f64::consts::TAU));
    m.set(crate::h!("INF"), Value::Float(f64::INFINITY));
    m.set(crate::h!("NAN"), Value::Float(f64::NAN));

    m.register_fn(crate::h!("abs"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        match &args[0] {
            Value::Int(n) => Ok(Value::Int(n.abs())),
            Value::Float(n) => Ok(Value::Float(n.abs())),
            _ => Err(format!(
                "{}{}",
                ion_str!("not supported for "),
                args[0].type_name()
            )),
        }
    });

    m.register_fn(crate::h!("min"), |args: &[Value]| {
        if args.len() < 2 {
            return Err(ion_str!("requires at least 2 arguments"));
        }
        let mut best = args[0].clone();
        for arg in &args[1..] {
            match (&best, arg) {
                (Value::Int(a), Value::Int(b)) if b < a => best = arg.clone(),
                (Value::Float(a), Value::Float(b)) if b < a => best = arg.clone(),
                (Value::Int(a), Value::Float(b)) if *b < (*a as f64) => best = arg.clone(),
                (Value::Float(a), Value::Int(b)) if (*b as f64) < *a => best = arg.clone(),
                (Value::Int(_), Value::Int(_))
                | (Value::Float(_), Value::Float(_))
                | (Value::Int(_), Value::Float(_))
                | (Value::Float(_), Value::Int(_)) => {}
                _ => return Err(ion_str!("requires numeric arguments")),
            }
        }
        Ok(best)
    });

    m.register_fn(crate::h!("max"), |args: &[Value]| {
        if args.len() < 2 {
            return Err(ion_str!("requires at least 2 arguments"));
        }
        let mut best = args[0].clone();
        for arg in &args[1..] {
            match (&best, arg) {
                (Value::Int(a), Value::Int(b)) if b > a => best = arg.clone(),
                (Value::Float(a), Value::Float(b)) if b > a => best = arg.clone(),
                (Value::Int(a), Value::Float(b)) if *b > (*a as f64) => best = arg.clone(),
                (Value::Float(a), Value::Int(b)) if (*b as f64) > *a => best = arg.clone(),
                (Value::Int(_), Value::Int(_))
                | (Value::Float(_), Value::Float(_))
                | (Value::Int(_), Value::Float(_))
                | (Value::Float(_), Value::Int(_)) => {}
                _ => return Err(ion_str!("requires numeric arguments")),
            }
        }
        Ok(best)
    });

    m.register_fn(crate::h!("floor"), |args: &[Value]| match &args[0] {
        Value::Float(n) => Ok(Value::Float(n.floor())),
        Value::Int(n) => Ok(Value::Int(*n)),
        _ => Err(format!(
            "{}{}",
            ion_str!("not supported for "),
            args[0].type_name()
        )),
    });

    m.register_fn(crate::h!("ceil"), |args: &[Value]| match &args[0] {
        Value::Float(n) => Ok(Value::Float(n.ceil())),
        Value::Int(n) => Ok(Value::Int(*n)),
        _ => Err(format!(
            "{}{}",
            ion_str!("not supported for "),
            args[0].type_name()
        )),
    });

    m.register_fn(crate::h!("round"), |args: &[Value]| match &args[0] {
        Value::Float(n) => Ok(Value::Float(n.round())),
        Value::Int(n) => Ok(Value::Int(*n)),
        _ => Err(format!(
            "{}{}",
            ion_str!("not supported for "),
            args[0].type_name()
        )),
    });

    m.register_fn(crate::h!("sqrt"), |args: &[Value]| {
        let n = args[0].as_float().ok_or(ion_str!("requires a number"))?;
        Ok(Value::Float(n.sqrt()))
    });

    m.register_fn(crate::h!("pow"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments"));
        }
        match (&args[0], &args[1]) {
            (Value::Int(base), Value::Int(exp)) => {
                if *exp >= 0 {
                    Ok(Value::Int(base.pow(*exp as u32)))
                } else {
                    Ok(Value::Float((*base as f64).powi(*exp as i32)))
                }
            }
            _ => {
                let b = args[0]
                    .as_float()
                    .ok_or(ion_str!("requires numeric arguments"))?;
                let e = args[1]
                    .as_float()
                    .ok_or(ion_str!("requires numeric arguments"))?;
                Ok(Value::Float(b.powf(e)))
            }
        }
    });

    m.register_fn(crate::h!("clamp"), |args: &[Value]| {
        if args.len() != 3 {
            return Err(ion_str!("requires 3 arguments: value, min, max"));
        }
        match (&args[0], &args[1], &args[2]) {
            (Value::Int(v), Value::Int(lo), Value::Int(hi)) => Ok(Value::Int(*v.max(lo).min(hi))),
            (Value::Float(v), Value::Float(lo), Value::Float(hi)) => {
                Ok(Value::Float(v.max(*lo).min(*hi)))
            }
            _ => {
                let v = args[0]
                    .as_float()
                    .ok_or(ion_str!("requires numeric arguments"))?;
                let lo = args[1]
                    .as_float()
                    .ok_or(ion_str!("requires numeric arguments"))?;
                let hi = args[2]
                    .as_float()
                    .ok_or(ion_str!("requires numeric arguments"))?;
                Ok(Value::Float(v.max(lo).min(hi)))
            }
        }
    });

    // Trigonometry
    m.register_fn(crate::h!("sin"), |args: &[Value]| {
        let n = args[0].as_float().ok_or(ion_str!("requires a number"))?;
        Ok(Value::Float(n.sin()))
    });

    m.register_fn(crate::h!("cos"), |args: &[Value]| {
        let n = args[0].as_float().ok_or(ion_str!("requires a number"))?;
        Ok(Value::Float(n.cos()))
    });

    m.register_fn(crate::h!("tan"), |args: &[Value]| {
        let n = args[0].as_float().ok_or(ion_str!("requires a number"))?;
        Ok(Value::Float(n.tan()))
    });

    m.register_fn(crate::h!("atan2"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments"));
        }
        let y = args[0]
            .as_float()
            .ok_or(ion_str!("requires numeric arguments"))?;
        let x = args[1]
            .as_float()
            .ok_or(ion_str!("requires numeric arguments"))?;
        Ok(Value::Float(y.atan2(x)))
    });

    // Logarithms
    m.register_fn(crate::h!("log"), |args: &[Value]| {
        let n = args[0].as_float().ok_or(ion_str!("requires a number"))?;
        Ok(Value::Float(n.ln()))
    });

    m.register_fn(crate::h!("log2"), |args: &[Value]| {
        let n = args[0].as_float().ok_or(ion_str!("requires a number"))?;
        Ok(Value::Float(n.log2()))
    });

    m.register_fn(crate::h!("log10"), |args: &[Value]| {
        let n = args[0].as_float().ok_or(ion_str!("requires a number"))?;
        Ok(Value::Float(n.log10()))
    });

    // Rounding/check
    m.register_fn(crate::h!("is_nan"), |args: &[Value]| match &args[0] {
        Value::Float(n) => Ok(Value::Bool(n.is_nan())),
        Value::Int(_) => Ok(Value::Bool(false)),
        _ => Err(format!(
            "{}{}",
            ion_str!("not supported for "),
            args[0].type_name()
        )),
    });

    m.register_fn(crate::h!("is_inf"), |args: &[Value]| match &args[0] {
        Value::Float(n) => Ok(Value::Bool(n.is_infinite())),
        Value::Int(_) => Ok(Value::Bool(false)),
        _ => Err(format!(
            "{}{}",
            ion_str!("not supported for "),
            args[0].type_name()
        )),
    });

    m
}

/// Build the `json` stdlib module.
///
/// Functions: encode, decode, pretty
pub fn json_module() -> Module {
    let mut m = Module::new(crate::h!("json"));

    m.register_fn(crate::h!("encode"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let json = args[0].to_json();
        Ok(Value::Str(json.to_string()))
    });

    m.register_fn(crate::h!("decode"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        let json: serde_json::Value =
            serde_json::from_str(s).map_err(|e| format!("{}{}", ion_str!("error: "), e))?;
        Ok(Value::from_json(json))
    });

    m.register_fn(crate::h!("pretty"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let json = args[0].to_json();
        serde_json::to_string_pretty(&json)
            .map(Value::Str)
            .map_err(|e| format!("{}{}", ion_str!("error: "), e))
    });

    #[cfg(feature = "msgpack")]
    m.register_fn(crate::h!("msgpack_encode"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        args[0].to_msgpack().map(Value::Bytes)
    });

    #[cfg(feature = "msgpack")]
    m.register_fn(crate::h!("msgpack_decode"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let data = match &args[0] {
            Value::Bytes(b) => b,
            _ => return Err(ion_str!("requires bytes")),
        };
        Value::from_msgpack(data)
    });

    m
}

/// Build the `bytes` stdlib module.
///
/// The module is callable by the runtimes, so legacy `bytes(...)` constructor
/// calls keep working after this module is registered as `bytes`.
pub fn bytes_module() -> Module {
    let mut m = Module::new(crate::h!("bytes"));

    m.register_fn(crate::h!("new"), |args: &[Value]| {
        if !args.is_empty() {
            return Err(ion_str!("takes no arguments"));
        }
        Ok(Value::Bytes(Vec::new()))
    });

    m.register_fn(crate::h!("zeroed"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let len = required_int_arg(args, 0, "bytes::zeroed")?;
        if len < 0 {
            return Err(ion_str!("byte count must be non-negative"));
        }
        let len = len as usize;
        check_bytes_len(len, "bytes::zeroed")?;
        Ok(Value::Bytes(vec![0u8; len]))
    });

    m.register_fn(crate::h!("repeat"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments: byte, count"));
        }
        let byte = byte_from_value(&args[0], "bytes::repeat")?;
        let count = required_int_arg(args, 1, "bytes::repeat")?;
        if count < 0 {
            return Err(ion_str!("repeat count must be non-negative"));
        }
        let len = count as usize;
        check_bytes_len(len, "bytes::repeat")?;
        Ok(Value::Bytes(vec![byte; len]))
    });

    m.register_fn(crate::h!("from_list"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        match &args[0] {
            Value::List(items) => bytes_from_list(items, "bytes::from_list").map(Value::Bytes),
            other => Err(format!(
                "{}{}",
                ion_str!("requires list, got "),
                other.type_name()
            )),
        }
    });

    m.register_fn(crate::h!("from_str"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let value = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        Ok(Value::Bytes(value.as_bytes().to_vec()))
    });

    m.register_fn(crate::h!("from_hex"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let value = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        Ok(result_bytes(bytes_from_hex_string(value)))
    });

    m.register_fn(crate::h!("from_base64"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let value = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        Ok(result_bytes(bytes_from_base64_string(value)))
    });

    m.register_fn(crate::h!("concat"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let parts = match &args[0] {
            Value::List(parts) => parts,
            other => {
                return Err(format!(
                    "{}{}",
                    ion_str!("requires list, got "),
                    other.type_name()
                ));
            }
        };
        let len = parts.iter().try_fold(0usize, |len, part| match part {
            Value::Bytes(bytes) => len
                .checked_add(bytes.len())
                .ok_or_else(|| ion_str!("bytes::concat length overflow")),
            other => Err(format!(
                "{}{}",
                ion_str!("requires list of bytes, got "),
                other.type_name()
            )),
        })?;
        check_bytes_len(len, "bytes::concat")?;
        let mut out = Vec::with_capacity(len);
        for part in parts {
            if let Value::Bytes(bytes) = part {
                out.extend_from_slice(bytes);
            }
        }
        Ok(Value::Bytes(out))
    });

    m.register_fn(crate::h!("join"), |args: &[Value]| {
        if args.is_empty() || args.len() > 2 {
            return Err(ion_str!("takes 1 or 2 arguments: parts, separator"));
        }
        let parts = match &args[0] {
            Value::List(parts) => parts,
            other => {
                return Err(format!(
                    "{}{}",
                    ion_str!("requires list, got "),
                    other.type_name()
                ));
            }
        };
        let sep = if args.len() == 2 {
            required_bytes_arg(args, 1, "bytes::join")?
        } else {
            &[]
        };
        let parts_len = parts.iter().try_fold(0usize, |len, part| match part {
            Value::Bytes(bytes) => len
                .checked_add(bytes.len())
                .ok_or_else(|| ion_str!("bytes::join length overflow")),
            other => Err(format!(
                "{}{}",
                ion_str!("requires list of bytes, got "),
                other.type_name()
            )),
        })?;
        let seps_len = sep
            .len()
            .checked_mul(parts.len().saturating_sub(1))
            .ok_or_else(|| ion_str!("bytes::join length overflow"))?;
        let len = parts_len
            .checked_add(seps_len)
            .ok_or_else(|| ion_str!("bytes::join length overflow"))?;
        check_bytes_len(len, "bytes::join")?;

        let mut out = Vec::with_capacity(len);
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                out.extend_from_slice(sep);
            }
            if let Value::Bytes(bytes) = part {
                out.extend_from_slice(bytes);
            }
        }
        Ok(Value::Bytes(out))
    });

    m.register_fn(crate::h!("u16_le"), |args| {
        pack_unsigned(args, 2, ByteOrder::Little, "bytes::u16_le")
    });
    m.register_fn(crate::h!("u16_be"), |args| {
        pack_unsigned(args, 2, ByteOrder::Big, "bytes::u16_be")
    });
    m.register_fn(crate::h!("u32_le"), |args| {
        pack_unsigned(args, 4, ByteOrder::Little, "bytes::u32_le")
    });
    m.register_fn(crate::h!("u32_be"), |args| {
        pack_unsigned(args, 4, ByteOrder::Big, "bytes::u32_be")
    });
    m.register_fn(crate::h!("u64_le"), |args| {
        pack_unsigned(args, 8, ByteOrder::Little, "bytes::u64_le")
    });
    m.register_fn(crate::h!("u64_be"), |args| {
        pack_unsigned(args, 8, ByteOrder::Big, "bytes::u64_be")
    });
    m.register_fn(crate::h!("i16_le"), |args| {
        pack_signed(args, 2, ByteOrder::Little, "bytes::i16_le")
    });
    m.register_fn(crate::h!("i16_be"), |args| {
        pack_signed(args, 2, ByteOrder::Big, "bytes::i16_be")
    });
    m.register_fn(crate::h!("i32_le"), |args| {
        pack_signed(args, 4, ByteOrder::Little, "bytes::i32_le")
    });
    m.register_fn(crate::h!("i32_be"), |args| {
        pack_signed(args, 4, ByteOrder::Big, "bytes::i32_be")
    });
    m.register_fn(crate::h!("i64_le"), |args| {
        pack_signed(args, 8, ByteOrder::Little, "bytes::i64_le")
    });
    m.register_fn(crate::h!("i64_be"), |args| {
        pack_signed(args, 8, ByteOrder::Big, "bytes::i64_be")
    });

    m
}

/// Build the `rand` stdlib module.
///
/// Functions: int, float, bool, bytes, choice, shuffle, sample
pub fn rand_module() -> Module {
    let mut m = Module::new(crate::h!("rand"));

    m.register_fn(crate::h!("int"), rand_int);
    m.register_fn(crate::h!("float"), rand_float);
    m.register_fn(crate::h!("bool"), rand_bool);
    m.register_fn(crate::h!("bytes"), rand_bytes);
    m.register_fn(crate::h!("choice"), rand_choice);
    m.register_fn(crate::h!("shuffle"), rand_shuffle);
    m.register_fn(crate::h!("sample"), rand_sample);

    m
}

fn format_output_args(args: &[Value]) -> String {
    args.iter()
        .map(|arg| arg.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the `log` stdlib module with a handler whose threshold is the
/// shared atomic level (used by `log::set_level`/`log::level`).
///
/// Functions: trace, debug, info, warn, error, set_level, level, enabled.
pub fn log_module_with_handler(
    handler: Arc<dyn crate::log::LogHandler>,
    level: Arc<crate::log::AtomicLogLevel>,
) -> Module {
    use crate::log::{AtomicLogLevel, LogLevel};

    let mut m = Module::new(crate::h!("log"));

    fn register_level(
        m: &mut Module,
        name_hash: u64,
        level: LogLevel,
        handler: Arc<dyn crate::log::LogHandler>,
    ) {
        m.register_closure(name_hash, move |args: &[Value]| {
            // Pre-flight check — handlers can short-circuit before any
            // formatting work happens for callsites that survived
            // compile-time stripping but fail the runtime threshold.
            if !handler.enabled(level) {
                return Ok(Value::Unit);
            }
            let (message, fields) = match args.len() {
                1 => (extract_message(&args[0])?, Vec::new()),
                2 => (extract_message(&args[0])?, extract_fields(&args[1])?),
                _ => {
                    return Err(ion_str!("requires 1 or 2 arguments: message, [fields]"));
                }
            };
            handler.log(level, &message, &fields);
            Ok(Value::Unit)
        });
    }

    register_level(
        &mut m,
        crate::h!("trace"),
        LogLevel::Trace,
        Arc::clone(&handler),
    );
    register_level(
        &mut m,
        crate::h!("debug"),
        LogLevel::Debug,
        Arc::clone(&handler),
    );
    register_level(
        &mut m,
        crate::h!("info"),
        LogLevel::Info,
        Arc::clone(&handler),
    );
    register_level(
        &mut m,
        crate::h!("warn"),
        LogLevel::Warn,
        Arc::clone(&handler),
    );
    register_level(
        &mut m,
        crate::h!("error"),
        LogLevel::Error,
        Arc::clone(&handler),
    );

    let level_for_set = Arc::clone(&level);
    m.register_closure(crate::h!("set_level"), move |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument: name"));
        }
        let name = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        let parsed = LogLevel::from_str_ci(name)
            .ok_or_else(|| format!("{}{}", ion_str!("unknown level: "), name))?;
        level_for_set.set(parsed);
        Ok(Value::Unit)
    });

    let level_for_get = Arc::clone(&level);
    m.register_closure(crate::h!("level"), move |args: &[Value]| {
        if !args.is_empty() {
            return Err(ion_str!("takes no arguments"));
        }
        Ok(Value::Str(level_for_get.get().as_str().to_string()))
    });

    let handler_for_enabled = Arc::clone(&handler);
    m.register_closure(crate::h!("enabled"), move |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument: name"));
        }
        let name = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        let parsed = LogLevel::from_str_ci(name)
            .ok_or_else(|| format!("{}{}", ion_str!("unknown level: "), name))?;
        Ok(Value::Bool(handler_for_enabled.enabled(parsed)))
    });

    // Avoid `unused` warning when no extra references are made later.
    let _ = AtomicLogLevel::default_runtime;
    let _ = level;

    m
}

/// Build the default `log` stdlib module — uses [`StdLogHandler`] writing to
/// stderr, with a fresh runtime threshold (honouring `ION_LOG`).
pub fn log_module() -> Module {
    let level = crate::log::AtomicLogLevel::default_runtime();
    let handler: Arc<dyn crate::log::LogHandler> = Arc::new(
        crate::log::StdLogHandler::with_threshold(Arc::clone(&level)),
    );
    log_module_with_handler(handler, level)
}

fn extract_message(v: &Value) -> Result<String, String> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        other => Ok(format!("{}", other)),
    }
}

fn extract_fields(v: &Value) -> Result<Vec<(String, Value)>, String> {
    match v {
        Value::Dict(map) => Ok(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        other => Err(format!(
            "{}{}",
            ion_str!("invalid fields: "),
            other.type_name()
        )),
    }
}

/// Build the `io` stdlib module.
///
/// Functions: print, println, eprintln
pub fn io_module() -> Module {
    io_module_with_output(missing_output_handler())
}

/// Build the `io` stdlib module with a host-provided output handler — sync
/// build. Calls into [`OutputHandler::write`] directly.
#[cfg(not(feature = "async-runtime"))]
pub fn io_module_with_output(output: Arc<dyn OutputHandler>) -> Module {
    let mut m = Module::new(crate::h!("io"));

    let stdout = Arc::clone(&output);
    m.register_closure(crate::h!("print"), move |args: &[Value]| {
        stdout.write(OutputStream::Stdout, &format_output_args(args))?;
        Ok(Value::Unit)
    });

    let stdout = Arc::clone(&output);
    m.register_closure(crate::h!("println"), move |args: &[Value]| {
        let mut text = format_output_args(args);
        text.push('\n');
        stdout.write(OutputStream::Stdout, &text)?;
        Ok(Value::Unit)
    });

    m.register_closure(crate::h!("eprintln"), move |args: &[Value]| {
        let mut text = format_output_args(args);
        text.push('\n');
        output.write(OutputStream::Stderr, &text)?;
        Ok(Value::Unit)
    });

    m
}

/// Build the `io` stdlib module with a host-provided output handler — async
/// build. Each `io::print*` call dispatches the (still-sync) handler write
/// onto Tokio's blocking thread pool via `spawn_blocking`, so a slow stdout
/// can't stall the executor running other Ion tasks.
#[cfg(feature = "async-runtime")]
pub fn io_module_with_output(output: Arc<dyn OutputHandler>) -> Module {
    use crate::error::IonError;
    let mut m = Module::new(crate::h!("io"));

    let stdout = Arc::clone(&output);
    m.register_async_fn(crate::h!("print"), move |args| {
        let stdout = Arc::clone(&stdout);
        async move {
            let text = format_output_args(&args);
            tokio::task::spawn_blocking(move || stdout.write(OutputStream::Stdout, &text))
                .await
                .map_err(|e| IonError::runtime(ion_format!("join error: {}", e), 0, 0))?
                .map_err(|e| IonError::runtime(ion_format!("{}", e), 0, 0))?;
            Ok(Value::Unit)
        }
    });

    let stdout = Arc::clone(&output);
    m.register_async_fn(crate::h!("println"), move |args| {
        let stdout = Arc::clone(&stdout);
        async move {
            let mut text = format_output_args(&args);
            text.push('\n');
            tokio::task::spawn_blocking(move || stdout.write(OutputStream::Stdout, &text))
                .await
                .map_err(|e| IonError::runtime(ion_format!("join error: {}", e), 0, 0))?
                .map_err(|e| IonError::runtime(ion_format!("{}", e), 0, 0))?;
            Ok(Value::Unit)
        }
    });

    let stderr = Arc::clone(&output);
    m.register_async_fn(crate::h!("eprintln"), move |args| {
        let stderr = Arc::clone(&stderr);
        async move {
            let mut text = format_output_args(&args);
            text.push('\n');
            tokio::task::spawn_blocking(move || stderr.write(OutputStream::Stderr, &text))
                .await
                .map_err(|e| IonError::runtime(ion_format!("join error: {}", e), 0, 0))?
                .map_err(|e| IonError::runtime(ion_format!("{}", e), 0, 0))?;
            Ok(Value::Unit)
        }
    });

    m
}

/// Build the `str` stdlib module.
///
/// Functions: join
pub fn string_module() -> Module {
    let mut m = Module::new(crate::h!("string"));

    m.register_fn(crate::h!("join"), |args: &[Value]| {
        if args.is_empty() || args.len() > 2 {
            return Err(ion_str!("requires 1-2 arguments: list, [separator]"));
        }
        let items = match &args[0] {
            Value::List(items) => items,
            _ => return Err(ion_str!("requires a list as first argument")),
        };
        let sep = if args.len() > 1 {
            args[1].as_str().unwrap_or("").to_string()
        } else {
            String::new()
        };
        let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
        Ok(Value::Str(parts.join(&sep)))
    });

    m
}

// ── semver ───────────────────────────────────────────────────────────────
//
// The `semver::` module is feature-gated. Versions cross the language
// boundary as dicts shaped `#{major, minor, patch, pre, build}` so scripts
// can inspect fields directly and round-trip through `json::encode`.

#[cfg(feature = "semver")]
fn semver_version_to_dict(v: &Version) -> Value {
    let mut d = indexmap::IndexMap::new();
    d.insert("major".to_string(), Value::Int(v.major as i64));
    d.insert("minor".to_string(), Value::Int(v.minor as i64));
    d.insert("patch".to_string(), Value::Int(v.patch as i64));
    d.insert("pre".to_string(), Value::Str(v.pre.as_str().to_string()));
    d.insert(
        "build".to_string(),
        Value::Str(v.build.as_str().to_string()),
    );
    Value::Dict(d)
}

/// Coerce a `Value` (string or dict) into a parsed `Version`. Used by every
/// semver function that compares or rewrites a version. Errors are
/// generic — the call site (interpreter / VM) prepends the resolved
/// function name from `names::lookup(qualified_hash)` for readable
/// diagnostics in debug builds and sidecar-loaded release builds.
#[cfg(feature = "semver")]
fn semver_parse_arg(v: &Value) -> Result<Version, String> {
    match v {
        Value::Str(s) => Version::parse(s).map_err(|e| e.to_string()),
        Value::Dict(map) => {
            let major = map
                .get("major")
                .and_then(Value::as_int)
                .ok_or_else(|| ion_str!("dict missing integer 'major'"))?;
            let minor = map
                .get("minor")
                .and_then(Value::as_int)
                .ok_or_else(|| ion_str!("dict missing integer 'minor'"))?;
            let patch = map
                .get("patch")
                .and_then(Value::as_int)
                .ok_or_else(|| ion_str!("dict missing integer 'patch'"))?;
            if major < 0 || minor < 0 || patch < 0 {
                return Err(ion_str!("version components must be non-negative"));
            }
            let pre_str = map.get("pre").and_then(Value::as_str).unwrap_or("");
            let build_str = map.get("build").and_then(Value::as_str).unwrap_or("");
            let pre = if pre_str.is_empty() {
                Prerelease::EMPTY
            } else {
                Prerelease::new(pre_str).map_err(|e| {
                    format!("{}'{}': {}", ion_str!("invalid pre-release "), pre_str, e)
                })?
            };
            let build = if build_str.is_empty() {
                BuildMetadata::EMPTY
            } else {
                BuildMetadata::new(build_str).map_err(|e| {
                    format!(
                        "{}'{}': {}",
                        ion_str!("invalid build metadata "),
                        build_str,
                        e
                    )
                })?
            };
            Ok(Version {
                major: major as u64,
                minor: minor as u64,
                patch: patch as u64,
                pre,
                build,
            })
        }
        _ => Err(format!(
            "{}{}",
            ion_str!("expected string or dict, got "),
            v.type_name()
        )),
    }
}

/// Build the `semver` stdlib module.
///
/// Functions: parse, is_valid, format, compare, eq, gt, gte, lt, lte,
///            satisfies, bump_major, bump_minor, bump_patch
#[cfg(feature = "semver")]
pub fn semver_module() -> Module {
    let mut m = Module::new(crate::h!("semver"));

    m.register_fn(crate::h!("parse"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        let v = Version::parse(s).map_err(|e| format!("{}", e))?;
        Ok(semver_version_to_dict(&v))
    });

    m.register_fn(crate::h!("is_valid"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        Ok(Value::Bool(Version::parse(s).is_ok()))
    });

    m.register_fn(crate::h!("format"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let v = semver_parse_arg(&args[0])?;
        Ok(Value::Str(v.to_string()))
    });

    m.register_fn(crate::h!("compare"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments"));
        }
        let a = semver_parse_arg(&args[0])?;
        let b = semver_parse_arg(&args[1])?;
        Ok(Value::Int(match a.cmp(&b) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }))
    });

    m.register_fn(crate::h!("eq"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments"));
        }
        let a = semver_parse_arg(&args[0])?;
        let b = semver_parse_arg(&args[1])?;
        Ok(Value::Bool(a == b))
    });

    m.register_fn(crate::h!("gt"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments"));
        }
        let a = semver_parse_arg(&args[0])?;
        let b = semver_parse_arg(&args[1])?;
        Ok(Value::Bool(a > b))
    });

    m.register_fn(crate::h!("gte"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments"));
        }
        let a = semver_parse_arg(&args[0])?;
        let b = semver_parse_arg(&args[1])?;
        Ok(Value::Bool(a >= b))
    });

    m.register_fn(crate::h!("lt"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments"));
        }
        let a = semver_parse_arg(&args[0])?;
        let b = semver_parse_arg(&args[1])?;
        Ok(Value::Bool(a < b))
    });

    m.register_fn(crate::h!("lte"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments"));
        }
        let a = semver_parse_arg(&args[0])?;
        let b = semver_parse_arg(&args[1])?;
        Ok(Value::Bool(a <= b))
    });

    m.register_fn(crate::h!("satisfies"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments: version, requirement"));
        }
        let v = semver_parse_arg(&args[0])?;
        let req_str = args[1]
            .as_str()
            .ok_or_else(|| ion_str!("requirement must be a string"))?;
        let req = VersionReq::parse(req_str).map_err(|e| format!("invalid requirement: {}", e))?;
        Ok(Value::Bool(req.matches(&v)))
    });

    m.register_fn(crate::h!("bump_major"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let v = semver_parse_arg(&args[0])?;
        let bumped = Version::new(v.major + 1, 0, 0);
        Ok(Value::Str(bumped.to_string()))
    });

    m.register_fn(crate::h!("bump_minor"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let v = semver_parse_arg(&args[0])?;
        let bumped = Version::new(v.major, v.minor + 1, 0);
        Ok(Value::Str(bumped.to_string()))
    });

    m.register_fn(crate::h!("bump_patch"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let v = semver_parse_arg(&args[0])?;
        // If the input has a pre-release, the bumped value is the same
        // numeric triple with the pre-release stripped (1.2.3-alpha → 1.2.3).
        // Otherwise increment patch.
        let bumped = if v.pre.is_empty() {
            Version::new(v.major, v.minor, v.patch + 1)
        } else {
            Version::new(v.major, v.minor, v.patch)
        };
        Ok(Value::Str(bumped.to_string()))
    });

    m
}

// ── os ───────────────────────────────────────────────────────────────────
//
// The `os::` module is feature-gated. OS / arch detection values are baked
// in at build time (from `std::env::consts`); env-var and process-info
// helpers call `std` directly. Script args are host-injected via
// `Engine::set_args` and reach scripts as `os::args()`.

/// Build the `os` stdlib module with the given script args (used by
/// `os::args()`). Pass `Arc::new(Vec::new())` for the default empty list.
#[cfg(feature = "os")]
pub fn os_module_with_args(args: Arc<Vec<String>>) -> Module {
    let mut m = Module::new(crate::h!("os"));

    // Detection constants
    m.set(crate::h!("name"), Value::Str(os_name()));
    m.set(crate::h!("arch"), Value::Str(os_arch()));
    m.set(crate::h!("family"), Value::Str(os_family()));
    m.set(crate::h!("dll_extension"), Value::Str(os_dll_extension()));
    m.set(crate::h!("exe_extension"), Value::Str(os_exe_extension()));
    m.set(
        crate::h!("pointer_width"),
        Value::Int((std::mem::size_of::<usize>() * 8) as i64),
    );

    m.register_fn(crate::h!("env_var"), |args: &[Value]| {
        if args.is_empty() || args.len() > 2 {
            return Err(ion_str!("takes 1 or 2 arguments: name, [default]"));
        }
        let name = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("name must be a string"))?;
        match std::env::var(name) {
            Ok(v) => Ok(Value::Str(v)),
            Err(_) if args.len() == 2 => Ok(args[1].clone()),
            Err(e) => Err(format!("'{}': {}", name, e)),
        }
    });

    m.register_fn(crate::h!("has_env_var"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let name = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("name must be a string"))?;
        Ok(Value::Bool(std::env::var_os(name).is_some()))
    });

    m.register_fn(crate::h!("env_vars"), |args: &[Value]| {
        if !args.is_empty() {
            return Err(ion_str!("takes no arguments"));
        }
        let mut d = indexmap::IndexMap::new();
        for (k, v) in std::env::vars() {
            d.insert(k, Value::Str(v));
        }
        Ok(Value::Dict(d))
    });

    m.register_fn(crate::h!("cwd"), |args: &[Value]| {
        if !args.is_empty() {
            return Err(ion_str!("takes no arguments"));
        }
        std::env::current_dir()
            .map(|p| Value::Str(p.to_string_lossy().into_owned()))
            .map_err(|e| format!("{}", e))
    });

    m.register_fn(crate::h!("pid"), |args: &[Value]| {
        if !args.is_empty() {
            return Err(ion_str!("takes no arguments"));
        }
        Ok(Value::Int(std::process::id() as i64))
    });

    m.register_fn(crate::h!("temp_dir"), |args: &[Value]| {
        if !args.is_empty() {
            return Err(ion_str!("takes no arguments"));
        }
        Ok(Value::Str(
            std::env::temp_dir().to_string_lossy().into_owned(),
        ))
    });

    let args_arc = Arc::clone(&args);
    m.register_closure(crate::h!("args"), move |call_args: &[Value]| {
        if !call_args.is_empty() {
            return Err(ion_str!("takes no arguments"));
        }
        Ok(Value::List(
            args_arc.iter().map(|s| Value::Str(s.clone())).collect(),
        ))
    });

    m
}

#[cfg(all(feature = "os", target_os = "windows"))]
fn os_name() -> String {
    ion_obf_string!("windows")
}

#[cfg(all(feature = "os", not(target_os = "windows")))]
fn os_name() -> String {
    std::env::consts::OS.to_string()
}

#[cfg(all(feature = "os", target_arch = "x86_64"))]
fn os_arch() -> String {
    ion_obf_string!("x86_64")
}

#[cfg(all(feature = "os", not(target_arch = "x86_64")))]
fn os_arch() -> String {
    std::env::consts::ARCH.to_string()
}

#[cfg(all(feature = "os", target_family = "windows"))]
fn os_family() -> String {
    ion_obf_string!("windows")
}

#[cfg(all(feature = "os", not(target_family = "windows")))]
fn os_family() -> String {
    std::env::consts::FAMILY.to_string()
}

#[cfg(all(feature = "os", target_os = "windows"))]
fn os_dll_extension() -> String {
    ion_obf_string!("dll")
}

#[cfg(all(feature = "os", not(target_os = "windows")))]
fn os_dll_extension() -> String {
    std::env::consts::DLL_EXTENSION.to_string()
}

#[cfg(all(feature = "os", target_os = "windows"))]
fn os_exe_extension() -> String {
    ion_obf_string!("exe")
}

#[cfg(all(feature = "os", not(target_os = "windows")))]
fn os_exe_extension() -> String {
    std::env::consts::EXE_EXTENSION.to_string()
}

/// Build the `os::` stdlib module with no script args. Use
/// `Engine::set_args` afterwards to inject argv reachable as `os::args()`.
#[cfg(feature = "os")]
pub fn os_module() -> Module {
    os_module_with_args(Arc::new(Vec::new()))
}

// ── path ─────────────────────────────────────────────────────────────────
//
// Pure string-level path manipulation. No I/O, no async, no feature gate —
// this is composition glue that should always be available. Operates on
// strings and returns strings; `Value::Bytes` paths are not supported.

/// Build the `path::` stdlib module.
pub fn path_module() -> Module {
    use std::path::{Path, PathBuf, MAIN_SEPARATOR_STR};

    let mut m = Module::new(crate::h!("path"));

    m.set(crate::h!("sep"), Value::Str(MAIN_SEPARATOR_STR.to_string()));

    m.register_fn(crate::h!("join"), |args: &[Value]| {
        if args.is_empty() {
            return Err(ion_str!("takes at least 1 argument"));
        }
        let mut buf = PathBuf::new();
        for (i, arg) in args.iter().enumerate() {
            let s = arg
                .as_str()
                .ok_or_else(|| ion_format!("argument {} must be a string", i + 1))?;
            buf.push(s);
        }
        Ok(Value::Str(buf.to_string_lossy().into_owned()))
    });

    m.register_fn(crate::h!("parent"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        Ok(Value::Str(
            Path::new(s)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    });

    m.register_fn(crate::h!("basename"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        Ok(Value::Str(
            Path::new(s)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    });

    m.register_fn(crate::h!("stem"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        Ok(Value::Str(
            Path::new(s)
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    });

    m.register_fn(crate::h!("extension"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        Ok(Value::Str(
            Path::new(s)
                .extension()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ))
    });

    m.register_fn(crate::h!("with_extension"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments: path, ext"));
        }
        let p = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("path must be a string"))?;
        let ext = args[1]
            .as_str()
            .ok_or_else(|| ion_str!("ext must be a string"))?;
        Ok(Value::Str(
            Path::new(p)
                .with_extension(ext)
                .to_string_lossy()
                .into_owned(),
        ))
    });

    m.register_fn(crate::h!("is_absolute"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        Ok(Value::Bool(Path::new(s).is_absolute()))
    });

    m.register_fn(crate::h!("is_relative"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        Ok(Value::Bool(Path::new(s).is_relative()))
    });

    m.register_fn(crate::h!("components"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        let comps: Vec<Value> = Path::new(s)
            .components()
            .map(|c| Value::Str(c.as_os_str().to_string_lossy().into_owned()))
            .collect();
        Ok(Value::List(comps))
    });

    m.register_fn(crate::h!("normalize"), |args: &[Value]| {
        // Lexical normalisation: collapse `.` and `..` without consulting the
        // filesystem. Mirrors `path.Clean` from Go / Node's `path.normalize`.
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let s = args[0]
            .as_str()
            .ok_or_else(|| ion_str!("requires a string"))?;
        use std::path::Component;
        let mut out = PathBuf::new();
        for c in Path::new(s).components() {
            match c {
                Component::Prefix(p) => {
                    out.push(p.as_os_str());
                }
                Component::RootDir => {
                    out.push(std::path::Component::RootDir);
                }
                Component::CurDir => { /* skip */ }
                Component::ParentDir => {
                    // Pop the last normal component if any, otherwise keep the `..`
                    let popped = match out.components().next_back() {
                        Some(Component::Normal(_)) => out.pop(),
                        _ => false,
                    };
                    if !popped {
                        out.push("..");
                    }
                }
                Component::Normal(n) => out.push(n),
            }
        }
        let result = if out.as_os_str().is_empty() {
            ".".to_string()
        } else {
            out.to_string_lossy().into_owned()
        };
        Ok(Value::Str(result))
    });

    m
}

// ── fs ───────────────────────────────────────────────────────────────────
//
// Filesystem I/O. The script-level surface (`fs::read`, `fs::write`, …) is
// identical regardless of build mode; only the underlying impl changes. In
// a sync build (`async-runtime` off) operations call `std::fs` directly. In
// an async build they're registered via `register_async_fn` and call
// `tokio::fs`, so they cooperate with the executor instead of blocking it.
//
// `read_bytes` returns `bytes`; `copy` and random helpers return byte counts;
// everything else returns strings or unit.

#[cfg(feature = "fs")]
fn fs_metadata_to_dict(md: &std::fs::Metadata) -> Value {
    let mut d = indexmap::IndexMap::new();
    d.insert(ion_obf_string!("size"), Value::Int(md.len() as i64));
    d.insert(ion_obf_string!("is_file"), Value::Bool(md.is_file()));
    d.insert(ion_obf_string!("is_dir"), Value::Bool(md.is_dir()));
    d.insert(
        ion_obf_string!("readonly"),
        Value::Bool(md.permissions().readonly()),
    );
    let modified = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| Value::Int(d.as_secs() as i64))
        .unwrap_or(Value::Unit);
    d.insert(ion_obf_string!("modified"), modified);
    Value::Dict(d)
}

#[cfg(feature = "fs")]
fn fs_nonnegative_int_arg(args: &[Value], index: usize, context: &str) -> Result<u64, String> {
    let value = required_int_arg(args, index, context)?;
    if value < 0 {
        return Err(format!("{}{}", context, ion_str!(" must be non-negative")));
    }
    Ok(value as u64)
}

#[cfg(all(feature = "fs", not(feature = "async-runtime")))]
fn write_random_chunks<W: std::io::Write>(writer: &mut W, count: u64) -> std::io::Result<()> {
    let mut remaining = count;
    if remaining == 0 {
        return Ok(());
    }

    let mut rng = RandomSource::new();
    let mut buffer = vec![0u8; remaining.min(FS_RANDOM_CHUNK_LEN as u64) as usize];
    while remaining > 0 {
        let len = remaining.min(buffer.len() as u64) as usize;
        rng.fill_bytes(&mut buffer[..len]);
        std::io::Write::write_all(writer, &buffer[..len])?;
        remaining -= len as u64;
    }
    Ok(())
}

#[cfg(all(feature = "fs", feature = "async-runtime"))]
async fn write_random_chunks_async<W>(writer: &mut W, count: u64) -> std::io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;

    let mut remaining = count;
    if remaining == 0 {
        return Ok(());
    }

    let mut rng = RandomSource::new();
    let mut buffer = vec![0u8; remaining.min(FS_RANDOM_CHUNK_LEN as u64) as usize];
    while remaining > 0 {
        let len = remaining.min(buffer.len() as u64) as usize;
        rng.fill_bytes(&mut buffer[..len]);
        writer.write_all(&buffer[..len]).await?;
        remaining -= len as u64;
    }
    Ok(())
}

#[cfg(all(feature = "fs", not(feature = "async-runtime")))]
fn fs_append_random_sync(path: &str, count: u64) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    write_random_chunks(&mut file, count)
}

#[cfg(all(feature = "fs", not(feature = "async-runtime")))]
fn fs_pad_random_sync(path: &str, target_size: u64) -> std::io::Result<u64> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let current_size = file.metadata()?.len();
    if target_size <= current_size {
        return Ok(0);
    }

    let count = target_size - current_size;
    write_random_chunks(&mut file, count)?;
    Ok(count)
}

#[cfg(all(feature = "fs", feature = "async-runtime"))]
async fn fs_append_random_async(path: &str, count: u64) -> std::io::Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    write_random_chunks_async(&mut file, count).await
}

#[cfg(all(feature = "fs", feature = "async-runtime"))]
async fn fs_pad_random_async(path: &str, target_size: u64) -> std::io::Result<u64> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    let current_size = file.metadata().await?.len();
    if target_size <= current_size {
        return Ok(0);
    }

    let count = target_size - current_size;
    write_random_chunks_async(&mut file, count).await?;
    Ok(count)
}

#[cfg(all(feature = "fs", not(feature = "async-runtime")))]
fn fs_arg_str<'a>(args: &'a [Value], idx: usize) -> Result<&'a str, String> {
    args[idx].as_str().ok_or_else(|| {
        format!(
            "{}{}{}",
            ion_str!("argument "),
            idx + 1,
            ion_str!(" must be a string")
        )
    })
}

/// Build the `fs::` stdlib module — sync impl backed by `std::fs`. Used when
/// the `async-runtime` feature is **not** enabled.
#[cfg(all(feature = "fs", not(feature = "async-runtime")))]
pub fn fs_module() -> Module {
    let mut m = Module::new(crate::h!("fs"));

    m.register_fn(crate::h!("read"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        std::fs::read_to_string(path)
            .map(Value::Str)
            .map_err(|e| ion_format!("('{}'): {}", path, e))
    });

    m.register_fn(crate::h!("read_bytes"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        std::fs::read(path)
            .map(Value::Bytes)
            .map_err(|e| ion_format!("('{}'): {}", path, e))
    });

    m.register_fn(crate::h!("write"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments: path, contents"));
        }
        let path = fs_arg_str(args, 0)?;
        let result = match &args[1] {
            Value::Str(s) => std::fs::write(path, s.as_bytes()),
            Value::Bytes(b) => std::fs::write(path, b),
            other => {
                return Err(format!(
                    "{}{}",
                    ion_str!("invalid contents: "),
                    other.type_name()
                ));
            }
        };
        result
            .map(|_| Value::Unit)
            .map_err(|e| ion_format!("('{}'): {}", path, e))
    });

    m.register_fn(crate::h!("append"), |args: &[Value]| {
        use std::io::Write;
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments: path, contents"));
        }
        let path = fs_arg_str(args, 0)?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| ion_format!("('{}'): {}", path, e))?;
        let bytes: &[u8] = match &args[1] {
            Value::Str(s) => s.as_bytes(),
            Value::Bytes(b) => b,
            other => {
                return Err(format!(
                    "{}{}",
                    ion_str!("invalid contents: "),
                    other.type_name()
                ));
            }
        };
        f.write_all(bytes)
            .map(|_| Value::Unit)
            .map_err(|e| ion_format!("('{}'): {}", path, e))
    });

    m.register_fn(crate::h!("append_random"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments: path, count"));
        }
        let path = fs_arg_str(args, 0)?;
        let count = fs_nonnegative_int_arg(args, 1, "fs::append_random count")?;
        fs_append_random_sync(path, count)
            .map(|_| Value::Int(count as i64))
            .map_err(|e| ion_format!("('{}'): {}", path, e))
    });

    m.register_fn(crate::h!("pad_random"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments: path, target_size"));
        }
        let path = fs_arg_str(args, 0)?;
        let target_size = fs_nonnegative_int_arg(args, 1, "fs::pad_random target_size")?;
        fs_pad_random_sync(path, target_size)
            .map(|count| Value::Int(count as i64))
            .map_err(|e| ion_format!("('{}'): {}", path, e))
    });

    m.register_fn(crate::h!("exists"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        Ok(Value::Bool(std::path::Path::new(path).exists()))
    });

    m.register_fn(crate::h!("is_file"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        Ok(Value::Bool(std::path::Path::new(path).is_file()))
    });

    m.register_fn(crate::h!("is_dir"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        Ok(Value::Bool(std::path::Path::new(path).is_dir()))
    });

    m.register_fn(crate::h!("list_dir"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        let entries = std::fs::read_dir(path).map_err(|e| ion_format!("('{}'): {}", path, e))?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| ion_format!("('{}'): {}", path, e))?;
            names.push(Value::Str(entry.file_name().to_string_lossy().into_owned()));
        }
        Ok(Value::List(names))
    });

    m.register_fn(crate::h!("create_dir"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        std::fs::create_dir(path)
            .map(|_| Value::Unit)
            .map_err(|e| ion_format!("('{}'): {}", path, e))
    });

    m.register_fn(crate::h!("create_dir_all"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        std::fs::create_dir_all(path)
            .map(|_| Value::Unit)
            .map_err(|e| ion_format!("('{}'): {}", path, e))
    });

    m.register_fn(crate::h!("remove_file"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        std::fs::remove_file(path)
            .map(|_| Value::Unit)
            .map_err(|e| ion_format!("('{}'): {}", path, e))
    });

    m.register_fn(crate::h!("remove_dir"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        std::fs::remove_dir(path)
            .map(|_| Value::Unit)
            .map_err(|e| ion_format!("('{}'): {}", path, e))
    });

    m.register_fn(crate::h!("remove_dir_all"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        std::fs::remove_dir_all(path)
            .map(|_| Value::Unit)
            .map_err(|e| ion_format!("('{}'): {}", path, e))
    });

    m.register_fn(crate::h!("rename"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments: from, to"));
        }
        let from = fs_arg_str(args, 0)?;
        let to = fs_arg_str(args, 1)?;
        std::fs::rename(from, to)
            .map(|_| Value::Unit)
            .map_err(|e| ion_format!("('{}' -> '{}'): {}", from, to, e))
    });

    m.register_fn(crate::h!("copy"), |args: &[Value]| {
        if args.len() != 2 {
            return Err(ion_str!("takes 2 arguments: from, to"));
        }
        let from = fs_arg_str(args, 0)?;
        let to = fs_arg_str(args, 1)?;
        std::fs::copy(from, to)
            .map(|n| Value::Int(n as i64))
            .map_err(|e| ion_format!("('{}' -> '{}'): {}", from, to, e))
    });

    m.register_fn(crate::h!("metadata"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        let md = std::fs::metadata(path).map_err(|e| ion_format!("('{}'): {}", path, e))?;
        Ok(fs_metadata_to_dict(&md))
    });

    m.register_fn(crate::h!("canonicalize"), |args: &[Value]| {
        if args.len() != 1 {
            return Err(ion_str!("takes 1 argument"));
        }
        let path = fs_arg_str(args, 0)?;
        std::fs::canonicalize(path)
            .map(|p| Value::Str(p.to_string_lossy().into_owned()))
            .map_err(|e| ion_format!("('{}'): {}", path, e))
    });

    m
}

/// Build the `fs::` stdlib module — async impl backed by `tokio::fs`. Used
/// when the `async-runtime` feature is enabled. Surface matches the sync
/// build exactly; scripts call these the same way under `Engine::eval_async`.
#[cfg(all(feature = "fs", feature = "async-runtime"))]
pub fn fs_module() -> Module {
    use crate::error::IonError;

    fn arg_str(args: &[Value], idx: usize) -> Result<String, IonError> {
        args.get(idx)
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .ok_or_else(|| {
                IonError::runtime(
                    format!(
                        "{}{}{}",
                        ion_str!("argument "),
                        idx + 1,
                        ion_str!(" must be a string")
                    ),
                    0,
                    0,
                )
            })
    }

    fn io_err(target: &str, e: std::io::Error) -> IonError {
        IonError::runtime(ion_format!("'{}': {}", target, e), 0, 0)
    }

    let mut m = Module::new(crate::h!("fs"));

    m.register_async_fn(crate::h!("read"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        tokio::fs::read_to_string(&path)
            .await
            .map(Value::Str)
            .map_err(|e| io_err(&path, e))
    });

    m.register_async_fn(crate::h!("read_bytes"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        tokio::fs::read(&path)
            .await
            .map(Value::Bytes)
            .map_err(|e| io_err(&path, e))
    });

    m.register_async_fn(crate::h!("write"), |args| async move {
        if args.len() != 2 {
            return Err(IonError::runtime(
                ion_str!("takes 2 arguments: path, contents"),
                0,
                0,
            ));
        }
        let path = arg_str(&args, 0)?;
        let bytes: Vec<u8> = match &args[1] {
            Value::Str(s) => s.as_bytes().to_vec(),
            Value::Bytes(b) => b.clone(),
            other => {
                return Err(IonError::runtime(
                    format!("{}{}", ion_str!("invalid contents: "), other.type_name()),
                    0,
                    0,
                ));
            }
        };
        tokio::fs::write(&path, &bytes)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err(&path, e))
    });

    m.register_async_fn(crate::h!("append"), |args| async move {
        use tokio::io::AsyncWriteExt;
        if args.len() != 2 {
            return Err(IonError::runtime(
                ion_str!("takes 2 arguments: path, contents"),
                0,
                0,
            ));
        }
        let path = arg_str(&args, 0)?;
        let bytes: Vec<u8> = match &args[1] {
            Value::Str(s) => s.as_bytes().to_vec(),
            Value::Bytes(b) => b.clone(),
            other => {
                return Err(IonError::runtime(
                    format!("{}{}", ion_str!("invalid contents: "), other.type_name()),
                    0,
                    0,
                ));
            }
        };
        let mut f = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|e| io_err(&path, e))?;
        f.write_all(&bytes)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err(&path, e))
    });

    m.register_async_fn(crate::h!("append_random"), |args| async move {
        if args.len() != 2 {
            return Err(IonError::runtime(
                ion_str!("takes 2 arguments: path, count"),
                0,
                0,
            ));
        }
        let path = arg_str(&args, 0)?;
        let count = fs_nonnegative_int_arg(&args, 1, "fs::append_random count")
            .map_err(|e| IonError::runtime(e, 0, 0))?;
        fs_append_random_async(&path, count)
            .await
            .map(|_| Value::Int(count as i64))
            .map_err(|e| io_err(&path, e))
    });

    m.register_async_fn(crate::h!("pad_random"), |args| async move {
        if args.len() != 2 {
            return Err(IonError::runtime(
                ion_str!("takes 2 arguments: path, target_size"),
                0,
                0,
            ));
        }
        let path = arg_str(&args, 0)?;
        let target_size = fs_nonnegative_int_arg(&args, 1, "fs::pad_random target_size")
            .map_err(|e| IonError::runtime(e, 0, 0))?;
        fs_pad_random_async(&path, target_size)
            .await
            .map(|count| Value::Int(count as i64))
            .map_err(|e| io_err(&path, e))
    });

    m.register_async_fn(crate::h!("exists"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        // `tokio::fs::try_exists` on stable. Fall back to a metadata check so
        // we behave the same as `Path::exists()` did in the sync impl.
        match tokio::fs::metadata(&path).await {
            Ok(_) => Ok(Value::Bool(true)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Bool(false)),
            Err(e) => Err(io_err(&path, e)),
        }
    });

    m.register_async_fn(crate::h!("is_file"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        match tokio::fs::metadata(&path).await {
            Ok(md) => Ok(Value::Bool(md.is_file())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Bool(false)),
            Err(e) => Err(io_err(&path, e)),
        }
    });

    m.register_async_fn(crate::h!("is_dir"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        match tokio::fs::metadata(&path).await {
            Ok(md) => Ok(Value::Bool(md.is_dir())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Bool(false)),
            Err(e) => Err(io_err(&path, e)),
        }
    });

    m.register_async_fn(crate::h!("list_dir"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        let mut rd = tokio::fs::read_dir(&path)
            .await
            .map_err(|e| io_err(&path, e))?;
        let mut names = Vec::new();
        loop {
            match rd.next_entry().await {
                Ok(Some(entry)) => {
                    names.push(Value::Str(entry.file_name().to_string_lossy().into_owned()));
                }
                Ok(None) => break,
                Err(e) => return Err(io_err(&path, e)),
            }
        }
        Ok(Value::List(names))
    });

    m.register_async_fn(crate::h!("create_dir"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        tokio::fs::create_dir(&path)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err(&path, e))
    });

    m.register_async_fn(crate::h!("create_dir_all"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        tokio::fs::create_dir_all(&path)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err(&path, e))
    });

    m.register_async_fn(crate::h!("remove_file"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        tokio::fs::remove_file(&path)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err(&path, e))
    });

    m.register_async_fn(crate::h!("remove_dir"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        tokio::fs::remove_dir(&path)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err(&path, e))
    });

    m.register_async_fn(crate::h!("remove_dir_all"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        tokio::fs::remove_dir_all(&path)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| io_err(&path, e))
    });

    m.register_async_fn(crate::h!("rename"), |args| async move {
        if args.len() != 2 {
            return Err(IonError::runtime(
                ion_str!("takes 2 arguments: from, to"),
                0,
                0,
            ));
        }
        let from = arg_str(&args, 0)?;
        let to = arg_str(&args, 1)?;
        tokio::fs::rename(&from, &to)
            .await
            .map(|_| Value::Unit)
            .map_err(|e| IonError::runtime(ion_format!("('{}' -> '{}'): {}", from, to, e), 0, 0))
    });

    m.register_async_fn(crate::h!("copy"), |args| async move {
        if args.len() != 2 {
            return Err(IonError::runtime(
                ion_str!("takes 2 arguments: from, to"),
                0,
                0,
            ));
        }
        let from = arg_str(&args, 0)?;
        let to = arg_str(&args, 1)?;
        tokio::fs::copy(&from, &to)
            .await
            .map(|n| Value::Int(n as i64))
            .map_err(|e| IonError::runtime(ion_format!("('{}' -> '{}'): {}", from, to, e), 0, 0))
    });

    m.register_async_fn(crate::h!("metadata"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        let md = tokio::fs::metadata(&path)
            .await
            .map_err(|e| io_err(&path, e))?;
        Ok(fs_metadata_to_dict(&md))
    });

    m.register_async_fn(crate::h!("canonicalize"), |args| async move {
        if args.len() != 1 {
            return Err(IonError::runtime(ion_str!("takes 1 argument"), 0, 0));
        }
        let path = arg_str(&args, 0)?;
        tokio::fs::canonicalize(&path)
            .await
            .map(|p| Value::Str(p.to_string_lossy().into_owned()))
            .map_err(|e| io_err(&path, e))
    });

    m
}

/// Register all stdlib modules in the given environment.
pub fn register_stdlib(env: &mut crate::env::Env) {
    register_stdlib_with_output(env, missing_output_handler());
}

/// Register all stdlib modules with a host-provided output handler.
pub fn register_stdlib_with_output(env: &mut crate::env::Env, output: Arc<dyn OutputHandler>) {
    let level = crate::log::AtomicLogLevel::default_runtime();
    let log_handler: Arc<dyn crate::log::LogHandler> = Arc::new(
        crate::log::StdLogHandler::with_threshold(Arc::clone(&level)),
    );
    register_stdlib_with_handlers(env, output, log_handler, level);
}

/// Register all stdlib modules with both a host output handler and a log
/// handler. The shared `level` is used by `log::set_level`/`log::level` to
/// gate the default `StdLogHandler`. Custom handlers are free to ignore it.
pub fn register_stdlib_with_handlers(
    env: &mut crate::env::Env,
    output: Arc<dyn OutputHandler>,
    log_handler: Arc<dyn crate::log::LogHandler>,
    level: Arc<crate::log::AtomicLogLevel>,
) {
    fn install(env: &mut crate::env::Env, m: Module) {
        let h = m.name_hash();
        env.define_h(h, m.into_value());
    }

    install(env, math_module());
    install(env, json_module());
    install(env, bytes_module());
    install(env, rand_module());
    install(env, io_module_with_output(output));
    install(env, string_module());
    install(env, log_module_with_handler(log_handler, level));

    #[cfg(feature = "semver")]
    install(env, semver_module());

    #[cfg(feature = "os")]
    install(env, os_module());

    install(env, path_module());

    #[cfg(feature = "fs")]
    install(env, fs_module());
}
