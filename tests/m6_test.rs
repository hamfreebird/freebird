//! M6 正确性与一致性测试。

use freebird::{RuntimeConfig, compile_program, run_source};

#[test]
fn mir_generator_has_no_todo_fallback() {
    let source = include_str!("../src/mir/generator.rs");
    assert!(
        !source.contains("TODO"),
        "MIR generator must not contain TODO placeholders"
    );
}

#[test]
fn id_expressions_are_rejected_clearly() {
    let source = "fn main() { let x = #foo; print(x); }";
    let error = match compile_program(source) {
        Ok(_) => panic!("#id must be rejected"),
        Err(error) => error,
    };
    assert!(
        error.contains("`#id`") || error.contains("unsupported feature"),
        "unexpected error: {error}"
    );
    assert!(run_source(source, RuntimeConfig::test()).is_err());
}
