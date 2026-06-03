use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::process;

use freebird::{check_source, run_source, RuntimeConfig};

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

            match check_source(&source) {
                Ok(()) => println!("{}: OK", file.display()),
                Err(errors) => {
                    for e in &errors {
                        eprintln!("{e}");
                    }
                    process::exit(1);
                }
            }
        }
        Some(Command::Repl) => {
            println!("Freebird REPL v0.1.0");
            println!("[Phase 4] REPL not yet implemented. Use interactive mode via the library API.");
        }
        Some(Command::Build { file, target }) => {
            let target_name = target.map_or("stardust".to_string(), |t| format!("{t:?}"));
            println!("Building: {} (target={})", file.display(), target_name);
            println!("[Phase 4] Code generation not yet implemented (Phase 9).");
        }
        None => {
            println!("Freebird v0.1.0 — type 'freebird --help' for usage.");
            println!("Current: Phase 4 — end-to-end compilation pipeline available via library API.");
        }
    }
}
