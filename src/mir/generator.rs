use std::collections::HashMap;

use crate::frontend::ast::{self, BinOp, Literal, UnOp};
use crate::hir::types::*;
use crate::typeck::TypeTable;
use crate::vm::id::FnId;
use crate::vm::mir::{MirInst, TypeId};
use crate::vm::value::Value;

/// Label 管理器 — 生成唯一跳转标签
#[derive(Debug, Clone)]
struct LabelManager {
    counter: usize,
}

impl LabelManager {
    fn new() -> Self {
        Self { counter: 0 }
    }

    fn fresh(&mut self) -> usize {
        let id = self.counter;
        self.counter += 1;
        id
    }
}

/// 栈槽追踪项
#[derive(Debug, Clone)]
struct SlotInfo {
    /// 变量名（None 表示匿名临时值）
    var_name: Option<String>,
}

/// 栈追踪器 — 维护编译期虚拟栈，追踪每个槽的内容
#[derive(Debug, Clone)]
pub struct StackTracker {
    /// 从栈底到栈顶的槽信息
    slots: Vec<SlotInfo>,
}

impl Default for StackTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl StackTracker {
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// 压入匿名值
    pub fn push_temp(&mut self) {
        self.slots.push(SlotInfo { var_name: None });
    }

    /// 压入命名变量
    pub fn push_var(&mut self, name: String) {
        self.slots.push(SlotInfo { var_name: Some(name) });
    }

    /// 弹出栈顶
    pub fn pop(&mut self) {
        self.slots.pop();
    }

    /// 复制栈顶
    pub fn dup(&mut self) {
        let top = self.slots.last().cloned();
        if let Some(_info) = top {
            // 复制的值是匿名的（新 ElementId）
            self.slots.push(SlotInfo { var_name: None });
        }
    }

    /// roll(n) 后更新栈追踪
    pub fn roll(&mut self, n: i32) {
        if n == 0 {
            return;
        }
        let len = self.slots.len();
        if n > 0 {
            let pos = n as usize;
            if pos < len {
                let idx = len - 1 - pos;
                let item = self.slots.remove(idx);
                self.slots.push(item);
            }
        } else {
            let pos = (-n) as usize;
            if pos < len {
                let item = self.slots.pop().unwrap();
                let insert_idx = self.slots.len() - pos;
                self.slots.insert(insert_idx, item);
            }
        }
    }

    /// 交换栈顶两个槽
    pub fn swap(&mut self) {
        let len = self.slots.len();
        if len >= 2 {
            self.slots.swap(len - 1, len - 2);
        }
    }

    /// 查找变量在栈中的位置（距栈顶的深度，0=栈顶）
    /// 从栈顶向下查找最近的匹配
    pub fn find_var(&self, name: &str) -> Option<usize> {
        for (i, slot) in self.slots.iter().rev().enumerate() {
            if slot.var_name.as_deref() == Some(name) {
                return Some(i);
            }
        }
        None
    }

    /// 更新指定位置的变量名
    pub fn update_var(&mut self, pos: usize, name: String) {
        let idx = self.slots.len() - 1 - pos;
        if idx < self.slots.len() {
            self.slots[idx].var_name = Some(name);
        }
    }

    /// 截断栈（用于 ret）
    pub fn truncate(&mut self, new_len: usize) {
        self.slots.truncate(new_len);
    }
}

/// MIR 生成器 — 将 HIR 翻译为 MIR 指令序列
pub struct MirGen {
    mir: Vec<MirInst>,
    stack: StackTracker,
    labels: LabelManager,
    /// 函数名 → FnId 映射
    name_to_id: HashMap<String, FnId>,
    /// 类型注册表（用于复合类型布局）
    type_table: TypeTable,
    /// 当前函数中活跃的变量名（用于去重）
    #[allow(dead_code)]
    active_vars: Vec<HashMap<String, usize>>,
}

impl MirGen {
    pub fn new() -> Self {
        Self {
            mir: Vec::new(),
            stack: StackTracker::new(),
            labels: LabelManager::new(),
            name_to_id: HashMap::new(),
            type_table: TypeTable::new(),
            active_vars: Vec::new(),
        }
    }

