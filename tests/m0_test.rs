//! M0 语义收敛验收测试。
//!
//! 覆盖前端已经接受、但此前会静默成功或产生残留值的语法：
//! 每项都必须给出明确诊断，正向能力则要求 VM 与 Stardust 输出一致。

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use freebird::{build_stardust, check_source, run_source, RuntimeConfig};

fn run(source: &str) -> Result<Vec<u8>, String> {
    run_source(source, RuntimeConfig::test())
}

/// 同时验证解释执行与 `check` 都会拒绝该程序，并包含指定诊断片段。
fn assert_rejected(name: &str, source: &str, needle: &str) {
    let run_error = run(source).expect_err(&format!("{name}: run should fail"));
    assert!(
        run_error.contains(needle),
        "{name}: run error did not contain {needle:?}:\n{run_error}"
    );

    let check_errors = check_source(source).expect_err(&format!("{name}: check should fail"));
    let check_text = check_errors.join("\n");
    assert!(
        check_text.contains(needle),
        "{name}: check error did not contain {needle:?}:\n{check_text}"
    );
}

/// 复用集成测试的 Stardust 执行方式：生成 `.sd` 后用仓库自带解释器运行。
fn run_stardust(source: &str, name: &str) -> Vec<u8> {
    let stardust = build_stardust(source).expect("Stardust build should succeed");
    let dir = PathBuf::from("target/stardust-tests");
    fs::create_dir_all(&dir).expect("create stardust test dir");
    let path = dir.join(format!("m0_{name}.sd"));
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
        "Stardust interpreter failed for {name}: status={:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn assert_backends_agree(source: &str, name: &str, expected: &[u8]) {
    let vm = run(source).expect("VM run should succeed");
    assert_eq!(vm, expected, "VM output mismatch for {name}");
    let stardust = run_stardust(source, name);
    assert_eq!(
        stardust, vm,
        "Stardust output differs from VM for {name}: VM={:?}, Stardust={:?}",
        String::from_utf8_lossy(&vm),
        String::from_utf8_lossy(&stardust)
    );
}

// ─── M0-1：ret 只接受整数立即数 ───

#[test]
fn ret_rejects_variable_operand() {
    assert_rejected(
        "ret variable",
        "fn main() { let a = 1; ret a; }",
        "`ret` expects an integer literal",
    );
}

#[test]
fn ret_rejects_expression_operand() {
    assert_rejected(
        "ret expression",
        "fn main() { ret 1 + 2; }",
        "`ret` expects an integer literal",
    );
}

#[test]
fn ret_accepts_integer_operand() {
    let source = "fn f() -> i32 { let x = 7; ret 1; } fn main() { print(f()); }";
    let output = run(source).expect("ret 1 should run");
    assert_eq!(output, b"7\n");
    assert_eq!(run_stardust(source, "ret_integer"), b"7\n");
}

// ─── M0-2：循环外的 break/continue ───

#[test]
fn break_outside_loop_is_rejected() {
    assert_rejected(
        "break outside loop",
        "fn main() { break; }",
        "`break` can only be used inside a loop",
    );
}

#[test]
fn continue_outside_loop_is_rejected() {
    assert_rejected(
        "continue outside loop",
        "fn main() { continue; }",
        "`continue` can only be used inside a loop",
    );
}

// ─── M0-3：break value 与 loop 值语义 ───

#[test]
fn break_with_value_is_rejected() {
    assert_rejected(
        "break value",
        "fn main() { loop { break 42; } }",
        "`break value` is not supported yet",
    );
}

#[test]
fn loop_used_as_value_is_rejected() {
    assert_rejected(
        "loop value",
        "fn main() { let v = loop { break; }; print(v); }",
        "`loop` cannot be used as a value yet",
    );
}

#[test]
fn unit_loop_statement_still_runs() {
    let source = "
        fn main() {
            let mut i = 0;
            loop { i = i + 1; if i == 3 { break; } }
            print(i);
        }
    ";
    assert_backends_agree(source, "unit_loop", b"3\n");
}

