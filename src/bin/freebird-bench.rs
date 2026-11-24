//! Freebird 基准套件（M5-1）。
//!
//! 用法：
//!
//! ```bash
//! cargo run --release --bin freebird-bench            # 打印当前结果（JSON）
//! cargo run --release --bin freebird-bench -- --update
//! cargo run --release --bin freebird-bench -- --check --tolerance 2.5
//! ```
//!
//! 结果保存到 `bench/baseline.json`；`--check` 对超过阈值的项返回非零退出码，
//! 供 CI 作为非阻塞告警使用。基准只依赖标准库与 crate 自身，不引入额外依赖。

use std::path::Path;
use std::process;
use std::time::Instant;

use serde_json::{Value, json};

use freebird::RuntimeConfig;
use freebird::codegen::llvm::{compile_llvm_ir, locate_stardust};
use freebird::{build_stardust, run_source};

const FIB: &str = r#"
fn fib(n: i64) -> i64 {
    if n <= 1 { return n; }
    return fib(n - 1) + fib(n - 2);
}
fn main() { print(fib(20)); }
"#;

const LOOP: &str = r#"
fn main() {
    let mut i = 0;
    let mut sum = 0;
    while i < 20000 { sum = sum + i; i = i + 1; }
    print(sum);
}
"#;

const VEC: &str = r#"
fn main() {
    let mut v = vec![0];
    let mut i = 0;
    while i < 5000 { v.push(i); i = i + 1; }
    print(v.len());
}
"#;

#[derive(Debug, Clone)]
struct Measurement {
    name: &'static str,
    unit: &'static str,
    value: f64,
}

fn bench_config() -> RuntimeConfig {
    let mut config = RuntimeConfig::test();
    config.max_steps = 20_000_000;
    config
}

/// 运行三次取中位数（毫秒）。
fn measure_ms(mut run: impl FnMut()) -> f64 {
    let mut samples = Vec::with_capacity(3);
    for _ in 0..3 {
        let start = Instant::now();
        run();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    samples[samples.len() / 2]
}

fn run_program(source: &str, config: &RuntimeConfig) {
    run_source(source, config.clone()).unwrap_or_else(|error| {
        panic!("benchmark program failed: {error}");
    });
}

fn collect(include_llvm: bool) -> Vec<Measurement> {
    let config = bench_config();
    let mut results = Vec::new();

    results.push(Measurement {
        name: "vm_fib_20_ms",
        unit: "ms",
        value: measure_ms(|| run_program(FIB, &config)),
    });
    results.push(Measurement {
        name: "vm_loop_20k_ms",
        unit: "ms",
        value: measure_ms(|| run_program(LOOP, &config)),
    });
    results.push(Measurement {
        name: "vm_vec_5k_ms",
        unit: "ms",
        value: measure_ms(|| run_program(VEC, &config)),
    });

    let stardust = build_stardust(FIB).expect("stardust generation should succeed");
    results.push(Measurement {
        name: "stardust_fib_bytes",
        unit: "bytes",
        value: stardust.len() as f64,
    });
    results.push(Measurement {
        name: "stardust_fib_gen_ms",
        unit: "ms",
        value: measure_ms(|| {
            let _ = build_stardust(FIB).expect("stardust generation should succeed");
        }),
    });

    if include_llvm && locate_stardust(None).is_ok() {
        if let Ok(ir) = compile_llvm_ir(LOOP) {
            results.push(Measurement {
                name: "llvm_loop_ir_bytes",
                unit: "bytes",
                value: ir.len() as f64,
            });
        }
        results.push(Measurement {
            name: "llvm_loop_compile_ms",
            unit: "ms",
            value: measure_ms(|| {
                let _ = compile_llvm_ir(LOOP).expect("llvm generation should succeed");
            }),
        });
    }

    results
}

fn report_json(results: &[Measurement]) -> Value {
    json!({
        "schema_version": 1,
        "results": results.iter().map(|result| json!({
            "name": result.name,
            "unit": result.unit,
            "value": result.value
        })).collect::<Vec<_>>()
    })
}

fn baseline_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("bench")
        .join("baseline.json")
}

