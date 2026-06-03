use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
            println!("Running: {} (debug={})", file.display(), debug);
            println!("[Phase 1] MIR execution backend ready — frontend not yet implemented.");
            println!("Use the library API to construct and execute MIR directly.");
        }
        Some(Command::Check { file }) => {
            println!("Checking: {}", file.display());
            println!("[Phase 1] Type checking not yet implemented.");
        }
        Some(Command::Repl) => {
            println!("Freebird REPL v0.1.0");
            println!("[Phase 1] REPL not yet implemented.");
        }
        Some(Command::Build { file, target }) => {
            let target_name = target.map_or("stardust".to_string(), |t| format!("{t:?}"));
            println!("Building: {} (target={})", file.display(), target_name);
            println!("[Phase 1] Code generation not yet implemented.");
        }
        None => {
            println!("Freebird v0.1.0 — type 'freebird --help' for usage.");
            println!("Current: Phase 1 VM core complete.");
        }
    }
}
