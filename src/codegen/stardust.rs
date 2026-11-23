//! Stardust 后端 — 将 MIR 翻译为自带解释器接受的原始 Stardust 编码。
//!
//! Stardust 的指令由「行首空格数 + 符号」构成，例如：
//!
//! * `Push(n)` → `n + 5` 个空格 + `+`
//! * `Pop` → 4 个空格 + `+`
//! * `Add` → `*`（0 个空格）
//! * `Mark(n)` → `n` 个空格 + `` ` ``
//!
//! 生成器维护一个编译期的槽位栈，用来计算变量的相对深度。变量本身仍留在
//! 运行时值栈上（MIR 的变量语义就是栈槽），因此递归调用天然安全：每个
//! Stardust 调用帧都有独立的栈。
//!
//! 数组/结构体/枚举/字符串/Box 统一表示为全局堆上的单槽句柄。堆块布局为
//! `[len, cap, elem_0, ..., elem_n]`，句柄就是块起始地址。堆槽 0 用于
//! `Roll`/`WriteVar`/`Ret` 的临时交换，槽 1 是 bump allocator，槽 2..15
//! 是生成序列的临时区，堆块从地址 16 开始分配。
//!
//! 远程数据（`Vec`/`String`/`Box` 等）在 Stardust 解释器中没有独立远程栈原语，
//! 统一表示为全局堆块；`stardust!` 的 `Store`/`Load` 直接映射为堆操作。
//! 文件 I/O 使用解释器的 `FileRead`/`FileWrite` 指令（`,` 前 2/3 个空格）。

use std::collections::{HashMap, HashSet};

use crate::vm::id::FnId;
use crate::vm::mir::{MirInst, TypeId};
use crate::vm::value::Value;

/// 堆槽 0：`Roll`/`WriteVar`/`Ret` 的临时交换地址（沿用既有约定）。
const HEAP_ALLOC_PTR: usize = 1;
/// 当前堆块句柄。
const HEAP_HANDLE: usize = 2;
/// 暂存值（元素值、被写入的值等）。
const HEAP_VALUE: usize = 3;
/// 暂存长度/下标。
const HEAP_INDEX: usize = 4;
/// 暂存分配大小或新长度。
const HEAP_SIZE: usize = 5;
/// 暂存新分配的句柄。
const HEAP_NEW_HANDLE: usize = 6;
/// 循环计数器。
const HEAP_COUNTER: usize = 7;
/// `read_line` 的 pending 字节（-1 表示无）。
const HEAP_PENDING: usize = 8;
/// `read_line` 当前多字节序列的字节数。
const HEAP_BYTE_COUNT: usize = 9;
/// 空闲块链表头（0 表示空）。
const HEAP_FREE_LIST: usize = 10;
/// 分配器的空闲链表游标。
const HEAP_ALLOC_CURSOR: usize = 11;
/// `Vec` 扩容时保存新容量（避免被分配器覆盖 `HEAP_SIZE`）。
const HEAP_CAP: usize = 12;
/// 第一个可分配的堆块地址。
const HEAP_BASE: i64 = 16;
/// 堆块头部长度：`[len, cap, elems...]`。
const BLOCK_HEADER: i64 = 2;

/// 生成 Stardust 代码所需的函数元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnInfo {
    pub name: String,
    /// 参数个数
    pub params: u32,
    /// 返回值个数
    pub returns: u32,
}

impl FnInfo {
    pub fn new(name: impl Into<String>, params: u32, returns: u32) -> Self {
        Self { name: name.into(), params, returns }
    }
}

/// 编译期已知的常量值。
#[derive(Debug, Clone, PartialEq)]
enum ConstValue {
    Int(i64),
    /// 字符串字面量。Stardust 没有字符串类型，这里只用于常量传播输出。
    Str(String),
}

/// 编译期栈槽。
#[derive(Debug, Clone)]
struct Slot {
    /// 变量绑定 ID（`BindVar` 分配，`Dup` 产生的新槽不带绑定）。
    binding: Option<usize>,
    constant: Option<ConstValue>,
    /// 该槽是函数调用元素时记录参数个数。
    call_element: Option<u32>,
    /// 读取变量产生的临时槽：来源绑定 ID（用于 DropVar 的别名/逃逸检查）。
    source_binding: Option<usize>,
    /// 该槽是否可能持有可回收的 Move 堆块（Vec/Box）。
    heap_owned: bool,
}

impl Slot {
    fn unknown() -> Self {
        Self {
            binding: None,
            constant: None,
            call_element: None,
            source_binding: None,
            heap_owned: false,
        }
    }
}

/// 词法作用域：变量名 → 绑定 ID。
#[derive(Debug, Clone, Default)]
struct Scope {
    vars: HashMap<String, usize>,
}

/// 控制流汇合点的栈/作用域快照。
#[derive(Debug, Clone)]
struct BranchState {
    stack: Vec<Slot>,
    scopes: Vec<Scope>,
}

/// Stardust 代码生成器。
pub struct StardustGen {
    output: String,
    /// 编译期值栈模型（与运行时栈一一对应）。
    stack: Vec<Slot>,
    /// 作用域栈，`BindVar` 记录名字，`ExitScope` 弹出。
    scopes: Vec<Scope>,
    /// 绑定 ID 分配器（函数内单调递增）。
    next_binding: usize,
    /// 当前函数中需要按堆单元间接访问的变量名（被 `Ref` 取址过）。
    indirect_names: HashSet<String>,
    /// 当前函数中已标记为间接的绑定 ID。
    indirect_bindings: HashSet<usize>,
    /// 前向跳转目标的状态记录。
    branch_states: HashMap<usize, BranchState>,
    /// 当前指令是否可达（Ret/UncondJump 之后为 false）。
    reachable: bool,
    fn_info: HashMap<FnId, FnInfo>,
    /// 当前函数的返回尾声标签。
    epilogue_label: usize,
    /// 当前正在生成的函数（用于识别自递归调用）。
    current_fn: Option<FnId>,
    /// 运行时辅助循环使用的标签分配器（高于当前函数所有 MIR 标签）。
    next_helper_label: usize,
    /// 自递归函数 → 互递归别名函数名。
    ///
    /// 自带的旧版 Stardust 解释器有一个解析缺陷：函数体中出现与函数自身
    /// 同名的调用时，会被误判为函数声明结束。把自递归改成与原函数互为
    /// 别名的两份函数体（A 调 B，B 调 A），既保持语义，也能被新旧解释器
    /// 接受。
    aliases: HashMap<FnId, FnId>,
    /// 当前生成的是否为别名副本。
    alias_body: bool,
    /// 本函数中已知持有可回收 Move 堆块（Vec/Box）的绑定 ID。
    heap_bindings: HashSet<usize>,
    /// 当前函数的返回值个数（用于 DropVar 的逃逸检查）。
    current_returns: u32,
}

