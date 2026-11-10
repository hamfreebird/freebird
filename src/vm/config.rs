/// 运行时配置
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// 单帧最大栈深度（默认 1,048,576）
    pub max_stack: usize,
    /// 最大调用深度（默认 256）
    pub max_call_depth: usize,
    /// 最大执行步数（0 = 无限制，默认 10,000,000）
    pub max_steps: u64,
    /// 执行时间限制（秒，None = 无限制）
    pub time_limit: Option<u64>,
    /// 调试模式
    pub debug: bool,
    /// 预置标准输入（空 = 读取真实 stdin；用于测试与嵌入）
    pub input: Vec<u8>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_stack: 1_048_576,
            max_call_depth: 256,
            max_steps: 10_000_000,
            time_limit: None,
            debug: false,
            input: Vec::new(),
        }
    }
}

impl RuntimeConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// 调试模式构造
    pub fn debug() -> Self {
        Self {
            debug: true,
            ..Default::default()
        }
    }

    /// 测试用配置（较严格的限制）
    pub fn test() -> Self {
        Self {
            max_stack: 1024,
            max_call_depth: 64,
            max_steps: 1_000_000,
            time_limit: None,
            debug: true,
            input: Vec::new(),
        }
    }
}
