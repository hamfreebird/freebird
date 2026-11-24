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

/// 测试用的独立历史文件（避免污染用户状态目录）。
fn repl_history_file(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    temp_path(&format!("repl-history-{tag}-{n}.txt"))
}

fn run_repl_with_history(history: &std::path::Path, input: &str) -> std::process::Output {
    let mut child = freebird()
        .arg("repl")
        .env("FREEBIRD_REPL_HISTORY", history)
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

fn run_repl(input: &str) -> std::process::Output {
    let history = repl_history_file("auto");
    let output = run_repl_with_history(&history, input);
    let _ = std::fs::remove_file(history);
    output
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

#[test]
fn repl_bindings_persist_across_inputs() {
    // `let` 绑定在后续输入中仍可用（每次输入重放）。
    let output = run_repl("let x = 41;\nx + 1\n:vars\n:q\n");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("42"), "stdout: {stdout}");
    assert!(
        stdout.contains("Session bindings"),
        "`:vars` should list session bindings: {stdout}"
    );
}

#[test]
fn repl_history_persists_across_sessions() {
    let history = repl_history_file("persist");
    let _ = std::fs::remove_file(&history);

    let first = run_repl_with_history(&history, "let a = 1;\n:q\n");
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    let second = run_repl_with_history(&history, ":history\n:q\n");
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.contains("let a = 1"),
        "history should persist across sessions: {stdout}"
    );

    let _ = std::fs::remove_file(history);
}

// ─── M4-1：LSP ───

fn write_lsp(writer: &mut impl Write, value: &serde_json::Value) {
    let body = serde_json::to_vec(value).unwrap();
    write!(writer, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    writer.write_all(&body).unwrap();
    writer.flush().unwrap();
}

fn read_lsp(reader: &mut impl std::io::BufRead) -> serde_json::Value {
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            length = value.trim().parse().unwrap();
        }
    }
    let mut body = vec![0u8; length];
    std::io::Read::read_exact(reader, &mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[test]
fn lsp_stdio_initialize_and_diagnostics() {
    let mut child = freebird()
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());

    write_lsp(
        &mut stdin,
        &serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
    );
    let initialize = read_lsp(&mut stdout);
    assert_eq!(initialize["result"]["capabilities"]["hoverProvider"], true);
    assert_eq!(initialize["result"]["capabilities"]["textDocumentSync"], 1);

    write_lsp(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": {
                "uri": "file:///lsp-test.fb",
                "languageId": "freebird",
                "version": 1,
                "text": "fn main() { let x: i32 = true; }\n"
            }}
        }),
    );
    let published = read_lsp(&mut stdout);
    assert_eq!(published["method"], "textDocument/publishDiagnostics");
    let diagnostics = published["params"]["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics[0]["code"], "T001");

    write_lsp(
        &mut stdin,
        &serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" }),
    );
    let _ = read_lsp(&mut stdout);
    write_lsp(
        &mut stdin,
        &serde_json::json!({ "jsonrpc": "2.0", "method": "exit" }),
    );
    drop(stdin);
    let _ = child.wait();
}

// ─── M4-2：DAP ───

fn dap_request(seq: i64, command: &str, arguments: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "seq": seq,
        "type": "request",
        "command": command,
        "arguments": arguments
    })
}

#[test]
fn dap_stdio_breakpoint_and_continue() {
    let path = temp_path("dap-program.fb");
    std::fs::write(
        &path,
        "fn main() {\n    let a = 1;\n    let b = 2;\n    print(a + b);\n}\n",
    )
    .unwrap();

    let mut child = freebird()
        .arg("dap")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());

    write_lsp(
        &mut stdin,
        &dap_request(1, "initialize", serde_json::json!({})),
    );
    let initialize = read_lsp(&mut stdout);
    assert_eq!(initialize["success"], true);
    let initialized = read_lsp(&mut stdout);
    assert_eq!(initialized["event"], "initialized");

    write_lsp(
        &mut stdin,
        &dap_request(
            2,
            "launch",
            serde_json::json!({ "program": path.to_string_lossy() }),
        ),
    );
    assert_eq!(read_lsp(&mut stdout)["success"], true);

    write_lsp(
        &mut stdin,
        &dap_request(
            3,
            "setBreakpoints",
            serde_json::json!({ "breakpoints": [{ "line": 3 }] }),
        ),
    );
    assert_eq!(read_lsp(&mut stdout)["success"], true);

    write_lsp(
        &mut stdin,
        &dap_request(4, "configurationDone", serde_json::json!({})),
    );
    assert_eq!(read_lsp(&mut stdout)["success"], true);
    let stopped = read_lsp(&mut stdout);
    assert_eq!(stopped["event"], "stopped");

    write_lsp(
        &mut stdin,
        &dap_request(5, "stackTrace", serde_json::json!({ "threadId": 1 })),
    );
    let stack = read_lsp(&mut stdout);
    assert_eq!(stack["body"]["stackFrames"][0]["name"], "main");
    assert_eq!(stack["body"]["stackFrames"][0]["line"], 3);

    write_lsp(
        &mut stdin,
        &dap_request(6, "continue", serde_json::json!({ "threadId": 1 })),
    );
    let continued = read_lsp(&mut stdout);
    assert_eq!(continued["success"], true);
    // continue 之后可能先收到 stdout output 事件，再收到 exited/terminated。
    let mut saw_exited = false;
    for _ in 0..3 {
        let message = read_lsp(&mut stdout);
        if message["event"] == "exited" {
            saw_exited = true;
            break;
        }
    }
    assert!(saw_exited, "expected an exited event");

    write_lsp(
        &mut stdin,
        &dap_request(7, "disconnect", serde_json::json!({})),
    );
    drop(stdin);
    let _ = child.wait();
    let _ = std::fs::remove_file(path);
}

