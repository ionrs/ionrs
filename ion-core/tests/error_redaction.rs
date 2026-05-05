use ion_core::error::IonError;
use redacted_error::{ErrorCode, PublicError};

#[test]
fn ion_error_redacts_runtime_detail_in_release() {
    let err = IonError::runtime("secret path /tmp/token", 1, 5);
    let display = err.to_string();
    let debug = format!("{err:?}");

    assert_eq!(ErrorCode::code(&err), "ion.runtime_error");
    assert_eq!(PublicError::public_message(&err), "runtime error");

    #[cfg(debug_assertions)]
    {
        assert!(display.contains("secret path"));
        assert!(debug.contains("secret path"));
        assert!(err
            .format_with_source("let token = 1;\n")
            .contains("let token"));
    }

    #[cfg(not(debug_assertions))]
    {
        assert_eq!(display, "runtime error");
        assert_eq!(debug, "runtime error");
        assert_eq!(err.message, "runtime error");
        let formatted = err.format_with_source("let token = 1;\n");
        assert_eq!(formatted, "error: runtime error\n");
        assert!(!formatted.contains("token"));
    }
}
