use std::collections::HashMap;

use crate::frontend::ast;
use crate::frontend::token::Span;
use crate::hir::types::{
    HirAssignTarget, HirBlock, HirCallArg, HirDecl, HirExpr, HirFnDecl, HirProgram, HirStmt, HirType,
};

/// 借用状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BorrowState {
    /// 未借用，可自由使用
    Unused,
    /// 共享借用（计数）
    Shared(usize),
    /// 独占借用（可变引用）
    Exclusive,
    /// 已移动（不可再使用）
    Moved,
}

/// 借用检查错误
#[derive(Debug, Clone)]
pub enum BorrowError {
    UseAfterMove { name: String, span: Span },
    UseWhileBorrowedMut { name: String, span: Span },
    BorrowWhileBorrowedMut { name: String, span: Span },
    MutBorrowWhileBorrowed { name: String, span: Span },
    MoveWhileBorrowed { name: String, span: Span },
    CannotAssignToImmutable { name: String, span: Span },
}

impl std::fmt::Display for BorrowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BorrowError::UseAfterMove { name, span } => {
                write!(f, "{span}: use of moved value `{name}`")
            }
            BorrowError::UseWhileBorrowedMut { name, span } => {
                write!(f, "{span}: cannot use `{name}` while it is mutably borrowed")
            }
            BorrowError::BorrowWhileBorrowedMut { name, span } => {
                write!(f, "{span}: cannot borrow `{name}` while it is mutably borrowed")
            }
            BorrowError::MutBorrowWhileBorrowed { name, span } => {
                write!(f, "{span}: cannot mutably borrow `{name}` while it is borrowed")
            }
            BorrowError::MoveWhileBorrowed { name, span } => {
                write!(f, "{span}: cannot move `{name}` while it is borrowed")
            }
            BorrowError::CannotAssignToImmutable { name, span } => {
                write!(f, "{span}: cannot assign to immutable variable `{name}`")
            }
        }
    }
}

/// 变量信息
#[derive(Debug, Clone)]
struct VarInfo {
    /// 是否可变
    mutable: bool,
    /// 是否为 Move 类型（Vec, String, Box）
    is_move_type: bool,
    /// 当前借用状态
    borrow_state: BorrowState,
    /// 声明位置所在的 scope 深度
    scope_depth: usize,
}

/// 借用检查器 — 在 HIR 层面验证所有权和借用规则
pub struct BorrowChecker {
    /// 变量名 → 同名绑定栈（最后一个为当前可见绑定）
    vars: HashMap<String, Vec<VarInfo>>,
    /// 当前作用域深度
    scope_depth: usize,
    /// 作用域帧栈（每帧记录该作用域声明的变量名）
    scope_vars: Vec<Vec<String>>,
    /// 错误列表
    pub errors: Vec<BorrowError>,
    /// 引用变量名 → 各层同名绑定的被借用变量名
    /// （最后一个元素为当前可见绑定；`None` 表示该层借用已释放）
    ref_targets: HashMap<String, Vec<Option<String>>>,
    /// 每个作用域中声明的引用变量名
    ref_scopes: Vec<Vec<String>>,
    /// 变量名在函数体中的剩余使用次数（用于近似 NLL 的借用释放）
    uses_left: HashMap<String, usize>,
}

impl BorrowChecker {
    pub fn new() -> Self {
        Self {
            vars: HashMap::new(),
            scope_depth: 0,
            scope_vars: Vec::new(),
            errors: Vec::new(),
            ref_targets: HashMap::new(),
            ref_scopes: Vec::new(),
            uses_left: HashMap::new(),
        }
    }

    /// 进入新作用域
    pub fn enter_scope(&mut self) {
        self.scope_depth += 1;
        self.scope_vars.push(Vec::new());
        self.ref_scopes.push(Vec::new());
    }

    /// 退出作用域（释放该作用域内声明的所有变量）
    pub fn exit_scope(&mut self) -> Vec<String> {
        if let Some(refs) = self.ref_scopes.pop() {
            for name in refs.into_iter().rev() {
                self.pop_ref_binding(&name);
            }
        }
        let dropped = self.scope_vars.pop().unwrap_or_default();
        // 释放已移动的变量不报错（它们已经被转移了所有权）
        for name in &dropped {
            if let Some(stack) = self.vars.get_mut(name) {
                if let Some(info) = stack.last()
                    && info.is_move_type
                    && info.borrow_state != BorrowState::Moved
                {
                    // Move 类型在此处应该被析构（Drop），但目前静默处理
                }
                stack.pop();
                if stack.is_empty() {
                    self.vars.remove(name);
                }
            }
        }
        self.scope_depth = self.scope_depth.saturating_sub(1);
        dropped
    }

