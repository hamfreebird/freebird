pub mod config;
pub mod error;
pub mod frame;
pub mod id;
pub mod mir;
pub mod registry;
pub mod stack;
pub mod value;
pub mod var_stack;

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::rc::Rc;

use config::RuntimeConfig;
use error::VmError;
use frame::CallFrame;
use id::{ElementId, FnId, IdAllocator};
use mir::MirInst;
use registry::GlobalRegistry;
use value::Value;

/// 函数表类型：FnId → MIR 指令序列
pub type FnTable = HashMap<FnId, Rc<Vec<MirInst>>>;

/// Freebird 虚拟机
pub struct Vm {
    /// 调用帧栈（frame 0 = 主程序）
    pub call_stack: Vec<CallFrame>,
    /// 远程栈注册表
    pub registry: GlobalRegistry,
    /// ID 分配器
    pub id_alloc: IdAllocator,
    /// 运行时配置
    pub config: RuntimeConfig,
    /// 已执行步数
    pub step_count: u64,
    /// 函数表
    pub fn_table: FnTable,
    /// 输出缓冲区
    pub output: Vec<u8>,
}

impl Vm {
    /// 使用默认配置创建 VM
    pub fn new() -> Self {
        Self::with_config(RuntimeConfig::default())
    }

    /// 使用指定配置创建 VM
    pub fn with_config(config: RuntimeConfig) -> Self {
        Self {
            call_stack: Vec::new(),
            registry: GlobalRegistry::new(),
            id_alloc: IdAllocator::new(),
            config,
            step_count: 0,
            fn_table: HashMap::new(),
            output: Vec::new(),
        }
    }

    /// 注册函数
    pub fn register_fn(&mut self, fn_id: FnId, instructions: Vec<MirInst>) {
        self.fn_table
            .insert(fn_id, Rc::new(instructions));
    }

    /// 加载主程序 MIR 并执行
    pub fn execute_mir(&mut self, instructions: Vec<MirInst>) -> Result<(), VmError> {
        let main_sid = self.id_alloc.alloc_stack();
        let main_frame = CallFrame::new_main(instructions, main_sid);
        self.call_stack.push(main_frame);
        self.run()
    }

    /// 主执行循环
    fn run(&mut self) -> Result<(), VmError> {
        loop {
            // 步数限制检查
            if self.config.max_steps > 0 && self.step_count >= self.config.max_steps {
                return Err(VmError::StepLimitExceeded);
            }

            // 检查调用栈深度
            if self.call_stack.len() > self.config.max_call_depth {
                return Err(VmError::CallStackOverflow);
            }

            // 获取当前帧
            match self.call_stack.last() {
                Some(f) if f.is_done() => {
                    if self.call_stack.len() == 1 {
                        // 主程序执行完毕
                        break;
                    } else {
                        // 函数帧执行完毕且无显式 ret —— 隐式返回
                        // 弹出当前帧（不保留任何返回值）
                        self.call_stack.pop();
                        continue;
                    }
                }
                Some(_) => {}
                None => return Err(VmError::EmptyCallStack),
            };

            // 取出当前指令（clone 以解 borrow）
            let inst = {
                let current_frame = self.call_stack.last().unwrap();
                current_frame.current_inst().cloned()
            };

            match inst {
                Some(inst) => {
                    self.execute_instruction(&inst)?;
                    self.step_count += 1;
                }
                None => break,
            }
        }
        Ok(())
    }

