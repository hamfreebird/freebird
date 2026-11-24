//! Freebird 语言运行时库
//!
//! 提供编译流水线、双栈 VM 核心、词法/语法、HIR、MIR 生成。

pub mod borrowck;
pub mod codegen;
pub mod dap;
pub mod diagnostics;
pub mod fmt;
pub mod frontend;
pub mod hir;
pub mod io;
pub mod lsp;
pub mod manifest;
pub mod mir;
pub mod pipeline;
pub mod repl;
pub mod typeck;
pub mod vm;

// 重导出常用类型
pub use diagnostics::{generate_diagnostic_json, generate_diagnostic_json_with_warnings};
pub use fmt::{FormatResult, format_source, format_source_checked};
pub use frontend::parse;
pub use hir::lower;
pub use io::{IoLib, std_library};
pub use pipeline::{
    CompiledProgram, build_project, build_stardust, check_project, check_source,
    check_source_with_warnings, compile_ast, compile_program, compile_project, load_project,
    run_file, run_project, run_source,
};
pub use vm::Vm;
pub use vm::config::RuntimeConfig;
pub use vm::error::VmError;
pub use vm::id::{ElementId, FnId, IdAllocator, StackId};
pub use vm::mir::{MirInst, TypeId};
pub use vm::stack::DualStack;
pub use vm::value::Value;