    /// 当前作用域可见的变量信息。
    fn var(&self, name: &str) -> Option<&VarInfo> {
        self.vars.get(name).and_then(|stack| stack.last())
    }

    /// 当前作用域可见变量信息的可变引用。
    fn var_mut(&mut self, name: &str) -> Option<&mut VarInfo> {
        self.vars.get_mut(name).and_then(|stack| stack.last_mut())
    }

    /// 获取待析构变量的 Drop 顺序（逆序创建）
    pub fn drop_order(&self) -> Vec<String> {
        let mut order: Vec<String> = self
            .vars
            .iter()
            .filter_map(|(name, stack)| {
                stack
                    .last()
                    .filter(|info| info.is_move_type && info.borrow_state != BorrowState::Moved)
                    .map(|_| name.clone())
            })
            .collect();
        // 逆序：后创建的先析构
        order.reverse();
        order
    }

    /// 声明新变量
    pub fn declare_var(
        &mut self,
        name: &str,
        mutable: bool,
        is_move_type: bool,
        span: Span,
    ) -> Result<(), BorrowError> {
        if let Some(existing) = self.var(name)
            && existing.scope_depth == self.scope_depth {
                return Err(BorrowError::CannotAssignToImmutable {
                    name: format!("duplicate variable `{name}`"),
                    span,
                });
            }
        self.vars.entry(name.to_string()).or_default().push(VarInfo {
            mutable,
            is_move_type,
            borrow_state: BorrowState::Unused,
            scope_depth: self.scope_depth,
        });
        if let Some(frame) = self.scope_vars.last_mut() {
            frame.push(name.to_string());
        }
        Ok(())
    }

    /// 检查变量读取（Copy 类型：直接读取；Move 类型：检查是否已移动）
    pub fn check_read(&mut self, name: &str, span: Span) -> Result<(), BorrowError> {
        let info = self.var(name).ok_or_else(|| BorrowError::UseAfterMove {
            name: format!("undefined variable `{name}`"),
            span,
        })?;

        match &info.borrow_state {
            BorrowState::Moved => Err(BorrowError::UseAfterMove {
                name: name.to_string(),
                span,
            }),
            BorrowState::Exclusive => Err(BorrowError::UseWhileBorrowedMut {
                name: name.to_string(),
                span,
            }),
            _ => Ok(()),
        }
    }

    /// 检查变量写入（赋值）
    pub fn check_write(&mut self, name: &str, span: Span) -> Result<(), BorrowError> {
        let info = self.var_mut(name).ok_or_else(|| BorrowError::UseAfterMove {
            name: format!("undefined variable `{name}`"),
            span,
        })?;

        if !info.mutable {
            return Err(BorrowError::CannotAssignToImmutable {
                name: name.to_string(),
                span,
            });
        }

        match &info.borrow_state {
            BorrowState::Moved => Err(BorrowError::UseAfterMove {
                name: name.to_string(),
                span,
            }),
            BorrowState::Exclusive => Err(BorrowError::UseWhileBorrowedMut {
                name: name.to_string(),
                span,
            }),
            BorrowState::Shared(n) if *n > 0 => Err(BorrowError::UseWhileBorrowedMut {
                name: name.to_string(),
                span,
            }),
            _ => Ok(()),
        }
    }

    /// 创建共享引用 &x
    pub fn borrow_shared(&mut self, name: &str, span: Span) -> Result<(), BorrowError> {
        let info = self.var_mut(name).ok_or_else(|| BorrowError::UseAfterMove {
            name: format!("undefined variable `{name}`"),
            span,
        })?;

        match &info.borrow_state {
            BorrowState::Moved => Err(BorrowError::UseAfterMove {
                name: name.to_string(),
                span,
            }),
            BorrowState::Exclusive => Err(BorrowError::BorrowWhileBorrowedMut {
                name: name.to_string(),
                span,
            }),
            BorrowState::Shared(n) => {
                info.borrow_state = BorrowState::Shared(n + 1);
                Ok(())
            }
            BorrowState::Unused => {
                info.borrow_state = BorrowState::Shared(1);
                Ok(())
            }
        }
    }

