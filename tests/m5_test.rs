//! M5-2/M5-3 的质量测试：优化一致性，以及后续属性/fuzz 测试的入口。

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use freebird::{
    CompileOptions, MirInst, RuntimeConfig, build_stardust, build_stardust_with_options,
    check_source, compile_program_with_options, run_source,
};

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

// ─── M5-3：固定 seed 属性测试与差分 fuzz ───

/// 固定语料目录：仓库内的 `.fb` 文件即 fuzz corpus，任何时候都可重现。
const CORPUS_DIR: &str = "tests/fuzz/corpus";

/// fuzz 规模倍数：`FREEBIRD_FUZZ_SCALE` 控制用例数与表达式深度。
///
/// 默认 1（CI 主门禁保持快速）；定时 job 用更大值扩展深层递归/长输入。
fn fuzz_scale() -> u64 {
    std::env::var("FREEBIRD_FUZZ_SCALE")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|scale| *scale > 0)
        .unwrap_or(1)
        .min(50)
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn gen_expr(rng: &mut Lcg, depth: u32) -> String {
    if depth == 0 {
        return format!("{}", 1 + rng.below(9));
    }
    match rng.below(4) {
        0 => format!(
            "({} + {})",
            gen_expr(rng, depth - 1),
            gen_expr(rng, depth - 1)
        ),
        1 => format!(
            "({} - {})",
            gen_expr(rng, depth - 1),
            gen_expr(rng, depth - 1)
        ),
        2 => format!(
            "({} * {})",
            gen_expr(rng, depth - 1),
            gen_expr(rng, depth - 1)
        ),
        _ => format!("({})", gen_expr(rng, depth - 1)),
    }
}

fn gen_program(rng: &mut Lcg, depth: u32) -> String {
    let a = gen_expr(rng, depth);
    let b = gen_expr(rng, depth);
    if rng.below(2) == 0 {
        format!(
            "fn main() {{ let a = {a}; let b = {b}; if a > b {{ print(a - b); }} else {{ print(b - a); }} }}"
        )
    } else {
        let limit = 1 + rng.below(8);
        format!(
            "fn main() {{ let mut i = 0; let mut acc = {a}; while i < {limit} {{ acc = acc + {b}; i = i + 1; }} print(acc); }}"
        )
    }
}

fn differential_mismatch(source: &str) -> bool {
    let vm = run_source(source, RuntimeConfig::test());
    let stardust = build_stardust(source).and_then(|text| run_stardust_source(&text));
    match (vm, stardust) {
        (Ok(vm_output), Ok(stardust_output)) => vm_output != stardust_output,
        _ => true,
    }
}

fn archive(kind: &str, source: &str, detail: &str) -> PathBuf {
    let dir = PathBuf::from("target/fuzz-failures");
    let _ = std::fs::create_dir_all(&dir);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("{kind}-{millis}.fb"));
    let _ = std::fs::write(&path, source);
    let _ = std::fs::write(
        path.with_extension("txt"),
        format!("kind: {kind}\n{detail}\n--- source ---\n{source}\n"),
    );
    path
}

/// 按行、再按字符块删除，收缩到仍满足 predicate 的最小输入。
fn minimize<F: Fn(&str) -> bool>(source: &str, predicate: F) -> String {
    let mut current = source.to_string();
    for _ in 0..200 {
        let mut changed = false;

        let lines: Vec<&str> = current.lines().collect();
        if lines.len() > 1 {
            for index in 0..lines.len() {
                let mut candidate_lines = lines.clone();
                candidate_lines.remove(index);
                let candidate = candidate_lines.join("\n");
                if predicate(&candidate) {
                    current = candidate;
                    changed = true;
                    break;
                }
            }
        }
        if changed {
            continue;
        }

        let chars: Vec<char> = current.chars().collect();
        'chunks: for size in [16usize, 8, 4, 2, 1] {
            if chars.len() < size {
                continue;
            }
            let mut index = 0;
            while index + size <= chars.len() {
                let mut candidate: String = chars[..index].iter().collect();
                candidate.extend(chars[index + size..].iter());
                if predicate(&candidate) {
                    current = candidate;
                    changed = true;
                    break 'chunks;
                }
                index += size;
            }
        }

        if !changed {
            break;
        }
    }
    current
}