// ─── M0-4/M0-5：完整被匹配表达式与模式绑定 ───

#[test]
fn match_full_scrutinee_expression_is_supported() {
    let source = "
        struct S { x: i64 }
        fn f(x: i64) -> i64 { return x + 1; }
        fn main() {
            let y = 2;
            match f(y) { 3 => print(30), _ => print(0), }
            match 1 + 1 { 2 => print(20), _ => print(0), }
            match (S { x: 7 }) { S { x } => print(x), }
        }
    ";
    assert_backends_agree(source, "match_scrutinee", b"30\n20\n7\n");
}

#[test]
fn match_struct_enum_tuple_or_and_ref_patterns() {
    let source = "
        struct P { x: i64, y: i64 }
        enum Inner { One, Two(i64) }
        enum Outer { Wrap(Inner), Nil }
        fn main() {
            let p = P { x: 5, y: 6 };
            match &p { &P { x, y } => { print(x); print(y); } }
            match ((1, 2), 3) { ((a, b), c) => { print(a); print(b); print(c); } }
            match (1, 2) { (1, y) => print(y), _ => print(0), }
            let i = Inner::Two(42);
            match Outer::Wrap(i) {
                Outer::Wrap(Inner::Two(v)) => print(v),
                Outer::Wrap(_) => print(1),
                Outer::Nil => print(2),
            }
            let o = Outer::Wrap(Inner::One);
            match o {
                Outer::Wrap(Inner::One) | Outer::Nil => print(77),
                Outer::Wrap(_) => print(0),
            }
            match (P { x: 5, y: 6 }) { P { x: 5, .. } => print(55), _ => print(0), }
        }
    ";
    assert_backends_agree(source, "match_patterns", b"5\n6\n1\n2\n3\n2\n42\n77\n55\n");
}

#[test]
fn struct_pattern_let_destructuring_is_supported() {
    let source = "
        struct P { x: i64, y: i64 }
        fn main() {
            let p = P { x: 3, y: 4 };
            let P { x, y } = p;
            print(x); print(y);
        }
    ";
    assert_backends_agree(source, "let_struct_pattern", b"3\n4\n");
}

#[test]
fn non_exhaustive_match_is_rejected() {
    assert_rejected(
        "non-exhaustive match",
        "enum E { A, B } fn main() { let e = E::A; match e { E::A => print(1), } }",
        "non-exhaustive `match` on `E`",
    );
}

#[test]
fn guarded_arm_does_not_make_match_exhaustive() {
    assert_rejected(
        "guarded match",
        "enum E { A, B } fn main() { let e = E::A; match e { E::A if true => print(1), E::B => print(2), } }",
        "non-exhaustive `match` on `E`",
    );
}

#[test]
fn refutable_let_pattern_is_rejected() {
    assert_rejected(
        "refutable let",
        "enum E { A(i64), B } fn main() { let e = E::A(1); let E::A(v) = e; print(v); }",
        "refutable pattern in `let`",
    );
}

// ─── M0-6：空体函数与 extern ───

#[test]
fn function_without_body_is_rejected() {
    assert_rejected(
        "missing body",
        "fn ext(a: i32) -> i32; fn main() { }",
        "has no body; only extern functions may omit it",
    );
}

#[test]
fn extern_call_is_rejected() {
    assert_rejected(
        "extern call",
        "extern \"stardust\" { fn ext(a: i32) -> i32; } fn main() { print(ext(1)); }",
        "calling extern function `ext`",
    );
}

// ─── M0-7：属性 ───

#[test]
fn documented_attributes_are_accepted() {
    let source = "
        #[stardust(id = 1)]
        fn f(a: i32) -> i32 { return a; }
        #[max_steps=100000]
        fn main() { print(f(5)); }
    ";
    let output = run(source).expect("attributes should be accepted");
    assert_eq!(output, b"5\n");
    assert!(build_stardust(source).is_ok());
}