    /// 设置函数名→ID 映射
    pub fn with_name_map(mut self, map: HashMap<String, FnId>) -> Self {
        self.name_to_id = map;
        self
    }

    /// 设置类型表
    pub fn with_type_table(mut self, table: TypeTable) -> Self {
        self.type_table = table;
        self
    }

    /// 获取生成的 MIR 指令
    pub fn into_mir(self) -> Vec<MirInst> {
        self.mir
    }

    // ─── 顶层入口 ───

    /// 生成整个程序的 MIR 指令
    pub fn gen_program(&mut self, program: &HirProgram) -> Vec<(FnId, Vec<MirInst>)> {
        let mut functions: Vec<(FnId, Vec<MirInst>)> = Vec::new();
        let mut fn_id_counter: u64 = 1;

        // 构建函数名→FnId 映射并存入 self
        self.name_to_id.clear();
        for sig in &program.fn_signatures {
            let id = FnId(fn_id_counter);
            fn_id_counter += 1;
            self.name_to_id.insert(sig.name.clone(), id);
        }

        // 为每个函数生成 MIR
        for decl in &program.decls {
            match decl {
                HirDecl::Fn(f) => {
                    let fn_id = *self.name_to_id.get(&f.name).unwrap_or(&FnId(fn_id_counter));
                    if fn_id.0 >= fn_id_counter {
                        fn_id_counter = fn_id.0 + 1;
                    }
                    let (mir, _sig) = self.gen_fn(f);
                    functions.push((fn_id, mir));
                }
                HirDecl::Extern(ext) => {
                    for item in &ext.items {
                        let HirExternItem::Fn(f) = item;
                        let fn_id = *self.name_to_id.get(&f.name).unwrap_or(&FnId(fn_id_counter));
                        if fn_id.0 >= fn_id_counter {
                            fn_id_counter = fn_id.0 + 1;
                        }
                        let (mir, _sig) = self.gen_fn(f);
                        functions.push((fn_id, mir));
                    }
                }
                _ => {}
            }
        }

        functions
    }

    /// 生成单个函数的 MIR
    pub fn gen_fn(&mut self, f: &HirFnDecl) -> (Vec<MirInst>, FnSignature) {
        self.mir = Vec::new();
        self.stack = StackTracker::new();
        self.labels = LabelManager::new();

        // 进入函数作用域
        self.mir.push(MirInst::EnterScope);

        // 参数由 VM 在帧创建时压入（CallElement + params）。
        // 栈布局: [CE=slot0, p1, p2, ..., pn] (底部→顶部)
        // 从最后一个参数开始绑定（因为它在栈顶）
        let n = f.sig.params.len();
        for (i, (name, _ty)) in f.sig.params.iter().enumerate() {
            self.stack.push_var(name.clone());
            // param_i 距栈顶的位置: pos = n - 1 - i
            let pos = n as i32 - 1 - i as i32;
            if pos > 0 {
                self.mir.push(MirInst::Roll { n: pos });
                self.stack.roll(pos);
            }
            self.mir.push(MirInst::BindVar { name: name.clone() });
            if pos > 0 {
                self.mir.push(MirInst::Roll { n: -pos });
                self.stack.roll(-pos);
            }
        }

        // 生成函数体
        if let Some(ref body) = f.body {
            self.gen_block(body, &f.sig);
        }

        // 确保函数以 ret 结束（如果没有显式 return）
        if self.mir.last().is_none_or(|inst| !matches!(inst, MirInst::Ret { .. })) {
            self.mir.push(MirInst::Ret { argc: 0 });
        }

        (std::mem::take(&mut self.mir), f.sig.clone())
    }

    // ─── 块 ───

