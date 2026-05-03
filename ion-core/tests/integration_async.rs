// Async-build integration coverage. Mirrors the most important sync tests
// from `integration.rs` so the `tokio::fs`-backed surface gets exercised
// end-to-end. This file only compiles with the `async-runtime` feature; in
// sync builds it's a no-op.

#![cfg(feature = "async-runtime")]

use ion_core::engine::Engine;
use ion_core::value::Value;

/// Build a current-thread tokio runtime and drive `engine.eval_async` on it.
/// Mirrors how an embedder like the `ion` CLI dispatches under
/// `--features async-runtime`.
fn eval(src: &str) -> Value {
    let mut engine = Engine::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    rt.block_on(engine.eval_async(src)).unwrap()
}

fn eval_err(src: &str) -> String {
    let mut engine = Engine::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    rt.block_on(engine.eval_async(src)).unwrap_err().message
}

fn fs_test_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ion_fs_async_test_{}_{}_{}",
        label,
        std::process::id(),
        id
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    dir
}

// --- Sanity: pure sync programs still work under eval_async ---

#[test]
fn test_async_pure_sync_program() {
    assert_eq!(eval("1 + 2"), Value::Int(3));
}

#[test]
fn test_async_path_module_works() {
    // `path::*` is pure-string; compiles to non-async builtins. Confirm it
    // still runs under eval_async (no async/sync confusion in the bridge).
    assert_eq!(
        eval(r#"path::basename("/usr/local/bin/ion")"#),
        Value::Str("ion".to_string())
    );
}

// --- fs:: under tokio ---

#[cfg(feature = "fs")]
#[test]
fn test_async_fs_read_write_roundtrip() {
    let dir = fs_test_dir("rw");
    let path = dir.join("hello.txt");
    let path_s = path.to_string_lossy().to_string();
    let mut engine = Engine::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        engine
            .eval_async(&format!(r#"fs::write("{}", "hello async")"#, path_s))
            .await
            .unwrap();
        let v = engine
            .eval_async(&format!(r#"fs::read("{}")"#, path_s))
            .await
            .unwrap();
        assert_eq!(v, Value::Str("hello async".to_string()));
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_async_fs_read_bytes() {
    let dir = fs_test_dir("bytes");
    let path = dir.join("blob.bin");
    std::fs::write(&path, [0xCA, 0xFE]).unwrap();
    let v = eval(&format!(
        r#"fs::read_bytes("{}")"#,
        path.to_string_lossy()
    ));
    assert_eq!(v, Value::Bytes(vec![0xCA, 0xFE]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_async_fs_append() {
    let dir = fs_test_dir("append");
    let path = dir.join("log.txt");
    let path_s = path.to_string_lossy().to_string();
    let mut engine = Engine::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        engine
            .eval_async(&format!(r#"fs::write("{}", "1")"#, path_s))
            .await
            .unwrap();
        engine
            .eval_async(&format!(r#"fs::append("{}", "2")"#, path_s))
            .await
            .unwrap();
        engine
            .eval_async(&format!(r#"fs::append("{}", "3")"#, path_s))
            .await
            .unwrap();
        let v = engine
            .eval_async(&format!(r#"fs::read("{}")"#, path_s))
            .await
            .unwrap();
        assert_eq!(v, Value::Str("123".to_string()));
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_async_fs_exists_is_file_is_dir() {
    let dir = fs_test_dir("exists");
    let file = dir.join("f.txt");
    std::fs::write(&file, "x").unwrap();
    let dir_s = dir.to_string_lossy().to_string();
    let file_s = file.to_string_lossy().to_string();
    let missing = dir.join("nope.txt").to_string_lossy().to_string();
    assert_eq!(
        eval(&format!(r#"fs::exists("{}")"#, file_s)),
        Value::Bool(true)
    );
    assert_eq!(
        eval(&format!(r#"fs::exists("{}")"#, missing)),
        Value::Bool(false)
    );
    assert_eq!(
        eval(&format!(r#"fs::is_file("{}")"#, file_s)),
        Value::Bool(true)
    );
    assert_eq!(
        eval(&format!(r#"fs::is_dir("{}")"#, dir_s)),
        Value::Bool(true)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_async_fs_list_dir() {
    let dir = fs_test_dir("list");
    std::fs::write(dir.join("a"), "").unwrap();
    std::fs::write(dir.join("b"), "").unwrap();
    let v = eval(&format!(r#"fs::list_dir("{}")"#, dir.to_string_lossy()));
    let Value::List(items) = v else {
        panic!("expected list");
    };
    let mut names: Vec<String> = items
        .iter()
        .map(|v| match v {
            Value::Str(s) => s.clone(),
            _ => panic!("expected strings"),
        })
        .collect();
    names.sort();
    assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_async_fs_create_remove_dirs() {
    let root = fs_test_dir("createrm");
    let nested = root.join("a/b/c");
    let nested_s = nested.to_string_lossy().to_string();
    let mut engine = Engine::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        engine
            .eval_async(&format!(r#"fs::create_dir_all("{}")"#, nested_s))
            .await
            .unwrap();
        assert!(nested.exists());
        engine
            .eval_async(&format!(
                r#"fs::remove_dir_all("{}")"#,
                root.to_string_lossy()
            ))
            .await
            .unwrap();
        assert!(!root.exists());
    });
}

#[cfg(feature = "fs")]
#[test]
fn test_async_fs_metadata() {
    let dir = fs_test_dir("meta");
    let file = dir.join("m.txt");
    std::fs::write(&file, "12345").unwrap();
    let v = eval(&format!(r#"fs::metadata("{}")"#, file.to_string_lossy()));
    let Value::Dict(map) = v else {
        panic!("expected dict");
    };
    assert_eq!(map.get("size"), Some(&Value::Int(5)));
    assert_eq!(map.get("is_file"), Some(&Value::Bool(true)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(feature = "fs")]
#[test]
fn test_async_fs_read_missing_errors() {
    // Phase 7 cleanup: error literals are generic; the path appears
    // in the formatted io error since it's user data.
    let err = eval_err(r#"fs::read("/no/such/path/exists/zP9")"#);
    assert!(err.contains("zP9"), "got: {}", err);
}

// --- io:: under async stays non-blocking ---

#[test]
fn test_async_io_println_via_eval_async() {
    // Smoke: io::println is registered as async under the async runtime.
    // Calling it under eval_async should succeed (the OutputHandler write is
    // dispatched onto a blocking thread; the future resolves to Unit).
    use ion_core::stdlib::OutputStream;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct CaptureBuf(Mutex<String>);
    impl ion_core::stdlib::OutputHandler for CaptureBuf {
        fn write(&self, _stream: OutputStream, text: &str) -> Result<(), String> {
            self.0.lock().unwrap().push_str(text);
            Ok(())
        }
    }

    let buf = Arc::new(CaptureBuf::default());
    let mut engine = Engine::with_output_handler(buf.clone());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(engine.eval_async(r#"io::println("hello async")"#))
        .unwrap();
    assert_eq!(buf.0.lock().unwrap().as_str(), "hello async\n");
}