#[test]
fn runtime_attributes_override_config() {
    let source = "
        #[max_steps=20]
        fn main() {
            let mut i = 0;
            loop { i = i + 1; if i > 100 { break; } }
            print(i);
        }
    ";
    let error = run(source).expect_err("step limit should stop the VM");
    assert!(error.contains("step limit exceeded"), "unexpected error: {error}");
}

#[test]
fn unknown_attribute_is_rejected() {
    assert_rejected("unknown attribute", "#[bogus] fn main() { }", "unknown attribute `#[bogus]`");
}

#[test]
fn invalid_attribute_value_is_rejected() {
    assert_rejected(
        "invalid attribute",
        "#[stardust(id = \"x\")] fn main() { }",
        "invalid attribute `#[stardust]`",
    );
}

// ─── M0-8：#id 与 use ───

#[test]
fn id_expression_is_explicitly_unsupported() {
    assert_rejected(
        "id expression",
        "fn main() { let x = 1; let y = #x; print(y); }",
        "unsupported feature: `#id`",
    );
}

#[test]
fn use_declaration_is_explicitly_unsupported() {
    assert_rejected(
        "use declaration",
        "use foo::bar; fn main() { }",
        "unsupported feature: `use` declarations",
    );
}

// ─── M0-9：check 与 run 使用同一套检查 ───

#[test]
fn check_source_reports_borrow_errors() {
    let source = r#"fn main() { let s = "hi"; let t = s; print_str(s); }"#;
    let errors = check_source(source).expect_err("use-after-move should fail check");
    assert!(
        errors.join("\n").contains("use of moved value `s`"),
        "expected borrow diagnostic, got: {errors:?}"
    );
}

#[test]
fn check_cli_reports_borrow_errors() {
    let dir = PathBuf::from("target/m0-tests");
    fs::create_dir_all(&dir).expect("create m0 test dir");
    let path = dir.join("use_after_move.fb");
    fs::write(&path, r#"fn main() { let s = "hi"; let t = s; print_str(s); }"#)
        .expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_freebird"))
        .arg("check")
        .arg(&path)
        .output()
        .expect("run freebird check");
    assert!(!output.status.success(), "check should exit non-zero");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("use of moved value `s`"),
        "JSON diagnostics should mention the borrow error:\n{stdout}"
    );

    let run_output = Command::new(env!("CARGO_BIN_EXE_freebird"))
        .arg("run")
        .arg(&path)
        .output()
        .expect("run freebird run");
    assert!(!run_output.status.success(), "run should exit non-zero");
}

// ─── 模式的显式拒绝路径 ───

#[test]
fn unknown_enum_variant_in_pattern_is_rejected() {
    assert_rejected(
        "unknown pattern variant",
        "enum E { A, B(i64) } fn main() { let e = E::A; match e { E::Z => print(1), _ => print(0), } }",
        "enum `E` has no variant `Z`",
    );
}

#[test]
fn unknown_struct_field_in_pattern_is_rejected() {
    assert_rejected(
        "unknown pattern field",
        "struct P { x: i64 } fn main() { let p = P { x: 1 }; match p { P { z, .. } => print(z), } }",
        "type `P` has no field `z`",
    );
}

#[test]
fn or_pattern_binding_mismatch_is_rejected() {
    assert_rejected(
        "or binding mismatch",
        "enum E { A(i64), B(i64) } fn main() { let e = E::A(1); match e { E::A(x) | E::B(y) => print(x), } }",
        "`or` pattern alternatives bind different variables",
    );
}

#[test]
fn documented_extern_attributes_parse() {
    let source = r#"
        extern "stardust" {
            #[stardust(id = 1)]
            fn sd_add(a: i32, b: i32) -> i32;
        }
        fn main() { }
    "#;
    check_source(source).expect("extern declaration attributes should parse and check");
    build_stardust(source).expect("extern declarations should not block code generation");
}
