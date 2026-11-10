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

    /// 将栈顶槽标记为命名变量（不改变栈深度）。
    ///
    /// `BindVar` 在运行时只是把栈顶值绑定到名字，并不会压入新槽；
    /// 因此 `let x = <expr>` 之后追踪器也应保持原有槽数，只更新槽名。
    pub fn bind_top(&mut self, name: String) {
        if let Some(slot) = self.slots.last_mut() {
            slot.var_name = Some(name);
        }
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
    /// 活跃的 loop end label 栈（用于 break 跳转）
    loop_ends: Vec<usize>,
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
            loop_ends: Vec::new(),
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

        // 确保函数以 ret 结束（如果没有显式 return）。
        // 隐式返回必须遵循函数签名的返回值个数：尾表达式的结果仍在栈顶。
        if self.mir.last().is_none_or(|inst| !matches!(inst, MirInst::Ret { .. })) {
            self.mir.push(MirInst::Ret { argc: f.sig.m });
        }

        (std::mem::take(&mut self.mir), f.sig.clone())
    }

    // ─── 块 ───

    fn gen_block(&mut self, block: &HirBlock, fn_sig: &FnSignature) {
        self.mir.push(MirInst::EnterScope);
        let scope_start = self.mir.len();

        for stmt in &block.stmts {
            self.gen_stmt(stmt, fn_sig);
        }

        if let Some(ref tail) = block.tail {
            self.gen_expr(tail, fn_sig);
        }

        // 收集本作用域中声明的变量，生成 Drop 指令。
        // DropVar 仅在变量实际持有 StackHandle 时释放远程栈，对其他值无副作用。
        let drop_vars: Vec<String> = self.collect_drop_vars(block);
        for name in drop_vars.iter().rev() {
            self.mir.push(MirInst::Comment(format!("drop {name}")));
            self.mir.push(MirInst::DropVar { name: name.clone() });
        }

        // 把本作用域的变量槽从值栈上移除。
        // 变量在运行时不会被读取变量表自动弹出；若保留到作用域外，
        // 分支内部声明的变量会让不同控制流路径的栈深度不一致
        // （后端按固定深度读变量时会出错），循环体每次迭代也会泄漏槽位。
        // 先 DropVar 释放资源，再按深度从深到浅移除槽位，保留栈顶的尾部表达式结果。
        for name in drop_vars.iter().rev() {
            if let Some(depth) = self.stack.find_var(name) {
                if depth > 0 {
                    self.mir.push(MirInst::Roll { n: depth as i32 });
                    self.stack.roll(depth as i32);
                }
                self.mir.push(MirInst::Pop { ty: TypeId::Unknown });
                self.stack.pop();
            }
        }

        self.mir.push(MirInst::ExitScope);
        let _ = scope_start;
    }

    /// 收集本作用域中已初始化、需要在退出时清理的变量。
    fn collect_drop_vars(&self, block: &HirBlock) -> Vec<String> {
        let mut names = Vec::new();
        for stmt in &block.stmts {
            if let HirStmt::Let { pattern, init: Some(_), .. } = stmt {
                collect_pattern_names(pattern, &mut names);
            }
        }
        names
    }

    // ─── 语句 ───

    fn gen_stmt(&mut self, stmt: &HirStmt, fn_sig: &FnSignature) {
        match stmt {
            HirStmt::Let { pattern, init, .. } => {
                // 求值 RHS（如果有）
                if let Some(init_expr) = init {
                    self.gen_expr(init_expr, fn_sig);
                    self.bind_pattern(pattern);
                }
            }
            HirStmt::Expr(expr, _) => {
                let before = self.stack.len();
                self.gen_expr(expr, fn_sig);
                // 只丢弃本表达式新增的结果。内建函数（如 print）返回 Unit，
                // 不会向栈上压值，不能误 Pop 已有变量。
                while self.stack.len() > before {
                    self.mir.push(MirInst::Pop { ty: TypeId::Unknown });
                    self.stack.pop();
                }
            }
            HirStmt::Assign { target, value, .. } => {
                match target {
                    HirAssignTarget::Var(var) => {
                        // 求值 RHS
                        self.gen_expr(value, fn_sig);
                        // RHS 结果在栈顶 → WriteVar 消费 RHS 并写回变量槽
                        self.mir.push(MirInst::WriteVar { name: var.name.clone() });
                        self.stack.pop();
                        if let Some(pos) = self.stack.find_var(&var.name) {
                            self.stack.update_var(pos, var.name.clone());
                        }
                    }
                    HirAssignTarget::Deref(inner) => {
                        // 先求值引用目标，再求值 RHS
                        self.gen_expr(inner, fn_sig);
                        self.gen_expr(value, fn_sig);
                        self.mir.push(MirInst::StoreDeref);
                        self.stack.pop(); // value
                        self.stack.pop(); // reference
                    }
                    HirAssignTarget::Field { object, field } => {
                        let HirExpr::Var(var) = object.as_ref() else {
                            self.mir.push(MirInst::Comment(
                                "ERROR: field assignment target must be a variable".to_string(),
                            ));
                            return;
                        };
                        let index = self.resolve_field_offset(object, field);
                        self.gen_expr(value, fn_sig);
                        self.mir.push(MirInst::FieldStore {
                            name: var.name.clone(),
                            index,
                        });
                        self.stack.pop(); // value
                    }
                    HirAssignTarget::Index { array, index } => {
                        let HirExpr::Var(var) = array.as_ref() else {
                            self.mir.push(MirInst::Comment(
                                "ERROR: index assignment target must be a variable".to_string(),
                            ));
                            return;
                        };
                        self.gen_expr(index, fn_sig);
                        self.gen_expr(value, fn_sig);
                        self.mir.push(MirInst::IndexStore { name: var.name.clone() });
                        self.stack.pop(); // value
                        self.stack.pop(); // index
                    }
                }
            }
        }
    }

    // ─── 表达式 ───

    /// 将栈顶的初始化值绑定到 let 模式。
    ///
    /// 简单变量直接绑定栈顶；元组解构时，值按模式元素顺序自底向上排列，
    /// 通过 roll 逐个把目标槽转到栈顶绑定，再恢复原布局。
    fn bind_pattern(&mut self, pattern: &ast::Pattern) {
        match pattern {
            ast::Pattern::Var(name, _) => {
                self.mir.push(MirInst::BindVar { name: name.clone() });
                self.stack.bind_top(name.clone());
            }
            ast::Pattern::Tuple(elems, _, _) => {
                let n = elems.len();
                for (i, elem) in elems.iter().enumerate() {
                    let ast::Pattern::Var(name, _) = elem else {
                        // 嵌套解构、字面量模式和通配符暂不绑定变量
                        continue;
                    };
                    let pos = (n - 1 - i) as i32;
                    if pos > 0 {
                        self.mir.push(MirInst::Roll { n: pos });
                        self.stack.roll(pos);
                    }
                    self.mir.push(MirInst::BindVar { name: name.clone() });
                    if pos > 0 {
                        self.mir.push(MirInst::Roll { n: -pos });
                        self.stack.roll(-pos);
                    }
                    self.stack.update_var(pos as usize, name.clone());
                }
            }
            _ => {}
        }
    }

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
            HirExpr::MethodCall { receiver, method, args, .. } => {
                match method.as_str() {
                    "len" => {
                        self.gen_expr(receiver, fn_sig);
                        self.mir.push(MirInst::ContainerLen);
                        self.stack.pop();
                        self.stack.push_temp();
                    }
                    "push" | "pop" => {
                        let HirExpr::Var(var) = receiver.as_ref() else {
                            self.mir.push(MirInst::Comment(format!(
                                "ERROR: `{method}` receiver must be a variable"
                            )));
                            return;
                        };
                        if method == "push" {
                            if let Some(arg) = args.first() {
                                self.gen_expr(arg, fn_sig);
                            }
                            self.mir.push(MirInst::VecPush { name: var.name.clone() });
                            if !args.is_empty() {
                                self.stack.pop();
                            }
                        } else {
                            self.mir.push(MirInst::VecPop { name: var.name.clone() });
                            self.stack.push_temp();
                        }
                    }
                    other => {
                        self.mir.push(MirInst::Comment(format!(
                            "ERROR: unknown method `{other}`"
                        )));
                    }
                }
            }
            HirExpr::TypeMethod { type_name, method, args, .. } => {
                let name = type_name.join("::");
                match (name.as_str(), method.as_str()) {
                    ("Vec", "new") | ("String", "new") => {
                        self.mir.push(MirInst::MakeArray { count: 0 });
                        self.stack.push_temp();
                    }
                    ("Box", "new") => {
                        if let Some(arg) = args.first() {
                            self.gen_expr(arg, fn_sig);
                            self.mir.push(MirInst::MakeBox);
                            self.stack.pop();
                            self.stack.push_temp();
                        }
                    }
                    _ => {
                        self.mir.push(MirInst::Comment(format!(
                            "ERROR: unknown associated function `{name}::{method}`"
                        )));
                    }
                }
            }
            HirExpr::Call { func, args, stack_pass, fn_sig: call_sig, .. } => {
                // 获取函数名（用于内建函数检测）
                let fn_name = match func.as_ref() {
                    HirExpr::Var(var_ref) => var_ref.name.clone(),
                    _ => String::new(),
                };

                // 处理内建/标准库函数（无需 CallElement + Call）。
                // 用户定义的同名函数优先，避免内建函数遮蔽用户函数。
                // 空参数调用（`read_int()` / `print_int()`）在 parser 中标记为
                // 栈传参，对内建函数同样有效。
                if Self::is_builtin(&fn_name)
                    && (!stack_pass || args.is_empty())
                    && !self.name_to_id.contains_key(&fn_name)
                {
                    // 求值参数
                    for arg in args {
                        match arg {
                            HirCallArg::Pos(e, _) | HirCallArg::Named(_, e, _) => {
                                self.gen_expr(e, fn_sig);
                            }
                        }
                    }
                    // 生成内建指令
                    self.gen_builtin_call(&fn_name);
                    return;
                }

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

                // 零参数调用没有可以栈传的参数，仍必须压入 CallElement，
                // 否则 VM 的 Call 指令找不到调用帧。有参数的栈传参保持原语义。
                let needs_call_element = !stack_pass || argc == 0;

                if needs_call_element {
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

                let fn_id = self.name_to_id.get(&fn_name).copied().unwrap_or_else(|| {
                    self.mir.push(MirInst::Comment(format!("ERROR: unknown function '{fn_name}'")));
                    FnId(0)
                });
                let ret_count = call_sig.as_ref().map_or(0, |s| s.m);
                self.mir.push(MirInst::Call { fn_id, return_arity: ret_count });
                // 清理调用者的栈追踪
                if needs_call_element {
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

                // 条件 — VM的Jump在条件为真时跳转，故先取反
                self.gen_expr(cond, fn_sig);
                self.mir.push(MirInst::Not);  // cond==false → true
                self.mir.push(MirInst::Jump(else_label));
                self.stack.pop(); // pop cond

                // 两支从同一个栈状态出发。追踪器必须按分支重置，
                // 否则 then 分支的局部变量/结果会污染 else 分支和汇合点的深度。
                let branch_base = self.stack.clone();

                // then 分支
                let then_label = self.labels.fresh();
                self.mir.push(MirInst::Mark(then_label));
                self.stack = branch_base.clone();
                self.gen_block(then_block, fn_sig);
                let then_results = self.stack.len().saturating_sub(branch_base.len());
                self.mir.push(MirInst::UncondJump(end_label));

                // else 分支
                self.mir.push(MirInst::Mark(else_label));
                self.stack = branch_base.clone();
                if let Some(else_blk) = else_block {
                    self.gen_block(else_blk, fn_sig);
                }
                let else_results = self.stack.len().saturating_sub(branch_base.len());

                self.mir.push(MirInst::Mark(end_label));
                // 汇合点：两条路径应留下同样多的结果；取较大值保持追踪器可用。
                if then_results != else_results {
                    self.mir.push(MirInst::Comment(format!(
                        "WARNING: if branches leave different stack heights ({then_results} vs {else_results})"
                    )));
                }
                self.stack = branch_base;
                for _ in 0..then_results.max(else_results) {
                    self.stack.push_temp();
                }
            }
            HirExpr::While(cond, body, _) => {
                let loop_start = self.labels.fresh();
                let loop_end = self.labels.fresh();

                self.mir.push(MirInst::Mark(loop_start));
                self.gen_expr(cond, fn_sig);
                self.mir.push(MirInst::Not);
                self.mir.push(MirInst::Jump(loop_end));
                self.stack.pop();

                let body_base = self.stack.clone();
                self.loop_ends.push(loop_end);
                self.gen_block(body, fn_sig);
                self.loop_ends.pop();
                // while 表达式的值是 unit：丢弃循环体尾部表达式的结果，
                // 并把追踪器恢复到循环入口状态（循环体局部变量已在 ExitScope 弹出）。
                let body_results = self.stack.len().saturating_sub(body_base.len());
                for _ in 0..body_results {
                    self.mir.push(MirInst::Pop { ty: TypeId::Unknown });
                }
                self.stack = body_base;

                self.mir.push(MirInst::UncondJump(loop_start));
                self.mir.push(MirInst::Mark(loop_end));
            }
            HirExpr::Loop(body, _) => {
                let loop_start = self.labels.fresh();
                let loop_end = self.labels.fresh();

                self.mir.push(MirInst::Mark(loop_start));

                self.loop_ends.push(loop_end);
                self.gen_block(body, fn_sig);
                self.loop_ends.pop();

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
                // 跳转到最内层循环的 end label
                if let Some(&loop_end) = self.loop_ends.last() {
                    self.mir.push(MirInst::UncondJump(loop_end));
                } else {
                    self.mir.push(MirInst::Comment("break outside loop".to_string()));
                }
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
                self.mir.push(MirInst::MakeArray { count: elems.len() as u32 });
                for _ in elems {
                    self.stack.pop();
                }
                self.stack.push_temp();
            }
            HirExpr::Repeat(elem, count, _) => {
                self.gen_expr(elem, fn_sig);
                self.gen_expr(count, fn_sig);
                self.mir.push(MirInst::MakeArrayRepeat);
                self.stack.pop(); // count
                self.stack.pop(); // elem
                self.stack.push_temp();
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
                // 结构体以单槽 Value::Tuple 表示，字段顺序与声明一致。
                self.mir.push(MirInst::MakeArray { count: field_order.len() as u32 });
                for _ in 0..field_order.len() {
                    self.stack.pop();
                }
                self.stack.push_temp();
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
            HirExpr::Match(scrutinee, arms, _) => {
                self.gen_expr(scrutinee, fn_sig);

                enum MatchCheck {
                    Enum(u32),
                    Lit(ast::Literal),
                    Any,
                }

                let end_label = self.labels.fresh();
                let fail_label = self.labels.fresh();

                // 先规划检查顺序；遇到无 guard 的变量/通配模式即可终止后续分支。
                let mut plans: Vec<(&HirMatchArm, MatchCheck, Option<(String, bool)>)> =
                    Vec::new();
                for arm in arms {
                    let (check, binding, terminal) = match &arm.pattern {
                        ast::Pattern::Enum { name, variant, inner, .. } => {
                            let tag = self.variant_tag_for_pattern(name, variant);
                            let binding = match inner.as_deref() {
                                Some(ast::Pattern::Var(name, _)) => {
                                    Some((name.clone(), true))
                                }
                                _ => None,
                            };
                            (MatchCheck::Enum(tag), binding, false)
                        }
                        ast::Pattern::Lit(lit, _) => {
                            (MatchCheck::Lit(lit.clone()), None, false)
                        }
                        ast::Pattern::Var(name, _) => (
                            MatchCheck::Any,
                            Some((name.clone(), false)),
                            arm.guard.is_none(),
                        ),
                        ast::Pattern::Wildcard(_) => {
                            (MatchCheck::Any, None, arm.guard.is_none())
                        }
                        _ => (MatchCheck::Any, None, false),
                    };
                    plans.push((arm, check, binding));
                    if terminal {
                        break;
                    }
                }

                let check_labels: Vec<usize> =
                    plans.iter().map(|_| self.labels.fresh()).collect();
                let body_labels: Vec<usize> =
                    plans.iter().map(|_| self.labels.fresh()).collect();

                // 检查块：匹配则跳转到对应分支体，否则继续下一个检查。
                for (i, (_, check, _)) in plans.iter().enumerate() {
                    self.mir.push(MirInst::Mark(check_labels[i]));
                    match check {
                        MatchCheck::Enum(tag) => {
                            self.mir.push(MirInst::EnumTag { total_slots: 0 });
                            self.stack.push_temp();
                            self.mir.push(MirInst::Push {
                                value: Value::I32(*tag as i64),
                                ty: TypeId::I32,
                            });
                            self.stack.push_temp();
                            self.mir.push(MirInst::Eq);
                            self.stack.pop();
                            self.stack.pop();
                            self.stack.push_temp();
                            self.mir.push(MirInst::Jump(body_labels[i]));
                            self.stack.pop();
                        }
                        MatchCheck::Lit(lit) => {
                            self.mir.push(MirInst::Dup { ty: TypeId::Unknown });
                            self.stack.dup();
                            self.mir.push(MirInst::Push {
                                value: literal_to_value(lit),
                                ty: TypeId::Unknown,
                            });
                            self.stack.push_temp();
                            self.mir.push(MirInst::Eq);
                            self.stack.pop();
                            self.stack.pop();
                            self.stack.push_temp();
                            self.mir.push(MirInst::Jump(body_labels[i]));
                            self.stack.pop();
                        }
                        MatchCheck::Any => {
                            self.mir.push(MirInst::UncondJump(body_labels[i]));
                        }
                    }
                }

                // 所有模式均未匹配：丢弃 scrutinee。
                self.mir.push(MirInst::Mark(fail_label));
                self.mir.push(MirInst::Pop { ty: TypeId::Unknown });
                self.stack.pop();
                self.mir.push(MirInst::UncondJump(end_label));

                // 分支体。
                for (i, (arm, _, binding)) in plans.iter().enumerate() {
                    self.mir.push(MirInst::Mark(body_labels[i]));
                    self.mir.push(MirInst::EnterScope);

                    let mut unwanted = 1; // scrutinee 本身
                    let mut guard_unwanted = 0;
                    match binding {
                        Some((name, true)) => {
                            // 枚举载荷位于 Value::Tuple 的 index 1。FieldAccess
                            // 会弹出聚合值，因此先 Dup 保留 scrutinee 供清理及
                            // guard 失败后的后续模式检查。
                            self.mir.push(MirInst::Dup { ty: TypeId::Unknown });
                            self.stack.dup();
                            self.mir.push(MirInst::FieldAccess { index: 1 });
                            self.stack.pop();
                            self.stack.push_temp();
                            self.mir.push(MirInst::BindVar { name: name.clone() });
                            self.stack.update_var(0, name.clone());
                            unwanted += 1;
                            guard_unwanted = 1;
                        }
                        Some((name, false)) => {
                            self.mir.push(MirInst::BindVar { name: name.clone() });
                            self.stack.update_var(0, name.clone());
                        }
                        None => {}
                    }

                    let next_label = check_labels
                        .get(i + 1)
                        .copied()
                        .unwrap_or(fail_label);
                    self.gen_match_guard_and_body(
                        arm,
                        fn_sig,
                        unwanted,
                        guard_unwanted,
                        end_label,
                        next_label,
                    );
                }

                self.mir.push(MirInst::Mark(end_label));
            }
            HirExpr::StardustMacro(insts, _) => {
                // 翻译 stardust! 宏指令为 MIR
                for inst in insts {
                    self.gen_stardust_inst(inst);
                }
            }
            HirExpr::Unsafe(block, _) => {
                // unsafe 块：标记作用域并生成代码（跳过类型检查）
                self.mir.push(MirInst::Comment("unsafe block begin".to_string()));
                self.gen_block(block, fn_sig);
                self.mir.push(MirInst::Comment("unsafe block end".to_string()));
            }

            // 其他表达式：占位
            _ => {
                self.mir.push(MirInst::Comment(format!("TODO: {:?}", std::mem::discriminant(expr))));
            }
        }
    }

    // ─── 复合类型辅助方法 ───

    /// 生成 match 分支体：可选 guard、分支结果清理，并跳转到 merge 点。
    fn gen_match_guard_and_body(
        &mut self,
        arm: &HirMatchArm,
        fn_sig: &FnSignature,
        unwanted: usize,
        guard_unwanted: usize,
        end_label: usize,
        next_label: usize,
    ) {
        let mut guard_skip = None;
        if let Some(guard) = &arm.guard {
            let skip_label = self.labels.fresh();
            self.gen_expr(guard, fn_sig);
            self.mir.push(MirInst::Not);
            self.mir.push(MirInst::Jump(skip_label));
            self.stack.pop();
            guard_skip = Some(skip_label);
        }

        let before_body = self.stack.len();
        self.gen_expr(&arm.body, fn_sig);
        let results = self.stack.len().saturating_sub(before_body);

        self.mir.push(MirInst::ExitScope);
        self.discard_below_results(unwanted, results);
        self.mir.push(MirInst::UncondJump(end_label));

        if let Some(skip_label) = guard_skip {
            self.mir.push(MirInst::Mark(skip_label));
            self.mir.push(MirInst::ExitScope);
            // guard 失败时只移除分支绑定的临时值，保留 scrutinee 供后续模式检查。
            self.discard_below_results(guard_unwanted, 0);
            self.mir.push(MirInst::UncondJump(next_label));
        }
    }

    /// 在保留栈顶 `results` 个结果的前提下，移除其下方的 `unwanted` 个槽。
    fn discard_below_results(&mut self, unwanted: usize, results: usize) {
        for i in 0..unwanted {
            let remaining = unwanted - i;
            let depth = results + remaining - 1;
            if depth > 0 {
                self.mir.push(MirInst::Roll { n: depth as i32 });
                self.stack.roll(depth as i32);
            }
            self.mir.push(MirInst::Pop { ty: TypeId::Unknown });
            self.stack.pop();
        }
    }

    /// 从模式路径解析枚举变体标签；支持 `Enum::Variant` 和 `mod::Enum::Variant`。
    fn variant_tag_for_pattern(&self, name: &[String], variant: &str) -> u32 {
        let full = name.join("::");
        let last = name.last().map(String::as_str).unwrap_or(full.as_str());
        self.type_table
            .variant_tag(&full, variant)
            .or_else(|| self.type_table.variant_tag(last, variant))
            .unwrap_or(0)
    }

    /// 从表达式推断枚举名（尝试所有已注册枚举）
    fn infer_enum_name(&self, expr: &HirExpr) -> String {
        match expr {
            HirExpr::EnumLit { name, .. } => name.join("::"),
            HirExpr::Var(_) => {
                // 遍历所有枚举，返回第一个（作为最佳猜测）
                // 未来需要类型标注来精确判断
                for (name, def) in self.type_table.iter() {
                    if matches!(def, crate::typeck::type_table::TypeDef::Enum(_)) {
                        return name.clone();
                    }
                }
                "?".to_string()
            }
            _ => "?".to_string(),
        }
    }

    /// 从表达式推断枚举总槽数
    fn infer_enum_slots(&self, expr: &HirExpr) -> u32 {
        match expr {
            HirExpr::EnumLit { name, .. } => {
                self.type_table.lookup_enum(&name.join("::"))
                    .map(|e| e.total_slots)
                    .unwrap_or(2)
            }
            _ => {
                // 尝试从已注册枚举获取槽数
                for (_, def) in self.type_table.iter() {
                    if let crate::typeck::type_table::TypeDef::Enum(e) = def {
                        return e.total_slots;
                    }
                }
                2
            }
        }
    }

    fn resolve_field_offset(&self, _expr: &HirExpr, field: &str) -> usize {
        // 搜索所有已注册结构体，找到包含该字段名的第一个
        for (_, type_def) in self.type_table.iter() {
            if let crate::typeck::type_table::TypeDef::Struct(s) = type_def {
                for f in &s.fields {
                    if f.name == field {
                        return f.index;
                    }
                }
            }
        }
        // 元组索引回退
        if let Ok(idx) = field.parse::<usize>() {
            return idx;
        }
        0
    }

    #[allow(dead_code)]
    fn get_variant_tag(&self, enum_name: &str, variant: &str) -> u32 {
        self.type_table.variant_tag(enum_name, variant).unwrap_or(0)
    }

    /// 检查函数名是否为内建/标准库函数
    fn is_builtin(name: &str) -> bool {
        crate::io::is_builtin(name)
    }

    /// 生成内建函数调用的 MIR 指令
    /// 调用前参数已在栈顶求值完毕
    fn gen_builtin_call(&mut self, name: &str) {
        match name {
            // print(value) / print_int(value) → NumOut 输出栈顶数值并换行
            "print" | "print_int" => {
                self.mir.push(MirInst::NumOut);
                self.stack.pop(); // pop 参数
            }
            "print_char" => {
                self.mir.push(MirInst::CharOut);
                self.stack.pop(); // pop 参数
            }
            "print_str" => {
                self.mir.push(MirInst::StrOut { newline: false });
                self.stack.pop(); // pop 参数
            }
            "println_str" => {
                self.mir.push(MirInst::StrOut { newline: true });
                self.stack.pop(); // pop 参数
            }
            "file_read" => {
                self.mir.push(MirInst::FileRead);
                self.stack.pop(); // path
                self.stack.push_temp(); // contents
            }
            "file_write" => {
                self.mir.push(MirInst::FileWrite);
                self.stack.pop(); // data
                self.stack.pop(); // path
            }
            "read_int" => {
                self.mir.push(MirInst::NumIn);
                self.stack.push_temp();
            }
            "read_char" => {
                self.mir.push(MirInst::CharIn);
                self.stack.push_temp();
            }
            "read_line" => {
                self.mir.push(MirInst::ReadLine);
                self.stack.push_temp();
            }
            _ => {
                self.mir.push(MirInst::Comment(format!(
                    "ERROR: unknown builtin '{name}'"
                )));
            }
        }
    }

    /// 将 stardust! 宏指令翻译为 MIR
    fn gen_stardust_inst(&mut self, inst: &crate::frontend::ast::StardustInst) {
        match inst {
            crate::frontend::ast::StardustInst::Push(v) => {
                self.mir.push(MirInst::Push { value: Value::I64(*v), ty: TypeId::Unknown });
                self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Pop => {
                self.mir.push(MirInst::Pop { ty: TypeId::Unknown });
                self.stack.pop();
            }
            crate::frontend::ast::StardustInst::Dup => {
                self.mir.push(MirInst::Dup { ty: TypeId::Unknown });
                self.stack.dup();
            }
            crate::frontend::ast::StardustInst::Swap => {
                self.mir.push(MirInst::Swap);
                self.stack.swap();
            }
            crate::frontend::ast::StardustInst::Add => {
                self.mir.push(MirInst::Add { ty: TypeId::Unknown });
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Sub => {
                self.mir.push(MirInst::Sub { ty: TypeId::Unknown });
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Mul => {
                self.mir.push(MirInst::Mul { ty: TypeId::Unknown });
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Div => {
                self.mir.push(MirInst::Div { ty: TypeId::Unknown });
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Mod => {
                self.mir.push(MirInst::Mod { ty: TypeId::Unknown });
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Eq => {
                self.mir.push(MirInst::Eq);
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Ne => {
                self.mir.push(MirInst::Ne);
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Lt => {
                self.mir.push(MirInst::Lt);
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Gt => {
                self.mir.push(MirInst::Gt);
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Le => {
                self.mir.push(MirInst::Le);
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Ge => {
                self.mir.push(MirInst::Ge);
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::And => {
                self.mir.push(MirInst::And);
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Or => {
                self.mir.push(MirInst::Or);
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Not => {
                self.mir.push(MirInst::Not);
                // Not: pop 1, push 1 — stack unchanged
            }
            crate::frontend::ast::StardustInst::Xor => {
                self.mir.push(MirInst::Xor);
                self.stack.pop(); self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Store => {
                self.mir.push(MirInst::RemoteStore { index: 0 });
                self.stack.pop(); self.stack.pop();
            }
            crate::frontend::ast::StardustInst::Load => {
                self.mir.push(MirInst::RemoteLoad { index: 0 });
                self.stack.pop(); self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::ShiftL => {
                self.mir.push(MirInst::Roll { n: -1 });
                self.stack.roll(-1);
            }
            crate::frontend::ast::StardustInst::ShiftR => {
                self.mir.push(MirInst::Roll { n: 1 });
                self.stack.roll(1);
            }
            crate::frontend::ast::StardustInst::Depth => {
                let len = self.stack.len() as i64;
                self.mir.push(MirInst::Push { value: Value::I64(len), ty: TypeId::Unknown });
                self.stack.push_temp();
            }
            crate::frontend::ast::StardustInst::Reverse => {
                self.mir.push(MirInst::Comment("stardust Reverse (not implemented)".to_string()));
            }
            crate::frontend::ast::StardustInst::Rotate => {
                self.mir.push(MirInst::Roll { n: 2 });
                self.stack.roll(2);
            }
            crate::frontend::ast::StardustInst::Call { n, m } => {
                self.mir.push(MirInst::Push {
                    value: Value::CallElement { argc: *n },
                    ty: TypeId::Unknown,
                });
                self.stack.push_temp();
                self.mir.push(MirInst::Call { fn_id: FnId(0), return_arity: *m });
                // 清理栈追踪
                for _ in 0..*n { self.stack.pop(); }
                self.stack.pop(); // CallElement
                for _ in 0..*m { self.stack.push_temp(); }
            }
            crate::frontend::ast::StardustInst::Ret { m } => {
                self.mir.push(MirInst::Ret { argc: *m });
            }
            crate::frontend::ast::StardustInst::Ident(_) => {
                self.mir.push(MirInst::Comment("stardust custom ident".to_string()));
            }
        }
    }
}

impl Default for MirGen {
    fn default() -> Self {
        Self::new()
    }
}

// ─── 辅助转换函数 ───

fn collect_pattern_names(pattern: &ast::Pattern, names: &mut Vec<String>) {
    match pattern {
        ast::Pattern::Var(name, _) => names.push(name.clone()),
        ast::Pattern::Tuple(elems, _, _) | ast::Pattern::Or(elems, _) => {
            for elem in elems {
                collect_pattern_names(elem, names);
            }
        }
        ast::Pattern::Struct { fields, .. } => {
            for (_, sub) in fields {
                if let Some(sub) = sub {
                    collect_pattern_names(sub, names);
                }
            }
        }
        ast::Pattern::Enum { inner, .. } => {
            if let Some(inner) = inner {
                collect_pattern_names(inner, names);
            }
        }
        ast::Pattern::Ref(inner, _, _) => collect_pattern_names(inner, names),
        ast::Pattern::Lit(_, _) | ast::Pattern::Wildcard(_) => {}
    }
}

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
