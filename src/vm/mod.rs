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
use std::io::{self, BufRead, Read};
use std::rc::Rc;

use config::RuntimeConfig;
use error::VmError;
use frame::CallFrame;
use id::{ElementId, FnId, IdAllocator};
use mir::{MirInst, TypeId};
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
    /// 预置的标准输入缓冲（优先于真实 stdin）
    input: Vec<u8>,
    /// 输入缓冲读取位置
    input_pos: usize,
    /// `stardust!` 的 store/load 使用的全局堆（与 Stardust 解释器一致）
    pub heap: HashMap<i64, i64>,
}

impl Vm {
    /// 使用默认配置创建 VM
    pub fn new() -> Self {
        Self::with_config(RuntimeConfig::default())
    }

    /// 使用指定配置创建 VM
    pub fn with_config(config: RuntimeConfig) -> Self {
        let input = config.input.clone();
        Self {
            call_stack: Vec::new(),
            registry: GlobalRegistry::new(),
            id_alloc: IdAllocator::new(),
            config,
            step_count: 0,
            fn_table: HashMap::new(),
            output: Vec::new(),
            input,
            input_pos: 0,
            heap: HashMap::new(),
        }
    }

    /// 设置标准输入（覆盖配置中的预置输入）
    pub fn set_input(&mut self, input: impl Into<Vec<u8>>) {
        self.input = input.into();
        self.input_pos = 0;
    }

    /// 读取一个输入字节；预置缓冲耗尽后回退到真实 stdin。
    fn read_input_byte(&mut self) -> io::Result<u8> {
        if self.input_pos < self.input.len() {
            let b = self.input[self.input_pos];
            self.input_pos += 1;
            return Ok(b);
        }
        let mut buf = [0u8; 1];
        io::stdin().read_exact(&mut buf)?;
        Ok(buf[0])
    }