// ─── M4-5：freebird.toml 项目 ───

fn create_project(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("m4-project-{name}-{}", std::process::id()));
    std::fs::create_dir_all(base.join("app/src")).unwrap();
    std::fs::create_dir_all(base.join("util/src")).unwrap();
    std::fs::write(
        base.join("app/freebird.toml"),
        "[package]\nname = \"app\"\nentry = \"src/main.fb\"\n\n[dependencies]\nutil = { path = \"../util\" }\n",
    )
    .unwrap();
    std::fs::write(
        base.join("app/src/main.fb"),
        "use util::double;\n\nfn main() {\n    print(double(21));\n}\n",
    )
    .unwrap();
    std::fs::write(
        base.join("util/freebird.toml"),
        "[package]\nname = \"util\"\nentry = \"src/lib.fb\"\n",
    )
    .unwrap();
    std::fs::write(
        base.join("util/src/lib.fb"),
        "pub fn double(x: i64) -> i64 {\n    x * 2\n}\n",
    )
    .unwrap();
    base
}

#[test]
fn project_run_check_and_build() {
    let base = create_project("main");
    let manifest = base.join("app/freebird.toml");

    let run = freebird()
        .args(["run", "--manifest-path"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "42\n");

    let check = freebird()
        .args(["check", "--manifest-path"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(check.status.success());
    let json = String::from_utf8_lossy(&check.stdout);
    assert!(json.contains("\"error_count\": 0"), "stdout: {json}");

    let build = freebird()
        .args(["build", "--manifest-path"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(String::from_utf8_lossy(&build.stdout).contains("util::double"));

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn project_is_found_from_current_directory() {
    let base = create_project("cwd");
    let run = freebird()
        .current_dir(base.join("app"))
        .arg("run")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "42\n");
    let _ = std::fs::remove_dir_all(base);
}

// ─── M6-3.3：传递依赖与 freebird.lock ───

/// 三层依赖：app → util → core（core 提供 `triple`，util 提供 `six`）。
fn create_nested_project(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("m6-deps-{name}-{}", std::process::id()));
    for dir in ["app/src", "util/src", "core/src"] {
        std::fs::create_dir_all(base.join(dir)).unwrap();
    }
    std::fs::write(
        base.join("app/freebird.toml"),
        "[package]\nname = \"app\"\nentry = \"src/main.fb\"\n\n[dependencies]\nutil = { path = \"../util\" }\n",
    )
    .unwrap();
    std::fs::write(
        base.join("app/src/main.fb"),
        "use core::triple;\nuse util::six;\n\nfn main() {\n    print(six(3));\n    print(triple(5));\n}\n",
    )
    .unwrap();
    std::fs::write(
        base.join("util/freebird.toml"),
        "[package]\nname = \"util\"\nentry = \"src/lib.fb\"\n\n[dependencies]\ncore = { path = \"../core\" }\n",
    )
    .unwrap();
    std::fs::write(
        base.join("util/src/lib.fb"),
        "use core::triple;\n\npub fn six(x: i64) -> i64 {\n    triple(x) * 2\n}\n",
    )
    .unwrap();
    std::fs::write(
        base.join("core/freebird.toml"),
        "[package]\nname = \"core\"\nentry = \"src/lib.fb\"\n",
    )
    .unwrap();
    std::fs::write(
        base.join("core/src/lib.fb"),
        "pub fn triple(x: i64) -> i64 {\n    x * 3\n}\n",
    )
    .unwrap();
    base
}

#[test]
fn project_resolves_transitive_dependencies() {
    let base = create_nested_project("transitive");
    let manifest = base.join("app/freebird.toml");
    let run = freebird()
        .args(["run", "--manifest-path"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    // 传递依赖 core 也可直接以包名访问
    assert_eq!(String::from_utf8_lossy(&run.stdout), "18\n15\n");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn project_lock_generates_and_verifies_dependencies() {
    let base = create_nested_project("lock");
    let manifest = base.join("app/freebird.toml");
    let lock = base.join("app/freebird.lock");

    let locked = freebird()
        .args(["lock", "--manifest-path"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        locked.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&locked.stderr)
    );
    let text = std::fs::read_to_string(&lock).unwrap();
    assert!(text.contains("name = \"util\""), "lock: {text}");
    assert!(text.contains("name = \"core\""), "lock: {text}");
    assert!(text.contains("path = \"../util\""), "lock: {text}");

    let run = freebird()
        .args(["run", "--manifest-path"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    // 依赖内容变化后锁文件校验失败，并提示重新生成
    std::fs::write(
        base.join("core/src/lib.fb"),
        "pub fn triple(x: i64) -> i64 {\n    x + x + x\n}\n",
    )
    .unwrap();
    let stale = freebird()
        .args(["run", "--manifest-path"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(!stale.status.success(), "stale lock should fail the run");
    let stderr = String::from_utf8_lossy(&stale.stderr);
    assert!(stderr.contains("freebird.lock"), "stderr: {stderr}");
    assert!(stderr.contains("core"), "stderr: {stderr}");

    // 重新生成锁文件后恢复正常
    let relocked = freebird()
        .args(["lock", "--manifest-path"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        relocked.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&relocked.stderr)
    );
    let run = freebird()
        .args(["run", "--manifest-path"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "18\n15\n");

    let _ = std::fs::remove_dir_all(base);
}
