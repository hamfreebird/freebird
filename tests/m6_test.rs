//! M6 正确性与一致性测试。

use std::path::PathBuf;

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
