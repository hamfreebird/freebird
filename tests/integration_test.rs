//! 集成测试 — 测试完整的 .fb 程序编译+执行

use freebird::{run_source, RuntimeConfig};
use std::fs;

fn run_file(path: &str) -> Result<Vec<u8>, String> {
    let source = fs::read_to_string(path).map_err(|e| e.to_string())?;
    run_source(&source, RuntimeConfig::test())
}

#[test]
fn test_fib_program() {
    let result = run_file("tests/programs/fib.fb");
    assert!(result.is_ok(), "fib.fb failed: {:?}", result.err());
}

#[test]
fn test_multi_ret_program() {
    let result = run_file("tests/programs/multi_ret.fb");
    assert!(result.is_ok(), "multi_ret.fb failed: {:?}", result.err());
}

#[test]
fn test_struct_field_program() {
    let result = run_file("tests/programs/struct_field.fb");
    assert!(result.is_ok(), "struct_field.fb failed: {:?}", result.err());
}

#[test]
fn test_hello_example() {
    let result = run_file("examples/hello.fb");
    assert!(result.is_ok(), "hello.fb failed: {:?}", result.err());
}
