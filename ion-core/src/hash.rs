//! Compile-time string hashing for host-registered names.
//!
//! Enums, variants, modules, functions, and qualified `mod::fn` paths are
//! hashed at macro-expansion / `const` time so the source string never lands
//! in the host binary's `.rodata`. At runtime, lookups are integer compares
//! against `u64` slot keys.
//!
//! Algorithm: FNV-1a 64-bit. Chosen for simplicity (~10 lines, `const fn`),
//! good distribution at small input sizes (identifiers), and zero deps. It
//! is **not** a cryptographic hash; it is not designed to resist collision
//! attacks. Collisions inside a single registry are detected at registration
//! time and panic the host startup — see docs/hide-names.md.

/// FNV-1a 64-bit hash. `const fn` so the result is computed by `rustc`
/// during compilation when the input is a constant.
pub const fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x100000001b3);
        i += 1;
    }
    h
}

/// Convenience wrapper over [`fnv1a64`] for `&str` inputs.
pub const fn h(s: &str) -> u64 {
    fnv1a64(s.as_bytes())
}

/// Mix two hashes into one. Used to derive `qualified_hash` from
/// `(module_hash, fn_hash)` without re-hashing the joined string.
///
/// Mixing strategy: FNV-1a-style fold of the second hash's bytes into the
/// running state seeded by the first. Order-sensitive: `mix(a, b) != mix(b, a)`.
pub const fn mix(a: u64, b: u64) -> u64 {
    let mut h = a;
    let bytes = b.to_le_bytes();
    let mut i = 0;
    while i < 8 {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x100000001b3);
        i += 1;
    }
    h
}

pub(crate) fn is_to_string_name(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 9
        && matches!(
            (
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                bytes[8],
            ),
            (116, 111, 95, 115, 116, 114, 105, 110, 103)
        )
}

#[cfg(feature = "async-runtime")]
pub(crate) fn is_len_name(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 3 && matches!((bytes[0], bytes[1], bytes[2]), (108, 101, 110))
}

pub(crate) fn is_log_name(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 3 && matches!((bytes[0], bytes[1], bytes[2]), (108, 111, 103))
}

pub(crate) fn starts_with_option_type(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 6
        && matches!(
            (bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]),
            (79, 112, 116, 105, 111, 110)
        )
}

pub(crate) fn starts_with_result_type(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 6
        && matches!(
            (bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]),
            (82, 101, 115, 117, 108, 116)
        )
}

pub(crate) fn starts_with_list_generic_type(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 5
        && matches!(
            (bytes[0], bytes[1], bytes[2], bytes[3], bytes[4]),
            (108, 105, 115, 116, 60)
        )
}

pub(crate) fn starts_with_dict_generic_type(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 5
        && matches!(
            (bytes[0], bytes[1], bytes[2], bytes[3], bytes[4]),
            (100, 105, 99, 116, 60)
        )
}

/// Hash a string literal at compile time. Expands to a `u64` expression
/// whose value is the FNV-1a hash of `$s` — the literal is folded by the
/// compiler and does not survive in the **release** binary.
///
/// In **debug builds** (`cfg(debug_assertions)`), the macro additionally
/// registers `(hash, $s)` with [`crate::names`] on first execution, via a
/// per-site `Once`. This gives readable diagnostics in tests and dev
/// builds without touching release performance or binary size — see
/// `docs/hide-names.md`.
///
/// **Const contexts:** the debug-build registration uses `Once::call_once`
/// which is not `const`, so under `cfg(debug_assertions)` the macro cannot
/// be used in `const`/`static` positions. Use [`crate::hash::h`] (the bare
/// `const fn`) directly when a constant is required.
///
/// ```
/// use ion_core::h;
/// let color_hash: u64 = h!("Color");
/// assert_eq!(color_hash, ion_core::hash::h("Color"));
/// ```
#[macro_export]
macro_rules! h {
    ($s:expr) => {{
        const __ION_HASH: u64 = $crate::hash::h($s);
        #[cfg(debug_assertions)]
        {
            static __ION_REG: ::std::sync::Once = ::std::sync::Once::new();
            __ION_REG.call_once(|| $crate::names::register(__ION_HASH, $s));
        }
        __ION_HASH
    }};
}

/// Hash two `&str` literals as `"a::b"` at compile time without ever
/// constructing the joined string. Equivalent to `h!(concat!(a, "::", b))`
/// in observable output but computed via [`mix`] to avoid emitting the
/// concatenated literal anywhere.
///
/// In debug builds, also registers `(qualified_hash, "mod::name")` with
/// [`crate::names`] for readable diagnostics. The joined string is only
/// constructed on the first call per site; release builds skip it entirely.
#[macro_export]
macro_rules! qh {
    ($mod:expr, $name:expr) => {{
        const __ION_QHASH: u64 = $crate::hash::mix($crate::hash::h($mod), $crate::hash::h($name));
        #[cfg(debug_assertions)]
        {
            static __ION_REG: ::std::sync::Once = ::std::sync::Once::new();
            __ION_REG.call_once(|| {
                let joined: &'static str =
                    ::std::boxed::Box::leak(format!("{}::{}", $mod, $name).into_boxed_str());
                $crate::names::register(__ION_QHASH, joined);
            });
        }
        __ION_QHASH
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_matches_known_vectors() {
        // Standard FNV-1a 64 test vectors (http://www.isthe.com/chongo/tech/comp/fnv/).
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn h_macro_matches_const_fn() {
        let x: u64 = h!("Color");
        assert_eq!(x, h("Color"));
    }

    #[test]
    fn distinct_inputs_distinct_hashes() {
        // Sanity: identifiers we expect to use don't collide.
        let names = [
            "Color",
            "Red",
            "Green",
            "Blue",
            "Custom",
            "Shape",
            "Circle",
            "Rect",
            "Empty",
            "math",
            "json",
            "io",
            "str",
            "log",
            "os",
            "path",
            "fs",
            "semver",
            "abs",
            "min",
            "max",
            "sqrt",
            "pow",
            "floor",
            "ceil",
            "round",
            "encode",
            "decode",
            "pretty",
            "msgpack_encode",
            "msgpack_decode",
            "join",
            "parse",
            "format",
            "compare",
            "eq",
            "lt",
            "gt",
            "lte",
            "gte",
        ];
        let mut hashes: Vec<u64> = names.iter().map(|n| h(n)).collect();
        hashes.sort();
        let len_before = hashes.len();
        hashes.dedup();
        assert_eq!(
            hashes.len(),
            len_before,
            "unexpected collision in stdlib name set"
        );
    }

    #[test]
    fn mix_is_order_sensitive() {
        let a = h("math");
        let b = h("abs");
        assert_ne!(mix(a, b), mix(b, a));
    }

    #[test]
    fn qh_matches_mix() {
        let q: u64 = qh!("math", "abs");
        assert_eq!(q, mix(h("math"), h("abs")));
    }
}
