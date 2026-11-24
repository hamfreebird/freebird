use clap::{Parser, Subcommand};
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process;

use freebird::borrowck::check_borrow;
use freebird::codegen::llvm::{LlvmArtifact, LlvmEmit, LlvmOptions, compile_llvm_compiled};
use freebird::dependency;
use freebird::diagnostics;
use freebird::frontend::parse;
use freebird::hir::lower;
use freebird::repl::Repl;
use freebird::typeck::{TypeTable, check_program_with_warnings};
use freebird::{
    CompileOptions, RuntimeConfig, build_project_with_options, build_stardust_with_options,
    check_project, compile_program_with_options, compile_project_with_options, run_file,
    run_project,
};

/// Freebird — 一门基于双栈虚拟机的高级系统编程语言
#[derive(Parser)]
#[command(
    name = "freebird",
    version = "0.1.0",
    about = "Freebird language toolchain"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 解释执行 Freebird 程序
    Run {
        /// 源文件路径 (.fb)；省略时读取当前目录的 freebird.toml
        file: Option<PathBuf>,
        /// 调试模式
        #[arg(long)]
        debug: bool,
        /// 项目清单路径（freebird.toml 或其所在目录）
        #[arg(long)]
        manifest_path: Option<PathBuf>,
    },
    /// 语法和类型检查（JSON 诊断输出）
    Check {
        /// 源文件路径 (.fb)；省略时读取当前目录的 freebird.toml
        file: Option<PathBuf>,
        /// 项目清单路径（freebird.toml 或其所在目录）
        #[arg(long)]
        manifest_path: Option<PathBuf>,
    },
    /// 交互式 REPL
    Repl,
    /// 启动 LSP 语言服务（stdio JSON-RPC，供编辑器使用）
    Lsp,
    /// 启动 DAP 调试适配器（stdio，供编辑器调试使用）
    Dap,
    /// 格式化 Freebird 源码（缩进/空白规范化）
    Fmt {
        /// 源文件路径；省略或使用 --stdin 时从标准输入读取
        file: Option<PathBuf>,
        /// 只检查是否已格式化（不一致时退出码 1）
        #[arg(long, conflicts_with = "write")]
        check: bool,
        /// 写回文件
        #[arg(long)]
        write: bool,
        /// 从标准输入读取
        #[arg(long, conflicts_with = "file")]
        stdin: bool,
    },
    /// 编译为 Stardust 源码或 LLVM IR / 可执行文件
    Build {
        /// 源文件路径 (.fb)；省略时读取当前目录的 freebird.toml
        file: Option<PathBuf>,
        /// 项目清单路径（freebird.toml 或其所在目录）
        #[arg(long)]
        manifest_path: Option<PathBuf>,
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
        /// 关闭 MIR 优化（仅影响 build 生成/LLVM，不影响 run）
        #[arg(long)]
        no_opt: bool,
        /// 追加外部 Stardust 定义文件（FFI 链接；适用于 stardust/llvm 目标）
        #[arg(long)]
        link_stardust: Option<PathBuf>,
    },
    /// 生成/更新 freebird.lock（记录依赖路径与内容哈希）
    Lock {
        /// 项目清单路径（freebird.toml 或其所在目录）；省略时使用当前目录
        #[arg(long)]
        manifest_path: Option<PathBuf>,
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

enum SourceTarget {
    File(PathBuf),
    Project(PathBuf),
}

/// 解析输入：显式 `--manifest-path` → 显式文件 → 当前目录的 `freebird.toml`。
fn resolve_target(
    file: Option<PathBuf>,
    manifest_path: Option<PathBuf>,
) -> Result<SourceTarget, String> {
    if let Some(path) = manifest_path {
        return Ok(SourceTarget::Project(path));
    }
    if let Some(file) = file {
        return Ok(SourceTarget::File(file));
    }
    let default = PathBuf::from("freebird.toml");
    if default.exists() {
        Ok(SourceTarget::Project(default))
    } else {
        Err(
            "no input file: pass <file.fb>, or run in a directory containing freebird.toml"
                .to_string(),
        )
    }
}

fn append_linked_stardust(code: &mut String, linked: &Option<String>) {
    let Some(linked) = linked else { return };
    if !code.ends_with('\n') {
        code.push('\n');
    }
    code.push_str(linked);
    if !code.ends_with('\n') {
        code.push('\n');
    }
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Run {
            file,
            debug,
            manifest_path,
        }) => {
            let config = if debug {
                RuntimeConfig::debug()
            } else {
                RuntimeConfig::default()
            };
            let target = match resolve_target(file, manifest_path) {
                Ok(target) => target,
                Err(error) => {
                    eprintln!("Error: {error}");
                    process::exit(1);
                }
            };
            let result = match target {
                SourceTarget::File(path) => run_file(&path, config),
                SourceTarget::Project(path) => run_project(&path, config),
            };
            match result {
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
        Some(Command::Check {
            file,
            manifest_path,
        }) => {
            let target = match resolve_target(file, manifest_path) {
                Ok(target) => target,
                Err(error) => {
                    eprintln!("Error: {error}");
                    process::exit(1);
                }
            };
            match target {
                SourceTarget::Project(path) => {
                    let (diagnostics, source) = match check_project(&path) {
                        Ok(result) => result,
                        Err(error) => {
                            eprintln!("Check error: {error}");
                            process::exit(1);
                        }
                    };
                    let (json, error_count, _warning_count) =
                        diagnostics::diagnostics_json(diagnostics, &source);
                    println!("{json}");
                    if error_count > 0 {
                        process::exit(1);
                    }
                }
                SourceTarget::File(file) => {
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
            }
        }
        Some(Command::Fmt {
            file,
            check,
            write,
            stdin,
        }) => {
            if write && (stdin || file.is_none()) {
                eprintln!("Error: `--write` requires a file path.");
                process::exit(1);
            }
            let (path, source) = match (stdin, file) {
                (false, Some(path)) => {
                    let source = fs::read_to_string(&path).unwrap_or_else(|e| {
                        eprintln!("Error reading {}: {e}", path.display());
                        process::exit(1);
                    });
                    (Some(path), source)
                }
                _ => {
                    let mut source = String::new();
                    if let Err(e) = io::stdin().read_to_string(&mut source) {
                        eprintln!("Error reading stdin: {e}");
                        process::exit(1);
                    }
                    (None, source)
                }
            };

            let result = freebird::fmt::format_source_checked(&source);
            if check {
                if result.changed {
                    let name = path
                        .as_ref()
                        .map_or_else(|| "<stdin>".to_string(), |p| p.display().to_string());
                    eprintln!("not formatted: {name}");
                    process::exit(1);
                }
            } else if write {
                if result.changed {
                    let path = path.expect("write requires a file path");
                    if let Err(e) = fs::write(&path, result.output) {
                        eprintln!("Error writing {}: {e}", path.display());
                        process::exit(1);
                    }
                }
            } else {
                print!("{}", result.output);
            }
        }
        Some(Command::Dap) => {
            if let Err(e) = freebird::dap::DapServer::new().run_stdio() {
                eprintln!("DAP error: {e}");
                process::exit(1);
            }
        }
        Some(Command::Lsp) => {
            if let Err(e) = freebird::lsp::LspServer::new().run_stdio() {
                eprintln!("LSP error: {e}");
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
        Some(Command::Lock { manifest_path }) => {
            let path = manifest_path.unwrap_or_else(|| PathBuf::from("freebird.toml"));
            match dependency::write_lock_for(&path) {
                Ok(lock) => println!("{}", lock.display()),
                Err(error) => {
                    eprintln!("Lock error: {error}");
                    process::exit(1);
                }
            }
        }
        Some(Command::Build {
            file,
            manifest_path,
            target,
            output,
            emit,
            opt_level,
            stardust_bin,
            no_opt,
            link_stardust,
        }) => {
            let compile_options = CompileOptions { optimize: !no_opt };
            let linked = link_stardust.as_ref().map(|path| {
                fs::read_to_string(path).unwrap_or_else(|error| {
                    eprintln!("Error reading {}: {error}", path.display());
                    process::exit(1);
                })
            });
            let target_name = target.unwrap_or(Target::Stardust);
            let source_target = match resolve_target(file, manifest_path) {
                Ok(target) => target,
                Err(error) => {
                    eprintln!("Error: {error}");
                    process::exit(1);
                }
            };

            match target_name {
                Target::Stardust => {
                    if matches!(emit, Some(EmitOption::Exe)) {
                        eprintln!(
                            "Error: `--emit exe` requires `--target llvm`; the stardust target emits source text."
                        );
                        process::exit(1);
                    }
                    let code = match &source_target {
                        SourceTarget::File(path) => {
                            let source = fs::read_to_string(path).unwrap_or_else(|e| {
                                eprintln!("Error reading {}: {e}", path.display());
                                process::exit(1);
                            });
                            build_stardust_with_options(&source, compile_options)
                        }
                        SourceTarget::Project(path) => {
                            build_project_with_options(path, compile_options)
                        }
                    };
                    let mut code = match code {
                        Ok(code) => code,
                        Err(e) => {
                            eprintln!("Build error: {e}");
                            process::exit(1);
                        }
                    };
                    append_linked_stardust(&mut code, &linked);
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
                    let compiled = match &source_target {
                        SourceTarget::File(path) => {
                            let source = fs::read_to_string(path).unwrap_or_else(|e| {
                                eprintln!("Error reading {}: {e}", path.display());
                                process::exit(1);
                            });
                            compile_program_with_options(&source, compile_options)
                        }
                        SourceTarget::Project(path) => {
                            compile_project_with_options(path, compile_options)
                        }
                    };
                    let artifact = match compiled {
                        Ok(mut compiled) => {
                            append_linked_stardust(&mut compiled.stardust, &linked);
                            compile_llvm_compiled(&compiled, &options)
                        }
                        Err(e) => Err(e),
                    };
                    match artifact {
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
