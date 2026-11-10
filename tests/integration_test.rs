//! 集成测试 — 测试完整的 .fb 程序编译+执行

use freebird::{build_stardust, run_source, RuntimeConfig};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn run_file(path: &str) -> Result<Vec<u8>, String> {
    let source = fs::read_to_string(path).map_err(|e| e.to_string())?;
    run_source(&source, RuntimeConfig::test())
}

/// 编译源码为 Stardust、写出临时 `.sd`，并用仓库自带的解释器执行。
fn run_stardust_source(source: &str, name: &str) -> String {
    let stardust = build_stardust(source).expect("Stardust build should succeed");

    let dir = PathBuf::from("target/stardust-tests");
    fs::create_dir_all(&dir).expect("create stardust test dir");
    let path = dir.join(format!("{name}.sd"));
    fs::write(&path, stardust).expect("write .sd file");

    let interpreter = "stardust/stardust-linux-x86_64";
    assert!(
        PathBuf::from(interpreter).exists(),
        "bundled Stardust interpreter is missing: {interpreter}"
    );
    let output = Command::new(interpreter)
        .arg(&path)
        .output()
        .expect("failed to launch Stardust interpreter");

    assert!(
        output.status.success(),
        "Stardust interpreter failed for {name}: status={:?}\nstderr: {}\nprogram:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&path).unwrap_or_default()
    );
    String::from_utf8(output.stdout).expect("interpreter stdout should be UTF-8")
}

/// 同 `run_stardust_source`，但向解释器 stdin 写入 `input`。
fn run_stardust_source_with_input(source: &str, name: &str, input: &[u8]) -> String {
    let stardust = build_stardust(source).expect("Stardust build should succeed");

    let dir = PathBuf::from("target/stardust-tests");
    fs::create_dir_all(&dir).expect("create stardust test dir");
    let path = dir.join(format!("{name}.sd"));
    fs::write(&path, stardust).expect("write .sd file");

    let interpreter = "stardust/stardust-linux-x86_64";
    assert!(
        PathBuf::from(interpreter).exists(),
        "bundled Stardust interpreter is missing: {interpreter}"
    );
    let mut child = Command::new(interpreter)
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to launch Stardust interpreter");
    {
        let mut stdin = child.stdin.take().expect("child stdin");
        stdin.write_all(input).expect("write interpreter stdin");
    } // 关闭 stdin，发送 EOF。
    let output = child.wait_with_output().expect("wait for Stardust interpreter");

    assert!(
        output.status.success(),
        "Stardust interpreter failed for {name}: status={:?}\nstderr: {}\nprogram:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        fs::read_to_string(&path).unwrap_or_default()
    );
    String::from_utf8(output.stdout).expect("interpreter stdout should be UTF-8")
}

