//! M1 验收测试：运行时一致性。
//!
//! 覆盖 `read_line` 的 UTF-8 / CRLF / 非法字节解码、运行时错误状态，
//! 以及 VM 的运行时资源限制。Stardust 侧的差分由 `tests/common` 提供。

mod common;

use common::{assert_same_output, run_stardust, run_vm, Run};
use freebird::{check_source, run_source, RuntimeConfig};

const READ_LINE_PROGRAM: &str = r#"
fn main() {
    let s = read_line();
    print_str(s);
    print(s.len());
}
"#;

fn assert_read_line(name: &str, input: &[u8]) {
    assert_same_output(READ_LINE_PROGRAM, &format!("readline_{name}"), input);
}

#[test]
fn read_line_empty_line() {
    assert_read_line("empty", b"\n");
}

#[test]
fn read_line_crlf() {
    assert_read_line("crlf", b"abc\r\n");
}

#[test]
fn read_line_utf8() {
    assert_read_line("utf8", "你好，Freebird\n".as_bytes());
}

#[test]
fn read_line_invalid_lead_byte() {
    assert_read_line("invalid_lead", b"\xff\n");
}

#[test]
fn read_line_invalid_continuation() {
    // 0xC3 后跟非续字节 0x28：VM 产生 U+FFFD + '('。
    assert_read_line("invalid_cont", b"\xc3(\n");
}

#[test]
fn read_line_overlong_sequence() {
    // 0xC0 0x80：两个字节都非法，VM 产生两个 U+FFFD。
    assert_read_line("overlong", b"\xc0\x80\n");
}

#[test]
fn read_line_surrogate() {
    // 0xED 0xA0 0x80 是 UTF-8 代理项，三个字节都非法。
    assert_read_line("surrogate", b"\xed\xa0\x80\n");
}

#[test]
fn read_line_out_of_range() {
    // 0xF4 0x90 0x80 0x80 超过 U+10FFFF，四个字节都非法。
    assert_read_line("out_of_range", b"\xf4\x90\x80\x80\n");
}

#[test]
fn read_line_grows_beyond_initial_capacity() {
    assert_read_line(
        "long",
        b"abcdefghijklmnopqrstuvwxyz0123456789\n",
    );
}

#[test]
fn read_line_multiple_lines() {
    assert_read_line("two_lines", b"first\nsecond\n");
}

#[test]
fn read_line_eof_is_a_documented_backend_difference() {
    // Stardust 解释器的 CharIn 在 EOF 直接报 I/O 错误，生成代码无法捕获；
    // VM 则返回空字符串。该差异记录在 USAGE.md，测试在此固定其行为。
    let vm = run_vm(READ_LINE_PROGRAM, b"");
    assert!(
        matches!(vm, Run::Ok(ref output) if output == b"0\n"),
        "VM EOF should return an empty string, got {vm:?}"
    );
    let sd = run_stardust(READ_LINE_PROGRAM, "readline_eof", b"");
    assert!(
        matches!(sd, Run::Failed(_)),
        "Stardust EOF should fail in the bundled interpreter, got {sd:?}"
    );
}

#[test]
fn io_formatting_matches() {
    let source = r#"
fn main() {
    print(42);
    print(-7);
    print_int(9);
    print_char('A');
    print_str("hi");
    println_str("!");
}
"#;
    assert_same_output(source, "io_formatting", b"");
}

#[test]
fn read_int_and_char_match() {
    let source = r#"
fn main() {
    let n = read_int();
    print(n);
    let c = read_char();
    print_char(c);
}
"#;
    assert_same_output(source, "io_read_int_char", b"21\nZ");
}

#[test]
fn file_write_then_read_matches() {
    let path = std::env::temp_dir().join("freebird-m1-file-io.txt");
    let source = format!(
        r#"
fn main() {{
    file_write("{path}", "hello 世界\nline2");
    let s = file_read("{path}");
    print_str(s);
    print(s.len());
}}
"#,
        path = path.display()
    );
    assert_same_output(&source, "io_file_roundtrip", b"");
    let _ = std::fs::remove_file(path);
}

#[test]
fn vm_step_limit_is_enforced() {
    let source = "fn main() { loop { let x = 1; } }";
    let mut config = RuntimeConfig::test();
    config.max_steps = 1_000;
    let error = run_source(source, config).expect_err("step limit should trigger");
    assert!(error.contains("step limit"), "unexpected error: {error}");
}

#[test]
fn vm_call_depth_limit_is_enforced() {    let source = r#"
fn recurse(n: i64) -> i64 {
    if n <= 0 { 0 } else { recurse(n - 1) + 1 }
}
fn main() { print(recurse(1000)); }
"#;
    let mut config = RuntimeConfig::test();
    config.max_call_depth = 8;
    let error = run_source(source, config).expect_err("call depth limit should trigger");
    assert!(
        error.contains("call stack") || error.contains("depth"),
        "unexpected error: {error}"
    );
}

fn sd_output(source: &str, name: &str) -> Vec<u8> {
    match run_stardust(source, name, b"") {
        Run::Ok(output) => output,
        Run::Failed(error) => panic!("Stardust failed for {name}: {error}"),
    }
}

fn parse_single_int(output: &[u8]) -> i64 {
    let text = String::from_utf8_lossy(output);
    text.trim()
        .parse()
        .unwrap_or_else(|error| panic!("expected an integer, got {text:?}: {error}"))
}

#[test]
fn stardust_loop_vec_blocks_are_reused() {
    for iterations in [100, 1000] {
        let source = format!(
            r#"
fn main() {{
    let mut i = 0;
    while i < {iterations} {{
        let mut v = Vec::new();
        v.push(i);
        v.push(i + 1);
        i = i + 1;
    }}
    let alloc = stardust! {{ Push(1) Load }};
    print(alloc);
}}
"#
        );
        let alloc = parse_single_int(&sd_output(
            &source,
            &format!("heap_loop_{iterations}"),
        ));
        assert!(
            alloc < 64,
            "{iterations} iterations grew the heap to {alloc} slots"
        );
    }
}

#[test]
fn stardust_vec_growth_is_bounded() {
    let source = r#"
fn main() {
    let mut v = Vec::new();
    let mut i = 0;
    while i < 1000 {
        v.push(i);
        i = i + 1;
    }
    let alloc = stardust! { Push(1) Load };
    print(alloc);
}
"#;
    let alloc = parse_single_int(&sd_output(source, "heap_vec_growth"));
    assert!(alloc < 10_000, "Vec growth allocated {alloc} slots");
}

#[test]
fn vec_alias_push_is_rejected() {
    let source =
        "fn main() { let mut a = vec![1, 2, 3]; let b = a; a.push(4); print(b[0]); }";
    let errors = check_source(source).expect_err("alias push should be rejected");
    assert!(
        errors.join("\n").contains("moved value"),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn vec_move_then_read_matches() {
    assert_same_output(
        "fn main() { let a = vec![1, 2, 3]; let b = a; print(b[0]); }",
        "heap_vec_move_read",
        b"",
    );
}

#[test]
fn vec_return_from_function_matches() {
    let source = r#"
fn make(n: i64) -> Vec<i64> {
    let mut v = Vec::new();
    v.push(n);
    v
}
fn main() {
    let a = make(7);
    print(a[0]);
}
"#;
    assert_same_output(source, "heap_vec_return", b"");
}