    /// 创建独占引用 &mut x
    pub fn borrow_mut(&mut self, name: &str, span: Span) -> Result<(), BorrowError> {
        let info = self.var_mut(name).ok_or_else(|| BorrowError::UseAfterMove {
            name: format!("undefined variable `{name}`"),
            span,
        })?;

        match &info.borrow_state {
            BorrowState::Moved => Err(BorrowError::UseAfterMove {
                name: name.to_string(),
                span,
            }),
            BorrowState::Shared(n) if *n > 0 => Err(BorrowError::MutBorrowWhileBorrowed {
                name: name.to_string(),
                span,
            }),
            BorrowState::Exclusive => Err(BorrowError::BorrowWhileBorrowedMut {
                name: name.to_string(),
                span,
            }),
            _ => {
                if !info.mutable {
                    return Err(BorrowError::CannotAssignToImmutable {
                        name: format!("cannot mutably borrow immutable variable `{name}`"),
                        span,
                    });
                }
                info.borrow_state = BorrowState::Exclusive;
                Ok(())
            }
        }
    }

    /// 释放引用（引用离开作用域时）
    pub fn release_ref(&mut self, name: &str) {
        if let Some(info) = self.var_mut(name) {
            match info.borrow_state {
                BorrowState::Shared(n) if n > 1 => {
                    info.borrow_state = BorrowState::Shared(n - 1);
                }
                BorrowState::Shared(_) | BorrowState::Exclusive => {
                    info.borrow_state = BorrowState::Unused;
                }
                _ => {}
            }
        }
    }

    /// 移动变量（赋值或传参时所有权转移）
    pub fn move_var(&mut self, name: &str, span: Span) -> Result<(), BorrowError> {
        let info = self.var(name).ok_or_else(|| BorrowError::UseAfterMove {
            name: format!("undefined variable `{name}`"),
            span,
        })?;

        match &info.borrow_state {
            BorrowState::Moved => Err(BorrowError::UseAfterMove {
                name: name.to_string(),
                span,
            }),
            BorrowState::Shared(n) if *n > 0 => Err(BorrowError::MoveWhileBorrowed {
                name: name.to_string(),
                span,
            }),
            BorrowState::Exclusive => Err(BorrowError::MoveWhileBorrowed {
                name: name.to_string(),
                span,
            }),
            _ => Ok(()),
        }?;

        if let Some(info) = self.var_mut(name) {
            info.borrow_state = BorrowState::Moved;
        }
        Ok(())
    }

    /// 重新初始化变量（赋值给已移动的变量时允许恢复）。
    pub fn reinit_var(&mut self, name: &str, span: Span) -> Result<(), BorrowError> {
        let info = self.var_mut(name).ok_or_else(|| BorrowError::UseAfterMove {
            name: format!("undefined variable `{name}`"),
            span,
        })?;
        if !info.mutable {
            return Err(BorrowError::CannotAssignToImmutable {
                name: name.to_string(),
                span,
            });
        }
        match &info.borrow_state {
            BorrowState::Shared(n) if *n > 0 => Err(BorrowError::UseWhileBorrowedMut {
                name: name.to_string(),
                span,
            }),
            BorrowState::Exclusive => Err(BorrowError::UseWhileBorrowedMut {
                name: name.to_string(),
                span,
            }),
            _ => {
                info.borrow_state = BorrowState::Unused;
                Ok(())
            }
        }
    }

    /// 记录一次变量使用；引用变量最后一次使用后释放借用（近似 NLL）。
    pub fn note_use(&mut self, name: &str) {
        if let Some(n) = self.uses_left.get_mut(name)
            && *n > 0
        {
            *n -= 1;
            if *n == 0 {
                self.release_ref_binding(name);
            }
        }
    }

    /// 声明引用变量并记录对目标变量的借用。
    pub fn declare_ref_var(
        &mut self,
        name: &str,
        target: &str,
        mutable: bool,
        span: Span,
    ) -> Result<(), BorrowError> {
        self.declare_var(name, false, false, span)?;
        if mutable {
            self.borrow_mut(target, span)?;
        } else {
            self.borrow_shared(target, span)?;
        }
        self.ref_targets
            .entry(name.to_string())
            .or_default()
            .push(Some(target.to_string()));
        if let Some(frame) = self.ref_scopes.last_mut() {
            frame.push(name.to_string());
        }
        // 引用变量从未被使用：借用立即结束（近似 NLL）。
        if self.uses_left.get(name).copied().unwrap_or(0) == 0 {
            self.release_ref_binding(name);
        }
        Ok(())
    }