    fn gen_block(&mut self, block: &HirBlock, fn_sig: &FnSignature) {
        self.mir.push(MirInst::EnterScope);

        for stmt in &block.stmts {
            self.gen_stmt(stmt, fn_sig);
        }

        if let Some(ref tail) = block.tail {
            self.gen_expr(tail, fn_sig);
            // 尾表达式的值是隐式返回值
            // 如果函数没有显式 return，尾表达式的结果留在栈顶作为返回值
        }

        self.mir.push(MirInst::ExitScope);
    }

    // ─── 语句 ───

    fn gen_stmt(&mut self, stmt: &HirStmt, fn_sig: &FnSignature) {
        match stmt {
            HirStmt::Let { pattern, init, .. } => {
                // 求值 RHS（如果有）
                if let Some(init_expr) = init {
                    self.gen_expr(init_expr, fn_sig);
                }

                // 提取变量名并绑定
                if let ast::Pattern::Var(name, _) = pattern {
                    // RHS 的值在栈顶 → BindVar
                    self.mir.push(MirInst::BindVar { name: name.clone() });
                    self.stack.push_var(name.clone());
                }
            }
            HirStmt::Expr(expr, _) => {
                self.gen_expr(expr, fn_sig);
                // 表达式语句的结果丢弃
                if !self.stack.is_empty() {
                    self.mir.push(MirInst::Pop { ty: TypeId::Unknown });
                    self.stack.pop();
                }
            }
            HirStmt::Assign { target, depth: _, value, .. } => {
                // 求值 RHS
                self.gen_expr(value, fn_sig);
                // RHS 结果在栈顶 → WriteVar
                self.mir.push(MirInst::WriteVar { name: target.clone() });
                // stack 追踪：WriteVar 替换旧值，栈深度不变但栈顶是旧值的位置
                // 简化：更新变量位置信息
                if let Some(pos) = self.stack.find_var(target) {
                    self.stack.update_var(pos, target.clone());
                }
            }
        }
    }

    // ─── 表达式 ───

