# Rust Error Handling

Rust errors must be useful for development without exposing sensitive runtime
details in release builds. The project uses `thiserror` for typed errors and
the published `redacted-error` crate for release-safe formatting.

## Goals

- Keep rich error messages in debug builds.
- Strip dynamic details from `Display` and `Debug` in release builds.
- Keep static public messages behind a stable facade.
- Give backend and protocol code stable machine-readable error codes.
- Avoid backend behavior that depends on parsing human error text.

## Dependency

Use `redacted-error` for Rust error types that cross crate, protocol, service,
or process boundaries.

```toml
[dependencies]
redacted-error = "0.2"
```

The default `obfuscate` feature currently uses `obfstr` internally for static
public messages. Callers must use the `redacted_error` API rather than depending
on that backend.

Disable static string obfuscation only in crates that do not contain sensitive
static strings:

```toml
[dependencies]
redacted-error = { version = "0.2", default-features = false }
```

## String Sensitivity

Obfuscation is for strings that reveal implementation, protocol, or operator
intent. It is not a blanket rule for every literal.

Obfuscation only reduces static binary inspection. It does not make a string
safe to show in `Display`, `PublicError`, logs, or serialized API responses.

Do not obfuscate empty strings, 1-character strings, or one-word strings that
are not sensitive. Use plain literals or `redacted_error::Message::from_static`
for those cases when a `Message` value is required.

Always obfuscate protocol strings, function names, method names, and Ion script
binding names. This includes wire keys, service names, RPC method names,
function or method identifiers, host binding names, and strings that describe
offensive security behavior.

Error messages must not leak protocol details, function names, method names, or
Ion binding names. Release-facing messages should use generic public wording
such as `request failed`, `invalid request`, or `operation unavailable`; put
internal routing in private codes, privileged logs, or debug-only detail
instead.

Sensitive strings include, but are not limited to, protocol identifiers,
function and method names, Ion bindings, and offensive security terms.

## Error Types

```rust
use redacted_error::message as m;
use thiserror::Error;

#[cfg_attr(debug_assertions, derive(Debug))]
#[derive(Error)]
pub enum TransportError {
    #[cfg_attr(
        debug_assertions,
        error("{prefix} {0}", prefix = m!("listener bind failed:"))
    )]
    #[cfg_attr(not(debug_assertions), error("{}", m!("listener bind failed")))]
    ListenerBindFailed(String),
}

redacted_error::impl_redacted_debug!(TransportError);
```

In debug builds this formats as:

```text
listener bind failed: 127.0.0.1:8080: address already in use
```

In release builds this formats as:

```text
listener bind failed
```

The release `Debug` implementation delegates to release `Display`, so
`format!("{err:?}")` does not accidentally dump enum fields.

`message!` accepts string literals and returns `redacted_error::Message`, a
displayable wrapper around the public text. Use `message_string!` only when an
owned `String` is required by an external API.

## Error Codes

Any error that needs to be consumed by backend, operator, protocol, or API code
should implement `redacted_error::ErrorCode`. If the error also crosses an API
or protocol boundary, implement `redacted_error::PublicError`.

```rust
impl redacted_error::ErrorCode for TransportError {
    fn code(&self) -> redacted_error::Message {
        match self {
            Self::ListenerBindFailed(_) => {
                redacted_error::message!("transport.listener_bind_failed")
            }
        }
    }
}

impl redacted_error::PublicError for TransportError {
    fn public_message(&self) -> redacted_error::Message {
        match self {
            Self::ListenerBindFailed(_) => {
                redacted_error::message!("listener bind failed")
            }
        }
    }
}
```

Backend behavior must use `code()`, `public_message()`, or explicit structured
fields, not `err.to_string()`.

Codes returned to public clients are public data. Do not include protocol
methods, function names, method names, Ion bindings, or offensive security
terms in externally serialized codes.

Good response shape:

```json
{
  "code": "transport.listener_bind_failed",
  "message": "listener bind failed"
}
```

When serializing, call `.as_str()` to borrow the message or `.into_string()` to
move it into an owned response type.

Debug-only or operator-only diagnostics may include a separate `detail` field,
but it must be gated by build mode, privilege, or explicit diagnostic mode.

## Runtime Detail Capture

When code constructs dynamic detail only for error messages, use the helper so
release builds do not retain the detail string:

```rust
return Err(MyError::InvalidConfig(redacted_error::detail!(
    "invalid endpoint {endpoint_id}: {reason}"
)));
```

For an existing displayable error:

```rust
.map_err(|err| MyError::Io(redacted_error::display(err)))?
```

This keeps the current variant shape while preventing release builds from
storing the formatted runtime detail.

## Rules

- Use `#[cfg_attr(debug_assertions, error(...))]` for detailed debug messages.
- Use `#[cfg_attr(not(debug_assertions), error("{}", m!("...")))]` for release
  messages.
- Derive `Debug` only in debug builds with
  `#[cfg_attr(debug_assertions, derive(Debug))]`.
- Add `redacted_error::impl_redacted_debug!(TypeName)` for release builds.
- Use `redacted_error::message!` for static public strings.
- Use `redacted_error::message_string!` only when an owned public string is
  required.
- Do not obfuscate empty strings, 1-character strings, or one-word
  non-sensitive strings.
- Obfuscate protocol strings, function names, method names, Ion binding names,
  and offensive security terms.
- Use `ErrorCode` and `PublicError` for backend-facing decisions and public API
  responses.
- Do not parse `Display` text for control flow.
- Do not use `#[error(transparent)]` for boundary errors unless the source is
  safe to expose in release builds.
- Avoid returning raw paths, addresses, tokens, module names, function names,
  method names, protocol identifiers, Ion binding names, remote messages, OS
  errors, SQL errors, or config values in release messages.

## Testing

Add focused tests for errors that cross process or API boundaries:

```rust
#[test]
fn display_and_debug_redact_runtime_detail_in_release() {
    let err = TransportError::ListenerBindFailed("secret detail".to_owned());
    let display = err.to_string();
    let debug = format!("{err:?}");

    #[cfg(debug_assertions)]
    {
        assert!(display.contains("secret detail"));
        assert!(debug.contains("secret detail"));
    }

    #[cfg(not(debug_assertions))]
    {
        assert_eq!(display, "listener bind failed");
        assert_eq!(debug, "listener bind failed");
    }
}
```

Run both debug and release checks for touched crates:

```sh
cargo test -p managed-transport
cargo test -p managed-transport --release error::tests::display_and_debug_redact_runtime_detail_in_release
cargo check -p managed-transport -p sensor-api-protocol --release
```