/// 比较基线，返回回归描述。
fn compare(baseline: &Value, current: &[Measurement], tolerance: f64) -> Vec<String> {
    let mut regressions = Vec::new();
    let Some(entries) = baseline.get("results").and_then(Value::as_array) else {
        return vec!["baseline has no `results` array".to_string()];
    };
    for entry in entries {
        let Some(name) = entry.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(base_value) = entry.get("value").and_then(Value::as_f64) else {
            continue;
        };
        let Some(current) = current.iter().find(|result| result.name == name) else {
            continue;
        };
        if base_value > 0.0 && current.value > base_value * tolerance {
            regressions.push(format!(
                "{name}: {:.3} {} (baseline {:.3}, tolerance {:.1}x)",
                current.value, current.unit, base_value, tolerance
            ));
        }
    }
    regressions
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut check = false;
    let mut update = false;
    let mut include_llvm = true;
    let mut tolerance = 1.5;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--check" => check = true,
            "--update" => update = true,
            "--no-llvm" => include_llvm = false,
            "--tolerance" => {
                index += 1;
                tolerance = args
                    .get(index)
                    .and_then(|value| value.parse::<f64>().ok())
                    .unwrap_or_else(|| {
                        eprintln!("Error: --tolerance expects a number");
                        process::exit(2);
                    });
            }
            "--help" | "-h" => {
                println!("freebird-bench [--check [--tolerance N]] [--update] [--no-llvm]");
                return;
            }
            other => {
                eprintln!("Error: unknown argument `{other}`");
                process::exit(2);
            }
        }
        index += 1;
    }

    let results = collect(include_llvm);
    let report = report_json(&results);

    if update {
        let path = baseline_path();
        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            eprintln!("Error creating {}: {error}", parent.display());
            process::exit(1);
        }
        if let Err(error) = std::fs::write(
            &path,
            serde_json::to_string_pretty(&report).unwrap_or_default(),
        ) {
            eprintln!("Error writing {}: {error}", path.display());
            process::exit(1);
        }
        eprintln!("baseline updated: {}", path.display());
        return;
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&report).unwrap_or_default()
    );

    if check {
        let path = baseline_path();
        let baseline: Value = match std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| serde_json::from_str(&text).map_err(|error| error.to_string()))
        {
            Ok(value) => value,
            Err(error) => {
                eprintln!("Error reading baseline {}: {error}", path.display());
                process::exit(2);
            }
        };
        let regressions = compare(&baseline, &results, tolerance);
        if regressions.is_empty() {
            eprintln!("benchmark check: OK (tolerance {tolerance}x)");
        } else {
            eprintln!("benchmark regressions:");
            for regression in &regressions {
                eprintln!("  - {regression}");
            }
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measurement(name: &'static str, value: f64) -> Measurement {
        Measurement {
            name,
            unit: "ms",
            value,
        }
    }

    #[test]
    fn compare_detects_regression() {
        let baseline = json!({
            "results": [{ "name": "vm_fib", "unit": "ms", "value": 10.0 }]
        });
        let current = vec![measurement("vm_fib", 12.0)];
        assert!(compare(&baseline, &current, 1.5).is_empty());
        let current = vec![measurement("vm_fib", 16.0)];
        assert_eq!(compare(&baseline, &current, 1.5).len(), 1);
    }

    #[test]
    fn compare_ignores_missing_current_results() {
        let baseline = json!({
            "results": [{ "name": "llvm_compile", "unit": "ms", "value": 1.0 }]
        });
        let current = vec![measurement("vm_fib", 100.0)];
        assert!(compare(&baseline, &current, 1.1).is_empty());
    }
}
