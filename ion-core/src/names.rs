//! Optional hash → name resolution for diagnostics.
//!
//! Host-registered identifiers (enum names, variant names, module names,
//! function names, struct field names) are stored in `Value`s as `u64`
//! FNV-1a hashes — they don't appear in the release binary at all. This
//! module provides an *optional* runtime mapping back to source-form strings,
//! used solely by `Display`, error messages, and JSON/msgpack rendering.
//!
//! # When the registry is populated
//!
//! - **Debug builds (`cfg(debug_assertions)`)**: every `h!()` and `qh!()`
//!   site auto-registers its source literal exactly once on first execution,
//!   via a static `Once`. Tests, dev binaries, and `cargo run` builds all
//!   render readable names with zero extra setup.
//! - **Release builds**: the registry is empty by default. Hosts that want
//!   readable diagnostics in production load a sidecar file (see
//!   [`load_sidecar_json`]) or call [`register_many`] with a hand-built
//!   table. Without that, `Display` emits the opaque `<enum#hhhh…>` form.
//! - **Sidecar workflow**: a build script can call [`dump_sidecar_json`]
//!   on a fully-populated debug build to emit `myapp.names`, which the
//!   release host loads at startup.
//!
//! # Performance
//!
//! Lookups use `RwLock<HashMap<u64, &'static str>>`. They are off the
//! dispatch hot path — only `Display::fmt`, `to_json`, and error rendering
//! consult the registry. The registry is initialised lazily on first use.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

static REGISTRY: OnceLock<RwLock<HashMap<u64, &'static str>>> = OnceLock::new();

fn registry() -> &'static RwLock<HashMap<u64, &'static str>> {
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register a `name_hash → "source_name"` mapping. Idempotent: the same
/// pair can be registered any number of times. Distinct names colliding
/// on the same hash are not detected here (handled at registry-build time
/// for `TypeRegistry`/`Module`); the last-write-wins for diagnostics.
///
/// `&'static str` so that lookups don't have to clone — names are either
/// string literals from the source, leaked from a sidecar load, or
/// otherwise long-lived.
pub fn register(name_hash: u64, name: &'static str) {
    registry()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(name_hash, name);
}

/// Bulk-register many `(hash, name)` pairs in a single lock acquisition.
/// Use this when loading a sidecar or pre-populating known stdlib names.
pub fn register_many<I>(pairs: I)
where
    I: IntoIterator<Item = (u64, &'static str)>,
{
    let mut guard = registry().write().unwrap_or_else(|e| e.into_inner());
    for (hash, name) in pairs {
        guard.insert(hash, name);
    }
}

/// Look up the source name for a hash, if registered. Used by `Display`,
/// JSON/msgpack output, and error messages to render readable identifiers
/// in dev/staging builds while leaving release binaries opaque.
pub fn lookup(name_hash: u64) -> Option<&'static str> {
    registry()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&name_hash)
        .copied()
}

/// Dump the current registry to a JSON object keyed by zero-padded hex
/// strings, e.g. `{"8a3f9c127b4e1d56": "Color"}`. Suitable for emitting a
/// `.names` sidecar from a fully-populated debug build:
///
/// ```ignore
/// // build.rs after cargo test
/// let json = ion_core::names::dump_sidecar_json();
/// std::fs::write("target/release/myapp.names", json)?;
/// ```
///
/// Returns the serialized JSON. Empty registry yields `{}`.
pub fn dump_sidecar_json() -> String {
    let guard = registry().read().unwrap_or_else(|e| e.into_inner());
    let map: serde_json::Map<String, serde_json::Value> = guard
        .iter()
        .map(|(h, n)| {
            (
                format!("{:016x}", h),
                serde_json::Value::String(n.to_string()),
            )
        })
        .collect();
    serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or_else(|_| String::from("{}"))
}

/// Load a sidecar JSON written by [`dump_sidecar_json`]. Names are leaked
/// to obtain `&'static str` references — sidecar files are small (one entry
/// per registered identifier) and loaded once at startup, so the leak is
/// bounded.
///
/// On any parse error, the existing registry is left untouched and the
/// error is returned. Unknown JSON shapes (non-object roots, non-string
/// values, malformed hex keys) are skipped silently.
pub fn load_sidecar_json(json: &str) -> Result<usize, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ion_format!("sidecar parse error: {}", e))?;
    let obj = match value {
        serde_json::Value::Object(o) => o,
        _ => return Err(String::from("sidecar root must be a JSON object")),
    };

    let mut count = 0usize;
    let mut guard = registry().write().unwrap_or_else(|e| e.into_inner());
    for (k, v) in obj {
        let Ok(hash) = u64::from_str_radix(&k, 16) else {
            continue;
        };
        let Some(name) = v.as_str() else {
            continue;
        };
        let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
        guard.insert(hash, leaked);
        count += 1;
    }
    Ok(count)
}

/// Number of registered (hash → name) entries. Diagnostic only.
pub fn len() -> usize {
    registry().read().unwrap_or_else(|e| e.into_inner()).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_lookup_roundtrip() {
        register(0x1234_5678_9abc_def0, "alpha");
        assert_eq!(lookup(0x1234_5678_9abc_def0), Some("alpha"));
    }

    #[test]
    fn lookup_returns_none_for_unknown_hash() {
        assert_eq!(lookup(0xffff_ffff_ffff_ffff), None);
    }

    #[test]
    fn register_many_inserts_in_one_lock() {
        register_many([
            (0xaaaa_aaaa_aaaa_aaaa, "one"),
            (0xbbbb_bbbb_bbbb_bbbb, "two"),
        ]);
        assert_eq!(lookup(0xaaaa_aaaa_aaaa_aaaa), Some("one"));
        assert_eq!(lookup(0xbbbb_bbbb_bbbb_bbbb), Some("two"));
    }

    #[test]
    fn dump_and_reload_via_json_sidecar() {
        register(0x0123_4567_89ab_cdef, "via_dump");
        let json = dump_sidecar_json();
        assert!(json.contains("0123456789abcdef"));
        assert!(json.contains("via_dump"));
        let loaded = load_sidecar_json(&json).unwrap();
        assert!(loaded > 0);
    }
}
