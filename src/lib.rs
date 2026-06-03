//! Freebird 语言运行时库
//!
//! 提供双栈虚拟机核心、MIR 指令集、词法/语法分析器。

pub mod frontend;
pub mod vm;

// 重导出常用类型
pub use frontend::parse;
pub use vm::Vm;
pub use vm::config::RuntimeConfig;
pub use vm::error::VmError;
pub use vm::id::{ElementId, FnId, IdAllocator, StackId};
pub use vm::mir::{MirInst, TypeId};
pub use vm::stack::DualStack;
pub use vm::value::Value;