    /// 释放当前可见引用变量对应的借用（幂等）。
    pub fn release_ref_binding(&mut self, name: &str) {
        let target = self
            .ref_targets
            .get_mut(name)
            .and_then(|stack| stack.last_mut())
            .and_then(|target| target.take());
        if let Some(target) = target {
            self.release_ref(&target);
        }
    }

    /// 离开作用域：释放并弹出该层引用绑定，恢复外层同名绑定。
    fn pop_ref_binding(&mut self, name: &str) {
        self.release_ref_binding(name);
        if let Some(stack) = self.ref_targets.get_mut(name) {
            stack.pop();
            if stack.is_empty() {
                self.ref_targets.remove(name);
            }
        }
    }

    /// 设置当前函数体内各变量的剩余使用次数。
    pub fn set_uses_left(&mut self, uses: HashMap<String, usize>) {
        self.uses_left = uses;
    }

    /// 判断变量是否为 Move 类型
    pub fn is_move_type(&self, name: &str) -> bool {
        self.var(name).is_some_and(|v| v.is_move_type)
    }

    /// 查询变量是否可变
    pub fn is_mutable(&self, name: &str) -> bool {
        self.var(name).is_some_and(|v| v.mutable)
    }

    // ─── HIR 遍历 ───

    /// 检查整个程序的所有函数体。
    pub fn check_program(&mut self, program: &HirProgram) {
        for decl in &program.decls {
            match decl {
                HirDecl::Fn(f) => self.check_fn(f),
                HirDecl::Extern(block) => {
                    for item in &block.items {
                        let crate::hir::types::HirExternItem::Fn(f) = item;
                        self.check_fn(f);
                    }
                }
                _ => {}
            }
        }
    }

    fn check_fn(&mut self, f: &HirFnDecl) {
        let Some(body) = &f.body else { return };
        let mut uses = HashMap::new();
        count_uses_block(body, &mut uses);
        self.set_uses_left(uses);

        self.enter_scope();
        for (name, ty) in &f.sig.params {
            let move_ty = is_move_hir_type(ty);
            // `&mut self`（以及 `&mut T` 参数）在方法体内需要可写。
            let mutable = matches!(ty, HirType::Ref(_, true));
            let result = self.declare_var(name, mutable, move_ty, f.sig.span);
            self.record(result);
        }
        self.check_block(body);
        self.exit_scope();
    }

    fn record(&mut self, result: Result<(), BorrowError>) {
        if let Err(error) = result {
            self.errors.push(error);
        }
    }

    fn check_block(&mut self, block: &HirBlock) {
        self.enter_scope();
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        if let Some(tail) = &block.tail {
            self.check_expr(tail);
        }
        self.exit_scope();
    }