    fn gen_expr(&mut self, expr: &HirExpr, fn_sig: &FnSignature) {
        match expr {
            HirExpr::Lit(lit, _) => {
                let value = literal_to_value(lit);
                self.mir.push(MirInst::Push { value, ty: TypeId::Unknown });
                self.stack.push_temp();
            }
            HirExpr::Var(var_ref) => {
                // scope_depth == usize::MAX 表示类型名或函数名，非变量
                if var_ref.scope_depth != usize::MAX {
                    self.mir.push(MirInst::ReadVar { name: var_ref.name.clone() });
                    self.stack.push_temp();
                }
                // 类型名不生成代码（在 StructLit/EnumLit 中使用）
            }
            HirExpr::Binary(lhs, op, rhs, _) => {
                self.gen_expr(lhs, fn_sig);
                self.gen_expr(rhs, fn_sig);

                let mir_op = binop_to_mir(*op);
                self.mir.push(mir_op);
                self.stack.pop(); // 弹出 rhs
                self.stack.pop(); // 弹出 lhs
                self.stack.push_temp(); // 压入结果
            }
            HirExpr::Unary(op, inner, _) => {
                self.gen_expr(inner, fn_sig);
                match op {
                    UnOp::Neg => {
                        self.mir.push(MirInst::Neg { ty: TypeId::Unknown });
                    }
                    UnOp::Not => {
                        self.mir.push(MirInst::Not);
                    }
                    UnOp::Deref => {
                        self.mir.push(MirInst::Deref);
                    }
                }
                // unary 操作 pop 1, push 1: 栈深度不变
            }
            HirExpr::Call { func, args, stack_pass, fn_sig: call_sig, .. } => {
                let argc = if *stack_pass {
                    // 栈传参：参数已在栈顶，查找被调函数签名
                    if let Some(sig) = call_sig {
                        sig.n
                    } else {
                        args.len() as u32
                    }
                } else {
                    args.len() as u32
                };

                if !stack_pass {
                    // 命名/位置参数：push CallElement, 然后 push 参数
                    self.mir.push(MirInst::Push {
                        value: Value::CallElement { argc },
                        ty: TypeId::Unknown,
                    });
                    self.stack.push_temp();

                    for arg in args {
                        match arg {
                            HirCallArg::Pos(e, _) | HirCallArg::Named(_, e, _) => {
                                self.gen_expr(e, fn_sig);
                            }
                        }
                    }
                }

                // 获取函数 ID
                let fn_name = match func.as_ref() {
                    HirExpr::Var(var_ref) => var_ref.name.clone(),
                    _ => "unknown".to_string(),
                };

                let fn_id = self.name_to_id.get(&fn_name).copied().unwrap_or(FnId(0));
                self.mir.push(MirInst::Call { fn_id });

                // 弹出参数和 CallElement，压入返回值
                let ret_count = call_sig.as_ref().map_or(0, |s| s.m);
                // 清理调用者的栈追踪
                if !stack_pass {
                    for _ in 0..argc {
                        self.stack.pop();
                    }
                    self.stack.pop(); // pop CallElement
                }
                for _ in 0..ret_count {
                    self.stack.push_temp();
                }
            }
            HirExpr::If(cond, then_block, else_block, _) => {
                let else_label = self.labels.fresh();
                let end_label = self.labels.fresh();

                // 条件
                self.gen_expr(cond, fn_sig);
                self.mir.push(MirInst::Jump(else_label)); // 条件为假时跳到 else
                self.stack.pop(); // pop cond

                // then 分支
                self.mir.push(MirInst::Mark(else_label + 1000)); // then 标记（临时）
                self.gen_block(then_block, fn_sig);
                self.mir.push(MirInst::UncondJump(end_label));

                // else 分支
                self.mir.push(MirInst::Mark(else_label));
                if let Some(else_blk) = else_block {
                    self.gen_block(else_blk, fn_sig);
                }

                self.mir.push(MirInst::Mark(end_label));
            }
            HirExpr::While(cond, body, _) => {
                let loop_start = self.labels.fresh();
                let loop_end = self.labels.fresh();

                self.mir.push(MirInst::Mark(loop_start));
                self.gen_expr(cond, fn_sig);
                self.mir.push(MirInst::Not); // 取反：cond==false 时跳出
                self.mir.push(MirInst::Jump(loop_end));
                self.stack.pop(); // pop cond

                self.gen_block(body, fn_sig);
                self.mir.push(MirInst::UncondJump(loop_start));
                self.mir.push(MirInst::Mark(loop_end));
            }
            HirExpr::Loop(body, _) => {
                let loop_start = self.labels.fresh();
                let loop_end = self.labels.fresh();

                self.mir.push(MirInst::Mark(loop_start));
                self.gen_block(body, fn_sig);
                self.mir.push(MirInst::UncondJump(loop_start));
                self.mir.push(MirInst::Mark(loop_end));
            }
            HirExpr::Block(block, _) => {
                self.gen_block(block, fn_sig);
            }
            HirExpr::Return(values, _) => {
                // 求值所有返回值
                for val in values {
                    self.gen_expr(val, fn_sig);
                }
                let argc = values.len() as u32;
                self.mir.push(MirInst::Ret { argc });
                // 清理栈追踪（ret 会截断栈）
                let keep = self.stack.len().saturating_sub(argc as usize);
                self.stack.truncate(keep);
            }
            HirExpr::Break(value, _) => {
                if let Some(val) = value {
                    self.gen_expr(val, fn_sig);
                }
                // Break 由上层处理（loop/while 提供 loop_end label）
                // 简化：生成注释
                self.mir.push(MirInst::Comment("break".to_string()));
            }
            HirExpr::Ref(inner, mutable, _) => {
                // &x: 查找 x 的 ElementId
                if let HirExpr::Var(var_ref) = inner.as_ref() {
                    self.mir.push(MirInst::Ref {
                        name: var_ref.name.clone(),
                        mutable: *mutable,
                    });
                    self.stack.push_temp();
                } else {
                    // 复杂引用表达式
                    self.gen_expr(inner, fn_sig);
                    self.mir.push(MirInst::Comment("complex ref".to_string()));
                }
            }
            HirExpr::Deref(inner, _) => {
                self.gen_expr(inner, fn_sig);
                self.mir.push(MirInst::Deref);
                // Deref: pop ref, push value (寻找+dup)
                self.stack.pop();
                self.stack.push_temp();
            }
            HirExpr::Tuple(elems, _) => {
                for e in elems {
                    self.gen_expr(e, fn_sig);
                }
            }
            HirExpr::Array(elems, _) => {
                for e in elems {
                    self.gen_expr(e, fn_sig);
                }
            }
            HirExpr::FieldAccess(inner, field, _) => {
                self.gen_expr(inner, fn_sig);
                // 尝试从类型表查找字段偏移量
                let offset = self.resolve_field_offset(inner, field);
                self.mir.push(MirInst::FieldAccess { index: offset });
                // 栈：pop 结构体（多槽），push 字段值（1槽）
                // 简化追踪：假设结构体占 N 槽，字段占 1 槽
                self.stack.pop(); // pop 结构体（实际是多槽但追踪器跟踪1槽）
                self.stack.push_temp();
            }
            HirExpr::Index(inner, index, _) => {
                self.gen_expr(inner, fn_sig);
                self.gen_expr(index, fn_sig);
                self.mir.push(MirInst::IndexAccess);
                self.stack.pop(); // index
                self.stack.pop(); // array
                self.stack.push_temp();
            }
            HirExpr::StructLit { name, fields, .. } => {
                // 按结构体字段声明顺序压入字段值
                let field_order = self.type_table.lookup_struct(&name.join("::"))
                    .map(|s| s.fields.iter().map(|f| f.name.clone()).collect::<Vec<_>>())
                    .unwrap_or_else(|| fields.iter().map(|(n, _)| n.clone()).collect());

                let mut field_values: HashMap<String, &HirExpr> = fields.iter()
                    .map(|(n, e)| (n.clone(), e))
                    .collect();

                for fname in &field_order {
                    if let Some(e) = field_values.remove(fname) {
                        self.gen_expr(e, fn_sig);
                    }
                }
                // 栈上有 N 个字段值（连续排列），表示整个结构体
            }
            HirExpr::EnumLit { name, variant, data, .. } => {
                let tag = self.type_table.variant_tag(&name.join("::"), variant)
                    .unwrap_or(0);
                // 压入标签值 + 可选数据
                self.mir.push(MirInst::Push {
                    value: Value::I32(tag as i64),
                    ty: TypeId::I32,
                });
                self.stack.push_temp();
                if let Some(data_expr) = data {
                    self.gen_expr(data_expr, fn_sig);
                }
                // MakeEnum: 将栈上 N+1 个槽合并为枚举
                let has_data = data.is_some();
                let data_slots = if has_data { 1 } else { 0 };
                self.mir.push(MirInst::MakeEnum { variant: tag as usize, data_slots });
                // 清理栈追踪
                if has_data {
                    self.stack.pop(); // data
                }
                self.stack.pop(); // tag
                self.stack.push_temp(); // enum value
            }
            HirExpr::Match(scrutinee, arms, ..) => {
                // 简化 match 实现：假设枚举类型，生成 EnumTag + 比较 + 跳转
                self.gen_expr(scrutinee, fn_sig);
                // 枚举值在栈顶，读取 tag
                self.mir.push(MirInst::EnumTag);
                // 栈：原枚举（多槽）→ 仅保留 tag（1槽）
                // 简化：枚举被消费，tag 留在栈顶用于匹配
                self.stack.pop(); // 原枚举
                self.stack.push_temp(); // tag

                let end_label = self.labels.fresh();
                for arm in arms {
                    let arm_label = self.labels.fresh();
                    // 对于每个分支：dup tag，比较，如果匹配则跳转
                    if let ast::Pattern::Enum { variant, .. } = &arm.pattern {
                        let vtag = self.type_table.variant_tag(
                            // 从 scrutinee 推断枚举名 — 简化处理
                            "Option", // 占位
                            variant,
                        ).unwrap_or(0);
                        self.mir.push(MirInst::Dup { ty: TypeId::I32 });
                        self.stack.dup();
                        self.mir.push(MirInst::Push { value: Value::I32(vtag as i64), ty: TypeId::I32 });
                        self.stack.push_temp();
                        self.mir.push(MirInst::Eq);
                        self.stack.pop();
                        self.stack.pop();
                        self.stack.push_temp();
                        self.mir.push(MirInst::Jump(arm_label));
                        self.stack.pop();
                    }
                    // 所有分支处理完，跳到 end
                    self.mir.push(MirInst::UncondJump(end_label));
                    // 分支体
                    self.mir.push(MirInst::Mark(arm_label));
                    self.gen_expr(&arm.body, fn_sig);
                }
                self.mir.push(MirInst::Mark(end_label));
            }
            HirExpr::StardustMacro(_insts, _) => {
                self.mir.push(MirInst::Comment("stardust! macro".to_string()));
            }
            HirExpr::Unsafe(block, _) => {
                self.gen_block(block, fn_sig);
            }

            // 其他表达式：占位
            _ => {
                self.mir.push(MirInst::Comment(format!("TODO: {:?}", std::mem::discriminant(expr))));
            }
        }
    }

