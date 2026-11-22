use std::collections::HashMap;
use std::rc::Rc;

use crate::vm::id::StackId;
use crate::vm::mir::MirInst;
use crate::vm::stack::DualStack;
use crate::vm::var_stack::VarStack;

/// 函数调用帧
#[derive(Debug, Clone)]
pub struct CallFrame {
    /// 帧标识（全局唯一 StackId）
    pub stack_id: StackId,

    /// 数据栈（ID + 值）
    pub data: DualStack,

    /// 变量栈
    pub vars: VarStack,

    /// MIR 指令序列（Rc 共享）
    pub instructions: Rc<Vec<MirInst>>,

    /// 指令指针
    pub pc: usize,

    /// 返回后调用者恢复的 PC
    pub ret_pc: usize,

    /// 函数签名的返回值个数（用于 ret m 验证）
    pub return_arity: u32,

    /// Marks 表：Mark编号 → 指令索引（用于跳转）
    pub marks: HashMap<usize, usize>,

    /// 函数名（调试用）
    pub fn_name: Option<String>,
}

impl CallFrame {
    /// 创建新的主程序帧
    pub fn new_main(instructions: Vec<MirInst>, stack_id: StackId) -> Self {
        let marks = Self::build_marks(&instructions);
        Self {
            stack_id,
            data: DualStack::new(),
            vars: VarStack::new(),
            instructions: Rc::new(instructions),
            pc: 0,
            ret_pc: 0,
            return_arity: 0,
            marks,
            fn_name: Some("main".to_string()),
        }
    }

    /// 创建函数调用帧
    pub fn new_call(
        stack_id: StackId,
        instructions: Rc<Vec<MirInst>>,
        ret_pc: usize,
        return_arity: u32,
        fn_name: Option<String>,
    ) -> Self {
        let marks = Self::build_marks(&instructions);
        let mut vars = VarStack::new();
        vars.enter_scope(); // 函数体作用域

        Self {
            stack_id,
            data: DualStack::new(),
            vars,
            instructions,
            pc: 0,
            ret_pc,
            return_arity,
            marks,
            fn_name,
        }
    }

    /// 预扫描指令序列，建立 Mark 编号到指令索引的映射
    fn build_marks(instructions: &[MirInst]) -> HashMap<usize, usize> {
        let mut marks = HashMap::new();
        for (i, inst) in instructions.iter().enumerate() {
            if let MirInst::Mark(n) = inst {
                marks.insert(*n, i);
            }
        }
        marks
    }

    /// 获取当前指令
    pub fn current_inst(&self) -> Option<&MirInst> {
        self.instructions.get(self.pc)
    }

    /// 检查执行是否已结束（pc 超出指令范围）
    pub fn is_done(&self) -> bool {
        self.pc >= self.instructions.len()
    }

    /// 递增 PC
    pub fn advance_pc(&mut self) {
        self.pc += 1;
    }

    /// 跳转到 marks 表中的目标
    pub fn jump_to_mark(&mut self, mark: usize) {
        if let Some(&idx) = self.marks.get(&mark) {
            self.pc = idx;
        }
    }
}