    fn check_stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Let { pattern, ty, init, mutable, span } => {
                let pattern_names = pattern_var_names(pattern);

                if let (Some(HirExpr::Ref(inner, ref_mutable, _)), ast::Pattern::Var(name, _)) =
                    (init.as_ref(), pattern)
                    && let HirExpr::Var(target) = inner.as_ref()
                {
                    let result = self.declare_ref_var(name, &target.name, *ref_mutable, *span);
                    self.record(result);
                    self.note_use(&target.name);
                    return;
                }

                if let Some(init_expr) = init {
                    self.check_expr(init_expr);
                }

                let annotated_move = ty.as_ref().is_some_and(is_move_hir_type);
                let inferred_move = init.as_ref().is_some_and(expr_is_move_value);
                let source_move = match init.as_ref() {
                    Some(HirExpr::Var(source)) => self.is_move_type(&source.name),
                    _ => false,
                };
                let move_ty = annotated_move || inferred_move || source_move;

                for name in pattern_names {
                    let result = self.declare_var(&name, *mutable, move_ty, *span);
                    self.record(result);
                }

                if move_ty
                    && let Some(HirExpr::Var(source)) = init.as_ref()
                {
                    let result = self.move_var(&source.name, *span);
                    self.record(result);
                    self.note_use(&source.name);
                }
            }
            HirStmt::Expr(expr, _) => self.check_expr(expr),
            HirStmt::Assign { target, value, span } => {
                self.check_expr(value);
                self.move_value_if_var(value, *span);

                match target {
                    HirAssignTarget::Var(var) => {
                        if self.is_move_type(&var.name) {
                            let result = self.reinit_var(&var.name, *span);
                            self.record(result);
                        } else {
                            let result = self.check_write(&var.name, *span);
                            self.record(result);
                        }
                    }
                    HirAssignTarget::Deref(inner) => self.check_expr(inner),
                    HirAssignTarget::Field { object, .. } => {
                        self.check_expr(object);
                        if let Some(root) = root_var_name(object).map(str::to_string) {
                            let result = self.check_write(&root, *span);
                            self.record(result);
                        }
                    }
                    HirAssignTarget::Index { array, index } => {
                        self.check_expr(array);
                        self.check_expr(index);
                        if let Some(root) = root_var_name(array).map(str::to_string) {
                            let result = self.check_write(&root, *span);
                            self.record(result);
                        }
                    }
                }
            }
        }
    }

    fn check_expr(&mut self, expr: &HirExpr) {
        match expr {
            HirExpr::Lit(_, _) | HirExpr::Ret(_, _) => {}
            HirExpr::Var(var) => {
                if var.scope_depth == usize::MAX {
                    // 函数名/类型名不是变量，借用检查忽略。
                    return;
                }
                let result = self.check_read(&var.name, var.span);
                self.record(result);
                self.note_use(&var.name);
            }
            HirExpr::IdRef(name, span) => {
                let result = self.check_read(name, *span);
                self.record(result);
                self.note_use(name);
            }
            HirExpr::Deref(inner, _) => self.check_expr(inner),
            HirExpr::TypeApplied { callee, .. } => self.check_expr(callee),
            HirExpr::Ref(inner, mutable, span) => {
                self.check_expr(inner);
                if let HirExpr::Var(target) = inner.as_ref()
                    && self.vars.contains_key(&target.name)
                {
                    let result = if *mutable {
                        self.borrow_mut(&target.name, *span)
                    } else {
                        self.borrow_shared(&target.name, *span)
                    };
                    self.record(result);
                    // 表达式结束后借用立即结束。
                    self.release_ref(&target.name);
                }
            }
            HirExpr::Binary(lhs, _, rhs, _) => {
                self.check_expr(lhs);
                self.check_expr(rhs);
            }
            HirExpr::Unary(_, inner, _) => self.check_expr(inner),
            HirExpr::Call { args, fn_sig, .. } => {
                let params: Vec<HirType> = fn_sig
                    .as_ref()
                    .map(|sig| sig.params.iter().map(|(_, ty)| ty.clone()).collect())
                    .unwrap_or_default();

                for (i, arg) in args.iter().enumerate() {
                    let param_ty = params.get(i);
                    let arg_expr = match arg {
                        HirCallArg::Pos(e, _) | HirCallArg::Named(_, e, _) => e,
                    };

                    if let (Some(_ty), HirExpr::Ref(inner, mutable, span)) = (param_ty, arg_expr)
                        && let HirExpr::Var(target) = inner.as_ref()
                    {
                        let result = if *mutable {
                            self.borrow_mut(&target.name, *span)
                        } else {
                            self.borrow_shared(&target.name, *span)
                        };
                        self.record(result);
                        self.note_use(&target.name);
                        self.release_ref(&target.name);
                        continue;
                    }

                    if param_ty.is_some_and(|ty| is_move_hir_type(ty))
                        && let HirExpr::Var(var) = arg_expr
                        && self.is_move_type(&var.name)
                    {
                        let result = self.move_var(&var.name, var.span);
                        self.record(result);
                        self.note_use(&var.name);
                        continue;
                    }

                    self.check_expr(arg_expr);
                }
            }
            HirExpr::MethodCall { receiver, method, args, span } => {
                match (method.as_str(), receiver.as_ref()) {
                    ("push" | "pop", HirExpr::Var(var)) => {
                        let result = self.borrow_mut(&var.name, *span);
                        self.record(result);
                        self.note_use(&var.name);
                        for arg in args {
                            self.check_expr(arg);
                        }
                        self.release_ref(&var.name);
                    }
                    ("len", HirExpr::Var(var)) => {
                        let result = self.borrow_shared(&var.name, *span);
                        self.record(result);
                        self.note_use(&var.name);
                        for arg in args {
                            self.check_expr(arg);
                        }
                        self.release_ref(&var.name);
                    }
                    _ => {
                        self.check_expr(receiver);
                        for arg in args {
                            self.check_expr(arg);
                        }
                    }
                }
            }
            HirExpr::TypeMethod { type_name, method, args, span } => {
                let name = type_name.join("::");
                if name == "Box" && method == "new" {
                    for arg in args {
                        if let HirExpr::Var(var) = arg
                            && self.is_move_type(&var.name)
                        {
                            let result = self.move_var(&var.name, *span);
                            self.record(result);
                            self.note_use(&var.name);
                        } else {
                            self.check_expr(arg);
                        }
                    }
                } else {
                    for arg in args {
                        self.check_expr(arg);
                    }
                }
            }
            HirExpr::FieldAccess(inner, _, _) => self.check_expr(inner),
            HirExpr::Index(inner, index, _) => {
                self.check_expr(inner);
                self.check_expr(index);
            }
            HirExpr::Tuple(elems, _) | HirExpr::Array(elems, _) => {
                for e in elems {
                    self.check_expr(e);
                }
            }
            HirExpr::Repeat(elem, count, _) => {
                self.check_expr(elem);
                self.check_expr(count);
            }
            HirExpr::StructLit { fields, .. } => {
                for (_, e) in fields {
                    self.check_expr(e);
                }
            }
            HirExpr::EnumLit { data, .. } => {
                if let Some(e) = data {
                    self.check_expr(e);
                }
            }
            HirExpr::If(cond, then_block, else_block, _) => {
                self.check_expr(cond);
                self.check_block(then_block);
                if let Some(block) = else_block {
                    self.check_block(block);
                }
            }
            HirExpr::Match(scrutinee, arms, _) => {
                self.check_expr(scrutinee);
                for arm in arms {
                    self.enter_scope();
                    for name in pattern_var_names(&arm.pattern) {
                        let result = self.declare_var(&name, false, false, arm.span);
                        self.record(result);
                    }
                    if let Some(guard) = &arm.guard {
                        self.check_expr(guard);
                    }
                    self.check_expr(&arm.body);
                    self.exit_scope();
                }
            }
            HirExpr::Loop(block, _) | HirExpr::Unsafe(block, _) => self.check_block(block),
            HirExpr::While(cond, block, _) => {
                self.check_expr(cond);
                self.check_block(block);
            }
            HirExpr::For { pattern, iterable, body, span } => {
                self.check_expr(iterable);
                self.enter_scope();
                for name in pattern_var_names(pattern) {
                    let result = self.declare_var(&name, false, false, *span);
                    self.record(result);
                }
                self.check_block(body);
                self.exit_scope();
            }
            HirExpr::Block(block, _) => self.check_block(block),
            HirExpr::Labelled(_, inner, _) => self.check_expr(inner),
            HirExpr::BreakLabel(_, value, _) => {
                if let Some(e) = value {
                    self.check_expr(e);
                }
            }
            HirExpr::ContinueLabel(_, _) => {}
            HirExpr::Break(value, _) => {
                if let Some(e) = value {
                    self.check_expr(e);
                }
            }
            HirExpr::Continue(_) => {}
            HirExpr::Return(exprs, _) => {
                for e in exprs {
                    self.check_expr(e);
                }
            }
            HirExpr::StardustMacro(_, _) => {}
            HirExpr::As(inner, _, _) => self.check_expr(inner),
        }
    }

    fn move_value_if_var(&mut self, value: &HirExpr, span: Span) {
        if let HirExpr::Var(var) = value
            && self.is_move_type(&var.name)
        {
            let result = self.move_var(&var.name, span);
            self.record(result);
            self.note_use(&var.name);
        }
    }
}

