//! M6 正确性与一致性测试。

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Run, assert_same_output, run_stardust, run_vm};
use freebird::codegen::llvm::{locate_clang, locate_stardust};
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
fn stardust_heap_growth_is_bounded_under_cow_pressure() {
    if !Path::new("stardust/stardust-linux-x86_64").exists() {
        eprintln!("skipping: stardust interpreter not available");
        return;
    }
    // 数组写时复制的旧块保守不回收（ADR 0002）：实测约 10 个堆元素/次迭代。
    // 1000 次迭代约 1 万元素，远低于 1M 堆上限；两端输出必须一致。
    let source = r#"fn main() {
    let mut i = 0;
    let mut acc = 0;
    while i < 1000 {
        let mut a = [i, i + 1, i + 2];
        a[0] = acc;
        acc = acc + a[1];
        i = i + 1;
    }
    print(acc);
}"#;
    let vm = run_source(source, RuntimeConfig::test()).expect("vm run");
    let text = build_stardust(source).expect("stardust build");
    let stardust = run_stardust_text(&text).expect("stardust run");
    assert_eq!(stardust, vm, "COW pressure must not change output");
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

#[test]
fn generic_instances_are_independent_on_both_backends() {
    let source = r#"
struct Pair<T> { first: T, second: T }
fn first<T>(p: Pair<T>) -> T { p.first }
fn main() {
    let a: Pair<i64> = Pair { first: 1, second: 2 };
    let b: Pair<String> = Pair { first: "x", second: "y" };
    print(first(a));
    print_str(first(b));
    print_str("\n");
}
"#;
    assert_same_output(source, "generic_instances", b"");
}

