//! M4 工具链测试：源码格式化。后续 LSP/DAP/包管理测试也放在这里。

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn freebird() -> Command {
    Command::new(env!("CARGO_BIN_EXE_freebird"))
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("freebird-m4-{}-{name}", std::process::id()))
}

const UNFORMATTED: &str = "fn main() {\nlet x = 1;\nif x > 0 {\nprint(x);\n}\n}\n";
const FORMATTED: &str =
    "fn main() {\n    let x = 1;\n    if x > 0 {\n        print(x);\n    }\n}\n";

#[test]
fn fmt_prints_formatted_source() {
    let path = temp_path("fmt-print.fb");
    std::fs::write(&path, UNFORMATTED).unwrap();
    let output = freebird().arg("fmt").arg(&path).output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), FORMATTED);
    let _ = std::fs::remove_file(path);
}

#[test]
fn fmt_write_updates_file() {
    let path = temp_path("fmt-write.fb");
    std::fs::write(&path, UNFORMATTED).unwrap();
    let output = freebird()
        .args(["fmt", "--write"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), FORMATTED);
    let _ = std::fs::remove_file(path);
}

#[test]
fn fmt_check_reports_unformatted_files() {
    let unformatted = temp_path("fmt-check-bad.fb");
    std::fs::write(&unformatted, UNFORMATTED).unwrap();
    let output = freebird()
        .args(["fmt", "--check"])
        .arg(&unformatted)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not formatted"));

    let formatted = temp_path("fmt-check-good.fb");
    std::fs::write(&formatted, FORMATTED).unwrap();
    let output = freebird()
        .args(["fmt", "--check"])
        .arg(&formatted)
        .output()
        .unwrap();
    assert!(output.status.success());

    let _ = std::fs::remove_file(unformatted);
    let _ = std::fs::remove_file(formatted);
}

#[test]
fn fmt_reads_stdin() {
    let mut child = freebird()
        .args(["fmt", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(UNFORMATTED.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), FORMATTED);
}

// ─── M4-6：诊断 JSON ───

#[test]
fn check_json_has_stable_codes_and_schema() {
    let path = temp_path("diag-type.fb");
    std::fs::write(&path, "fn main() { let x: i32 = true; }\n").unwrap();
    let output = freebird().arg("check").arg(&path).output().unwrap();
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("\"schema_version\": 1"));
    assert!(text.contains("\"code\": \"T001\""));
    assert!(text.contains("\"suggestion\""));
    let _ = std::fs::remove_file(path);
}

#[test]
fn check_json_reports_related_locations() {
    let path = temp_path("diag-dup.fb");
    std::fs::write(&path, "fn main() { let x = 1; let x = 2; }\n").unwrap();
    let output = freebird().arg("check").arg(&path).output().unwrap();
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("\"code\": \"R003\""));
    assert!(text.contains("\"related\""));
    assert!(text.contains("first defined here"));
    let _ = std::fs::remove_file(path);
}

// ─── M4-3：REPL 增强 ───

fn run_repl(input: &str) -> std::process::Output {
    let mut child = freebird()
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn repl_multiline_type_and_history() {
    let output = run_repl(
        "fn add(a: i64, b: i64) -> i64 {\nreturn a + b;\n}\n:type add(1, 2)\n:history\n:q\n",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[declared]"), "stdout: {stdout}");
    assert!(stdout.contains("=> i64"), "stdout: {stdout}");
    assert!(stdout.contains("fn add"), "stdout: {stdout}");
}

#[test]
fn repl_load_and_type() {
    let path = temp_path("repl-load.fb");
    std::fs::write(&path, "fn double(x: i64) -> i64 { x * 2 }\n").unwrap();
    let input = format!(":load {}\n:type double(2)\n:q\n", path.display());
    let output = run_repl(&input);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[declared]"), "stdout: {stdout}");
    assert!(stdout.contains("=> i64"), "stdout: {stdout}");
    let _ = std::fs::remove_file(path);
}

#[test]
fn repl_reports_type_errors_and_recovers() {
    let output = run_repl("let x: i32 = true;\n:type 1 + 1\n:q\n");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("type error"), "stderr: {stderr}");
    assert!(stdout.contains("=> i32"), "stdout: {stdout}");
}
