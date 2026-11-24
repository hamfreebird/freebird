//! M5-2/M5-3 的质量测试：优化一致性，以及后续属性/fuzz 测试的入口。

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use freebird::{CompileOptions, build_stardust_with_options, compile_program_with_options};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const PROGRAMS: &[&str] = &[
    "fn main() { print(1 + 2 * 3); }",
    r#"fn fib(n: i64) -> i64 { if n <= 1 { return n; } return fib(n - 1) + fib(n - 2); }
       fn main() { print(fib(10)); }"#,
    r#"struct Point { x: i64, y: i64 }
       fn main() { let p = Point { x: 3, y: 4 }; print(p.x + p.y); }"#,
    r#"enum E { A(i64), B }
       fn main() { let e = E::A(5); match e { E::A(v) => print(v), E::B => print(0) } }"#,
    r#"fn main() { let mut i = 0; let mut s = 0; while i < 10 { s = s + i; i = i + 1; } print(s); }"#,
    r#"fn main() { let mut v = vec![1, 2]; v.push(3); print(v.len()); }"#,
];

fn run_stardust_source(source: &str) -> Result<Vec<u8>, String> {
    let interpreter = PathBuf::from("stardust/stardust-linux-x86_64");
    if !interpreter.exists() {
        return Err("stardust interpreter not available".to_string());
    }
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("freebird-m5-{}-{id}.sd", std::process::id()));
    std::fs::write(&path, source).map_err(|error| error.to_string())?;
    let output = Command::new(&interpreter)
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

fn main_mir_len(program: &freebird::CompiledProgram) -> usize {
    program
        .entry
        .and_then(|entry| {
            program
                .functions
                .iter()
                .find(|(id, _)| *id == entry)
                .map(|(_, mir)| mir.len())
        })
        .unwrap_or(0)
}

#[test]
fn optimized_and_unoptimized_outputs_match() {
    if !PathBuf::from("stardust/stardust-linux-x86_64").exists() {
        eprintln!("skipping: stardust interpreter not available");
        return;
    }
    for (index, source) in PROGRAMS.iter().enumerate() {
        let optimized = build_stardust_with_options(source, CompileOptions { optimize: true })
            .expect("optimized build");
        let unoptimized = build_stardust_with_options(source, CompileOptions { optimize: false })
            .expect("unoptimized build");
        let optimized_output =
            run_stardust_source(&optimized).unwrap_or_else(|e| panic!("program {index}: {e}"));
        let unoptimized_output =
            run_stardust_source(&unoptimized).unwrap_or_else(|e| panic!("program {index}: {e}"));
        assert_eq!(
            optimized_output, unoptimized_output,
            "program {index}: optimized and unoptimized outputs differ"
        );
    }
}

#[test]
fn optimization_reduces_instruction_count() {
    let source = "fn main() { let x = 1 + 2 * 3; print(x); }";
    let optimized = compile_program_with_options(source, CompileOptions { optimize: true })
        .expect("optimized compile");
    let unoptimized = compile_program_with_options(source, CompileOptions { optimize: false })
        .expect("unoptimized compile");
    let optimized_len = main_mir_len(&optimized);
    let unoptimized_len = main_mir_len(&unoptimized);
    assert!(
        optimized_len < unoptimized_len,
        "expected optimization to reduce MIR: optimized={optimized_len}, unoptimized={unoptimized_len}"
    );
}