impl StardustGen {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            stack: Vec::new(),
            scopes: Vec::new(),
            next_binding: 0,
            indirect_names: HashSet::new(),
            indirect_bindings: HashSet::new(),
            branch_states: HashMap::new(),
            reachable: true,
            fn_info: HashMap::new(),
            epilogue_label: 0,
            current_fn: None,
            next_helper_label: 0,
            aliases: HashMap::new(),
            alias_body: false,
            heap_bindings: HashSet::new(),
            current_returns: 0,
        }
    }

    /// 生成整个程序的 Stardust 源码。
    ///
    /// `functions` 为 `(FnId, MIR)` 列表；`fn_info` 提供每个函数的参数/返回个数；
    /// `entry` 为程序入口（通常是 `main`）。入口会被生成顶层调用，其余函数
    /// 生成为 Stardust 函数声明。
    pub fn generate_program(
        &mut self,
        functions: &[(FnId, Vec<MirInst>)],
        fn_info: &HashMap<FnId, FnInfo>,
        entry: Option<FnId>,
    ) -> String {
        self.output.clear();
        self.fn_info = fn_info.clone();

        // Unicode 字符输出模式。该注解由解释器在词法分析前提取。
        self.output.push_str("#[char_mode=unicode]\n");
        self.emit_comment("Generated by Freebird Stardust backend");
        self.emit_comment(&format!(
            "{} function(s); entry: {}",
            functions.len(),
            entry.map_or("<none>".to_string(), |id| format!("FnId({})", id.0))
        ));
        self.output.push('\n');

        // 为自递归函数分配别名函数名。别名名从现有最大 FnId 之后开始，
        // 避免与真实函数冲突。
        self.aliases.clear();
        let max_id = functions.iter().map(|(id, _)| id.0).max().unwrap_or(0);
        let mut next_alias = max_id + 1;
        for (fn_id, mir) in functions {
            let self_recursive = mir
                .iter()
                .any(|inst| matches!(inst, MirInst::Call { fn_id: callee, .. } if callee == fn_id));
            if self_recursive {
                self.aliases.insert(*fn_id, FnId(next_alias));
                next_alias += 1;
            }
        }

        for (fn_id, mir) in functions {
            self.gen_function(*fn_id, *fn_id, mir, false);
            self.output.push('\n');
            if let Some(alias) = self.aliases.get(fn_id).copied() {
                self.gen_function(*fn_id, alias, mir, true);
                self.output.push('\n');
            }
        }

        // 全局 bump allocator 初始化：heap[1] = 16。堆是解释器全局状态，
        // 只需在程序入口调用前初始化一次；递归/多次调用共享同一分配器。
        if entry.is_some() {
            self.emit_comment("initialise global heap allocator: heap[1] = 16, heap[10] = 0");
            self.emit_push_int(HEAP_BASE);
            self.emit_push_int(HEAP_ALLOC_PTR as i64);
            self.raw_inst(0, '-'); // Store
            self.emit_push_int(0);
            self.emit_push_int(HEAP_FREE_LIST as i64);
            self.raw_inst(0, '-'); // Store
        }

        // 顶层入口调用：压入 CallElement（值 = 参数个数）和占位参数，
        // 然后用 argc+1 调用入口函数，使被调帧布局与 MIR 的 [CE, p1..pn] 一致。
        if let Some(entry_id) = entry {
            let info = self.fn_info.get(&entry_id).cloned();
            let params = info.as_ref().map_or(0, |i| i.params);
            let returns = info.as_ref().map_or(0, |i| i.returns);

            self.emit_comment(&format!(
                "program entry: call FnId({}) `{}`",
                entry_id.0,
                info.as_ref().map_or("?", |i| i.name.as_str())
            ));
            self.emit_push_int(params as i64);
            for _ in 0..params {
                // 入口函数（通常 main）没有实参；为有参数的兜底入口补 0。
                self.emit_push_int(0);
            }
            self.emit_call(entry_id, params + 1);
            if returns > 0 {
                self.emit_push_int(returns as i64);
                self.raw_inst(1, '>'); // DropN，丢弃入口返回值
            }
        }

        self.output.clone()
    }

    // ─── 函数生成 ───

    fn gen_function(&mut self, fn_id: FnId, output_id: FnId, mir: &[MirInst], alias_body: bool) {
        let info = self
            .fn_info
            .get(&fn_id)
            .cloned()
            .unwrap_or_else(|| FnInfo::new("?", 0, 0));

        self.emit_comment(&format!(
            "---- FnId({}) `{}`{}: {} param(s) -> {} return(s) ----",
            fn_id.0,
            info.name,
            if alias_body { " (mutual-recursion alias)" } else { "" },
            info.params,
            info.returns
        ));

        // 重置函数级状态。
        self.current_fn = Some(fn_id);
        self.alias_body = alias_body;
        self.next_binding = 0;
        self.heap_bindings.clear();
        self.current_returns = info.returns;
        self.indirect_names.clear();
        self.indirect_bindings.clear();
        self.branch_states.clear();
        self.reachable = true;
        self.scopes = vec![Scope::default()];

        // 被 `Ref` 取址的变量需要改成堆单元间接访问，这样 `*p = v`
        // 才能回写到变量本身。先扫描整个函数，保证 BindVar 时就能决定。
        for inst in mir {
            if let MirInst::Ref { name, .. } = inst {
                self.indirect_names.insert(name.clone());
            }
        }

        // 函数帧布局：[CallElement(n), p1, ..., pn]
        self.stack = Vec::with_capacity(info.params as usize + 1);
        let mut ce = Slot::unknown();
        ce.call_element = Some(info.params);
        ce.constant = Some(ConstValue::Int(info.params as i64));
        self.stack.push(ce);
        for _ in 0..info.params {
            self.stack.push(Slot::unknown());
        }

        let max_label = mir
            .iter()
            .filter_map(|inst| match inst {
                MirInst::Mark(n) | MirInst::Jump(n) | MirInst::UncondJump(n) => Some(*n),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        self.epilogue_label = max_label + 1;
        // 辅助循环标签必须避开 MIR 的所有 Mark/Jump 目标。
        self.next_helper_label = self.epilogue_label + 1;

        // 函数声明：(name):
        self.raw_inst(output_id.0 as usize, ':');

        for inst in mir {
            self.gen_inst(inst);
        }

        // 兜底返回：MIR 正常会以 Ret 结束；若最后一条是 Mark 等指令，
        // 这里补一个使用签名返回个数的清理。
        if self.reachable {
            self.emit_return(info.returns);
        }

        // 返回尾声：所有 Ret 清理完栈后跳转到这里，随后函数帧结束。
        self.raw_inst(self.epilogue_label, '`');

        // 函数声明结束：(name):
        self.raw_inst(output_id.0 as usize, ':');
    }

    /// 生成一条 MIR 指令。
    fn gen_inst(&mut self, inst: &MirInst) {
        match inst {
            MirInst::Mark(n) => {
                self.handle_mark(*n);
                return;
            }
            _ if !self.reachable => return,
            _ => {}
        }

        match inst {
            MirInst::Mark(_) => unreachable!(),

            // ─── 栈操作 ───
            MirInst::Push { value, .. } => {
                if let Value::FnPtr(id) = value {
                    // 函数指针用 FnId 整数表示，间接调用由 dispatch 解析。
                    self.emit_push_int(id.0 as i64);
                    self.stack.push(Slot::unknown());
                    return;
                }
                let (constant, call_element) = classify_value(value);
                if let Value::Tuple(elems) = value {
                    // 字符串字面量与元组字面量都建成堆上的定长块。
                    self.gen_push_tuple_literal(elems);
                } else {
                    match &constant {
                        Some(ConstValue::Int(n)) => self.emit_push_int(*n),
                        _ => {
                            // 只有不可由用户代码构造的内部值会走到这里
                            //（Ref/FnPtr/ElementId 等）；触发运行时陷阱，
                            // 避免静默压入 0。
                            self.emit_comment(&format!(
                                "internal value `{}` cannot be materialised",
                                value.type_name()
                            ));
                            self.emit_bounds_trap();
                        }
                    }
                }
                let mut slot = Slot::unknown();
                slot.constant = constant;
                slot.call_element = call_element;
                self.stack.push(slot);
            }
            MirInst::Pop { .. } => {
                if self.stack.pop().is_some() {
                    self.raw_inst(4, '+');
                } else {
                    self.emit_comment("WARNING: Pop with empty codegen stack skipped");
                }
            }
            MirInst::Dup { .. } => {
                if let Some(top) = self.stack.last().cloned() {
                    self.raw_inst(1, '+');
                    let mut slot = Slot::unknown();
                    slot.constant = top.constant;
                    slot.call_element = top.call_element;
                    slot.source_binding = top.source_binding;
                    slot.heap_owned = top.heap_owned;
                    self.stack.push(slot);
                } else {
                    self.emit_comment("WARNING: Dup with empty codegen stack skipped");
                }
            }
            MirInst::Swap => {
                if self.stack.len() >= 2 {
                    self.raw_inst(2, '+');
                    let len = self.stack.len();
                    self.stack.swap(len - 1, len - 2);
                } else {
                    self.emit_comment("WARNING: Swap with fewer than 2 stack slots skipped");
                }
            }
            MirInst::Roll { n } => {
                self.emit_roll(*n);
                self.model_roll(*n);
            }
            MirInst::Pick => {
                self.raw_inst(2, '<');
                let n_slot = self.stack.pop();
                let n = match n_slot.as_ref().and_then(|s| match &s.constant {
                    Some(ConstValue::Int(v)) => Some(*v),
                    _ => None,
                }) {
                    Some(v) => v,
                    None => {
                        self.emit_comment("WARNING: Pick depth is not a compile-time constant");
                        0
                    }
                };
                if n >= 0 && (n as usize) < self.stack.len() {
                    let idx = self.stack.len() - 1 - n as usize;
                    let constant = self.stack[idx].constant.clone();
                    let mut slot = Slot::unknown();
                    slot.constant = constant;
                    self.stack.push(slot);
                } else {
                    self.stack.push(Slot::unknown());
                }
            }

            // ─── 算术 ───
            MirInst::Add { .. } => self.gen_binary_arith(0, BinOp::Add),
            MirInst::Sub { .. } => self.gen_binary_arith(1, BinOp::Sub),
            MirInst::Mul { .. } => self.gen_binary_arith(2, BinOp::Mul),
            MirInst::Div { .. } => self.gen_binary_arith(3, BinOp::Div),
            MirInst::Mod { .. } => self.gen_binary_arith(4, BinOp::Mod),
            MirInst::Neg { .. } => {
                // 没有 Neg 指令：Push(0); Swap; Sub → 0 - x
                self.emit_push_int(0);
                self.raw_inst(2, '+');
                self.raw_inst(1, '*');
                let value = self.stack.pop();
                let constant = value.and_then(|s| match s.constant {
                    Some(ConstValue::Int(v)) => v.checked_neg().map(ConstValue::Int),
                    _ => None,
                });
                self.stack.push(Slot {
                    constant,
                    ..Slot::unknown()
                });
            }

            MirInst::Cast { target } => {
                let slot = self.stack.pop().unwrap_or_else(Slot::unknown);
                self.gen_cast(*target);
                // 宽整型/char 保持原常量；窄化或 bool 转换可能改变值，
                // 保守地丢弃常量跟踪，避免后续折叠使用错误的数值。
                let constant = match target {
                    TypeId::I64
                    | TypeId::Isize
                    | TypeId::U64
                    | TypeId::Usize
                    | TypeId::Char => slot.constant,
                    _ => None,
                };
                self.stack.push(Slot {
                    constant,
                    ..Slot::unknown()
                });
            }

            // ─── 比较 ───
            MirInst::Eq => self.gen_binary_cmp(0, CmpOp::Eq),
            MirInst::Ne => self.gen_binary_cmp(1, CmpOp::Ne),
            MirInst::Lt => self.gen_binary_cmp(2, CmpOp::Lt),
            MirInst::Gt => self.gen_binary_cmp(3, CmpOp::Gt),
            MirInst::Le => self.gen_binary_cmp(4, CmpOp::Le),
            MirInst::Ge => self.gen_binary_cmp(5, CmpOp::Ge),

            // ─── 逻辑 ───
            MirInst::And => self.gen_binary_logic(0, LogicOp::And),
            MirInst::Or => self.gen_binary_logic(1, LogicOp::Or),
            MirInst::Not => {
                self.raw_inst(2, '&');
                let value = self.stack.pop();
                let constant = value.and_then(|s| match s.constant {
                    Some(ConstValue::Int(v)) => Some(ConstValue::Int(if v == 0 { 1 } else { 0 })),
                    _ => None,
                });
                self.stack.push(Slot {
                    constant,
                    ..Slot::unknown()
                });
            }
            MirInst::Xor => self.gen_binary_logic(3, LogicOp::Xor),

            // ─── 控制流 ───
            MirInst::Jump(n) => {
                // Jump 总是弹出条件。
                self.stack.pop();
                self.record_branch_state(*n);
                self.raw_inst(*n, '\'');
            }
            MirInst::UncondJump(n) => {
                self.record_branch_state(*n);
                self.raw_inst(*n, '~');
                self.reachable = false;
            }
            MirInst::Call { fn_id, .. } => self.gen_call(*fn_id),
            MirInst::CallIndirect { argc, return_arity } => {
                self.gen_call_indirect(*argc, *return_arity);
            }
            MirInst::Ret { argc } => {
                self.emit_return(*argc);
            }

            // ─── 变量与作用域 ───
            MirInst::EnterScope => {
                self.scopes.push(Scope::default());
            }
            MirInst::ExitScope => {
                if self.scopes.len() > 1 {
                    self.scopes.pop();
                }
            }
            MirInst::BindVar { name } => {
                if self.stack.is_empty() {
                    self.emit_comment(&format!(
                        "WARNING: BindVar({name}) with empty codegen stack skipped"
                    ));
                } else {
                    let id = self.next_binding;
                    self.next_binding += 1;
                    let indirect = self.indirect_names.contains(name);
                    let source = self.stack.last().and_then(|slot| slot.source_binding);
                    let top_heap = self.stack.last().is_some_and(|slot| slot.heap_owned);
                    let mut heap_owned = false;
                    if !indirect {
                        // 所有权转移：来源绑定持有可回收堆块时，转给新绑定。
                        if let Some(src) = source
                            && self.heap_bindings.contains(&src)
                        {
                            self.heap_bindings.remove(&src);
                            self.heap_bindings.insert(id);
                            heap_owned = true;
                        } else if top_heap {
                            self.heap_bindings.insert(id);
                            heap_owned = true;
                        }
                    }
                    if let Some(top) = self.stack.last_mut() {
                        top.binding = Some(id);
                        top.source_binding = None;
                        top.heap_owned = heap_owned;
                        if indirect {
                            // 被取址的变量不能保留编译期常量。
                            top.constant = None;
                        }
                    }
                    if let Some(scope) = self.scopes.last_mut() {
                        scope.vars.insert(name.clone(), id);
                    }
                    if indirect {
                        self.indirect_bindings.insert(id);
                        // 运行时栈顶是初始值：包装成 [1, 1, value] 单元，
                        // 变量槽改为保存该单元句柄。
                        self.gen_make_box();
                    }
                }
            }
            MirInst::ReadVar { name } => self.gen_read_var(name),
            MirInst::WriteVar { name } => self.gen_write_var(name),

            // ─── 引用 ───
            MirInst::Ref { name, .. } => {
                // 被取址的变量在 BindVar 时已包装成堆单元 [1, 1, value]，
                // 变量槽保存单元句柄。这里直接返回句柄，使 `*p = v` 能
                // 写回原变量。
                let indirect = self
                    .find_binding(name)
                    .is_some_and(|binding| self.indirect_bindings.contains(&binding));
                if indirect {
                    let binding = self.find_binding(name).unwrap();
                    let index = self.find_slot_index(binding).unwrap();
                    let depth = self.stack.len() - 1 - index;
                    self.emit_push_int(depth as i64);
                    self.raw_inst(2, '<'); // Pick 单元句柄
                    self.stack.push(Slot::unknown());
                } else {
                    // 兜底：变量未走间接绑定（例如生成器栈模型不完整）。
                    // 复制值到堆单元，至少能安全解引用，但写入不会回写。
                    self.emit_comment(&format!(
                        "Ref({name}): copied into a heap cell (writes do not propagate back)"
                    ));
                    self.gen_read_var(name);
                    self.gen_make_box();
                }
            }
            MirInst::Deref => {
                self.gen_deref();
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }

            // ─── 复合类型（堆句柄）───
            MirInst::FieldAccess { index } => {
                self.gen_field_access(*index);
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }
            MirInst::IndexAccess => {
                self.gen_index_access();
                self.stack.pop();
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }
            MirInst::StoreDeref => {
                self.gen_store_deref();
            }
            MirInst::FieldStore { name, index } => {
                self.gen_field_store(name, *index);
            }
            MirInst::FieldStoreRef { name, index } => {
                self.gen_field_store_ref(name, *index);
            }
            MirInst::IndexStore { name } => {
                self.gen_index_store(name);
            }
            MirInst::MakeArray { count } => {
                self.gen_build_array(*count as usize);
                self.pop_n_model(*count as usize);
                self.stack.push(Slot::unknown());
            }
            MirInst::MakeArrayRepeat => {
                self.gen_make_array_repeat();
                self.stack.pop();
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }
            MirInst::MakeBox => {
                self.gen_make_box();
                self.stack.pop();
                let mut slot = Slot::unknown();
                slot.heap_owned = true;
                self.stack.push(slot);
            }
            MirInst::ContainerLen => {
                let constant = self
                    .stack
                    .last()
                    .and_then(|slot| slot.constant.clone());
                if let Some(ConstValue::Str(s)) = constant {
                    self.emit_comment("ContainerLen on string literal: constant-folded");
                    self.pop_n_emit(1);
                    self.emit_push_int(s.chars().count() as i64);
                } else {
                    self.gen_container_len();
                    self.stack.pop();
                }
                self.stack.push(Slot::unknown());
            }
            MirInst::VecPush { name } => {
                self.gen_vec_push(name);
            }
            MirInst::VecPop { name } => {
                self.gen_vec_pop(name);
                self.stack.push(Slot::unknown());
            }
            MirInst::DropVar { name } => {
                // 只回收确定持有 Move 堆块、且没有被返回/别名引用的绑定。
                let Some(binding) = self.find_binding(name) else {
                    return;
                };
                if !self.heap_bindings.contains(&binding)
                    || self.indirect_bindings.contains(&binding)
                    || self
                        .stack
                        .iter()
                        .any(|slot| slot.source_binding == Some(binding))
                {
                    return;
                }
                let Some(index) = self.find_slot_index(binding) else {
                    return;
                };
                let depth = self.stack.len() - 1 - index;
                self.emit_push_int(depth as i64);
                self.raw_inst(2, '<'); // Pick
                self.emit_store_scratch(HEAP_HANDLE);
                self.emit_release_block(HEAP_HANDLE);
                self.heap_bindings.remove(&binding);
            }
            MirInst::MakeEnum { variant: _, data_slots } => {
                self.gen_build_array(*data_slots as usize + 1);
                self.pop_n_model(*data_slots as usize + 1);
                self.stack.push(Slot::unknown());
            }
            MirInst::EnumTag { .. } => {
                self.gen_enum_tag();
                self.stack.push(Slot::unknown());
            }

            // ─── 堆操作（stardust! 的 store/load）───
            MirInst::HeapStore => {
                self.raw_inst(0, '-'); // Store: [value, addr]
                self.stack.pop();
                self.stack.pop();
            }
            MirInst::HeapLoad => {
                self.raw_inst(1, '-'); // Load: [addr] -> [heap[addr]]
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }
            MirInst::Abs => {
                // [x] -> [|x|]
                let neg = self.fresh_helper_label();
                let end = self.fresh_helper_label();
                self.emit_dup();
                self.emit_push_int(0);
                self.emit_lt();
                self.raw_inst(neg, '\'');
                self.raw_inst(end, '~');
                self.raw_inst(neg, '`');
                self.emit_push_int(0);
                self.emit_swap();
                self.emit_sub();
                self.raw_inst(end, '`');
            }
            MirInst::Min => {
                // [a, b] -> [min(a, b)]
                let keep_a = self.fresh_helper_label();
                let end = self.fresh_helper_label();
                self.emit_store_scratch(HEAP_VALUE); // b
                self.emit_store_scratch(HEAP_INDEX); // a
                self.emit_load_scratch(HEAP_INDEX);
                self.emit_load_scratch(HEAP_VALUE);
                self.emit_lt();
                self.raw_inst(keep_a, '\'');
                self.emit_load_scratch(HEAP_VALUE);
                self.raw_inst(end, '~');
                self.raw_inst(keep_a, '`');
                self.emit_load_scratch(HEAP_INDEX);
                self.raw_inst(end, '`');
                self.stack.pop();
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }
            MirInst::Max => {
                // [a, b] -> [max(a, b)]
                let keep_a = self.fresh_helper_label();
                let end = self.fresh_helper_label();
                self.emit_store_scratch(HEAP_VALUE); // b
                self.emit_store_scratch(HEAP_INDEX); // a
                self.emit_load_scratch(HEAP_INDEX);
                self.emit_load_scratch(HEAP_VALUE);
                self.emit_gt();
                self.raw_inst(keep_a, '\'');
                self.emit_load_scratch(HEAP_VALUE);
                self.raw_inst(end, '~');
                self.raw_inst(keep_a, '`');
                self.emit_load_scratch(HEAP_INDEX);
                self.raw_inst(end, '`');
                self.stack.pop();
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }
            MirInst::IntToStr => {
                self.gen_int_to_str();
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }
            MirInst::StrToInt {
                some_tag,
                none_tag,
            } => {
                self.gen_str_to_int(*some_tag, *none_tag);
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }
            MirInst::Unwrap { success_tag } => {
                // [enum] -> [payload]；失败时除零陷阱。
                let ok = self.fresh_helper_label();
                self.emit_store_scratch(HEAP_HANDLE);
                self.emit_load_scratch(HEAP_HANDLE);
                self.emit_push_int(BLOCK_HEADER);
                self.emit_add();
                self.raw_inst(1, '-');
                self.emit_push_int(*success_tag);
                self.raw_inst(0, '=');
                self.raw_inst(ok, '\'');
                self.emit_bounds_trap();
                self.raw_inst(ok, '`');
                self.emit_load_scratch(HEAP_HANDLE);
                self.emit_push_int(BLOCK_HEADER + 1);
                self.emit_add();
                self.raw_inst(1, '-');
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }
            MirInst::UnwrapOr { success_tag } => {
                // [enum, default] -> [payload 或 default]
                let use_payload = self.fresh_helper_label();
                let end = self.fresh_helper_label();
                self.emit_store_scratch(HEAP_VALUE); // default
                self.emit_store_scratch(HEAP_HANDLE); // enum
                self.emit_load_scratch(HEAP_HANDLE);
                self.emit_push_int(BLOCK_HEADER);
                self.emit_add();
                self.raw_inst(1, '-');
                self.emit_push_int(*success_tag);
                self.raw_inst(0, '=');
                self.raw_inst(use_payload, '\'');
                self.emit_load_scratch(HEAP_VALUE);
                self.raw_inst(end, '~');
                self.raw_inst(use_payload, '`');
                self.emit_load_scratch(HEAP_HANDLE);
                self.emit_push_int(BLOCK_HEADER + 1);
                self.emit_add();
                self.raw_inst(1, '-');
                self.raw_inst(end, '`');
                self.stack.pop();
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }
            MirInst::EnumPayloadToOption {
                match_tag,
                some_tag,
                none_tag,
            } => {
                // [enum] -> [option]：源 tag 等于 match_tag 时携带 payload。
                let some_end = self.fresh_helper_label();
                let end = self.fresh_helper_label();
                self.emit_store_scratch(HEAP_HANDLE); // 源枚举
                self.emit_load_scratch(HEAP_HANDLE);
                self.emit_push_int(BLOCK_HEADER);
                self.emit_add();
                self.raw_inst(1, '-');
                self.emit_push_int(*match_tag);
                self.raw_inst(0, '=');
                self.raw_inst(some_end, '\'');
                // None：单槽块。
                self.emit_alloc_const(BLOCK_HEADER as usize + 1);
                self.emit_store_scratch(HEAP_VALUE);
                self.emit_write_header_const(HEAP_VALUE, 0, 1);
                self.emit_write_header_const(HEAP_VALUE, 1, 1);
                self.emit_push_int(*none_tag);
                self.emit_load_scratch(HEAP_VALUE);
                self.emit_push_int(BLOCK_HEADER);
                self.emit_add();
                self.raw_inst(0, '-');
                self.emit_load_scratch(HEAP_VALUE);
                self.raw_inst(end, '~');
                // Some：标签 + payload。
                self.raw_inst(some_end, '`');
                self.emit_alloc_const(BLOCK_HEADER as usize + 2);
                self.emit_store_scratch(HEAP_VALUE);
                self.emit_write_header_const(HEAP_VALUE, 0, 2);
                self.emit_write_header_const(HEAP_VALUE, 1, 2);
                self.emit_push_int(*some_tag);
                self.emit_load_scratch(HEAP_VALUE);
                self.emit_push_int(BLOCK_HEADER);
                self.emit_add();
                self.raw_inst(0, '-');
                self.emit_load_scratch(HEAP_HANDLE);
                self.emit_push_int(BLOCK_HEADER + 1);
                self.emit_add();
                self.raw_inst(1, '-');
                self.emit_load_scratch(HEAP_VALUE);
                self.emit_push_int(BLOCK_HEADER + 1);
                self.emit_add();
                self.raw_inst(0, '-');
                self.emit_load_scratch(HEAP_VALUE);
                self.raw_inst(end, '`');
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }
            MirInst::OptionToResult {
                some_tag,
                ok_tag,
                err_tag,
            } => {
                // [option, default] -> [result]
                let some_end = self.fresh_helper_label();
                let end = self.fresh_helper_label();
                self.emit_store_scratch(HEAP_VALUE); // default
                self.emit_store_scratch(HEAP_HANDLE); // option
                self.emit_load_scratch(HEAP_HANDLE);
                self.emit_push_int(BLOCK_HEADER);
                self.emit_add();
                self.raw_inst(1, '-');
                self.emit_push_int(*some_tag);
                self.raw_inst(0, '=');
                self.raw_inst(some_end, '\'');
                // Err：标签 + default。
                self.emit_alloc_const(BLOCK_HEADER as usize + 2);
                self.emit_store_scratch(HEAP_INDEX);
                self.emit_write_header_const(HEAP_INDEX, 0, 2);
                self.emit_write_header_const(HEAP_INDEX, 1, 2);
                self.emit_push_int(*err_tag);
                self.emit_load_scratch(HEAP_INDEX);
                self.emit_push_int(BLOCK_HEADER);
                self.emit_add();
                self.raw_inst(0, '-');
                self.emit_load_scratch(HEAP_VALUE);
                self.emit_load_scratch(HEAP_INDEX);
                self.emit_push_int(BLOCK_HEADER + 1);
                self.emit_add();
                self.raw_inst(0, '-');
                self.emit_load_scratch(HEAP_INDEX);
                self.raw_inst(end, '~');
                // Ok：标签 + payload。
                self.raw_inst(some_end, '`');
                self.emit_alloc_const(BLOCK_HEADER as usize + 2);
                self.emit_store_scratch(HEAP_INDEX);
                self.emit_write_header_const(HEAP_INDEX, 0, 2);
                self.emit_write_header_const(HEAP_INDEX, 1, 2);
                self.emit_push_int(*ok_tag);
                self.emit_load_scratch(HEAP_INDEX);
                self.emit_push_int(BLOCK_HEADER);
                self.emit_add();
                self.raw_inst(0, '-');
                self.emit_load_scratch(HEAP_HANDLE);
                self.emit_push_int(BLOCK_HEADER + 1);
                self.emit_add();
                self.raw_inst(1, '-');
                self.emit_load_scratch(HEAP_INDEX);
                self.emit_push_int(BLOCK_HEADER + 1);
                self.emit_add();
                self.raw_inst(0, '-');
                self.emit_load_scratch(HEAP_INDEX);
                self.raw_inst(end, '`');
                self.stack.pop();
                self.stack.pop();
                self.stack.push(Slot::unknown());
            }

            // ─── I/O ───
            MirInst::NumOut => {
                self.stack.pop();
                self.raw_inst(0, '.');
                // Freebird 的数值输出总带换行。
                self.emit_push_int(10);
                self.raw_inst(0, ',');
            }
            MirInst::NumIn => {
                self.raw_inst(1, '.');
                self.stack.push(Slot::unknown());
            }
            MirInst::CharOut => {
                self.stack.pop();
                self.raw_inst(0, ',');
            }
            MirInst::CharIn => {
                self.raw_inst(1, ',');
                self.stack.push(Slot::unknown());
            }
            MirInst::StrOut { newline } => {
                let top = self.stack.pop();
                match top {
                    Some(Slot { constant: Some(ConstValue::Str(s)), .. }) => {
                        self.raw_inst(4, '+'); // 丢弃堆句柄
                        for ch in s.chars() {
                            self.emit_push_int(ch as i64);
                            self.raw_inst(0, ',');
                        }
                        if *newline {
                            self.emit_push_int(10);
                            self.raw_inst(0, ',');
                        }
                    }
                    Some(_) => {
                        self.gen_str_out_runtime(*newline);
                    }
                    None => {
                        self.emit_comment("WARNING: StrOut with empty codegen stack skipped");
                    }
                }
            }
            MirInst::ReadLine => {
                self.gen_read_line();
                self.stack.push(Slot::unknown());
            }
            MirInst::FileRead => {
                self.gen_file_read();
                self.stack.pop(); // path
                self.stack.push(Slot::unknown()); // contents
            }
            MirInst::FileWrite => {
                self.gen_file_write();
                self.stack.pop(); // data
                self.stack.pop(); // path
            }

            // ─── 调试与元数据 ───
            MirInst::DumpStack => self.raw_inst(0, '#'),
            MirInst::DumpState => self.raw_inst(1, '#'),
            MirInst::Breakpoint => self.raw_inst(2, '#'),
            MirInst::Nop => {}
            MirInst::Comment(text) => self.emit_comment(text),
        }
    }

    // ─── 变量 ───

    fn find_binding(&self, name: &str) -> Option<usize> {
        for scope in self.scopes.iter().rev() {
            if let Some(id) = scope.vars.get(name) {
                return Some(*id);
            }
        }
        None
    }

    fn find_slot_index(&self, binding: usize) -> Option<usize> {
        self.stack.iter().rposition(|slot| slot.binding == Some(binding))
    }

    fn gen_read_var(&mut self, name: &str) {
        let Some(binding) = self.find_binding(name) else {
            self.emit_comment(&format!("ERROR: ReadVar({name}) is not bound; push 0"));
            self.emit_push_int(0);
            self.stack.push(Slot::unknown());
            return;
        };
        let Some(index) = self.find_slot_index(binding) else {
            self.emit_comment(&format!(
                "ERROR: ReadVar({name}) has no stack slot; push 0"
            ));
            self.emit_push_int(0);
            self.stack.push(Slot::unknown());
            return;
        };
        let depth = self.stack.len() - 1 - index;
        self.emit_push_int(depth as i64);
        self.raw_inst(2, '<'); // Pick
        let indirect = self.indirect_bindings.contains(&binding);
        let constant = if indirect {
            None
        } else {
            self.stack[index].constant.clone()
        };
        if indirect {
            // 变量槽保存的是单元句柄，读出值需要解引用。
            self.gen_deref();
        }
        let heap_owned = !indirect && self.heap_bindings.contains(&binding);
        let mut slot = Slot::unknown();
        slot.constant = constant;
        slot.source_binding = if indirect { None } else { Some(binding) };
        slot.heap_owned = heap_owned;
        self.stack.push(slot);
    }

    fn gen_write_var(&mut self, name: &str) {
        let Some(binding) = self.find_binding(name) else {
            self.emit_comment(&format!("ERROR: WriteVar({name}) is not bound; drop RHS"));
            self.pop_n_emit(1);
            return;
        };
        let Some(index) = self.find_slot_index(binding) else {
            self.emit_comment(&format!(
                "ERROR: WriteVar({name}) has no stack slot; drop RHS"
            ));
            self.pop_n_emit(1);
            return;
        };

        if self.indirect_bindings.contains(&binding) {
            // 间接变量：RHS 写入堆单元，变量槽中的句柄保持不变。
            self.stack.pop(); // RHS
            self.emit_store_scratch(HEAP_VALUE);
            let depth = self.stack.len() - 1 - index;
            self.emit_push_int(depth as i64);
            self.raw_inst(2, '<'); // Pick 单元句柄
            self.emit_push_int(2);
            self.emit_add();
            self.emit_load_scratch(HEAP_VALUE);
            self.emit_swap();
            self.raw_inst(0, '-'); // Store
            if let Some(slot) = self.stack.get_mut(index) {
                slot.constant = None;
            }
            return;
        }

        let len = self.stack.len();
        let depth = len - 1 - index;

        // 与 VM 的 execute_write_var 一致：
        //   roll(depth); Pop; if depth >= 2 { roll(-(depth-1)) }
        // 目的：RHS 回到旧变量原槽位，保持变量栈布局稳定。
        self.emit_roll(depth as i32);
        self.raw_inst(4, '+'); // Pop
        if depth >= 2 {
            self.emit_roll(-((depth - 1) as i32));
        }

        // 更新模型：RHS 落到变量原来的位置。
        let rhs = self.stack.pop().expect("write var requires RHS");
        self.stack.remove(index);
        let mut slot = rhs;
        slot.binding = Some(binding);
        self.stack.insert(index, slot);

        // 名字表仍然指向同一个绑定 ID。
    }

    // ─── 函数调用 / 返回 ───

    /// 间接调用：栈布局 `[args..., fnptr]`。
    ///
    /// Stardust 没有间接调用原语，这里按 FnId 生成一组比较 + 直接调用。
    fn gen_call_indirect(&mut self, argc: u32, return_arity: u32) {
        let end = self.fresh_helper_label();
        let mut targets: Vec<(FnId, FnInfo)> = self
            .fn_info
            .iter()
            .filter(|(_, info)| info.params == argc && info.returns == return_arity)
            .map(|(id, info)| (*id, info.clone()))
            .collect();
        targets.sort_by_key(|(id, _)| id.0);

        let mut labels = Vec::with_capacity(targets.len());
        for (id, info) in targets {
            let label = self.fresh_helper_label();
            self.emit_dup();
            self.emit_push_int(id.0 as i64);
            self.raw_inst(0, '='); // Eq
            self.raw_inst(label, '\'');
            labels.push((id, info, label));
        }
        // 没有匹配的 FnId：运行时陷阱。
        self.emit_bounds_trap();

        for (id, info, label) in labels {
            self.raw_inst(label, '`');
            self.emit_pop(); // fnptr
            self.emit_push_int(info.params as i64); // CallElement
            // 只把 CallElement 下移到参数下方，不能越过调用者的变量槽。
            self.emit_roll(-(argc as i32));
            self.emit_call(id, info.params + 1);
            self.raw_inst(end, '~');
        }
        self.raw_inst(end, '`');

        for _ in 0..argc {
            self.stack.pop();
        }
        self.stack.pop(); // fnptr
        for _ in 0..return_arity {
            self.stack.push(Slot::unknown());
        }
    }

    fn gen_call(&mut self, fn_id: FnId) {        let info = self.fn_info.get(&fn_id).cloned();
        if info.is_none() || fn_id.0 == 0 {
            self.emit_comment(&format!(
                "ERROR: call to unknown FnId({}); call skipped",
                fn_id.0
            ));
            // 未知函数：按 CallElement 记录的 argc（若有）清理栈。
            let argc = self
                .stack
                .iter()
                .rev()
                .find_map(|slot| slot.call_element)
                .unwrap_or(0) as usize;
            self.pop_n_emit(argc + 1);
            return;
        }
        let info = info.unwrap();

        // 实参若来自 Move 堆绑定，所有权转移给被调函数；调用者不再释放。
        let start = self.stack.len().saturating_sub(info.params as usize);
        let sources: Vec<usize> = self.stack[start..]
            .iter()
            .filter_map(|slot| slot.source_binding)
            .collect();
        for src in sources {
            self.heap_bindings.remove(&src);
        }

        // 主副本内的自递归调用改调别名副本；别名副本内的调用照常指向原函数。
        // 两者互为镜像，语义等价且不触发旧解释器的同名调用解析缺陷。
        let target = if !self.alias_body && self.current_fn == Some(fn_id) {
            self.aliases.get(&fn_id).copied().unwrap_or(fn_id)
        } else {
            fn_id
        };
        self.emit_call(target, info.params + 1);

        // MIR 栈效果：弹出 CallElement + 参数，压入返回值。
        let needed = info.params as usize + 1;
        if self.stack.len() < needed {
            self.emit_comment(&format!(
                "WARNING: call FnId({}) expects {} stack slots, model has {}",
                fn_id.0,
                needed,
                self.stack.len()
            ));
        }
        self.pop_n_model(needed);
        for _ in 0..info.returns {
            // 返回值可能是 Copy 类型（结构体/数组），保守不标记为可回收；
            // Vec 返回值在调用者 push/pop 时再标记。
            self.stack.push(Slot::unknown());
        }
    }

    fn emit_return(&mut self, argc: u32) {
        // 保留栈顶 argc 个值，丢弃其余所有槽。
        // 序列已验证：Depth Push(argc) Sub Push(0) Store
        //             Reverse Push(0) Load DropN Reverse
        self.raw_inst(1, '<'); // Depth
        self.emit_push_int(argc as i64);
        self.raw_inst(1, '*'); // Sub
        self.emit_push_int(0);
        self.raw_inst(0, '-'); // Store
        self.raw_inst(5, '*'); // Reverse
        self.emit_push_int(0);
        self.raw_inst(1, '-'); // Load
        self.raw_inst(1, '>'); // DropN
        self.raw_inst(5, '*'); // Reverse
        self.raw_inst(self.epilogue_label, '~'); // UncondJump

        let keep = (argc as usize).min(self.stack.len());
        let start = self.stack.len() - keep;
        self.stack = self.stack[start..].to_vec();
        for slot in &mut self.stack {
            slot.binding = None;
        }
        self.reachable = false;
    }

    // ─── 控制流汇合 ───

    fn record_branch_state(&mut self, label: usize) {
        let state = BranchState { stack: self.stack.clone(), scopes: self.scopes.clone() };
        if let Some(existing) = self.branch_states.get(&label).cloned() {
            self.branch_states.insert(label, merge_branch_states(existing, state));
        } else {
            self.branch_states.insert(label, state);
        }
    }

    fn handle_mark(&mut self, label: usize) {
        if let Some(state) = self.branch_states.get(&label).cloned() {
            if self.reachable {
                if state.stack.len() != self.stack.len() {
                    self.emit_comment(&format!(
                        "WARNING: merge at Mark({label}) has different stack heights ({} vs {}); using jump target state",
                        self.stack.len(),
                        state.stack.len()
                    ));
                }
                self.stack = merge_slots(&self.stack, &state.stack);
            } else {
                self.stack = state.stack;
            }
            self.scopes = state.scopes;
            self.reachable = true;
        }
        // Mark 本身必须始终输出：即使当前不可达，也可能有前向跳转引用它。
        self.raw_inst(label, '`');
    }

    // ─── 算术/比较/逻辑辅助 ───

    fn gen_binary_arith(&mut self, spaces: usize, op: BinOp) {
        self.raw_inst(spaces, '*');
        let rhs = self.stack.pop();
        let lhs = self.stack.pop();
        let constant = match (lhs.and_then(|s| s.constant), rhs.and_then(|s| s.constant)) {
            (Some(ConstValue::Int(a)), Some(ConstValue::Int(b))) => {
                let r = match op {
                    BinOp::Add => a.checked_add(b),
                    BinOp::Sub => a.checked_sub(b),
                    BinOp::Mul => a.checked_mul(b),
                    BinOp::Div => a.checked_div(b),
                    BinOp::Mod => a.checked_rem(b),
                };
                r.map(ConstValue::Int)
            }
            _ => None,
        };
        self.stack.push(Slot {
            constant,
            ..Slot::unknown()
        });
    }

    fn gen_binary_cmp(&mut self, spaces: usize, op: CmpOp) {
        self.raw_inst(spaces, '=');
        let rhs = self.stack.pop();
        let lhs = self.stack.pop();
        let constant = match (lhs.and_then(|s| s.constant), rhs.and_then(|s| s.constant)) {
            (Some(ConstValue::Int(a)), Some(ConstValue::Int(b))) => {
                let r = match op {
                    CmpOp::Eq => a == b,
                    CmpOp::Ne => a != b,
                    CmpOp::Lt => a < b,
                    CmpOp::Gt => a > b,
                    CmpOp::Le => a <= b,
                    CmpOp::Ge => a >= b,
                };
                Some(ConstValue::Int(if r { 1 } else { 0 }))
            }
            _ => None,
        };
        self.stack.push(Slot {
            constant,
            ..Slot::unknown()
        });
    }

    fn gen_binary_logic(&mut self, spaces: usize, op: LogicOp) {
        self.raw_inst(spaces, '&');
        let rhs = self.stack.pop();
        let lhs = self.stack.pop();
        let constant = match (lhs.and_then(|s| s.constant), rhs.and_then(|s| s.constant)) {
            (Some(ConstValue::Int(a)), Some(ConstValue::Int(b))) => {
                let r = match op {
                    LogicOp::And => a != 0 && b != 0,
                    LogicOp::Or => a != 0 || b != 0,
                    LogicOp::Xor => (a != 0) ^ (b != 0),
                };
                Some(ConstValue::Int(if r { 1 } else { 0 }))
            }
            _ => None,
        };
        self.stack.push(Slot {
            constant,
            ..Slot::unknown()
        });
    }

    // ─── 栈效果辅助 ───

    /// 从模型弹出 n 个槽，并为每个实际存在的槽生成 Pop。
    fn pop_n_emit(&mut self, n: usize) {
        for _ in 0..n {
            if self.stack.pop().is_some() {
                self.raw_inst(4, '+');
            } else {
                self.emit_comment("model stack underflow: skipping Pop on an unreachable path");
            }
        }
    }

    /// 只更新模型，不生成指令。
    fn pop_n_model(&mut self, n: usize) {
        for _ in 0..n {
            self.stack.pop();
        }
    }

    // ─── 堆运行时 ───
    //
    // 所有辅助序列只操作运行时栈和保留的堆临时槽，不修改编译期栈模型；
    // 唯一的例外是显式调用 `self.stack` 的地方。这样 MIR 指令之间的模型
    // 始终与运行时值栈一致，变量深度计算才不会漂移。

    fn fresh_helper_label(&mut self) -> usize {
        let label = self.next_helper_label;
        self.next_helper_label += 1;
        label
    }

    fn emit_dup(&mut self) {
        self.raw_inst(1, '+');
    }

    fn emit_swap(&mut self) {
        self.raw_inst(2, '+');
    }

    fn emit_pop(&mut self) {
        self.raw_inst(4, '+');
    }

    fn emit_add(&mut self) {
        self.raw_inst(0, '*');
    }

    fn emit_sub(&mut self) {
        self.raw_inst(1, '*');
    }

    fn emit_lt(&mut self) {
        self.raw_inst(2, '=');
    }

    fn emit_gt(&mut self) {
        self.raw_inst(3, '=');
    }

    fn emit_ge(&mut self) {
        self.raw_inst(5, '=');
    }

    /// 生成 `as T` 的运行时截断。Stardust 数值统一为 i64，
    /// 因此仅窄整型和 bool 需要显式处理；char/宽整型保持位模式不变。
    fn gen_cast(&mut self, target: TypeId) {
        match target {
            TypeId::I8 => self.emit_truncate_signed(8),
            TypeId::I16 => self.emit_truncate_signed(16),
            TypeId::I32 => self.emit_truncate_signed(32),
            TypeId::U8 => self.emit_truncate_unsigned(256),
            TypeId::U16 => self.emit_truncate_unsigned(65536),
            TypeId::U32 => self.emit_truncate_unsigned(1_i64 << 32),
            TypeId::Bool => {
                self.emit_push_int(0);
                self.raw_inst(1, '='); // Ne
            }
            TypeId::I64
            | TypeId::Isize
            | TypeId::U64
            | TypeId::Usize
            | TypeId::Char
            | TypeId::F32
            | TypeId::F64
            | TypeId::Unit
            | TypeId::Ref
            | TypeId::MutRef
            | TypeId::ElementId
            | TypeId::StackHandle
            | TypeId::FnPtr
            | TypeId::Unknown => {}
        }
    }

    /// `[v] -> [v mod m]`，并把负数规范到 `[0, m)`。
    fn emit_truncate_unsigned(&mut self, modulus: i64) {
        self.emit_push_int(modulus);
        self.raw_inst(4, '*'); // Mod
        self.emit_push_int(modulus);
        self.emit_add();
        self.emit_push_int(modulus);
        self.raw_inst(4, '*'); // Mod
    }

    /// `[v] -> [v as iN]`：先按无符号截断，再对超出符号位的部分减 2^N。
    fn emit_truncate_signed(&mut self, bits: u32) {
        let modulus = 1_i64 << bits;
        self.emit_truncate_unsigned(modulus);
        let done = self.fresh_helper_label();
        self.emit_dup();
        self.emit_push_int(modulus / 2);
        self.emit_lt();
        self.raw_inst(done, '\'');
        self.emit_push_int(modulus);
        self.emit_sub();
        self.raw_inst(done, '`');
    }

    /// 弹出栈顶并写入 `heap[slot]`。
    fn emit_store_scratch(&mut self, slot: usize) {
        self.emit_push_int(slot as i64);
        self.raw_inst(0, '-'); // Store
    }

    /// 读 `heap[slot]` 并压栈。
    fn emit_load_scratch(&mut self, slot: usize) {
        self.emit_push_int(slot as i64);
        self.raw_inst(1, '-'); // Load
    }

    /// 把 `read_line` 的字节计数设为 1（合法字符只追加一次）。
    fn emit_set_byte_count_one(&mut self) {
        self.emit_push_int(1);
        self.emit_store_scratch(HEAP_BYTE_COUNT);
    }

    /// 读取下一个输入字节：优先消费 `read_line` 的 pending 字节，否则 CharIn。
    /// 栈效果：压入一个字节。
    fn emit_read_byte_or_pending(&mut self) {
        let has_pending = self.fresh_helper_label();
        let got_byte = self.fresh_helper_label();
        self.emit_load_scratch(HEAP_PENDING); // [p]
        self.emit_dup(); // [p, p]
        self.emit_push_int(-1); // [p, p, -1]
        self.raw_inst(1, '='); // Ne: [p, cond]
        self.raw_inst(has_pending, '\''); // pop cond -> [p]
        self.emit_pop(); // []
        self.raw_inst(1, ','); // CharIn -> [b]
        self.raw_inst(got_byte, '~');
        self.raw_inst(has_pending, '`');
        // [p]：暂存 p，把 pending 清为 -1，再把 p 压回栈。
        self.emit_store_scratch(HEAP_VALUE); // [p] -> []
        self.emit_push_int(-1);
        self.emit_store_scratch(HEAP_PENDING);
        self.emit_load_scratch(HEAP_VALUE); // [p]
        self.raw_inst(got_byte, '`');
        // [b]
    }

    /// `[.., size]` → `[.., handle]`。优先从空闲链表复用块，否则推进 bump 指针。
    ///
    /// 空闲块布局为 `[len = -1, block_size, next, ...]`，`block_size` 含两个
    /// 头部槽；因此空闲块至少 3 个槽。普通块的 `cap` 仍是元素容量。
    fn emit_alloc_from_stack(&mut self) {
        self.emit_store_scratch(HEAP_SIZE);

        // 最小块大小 3（两个头部 + 空闲链表 next）。
        let size_ok = self.fresh_helper_label();
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_push_int(3);
        self.emit_ge();
        self.raw_inst(size_ok, '\'');
        self.emit_push_int(3);
        self.emit_store_scratch(HEAP_SIZE);
        self.raw_inst(size_ok, '`');

        let loop_label = self.fresh_helper_label();
        let found = self.fresh_helper_label();
        let bump = self.fresh_helper_label();
        let done = self.fresh_helper_label();

        self.emit_load_scratch(HEAP_FREE_LIST);
        self.emit_store_scratch(HEAP_ALLOC_CURSOR);

        self.raw_inst(loop_label, '`');
        self.emit_load_scratch(HEAP_ALLOC_CURSOR);
        self.emit_push_int(0);
        self.raw_inst(0, '='); // cursor == 0 -> bump
        self.raw_inst(bump, '\'');

        // heap[cursor + 1] 是空闲块大小（含头部）。
        self.emit_load_scratch(HEAP_ALLOC_CURSOR);
        self.emit_push_int(1);
        self.emit_add();
        self.raw_inst(1, '-');
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_ge();
        self.raw_inst(found, '\'');

        // next = heap[cursor + 2]
        self.emit_load_scratch(HEAP_ALLOC_CURSOR);
        self.emit_push_int(2);
        self.emit_add();
        self.raw_inst(1, '-');
        self.emit_store_scratch(HEAP_ALLOC_CURSOR);
        self.raw_inst(loop_label, '~');

        self.raw_inst(found, '`');
        // 从链表摘除：free_head = heap[cursor + 2]
        self.emit_load_scratch(HEAP_ALLOC_CURSOR);
        self.emit_push_int(2);
        self.emit_add();
        self.raw_inst(1, '-');
        self.emit_store_scratch(HEAP_FREE_LIST);
        self.emit_load_scratch(HEAP_ALLOC_CURSOR);
        self.raw_inst(done, '~');

        self.raw_inst(bump, '`');
        self.emit_load_scratch(HEAP_ALLOC_PTR);
        self.emit_dup();
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_add();
        self.emit_store_scratch(HEAP_ALLOC_PTR);

        self.raw_inst(done, '`');
    }

    /// 把 `heap[handle_slot]` 指向的块加入空闲链表（保守检查，重复释放安全）。
    ///
    /// 只回收块本身，不递归回收嵌套句柄；嵌套容器仍可能泄漏，但不会悬垂。
    fn emit_release_block(&mut self, handle_slot: usize) {
        let done = self.fresh_helper_label();

        // handle < HEAP_BASE -> 不是堆块。
        self.emit_load_scratch(handle_slot);
        self.emit_push_int(HEAP_BASE);
        self.emit_lt();
        self.raw_inst(done, '\'');

        // handle >= alloc_ptr -> 不是已分配块。
        self.emit_load_scratch(handle_slot);
        self.emit_load_scratch(HEAP_ALLOC_PTR);
        self.emit_ge();
        self.raw_inst(done, '\'');

        // heap[handle] == -1 -> 已经在空闲链表中。
        self.emit_load_scratch(handle_slot);
        self.raw_inst(1, '-');
        self.emit_push_int(-1);
        self.raw_inst(0, '=');
        self.raw_inst(done, '\'');

        // block_size = cap + BLOCK_HEADER。
        self.emit_load_scratch(handle_slot);
        self.emit_push_int(1);
        self.emit_add();
        self.raw_inst(1, '-');
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_store_scratch(HEAP_SIZE);

        // 太小、放不下 next 的块不回收。
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_push_int(3);
        self.emit_lt();
        self.raw_inst(done, '\'');

        // heap[handle] = -1
        self.emit_load_scratch(handle_slot);
        self.emit_push_int(-1);
        self.emit_swap();
        self.raw_inst(0, '-');
        // heap[handle + 1] = block_size
        self.emit_load_scratch(handle_slot);
        self.emit_push_int(1);
        self.emit_add();
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_swap();
        self.raw_inst(0, '-');
        // heap[handle + 2] = free_head
        self.emit_load_scratch(handle_slot);
        self.emit_push_int(2);
        self.emit_add();
        self.emit_load_scratch(HEAP_FREE_LIST);
        self.emit_swap();
        self.raw_inst(0, '-');
        // free_head = handle
        self.emit_load_scratch(handle_slot);
        self.emit_store_scratch(HEAP_FREE_LIST);

        self.raw_inst(done, '`');
    }

    fn emit_alloc_const(&mut self, size: usize) {
        self.emit_push_int(size as i64);
        self.emit_alloc_from_stack();
    }

    /// 把 `heap[handle_slot + offset]` 设为常量 `value`。
    fn emit_write_header_const(&mut self, handle_slot: usize, offset: i64, value: i64) {
        self.emit_load_scratch(handle_slot);
        self.emit_push_int(offset);
        self.emit_add();
        self.emit_push_int(value);
        self.emit_swap();
        self.raw_inst(0, '-'); // Store
    }

    /// 把 `heap[handle_slot + offset]` 设为 `heap[value_slot]`。
    fn emit_write_header_slot(&mut self, handle_slot: usize, offset: i64, value_slot: usize) {
        self.emit_load_scratch(handle_slot);
        self.emit_push_int(offset);
        self.emit_add();
        self.emit_load_scratch(value_slot);
        self.emit_swap();
        self.raw_inst(0, '-');
    }

    /// 把栈顶 `count` 个值合并为堆数组，压入句柄。
    fn gen_build_array(&mut self, count: usize) {
        // 空容器预留 4 个容量，使 free list 在循环中能复用同一块；
        // 非空容器保持精确容量，避免改变既有布局。
        let cap = if count == 0 { 4 } else { count };
        self.emit_alloc_const(BLOCK_HEADER as usize + cap);
        self.emit_store_scratch(HEAP_HANDLE);
        self.emit_write_header_const(HEAP_HANDLE, 0, count as i64);
        self.emit_write_header_const(HEAP_HANDLE, 1, cap as i64);
        // 栈顶是最后一个元素；倒序写入下标 count-1 .. 0。
        for i in (0..count).rev() {
            self.emit_load_scratch(HEAP_HANDLE);
            self.emit_push_int(BLOCK_HEADER + i as i64);
            self.emit_add();
            self.raw_inst(0, '-');
        }
        self.emit_load_scratch(HEAP_HANDLE);
    }

    /// 将编译期元组/字符串字面量复制到堆上，压入句柄。
    fn gen_push_tuple_literal(&mut self, elems: &[Value]) {
        let count = elems.len();
        let cap = if count == 0 { 4 } else { count };
        self.emit_alloc_const(BLOCK_HEADER as usize + cap);
        self.emit_store_scratch(HEAP_HANDLE);
        self.emit_write_header_const(HEAP_HANDLE, 0, count as i64);
        self.emit_write_header_const(HEAP_HANDLE, 1, cap as i64);
        for (i, elem) in elems.iter().enumerate() {
            self.emit_load_scratch(HEAP_HANDLE);
            self.emit_push_int(BLOCK_HEADER + i as i64);
            self.emit_add();
            if let Some(n) = elem.as_i64() {
                self.emit_push_int(n);
            } else {
                // 嵌套元组字面量目前没有可用的字段访问语法（`t.0.0` 不受支持）；
                // 触发陷阱而不是静默写入 0。
                self.emit_comment(&format!(
                    "nested tuple element `{}` has no supported representation",
                    elem.type_name()
                ));
                self.emit_bounds_trap();
            }
            self.emit_swap();
            self.raw_inst(0, '-');
        }
        self.emit_load_scratch(HEAP_HANDLE);
    }

    /// `[value]` → `[box_handle]`，Box 布局为 `[1, 1, value]`。
    fn gen_make_box(&mut self) {
        self.emit_store_scratch(HEAP_VALUE);
        self.emit_alloc_const(3);
        self.emit_store_scratch(HEAP_HANDLE);
        self.emit_write_header_const(HEAP_HANDLE, 0, 1);
        self.emit_write_header_const(HEAP_HANDLE, 1, 1);
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(2);
        self.emit_add();
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_swap();
        self.raw_inst(0, '-');
        self.emit_load_scratch(HEAP_HANDLE);
    }

    /// `[box_handle]` → `[heap[handle + 2]]`。
    fn gen_deref(&mut self) {
        self.emit_push_int(2);
        self.emit_add();
        self.raw_inst(1, '-');
    }

    /// `[handle]` → `[field]`（聚合值被消费；match 会先 Dup）。
    fn gen_field_access(&mut self, index: usize) {
        self.emit_push_int(BLOCK_HEADER + index as i64);
        self.emit_add();
        self.raw_inst(1, '-');
    }

    /// `[handle, index]` → `[element]`。
    fn gen_index_access(&mut self) {
        // 边界检查：index 必须在 [0, len) 内；越界触发除零陷阱，
        // 使 Stardust 解释器以非零状态退出（解释器没有自定义错误原语）。
        self.emit_store_scratch(HEAP_INDEX);
        self.emit_store_scratch(HEAP_HANDLE);
        let non_negative = self.fresh_helper_label();
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(0);
        self.emit_ge(); // index >= 0
        self.raw_inst(non_negative, '\'');
        self.emit_bounds_trap();
        self.raw_inst(non_negative, '`');

        let in_bounds = self.fresh_helper_label();
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_load_scratch(HEAP_HANDLE);
        self.raw_inst(1, '-'); // Load len（偏移 0）
        self.emit_lt(); // index < len
        self.raw_inst(in_bounds, '\'');
        self.emit_bounds_trap();
        self.raw_inst(in_bounds, '`');

        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_add();
        self.raw_inst(1, '-');
    }

    /// 越界陷阱：Stardust 解释器没有断言/自定义错误指令，用除零触发非零退出。
    fn emit_bounds_trap(&mut self) {
        self.emit_push_int(1);
        self.emit_push_int(0);
        self.raw_inst(3, '*'); // Div
    }

    /// 保留枚举值并压入标签（块下标 0，位于头部之后）。
    /// `IntToStr`：`[n]` → `[十进制字符串块]`。
    ///
    /// 块元素是 Unicode 码点（与字符串字面量、`print_str` 一致），负数前置 `-`。
    fn gen_int_to_str(&mut self) {
        let negative = self.fresh_helper_label();
        let after_sign = self.fresh_helper_label();
        let count_loop = self.fresh_helper_label();
        let count_body = self.fresh_helper_label();
        let count_done = self.fresh_helper_label();
        let write_loop = self.fresh_helper_label();
        let write_body = self.fresh_helper_label();
        let write_done = self.fresh_helper_label();
        let sign_block = self.fresh_helper_label();
        let end = self.fresh_helper_label();

        self.emit_store_scratch(HEAP_VALUE); // n
        self.emit_push_int(0);
        self.emit_store_scratch(HEAP_BYTE_COUNT); // sign
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_push_int(0);
        self.emit_lt();
        self.raw_inst(negative, '\'');
        self.raw_inst(after_sign, '~');
        self.raw_inst(negative, '`');
        self.emit_push_int(1);
        self.emit_store_scratch(HEAP_BYTE_COUNT);
        self.emit_push_int(0);
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_sub();
        self.emit_store_scratch(HEAP_VALUE);
        self.raw_inst(after_sign, '`');

        // 统计位数：count = 1，tmp = |n|
        self.emit_push_int(1);
        self.emit_store_scratch(HEAP_COUNTER);
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_store_scratch(HEAP_INDEX);
        self.raw_inst(count_loop, '`');
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(10);
        self.emit_ge();
        self.raw_inst(count_body, '\'');
        self.raw_inst(count_done, '~');
        self.raw_inst(count_body, '`');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(1);
        self.emit_add();
        self.emit_store_scratch(HEAP_COUNTER);
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(10);
        self.raw_inst(3, '*'); // Div
        self.emit_store_scratch(HEAP_INDEX);
        self.raw_inst(count_loop, '~');
        self.raw_inst(count_done, '`');

        // 分配块：len = count + sign
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_load_scratch(HEAP_BYTE_COUNT);
        self.emit_add();
        self.emit_store_scratch(HEAP_CAP);
        self.emit_load_scratch(HEAP_CAP);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_alloc_from_stack();
        self.emit_store_scratch(HEAP_HANDLE);
        self.emit_write_header_slot(HEAP_HANDLE, 0, HEAP_CAP);
        self.emit_write_header_slot(HEAP_HANDLE, 1, HEAP_CAP);

        // 从末位向前写数字。
        self.emit_load_scratch(HEAP_CAP);
        self.emit_push_int(1);
        self.emit_sub();
        self.emit_store_scratch(HEAP_INDEX);
        self.raw_inst(write_loop, '`');
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_load_scratch(HEAP_BYTE_COUNT);
        self.emit_ge();
        self.raw_inst(write_body, '\'');
        self.raw_inst(write_done, '~');
        self.raw_inst(write_body, '`');
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_push_int(10);
        self.raw_inst(4, '*'); // Mod
        self.emit_push_int(48); // '0'
        self.emit_add();
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_add();
        self.raw_inst(0, '-'); // Store
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_push_int(10);
        self.raw_inst(3, '*'); // Div
        self.emit_store_scratch(HEAP_VALUE);
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(1);
        self.emit_sub();
        self.emit_store_scratch(HEAP_INDEX);
        self.raw_inst(write_loop, '~');
        self.raw_inst(write_done, '`');

        // 负号
        self.emit_load_scratch(HEAP_BYTE_COUNT);
        self.emit_push_int(1);
        self.raw_inst(0, '='); // Eq
        self.raw_inst(sign_block, '\'');
        self.raw_inst(end, '~');
        self.raw_inst(sign_block, '`');
        self.emit_push_int(45); // '-'
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.raw_inst(0, '-');
        self.raw_inst(end, '`');
        self.emit_load_scratch(HEAP_HANDLE);
    }

    /// 构造单槽枚举块 `[tag]` 并压入句柄。
    fn gen_push_empty_enum(&mut self, tag: i64) {
        self.emit_alloc_const(BLOCK_HEADER as usize + 1);
        self.emit_store_scratch(HEAP_COUNTER);
        self.emit_write_header_const(HEAP_COUNTER, 0, 1);
        self.emit_write_header_const(HEAP_COUNTER, 1, 1);
        self.emit_push_int(tag);
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.raw_inst(0, '-');
        self.emit_load_scratch(HEAP_COUNTER);
    }

    /// 构造 `[tag, payload]` 双槽枚举块；payload 取自栈顶。
    fn gen_push_enum_with_payload(&mut self, tag: i64) {
        self.emit_store_scratch(HEAP_VALUE);
        self.emit_alloc_const(BLOCK_HEADER as usize + 2);
        self.emit_store_scratch(HEAP_COUNTER);
        self.emit_write_header_const(HEAP_COUNTER, 0, 2);
        self.emit_write_header_const(HEAP_COUNTER, 1, 2);
        self.emit_push_int(tag);
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.raw_inst(0, '-');
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(BLOCK_HEADER + 1);
        self.emit_add();
        self.raw_inst(0, '-');
        self.emit_load_scratch(HEAP_COUNTER);
    }

    /// `StrToInt`：`[字符串块]` → `[Option<i64>]`。
    ///
    /// 块布局为 `[len, cap, chars...]`：长度在句柄 +0（可直接 `Load`），
    /// 字符从句柄 +`BLOCK_HEADER` 起。可选 `-` 前缀；非数字字符 → `None`。
    fn gen_str_to_int(&mut self, some_tag: i64, none_tag: i64) {
        let empty = self.fresh_helper_label();
        let start = self.fresh_helper_label();
        let neg = self.fresh_helper_label();
        let after_neg = self.fresh_helper_label();
        let loop_start = self.fresh_helper_label();
        let body = self.fresh_helper_label();
        let bad_low = self.fresh_helper_label();
        let bad_high = self.fresh_helper_label();
        let digits_done = self.fresh_helper_label();
        let ok_path = self.fresh_helper_label();
        let none_path = self.fresh_helper_label();
        let done = self.fresh_helper_label();

        self.emit_store_scratch(HEAP_HANDLE);
        // len 在块头 +0。
        self.emit_load_scratch(HEAP_HANDLE);
        self.raw_inst(1, '-');
        self.emit_store_scratch(HEAP_CAP);

        // 空串 → None
        self.emit_load_scratch(HEAP_CAP);
        self.emit_push_int(0);
        self.raw_inst(0, '=');
        self.raw_inst(empty, '\'');
        self.raw_inst(start, '~');
        self.raw_inst(empty, '`');
        self.gen_push_empty_enum(none_tag);
        self.raw_inst(done, '~');
        self.raw_inst(start, '`');

        // idx = 0；value = 0；sign = 1；first_digit = 0
        self.emit_push_int(0);
        self.emit_store_scratch(HEAP_INDEX);
        self.emit_push_int(0);
        self.emit_store_scratch(HEAP_VALUE);
        self.emit_push_int(1);
        self.emit_store_scratch(HEAP_BYTE_COUNT);
        self.emit_push_int(0);
        self.emit_store_scratch(HEAP_SIZE);

        // 首位 `-`：idx = first_digit = 1，sign = -1
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.raw_inst(1, '-');
        self.emit_push_int(45);
        self.raw_inst(0, '=');
        self.raw_inst(neg, '\'');
        self.raw_inst(after_neg, '~');
        self.raw_inst(neg, '`');
        self.emit_push_int(1);
        self.emit_store_scratch(HEAP_INDEX);
        self.emit_push_int(1);
        self.emit_store_scratch(HEAP_SIZE);
        self.emit_push_int(0);
        self.emit_push_int(1);
        self.emit_sub();
        self.emit_store_scratch(HEAP_BYTE_COUNT);
        self.raw_inst(after_neg, '`');

        self.raw_inst(loop_start, '`');
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_load_scratch(HEAP_CAP);
        self.emit_lt();
        self.raw_inst(body, '\'');
        self.raw_inst(digits_done, '~');
        self.raw_inst(body, '`');
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_add();
        self.raw_inst(1, '-'); // ch
        self.emit_dup();
        self.emit_push_int(48);
        self.emit_lt();
        self.raw_inst(bad_low, '\'');
        self.emit_dup();
        self.emit_push_int(57);
        self.emit_swap();
        self.emit_lt(); // 57 < ch，等价于 ch > '9'
        self.raw_inst(bad_high, '\'');
        self.emit_push_int(48);
        self.emit_sub();
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_push_int(10);
        self.raw_inst(2, '*'); // Mul
        self.emit_add();
        self.emit_store_scratch(HEAP_VALUE);
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(1);
        self.emit_add();
        self.emit_store_scratch(HEAP_INDEX);
        self.raw_inst(loop_start, '~');
        // 非法字符：丢弃 ch，累积值作废（下标设为长度以结束循环）。
        // 非法字符：丢弃 ch，令 first_digit = idx = len（判定 `first < idx` 失败并退出循环）。
        self.raw_inst(bad_low, '`');
        self.emit_pop();
        self.emit_load_scratch(HEAP_CAP);
        self.emit_store_scratch(HEAP_SIZE);
        self.emit_load_scratch(HEAP_CAP);
        self.emit_store_scratch(HEAP_INDEX);
        self.raw_inst(loop_start, '~');
        self.raw_inst(bad_high, '`');
        self.emit_pop();
        self.emit_load_scratch(HEAP_CAP);
        self.emit_store_scratch(HEAP_SIZE);
        self.emit_load_scratch(HEAP_CAP);
        self.emit_store_scratch(HEAP_INDEX);
        self.raw_inst(loop_start, '~');
        self.raw_inst(digits_done, '`');

        // 有数字 ⇔ first_digit >= 0 且 idx > first_digit
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_swap(); // [first, idx]
        self.emit_lt(); // first < idx
        self.raw_inst(ok_path, '\'');
        self.raw_inst(none_path, '~');
        self.raw_inst(ok_path, '`');
        self.emit_load_scratch(HEAP_BYTE_COUNT);
        self.emit_load_scratch(HEAP_VALUE);
        self.raw_inst(2, '*'); // sign * value
        self.gen_push_enum_with_payload(some_tag);
        self.raw_inst(done, '~');
        self.raw_inst(none_path, '`');
        self.gen_push_empty_enum(none_tag);
        self.raw_inst(done, '`');
    }

    fn gen_enum_tag(&mut self) {
        self.emit_dup();
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.raw_inst(1, '-');
    }

    /// `[handle]` → `[len]`。
    fn gen_container_len(&mut self) {
        self.raw_inst(1, '-');
    }

    /// `MakeArrayRepeat`：`[value, count]` → `[handle]`。
    fn gen_make_array_repeat(&mut self) {
        // count 存 HEAP_INDEX：emit_alloc_from_stack 会覆盖 HEAP_SIZE。
        self.emit_store_scratch(HEAP_INDEX);
        self.emit_store_scratch(HEAP_VALUE);
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_alloc_from_stack();
        self.emit_store_scratch(HEAP_HANDLE);
        self.emit_write_header_slot(HEAP_HANDLE, 0, HEAP_INDEX);
        self.emit_write_header_slot(HEAP_HANDLE, 1, HEAP_INDEX);

        let loop_label = self.fresh_helper_label();
        let end_label = self.fresh_helper_label();
        self.emit_push_int(0);
        self.emit_store_scratch(HEAP_COUNTER);
        self.raw_inst(loop_label, '`');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_ge();
        self.raw_inst(end_label, '\'');
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_add();
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_swap();
        self.raw_inst(0, '-');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(1);
        self.emit_add();
        self.emit_store_scratch(HEAP_COUNTER);
        self.raw_inst(loop_label, '~');
        self.raw_inst(end_label, '`');
        self.emit_load_scratch(HEAP_HANDLE);
    }

    /// 复制 `heap[src_slot]` 指向的块到新分配地址，句柄写入 `heap[dst_slot]`。
    ///
    /// 只做浅拷贝：嵌套聚合值仍是共享句柄。Copy 类型（数组/结构体）依赖它
    /// 实现写时复制，避免别名变量看到修改。
    fn gen_clone_block(&mut self, src_slot: usize, dst_slot: usize) {
        // size = len + BLOCK_HEADER
        self.emit_load_scratch(src_slot);
        self.raw_inst(1, '-'); // len
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_alloc_from_stack();
        self.emit_store_scratch(dst_slot);

        // 写入 [len, len]。
        self.emit_load_scratch(src_slot);
        self.raw_inst(1, '-');
        self.emit_store_scratch(HEAP_SIZE);
        self.emit_write_header_slot(dst_slot, 0, HEAP_SIZE);
        self.emit_write_header_slot(dst_slot, 1, HEAP_SIZE);

        // 逐元素复制。
        let loop_label = self.fresh_helper_label();
        let end_label = self.fresh_helper_label();
        self.emit_push_int(0);
        self.emit_store_scratch(HEAP_COUNTER);
        self.raw_inst(loop_label, '`');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_ge();
        self.raw_inst(end_label, '\'');
        self.emit_load_scratch(dst_slot);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_add();
        self.emit_load_scratch(src_slot);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_add();
        self.raw_inst(1, '-'); // src elem
        self.emit_swap();
        self.raw_inst(0, '-');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(1);
        self.emit_add();
        self.emit_store_scratch(HEAP_COUNTER);
        self.raw_inst(loop_label, '~');
        self.raw_inst(end_label, '`');
    }

    /// `VecPush`：优先原地追加；容量不足时按 2 倍扩容并重绑变量。
    ///
    /// `Vec` 是 Move 类型（借用检查拒绝别名 push），因此变量独占数据块，
    /// 原地写入不会影响其他变量。扩容时旧块可以安全回收。
    fn gen_vec_push(&mut self, name: &str) {
        if let Some(binding) = self.find_binding(name)
            && !self.indirect_bindings.contains(&binding)
        {
            self.heap_bindings.insert(binding);
        }
        self.gen_read_var(name);
        self.emit_store_scratch(HEAP_HANDLE);
        self.stack.pop(); // 读变量产生的临时句柄槽

        self.emit_load_scratch(HEAP_HANDLE);
        self.raw_inst(1, '-'); // len
        self.emit_store_scratch(HEAP_INDEX);
        self.emit_store_scratch(HEAP_VALUE); // 弹出原值
        self.stack.pop(); // 原值被消费

        // cap = heap[handle + 1]
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(1);
        self.emit_add();
        self.raw_inst(1, '-');
        self.emit_store_scratch(HEAP_SIZE);

        let in_place = self.fresh_helper_label();
        let done = self.fresh_helper_label();

        // len < cap -> 原地追加。
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_lt();
        self.raw_inst(in_place, '\'');

        // ─── 扩容：new_cap = max(4, cap * 2) ───
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_push_int(2);
        self.raw_inst(2, '*'); // Mul
        self.emit_store_scratch(HEAP_CAP);
        let cap_ok = self.fresh_helper_label();
        self.emit_load_scratch(HEAP_CAP);
        self.emit_push_int(4);
        self.emit_ge();
        self.raw_inst(cap_ok, '\'');
        self.emit_push_int(4);
        self.emit_store_scratch(HEAP_CAP);
        self.raw_inst(cap_ok, '`');

        self.emit_load_scratch(HEAP_CAP);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_alloc_from_stack();
        self.emit_store_scratch(HEAP_NEW_HANDLE);
        self.emit_write_header_slot(HEAP_NEW_HANDLE, 0, HEAP_INDEX);
        self.emit_write_header_slot(HEAP_NEW_HANDLE, 1, HEAP_CAP);

        // 复制旧元素。
        let loop_label = self.fresh_helper_label();
        let end_label = self.fresh_helper_label();
        self.emit_push_int(0);
        self.emit_store_scratch(HEAP_COUNTER);
        self.raw_inst(loop_label, '`');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_ge();
        self.raw_inst(end_label, '\'');
        self.emit_load_scratch(HEAP_NEW_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_add();
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_add();
        self.raw_inst(1, '-');
        self.emit_swap();
        self.raw_inst(0, '-');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(1);
        self.emit_add();
        self.emit_store_scratch(HEAP_COUNTER);
        self.raw_inst(loop_label, '~');
        self.raw_inst(end_label, '`');

        // 追加新值到下标 len。
        self.emit_load_scratch(HEAP_NEW_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_add();
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_swap();
        self.raw_inst(0, '-');

        // len + 1 写回新块。
        self.emit_load_scratch(HEAP_NEW_HANDLE);
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(1);
        self.emit_add();
        self.emit_swap();
        self.raw_inst(0, '-');

        // 用新句柄替换变量槽。
        self.emit_load_scratch(HEAP_NEW_HANDLE);
        self.stack.push(Slot::unknown());
        self.gen_write_var(name);

        // 旧块已不再被这个 Vec 引用（Move 类型保证无别名），回收复用。
        self.emit_release_block(HEAP_HANDLE);
        self.raw_inst(done, '~');

        // ─── 原地追加 ───
        self.raw_inst(in_place, '`');
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_add();
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_swap();
        self.raw_inst(0, '-');
        // len + 1 写回原块。
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(1);
        self.emit_add();
        self.emit_swap();
        self.raw_inst(0, '-');

        self.raw_inst(done, '`');
    }

    /// `VecPop`：移除最后一个元素并压栈；空容器压入 0（Unit 占位）。
    fn gen_vec_pop(&mut self, name: &str) {
        if let Some(binding) = self.find_binding(name)
            && !self.indirect_bindings.contains(&binding)
        {
            self.heap_bindings.insert(binding);
        }
        self.gen_read_var(name);
        self.emit_store_scratch(HEAP_HANDLE);
        self.stack.pop();

        // Vec 是 Move 类型，变量独占块；原地出栈，无需克隆。
        self.emit_load_scratch(HEAP_HANDLE);
        self.raw_inst(1, '-'); // len
        self.emit_dup();
        self.emit_push_int(0);
        self.emit_gt();

        let nonempty_label = self.fresh_helper_label();
        let end_label = self.fresh_helper_label();
        self.raw_inst(nonempty_label, '\'');
        // 空容器：丢弃 len，压入 0。
        self.emit_pop();
        self.emit_push_int(0);
        self.raw_inst(end_label, '~');

        self.raw_inst(nonempty_label, '`');
        self.emit_push_int(1);
        self.emit_sub(); // len - 1
        self.emit_store_scratch(HEAP_INDEX);
        // heap[handle] = len - 1
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_swap();
        self.raw_inst(0, '-');
        // 读取 heap[handle + BLOCK_HEADER + len - 1]
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_add();
        self.raw_inst(1, '-');
        self.raw_inst(end_label, '`');
    }

    /// `[index, value]` + 变量名 → 数组元素写入（copy-on-write）。
    fn gen_index_store(&mut self, name: &str) {
        // 先让模型与运行时同步（这两个值会被写入堆临时槽），
        // 这样后面的 ReadVar 深度才正确。
        self.stack.pop();
        self.stack.pop();
        self.emit_store_scratch(HEAP_VALUE);
        self.emit_store_scratch(HEAP_INDEX);
        self.gen_read_var(name);
        self.emit_store_scratch(HEAP_HANDLE);
        self.stack.pop();

        // 边界检查：越界写入同样触发除零陷阱。
        let non_negative = self.fresh_helper_label();
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(0);
        self.emit_ge(); // index >= 0
        self.raw_inst(non_negative, '\'');
        self.emit_bounds_trap();
        self.raw_inst(non_negative, '`');

        let in_bounds = self.fresh_helper_label();
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_load_scratch(HEAP_HANDLE);
        self.raw_inst(1, '-'); // Load len（偏移 0）
        self.emit_lt(); // index < len
        self.raw_inst(in_bounds, '\'');
        self.emit_bounds_trap();
        self.raw_inst(in_bounds, '`');

        // 数组是 Copy 类型，可能被别名共享；重绑到修改后的副本。
        self.gen_clone_block(HEAP_HANDLE, HEAP_NEW_HANDLE);
        self.emit_load_scratch(HEAP_NEW_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_add();
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_swap();
        self.raw_inst(0, '-');

        self.emit_load_scratch(HEAP_NEW_HANDLE);
        self.stack.push(Slot::unknown());
        self.gen_write_var(name);
    }

    /// `[value]` + 变量名 → 结构体字段写入（copy-on-write）。
    fn gen_field_store(&mut self, name: &str, index: usize) {
        self.stack.pop();
        self.emit_store_scratch(HEAP_VALUE);
        self.gen_read_var(name);
        self.emit_store_scratch(HEAP_HANDLE);
        self.stack.pop();

        // 结构体是 Copy 类型，可能被别名共享；重绑到修改后的副本。
        self.gen_clone_block(HEAP_HANDLE, HEAP_NEW_HANDLE);
        self.emit_load_scratch(HEAP_NEW_HANDLE);
        self.emit_push_int(BLOCK_HEADER + index as i64);
        self.emit_add();
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_swap();
        self.raw_inst(0, '-');

        self.emit_load_scratch(HEAP_NEW_HANDLE);
        self.stack.push(Slot::unknown());
        self.gen_write_var(name);
    }

    /// `&mut self` 方法内 `self.field = v`。
    ///
    /// Stardust 的引用是堆单元（cell）：变量槽持有 cell 句柄，被引用值在
    /// `cell + BLOCK_HEADER`。这里克隆目标块、写字段，再把新句柄放回 cell，
    /// 使调用方通过原变量能看到更新（与 VM 的引用写回语义一致）。
    fn gen_field_store_ref(&mut self, name: &str, index: usize) {
        self.stack.pop();
        self.emit_store_scratch(HEAP_VALUE); // 新值
        self.gen_read_var(name); // cell 句柄
        self.emit_store_scratch(HEAP_HANDLE);
        self.stack.pop();

        // cell 里的目标块句柄
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.raw_inst(1, '-');
        self.emit_store_scratch(HEAP_NEW_HANDLE);

        // 克隆后写字段（结构体是 Copy 类型）。
        self.gen_clone_block(HEAP_NEW_HANDLE, HEAP_INDEX);
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(BLOCK_HEADER + index as i64);
        self.emit_add();
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_swap();
        self.raw_inst(0, '-');

        // 把新块句柄写回 cell 的 value 槽。
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.raw_inst(0, '-');
    }

    /// `[reference, value]` → 写入 Box 单元（`heap[handle+2]`）。
    fn gen_store_deref(&mut self) {
        self.stack.pop();
        self.stack.pop();
        self.emit_store_scratch(HEAP_VALUE);
        self.emit_push_int(2);
        self.emit_add();
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_swap();
        self.raw_inst(0, '-');
    }

    /// 运行时字符串输出：按堆块长度逐字符 CharOut。
    fn gen_str_out_runtime(&mut self, newline: bool) {
        self.emit_store_scratch(HEAP_HANDLE);
        self.emit_push_int(0);
        self.emit_store_scratch(HEAP_COUNTER);
        let loop_label = self.fresh_helper_label();
        let end_label = self.fresh_helper_label();
        self.raw_inst(loop_label, '`');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_load_scratch(HEAP_HANDLE);
        self.raw_inst(1, '-'); // len
        self.emit_ge();
        self.raw_inst(end_label, '\'');
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_add();
        self.raw_inst(1, '-');
        self.raw_inst(0, ','); // CharOut
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(1);
        self.emit_add();
        self.emit_store_scratch(HEAP_COUNTER);
        self.raw_inst(loop_label, '~');
        self.raw_inst(end_label, '`');
        if newline {
            self.emit_push_int(10);
            self.raw_inst(0, ',');
        }
    }

    /// `[path_handle]` → 文件内容堆块句柄。
    fn gen_file_read(&mut self) {
        self.emit_store_scratch(HEAP_HANDLE);
        self.gen_push_block_chars(HEAP_HANDLE);
        self.raw_inst(2, ','); // FileRead: [path chars..., len] -> [contents chars..., len]
        self.gen_build_array_from_stack();
    }

    /// `[path_handle, data_handle]` → 写入文件（消费两个句柄）。
    fn gen_file_write(&mut self) {
        self.emit_store_scratch(HEAP_VALUE); // data
        self.emit_store_scratch(HEAP_HANDLE); // path
        self.gen_push_block_chars(HEAP_HANDLE);
        self.gen_push_block_chars(HEAP_VALUE);
        self.raw_inst(3, ','); // FileWrite
    }

    /// 把 `heap[handle_slot]` 指向的字符串块按顺序压栈，最后压入字符数。
    ///
    /// 运行时栈效果：`[] -> [chars..., len]`。只使用 `HEAP_SIZE` / `HEAP_COUNTER`
    /// 两个临时槽，`handle_slot` 本身不会被覆盖。
    fn gen_push_block_chars(&mut self, handle_slot: usize) {
        // len → HEAP_SIZE
        self.emit_load_scratch(handle_slot);
        self.raw_inst(1, '-'); // Load len
        self.emit_store_scratch(HEAP_SIZE);

        let loop_label = self.fresh_helper_label();
        let end_label = self.fresh_helper_label();
        self.emit_push_int(0);
        self.emit_store_scratch(HEAP_COUNTER);
        self.raw_inst(loop_label, '`');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_ge();
        self.raw_inst(end_label, '\'');
        // 压入 heap[handle + BLOCK_HEADER + i]
        self.emit_load_scratch(handle_slot);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_add();
        self.raw_inst(1, '-'); // Load
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(1);
        self.emit_add();
        self.emit_store_scratch(HEAP_COUNTER);
        self.raw_inst(loop_label, '~');
        self.raw_inst(end_label, '`');
        self.emit_load_scratch(HEAP_SIZE);
    }

    /// `[chars..., len]` → `[handle]`，按运行时长度构造字符串堆块。
    fn gen_build_array_from_stack(&mut self) {
        // 长度先存 HEAP_INDEX；emit_alloc_from_stack 会覆盖 HEAP_SIZE。
        self.emit_store_scratch(HEAP_INDEX); // len
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_alloc_from_stack();
        self.emit_store_scratch(HEAP_HANDLE);
        self.emit_write_header_slot(HEAP_HANDLE, 0, HEAP_INDEX);
        self.emit_write_header_slot(HEAP_HANDLE, 1, HEAP_INDEX);

        // 栈顶是最后一个字符：从尾部向前填块，逐次弹出。
        let loop_label = self.fresh_helper_label();
        let end_label = self.fresh_helper_label();
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(1);
        self.emit_sub();
        self.emit_store_scratch(HEAP_COUNTER);
        self.raw_inst(loop_label, '`');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(0);
        self.emit_lt();
        self.raw_inst(end_label, '\'');
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_add();
        self.raw_inst(0, '-'); // Store: 弹出地址，再弹出栈顶字符
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(1);
        self.emit_sub();
        self.emit_store_scratch(HEAP_COUNTER);
        self.raw_inst(loop_label, '~');
        self.raw_inst(end_label, '`');
        self.emit_load_scratch(HEAP_HANDLE);
    }

    /// `ReadLine`：用 CharIn 逐字节读取一行，并按 UTF-8 解码成字符序列。
    ///
    /// 块中保存的是 Unicode 码点（与字符串字面量一致），因此 `print_str`
    /// 可以直接 CharOut。块从 16 个字符开始按需翻倍，行尾的 `\r` 会在
    /// 读到 `\n` 后统一去掉（与 VM 的 trim 行为一致）。
    ///
    /// 解码与 VM 的 `String::from_utf8_lossy` 对齐：非法首字节、非法续字节
    /// 都替换为 U+FFFD；非法续字节会通过 `HEAP_PENDING` 留到下一轮，
    /// 因此不会吞掉后续字符。输入 EOF 仍无法检测（CharIn 会报 I/O 错误），
    /// 该差异记录在 `USAGE.md` 中。
    fn gen_read_line(&mut self) {
        const INITIAL_CAP: i64 = 16;
        self.emit_alloc_const(BLOCK_HEADER as usize + INITIAL_CAP as usize);
        self.emit_store_scratch(HEAP_HANDLE);
        self.emit_write_header_const(HEAP_HANDLE, 0, 0);
        self.emit_write_header_const(HEAP_HANDLE, 1, INITIAL_CAP);
        self.emit_push_int(0);
        self.emit_store_scratch(HEAP_COUNTER);
        self.emit_push_int(-1);
        self.emit_store_scratch(HEAP_PENDING);

        let read_loop = self.fresh_helper_label();
        let grow = self.fresh_helper_label();
        let grow_copy = self.fresh_helper_label();
        let grow_copy_end = self.fresh_helper_label();
        let newline_end = self.fresh_helper_label();
        let trim_loop = self.fresh_helper_label();
        let trim_end = self.fresh_helper_label();
        let ascii = self.fresh_helper_label();
        let invalid = self.fresh_helper_label();
        let utf8_2 = self.fresh_helper_label();
        let utf8_3 = self.fresh_helper_label();
        let utf8_4 = self.fresh_helper_label();
        let cont_loop = self.fresh_helper_label();
        let cont_done = self.fresh_helper_label();
        let valid_cont = self.fresh_helper_label();
        let check2 = self.fresh_helper_label();
        let check3 = self.fresh_helper_label();
        let check4 = self.fresh_helper_label();
        let check_surrogate = self.fresh_helper_label();
        let range_bad = self.fresh_helper_label();
        let append = self.fresh_helper_label();

        self.raw_inst(read_loop, '`');
        // 容量检查放在读取之前，避免多消费一个输入字节。
        // 预留 4 个槽：非法多字节序列会逐字节追加 U+FFFD（最多 4 个）。
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(4);
        self.emit_add();
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(1);
        self.emit_add();
        self.raw_inst(1, '-'); // Load capacity
        self.emit_ge();
        self.raw_inst(grow, '\'');

        self.emit_read_byte_or_pending(); // [b0]
        self.emit_dup();
        self.emit_push_int(10); // '\n'
        self.raw_inst(0, '=');
        self.raw_inst(newline_end, '\'');

        // UTF-8 首字节分类。
        self.emit_dup();
        self.emit_push_int(128);
        self.raw_inst(2, '='); // Lt
        self.raw_inst(ascii, '\'');
        self.emit_dup();
        self.emit_push_int(192);
        self.raw_inst(2, '=');
        self.raw_inst(invalid, '\'');
        self.emit_dup();
        self.emit_push_int(224);
        self.raw_inst(2, '=');
        self.raw_inst(utf8_2, '\'');
        self.emit_dup();
        self.emit_push_int(240);
        self.raw_inst(2, '=');
        self.raw_inst(utf8_3, '\'');
        self.emit_dup();
        self.emit_push_int(248);
        self.raw_inst(2, '=');
        self.raw_inst(utf8_4, '\'');
        self.raw_inst(invalid, '~'); // 0xF8..0xFF 非法

        // ASCII：码点就是该字节。
        self.raw_inst(ascii, '`');
        self.emit_store_scratch(HEAP_INDEX);
        self.emit_push_int(1);
        self.emit_store_scratch(HEAP_BYTE_COUNT);
        self.raw_inst(append, '~');

        // 非法首字节：替换字符 U+FFFD。
        self.raw_inst(invalid, '`');
        self.emit_pop();
        self.emit_push_int(0xFFFD);
        self.emit_store_scratch(HEAP_INDEX);
        self.emit_push_int(1);
        self.emit_store_scratch(HEAP_BYTE_COUNT);
        self.raw_inst(append, '~');

        // 多字节首字节：先写入掩码后的高位。
        self.raw_inst(utf8_2, '`');
        self.emit_push_int(32); // 0x1F 掩码 = mod 32
        self.raw_inst(4, '*'); // Mod
        self.emit_store_scratch(HEAP_INDEX);
        self.emit_push_int(1);
        self.emit_store_scratch(HEAP_SIZE);
        self.emit_push_int(2);
        self.emit_store_scratch(HEAP_BYTE_COUNT);
        self.raw_inst(cont_loop, '~');

        self.raw_inst(utf8_3, '`');
        self.emit_push_int(16); // 0x0F 掩码 = mod 16
        self.raw_inst(4, '*'); // Mod
        self.emit_store_scratch(HEAP_INDEX);
        self.emit_push_int(2);
        self.emit_store_scratch(HEAP_SIZE);
        self.emit_push_int(3);
        self.emit_store_scratch(HEAP_BYTE_COUNT);
        self.raw_inst(cont_loop, '~');

        self.raw_inst(utf8_4, '`');
        self.emit_push_int(8); // 0x07 掩码 = mod 8
        self.raw_inst(4, '*'); // Mod
        self.emit_store_scratch(HEAP_INDEX);
        self.emit_push_int(3);
        self.emit_store_scratch(HEAP_SIZE);
        self.emit_push_int(4);
        self.emit_store_scratch(HEAP_BYTE_COUNT);
        self.raw_inst(cont_loop, '~');

        // 读后续字节：先校验 10xxxxxx，再累加 acc = acc * 64 + (byte % 64)。
        self.raw_inst(cont_loop, '`');
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_push_int(0);
        self.raw_inst(0, '='); // Eq: SIZE == 0 时结束
        self.raw_inst(cont_done, '\'');
        self.emit_read_byte_or_pending(); // [b]
        self.emit_dup();
        self.emit_push_int(64);
        self.raw_inst(3, '*'); // Div: [b, b / 64]
        self.emit_push_int(2);
        self.raw_inst(0, '='); // Eq: b / 64 == 2（即 10xxxxxx）
        self.raw_inst(valid_cont, '\''); // pop cond -> [b]

        // 非法续字节：把该字节留到下一轮，当前码点替换为 U+FFFD。
        self.emit_store_scratch(HEAP_PENDING); // [b] -> []
        self.emit_push_int(0xFFFD);
        self.emit_store_scratch(HEAP_INDEX);
        self.emit_push_int(1);
        self.emit_store_scratch(HEAP_BYTE_COUNT);
        self.raw_inst(cont_done, '~');

        self.raw_inst(valid_cont, '`');
        self.emit_push_int(64); // 0x3F 掩码 = mod 64
        self.raw_inst(4, '*'); // Mod
        self.emit_store_scratch(HEAP_VALUE);
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(64);
        self.raw_inst(2, '*'); // Mul
        self.emit_load_scratch(HEAP_VALUE);
        self.emit_add();
        self.emit_store_scratch(HEAP_INDEX);
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_push_int(1);
        self.emit_sub();
        self.emit_store_scratch(HEAP_SIZE);
        self.raw_inst(cont_loop, '~');

        self.raw_inst(cont_done, '`');

        // 校验码点范围：overlong、surrogate、> U+10FFFF 都替换为 U+FFFD。
        self.emit_load_scratch(HEAP_BYTE_COUNT);
        self.emit_push_int(2);
        self.raw_inst(0, '=');
        self.raw_inst(check2, '\'');
        self.emit_load_scratch(HEAP_BYTE_COUNT);
        self.emit_push_int(3);
        self.raw_inst(0, '=');
        self.raw_inst(check3, '\'');
        self.emit_load_scratch(HEAP_BYTE_COUNT);
        self.emit_push_int(4);
        self.raw_inst(0, '=');
        self.raw_inst(check4, '\'');
        self.raw_inst(append, '~');

        self.raw_inst(check2, '`');
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(0x80);
        self.raw_inst(2, '='); // Lt: 2 字节码点必须 >= 0x80
        self.raw_inst(range_bad, '\'');
        self.emit_set_byte_count_one();
        self.raw_inst(append, '~');

        self.raw_inst(check3, '`');
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(0x800);
        self.raw_inst(2, '='); // Lt: 3 字节码点必须 >= 0x800
        self.raw_inst(range_bad, '\'');
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(0xD800);
        self.emit_ge(); // INDEX >= 0xD800 时继续检查 surrogate 上界
        self.raw_inst(check_surrogate, '\'');
        self.emit_set_byte_count_one();
        self.raw_inst(append, '~');

        self.raw_inst(check_surrogate, '`');
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(0xE000);
        self.raw_inst(2, '='); // Lt: 0xD800..0xDFFF 非法
        self.raw_inst(range_bad, '\'');
        self.emit_set_byte_count_one();
        self.raw_inst(append, '~');

        self.raw_inst(check4, '`');
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(0x10000);
        self.raw_inst(2, '='); // Lt: 4 字节码点必须 >= 0x10000
        self.raw_inst(range_bad, '\'');
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(0x110000);
        self.emit_ge(); // INDEX >= 0x110000 非法
        self.raw_inst(range_bad, '\'');
        self.emit_set_byte_count_one();
        self.raw_inst(append, '~');

        self.raw_inst(range_bad, '`');
        self.emit_push_int(0xFFFD);
        self.emit_store_scratch(HEAP_INDEX);
        self.raw_inst(append, '~');

        // 追加码点到块末尾；BYTE_COUNT 次（非法多字节序列逐字节替换）。
        self.raw_inst(append, '`');
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_add();
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_swap();
        self.raw_inst(0, '-');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(1);
        self.emit_add();
        self.emit_store_scratch(HEAP_COUNTER);
        self.emit_load_scratch(HEAP_BYTE_COUNT);
        self.emit_push_int(1);
        self.emit_sub();
        self.emit_store_scratch(HEAP_BYTE_COUNT);
        self.emit_load_scratch(HEAP_BYTE_COUNT);
        self.emit_push_int(0);
        self.raw_inst(0, '=');
        self.raw_inst(read_loop, '\'');
        self.raw_inst(append, '~');

        // 容量不足：分配两倍大小的新块并复制已有字符。
        self.raw_inst(grow, '`');
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(1);
        self.emit_add();
        self.raw_inst(1, '-'); // Load capacity
        self.emit_push_int(2);
        self.raw_inst(2, '*'); // Mul
        self.emit_store_scratch(HEAP_INDEX); // new capacity
        self.emit_load_scratch(HEAP_INDEX);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_alloc_from_stack();
        self.emit_store_scratch(HEAP_NEW_HANDLE);
        self.emit_write_header_slot(HEAP_NEW_HANDLE, 0, HEAP_COUNTER);
        self.emit_write_header_slot(HEAP_NEW_HANDLE, 1, HEAP_INDEX);

        // 逐个复制旧字符（HEAP_SIZE 复用为下标）。
        self.emit_push_int(0);
        self.emit_store_scratch(HEAP_SIZE);
        self.raw_inst(grow_copy, '`');
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_ge();
        self.raw_inst(grow_copy_end, '\'');
        self.emit_load_scratch(HEAP_NEW_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_add();
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_add();
        self.raw_inst(1, '-'); // Load old char
        self.emit_swap();
        self.raw_inst(0, '-'); // Store new char
        self.emit_load_scratch(HEAP_SIZE);
        self.emit_push_int(1);
        self.emit_add();
        self.emit_store_scratch(HEAP_SIZE);
        self.raw_inst(grow_copy, '~');
        self.raw_inst(grow_copy_end, '`');
        self.emit_load_scratch(HEAP_NEW_HANDLE);
        self.emit_store_scratch(HEAP_HANDLE);
        self.raw_inst(read_loop, '~');

        // 换行结束：丢弃 \n，再删除行尾所有 \r（CRLF 与 VM 对齐）。
        self.raw_inst(newline_end, '`');
        self.emit_pop();
        self.raw_inst(trim_loop, '`');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(0);
        self.raw_inst(0, '=');
        self.raw_inst(trim_end, '\'');
        self.emit_load_scratch(HEAP_HANDLE);
        self.emit_push_int(BLOCK_HEADER);
        self.emit_add();
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_add();
        self.emit_push_int(1);
        self.emit_sub();
        self.raw_inst(1, '-'); // Load last char
        self.emit_push_int(13); // '\r'
        self.raw_inst(1, '='); // Ne
        self.raw_inst(trim_end, '\'');
        self.emit_load_scratch(HEAP_COUNTER);
        self.emit_push_int(1);
        self.emit_sub();
        self.emit_store_scratch(HEAP_COUNTER);
        self.raw_inst(trim_loop, '~');
        self.raw_inst(trim_end, '`');

        // 写回长度并留下句柄。
        self.emit_write_header_slot(HEAP_HANDLE, 0, HEAP_COUNTER);
        self.emit_load_scratch(HEAP_HANDLE);
    }

    // ─── 原始编码输出 ───

    fn raw_inst(&mut self, spaces: usize, symbol: char) {
        for _ in 0..spaces {
            self.output.push(' ');
        }
        self.output.push(symbol);
        self.output.push('\n');
    }

    fn emit_comment(&mut self, text: &str) {
        if text.is_empty() {
            self.output.push_str("//\n");
            return;
        }
        for line in text.split('\n') {
            self.output.push_str("// ");
            self.output.push_str(line.trim_end_matches('\r'));
            self.output.push('\n');
        }
    }

    /// 压入一个整数常量。小的非负数直接编码；较大的数用十进制分解，
    /// 避免生成 `n+5` 个空格导致文件体积爆炸。
    fn emit_push_int(&mut self, n: i64) {
        if n >= 0 {
            self.emit_push_u64(n as u64);
        } else {
            // 0 - abs(n)
            self.raw_inst(5, '+');
            self.emit_push_u64(n.unsigned_abs());
            self.raw_inst(1, '*');
        }
    }

    fn emit_push_u64(&mut self, n: u64) {
        const DIRECT_LIMIT: u64 = 1000;
        if n <= DIRECT_LIMIT {
            self.raw_inst(n as usize + 5, '+');
            return;
        }
        let digits = n.to_string();
        let mut chars = digits.chars();
        let first = chars.next().and_then(|c| c.to_digit(10)).unwrap_or(0) as u64;
        self.emit_push_u64(first);
        for ch in chars {
            let d = ch.to_digit(10).unwrap_or(0) as u64;
            self.raw_inst(15, '+'); // Push(10)
            self.raw_inst(2, '*'); // Mul
            self.raw_inst(d as usize + 5, '+'); // Push(d)
            self.raw_inst(0, '*'); // Add
        }
    }

    fn emit_roll(&mut self, n: i32) {
        if n == 0 {
            return;
        }
        if n > 0 {
            let k = n as usize;
            for _ in 0..k {
                self.raw_inst(0, '>'); // ShiftR
            }
            self.raw_inst(5, '+'); // Push(0)
            self.raw_inst(0, '-'); // Store
            for _ in 0..k {
                self.raw_inst(0, '<'); // ShiftL
            }
            self.raw_inst(5, '+'); // Push(0)
            self.raw_inst(1, '-'); // Load
        } else {
            let k = (-n) as usize;
            self.raw_inst(5, '+'); // Push(0)
            self.raw_inst(0, '-'); // Store
            for _ in 0..k {
                self.raw_inst(0, '>'); // ShiftR
            }
            self.raw_inst(5, '+'); // Push(0)
            self.raw_inst(1, '-'); // Load
            for _ in 0..k {
                self.raw_inst(0, '<'); // ShiftL
            }
        }
    }

    fn emit_call(&mut self, fn_id: FnId, argc: u32) {
        self.raw_inst(fn_id.0 as usize, ':'); // (name):
        self.raw_inst(argc as usize, ';'); // (argc);
    }

    fn model_roll(&mut self, n: i32) {
        if n == 0 {
            return;
        }
        let len = self.stack.len();
        if n > 0 {
            let pos = n as usize;
            if pos < len {
                let item = self.stack.remove(len - 1 - pos);
                self.stack.push(item);
            } else {
                self.emit_comment(&format!("WARNING: Roll({n}) out of model bounds"));
            }
        } else {
            let pos = (-n) as usize;
            if pos < len {
                let item = self.stack.pop().expect("len > 0");
                let insert = self.stack.len() - pos;
                self.stack.insert(insert, item);
            } else {
                self.emit_comment(&format!("WARNING: Roll({n}) out of model bounds"));
            }
        }
    }
}

impl Default for StardustGen {
    fn default() -> Self {
        Self::new()
    }
}

// ─── 常量分类与汇合辅助 ───

fn classify_value(value: &Value) -> (Option<ConstValue>, Option<u32>) {
    if let Value::CallElement { argc } = value {
        return (Some(ConstValue::Int(*argc as i64)), Some(*argc));
    }
    if let Value::Tuple(elems) = value {
        let mut s = String::new();
        for elem in elems {
            if let Value::Char(code) = elem {
                if let Some(ch) = char::from_u32(*code) {
                    s.push(ch);
                } else {
                    return (None, None);
                }
            } else {
                return (None, None);
            }
        }
        return (Some(ConstValue::Str(s)), None);
    }
    if matches!(value, Value::Unit) {
        return (Some(ConstValue::Int(0)), None);
    }
    if let Some(n) = value.as_i64() {
        return (Some(ConstValue::Int(n)), None);
    }
    (None, None)
}

fn merge_branch_states(a: BranchState, b: BranchState) -> BranchState {
    if a.stack.len() != b.stack.len() {
        // 栈高度不一致时保留先记录的状态；代码生成阶段会再输出警告。
        return a;
    }
    BranchState { stack: merge_slots(&a.stack, &b.stack), scopes: b.scopes }
}

fn merge_slots(a: &[Slot], b: &[Slot]) -> Vec<Slot> {
    let len = a.len().min(b.len());
    let mut merged = Vec::with_capacity(len);
    for i in 0..len {
        let mut slot = b[i].clone();
        // 绑定/调用元素必须一致；常量不同则退化为未知。
        if a[i].binding != b[i].binding || a[i].call_element != b[i].call_element {
            slot.constant = None;
        } else if a[i].constant != b[i].constant {
            slot.constant = None;
        }
        merged.push(slot);
    }
    merged
}

#[derive(Debug, Clone, Copy)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Debug, Clone, Copy)]
enum CmpOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Debug, Clone, Copy)]
enum LogicOp {
    And,
    Or,
    Xor,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::mir::TypeId;

    fn info(params: u32, returns: u32) -> HashMap<FnId, FnInfo> {
        let mut map = HashMap::new();
        map.insert(FnId(1), FnInfo::new("main", params, returns));
        map
    }

    fn generate(mir: Vec<MirInst>) -> String {
        let mut generator = StardustGen::new();
        generator.generate_program(&[(FnId(1), mir)], &info(0, 0), Some(FnId(1)))
    }

    fn line_with_spaces(spaces: usize, symbol: char) -> String {
        format!("{}{}", " ".repeat(spaces), symbol)
    }

    #[test]
    fn test_push_encoding() {
        let code = generate(vec![MirInst::Push {
            value: Value::I64(3),
            ty: TypeId::I64,
        }]);
        assert!(code.contains(&format!("{}\n", line_with_spaces(8, '+'))));
    }

    #[test]
    fn test_negative_push_encoding() {
        let code = generate(vec![MirInst::Push {
            value: Value::I64(-3),
            ty: TypeId::I64,
        }]);
        // 0 - 3
        assert!(code.contains(&format!("{}\n", line_with_spaces(5, '+'))));
        assert!(code.contains(&format!("{}\n", line_with_spaces(8, '+'))));
        assert!(code.contains(&format!("{}\n", line_with_spaces(1, '*'))));
    }

    #[test]
    fn test_large_push_uses_decimal_decomposition() {
        let code = generate(vec![MirInst::Push {
            value: Value::I64(123_456),
            ty: TypeId::I64,
        }]);
        // 不应出现 12 万空格的单行 Push。
        let longest = code.lines().map(str::len).max().unwrap_or(0);
        assert!(longest < 1_000, "longest line was {longest}");
        assert!(code.contains(&format!("{}\n", line_with_spaces(15, '+')))); // Push(10)
    }

    #[test]
    fn test_variable_read_uses_depth_and_pick() {
        let mir = vec![
            MirInst::EnterScope,
            MirInst::Push { value: Value::I64(7), ty: TypeId::I64 },
            MirInst::BindVar { name: "x".into() },
            MirInst::ReadVar { name: "x".into() },
        ];
        let code = generate(mir);
        // 变量在栈顶（深度 0）：Push(0); Pick
        assert!(code.contains(&format!("{}\n", line_with_spaces(5, '+'))));
        assert!(code.contains(&format!("{}\n", line_with_spaces(2, '<'))));
    }

    #[test]
    fn test_roll_encoding_uses_shift_and_scratch_slot() {
        let code = generate(vec![MirInst::Roll { n: 2 }]);
        let count_exact = |symbol: &str| code.lines().filter(|line| *line == symbol).count();
        assert_eq!(count_exact(">"), 2); // ShiftR ×2
        assert_eq!(count_exact("<"), 2); // ShiftL ×2
        assert!(count_exact("-") >= 1); // Store
        assert!(count_exact(" -") >= 1); // Load
    }

    #[test]
    fn test_ret_keeps_top_values_and_jumps_to_epilogue() {
        let code = generate(vec![MirInst::Ret { argc: 1 }]);
        assert!(code.contains(&format!("{}\n", line_with_spaces(1, '<')))); // Depth
        assert!(code.contains(&format!("{}\n", line_with_spaces(5, '*')))); // Reverse
        assert!(code.contains(&format!("{}\n", line_with_spaces(1, '>')))); // DropN
        assert!(code.contains(&format!("{}\n", line_with_spaces(1, '~')))); // epilogue label 1
    }

    #[test]
    fn test_negative_roll_encoding() {
        let code = generate(vec![MirInst::Roll { n: -2 }]);
        let count_exact = |symbol: &str| code.lines().filter(|line| *line == symbol).count();
        assert_eq!(count_exact(">"), 2); // ShiftR ×2
        assert_eq!(count_exact("<"), 2); // ShiftL ×2
        assert!(count_exact("-") >= 1); // Store
        assert!(count_exact(" -") >= 1); // Load
    }

    #[test]
    fn test_str_out_constant_folds_to_chars() {
        let mir = vec![
            MirInst::Push {
                value: Value::Tuple(vec![Value::Char(b'h' as u32), Value::Char(b'i' as u32)]),
                ty: TypeId::Unknown,
            },
            MirInst::StrOut { newline: false },
        ];
        let code = generate(mir);
        assert!(code.contains(&format!("{}\n", line_with_spaces(0, ',')))); // CharOut
        assert!(code.contains(&format!("{}\n", line_with_spaces(5, '+')))); // Pop / Push(0) encoding
    }

    #[test]
    fn test_call_uses_function_id_and_argc() {
        let mut map = HashMap::new();
        map.insert(FnId(1), FnInfo::new("main", 0, 0));
        map.insert(FnId(2), FnInfo::new("add", 2, 1));
        let mir = vec![
            MirInst::Push { value: Value::CallElement { argc: 2 }, ty: TypeId::Unknown },
            MirInst::Push { value: Value::I64(1), ty: TypeId::I64 },
            MirInst::Push { value: Value::I64(2), ty: TypeId::I64 },
            MirInst::Call { fn_id: FnId(2), return_arity: 1 },
        ];
        let mut generator = StardustGen::new();
        let code = generator.generate_program(&[(FnId(1), mir)], &map, Some(FnId(1)));
        assert!(code.contains(&format!("{}\n", line_with_spaces(2, ':'))));
        assert!(code.contains(&format!("{}\n", line_with_spaces(3, ';'))));
    }

    #[test]
    fn test_self_recursion_uses_mutual_alias() {
        let mut map = HashMap::new();
        map.insert(FnId(1), FnInfo::new("main", 0, 0));
        map.insert(FnId(2), FnInfo::new("fact", 1, 1));

        let main_mir = vec![
            MirInst::Push { value: Value::CallElement { argc: 0 }, ty: TypeId::Unknown },
            MirInst::Call { fn_id: FnId(2), return_arity: 1 },
            MirInst::Pop { ty: TypeId::Unknown },
        ];
        let fact_mir = vec![
            MirInst::EnterScope,
            MirInst::BindVar { name: "n".into() },
            MirInst::Push { value: Value::CallElement { argc: 1 }, ty: TypeId::Unknown },
            MirInst::ReadVar { name: "n".into() },
            MirInst::Push { value: Value::I64(1), ty: TypeId::I64 },
            MirInst::Sub { ty: TypeId::I64 },
            MirInst::Call { fn_id: FnId(2), return_arity: 1 },
            MirInst::Ret { argc: 1 },
        ];
        let functions = vec![(FnId(1), main_mir), (FnId(2), fact_mir)];
        let mut generator = StardustGen::new();
        let code = generator.generate_program(&functions, &map, Some(FnId(1)));

        // 自递归函数生成两个副本，别名名从最大 FnId 之后开始（3）。
        assert!(code.contains("(mutual-recursion alias)"));
        // 主副本内的递归调用使用别名 FnId(3)，argc = 参数 1 + CallElement。
        assert!(code.contains("\n   :\n  ;\n"), "missing alias call:\n{code}");
        // 别名副本内的调用回到原函数 FnId(2)。
        assert!(code.contains("\n  :\n  ;\n"), "missing primary call:\n{code}");
    }

    #[test]
    fn test_generated_program_contains_unicode_mode() {
        let code = generate(vec![]);
        assert!(code.starts_with("#[char_mode=unicode]\n"));
    }

    // ─── 堆序列的栈效果测试 ───
    //
    // 用一个最小解释器执行生成器直接产出的原始指令，验证数组块布局、
    // copy-on-write 与 VecPop 的运行时栈效果。

    #[derive(Debug, Clone, Copy)]
    enum TestInst {
        Push(i64),
        Dup,
        Swap,
        Pop,
        Add,
        Sub,
        Mul,
        Div,
        Mod,
        Eq,
        Ne,
        Gt,
        Ge,
        Lt,
        Le,
        Store,
        Load,
        ShiftL,
        ShiftR,
        Pick,
        Mark(usize),
        Jump(usize),
        UncondJump(usize),
    }

    fn decode(code: &str) -> Vec<TestInst> {
        let mut insts = Vec::new();
        for line in code.lines() {
            let line = line.trim_end();
            if line.trim().is_empty() || line.trim_start().starts_with("//") {
                continue;
            }
            let spaces = line.len() - line.trim_start_matches(' ').len();
            let symbol = line.trim_start_matches(' ').chars().next().unwrap();
            insts.push(match symbol {
                '+' => match spaces {
                    1 => TestInst::Dup,
                    2 => TestInst::Swap,
                    4 => TestInst::Pop,
                    n if n >= 5 => TestInst::Push((n - 5) as i64),
                    _ => panic!("unexpected '+' encoding: {line:?}"),
                },
                '*' => match spaces {
                    0 => TestInst::Add,
                    1 => TestInst::Sub,
                    2 => TestInst::Mul,
                    3 => TestInst::Div,
                    4 => TestInst::Mod,
                    _ => panic!("unexpected '*' encoding: {line:?}"),
                },
                '=' => match spaces {
                    0 => TestInst::Eq,
                    1 => TestInst::Ne,
                    2 => TestInst::Lt,
                    3 => TestInst::Gt,
                    4 => TestInst::Le,
                    5 => TestInst::Ge,
                    _ => panic!("unexpected '=' encoding: {line:?}"),
                },
                '-' => match spaces {
                    0 => TestInst::Store,
                    1 => TestInst::Load,
                    _ => panic!("unexpected '-' encoding: {line:?}"),
                },
                '<' => match spaces {
                    0 => TestInst::ShiftL,
                    2 => TestInst::Pick,
                    _ => panic!("unexpected '<' encoding: {line:?}"),
                },
                '>' => match spaces {
                    0 => TestInst::ShiftR,
                    _ => panic!("unexpected '>' encoding: {line:?}"),
                },
                '`' => TestInst::Mark(spaces),
                '\'' => TestInst::Jump(spaces),
                '~' => TestInst::UncondJump(spaces),
                other => panic!("unexpected instruction {other:?} in {line:?}"),
            });
        }
        insts
    }

    /// 执行生成序列；堆分配器预置为 `HEAP_BASE`。
    fn execute(code: &str) -> (Vec<i64>, HashMap<i64, i64>) {
        let insts = decode(code);
        let mut marks = HashMap::new();
        for (i, inst) in insts.iter().enumerate() {
            if let TestInst::Mark(n) = inst {
                marks.insert(*n, i);
            }
        }
        let mut stack: Vec<i64> = Vec::new();
        let mut heap: HashMap<i64, i64> = HashMap::new();
        heap.insert(HEAP_ALLOC_PTR as i64, HEAP_BASE);
        let mut pc = 0usize;
        while pc < insts.len() {
            match insts[pc] {
                TestInst::Push(n) => stack.push(n),
                TestInst::Dup => stack.push(*stack.last().expect("dup underflow")),
                TestInst::Swap => {
                    let len = stack.len();
                    assert!(len >= 2, "swap underflow");
                    stack.swap(len - 1, len - 2);
                }
                TestInst::Pop => {
                    stack.pop().expect("pop underflow");
                }
                TestInst::Add => {
                    let b = stack.pop().expect("add underflow");
                    let a = stack.pop().expect("add underflow");
                    stack.push(a + b);
                }
                TestInst::Sub => {
                    let b = stack.pop().expect("sub underflow");
                    let a = stack.pop().expect("sub underflow");
                    stack.push(a - b);
                }
                TestInst::Mul => {
                    let b = stack.pop().expect("mul underflow");
                    let a = stack.pop().expect("mul underflow");
                    stack.push(a * b);
                }
                TestInst::Div => {
                    let b = stack.pop().expect("div underflow");
                    let a = stack.pop().expect("div underflow");
                    assert_ne!(b, 0, "division by zero trap");
                    stack.push(a / b);
                }
                TestInst::Mod => {
                    let b = stack.pop().expect("mod underflow");
                    let a = stack.pop().expect("mod underflow");
                    assert_ne!(b, 0, "modulo by zero trap");
                    stack.push(a % b);
                }
                TestInst::Eq => {
                    let b = stack.pop().expect("eq underflow");
                    let a = stack.pop().expect("eq underflow");
                    stack.push((a == b) as i64);
                }
                TestInst::Ne => {
                    let b = stack.pop().expect("ne underflow");
                    let a = stack.pop().expect("ne underflow");
                    stack.push((a != b) as i64);
                }
                TestInst::Le => {
                    let b = stack.pop().expect("le underflow");
                    let a = stack.pop().expect("le underflow");
                    stack.push((a <= b) as i64);
                }
                TestInst::Lt => {
                    let b = stack.pop().expect("lt underflow");
                    let a = stack.pop().expect("lt underflow");
                    stack.push((a < b) as i64);
                }
                TestInst::Gt => {
                    let b = stack.pop().expect("gt underflow");
                    let a = stack.pop().expect("gt underflow");
                    stack.push((a > b) as i64);
                }
                TestInst::Ge => {
                    let b = stack.pop().expect("ge underflow");
                    let a = stack.pop().expect("ge underflow");
                    stack.push((a >= b) as i64);
                }
                TestInst::Store => {
                    let addr = stack.pop().expect("store addr underflow");
                    let val = stack.pop().expect("store val underflow");
                    heap.insert(addr, val);
                }
                TestInst::Load => {
                    let addr = stack.pop().expect("load addr underflow");
                    stack.push(*heap.get(&addr).unwrap_or(&0));
                }
                TestInst::ShiftL => {
                    let first = stack.remove(0);
                    stack.push(first);
                }
                TestInst::ShiftR => {
                    let last = stack.pop().expect("shiftr underflow");
                    stack.insert(0, last);
                }
                TestInst::Pick => {
                    let n = stack.pop().expect("pick depth underflow").max(0) as usize;
                    assert!(n < stack.len(), "pick underflow");
                    let val = stack[stack.len() - 1 - n];
                    stack.push(val);
                }
                TestInst::Mark(_) => {}
                TestInst::Jump(n) => {
                    let cond = stack.pop().expect("jump condition underflow");
                    if cond != 0 {
                        pc = marks[&n];
                    }
                }
                TestInst::UncondJump(n) => pc = marks[&n],
            }
            pc += 1;
        }
        (stack, heap)
    }

    fn push_model(g: &mut StardustGen, n: i64) {
        g.emit_push_int(n);
        g.stack.push(Slot::unknown());
    }

    fn bind_model(g: &mut StardustGen, name: &str) {
        let id = g.next_binding;
        g.next_binding += 1;
        let top = g.stack.last_mut().expect("value to bind");
        top.binding = Some(id);
        g.scopes
            .last_mut()
            .expect("test scope")
            .vars
            .insert(name.to_string(), id);
    }

    fn new_gen() -> StardustGen {
        let mut g = StardustGen::new();
        g.scopes.push(Scope::default());
        g
    }

    fn build_block(g: &mut StardustGen, values: &[i64]) {
        for v in values {
            push_model(g, *v);
        }
        g.gen_build_array(values.len());
        g.pop_n_model(values.len());
        g.stack.push(Slot::unknown());
    }

    #[test]
    fn test_make_array_heap_block_layout() {
        let mut g = new_gen();
        build_block(&mut g, &[10, 20, 30]);
        let code = g.output;
        let (stack, heap) = execute(&code);

        assert_eq!(stack, vec![HEAP_BASE], "array handle should be on top");
        let handle = HEAP_BASE;
        assert_eq!(heap[&handle], 3, "len header");
        assert_eq!(heap[&(handle + 1)], 3, "cap header");
        assert_eq!(heap[&(handle + 2)], 10);
        assert_eq!(heap[&(handle + 3)], 20);
        assert_eq!(heap[&(handle + 4)], 30);
        assert_eq!(heap[&(HEAP_ALLOC_PTR as i64)], HEAP_BASE + 5);
    }

    #[test]
    fn test_make_array_repeat_heap_block_layout() {
        let mut g = new_gen();
        push_model(&mut g, 7);
        push_model(&mut g, 4);
        g.gen_make_array_repeat();
        let (stack, heap) = execute(&g.output);

        assert_eq!(stack, vec![HEAP_BASE]);
        let handle = HEAP_BASE;
        assert_eq!(heap[&handle], 4);
        assert_eq!(heap[&(handle + 1)], 4);
        for i in 0..4 {
            assert_eq!(heap[&(handle + 2 + i)], 7, "element {i}");
        }
        assert_eq!(heap[&(HEAP_ALLOC_PTR as i64)], HEAP_BASE + 6);
    }

    #[test]
    fn test_field_store_copies_block_and_rebinds() {
        let mut g = new_gen();
        build_block(&mut g, &[1, 2]);
        bind_model(&mut g, "p");

        push_model(&mut g, 9);
        g.gen_field_store("p", 0);
        let (stack, heap) = execute(&g.output);

        assert_eq!(stack.len(), 1, "variable slot should remain");
        let new_handle = stack[0];
        assert_ne!(new_handle, HEAP_BASE, "field store must copy the block");
        // 旧块保持原值。
        assert_eq!(heap[&HEAP_BASE], 2);
        assert_eq!(heap[&(HEAP_BASE + 2)], 1);
        assert_eq!(heap[&(HEAP_BASE + 3)], 2);
        // 新块字段已更新。
        assert_eq!(heap[&new_handle], 2);
        assert_eq!(heap[&(new_handle + 1)], 2);
        assert_eq!(heap[&(new_handle + 2)], 9);
        assert_eq!(heap[&(new_handle + 3)], 2);
    }

    #[test]
    fn test_index_store_copies_block_and_rebinds() {
        let mut g = new_gen();
        build_block(&mut g, &[10, 20, 30]);
        bind_model(&mut g, "v");

        push_model(&mut g, 1); // index
        push_model(&mut g, 99); // value
        g.gen_index_store("v");
        let (stack, heap) = execute(&g.output);

        assert_eq!(stack.len(), 1);
        let new_handle = stack[0];
        assert_ne!(new_handle, HEAP_BASE);
        // 旧块不被修改。
        assert_eq!(heap[&(HEAP_BASE + 3)], 20);
        // 新块写入下标 1。
        assert_eq!(heap[&new_handle], 3);
        assert_eq!(heap[&(new_handle + 2)], 10);
        assert_eq!(heap[&(new_handle + 3)], 99);
        assert_eq!(heap[&(new_handle + 4)], 30);
    }

    #[test]
    fn test_vec_pop_mutates_block_in_place() {
        let mut g = new_gen();
        build_block(&mut g, &[1, 2, 3]);
        bind_model(&mut g, "v");
        g.gen_vec_pop("v");
        let (stack, heap) = execute(&g.output);

        assert_eq!(stack.len(), 2, "variable slot + popped value");
        let handle = stack[0];
        let popped = stack[1];
        assert_eq!(popped, 3);
        assert_eq!(handle, HEAP_BASE, "pop should not relocate the block");
        assert_eq!(heap[&handle], 2, "length decremented");
        assert_eq!(heap[&(handle + 2)], 1);
        assert_eq!(heap[&(handle + 3)], 2);
    }

    #[test]
    fn test_vec_pop_empty_returns_zero_and_preserves_block() {
        let mut g = new_gen();
        build_block(&mut g, &[]);
        bind_model(&mut g, "v");
        g.gen_vec_pop("v");
        let (stack, heap) = execute(&g.output);

        assert_eq!(stack.len(), 2);
        let new_handle = stack[0];
        assert_eq!(stack[1], 0);
        assert_eq!(heap[&new_handle], 0);
    }
}
