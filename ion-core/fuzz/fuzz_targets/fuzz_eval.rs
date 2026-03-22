#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Limit input size to prevent timeouts
        if s.len() > 512 {
            return;
        }
        let mut engine = ion_core::engine::Engine::new();
        // Set tight limits to prevent infinite loops
        engine.set_limits(ion_core::interpreter::Limits {
            max_loop_iters: 100,
            max_call_depth: 20,
        });
        let _ = engine.eval(s);
    }
});