/// 断言 Stardust 后端输出与 VM 完全一致，避免只验证“能跑”。
fn assert_stardust_matches_vm(source: &str, name: &str, input: &[u8]) {
    let mut config = RuntimeConfig::test();
    config.input = input.to_vec();
    let vm_output = run_source(source, config)
        .unwrap_or_else(|e| panic!("VM failed for {name}: {e}"));
    let sd_output = run_stardust_source_with_input(source, name, input);
    assert_eq!(
        sd_output.as_bytes(),
        vm_output.as_slice(),
        "Stardust output differs from VM for {name}\nVM: {:?}\nSD: {:?}",
        String::from_utf8_lossy(&vm_output),
        sd_output
    );
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

#[test]
fn test_stardust_backend_variables_and_control_flow() {
    let source = r#"
fn main() {
    let mut i = 0;
    let mut sum = 0;
    while i < 5 { sum = sum + i; i = i + 1; }
    print(sum);
    let x = 3;
    if x > 2 { print(1); } else { print(2); }
    let y = if x > 5 { 10 } else { 20 };
    print(y);
}
"#;
    assert_eq!(
        run_stardust_source(source, "variables_control_flow"),
        "10\n1\n20\n"
    );
}

#[test]
fn test_stardust_backend_function_calls() {
    let source = r#"
fn add(a: i64, b: i64) -> i64 { return a + b; }
fn double(x: i64) -> i64 { x * 2 }
fn main() {
    print(add(1, 2));
    print(double(add(3, 4)));
}
"#;
    assert_eq!(run_stardust_source(source, "function_calls"), "3\n14\n");
}

#[test]
fn test_stardust_backend_recursion() {
    // 直接自递归会被旧版解释器误解析；后端用互递归别名规避。
    let source = r#"
fn fact(n: i64) -> i64 {
    if n <= 1 { return 1; }
    return n * fact(n - 1);
}
fn main() { print(fact(5)); }
"#;
    assert_eq!(run_stardust_source(source, "recursion"), "120\n");
}

#[test]
fn test_stardust_backend_string_and_char_io() {
    let source = r#"
fn main() {
    print_str("hello");
    print_char(' ');
    print_char('!');
    println_str(" done");
}
"#;
    assert_eq!(
        run_stardust_source(source, "string_char_io"),
        "hello ! done\n"
    );
}


#[test]
fn test_stardust_backend_nested_loops() {
    let source = r#"
fn main() {
    let mut i = 0;
    let mut total = 0;
    while i < 3 {
        let mut j = 0;
        while j < 3 { total = total + 1; j = j + 1; }
        i = i + 1;
    }
    print(total);
}
"#;
    assert_eq!(run_stardust_source(source, "nested_loops"), "9\n");
}

#[test]
fn test_stardust_backend_struct_and_enum_values() {
    let source = r#"
struct Point { x: i64, y: i64 }
enum Maybe { Nothing, Just(i64) }
fn main() {
    let mut p = Point { x: 3, y: 4 };
    print(p.x + p.y);
    p.x = 10;
    print(p.x);
    print(p.y);
    let o = Maybe::Just(42);
    match o { Maybe::Just(v) => print(v), Maybe::Nothing => print(0), }
    let n = Maybe::Nothing;
    match n { Maybe::Just(v) => print(v), Maybe::Nothing => print(99), }
}
"#;
    assert_stardust_matches_vm(source, "struct_enum_values", b"");
}

#[test]
fn test_stardust_backend_vec_box_and_array_repeat() {
    let source = r#"
fn main() {
    let mut v = vec![1, 2, 3];
    print(v.len());
    v.push(4);
    print(v.len());
    print(v[3]);
    print(v.pop());
    print(v.len());
    v[1] = 20;
    print(v[0]);
    print(v[1]);
    print(v[2]);
    let b = Box::new(7 + 35);
    print(*b);
    let a = [7; 4];
    print(a.len());
    print(a[0] + a[3]);
}
"#;
    assert_stardust_matches_vm(source, "vec_box_repeat", b"");
}

#[test]
fn test_stardust_backend_copy_on_write_values() {
    let source = r#"
struct P { x: i64, y: i64 }
fn main() {
    let a = P { x: 1, y: 2 };
    let mut b = a;
    b.y = 9;
    print(a.y);
    print(b.y);
    let mut v = vec![5, 6, 7];
    let mut u = v;
    u[1] = 60;
    print(u[0]);
    print(u[1]);
    print(u[2]);
    print(u.pop());
    print(u.len());
}
"#;
    assert_stardust_matches_vm(source, "copy_on_write", b"");
}

#[test]
fn test_stardust_backend_string_parameters_and_read_line() {
    let source = r#"
fn show(s: String) { print_str(s); }
fn length(s: String) -> i64 { return s.len(); }
fn main() {
    print(length("hello"));
    show("dynamic");
    print_char('\n');
    let line = read_line();
    print_str(line);
    print_char('\n');
}
"#;
    assert_stardust_matches_vm(source, "string_parameters_readline", b"piped line\n");
}

#[test]
fn test_stardust_backend_struct_across_recursion() {
    let source = r#"
struct P { x: i64, y: i64 }
fn mk(n: i64) -> P { return P { x: n, y: n + 1 }; }
fn f(n: i64) -> i64 {
    if n <= 0 { return 0; }
    let p = mk(n);
    return p.x + p.y + f(n - 1);
}
fn main() { print(f(5)); }
"#;
    assert_stardust_matches_vm(source, "struct_recursion", b"");
}

#[test]
fn test_stardust_backend_read_int_and_char() {
    let source = r#"
fn main() {
    let n = read_int();
    print(n * 2);
    let c = read_char();
    print_char(c);
}
"#;
    assert_stardust_matches_vm(source, "read_int_char", b"21\nZ");
}

#[test]
fn test_stardust_backend_read_line_utf8() {
    let source = r#"
fn main() {
    let line = read_line();
    print(line.len());
    print_str(line);
    print_char('\n');
}
"#;
    assert_stardust_matches_vm(source, "read_line_utf8", "héllo 世界 🎉\n".as_bytes());
}

#[test]
fn test_stardust_backend_read_line_multiple_utf8_lines() {
    let source = r#"
fn main() {
    let a = read_line();
    let b = read_line();
    print_str(a);
    print_char('\n');
    print_str(b);
    print_char('\n');
}
"#;
    assert_stardust_matches_vm(source, "read_line_multiple_utf8", "one\ntwø 世界\n".as_bytes());
}

#[test]
fn test_stardust_backend_mutable_reference_write_back() {
    let source = r#"
fn main() {
    let mut x = 1;
    let p = &mut x;
    *p = 5;
    *p = *p + 2;
    print(*p);
    print(x);
}
"#;
    // NLL 在最后一次使用 p 之后释放借用，因此 `print(x)` 之前必须已经写回。
    assert_stardust_matches_vm(source, "mutable_ref_write_back", b"");
}

#[test]
fn test_stardust_backend_reference_passed_to_function() {
    let source = r#"
fn inc(p: &mut i64) { *p = *p + 1; }
fn twice(p: &mut i64) { inc(p); inc(p); }
fn set(p: &mut i64) { *p = 42; }
fn main() {
    let mut x = 10;
    twice(&mut x);
    print(x);
    let mut y = 1;
    let p = &mut y;
    set(p);
    print(y);
}
"#;
    assert_stardust_matches_vm(source, "reference_passed_to_function", b"");
}

#[test]
fn test_stardust_backend_reference_replaces_struct_value() {
    let source = r#"
struct P { x: i64, y: i64 }
fn main() {
    let mut p = P { x: 1, y: 2 };
    let r = &mut p;
    *r = P { x: 30, y: 40 };
    print(p.x);
    print(p.y);
}
"#;
    assert_stardust_matches_vm(source, "reference_struct_replace", b"");
}