#[test]
fn random_programs_match_across_backends() {
    if !PathBuf::from("stardust/stardust-linux-x86_64").exists() {
        eprintln!("skipping: stardust interpreter not available");
        return;
    }
    let mut rng = Lcg(0x5EED_2026);
    let scale = fuzz_scale();
    let depth = if scale > 1 { 3 } else { 2 };
    let cases = 30 * scale;
    for index in 0..cases {
        let source = gen_program(&mut rng, depth);
        if differential_mismatch(&source) {
            let minimal = minimize(&source, differential_mismatch);
            let path = archive("differential", &minimal, &format!("generated case {index}"));
            panic!(
                "VM/Stardust mismatch on generated case {index}; archived at {}",
                path.display()
            );
        }
    }
}

#[test]
fn inlining_constant_leaf_functions_removes_calls() {
    let source = "fn zero() -> i64 { 0 }\nfn answer() -> i64 { 42 }\nfn main() { print(zero() + answer()); }";
    let count_calls = |program: &freebird::CompiledProgram| {
        program
            .functions
            .iter()
            .map(|(_, mir)| {
                mir.iter()
                    .filter(|inst| matches!(inst, MirInst::Call { .. }))
                    .count()
            })
            .sum::<usize>()
    };
    let optimized = compile_program_with_options(source, CompileOptions { optimize: true })
        .expect("optimized compile");
    let unoptimized = compile_program_with_options(source, CompileOptions { optimize: false })
        .expect("unoptimized compile");
    assert!(
        count_calls(&unoptimized) >= 2,
        "unoptimized program should still call the helpers"
    );
    assert_eq!(
        count_calls(&optimized),
        0,
        "zero-arg constant leaf calls should be inlined"
    );

    // 内联是语义保持的：VM 与 Stardust 输出一致。
    let vm = run_source(source, RuntimeConfig::test()).expect("vm run");
    let stardust = build_stardust(source).expect("stardust build");
    assert_eq!(
        run_stardust_source(&stardust).expect("stardust run"),
        vm,
        "inlining must not change output"
    );
}

#[test]
fn fuzz_corpus_matches_across_backends() {
    if !PathBuf::from("stardust/stardust-linux-x86_64").exists() {
        eprintln!("skipping: stardust interpreter not available");
        return;
    }
    let dir = PathBuf::from(CORPUS_DIR);
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read {CORPUS_DIR}: {error}"))
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "fb"))
        .collect();
    entries.sort();
    assert!(
        !entries.is_empty(),
        "{CORPUS_DIR} should contain .fb corpus files"
    );

    for path in entries {
        let source = std::fs::read_to_string(&path).expect("read corpus file");
        if differential_mismatch(&source) {
            let minimal = minimize(&source, differential_mismatch);
            let archived = archive(
                "corpus",
                &minimal,
                &format!("corpus file {}", path.display()),
            );
            panic!(
                "VM/Stardust mismatch on {}{}archived at {}",
                path.display(),
                "\n",
                archived.display()
            );
        }
    }
}

#[test]
fn malformed_inputs_do_not_panic() {
    let tokens = [
        "fn",
        "main",
        "(",
        ")",
        "{",
        "}",
        "let",
        "x",
        "=",
        "1",
        "+",
        "-",
        "*",
        "/",
        ";",
        "\"",
        "'",
        "/*",
        "*/",
        "stardust!",
        "match",
        "if",
        "else",
        "while",
        "for",
        "in",
        "use",
        "mod",
        "impl",
        "struct",
        "enum",
        "[",
        "]",
        "<",
        ">",
        "::",
        ",",
        "#",
        "@",
        "\n",
    ];
    let mut rng = Lcg(0xBAD_2026);
    let cases = 60 * fuzz_scale();
    for index in 0..cases {
        let length = 1 + rng.below(18) as usize;
        let mut source = String::new();
        for _ in 0..length {
            source.push_str(tokens[rng.below(tokens.len() as u64) as usize]);
            source.push(' ');
        }
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = check_source(&source);
        }))
        .is_err();
        if panicked {
            let minimal = minimize(&source, |candidate| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = check_source(candidate);
                }))
                .is_err()
            });
            let path = archive("panic", &minimal, &format!("generated case {index}"));
            panic!(
                "checker panicked on generated input {index}; archived at {}",
                path.display()
            );
        }
    }
}

#[test]
fn minimize_shrinks_input() {
    let source = "line one\nline two\nline three needle line four\n";
    let minimal = minimize(source, |candidate| candidate.contains("needle"));
    assert!(minimal.contains("needle"), "minimal: {minimal:?}");
    assert!(minimal.len() < source.len(), "minimal: {minimal:?}");
}
