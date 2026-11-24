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