/// 收集模式中绑定的变量名。
fn pattern_var_names(pattern: &ast::Pattern) -> Vec<String> {
    match pattern {
        ast::Pattern::Var(name, _) => vec![name.clone()],
        ast::Pattern::Tuple(elems, _, _) | ast::Pattern::Or(elems, _) => {
            elems.iter().flat_map(pattern_var_names).collect()
        }
        ast::Pattern::Struct { fields, .. } => fields
            .iter()
            .flat_map(|(field, p)| match p {
                Some(p) => pattern_var_names(p),
                // 字段简写 `Point { x, y }` 绑定同名字段。
                None => vec![field.clone()],
            })
            .collect(),
        ast::Pattern::Enum { inner, .. } => inner
            .as_ref()
            .map(|p| pattern_var_names(p))
            .unwrap_or_default(),
        ast::Pattern::Ref(inner, _, _) => pattern_var_names(inner),
        ast::Pattern::Lit(_, _) | ast::Pattern::Wildcard(_) => Vec::new(),
    }
}

/// 找到字段/索引赋值目标的根变量。
fn root_var_name(expr: &HirExpr) -> Option<&str> {
    match expr {
        HirExpr::Var(var) => Some(&var.name),
        HirExpr::FieldAccess(inner, _, _) | HirExpr::Index(inner, _, _) => root_var_name(inner),
        _ => None,
    }
}

