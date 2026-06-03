//! Freebird 语言运行时库
//!
//! 提供双栈虚拟机核心、MIR 指令集及相关基础设施。

pub mod vm;

// 重导出常用类型
pub use vm::Vm;
pub use vm::config::RuntimeConfig;
pub use vm::error::VmError;
pub use vm::id::{ElementId, FnId, IdAllocator, StackId};
pub use vm::mir::{MirInst, TypeId};
pub use vm::stack::DualStack;
pub use vm::value::Value;