#[test]
fn stardust_runtime_limits_are_emitted_and_enforced() {
    let source = "#[max_steps=1000]\nfn main() { loop { } }";
    let sd = freebird::build_stardust(source).expect("build stardust");
    assert!(
        sd.lines().any(|line| line == "#[max_steps=1000]"),
        "missing runtime annotation: {sd}"
    );
    assert!(
        freebird::run_source(source, RuntimeConfig::test()).is_err(),
        "VM must enforce max_steps"
    );
    if !Path::new("stardust/stardust-linux-x86_64").exists() {
        eprintln!("skipping: stardust interpreter not available");
        return;
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("m6-limits-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("limit.sd");
    std::fs::write(&path, &sd).unwrap();
    // timeout 防止“注解未生效”时测试挂起。
    let output = Command::new("timeout")
        .arg("5")
        .arg("stardust/stardust-linux-x86_64")
        .arg(&path)
        .output()
        .unwrap();
    assert_ne!(
        output.status.code(),
        Some(124),
        "stardust did not enforce max_steps within 5s"
    );
    assert!(
        !output.status.success(),
        "expected a non-zero exit for step limit"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn traits_dispatch_statically_on_both_backends() {
    let source = r#"
trait Greet { fn greet(&self) -> i64; }
struct P { x: i64 }
impl Greet for P { fn greet(&self) -> i64 { self.x } }
fn main() {
    let p = P { x: 7 };
    print(p.greet());
    print_str("\n");
}
"#;
    assert_same_output(source, "trait_static", b"");
}

#[test]
fn closures_are_rejected_clearly() {
    let errors = freebird::check_source("fn main() { let f = |x: i64| x + 1; }")
        .expect_err("closures must be rejected");
    let joined = errors.join("\n");
    assert!(
        joined.contains("closures"),
        "unexpected diagnostics: {joined}"
    );
}

#[test]
fn const_evaluation_runs_on_both_backends() {
    let source = r#"
const BASE: i64 = 40;
const ANSWER: i64 = BASE + 2;
const FLAG: bool = ANSWER > 10 && !false;
fn main() {
    print(ANSWER);
    print_char(if FLAG { 'y' } else { 'n' });
    print_str("\n");
}
"#;
    assert_same_output(source, "const_eval", b"");
}

#[test]
fn const_rejects_division_by_zero() {
    let errors = freebird::check_source("const BAD: i64 = 1 / 0;\nfn main() {}")
        .expect_err("division by zero must be rejected");
    let joined = errors.join("\n");
    assert!(
        joined.contains("division by zero"),
        "unexpected diagnostics: {joined}"
    );
}

#[test]
fn const_rejects_forward_reference() {
    let errors = freebird::check_source("const A: i64 = B;\nconst B: i64 = 1;\nfn main() {}")
        .expect_err("forward reference must be rejected");
    let joined = errors.join("\n");
    assert!(
        joined.contains("previously defined const"),
        "unexpected diagnostics: {joined}"
    );
}

#[test]
fn generic_bounds_run_on_both_backends() {
    let source = r#"
fn pick<T: Ord>(a: T, b: T) -> T { a }
struct Show<T: Display> { v: T }
fn main() {
    print(pick(3, 4));
    let s: String = pick("a", "b");
    print_str(s);
    let sh: Show<i64> = Show { v: 9 };
    print(sh.v);
    print_str("\n");
}
"#;
    assert_same_output(source, "generic_bounds", b"");
}

#[test]
fn generic_bound_violation_is_rejected() {
    let source = r#"
struct Point { x: i64 }
struct Show<T: Display> { v: T }
fn main() { let s: Show<Point> = Show { v: Point { x: 1 } }; }
"#;
    let errors = freebird::check_source(source).expect_err("bound must be rejected");
    let joined = errors.join("\n");
    assert!(
        joined.contains("does not satisfy"),
        "unexpected diagnostics: {joined}"
    );
}

#[test]
fn format_renders_supported_kinds_on_both_backends() {
    let source = r#"
fn main() {
    let s: String = format("n={} b={} c={} s={}", 42, true, 'Z', "ok");
    print_str(s);
    print_str("\n");
    print_str(format("plain"));
    print_str("\n");
}
"#;
    assert_same_output(source, "format_kinds", b"");
}

#[test]
fn format_handles_escapes_negative_and_variable_templates() {
    let source = r#"
fn main() {
    let t: String = "{{x}} = {} / {}";
    print_str(format(t, -7, "y"));
    print_str("\n");
    let empty: String = "no placeholders";
    print_str(format(empty));
    print_str("\n");
}
"#;
    assert_same_output(source, "format_template", b"");
}

#[test]
fn format_literal_template_mismatch_is_rejected() {
    let errors = freebird::check_source(r#"fn main() { let s: String = format("{} {}", 1); }"#)
        .expect_err("placeholder mismatch must be rejected");
    let joined = errors.join("\n");
    assert!(
        joined.contains("placeholder"),
        "unexpected diagnostics: {joined}"
    );
}

#[test]
fn format_rejects_unsupported_argument_type() {
    let source = r#"
struct P { x: i64 }
fn main() { let s: String = format("{}", P { x: 1 }); }
"#;
    let errors = freebird::check_source(source).expect_err("struct argument must be rejected");
    let joined = errors.join("\n");
    assert!(
        joined.contains("cannot be formatted"),
        "unexpected diagnostics: {joined}"
    );
}

#[test]
fn format_runtime_mismatch_fails_on_both_backends() {
    let source = r#"
fn main() {
    let t: String = "{} {}";
    print_str(format(t, 1));
}
"#;
    assert!(matches!(run_vm(source, b""), Run::Failed(_)));
    assert!(matches!(
        run_stardust(source, "format_runtime_mismatch", b""),
        Run::Failed(_)
    ));
}

#[test]
fn format_works_on_llvm_target() {
    if locate_stardust(None).is_err() || locate_clang().is_err() {
        eprintln!("skipping: LLVM toolchain not available");
        return;
    }
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("m6-format-llvm-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let program = base.join("program.fb");
    let exe = base.join("format_exe");
    std::fs::write(
        &program,
        r#"fn main() {
            print_str(format("v={} b={}", 7, false));
            print_str("\n");
        }"#,
    )
    .unwrap();
    let build = freebird()
        .args(["build", "--target", "llvm", "--emit", "exe"])
        .arg(&program)
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success());
    assert_eq!(run.stdout, b"v=7 b=false\n");
    let _ = std::fs::remove_dir_all(base);
}

fn freebird() -> Command {
    Command::new(env!("CARGO_BIN_EXE_freebird"))
}

#[test]
fn extern_functions_cannot_be_used_as_values() {
    let source = r#"extern "stardust" {
        #[stardust(id = 1)]
        fn ext(x: i64) -> i64;
    }
    fn main() { let f = ext; }"#;
    let errors = freebird::check_source(source).expect_err("extern value must be rejected");
    assert!(
        errors.join("\n").contains("extern function"),
        "unexpected diagnostics: {errors:?}"
    );
}

#[test]
fn link_stardust_appends_external_definitions() {
    if !Path::new("stardust/stardust-linux-x86_64").exists() {
        eprintln!("skipping: stardust interpreter not available");
        return;
    }
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("m6-link-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let program = base.join("program.fb");
    let external = base.join("external.sd");
    let output = base.join("linked.sd");
    std::fs::write(
        &program,
        r#"fn main() {
            stardust! {
                Push(3)
                Push(5)
                Call(7, 2, 1)
                NumOut
            }
        }"#,
    )
    .unwrap();
    let pad = " ".repeat(7);
    std::fs::write(&external, format!("{pad}:\n*\n{pad}:\n")).unwrap();

    let build = freebird()
        .args(["build", "--link-stardust"])
        .arg(&external)
        .arg(&program)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let stardust = std::fs::read_to_string(&output).unwrap();
    let result = run_stardust_text(&stardust).expect("run linked Stardust");
    assert_eq!(result, b"8\n");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn link_stardust_works_for_llvm_executable() {
    if locate_stardust(None).is_err() || locate_clang().is_err() {
        eprintln!("skipping: LLVM toolchain not available");
        return;
    }
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("m6-link-llvm-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let program = base.join("program.fb");
    let external = base.join("external.sd");
    let exe = base.join("linked_exe");
    std::fs::write(
        &program,
        r#"fn main() {
            stardust! {
                Push(3)
                Push(5)
                Call(7, 2, 1)
                NumOut
            }
        }"#,
    )
    .unwrap();
    let pad = " ".repeat(7);
    std::fs::write(&external, format!("{pad}:\n*\n{pad}:\n")).unwrap();

    let build = freebird()
        .args([
            "build",
            "--target",
            "llvm",
            "--emit",
            "exe",
            "--link-stardust",
        ])
        .arg(&external)
        .arg(&program)
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success());
    assert_eq!(run.stdout, b"8\n");
    let _ = std::fs::remove_dir_all(base);
}