    /// 执行单条 MIR 指令
    fn execute_instruction(&mut self, inst: &MirInst) -> Result<(), VmError> {
        // 辅助宏：对于不需要访问 self 其他字段的简单操作，就地执行
        // 对于需要 self.id_alloc / self.output 的操作，先 pop 帧再 push 回

        match inst {
            // === 栈操作 ===
            MirInst::Push { value, .. } => {
                let new_id = self.id_alloc.alloc_element();
                let mut frame = self.pop_frame()?;
                frame.data.push(new_id, value.clone());
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::Pop { .. } => {
                let mut frame = self.pop_frame()?;
                frame.data.pop();
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::Dup { .. } => {
                let mut frame = self.pop_frame()?;
                frame.data.dup(&mut self.id_alloc)?;
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::Roll { n } => {
                let mut frame = self.pop_frame()?;
                frame.data.roll(*n)?;
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::Swap => {
                let mut frame = self.pop_frame()?;
                frame.data.swap()?;
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::Pick => {
                let mut frame = self.pop_frame()?;
                frame.data.pick(&mut self.id_alloc)?;
                frame.advance_pc();
                self.call_stack.push(frame);
            }

            // === 算术 ===
            MirInst::Add { .. } => self.binary_arith_op(|a, b| Ok(a + b))?,
            MirInst::Sub { .. } => self.binary_arith_op(|a, b| Ok(a - b))?,
            MirInst::Mul { .. } => self.binary_arith_op(|a, b| Ok(a * b))?,
            MirInst::Div { .. } => self.binary_arith_op(|a, b| {
                if b == 0 { Err(VmError::DivisionByZero) } else { Ok(a / b) }
            })?,
            MirInst::Mod { .. } => self.binary_arith_op(|a, b| {
                if b == 0 { Err(VmError::DivisionByZero) } else { Ok(a % b) }
            })?,
            MirInst::Neg { .. } => {
                let new_id = self.id_alloc.alloc_element();
                let mut frame = self.pop_frame()?;
                let (_, val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let v = val.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "numeric".to_string(),
                    found: val.type_name().to_string(),
                })?;
                frame.data.push(new_id, Value::I64(-v));
                frame.advance_pc();
                self.call_stack.push(frame);
            }

            // === 比较 ===
            MirInst::Eq => self.binary_cmp_op(|a, b| a == b)?,
            MirInst::Ne => self.binary_cmp_op(|a, b| a != b)?,
            MirInst::Lt => self.binary_cmp_op(|a, b| a < b)?,
            MirInst::Gt => self.binary_cmp_op(|a, b| a > b)?,
            MirInst::Le => self.binary_cmp_op(|a, b| a <= b)?,
            MirInst::Ge => self.binary_cmp_op(|a, b| a >= b)?,

            // === 逻辑 ===
            MirInst::And => self.binary_cmp_op(|a, b| a != 0 && b != 0)?,
            MirInst::Or => self.binary_cmp_op(|a, b| a != 0 || b != 0)?,
            MirInst::Xor => self.binary_cmp_op(|a, b| (a != 0) ^ (b != 0))?,
            MirInst::Not => {
                let new_id = self.id_alloc.alloc_element();
                let mut frame = self.pop_frame()?;
                let (_, val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let v = val.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer".to_string(),
                    found: val.type_name().to_string(),
                })?;
                let result = if v == 0 { Value::I64(1) } else { Value::I64(0) };
                frame.data.push(new_id, result);
                frame.advance_pc();
                self.call_stack.push(frame);
            }

            // === 控制流 ===
            MirInst::Mark(_) => {
                let mut frame = self.pop_frame()?;
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::Jump(mark) => {
                let mut frame = self.pop_frame()?;
                let (_, cond) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let cond_val = cond.as_i64().unwrap_or(0);
                if cond_val != 0 {
                    let target = *frame.marks.get(mark).ok_or(VmError::InvalidJumpTarget(*mark))?;
                    frame.pc = target;
                } else {
                    frame.advance_pc();
                }
                self.call_stack.push(frame);
            }
            MirInst::UncondJump(mark) => {
                let mut frame = self.pop_frame()?;
                let target = *frame.marks.get(mark).ok_or(VmError::InvalidJumpTarget(*mark))?;
                frame.pc = target;
                self.call_stack.push(frame);
            }
            MirInst::Call { fn_id } => {
                self.execute_call(*fn_id)?;
            }
            MirInst::Ret { argc } => {
                self.execute_ret(*argc)?;
            }

            // === 变量与作用域 ===
            MirInst::EnterScope => {
                let mut frame = self.pop_frame()?;
                frame.vars.enter_scope();
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::ExitScope => {
                let mut frame = self.pop_frame()?;
                let _dropped_bindings = frame.vars.exit_scope();
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::BindVar { name } => {
                let mut frame = self.pop_frame()?;
                let top_id = frame.data.top_id().ok_or(VmError::StackUnderflow)?;
                frame.vars.insert(name.clone(), top_id);
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::ReadVar { name } => {
                self.execute_read_var(name)?;
            }
            MirInst::WriteVar { name } => {
                self.execute_write_var(name)?;
            }

            // === 引用 ===
            MirInst::Ref { name, mutable } => {
                let new_id = self.id_alloc.alloc_element();
                let mut frame = self.pop_frame()?;
                let target_id = frame
                    .vars
                    .lookup(name)
                    .ok_or_else(|| VmError::UndefinedVariable(name.clone()))?;
                let ref_val = if *mutable {
                    Value::MutRef(target_id)
                } else {
                    Value::Ref(target_id)
                };
                frame.data.push(new_id, ref_val);
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::Deref => {
                let mut frame = self.pop_frame()?;
                let (_, ref_val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let target_id = match ref_val {
                    Value::Ref(id) | Value::MutRef(id) => id,
                    _ => {
                        return Err(VmError::TypeMismatch {
                            expected: "&T or &mut T".to_string(),
                            found: ref_val.type_name().to_string(),
                        })
                    }
                };
                let pos = frame
                    .data
                    .find_id(target_id)
                    .ok_or(VmError::DanglingReference {
                        id: target_id.0,
                        name: "deref target".to_string(),
                    })?;
                frame.data.roll(pos as i32)?;
                frame.data.dup(&mut self.id_alloc)?;
                frame.data.roll(-(pos as i32))?;
                frame.advance_pc();
                self.call_stack.push(frame);
            }

            // === 复合类型（骨架实现）===
            MirInst::FieldAccess { .. }
            | MirInst::IndexAccess
            | MirInst::MakeEnum { .. }
            | MirInst::EnumTag => {
                return Err(VmError::AssertionFailed(format!(
                    "{inst} not yet implemented in Phase 1"
                )));
            }

            // === 远程栈操作（骨架实现）===
            MirInst::AllocRemote
            | MirInst::FreeRemote
            | MirInst::RemotePush
            | MirInst::RemotePop
            | MirInst::RemoteLoad { .. }
            | MirInst::RemoteStore { .. }
            | MirInst::RemoteLen
            | MirInst::SwitchToRemote
            | MirInst::SwitchToLocal => {
                return Err(VmError::AssertionFailed(format!(
                    "{inst} not yet implemented in Phase 1"
                )));
            }

            // === I/O ===
            MirInst::NumOut => {
                let mut frame = self.pop_frame()?;
                let (_, val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let n = val.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "numeric".to_string(),
                    found: val.type_name().to_string(),
                })?;
                let output_str = format!("{n}\n");
                self.output.extend(output_str.as_bytes());
                println!("{n}");
                io::stdout().flush()?;
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::NumIn => {
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let n: i64 = input.trim().parse().map_err(|_| {
                    VmError::IoError(format!("failed to parse input as integer: {input}"))
                })?;
                let new_id = self.id_alloc.alloc_element();
                let mut frame = self.pop_frame()?;
                frame.data.push(new_id, Value::I64(n));
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::CharOut => {
                let mut frame = self.pop_frame()?;
                let (_, val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let c = val.as_i64().and_then(|v| char::from_u32(v as u32)).ok_or(
                    VmError::TypeMismatch {
                        expected: "char".to_string(),
                        found: val.type_name().to_string(),
                    },
                )?;
                let output_str = format!("{c}");
                self.output.extend(output_str.as_bytes());
                print!("{c}");
                io::stdout().flush()?;
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::CharIn => {
                let mut char_buf = [0u8; 1];
                io::stdin().read_exact(&mut char_buf)?;
                let c = char_buf[0] as u32;
                let new_id = self.id_alloc.alloc_element();
                let mut frame = self.pop_frame()?;
                frame.data.push(new_id, Value::Char(c));
                frame.advance_pc();
                self.call_stack.push(frame);
            }

            // === 调试 ===
            MirInst::DumpStack => {
                let mut frame = self.pop_frame()?;
                eprintln!("=== Stack Dump (frame: {:?}) ===", frame.stack_id);
                for (i, (id, val)) in frame.data.iter_bottom_to_top().enumerate() {
                    eprintln!("  [{i}] {id:?}: {val}");
                }
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::DumpState => {
                let step_count = self.step_count;
                let call_depth = self.call_stack.len();
                let mut frame = self.pop_frame()?;
                eprintln!("=== VM State ===");
                eprintln!("  step_count: {step_count}");
                eprintln!("  call_depth: {call_depth}");
                eprintln!("  frame.stack_id: {:?}", frame.stack_id);
                eprintln!("  frame.pc: {}", frame.pc);
                eprintln!("  frame.data.len(): {}", frame.data.len());
                eprintln!("  frame.vars.scope_depth(): {}", frame.vars.scope_depth());
                eprintln!(
                    "  frame.vars.bindings: {:?}",
                    frame.vars.current_frame_bindings()
                );
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::Breakpoint => {
                eprintln!("[Breakpoint] step={}", self.step_count);
                let mut frame = self.pop_frame()?;
                frame.advance_pc();
                self.call_stack.push(frame);
            }

            // === 元数据 ===
            MirInst::Nop => {
                let mut frame = self.pop_frame()?;
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::Comment(_) => {
                let mut frame = self.pop_frame()?;
                frame.advance_pc();
                self.call_stack.push(frame);
            }
        }
        Ok(())
    }

    // ─── 辅助方法 ───

    /// 弹出当前帧（从 call_stack 中取出，用于需要同时访问 self 其他字段的场景）
    fn pop_frame(&mut self) -> Result<CallFrame, VmError> {
        self.call_stack.pop().ok_or(VmError::EmptyCallStack)
    }

    /// 二元算术运算
    fn binary_arith_op<F>(&mut self, op: F) -> Result<(), VmError>
    where
        F: FnOnce(i64, i64) -> Result<i64, VmError>,
    {
        let new_id = self.id_alloc.alloc_element();
        let mut frame = self.pop_frame()?;
        let (_, rhs) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
        let (_, lhs) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
        let a = lhs.as_i64().ok_or(VmError::TypeMismatch {
            expected: "numeric".to_string(),
            found: lhs.type_name().to_string(),
        })?;
        let b = rhs.as_i64().ok_or(VmError::TypeMismatch {
            expected: "numeric".to_string(),
            found: rhs.type_name().to_string(),
        })?;
        let result = op(a, b)?;
        frame.data.push(new_id, Value::I64(result));
        frame.advance_pc();
        self.call_stack.push(frame);
        Ok(())
    }

    /// 二元比较/逻辑运算（结果为 bool → i64）
    fn binary_cmp_op<F>(&mut self, op: F) -> Result<(), VmError>
    where
        F: FnOnce(i64, i64) -> bool,
    {
        let new_id = self.id_alloc.alloc_element();
        let mut frame = self.pop_frame()?;
        let (_, rhs) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
        let (_, lhs) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
        let a = lhs.as_i64().ok_or(VmError::TypeMismatch {
            expected: "comparable".to_string(),
            found: lhs.type_name().to_string(),
        })?;
        let b = rhs.as_i64().ok_or(VmError::TypeMismatch {
            expected: "comparable".to_string(),
            found: rhs.type_name().to_string(),
        })?;
        let result = if op(a, b) { Value::Bool(true) } else { Value::Bool(false) };
        frame.data.push(new_id, result);
        frame.advance_pc();
        self.call_stack.push(frame);
        Ok(())
    }

    /// 执行函数调用
    fn execute_call(&mut self, fn_id: FnId) -> Result<(), VmError> {
        // CallElement 在参数下方，因此从栈顶向下扫描查找
        let (argc, ret_pc) = {
            let frame = self.pop_frame()?;
            // 从栈顶向下扫描，找到第一个 CallElement
            let argc = {
                let mut found = None;
                for n in 0..frame.data.len() {
                    if let Some((_, Value::CallElement { argc })) = frame.data.get_from_top(n) {
                        found = Some(*argc);
                        break;
                    }
                }
                found.ok_or_else(|| {
                    VmError::AssertionFailed(
                        "Call instruction requires a CallElement on the stack".to_string(),
                    )
                })?
            };
            let ret_pc = frame.pc + 1;
            self.call_stack.push(frame);
            (argc, ret_pc)
        };

        // 获取被调用函数的指令
        let callee_instructions = self
            .fn_table
            .get(&fn_id)
            .cloned()
            .ok_or_else(|| {
                VmError::AssertionFailed(format!(
                    "function {fn_id:?} not found in fn_table"
                ))
            })?;

        // 从调用者栈弹出：argc 个参数 + CallElement
        let mut params: Vec<(ElementId, Value)> = Vec::with_capacity(argc as usize);
        let call_element_id = {
            let mut frame = self.pop_frame()?;
            for _ in 0..argc {
                let pair = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                params.push(pair);
            }
            let (ce_id, ce_val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
            if !matches!(ce_val, Value::CallElement { .. }) {
                return Err(VmError::AssertionFailed(format!(
                    "expected CallElement, found {ce_val}"
                )));
            }
            self.call_stack.push(frame);
            ce_id
        };
        params.reverse(); // 恢复声明顺序（第一个参数在栈深）

        // 派生被调函数的 StackId
        self.id_alloc.bump_recursion();
        let callee_stack_id = self.id_alloc.derive_stack(call_element_id);

        // 构建新帧
        let mut new_frame = CallFrame::new_call(
            callee_stack_id,
            callee_instructions,
            ret_pc,
            0, // return_arity 在后续阶段从函数签名获取
            None,
        );

        // 压入调用元素（被调函数 slot 0）
        new_frame.data.push(
            self.id_alloc.alloc_element(),
            Value::CallElement { argc },
        );

        // 压入参数
        for (id, val) in params {
            new_frame.data.push(id, val);
        }

        self.call_stack.push(new_frame);
        Ok(())
    }

    /// 执行返回
    fn execute_ret(&mut self, argc: u32) -> Result<(), VmError> {
        if self.call_stack.len() < 2 {
            // 主程序返回：保留栈顶 argc 个元素
            let mut frame = self.pop_frame()?;
            let keep_start = frame.data.len().saturating_sub(argc as usize);
            frame.data.truncate(keep_start);
            frame.pc = frame.instructions.len(); // 标记执行完毕
            self.call_stack.push(frame);
            return Ok(());
        }

        let m = argc as usize;

        // 从当前帧弹出返回值
        let mut ret_vals: Vec<(ElementId, Value)> = Vec::with_capacity(m);
        let ret_pc: usize;
        {
            let mut frame = self.pop_frame()?;
            for _ in 0..m {
                let pair = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                ret_vals.push(pair);
            }
            ret_pc = frame.ret_pc;
            // 不 push 回，直接丢弃帧
        }
        ret_vals.reverse(); // 恢复栈顶顺序（第一个返回值在栈深）

        // 将返回值压入调用者栈
        {
            let mut caller = self.pop_frame()?;
            caller.pc = ret_pc;
            for (id, val) in ret_vals {
                caller.data.push(id, val);
            }
            self.call_stack.push(caller);
        }

        Ok(())
    }

    /// 执行 ReadVar：动态查找 + roll+dup+roll(-pos)
    fn execute_read_var(&mut self, name: &str) -> Result<(), VmError> {
        let pos = {
            let frame = self.pop_frame()?;
            let id = frame
                .vars
                .lookup(name)
                .ok_or_else(|| VmError::UndefinedVariable(name.to_string()))?;
            let pos = frame.data.find_id(id).ok_or(VmError::DanglingReference {
                id: id.0,
                name: name.to_string(),
            })?;
            self.call_stack.push(frame);
            pos
        };

        let mut frame = self.pop_frame()?;
        frame.data.roll(pos as i32)?;
        frame.data.dup(&mut self.id_alloc)?;
        frame.data.roll(-(pos as i32))?;
        frame.advance_pc();
        self.call_stack.push(frame);
        Ok(())
    }

    /// 执行 WriteVar：前提是 RHS 求值结果已在栈顶
    fn execute_write_var(&mut self, name: &str) -> Result<(), VmError> {
        let pos = {
            let frame = self.pop_frame()?;
            let id_old = frame
                .vars
                .lookup(name)
                .ok_or_else(|| VmError::UndefinedVariable(name.to_string()))?;
            let pos = frame.data.find_id(id_old).ok_or(VmError::DanglingReference {
                id: id_old.0,
                name: name.to_string(),
            })?;
            self.call_stack.push(frame);
            pos
        };

        // roll(pos); pop; roll(-pos); update VarStack
        let mut frame = self.pop_frame()?;
        frame.data.roll(pos as i32)?;
        frame.data.pop(); // 丢弃旧值
        // 新值归位（仅当旧值不是栈底元素时；若旧值在栈底，pop 后新值已在正确位置）
        let new_len = frame.data.len();
        if pos < new_len {
            frame.data.roll(-(pos as i32))?;
        }
        let new_id = frame.data.top_id().ok_or(VmError::StackUnderflow)?;
        frame.vars.update(name, new_id)?;
        frame.advance_pc();
        self.call_stack.push(frame);
        Ok(())
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mir::TypeId;

    fn i64_val(v: i64) -> Value {
        Value::I64(v)
    }

    fn make_vm() -> Vm {
        Vm::with_config(RuntimeConfig::test())
    }

    #[test]
    fn test_simple_arithmetic() {
        let mut vm = make_vm();
        let mir = vec![
            MirInst::Push { value: i64_val(3), ty: TypeId::I64 },
            MirInst::Push { value: i64_val(5), ty: TypeId::I64 },
            MirInst::Add { ty: TypeId::I64 },
        ];
        vm.execute_mir(mir).unwrap();
        let frame = vm.call_stack.last().unwrap();
        assert_eq!(frame.data.top_value(), Some(&i64_val(8)));
    }

    #[test]
    fn test_variable_bind_and_read() {
        let mut vm = make_vm();
        let mir = vec![
            MirInst::EnterScope,
            MirInst::Push { value: i64_val(10), ty: TypeId::I64 },
            MirInst::BindVar { name: "x".to_string() },
            MirInst::ReadVar { name: "x".to_string() },
            MirInst::Push { value: i64_val(1), ty: TypeId::I64 },
            MirInst::Add { ty: TypeId::I64 },
            MirInst::BindVar { name: "y".to_string() },
            MirInst::ExitScope,
        ];
        vm.execute_mir(mir).unwrap();
        let frame = vm.call_stack.last().unwrap();
        // 栈顶应为 y = 11 (x=10 在下面)
        assert_eq!(frame.data.get_from_top(0).unwrap().1, &i64_val(11));
        assert_eq!(frame.data.get_from_top(1).unwrap().1, &i64_val(10));
    }

    #[test]
    fn test_read_var_does_not_change_depth() {
        let mut vm = make_vm();
        let mir = vec![
            MirInst::EnterScope,
            MirInst::Push { value: i64_val(1), ty: TypeId::I64 },
            MirInst::BindVar { name: "a".to_string() },
            MirInst::Push { value: i64_val(2), ty: TypeId::I64 },
            MirInst::BindVar { name: "b".to_string() },
            MirInst::Push { value: i64_val(3), ty: TypeId::I64 },
            MirInst::BindVar { name: "c".to_string() },
            MirInst::ReadVar { name: "b".to_string() },
            MirInst::ExitScope,
        ];
        vm.execute_mir(mir).unwrap();
        let frame = vm.call_stack.last().unwrap();
        // ReadVar 后栈深度: 原始 3 个变量 + 1 个副本 = 4
        assert_eq!(frame.data.len(), 4);
    }

    #[test]
    fn test_write_var() {
        let mut vm = make_vm();
        let mir = vec![
            MirInst::EnterScope,
            MirInst::Push { value: i64_val(5), ty: TypeId::I64 },
            MirInst::BindVar { name: "x".to_string() },
            // x = x + 3
            MirInst::ReadVar { name: "x".to_string() },
            MirInst::Push { value: i64_val(3), ty: TypeId::I64 },
            MirInst::Add { ty: TypeId::I64 },
            MirInst::WriteVar { name: "x".to_string() },
            MirInst::ExitScope,
        ];
        vm.execute_mir(mir).unwrap();
        let frame = vm.call_stack.last().unwrap();
        assert_eq!(frame.data.top_value(), Some(&i64_val(8)));
    }

    #[test]
    fn test_if_else_control_flow() {
        let mut vm = make_vm();
        // if true { push(1) } else { push(2) }
        let mir = vec![
            MirInst::Push { value: Value::Bool(true), ty: TypeId::Bool },
            MirInst::Jump(1),         // if true, jump to mark 1
            MirInst::UncondJump(2),   // else branch → mark 2
            MirInst::Mark(1),         // then branch
            MirInst::Push { value: i64_val(1), ty: TypeId::I64 },
            MirInst::UncondJump(3),   // skip else
            MirInst::Mark(2),         // else branch
            MirInst::Push { value: i64_val(2), ty: TypeId::I64 },
            MirInst::Mark(3),         // merge point
        ];
        vm.execute_mir(mir).unwrap();
        let frame = vm.call_stack.last().unwrap();
        assert_eq!(frame.data.top_value(), Some(&i64_val(1)));
    }

    #[test]
    fn test_call_and_ret() {
        let mut vm = make_vm();

        // 被调函数: 弹出 2 个参数，相加，返回 1 个值
        let callee_id = FnId(1);
        let callee_mir = vec![
            MirInst::Add { ty: TypeId::I64 },
            MirInst::Ret { argc: 1 },
        ];
        vm.register_fn(callee_id, callee_mir);

        // 主程序: add(20, 22) → 42
        let main_mir = vec![
            // 调用布局: [..., call_element, param_1, param_2]
            MirInst::Push { value: Value::CallElement { argc: 2 }, ty: TypeId::Unknown },
            MirInst::Push { value: i64_val(20), ty: TypeId::I64 },
            MirInst::Push { value: i64_val(22), ty: TypeId::I64 },
            MirInst::Call { fn_id: callee_id },
        ];
        vm.execute_mir(main_mir).unwrap();
        let frame = vm.call_stack.last().unwrap();
        assert_eq!(frame.data.len(), 1);
        assert_eq!(frame.data.top_value(), Some(&i64_val(42)));
    }
}