    /// 读取一行输入（不含行尾换行）；预置缓冲耗尽后回退到真实 stdin。
    fn read_input_line(&mut self) -> io::Result<String> {
        if self.input_pos < self.input.len() {
            let rest = &self.input[self.input_pos..];
            let (line, consumed) = match rest.iter().position(|&b| b == b'\n') {
                Some(i) => (&rest[..i], i + 1),
                None => (rest, rest.len()),
            };
            self.input_pos += consumed;
            // 与真实 stdin 路径一致：行尾的 `\r` 不进入结果（CRLF）。
            return Ok(String::from_utf8_lossy(line)
                .trim_end_matches('\r')
                .to_string());
        }
        // 按字节读取一行后再做 UTF-8 lossy 解码：与预置输入路径一致，
        // 非法字节替换为 U+FFFD，而不是让 String::read_line 直接报 I/O 错误。
        let mut bytes = Vec::new();
        io::stdin().lock().read_until(b'\n', &mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes)
            .trim_end_matches(['\n', '\r'])
            .to_string())
    }

    /// 注册函数
    pub fn register_fn(&mut self, fn_id: FnId, instructions: Vec<MirInst>) {
        self.fn_table
            .insert(fn_id, Rc::new(instructions));
    }

    /// 取出并清空当前输出缓冲
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
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
        let start = std::time::Instant::now();
        let result = self.run_loop(start);
        if let Err(ref error) = result
            && self.config.debug
        {
            // `--debug`：运行时错误附带 VM 状态，便于定位。
            eprintln!("[freebird debug] runtime error: {error}");
            if let Some(frame) = self.call_stack.last() {
                eprintln!(
                    "  pc={} stack_depth={} call_depth={} steps={}",
                    frame.pc,
                    frame.data.len(),
                    self.call_stack.len(),
                    self.step_count
                );
            }
        }
        result
    }

    fn run_loop(&mut self, start: std::time::Instant) -> Result<(), VmError> {
        loop {
            // 步数限制检查
            if self.config.max_steps > 0 && self.step_count >= self.config.max_steps {
                return Err(VmError::StepLimitExceeded);
            }

            // 执行时间限制检查
            if let Some(limit) = self.config.time_limit
                && start.elapsed().as_secs() >= limit
            {
                return Err(VmError::TimeLimitExceeded { limit });
            }

            // 检查调用栈深度
            if self.call_stack.len() > self.config.max_call_depth {
                return Err(VmError::CallStackOverflow);
            }

            // 获取当前帧
            match self.call_stack.last() {
                Some(f) if f.is_done() => {
                    if self.call_stack.len() == 1 {
                        break;
                    } else {
                        // 隐式返回: 保留 return_arity 个栈顶值, 其余丢弃
                        let return_arity = f.return_arity as usize;
                        let frame = self.call_stack.pop().unwrap();
                        let keep_start = frame.data.len().saturating_sub(return_arity);
                        let kept: Vec<(ElementId, Value)> = (keep_start..frame.data.len())
                            .map(|i| (frame.data.ids[i], frame.data.values[i].clone()))
                            .collect();
                        // 将保留值压入调用者栈
                        if let Some(caller) = self.call_stack.last_mut() {
                            caller.pc = frame.ret_pc;
                            for (id, val) in kept {
                                caller.data.push(id, val);
                            }
                        }
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
                let limit = self.config.max_stack;
                frame.data.push_checked(new_id, value.clone(), limit)?;
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

            MirInst::Cast { target } => {
                let new_id = self.id_alloc.alloc_element();
                let mut frame = self.pop_frame()?;
                let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let converted = cast_value(value, *target)?;
                frame.data.push(new_id, converted);
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
                let cond_val = cond.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer or bool".to_string(),
                    found: cond.type_name().to_string(),
                })?;
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
            MirInst::Call { fn_id, return_arity } => {
                self.execute_call(*fn_id, *return_arity)?;
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
                let val = match ref_val {
                    Value::Ref(id) | Value::MutRef(id) => {
                        if let Some(pos) = frame.data.find_id(id) {
                            let idx = frame.data.len() - 1 - pos;
                            frame.data.values[idx].clone()
                        } else if let Some((frame_idx, idx)) = self.find_ref_target(id) {
                            // 引用目标是调用者帧中的变量（例如 `f(&mut x)`）。
                            self.call_stack[frame_idx].data.values[idx].clone()
                        } else {
                            return Err(VmError::DanglingReference {
                                id: id.0,
                                name: "deref target".to_string(),
                            });
                        }
                    }
                    Value::Boxed(inner) => *inner,
                    _ => {
                        return Err(VmError::TypeMismatch {
                            expected: "&T, &mut T or Box<T>".to_string(),
                            found: ref_val.type_name().to_string(),
                        })
                    }
                };
                let new_id = self.id_alloc.alloc_element();
                frame.data.ids.push(new_id);
                frame.data.values.push(val);
                frame.advance_pc();
                self.call_stack.push(frame);
            }

            // === 复合类型 ===
            MirInst::FieldAccess { index } => {
                // 复合值以单槽 Value::Tuple 表示：弹出聚合值，压入字段副本。
                // 需要保留聚合值的场景（如 match）由生成器显式插入 Dup。
                let mut frame = self.pop_frame()?;
                let (_, aggregate) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let val = match aggregate {
                    Value::Tuple(elems) => {
                        elems.get(*index).cloned().ok_or(VmError::IndexOutOfBounds {
                            index: *index as i64,
                            len: elems.len(),
                        })?
                    }
                    other => {
                        return Err(VmError::TypeMismatch {
                            expected: "tuple/struct".to_string(),
                            found: other.type_name().to_string(),
                        });
                    }
                };
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, val);
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::IndexAccess => {
                // arr[i]: pop i, pop arr; 数组以单槽 Value::Tuple 表示。
                let mut frame = self.pop_frame()?;
                let (_, idx_val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let idx = idx_val.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer".to_string(),
                    found: idx_val.type_name().to_string(),
                })?;
                let (_, arr_val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;

                if idx < 0 {
                    return Err(VmError::IndexOutOfBounds {
                        index: idx,
                        len: frame.data.len(),
                    });
                }
                let idx = idx as usize;

                if let Value::Tuple(values) = arr_val {
                    let val = values.get(idx).cloned().ok_or(VmError::IndexOutOfBounds {
                        index: idx as i64,
                        len: values.len(),
                    })?;
                    let new_id = self.id_alloc.alloc_element();
                    frame.data.ids.push(new_id);
                    frame.data.values.push(val);
                    frame.advance_pc();
                    self.call_stack.push(frame);
                    return Ok(());
                }

                // 兼容旧式多槽数组布局（元素连续排列在栈上）。
                let len = frame.data.len();
                let val_idx = len
                    .checked_sub(1)
                    .and_then(|last| last.checked_sub(idx))
                    .ok_or(VmError::IndexOutOfBounds { index: idx as i64, len })?;

                let val = frame.data.values[val_idx].clone();
                let new_id = self.id_alloc.alloc_element();
                frame.data.ids.push(new_id);
                frame.data.values.push(val);
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::StoreDeref => {
                // *p = v: 栈顶为 value，其下为引用
                let mut frame = self.pop_frame()?;
                let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let (_, reference) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let target_id = match reference {
                    Value::Ref(id) | Value::MutRef(id) => id,
                    other => {
                        return Err(VmError::TypeMismatch {
                            expected: "reference".to_string(),
                            found: other.type_name().to_string(),
                        });
                    }
                };
                if let Some(idx) = frame.data.ids.iter().rposition(|id| *id == target_id) {
                    frame.data.values[idx] = value;
                } else if let Some((frame_idx, idx)) = self.find_ref_target(target_id) {
                    // 写入调用者帧中的变量（例如 `fn set(p: &mut i64)`）。
                    self.call_stack[frame_idx].data.values[idx] = value;
                } else {
                    return Err(VmError::DanglingReference {
                        id: target_id.0,
                        name: "<deref>".to_string(),
                    });
                }
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::FieldStore { name, index } => {
                // obj.field = v: 栈顶为 value；就地修改变量持有的 Tuple
                let mut frame = self.pop_frame()?;
                let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let var_id = frame
                    .vars
                    .lookup(name)
                    .ok_or_else(|| VmError::UndefinedVariable(name.clone()))?;
                let pos = frame.data.find_id(var_id).ok_or(VmError::DanglingReference {
                    id: var_id.0,
                    name: name.clone(),
                })?;
                let idx = frame.data.len() - 1 - pos;
                match &mut frame.data.values[idx] {
                    Value::Tuple(elems) => {
                        let len = elems.len();
                        let slot = elems.get_mut(*index).ok_or(VmError::IndexOutOfBounds {
                            index: *index as i64,
                            len,
                        })?;
                        *slot = value;
                    }
                    other => {
                        return Err(VmError::TypeMismatch {
                            expected: "struct".to_string(),
                            found: other.type_name().to_string(),
                        });
                    }
                }
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::IndexStore { name } => {
                // arr[i] = v: 栈顶为 value，其下为 index
                let mut frame = self.pop_frame()?;
                let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let (_, index_val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let index = index_val.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer".to_string(),
                    found: index_val.type_name().to_string(),
                })?;
                if index < 0 {
                    return Err(VmError::IndexOutOfBounds { index, len: 0 });
                }
                let index = index as usize;
                let var_id = frame
                    .vars
                    .lookup(name)
                    .ok_or_else(|| VmError::UndefinedVariable(name.clone()))?;
                let pos = frame.data.find_id(var_id).ok_or(VmError::DanglingReference {
                    id: var_id.0,
                    name: name.clone(),
                })?;
                let idx = frame.data.len() - 1 - pos;
                match &mut frame.data.values[idx] {
                    Value::Tuple(elems) => {
                        let len = elems.len();
                        let slot = elems.get_mut(index).ok_or(VmError::IndexOutOfBounds {
                            index: index as i64,
                            len,
                        })?;
                        *slot = value;
                    }
                    other => {
                        return Err(VmError::TypeMismatch {
                            expected: "array".to_string(),
                            found: other.type_name().to_string(),
                        });
                    }
                }
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::MakeArray { count } => {
                let mut frame = self.pop_frame()?;
                let count = *count as usize;
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                    values.push(value);
                }
                values.reverse();
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, Value::Tuple(values));
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::MakeArrayRepeat => {
                let mut frame = self.pop_frame()?;
                let (_, count_val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let count = count_val.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer".to_string(),
                    found: count_val.type_name().to_string(),
                })?;
                if count < 0 {
                    return Err(VmError::IndexOutOfBounds { index: count, len: 0 });
                }
                let (_, elem) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let values = vec![elem; count as usize];
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, Value::Tuple(values));
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::MakeBox => {
                let mut frame = self.pop_frame()?;
                let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, Value::Boxed(Box::new(value)));
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::ContainerLen => {
                let mut frame = self.pop_frame()?;
                let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let len = match value {
                    Value::Tuple(elems) => elems.len() as i64,
                    Value::Boxed(_) => 1,
                    other => {
                        return Err(VmError::TypeMismatch {
                            expected: "array/Vec/String".to_string(),
                            found: other.type_name().to_string(),
                        });
                    }
                };
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, Value::I64(len));
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::VecPush { name } => {
                let mut frame = self.pop_frame()?;
                let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let var_id = frame
                    .vars
                    .lookup(name)
                    .ok_or_else(|| VmError::UndefinedVariable(name.clone()))?;
                let pos = frame.data.find_id(var_id).ok_or(VmError::DanglingReference {
                    id: var_id.0,
                    name: name.clone(),
                })?;
                let idx = frame.data.len() - 1 - pos;
                match &mut frame.data.values[idx] {
                    Value::Tuple(elems) => elems.push(value),
                    other => {
                        return Err(VmError::TypeMismatch {
                            expected: "Vec/String".to_string(),
                            found: other.type_name().to_string(),
                        });
                    }
                }
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::VecPop { name } => {
                let mut frame = self.pop_frame()?;
                let var_id = frame
                    .vars
                    .lookup(name)
                    .ok_or_else(|| VmError::UndefinedVariable(name.clone()))?;
                let pos = frame.data.find_id(var_id).ok_or(VmError::DanglingReference {
                    id: var_id.0,
                    name: name.clone(),
                })?;
                let idx = frame.data.len() - 1 - pos;
                let popped = match &mut frame.data.values[idx] {
                    Value::Tuple(elems) => elems.pop().unwrap_or(Value::Unit),
                    other => {
                        return Err(VmError::TypeMismatch {
                            expected: "Vec/String".to_string(),
                            found: other.type_name().to_string(),
                        });
                    }
                };
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, popped);
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::MakeEnum { variant: _, data_slots } => {
                // 将 [tag, data...] 合并为单槽 Value::Tuple。
                // 变量绑定只跟踪一个 ElementId，因此枚举必须作为单槽值保存；
                // 原先的多槽布局会让 ReadVar 只复制最后一个槽。
                let mut frame = self.pop_frame()?;
                let count = *data_slots as usize + 1;
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                    values.push(value);
                }
                values.reverse();
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, Value::Tuple(values));
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::EnumTag { total_slots: _ } => {
                // 从栈顶枚举值读取标签并压入副本，保留枚举本身。
                let mut frame = self.pop_frame()?;
                let tag = match frame.data.top_value() {
                    Some(Value::Tuple(values)) => {
                        values.first().cloned().unwrap_or(Value::Unit)
                    }
                    Some(other) => other.clone(),
                    None => return Err(VmError::StackUnderflow),
                };
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, tag);
                frame.advance_pc();
                self.call_stack.push(frame);
            }

            // DropVar：VM 的值随栈槽释放；Stardust 的堆块由后端管理。
            MirInst::DropVar { .. } => {
                let mut frame = self.pop_frame()?;
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            // === 堆操作（stardust! 的 store/load）===
            MirInst::HeapStore => {
                // 栈布局：[value, addr]；Store 弹出地址，再弹出值。
                let mut frame = self.pop_frame()?;
                let (_, addr_val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let addr = addr_val.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer heap address".to_string(),
                    found: addr_val.type_name().to_string(),
                })?;
                let value = value.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer heap value".to_string(),
                    found: value.type_name().to_string(),
                })?;
                self.heap.insert(addr, value);
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::HeapLoad => {
                // 栈布局：[addr]；Load 弹出地址并压入 heap[addr]（未写入为 0）。
                let mut frame = self.pop_frame()?;
                let (_, addr_val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let addr = addr_val.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer heap address".to_string(),
                    found: addr_val.type_name().to_string(),
                })?;
                let value = self.heap.get(&addr).copied().unwrap_or(0);
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, Value::I64(value));
                frame.advance_pc();
                self.call_stack.push(frame);
            }

            // === 标准库 ===
            MirInst::Abs => {
                let mut frame = self.pop_frame()?;
                let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let n = value.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer".to_string(),
                    found: value.type_name().to_string(),
                })?;
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, Value::I64(n.wrapping_abs()));
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::Min => {
                let mut frame = self.pop_frame()?;
                let (_, b) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let (_, a) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let a = a.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer".to_string(),
                    found: a.type_name().to_string(),
                })?;
                let b = b.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer".to_string(),
                    found: b.type_name().to_string(),
                })?;
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, Value::I64(a.min(b)));
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::Max => {
                let mut frame = self.pop_frame()?;
                let (_, b) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let (_, a) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let a = a.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer".to_string(),
                    found: a.type_name().to_string(),
                })?;
                let b = b.as_i64().ok_or(VmError::TypeMismatch {
                    expected: "integer".to_string(),
                    found: b.type_name().to_string(),
                })?;
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, Value::I64(a.max(b)));
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::Unwrap { success_tag } => {
                let mut frame = self.pop_frame()?;
                let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let payload = match &value {
                    Value::Tuple(values) => {
                        let tag = values
                            .first()
                            .and_then(Value::as_i64)
                            .unwrap_or(i64::MIN);
                        if tag != *success_tag {
                            return Err(VmError::AssertionFailed(
                                "unwrap on empty/failed value".to_string(),
                            ));
                        }
                        values.get(1).cloned().unwrap_or(Value::Unit)
                    }
                    _ => {
                        return Err(VmError::TypeMismatch {
                            expected: "enum".to_string(),
                            found: value.type_name().to_string(),
                        });
                    }
                };
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, payload);
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::UnwrapOr { success_tag } => {
                let mut frame = self.pop_frame()?;
                let (_, default) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let (_, value) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let payload = match &value {
                    Value::Tuple(values) => {
                        let tag = values
                            .first()
                            .and_then(Value::as_i64)
                            .unwrap_or(i64::MIN);
                        if tag == *success_tag {
                            values.get(1).cloned().unwrap_or(Value::Unit)
                        } else {
                            default
                        }
                    }
                    _ => default,
                };
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, payload);
                frame.advance_pc();
                self.call_stack.push(frame);
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
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::NumIn => {
                let input = self.read_input_line()?;
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
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::CharIn => {
                let byte = self.read_input_byte()?;
                let c = byte as u32;
                let new_id = self.id_alloc.alloc_element();
                let mut frame = self.pop_frame()?;
                frame.data.push(new_id, Value::Char(c));
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::ReadLine => {
                let line = self.read_input_line()?;
                let new_id = self.id_alloc.alloc_element();
                let mut frame = self.pop_frame()?;
                frame.data.push(new_id, string_to_value(&line));
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::StrOut { newline } => {
                let mut frame = self.pop_frame()?;
                let (_, val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let s = value_to_string(&val)?;
                self.output.extend(s.as_bytes());
                if *newline {
                    self.output.push(b'\n');
                }
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::FileRead => {
                let mut frame = self.pop_frame()?;
                let (_, path_val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let path = value_to_string(&path_val)?;
                let contents = std::fs::read_to_string(&path)
                    .map_err(|e| VmError::IoError(format!("failed to read `{path}`: {e}")))?;
                let new_id = self.id_alloc.alloc_element();
                frame.data.push(new_id, string_to_value(&contents));
                frame.advance_pc();
                self.call_stack.push(frame);
            }
            MirInst::FileWrite => {
                let mut frame = self.pop_frame()?;
                let (_, data_val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let (_, path_val) = frame.data.pop().ok_or(VmError::StackUnderflow)?;
                let data = value_to_string(&data_val)?;
                let path = value_to_string(&path_val)?;
                std::fs::write(&path, data.as_bytes())
                    .map_err(|e| VmError::IoError(format!("failed to write `{path}`: {e}")))?;
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
    fn execute_call(&mut self, fn_id: FnId, return_arity: u32) -> Result<(), VmError> {
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
            return_arity,
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
            frame.pc = frame.instructions.len();
            self.call_stack.push(frame);
            return Ok(());
        }

        // 验证 return_arity
        {
            let frame = self.call_stack.last().ok_or(VmError::EmptyCallStack)?;
            if argc as u32 != frame.return_arity {
                return Err(VmError::ReturnArityMismatch {
                    expected: frame.return_arity,
                    actual: argc as u32,
                });
            }
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

    /// 在调用者帧中查找引用目标 `id`，返回 `(call_stack 下标, 栈槽下标)`。
    ///
    /// 引用按 ElementId 指向变量槽；把 `&mut x` 作为参数传给函数后，目标
    /// 仍在调用者帧里，因此解引用/写入需要跨帧查找。借用检查保证引用不会
    /// 逃逸出目标变量的生命周期。
    fn find_ref_target(&self, id: ElementId) -> Option<(usize, usize)> {
        for (frame_idx, frame) in self.call_stack.iter().enumerate().rev() {
            if let Some(pos) = frame.data.find_id(id) {
                return Some((frame_idx, frame.data.len() - 1 - pos));
            }
        }
        None
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
        // 直接索引访问，避免 roll/dup/roll 栈破坏
        let idx = frame.data.len() - 1 - pos;
        let val = frame.data.values[idx].clone();
        let new_id = self.id_alloc.alloc_element();
        frame.data.ids.push(new_id);
        frame.data.values.push(val);
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

        // roll(pos); pop; roll(-(pos-1)); update VarStack
        //
        // 目标是把 RHS 放到旧变量原来的槽位，保持变量槽位置不变。
        // roll(pos) 把旧变量移到栈顶；pop 丢弃旧值后，RHS 仍在栈顶，
        // 此时列表长度为 L-1，需要用 roll(-(pos-1)) 才能把 RHS 插回
        // 旧变量原来的索引 L-1-pos（roll(-pos) 会少移一位）。
        let mut frame = self.pop_frame()?;
        // RHS 在栈顶；roll/pop 之后它会落到变量原来的位置，
        // 因此必须记住它的 ElementId，不能用操作后的栈顶 ID。
        let new_id = frame.data.top_id().ok_or(VmError::StackUnderflow)?;
        frame.data.roll(pos as i32)?;
        frame.data.pop(); // 丢弃旧值
        let shift = pos.saturating_sub(1);
        if shift > 0 {
            frame.data.roll(-(shift as i32))?;
        }
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

/// 执行 `expr as T` 的运行时转换。
fn cast_value(value: Value, target: TypeId) -> Result<Value, VmError> {
    let numeric = |value: &Value| {
        value.as_i64().ok_or_else(|| VmError::TypeMismatch {
            expected: "numeric or scalar".to_string(),
            found: value.type_name().to_string(),
        })
    };
    match target {
        TypeId::I8 => Ok(Value::I8(numeric(&value)? as i8 as i64)),
        TypeId::I16 => Ok(Value::I16(numeric(&value)? as i16 as i64)),
        TypeId::I32 => Ok(Value::I32(numeric(&value)? as i32 as i64)),
        TypeId::I64 => Ok(Value::I64(numeric(&value)?)),
        TypeId::Isize => Ok(Value::Isize(numeric(&value)?)),
        TypeId::U8 => Ok(Value::U8(numeric(&value)? as u8 as i64)),
        TypeId::U16 => Ok(Value::U16(numeric(&value)? as u16 as i64)),
        TypeId::U32 => Ok(Value::U32(numeric(&value)? as u32 as i64)),
        TypeId::U64 => Ok(Value::U64(numeric(&value)? as u64 as i64)),
        TypeId::Usize => Ok(Value::Usize(numeric(&value)? as usize as i64)),
        TypeId::F32 => {
            let f = value.as_f64().ok_or_else(|| VmError::TypeMismatch {
                expected: "numeric".to_string(),
                found: value.type_name().to_string(),
            })?;
            Ok(Value::F32(f as f32))
        }
        TypeId::F64 => {
            let f = value.as_f64().ok_or_else(|| VmError::TypeMismatch {
                expected: "numeric".to_string(),
                found: value.type_name().to_string(),
            })?;
            Ok(Value::F64(f))
        }
        TypeId::Bool => {
            let b = numeric(&value)? != 0;
            Ok(Value::Bool(b))
        }
        TypeId::Char => {
            let n = numeric(&value)?;
            let ch = u32::try_from(n)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| VmError::TypeMismatch {
                    expected: "valid Unicode scalar value".to_string(),
                    found: n.to_string(),
                })?;
            Ok(Value::Char(ch as u32))
        }
        TypeId::Unit | TypeId::Ref | TypeId::MutRef | TypeId::ElementId
        | TypeId::StackHandle | TypeId::FnPtr | TypeId::Unknown => Ok(value),
    }
}

/// 将 VM 值解释为字符串：字符串字面量以 `Value::Tuple([Char...])` 表示。
fn value_to_string(value: &Value) -> Result<String, VmError> {
    if let Value::Tuple(chars) = value {
        let mut s = String::with_capacity(chars.len());
        for ch in chars {
            let code = match ch {
                Value::Char(c) => *c,
                other => {
                    return Err(VmError::TypeMismatch {
                        expected: "char".to_string(),
                        found: other.type_name().to_string(),
                    });
                }
            };
            let c = char::from_u32(code).ok_or_else(|| {
                VmError::IoError(format!("invalid char code in string: {code}"))
            })?;
            s.push(c);
        }
        return Ok(s);
    }
    if let Value::Char(c) = value {
        let c = char::from_u32(*c)
            .ok_or_else(|| VmError::IoError(format!("invalid char code: {c}")))?;
        return Ok(c.to_string());
    }
    Err(VmError::TypeMismatch {
        expected: "String".to_string(),
        found: value.type_name().to_string(),
    })
}

/// 将 Rust 字符串转换为 VM 的字符元组表示。
fn string_to_value(s: &str) -> Value {
    Value::Tuple(s.chars().map(|c| Value::Char(c as u32)).collect())
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
    fn test_write_var_preserves_slot_positions() {
        let mut vm = make_vm();
        let mir = vec![
            MirInst::EnterScope,
            MirInst::Push { value: i64_val(1), ty: TypeId::I64 },
            MirInst::BindVar { name: "a".to_string() },
            MirInst::Push { value: i64_val(2), ty: TypeId::I64 },
            MirInst::BindVar { name: "b".to_string() },
            // a = a + 10
            MirInst::ReadVar { name: "a".to_string() },
            MirInst::Push { value: i64_val(10), ty: TypeId::I64 },
            MirInst::Add { ty: TypeId::I64 },
            MirInst::WriteVar { name: "a".to_string() },
            MirInst::ExitScope,
        ];
        vm.execute_mir(mir).unwrap();
        let frame = vm.call_stack.last().unwrap();
        // 赋值不能改变变量槽的位置：a 在 b 下面。
        assert_eq!(frame.data.get_from_top(0).unwrap().1, &i64_val(2));
        assert_eq!(frame.data.get_from_top(1).unwrap().1, &i64_val(11));
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
            MirInst::Call { fn_id: callee_id, return_arity: 1 },
        ];
        vm.execute_mir(main_mir).unwrap();
        let frame = vm.call_stack.last().unwrap();
        assert_eq!(frame.data.len(), 1);
        assert_eq!(frame.data.top_value(), Some(&i64_val(42)));
    }

    #[test]
    fn test_heap_store_and_load() {
        let mut vm = make_vm();
        let mir = vec![
            // heap[5] = 42：栈布局 [value, addr]
            MirInst::Push { value: Value::I64(42), ty: TypeId::I64 },
            MirInst::Push { value: Value::I64(5), ty: TypeId::I64 },
            MirInst::HeapStore,
            // load heap[5]
            MirInst::Push { value: Value::I64(5), ty: TypeId::I64 },
            MirInst::HeapLoad,
        ];
        vm.execute_mir(mir).unwrap();
        assert_eq!(vm.heap.get(&5), Some(&42));
        let frame = vm.call_stack.last().unwrap();
        assert_eq!(frame.data.top_value(), Some(&i64_val(42)));
    }

    #[test]
    fn test_heap_load_unwritten_is_zero() {
        let mut vm = make_vm();
        let mir = vec![
            MirInst::Push { value: Value::I64(99), ty: TypeId::I64 },
            MirInst::HeapLoad,
        ];
        vm.execute_mir(mir).unwrap();
        let frame = vm.call_stack.last().unwrap();
        assert_eq!(frame.data.top_value(), Some(&i64_val(0)));
    }
}
