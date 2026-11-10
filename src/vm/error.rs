use std::fmt;

/// 虚拟机运行时错误
#[derive(Debug, Clone, PartialEq)]
pub enum VmError {
    /// 栈为空时尝试操作
    StackUnderflow,
    /// roll(n) 的 n 超出栈范围
    RollOutOfBounds { n: i32, len: usize },
    /// pick(n) 的 n 超出栈范围
    PickOutOfBounds { n: usize, len: usize },
    /// swap 时栈元素不足 2 个
    SwapUnderflow,
    /// 未定义的变量
    UndefinedVariable(String),
    /// 悬空引用（ElementId 在栈中找不到）
    DanglingReference { id: u64, name: String },
    /// 类型不匹配
    TypeMismatch {
        expected: String,
        found: String,
    },
    /// 步数超限
    StepLimitExceeded,
    /// 执行时间超限（秒）
    TimeLimitExceeded { limit: u64 },
    /// 数据栈溢出
    StackOverflow { limit: usize },
    /// 调用栈溢出
    CallStackOverflow,
    /// 调用栈为空
    EmptyCallStack,
    /// 无效的跳转目标
    InvalidJumpTarget(usize),
    /// 返回值个数不匹配
    ReturnArityMismatch { expected: u32, actual: u32 },
    /// 除零错误
    DivisionByZero,
    /// 远程栈未找到
    RemoteStackNotFound { sid: u64 },
    /// 数组/栈索引越界
    IndexOutOfBounds { index: i64, len: usize },
    /// 断言失败（调试用）
    AssertionFailed(String),
    /// I/O 错误
    IoError(String),
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmError::StackUnderflow => write!(f, "stack underflow"),
            VmError::RollOutOfBounds { n, len } => {
                write!(f, "roll({n}) out of bounds: stack depth is {len}")
            }
            VmError::PickOutOfBounds { n, len } => {
                write!(f, "pick({n}) out of bounds: stack depth is {len}")
            }
            VmError::SwapUnderflow => {
                write!(f, "swap requires at least 2 elements on the stack")
            }
            VmError::UndefinedVariable(name) => {
                write!(f, "undefined variable: `{name}`")
            }
            VmError::DanglingReference { id, name } => {
                write!(
                    f,
                    "dangling reference: ElementId({id}) for variable `{name}` not found on stack"
                )
            }
            VmError::TypeMismatch { expected, found } => {
                write!(f, "type mismatch: expected {expected}, found {found}")
            }
            VmError::StepLimitExceeded => {
                write!(f, "step limit exceeded")
            }
            VmError::TimeLimitExceeded { limit } => {
                write!(f, "time limit exceeded: more than {limit}s")
            }
            VmError::StackOverflow { limit } => {
                write!(f, "stack overflow: exceeded maximum stack depth of {limit}")
            }
            VmError::CallStackOverflow => {
                write!(f, "call stack overflow")
            }
            VmError::EmptyCallStack => {
                write!(f, "empty call stack")
            }
            VmError::InvalidJumpTarget(mark) => {
                write!(f, "invalid jump target: mark {mark} not found")
            }
            VmError::ReturnArityMismatch { expected, actual } => {
                write!(
                    f,
                    "return arity mismatch: expected {expected} values, but ret specifies {actual}"
                )
            }
            VmError::DivisionByZero => {
                write!(f, "division by zero")
            }
            VmError::RemoteStackNotFound { sid } => {
                write!(f, "remote stack not found: StackId({sid})")
            }
            VmError::IndexOutOfBounds { index, len } => {
                write!(f, "index out of bounds: {index} (available depth {len})")
            }
            VmError::AssertionFailed(msg) => {
                write!(f, "assertion failed: {msg}")
            }
            VmError::IoError(msg) => {
                write!(f, "I/O error: {msg}")
            }
        }
    }
}

impl std::error::Error for VmError {}

impl From<std::io::Error> for VmError {
    fn from(e: std::io::Error) -> Self {
        VmError::IoError(e.to_string())
    }
}
