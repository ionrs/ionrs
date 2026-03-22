#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let mut lexer = ion_core::lexer::Lexer::new(s);
        if let Ok(tokens) = lexer.tokenize() {
            let mut parser = ion_core::parser::Parser::new(tokens);
            let _ = parser.parse_program();
        }
    }
});
