//! M6 正确性与一致性测试。

use std::path::{Path, PathBuf};
use std::process::Command;

use freebird::{RuntimeConfig, build_stardust, compile_program, run_source};

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

fn create_diag_project(name: &str, main_source: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("m6-diag-{name}-{}", std::process::id()));
    std::fs::create_dir_all(base.join("app/src")).unwrap();
    std::fs::create_dir_all(base.join("util/src")).unwrap();
    std::fs::write(
        base.join("app/freebird.toml"),
        "[package]\nname = \"app\"\nentry = \"src/main.fb\"\n\n[dependencies]\nutil = { path = \"../util\" }\n",
    )
    .unwrap();
    std::fs::write(base.join("app/src/main.fb"), main_source).unwrap();
    std::fs::write(
        base.join("util/freebird.toml"),
        "[package]\nname = \"util\"\nentry = \"src/lib.fb\"\n",
    )
    .unwrap();
    std::fs::write(
        base.join("util/src/lib.fb"),
        "pub fn bad(x: i64) -> i64 {\n    let y: i32 = true;\n    x\n}\n",
    )
    .unwrap();
    base
}

#[test]
fn project_diagnostics_include_file_and_line() {
    let base = create_diag_project(
        "dep",
        "use util::bad;\n\nfn main() {\n    print(bad(1));\n}\n",
    );
    let (diagnostics, _) =
        freebird::check_project(&base.join("app/freebird.toml")).expect("check project");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "T001")
        .expect("expected T001");
    assert_eq!(diagnostic.span.line, 2);
    assert_eq!(diagnostic.file.as_deref(), Some("../util/src/lib.fb"));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn project_entry_diagnostics_use_entry_file() {
    let base = create_diag_project(
        "entry",
        "use util::bad;\n\nfn main() {\n    let z: i32 = true;\n    print(bad(1));\n}\n",
    );
    let (diagnostics, _) =
        freebird::check_project(&base.join("app/freebird.toml")).expect("check project");
    let entry = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.file.as_deref() == Some("src/main.fb"))
        .expect("expected an entry-file diagnostic");
    assert_eq!(entry.span.line, 4);
    let _ = std::fs::remove_dir_all(base);
}

fn run_stardust_text(text: &str) -> Result<Vec<u8>, String> {
    let interpreter = Path::new("stardust/stardust-linux-x86_64");
    if !interpreter.exists() {
        return Err("stardust interpreter not available".to_string());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("freebird-m6-{}-{stamp}.sd", std::process::id()));
    std::fs::write(&path, text).map_err(|error| error.to_string())?;
    let output = Command::new(interpreter)
        .arg(&path)
        .output()
        .map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(&path);
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

#[test]
fn stardust_call_arity_declares_return_values() {
    if !Path::new("stardust/stardust-linux-x86_64").exists() {
        eprintln!("skipping: stardust interpreter not available");
        return;
    }
    let source = r#"fn main() {
        stardust! {
            Push(3)
            Push(5)
            Call(7, 2, 1)
            NumOut
        }
    }"#;
    let stardust = build_stardust(source).expect("build stardust");
    let pad = " ".repeat(7);
    let external = format!("\n{pad}:\n*\n{pad}:\n");
    let output =
        run_stardust_text(&format!("{stardust}{external}")).expect("run combined Stardust");
    assert_eq!(output, b"8\n");

    let error = run_source(source, RuntimeConfig::test()).unwrap_err();
    assert!(error.contains("Call"), "unexpected error: {error}");
}
