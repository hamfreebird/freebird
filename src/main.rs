use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::process;

use freebird::{build_stardust, run_source, RuntimeConfig};
use freebird::borrowck::check_borrow;
use freebird::diagnostics;
use freebird::frontend::parse;
use freebird::hir::lower;
use freebird::repl::Repl;
use freebird::typeck::{check_program, TypeTable};

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
    /// 编译为 Stardust 源码或 LLVM IR
    Build {
        /// 源文件路径 (.fb)
        file: PathBuf,
        /// 编译目标
        #[arg(long, value_enum)]
        target: Option<Target>,
    },
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum Target {
    Stardust,
    Llvm,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Run { file, debug }) => {
            let source = fs::read_to_string(&file).unwrap_or_else(|e| {
                eprintln!("Error reading {}: {e}", file.display());
                process::exit(1);
            });

            let config = if debug {
                RuntimeConfig::debug()
            } else {
                RuntimeConfig::default()
            };

            match run_source(&source, config) {
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
            let mut borrow_errors = Vec::new();
            if parse_errors.is_empty() && resolve_errors.is_empty() {
                let type_table = TypeTable::from_ast_decls(&ast.decls);
                type_errors = check_program(&hir, &type_table);
                // 与 run/build 保持一致：类型检查通过后继续做借用/移动检查。
                if type_errors.is_empty() {
                    borrow_errors = check_borrow(&hir);
                }
            }
            let json = diagnostics::generate_diagnostic_json(
                &source,
                &parse_errors,
                &resolve_errors,
                &type_errors,
                &borrow_errors,
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
        Some(Command::Build { file, target }) => {
            let source = fs::read_to_string(&file).unwrap_or_else(|e| {
                eprintln!("Error reading {}: {e}", file.display());
                process::exit(1);
            });

            let target_name = target.unwrap_or(Target::Stardust);
            match target_name {
                Target::Stardust => {
                    match build_stardust(&source) {
                        Ok(code) => println!("{code}"),
                        Err(e) => {
                            eprintln!("Build error: {e}");
                            process::exit(1);
                        }
                    }
                }
                Target::Llvm => {
                    eprintln!("LLVM backend not yet implemented (Phase 9).");
                    process::exit(1);
                }
            }
        }
        None => {
            println!("Freebird v0.1.0 — type 'freebird --help' for usage.");
            println!("Phases 0-10 complete: parser, HIR, MIR, VM, codegen, REPL, IO library.");
        }
    }
}