/// 判断 HIR 类型是否为 Move 类型。
fn is_move_hir_type(ty: &HirType) -> bool {
    match ty {
        HirType::Path(segments) => {
            let name = segments.join("::");
            matches!(name.as_str(), "String" | "Vec" | "Box" | "StackHandle")
        }
        HirType::Generic(base, _) => matches!(base.name().as_str(), "Vec" | "Box"),
        _ => false,
    }
}

/// 判断表达式是否直接产生 Move 类型的新值。
fn expr_is_move_value(expr: &HirExpr) -> bool {
    match expr {
        HirExpr::Lit(ast::Literal::String(_), _) => true,
        HirExpr::Array(_, _) | HirExpr::Repeat(_, _, _) => true,
        HirExpr::TypeMethod { type_name, method, .. } => {
            method == "new"
                && type_name
                    .last()
                    .is_some_and(|name| matches!(name.as_str(), "Vec" | "String" | "Box"))
        }
        _ => false,
    }
}

fn count_uses_block(block: &HirBlock, uses: &mut HashMap<String, usize>) {
    for stmt in &block.stmts {
        count_uses_stmt(stmt, uses);
    }
    if let Some(tail) = &block.tail {
        count_uses_expr(tail, uses);
    }
}

fn count_uses_stmt(stmt: &HirStmt, uses: &mut HashMap<String, usize>) {
    match stmt {
        HirStmt::Let { init, .. } => {
            if let Some(init) = init {
                count_uses_expr(init, uses);
            }
        }
        HirStmt::Expr(expr, _) => count_uses_expr(expr, uses),
        HirStmt::Assign { target, value, .. } => {
            count_uses_expr(value, uses);
            match target {
                HirAssignTarget::Var(_) => {}
                HirAssignTarget::Deref(inner) => count_uses_expr(inner, uses),
                HirAssignTarget::Field { object, .. } => count_uses_expr(object, uses),
                HirAssignTarget::Index { array, index } => {
                    count_uses_expr(array, uses);
                    count_uses_expr(index, uses);
                }
            }
        }
    }
}

