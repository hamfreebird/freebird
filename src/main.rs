use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::process;

use freebird::{build_stardust, run_file, RuntimeConfig};
use freebird::borrowck::check_borrow;
use freebird::codegen::llvm::{compile_llvm, LlvmArtifact, LlvmEmit, LlvmOptions};
use freebird::diagnostics;
use freebird::frontend::parse;
use freebird::hir::lower;
use freebird::repl::Repl;
use freebird::typeck::{check_program_with_warnings, TypeTable};

/// Freebird — 一门基于双栈虚拟机的高级系统编程语言
#[derive(Parser)]
#[command(name = "freebird", version = "0.1.0", about = "Freebird language toolchain")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 解释执行 Freebird 程序
    Run {
        /// 源文件路径 (.fb)
        file: PathBuf,
        /// 调试模式
        #[arg(long)]
        debug: bool,
    },
    /// 语法和类型检查（JSON 诊断输出）
    Check {
        /// 源文件路径 (.fb)
        file: PathBuf,
    },
    /// 交互式 REPL
    Repl,
    /// 编译为 Stardust 源码或 LLVM IR / 可执行文件
    Build {
        /// 源文件路径 (.fb)
        file: PathBuf,
        /// 编译目标（默认 stardust）
        #[arg(long, value_enum)]
        target: Option<Target>,
        /// 输出文件路径；省略时打印到 stdout（stardust/llvm-ir）或使用源文件名（可执行文件）
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// 产物类型：ir（LLVM IR 文本）或 exe（可执行文件）；仅 --target llvm 使用
        #[arg(long, value_enum)]
        emit: Option<EmitOption>,
        /// 传给 clang 的优化级别（0-3）
        #[arg(long)]
        opt_level: Option<u8>,
        /// Stardust 可执行文件路径（也可用 STARDUST_BIN 环境变量）
        #[arg(long)]
        stardust_bin: Option<PathBuf>,
    },
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum Target {
    Stardust,
    Llvm,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum EmitOption {
    Ir,
    Exe,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Run { file, debug }) => {
            let config = if debug {
                RuntimeConfig::debug()
            } else {
                RuntimeConfig::default()
            };

            match run_file(&file, config) {
                Ok(output) => {
                    if !output.is_empty() {
                        print!("{}", String::from_utf8_lossy(&output));
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    process::exit(1);
                }
            }
        }
        Some(Command::Check { file }) => {
            let source = fs::read_to_string(&file).unwrap_or_else(|e| {
                eprintln!("Error reading {}: {e}", file.display());
                process::exit(1);
            });

            // JSON 诊断输出
            let (ast, parse_errors) = parse(&source);
            let (hir, resolve_errors) = lower(&ast);
            let mut type_errors = Vec::new();
            let mut type_warnings = Vec::new();
            let mut borrow_errors = Vec::new();
            if parse_errors.is_empty() && resolve_errors.is_empty() {
                let type_table = TypeTable::from_ast_decls(&ast.decls);
                let (errors, warnings) = check_program_with_warnings(&hir, &type_table);
                type_errors = errors;
                type_warnings = warnings;
                // 与 run/build 保持一致：类型检查通过后继续做借用/移动检查。
                if type_errors.is_empty() {
                    borrow_errors = check_borrow(&hir);
                }
            }
            let json = diagnostics::generate_diagnostic_json_with_warnings(
                &source,
                &parse_errors,
                &resolve_errors,
                &type_errors,
                &borrow_errors,
                &type_warnings,
            );
            println!("{json}");

            if !parse_errors.is_empty()
                || !resolve_errors.is_empty()
                || !type_errors.is_empty()
                || !borrow_errors.is_empty()
            {
                process::exit(1);
            }
        }
        Some(Command::Repl) => {
            let mut repl = Repl::new();
            if let Err(e) = repl.run() {
                eprintln!("REPL error: {e}");
                process::exit(1);
            }
        }
        Some(Command::Build {
            file,
            target,
            output,
            emit,
            opt_level,
            stardust_bin,
        }) => {
            let source = fs::read_to_string(&file).unwrap_or_else(|e| {
                eprintln!("Error reading {}: {e}", file.display());
                process::exit(1);
            });

            let target_name = target.unwrap_or(Target::Stardust);
            match target_name {
                Target::Stardust => {
                    if matches!(emit, Some(EmitOption::Exe)) {
                        eprintln!(
                            "Error: `--emit exe` requires `--target llvm`; the stardust target emits source text."
                        );
                        process::exit(1);
                    }
                    let code = match build_stardust(&source) {
                        Ok(code) => code,
                        Err(e) => {
                            eprintln!("Build error: {e}");
                            process::exit(1);
                        }
                    };
                    match output {
                        Some(path) => {
                            if let Err(e) = fs::write(&path, code) {
                                eprintln!("Error writing {}: {e}", path.display());
                                process::exit(1);
                            }
                            eprintln!("Stardust source written to {}", path.display());
                        }
                        None => println!("{code}"),
                    }
                }
                Target::Llvm => {
                    let emit = match emit.unwrap_or(EmitOption::Ir) {
                        EmitOption::Ir => LlvmEmit::Ir,
                        EmitOption::Exe => LlvmEmit::Executable,
                    };
                    if matches!(emit, LlvmEmit::Executable) && output.is_none() {
                        eprintln!(
                            "Error: `--emit exe` requires `--output <path>` (the output executable path)."
                        );
                        process::exit(1);
                    }
                    let opt_level = match opt_level.unwrap_or(0) {
                        level @ 0..=3 => level,
                        other => {
                            eprintln!("Error: `--opt-level` must be 0-3, got {other}");
                            process::exit(1);
                        }
                    };
                    let options = LlvmOptions {
                        emit,
                        output: output.clone(),
                        stardust_bin,
                        opt_level: opt_level as u8,
                    };
                    match compile_llvm(&source, &options) {
                        Ok(LlvmArtifact::Ir(ir)) => match output {
                            Some(path) => {
                                if let Err(e) = fs::write(&path, ir) {
                                    eprintln!("Error writing {}: {e}", path.display());
                                    process::exit(1);
                                }
                                eprintln!("LLVM IR written to {}", path.display());
                            }
                            None => print!("{ir}"),
                        },
                        Ok(LlvmArtifact::Executable(path)) => {
                            eprintln!("Executable written to {}", path.display());
                        }
                        Err(e) => {
                            eprintln!("Build error: {e}");
                            process::exit(1);
                        }
                    }
                }
            }
        }
        None => {
            println!("Freebird v0.1.0 — type 'freebird --help' for usage.");
            println!("Phases 0-10 complete: parser, HIR, MIR, VM, codegen, REPL, IO library.");
        }
    }
}