    // ─── 复合类型辅助方法 ───

    /// 解析字段访问的偏移量
    fn resolve_field_offset(&self, expr: &HirExpr, field: &str) -> usize {
        if let HirExpr::Var(var_ref) = expr {
            let _ = var_ref;
        }
        if let Ok(idx) = field.parse::<usize>() {
            return idx;
        }
        0
    }

    #[allow(dead_code)]
    fn get_variant_tag(&self, enum_name: &str, variant: &str) -> u32 {
        self.type_table.variant_tag(enum_name, variant).unwrap_or(0)
    }
}

impl Default for MirGen {
    fn default() -> Self {
        Self::new()
    }
}

// ─── 辅助转换函数 ───

fn literal_to_value(lit: &Literal) -> Value {
    match lit {
        Literal::Int(v, suffix) => match suffix {
            ast::IntSuffix::None => Value::I64(*v),
            ast::IntSuffix::I8 => Value::I8(*v),
            ast::IntSuffix::I16 => Value::I16(*v),
            ast::IntSuffix::I32 => Value::I32(*v),
            ast::IntSuffix::I64 => Value::I64(*v),
            ast::IntSuffix::Isize => Value::Isize(*v),
            ast::IntSuffix::U8 => Value::U8(*v),
            ast::IntSuffix::U16 => Value::U16(*v),
            ast::IntSuffix::U32 => Value::U32(*v),
            ast::IntSuffix::U64 => Value::U64(*v),
            ast::IntSuffix::Usize => Value::Usize(*v),
        },
        Literal::Float(v) => Value::F64(*v),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Char(c) => Value::Char(*c),
        Literal::String(s) => Value::Tuple(
            s.chars().map(|c| Value::Char(c as u32)).collect()
        ),
        Literal::Unit => Value::Unit,
    }
}

fn binop_to_mir(op: BinOp) -> MirInst {
    match op {
        BinOp::Add => MirInst::Add { ty: TypeId::Unknown },
        BinOp::Sub => MirInst::Sub { ty: TypeId::Unknown },
        BinOp::Mul => MirInst::Mul { ty: TypeId::Unknown },
        BinOp::Div => MirInst::Div { ty: TypeId::Unknown },
        BinOp::Mod => MirInst::Mod { ty: TypeId::Unknown },
        BinOp::Eq => MirInst::Eq,
        BinOp::Ne => MirInst::Ne,
        BinOp::Lt => MirInst::Lt,
        BinOp::Gt => MirInst::Gt,
        BinOp::Le => MirInst::Le,
        BinOp::Ge => MirInst::Ge,
        BinOp::And => MirInst::And,
        BinOp::Or => MirInst::Or,
        BinOp::Xor => MirInst::Xor,
        BinOp::Assign => MirInst::Nop, // 赋值由 WriteVar 处理
    }
}