fn count_uses_expr(expr: &HirExpr, uses: &mut HashMap<String, usize>) {
    match expr {
        HirExpr::Var(var) => {
            if var.scope_depth != usize::MAX {
                *uses.entry(var.name.clone()).or_insert(0) += 1;
            }
        }
        HirExpr::IdRef(name, _) => {
            *uses.entry(name.clone()).or_insert(0) += 1;
        }
        HirExpr::Lit(_, _)
        | HirExpr::Ret(_, _)
        | HirExpr::Continue(_)
        | HirExpr::ContinueLabel(_, _)
        | HirExpr::StardustMacro(_, _) => {}
        HirExpr::Deref(inner, _)
        | HirExpr::Unary(_, inner, _)
        | HirExpr::FieldAccess(inner, _, _)
        | HirExpr::As(inner, _, _)
        | HirExpr::TypeApplied { callee: inner, .. } => count_uses_expr(inner, uses),
        HirExpr::Ref(inner, _, _) => count_uses_expr(inner, uses),
        HirExpr::Binary(lhs, _, rhs, _) | HirExpr::Index(lhs, rhs, _) => {
            count_uses_expr(lhs, uses);
            count_uses_expr(rhs, uses);
        }
        HirExpr::Call { args, .. } => {
            for arg in args {
                match arg {
                    HirCallArg::Pos(e, _) | HirCallArg::Named(_, e, _) => count_uses_expr(e, uses),
                }
            }
        }
        HirExpr::MethodCall { receiver, args, .. } => {
            count_uses_expr(receiver, uses);
            for arg in args {
                count_uses_expr(arg, uses);
            }
        }
        HirExpr::TypeMethod { args, .. } => {
            for arg in args {
                count_uses_expr(arg, uses);
            }
        }
        HirExpr::Tuple(elems, _) | HirExpr::Array(elems, _) => {
            for e in elems {
                count_uses_expr(e, uses);
            }
        }
        HirExpr::Repeat(elem, count, _) => {
            count_uses_expr(elem, uses);
            count_uses_expr(count, uses);
        }
        HirExpr::StructLit { fields, .. } => {
            for (_, e) in fields {
                count_uses_expr(e, uses);
            }
        }
        HirExpr::EnumLit { data, .. } => {
            if let Some(e) = data {
                count_uses_expr(e, uses);
            }
        }
        HirExpr::If(cond, then_block, else_block, _) => {
            count_uses_expr(cond, uses);
            count_uses_block(then_block, uses);
            if let Some(block) = else_block {
                count_uses_block(block, uses);
            }
        }
        HirExpr::Match(scrutinee, arms, _) => {
            count_uses_expr(scrutinee, uses);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    count_uses_expr(guard, uses);
                }
                count_uses_expr(&arm.body, uses);
            }
        }
        HirExpr::Loop(block, _) | HirExpr::Unsafe(block, _) | HirExpr::Block(block, _) => {
            count_uses_block(block, uses);
        }
        HirExpr::Labelled(_, inner, _) => count_uses_expr(inner, uses),
        HirExpr::BreakLabel(_, value, _) => {
            if let Some(e) = value {
                count_uses_expr(e, uses);
            }
        }
        HirExpr::While(cond, block, _) => {
            count_uses_expr(cond, uses);
            count_uses_block(block, uses);
        }
        HirExpr::For { iterable, body, .. } => {
            count_uses_expr(iterable, uses);
            count_uses_block(body, uses);
        }
        HirExpr::Break(value, _) => {
            if let Some(e) = value {
                count_uses_expr(e, uses);
            }
        }
        HirExpr::Return(exprs, _) => {
            for e in exprs {
                count_uses_expr(e, uses);
            }
        }
    }
}

impl Default for BorrowChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::token::Span;

    fn span() -> Span {
        Span::new(1, 1, 0)
    }

    #[test]
    fn test_shared_borrow() {
        let mut bc = BorrowChecker::new();
        bc.enter_scope();
        bc.declare_var("x", false, false, span()).unwrap();
        bc.borrow_shared("x", span()).unwrap();
        // 再次共享借用 OK
        bc.borrow_shared("x", span()).unwrap();
        // 不能可变借用
        assert!(bc.borrow_mut("x", span()).is_err());
    }

    #[test]
    fn test_exclusive_borrow() {
        let mut bc = BorrowChecker::new();
        bc.enter_scope();
        bc.declare_var("x", true, false, span()).unwrap();
        bc.borrow_mut("x", span()).unwrap();
        // 不能再共享借用
        assert!(bc.borrow_shared("x", span()).is_err());
    }

    #[test]
    fn test_move_semantics() {
        let mut bc = BorrowChecker::new();
        bc.enter_scope();
        bc.declare_var("v", false, true, span()).unwrap(); // Move type
        bc.move_var("v", span()).unwrap();
        // 移动后不可再使用
        assert!(bc.check_read("v", span()).is_err());
    }

    #[test]
    fn test_release_ref() {
        let mut bc = BorrowChecker::new();
        bc.enter_scope();
        bc.declare_var("x", true, false, span()).unwrap();
        bc.borrow_mut("x", span()).unwrap();
        bc.release_ref("x");
        // 释放后可重新借用
        assert!(bc.borrow_shared("x", span()).is_ok());
    }

    #[test]
    fn test_cannot_assign_immutable() {
        let mut bc = BorrowChecker::new();
        bc.enter_scope();
        bc.declare_var("x", false, false, span()).unwrap();
        assert!(bc.check_write("x", span()).is_err());
    }
}
