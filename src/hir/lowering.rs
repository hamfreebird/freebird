use std::collections::HashMap;

use crate::frontend::ast;
use crate::frontend::token::Span;
use crate::hir::resolver::{ResolveError, Resolver};
use crate::hir::types::*;

/// AST → HIR 降级器
pub struct Lowering {
    resolver: Resolver,
    errors: Vec<ResolveError>,
    /// `use Type::item;` 的别名表：别名 → 限定名。
    aliases: HashMap<String, String>,
}

impl Lowering {
    pub fn new() -> Self {
        Self {
            resolver: Resolver::new(),
            errors: Vec::new(),
            aliases: HashMap::new(),
        }
    }

    pub fn errors(&self) -> &[ResolveError] {
        &self.errors
    }

    /// 降级整个程序
    pub fn lower_program(&mut self, ast_program: &ast::Program) -> HirProgram {
        // 第 0 遍：`use Type::item;` 建立别名表（别名 → 限定名）。
        for decl in &ast_program.decls {
            if let ast::Decl::Use(path, _) = decl
                && let Some(alias) = path.last()
            {
                self.aliases.insert(alias.clone(), path.join("::"));
            }
        }

        // 第一遍：收集所有函数签名
        self.collect_fn_signatures(ast_program);

        // 第二遍：降级所有声明
        let mut decls = Vec::new();
        for decl in &ast_program.decls {
            // `impl` 块展开为若干个扁平化函数（`Type::name`）。
            if let ast::Decl::Impl(i) = decl {
                for f in expand_impl_decls(i) {
                    if let Some(hir) = self.lower_fn(&f) {
                        decls.push(HirDecl::Fn(hir));
                    }
                }
                continue;
            }
            if let Some(hir_decl) = self.lower_decl(decl) {
                decls.push(hir_decl);
            }
        }

        self.errors.append(&mut self.resolver.errors);

        HirProgram {
            decls,
            fn_signatures: self.resolver.fn_signatures.clone(),
        }
    }

    // ─── 第一遍：收集函数签名 ───

    fn collect_fn_signatures(&mut self, program: &ast::Program) {
        for decl in &program.decls {
            match decl {
                ast::Decl::Fn(f) => {
                    let sig = self.fn_decl_to_signature(f);
                    self.resolver.register_fn(sig);
                }
                ast::Decl::Extern(ext) => {
                    for item in &ext.items {
                        #[allow(irrefutable_let_patterns)]
                        let ast::ExternItem::Fn(f) = item;
                        let sig = self.fn_decl_to_signature(f);
                        self.resolver.register_fn(sig);
                    }
                }
                ast::Decl::Impl(i) => {
                    for f in expand_impl_decls(i) {
                        let sig = self.fn_decl_to_signature(&f);
                        self.resolver.register_fn(sig);
                    }
                }
                _ => {}
            }
        }
    }

    fn fn_decl_to_signature(&self, f: &ast::FnDecl) -> FnSignature {
        let params: Vec<(String, HirType)> = f
            .params
            .iter()
            .map(|p| (p.name.clone(), HirType::from_ast_type(&p.ty)))
            .collect();
        let n = params.len() as u32;

        let return_types: Vec<HirType> = match &f.return_type {
            Some(ast::Type::Tuple(types, _)) => {
                types.iter().map(HirType::from_ast_type).collect()
            }
            Some(ty) => vec![HirType::from_ast_type(ty)],
            None => vec![],
        };
        let m = return_types.len() as u32;

        FnSignature {
            name: f.name.clone(),
            params,
            return_types,
            n,
            m,
            span: f.span,
        }
    }

    // ─── 声明降级 ───

    fn lower_decl(&mut self, decl: &ast::Decl) -> Option<HirDecl> {
        match decl {
            ast::Decl::Fn(f) => self.lower_fn(f).map(HirDecl::Fn),
            ast::Decl::Struct(s) => Some(HirDecl::Struct(s.clone())),
            ast::Decl::Enum(e) => Some(HirDecl::Enum(e.clone())),
            ast::Decl::Extern(ext) => {
                let items: Vec<HirExternItem> = ext
                    .items
                    .iter()
                    .map(|item| {
                        let ast::ExternItem::Fn(f) = item;
                        let sig = self.fn_decl_to_signature(f);
                        HirExternItem::Fn(HirFnDecl {
                            name: f.name.clone(),
                            generics: f.generics.clone(),
                            sig,
                            body: None,
                            attrs: f.attrs.clone(),
                            span: f.span,
                        })
                    })
                    .collect();
                Some(HirDecl::Extern(HirExternBlock {
                    abi: ext.abi.clone(),
                    items,
                    span: ext.span,
                }))
            }
            ast::Decl::Use(path, span) => Some(HirDecl::Use(path.clone(), *span)),
            // `impl` 已在上层展开为多个函数声明。
            ast::Decl::Impl(_) => None,
            // `mod` 由流水线在降级前展开为子模块的声明。
            ast::Decl::Mod { .. } => None,
        }
    }

    fn lower_fn(&mut self, f: &ast::FnDecl) -> Option<HirFnDecl> {
        let sig = self.resolver.lookup_fn(&f.name)?.clone();

        // 降级函数体
        let body = f.body.as_ref().map(|block| {
            self.resolver.enter_scope();

            // 先声明所有参数
            for param in &f.params {
                // `&mut self` 接收者需要在方法体内可写。
                let mutable = matches!(&param.ty, ast::Type::Ref(_, true, _));
                let _ = self.resolver.declare_var(&param.name, mutable, param.span);
                self.resolver.mark_initialized(&param.name);
            }

            let hir_block = self.lower_block(block);
            self.resolver.exit_scope();
            hir_block
        });

        Some(HirFnDecl {
            name: f.name.clone(),
            generics: f.generics.clone(),
            sig,
            body,
            attrs: f.attrs.clone(),
            span: f.span,
        })
    }

    // ─── 块 ───

    fn lower_block(&mut self, block: &ast::Block) -> HirBlock {
        self.resolver.enter_scope();

        let mut stmts = Vec::new();
        for stmt in &block.stmts {
            if let Some(hir_stmt) = self.lower_stmt(stmt) {
                stmts.push(hir_stmt);
            }
        }

        let tail = block.tail.as_ref().and_then(|e| self.lower_expr(e).map(Box::new));
        let scope_size = self.resolver.current_bindings().len();

        self.resolver.exit_scope();

        HirBlock {
            stmts,
            tail,
            span: block.span,
            scope_size,
        }
    }

    // ─── 语句 ───

    fn lower_stmt(&mut self, stmt: &ast::Stmt) -> Option<HirStmt> {
        match stmt {
            ast::Stmt::Let { pattern, ty, init, mutable, span } => {
                // 先声明变量（未初始化），再求值 RHS
                // 这样 RHS 中引用同名变量会触发前向引用错误
                let var_names = self.pattern_var_names(pattern);
                for name in &var_names {
                    if let Err(e) = self.resolver.declare_var(name, *mutable, *span) {
                        self.errors.push(e);
                    }
                }

                // 求值 RHS
                let hir_init = init.as_ref().and_then(|e| self.lower_expr(e));

                // 标记已初始化
                for name in &var_names {
                    self.resolver.mark_initialized(name);
                }

                let hir_ty = ty.as_ref().map(HirType::from_ast_type);

                Some(HirStmt::Let {
                    pattern: pattern.clone(),
                    ty: hir_ty,
                    init: hir_init,
                    mutable: *mutable,
                    span: *span,
                })
            }
            ast::Stmt::Expr(expr, span) => {
                self.lower_expr(expr).map(|e| HirStmt::Expr(e, *span))
            }
            ast::Stmt::Assign { target, value, span } => {
                let hir_value = self.lower_expr(value)?;
                let hir_target = self.lower_assign_target(target, *span)?;
                Some(HirStmt::Assign {
                    target: hir_target,
                    value: hir_value,
                    span: *span,
                })
            }
        }
    }

    /// 降级赋值目标（左值），并验证根变量可变性。
    fn lower_assign_target(&mut self, target: &ast::Expr, fallback_span: Span) -> Option<HirAssignTarget> {
        match target {
            ast::Expr::Var(name, tspan) => {
                let depth = match self.resolver.resolve_var(name, *tspan) {
                    Ok(depth) => depth,
                    Err(e) => {
                        self.errors.push(e);
                        return None;
                    }
                };
                if self.resolver.is_mutable(name) == Some(false) {
                    self.errors.push(ResolveError::CannotAssignToImmutable {
                        name: name.clone(),
                        span: *tspan,
                    });
                }
                Some(HirAssignTarget::Var(HirVarRef {
                    name: name.clone(),
                    scope_depth: depth,
                    span: *tspan,
                }))
            }
            ast::Expr::Deref(inner, _) => {
                let hir_inner = self.lower_expr(inner)?;
                Some(HirAssignTarget::Deref(hir_inner))
            }
            ast::Expr::FieldAccess(inner, field, _) => {
                self.check_assign_root_mutable(inner);
                let hir_inner = self.lower_expr(inner)?;
                Some(HirAssignTarget::Field {
                    object: Box::new(hir_inner),
                    field: field.clone(),
                })
            }
            ast::Expr::Index(array, index, _) => {
                self.check_assign_root_mutable(array);
                let hir_array = self.lower_expr(array)?;
                let hir_index = self.lower_expr(index)?;
                Some(HirAssignTarget::Index {
                    array: Box::new(hir_array),
                    index: Box::new(hir_index),
                })
            }
            _ => {
                self.errors
                    .push(ResolveError::InvalidAssignmentTarget { span: fallback_span });
                None
            }
        }
    }

    /// 字段/索引赋值要求根变量可变。
    fn check_assign_root_mutable(&mut self, expr: &ast::Expr) {
        match expr {
            ast::Expr::Var(name, span) => {
                if self.resolver.is_mutable(name) == Some(false) {
                    self.errors.push(ResolveError::CannotAssignToImmutable {
                        name: name.clone(),
                        span: *span,
                    });
                }
            }
            ast::Expr::FieldAccess(inner, _, _) | ast::Expr::Index(inner, _, _) => {
                self.check_assign_root_mutable(inner);
            }
            ast::Expr::Deref(inner, _) => self.check_assign_root_mutable(inner),
            _ => {}
        }
    }

    // ─── 表达式 ───

    fn lower_expr(&mut self, expr: &ast::Expr) -> Option<HirExpr> {
        match expr {
            ast::Expr::Lit(lit, span) => Some(HirExpr::Lit(lit.clone(), *span)),
            ast::Expr::Var(name, span) => {
                match self.resolver.resolve_var(name, *span) {
                    Ok(depth) => Some(HirExpr::Var(HirVarRef {
                        name: name.clone(),
                        scope_depth: depth,
                        span: *span,
                    })),
                    Err(e) => {
                        // 函数名、类型名（首字母大写）或内建函数名不作为变量错误；
                        // `use` 别名在查函数表前先解析成限定名。
                        let resolved_fn = self
                            .aliases
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| name.clone());
                        let is_type_name = name.chars().next().is_some_and(|c| c.is_uppercase());
                        if self.resolver.lookup_fn(&resolved_fn).is_some() || is_type_name || is_builtin_fn(name) {
                            Some(HirExpr::Var(HirVarRef {
                                name: resolved_fn,
                                scope_depth: usize::MAX,
                                span: *span,
                            }))
                        } else {
                            self.errors.push(e);
                            None
                        }
                    }
                }
            }
            ast::Expr::IdRef(name, span) => {
                // #var: 获取变量的 ElementId
                // 允许引用未初始化的变量（获取其 ID），但变量必须已声明
                if let Err(e) = self.resolver.resolve_var(name, *span) {
                    self.errors.push(e);
                    return None;
                }
                Some(HirExpr::IdRef(name.clone(), *span))
            }
            ast::Expr::Deref(inner, span) => {
                let hir_inner = self.lower_expr(inner)?;
                Some(HirExpr::Deref(Box::new(hir_inner), *span))
            }
            ast::Expr::Ref(inner, mutable, span) => {
                let hir_inner = self.lower_expr(inner)?;
                Some(HirExpr::Ref(Box::new(hir_inner), *mutable, *span))
            }
            ast::Expr::Binary(lhs, op, rhs, span) => {
                let hir_lhs = self.lower_expr(lhs)?;
                let hir_rhs = self.lower_expr(rhs)?;
                Some(HirExpr::Binary(Box::new(hir_lhs), *op, Box::new(hir_rhs), *span))
            }
            ast::Expr::Unary(op, inner, span) => {
                let hir_inner = self.lower_expr(inner)?;
                Some(HirExpr::Unary(*op, Box::new(hir_inner), *span))
            }
            ast::Expr::Call { func, args, stack_pass, span } => {
                // 方法调用: receiver.method(args)
                if let ast::Expr::FieldAccess(receiver, method, _) = func.as_ref() {
                    let hir_receiver = self.lower_expr(receiver)?;
                    if matches!(method.as_str(), "push" | "pop") {
                        self.check_assign_root_mutable(receiver);
                    }
                    let mut hir_args = Vec::new();
                    for arg in args {
                        let e = match arg {
                            ast::CallArg::Pos(e, _) | ast::CallArg::Named(_, e, _) => e,
                        };
                        if let Some(he) = self.lower_expr(e) {
                            hir_args.push(he);
                        }
                    }
                    return Some(HirExpr::MethodCall {
                        receiver: Box::new(hir_receiver),
                        method: method.clone(),
                        args: hir_args,
                        span: *span,
                    });
                }

                let hir_func = self.lower_expr(func)?;

                // 查找函数签名（包括内建函数）
                let fn_sig = if let ast::Expr::Var(name, _) = func.as_ref() {
                    // `use` 别名：按限定名查签名，避免调用点拿不到返回元数。
                    let lookup_name = self
                        .aliases
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.clone());
                    self.resolver
                        .lookup_fn(&lookup_name)
                        .cloned()
                        .or_else(|| builtin_fn_signature(name))
                } else {
                    None
                };

                // 验证参数个数
                if let Some(ref sig) = fn_sig
                    && !stack_pass && args.len() as u32 != sig.n {
                        self.errors.push(ResolveError::ArgumentCountMismatch {
                            expected: sig.n,
                            got: args.len(),
                            fn_name: sig.name.clone(),
                            span: *span,
                        });
                    }

                let mut hir_args: Vec<HirCallArg> = Vec::new();
                for arg in args {
                    let lowered = match arg {
                        ast::CallArg::Pos(e, s) => self.lower_expr(e).map(|he| HirCallArg::Pos(he, *s)),
                        ast::CallArg::Named(n, e, s) => self.lower_expr(e).map(|he| HirCallArg::Named(n.clone(), he, *s)),
                    };
                    match lowered {
                        Some(a) => hir_args.push(a),
                        None => { /* error already pushed by lower_expr */ }
                    }
                }

                Some(HirExpr::Call {
                    func: Box::new(hir_func),
                    args: hir_args,
                    stack_pass: *stack_pass,
                    fn_sig,
                    span: *span,
                })
            }
            ast::Expr::FieldAccess(inner, field, span) => {
                let hir_inner = self.lower_expr(inner)?;
                Some(HirExpr::FieldAccess(Box::new(hir_inner), field.clone(), *span))
            }
            ast::Expr::Index(inner, index, span) => {
                let hir_inner = self.lower_expr(inner)?;
                let hir_index = self.lower_expr(index)?;
                Some(HirExpr::Index(Box::new(hir_inner), Box::new(hir_index), *span))
            }
            ast::Expr::Tuple(elems, span) => {
                let mut hir_elems = Vec::new();
                for e in elems { if let Some(he) = self.lower_expr(e) { hir_elems.push(he); } }
                Some(HirExpr::Tuple(hir_elems, *span))
            }
            ast::Expr::Array(elems, span) => {
                let mut hir_elems = Vec::new();
                for e in elems { if let Some(he) = self.lower_expr(e) { hir_elems.push(he); } }
                Some(HirExpr::Array(hir_elems, *span))
            }
            ast::Expr::Repeat(elem, count, span) => {
                let hir_elem = self.lower_expr(elem)?;
                let hir_count = self.lower_expr(count)?;
                Some(HirExpr::Repeat(Box::new(hir_elem), Box::new(hir_count), *span))
            }
            ast::Expr::StructLit { name, fields, span } => {
                let hir_fields: Vec<(String, HirExpr)> = fields
                    .iter()
                    .map(|(n, e)| self.lower_expr(e).map(|he| (n.clone(), he)))
                    .collect::<Option<_>>()?;
                Some(HirExpr::StructLit { name: name.clone(), fields: hir_fields, span: *span })
            }
            ast::Expr::EnumLit { name, variant, data, span } => {
                // 关联函数: Vec::new() / String::new() / Box::new(x)
                if name.len() == 1
                    && matches!(name[0].as_str(), "Vec" | "String" | "Box")
                    && variant == "new"
                {
                    let mut hir_args = Vec::new();
                    if let Some(e) = data
                        && let Some(he) = self.lower_expr(e)
                    {
                        hir_args.push(he);
                    }
                    return Some(HirExpr::TypeMethod {
                        type_name: name.clone(),
                        method: variant.clone(),
                        args: hir_args,
                        span: *span,
                    });
                }

                // 用户 `impl` 中的关联函数：`Type::func(args)` 解析为普通调用。
                let assoc_name = format!("{}::{variant}", name.join("::"));
                if let Some(sig) = self.resolver.lookup_fn(&assoc_name).cloned() {
                    let mut hir_args = Vec::new();
                    if let Some(e) = data
                        && let Some(he) = self.lower_expr(e)
                    {
                        hir_args.push(HirCallArg::Pos(he, *span));
                    }
                    return Some(HirExpr::Call {
                        func: Box::new(HirExpr::Var(HirVarRef {
                            name: assoc_name,
                            scope_depth: usize::MAX,
                            span: *span,
                        })),
                        args: hir_args,
                        stack_pass: false,
                        fn_sig: Some(sig),
                        span: *span,
                    });
                }

                let hir_data = data.as_ref().and_then(|e| self.lower_expr(e).map(Box::new));
                Some(HirExpr::EnumLit {
                    name: name.clone(),
                    variant: variant.clone(),
                    data: hir_data,
                    span: *span,
                })
            }
            ast::Expr::If(cond, then_block, else_block, span) => {
                let hir_cond = self.lower_expr(cond)?;
                let hir_then = self.lower_block(then_block);
                let hir_else = else_block.as_ref().map(|b| self.lower_block(b));
                Some(HirExpr::If(Box::new(hir_cond), hir_then, hir_else, *span))
            }
            ast::Expr::Match(scrutinee, arms, span) => {
                let hir_scrutinee = self.lower_expr(scrutinee)?;
                let mut hir_arms = Vec::new();
                for arm in arms {
                    // 每个分支拥有独立作用域，模式绑定的变量只在分支内可见。
                    let bound_names = self.pattern_var_names(&arm.pattern);
                    self.resolver.enter_scope();
                    for name in &bound_names {
                        let _ = self.resolver.declare_var(name, false, arm.span);
                        self.resolver.mark_initialized(name);
                    }

                    let guard = arm.guard.as_ref().and_then(|g| self.lower_expr(g).map(Box::new));
                    let body = self.lower_expr(&arm.body);
                    self.resolver.exit_scope();

                    if let Some(body) = body {
                        hir_arms.push(HirMatchArm {
                            pattern: arm.pattern.clone(),
                            guard,
                            body: Box::new(body),
                            span: arm.span,
                        });
                    }
                }
                Some(HirExpr::Match(Box::new(hir_scrutinee), hir_arms, *span))
            }
            ast::Expr::Loop(body, span) => {
                let hir_body = self.lower_block(body);
                Some(HirExpr::Loop(hir_body, *span))
            }
            ast::Expr::While(cond, body, span) => {
                let hir_cond = self.lower_expr(cond)?;
                let hir_body = self.lower_block(body);
                Some(HirExpr::While(Box::new(hir_cond), hir_body, *span))
            }
            ast::Expr::For { pattern, iterable, body, span } => {
                let hir_iterable = self.lower_expr(iterable)?;
                // 模式变量在循环体整体作用域内可见。
                let bound_names = self.pattern_var_names(pattern);
                self.resolver.enter_scope();
                for name in &bound_names {
                    let _ = self.resolver.declare_var(name, false, *span);
                    self.resolver.mark_initialized(name);
                }
                let hir_body = self.lower_block(body);
                self.resolver.exit_scope();
                Some(HirExpr::For {
                    pattern: pattern.clone(),
                    iterable: Box::new(hir_iterable),
                    body: hir_body,
                    span: *span,
                })
            }
            ast::Expr::Block(block, span) => {
                let hir_block = self.lower_block(block);
                Some(HirExpr::Block(hir_block, *span))
            }
            ast::Expr::Labelled(label, inner, span) => {
                let hir_inner = self.lower_expr(inner)?;
                Some(HirExpr::Labelled(label.clone(), Box::new(hir_inner), *span))
            }
            ast::Expr::BreakLabel(label, value, span) => {
                let hir_value = value.as_ref().and_then(|e| self.lower_expr(e).map(Box::new));
                Some(HirExpr::BreakLabel(label.clone(), hir_value, *span))
            }
            ast::Expr::ContinueLabel(label, span) => {
                Some(HirExpr::ContinueLabel(label.clone(), *span))
            }
            ast::Expr::Break(value, span) => {
                let hir_value = value.as_ref().and_then(|e| self.lower_expr(e).map(Box::new));
                Some(HirExpr::Break(hir_value, *span))
            }
            ast::Expr::Continue(span) => Some(HirExpr::Continue(*span)),
            ast::Expr::Return(values, span) => {
                let mut hir_values = Vec::new();
                for e in values { if let Some(he) = self.lower_expr(e) { hir_values.push(he); } }
                Some(HirExpr::Return(hir_values, *span))
            }
            ast::Expr::Ret(n, span) => Some(HirExpr::Ret(*n, *span)),
            ast::Expr::Unsafe(block, span) => {
                let hir_block = self.lower_block(block);
                Some(HirExpr::Unsafe(hir_block, *span))
            }
            ast::Expr::StardustMacro(insts, span) => {
                Some(HirExpr::StardustMacro(insts.clone(), *span))
            }
            ast::Expr::As(expr, ty, span) => {
                let hir_expr = self.lower_expr(expr)?;
                let hir_ty = HirType::from_ast_type(ty);
                Some(HirExpr::As(Box::new(hir_expr), hir_ty, *span))
            }
        }
    }

    // ─── 辅助方法 ───

    /// 从模式中提取所有变量名（递归处理解构模式）
    fn pattern_var_names(&self, pattern: &ast::Pattern) -> Vec<String> {
        let mut names = Vec::new();
        Self::collect_pattern_var_names(pattern, &mut names);
        // or 模式各分支可能绑定同名变量；声明一次即可。
        let mut unique = Vec::new();
        for name in names {
            if !unique.contains(&name) {
                unique.push(name);
            }
        }
        unique
    }

    fn collect_pattern_var_names(pattern: &ast::Pattern, names: &mut Vec<String>) {
        match pattern {
            ast::Pattern::Var(name, _) => names.push(name.clone()),
            ast::Pattern::Tuple(elems, _, _) => {
                for elem in elems {
                    Self::collect_pattern_var_names(elem, names);
                }
            }
            ast::Pattern::Struct { fields, .. } => {
                for (field_name, field_pattern) in fields {
                    match field_pattern {
                        Some(pat) => Self::collect_pattern_var_names(pat, names),
                        // `Point { x, y }` 简写会绑定同名字段。
                        None => names.push(field_name.clone()),
                    }
                }
            }
            ast::Pattern::Enum { inner, .. } => {
                if let Some(inner) = inner {
                    Self::collect_pattern_var_names(inner, names);
                }
            }
            ast::Pattern::Ref(inner, _, _) => {
                Self::collect_pattern_var_names(inner, names);
            }
            ast::Pattern::Or(patterns, _) => {
                for pat in patterns {
                    Self::collect_pattern_var_names(pat, names);
                }
            }
            ast::Pattern::Lit(_, _) | ast::Pattern::Wildcard(_) => {}
        }
    }
}

impl Default for Lowering {
    fn default() -> Self {
        Self::new()
    }
}

/// 便捷函数：完整的 AST → HIR 降级
pub fn lower(ast_program: &ast::Program) -> (HirProgram, Vec<ResolveError>) {
    let mut lowering = Lowering::new();
    let hir = lowering.lower_program(ast_program);
    let errors = lowering.errors;
    (hir, errors)
}

// ─── 内建/标准库函数辅助 ───

/// 检查函数名是否为内建/标准库函数
fn is_builtin_fn(name: &str) -> bool {
    crate::io::is_builtin(name)
}

/// 将内建函数声明的类型名转换为 HIR 类型
fn builtin_type(name: &str) -> HirType {
    match name {
        "()" => HirType::Unit,
        "!" => HirType::Never,
        "_" => HirType::Wildcard,
        other => {
            // 支持 `Option<i64>` 这类单层泛型实参写法。
            if let Some(open) = other.find('<')
                && other.ends_with('>')
            {
                let base = &other[..open];
                let args = &other[open + 1..other.len() - 1];
                return HirType::Generic(
                    Box::new(HirType::Path(vec![base.to_string()])),
                    args.split(',').map(|arg| builtin_type(arg.trim())).collect(),
                );
            }
            HirType::Path(other.split("::").map(str::to_string).collect())
        }
    }
}

/// 获取内建函数的签名
fn builtin_fn_signature(name: &str) -> Option<FnSignature> {
    let def = crate::io::builtin_def(name)?;
    let params: Vec<(String, HirType)> = def
        .params
        .iter()
        .enumerate()
        .map(|(i, ty)| (format!("arg{i}"), builtin_type(ty)))
        .collect();
    let return_types: Vec<HirType> = def.returns.iter().map(|ty| builtin_type(ty)).collect();
    Some(FnSignature {
        name: def.name.to_string(),
        n: params.len() as u32,
        m: return_types.len() as u32,
        params,
        return_types,
        span: crate::frontend::token::Span::new(0, 0, 0),
    })
}

#[cfg(test)]
mod tests {
    use crate::frontend::parse;

    use super::*;

    fn lower_source(source: &str) -> (HirProgram, Vec<ResolveError>) {
        let (ast, _) = parse(source);
        // 解析错误通过 parse 的返回值传播，_ 忽略前端错误
        lower(&ast)
    }

    #[test]
    fn test_simple_function() {
        let src = "fn add(a: i32, b: i32) -> i32 { return a + b; }";
        let (hir, errors) = lower_source(src);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(hir.decls.len(), 1);
        if let HirDecl::Fn(ref f) = hir.decls[0] {
            assert_eq!(f.name, "add");
            assert_eq!(f.sig.n, 2);
            assert_eq!(f.sig.m, 1);
            assert!(f.body.is_some());
        } else {
            panic!("expected Fn");
        }
    }

    #[test]
    fn test_variable_resolution() {
        let src = "fn main() { let x = 42; let y = x + 1; }";
        let (hir, errors) = lower_source(src);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        if let HirDecl::Fn(ref f) = hir.decls[0] {
            if let Some(ref body) = f.body {
                // x 的绑定应该是第一个 let
                assert_eq!(body.stmts.len(), 2);
            }
        }
    }

    #[test]
    fn test_undefined_variable() {
        let src = "fn main() { let x = y; }";
        let (_, errors) = lower_source(src);
        assert!(!errors.is_empty());
        let has_undefined = errors.iter().any(|e| {
            matches!(e, ResolveError::UndefinedVariable { name, .. } if name == "y")
        });
        assert!(has_undefined, "expected undefined variable error, got: {errors:?}");
    }

    #[test]
    fn test_forward_reference() {
        let src = "fn main() { let x = x + 1; }";
        let (_, errors) = lower_source(src);
        let has_forward = errors.iter().any(|e| {
            matches!(e, ResolveError::ForwardReference { name, .. } if name == "x")
        });
        assert!(has_forward, "expected forward reference error, got: {errors:?}");
    }

    #[test]
    fn test_shadowing() {
        let src = "fn main() { let x = 1; { let x = 2; } let y = x; }";
        let (hir, errors) = lower_source(src);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        // y 引用的是外层 x (值为1)
        if let HirDecl::Fn(ref f) = hir.decls[0] {
            if let Some(ref body) = f.body {
                // 最后一个 let 应该能正确解析 x -> 外层
                assert!(body.stmts.len() >= 3);
            }
        }
    }

    #[test]
    fn test_function_forward_reference() {
        let src = "fn main() -> i32 { return foo(); } fn foo() -> i32 { return 42; }";
        let (hir, errors) = lower_source(src);
        // foo 应该可以在 main 之前被调用（函数前向引用是合法的）
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(hir.fn_signatures.len(), 2);
    }

    #[test]
    fn test_call_arg_count_mismatch() {
        let src = "fn foo(a: i32) -> i32 { return a; } fn main() { foo(1, 2); }";
        let (_, errors) = lower_source(src);
        assert!(!errors.is_empty());
    }
}

/// 把 `impl Type { fn ... }` 展开为名字扁平化（`Type::name`）的函数声明，
/// 并把接收者/参数类型中的 `Self` 替换为具体类型。
fn expand_impl_decls(decl: &ast::ImplDecl) -> Vec<ast::FnDecl> {
    // `Self` 在泛型 impl 中表示 `Type<A, B>`；非泛型 impl 保持 `Type`。
    let self_ty = if decl.generics.is_empty() {
        ast::Type::Path(vec![decl.type_name.clone()], decl.span)
    } else {
        ast::Type::Generic(
            Box::new(ast::Type::Path(vec![decl.type_name.clone()], decl.span)),
            decl.generics
                .iter()
                .map(|g| ast::Type::Path(vec![g.clone()], decl.span))
                .collect(),
            decl.span,
        )
    };
    decl.fns
        .iter()
        .map(|f| {
            let mut f = f.clone();
            f.name = format!("{}::{}", decl.type_name, f.name);
            // impl 的类型参数并入方法自身的泛型参数，供类型检查做实参推断与替换。
            let mut generics = decl.generics.clone();
            for g in &f.generics {
                if !generics.contains(g) {
                    generics.push(g.clone());
                }
            }
            f.generics = generics;
            for param in &mut f.params {
                param.ty = replace_self_type(&param.ty, &self_ty);
            }
            if let Some(ret) = &mut f.return_type {
                *ret = replace_self_type(ret, &self_ty);
            }
            f
        })
        .collect()
}

/// 把类型中的 `Self` 替换为 impl 目标类型（泛型 impl 为 `Type<T...>`）。
fn replace_self_type(ty: &ast::Type, self_ty: &ast::Type) -> ast::Type {
    match ty {
        ast::Type::Path(segments, _)
            if segments.len() == 1 && segments[0] == "Self" =>
        {
            self_ty.clone()
        }
        ast::Type::Ref(inner, mutable, span) => ast::Type::Ref(
            Box::new(replace_self_type(inner, self_ty)),
            *mutable,
            *span,
        ),
        ast::Type::Generic(base, args, span) => ast::Type::Generic(
            Box::new(replace_self_type(base, self_ty)),
            args.iter().map(|a| replace_self_type(a, self_ty)).collect(),
            *span,
        ),
        ast::Type::Tuple(types, span) => ast::Type::Tuple(
            types.iter().map(|t| replace_self_type(t, self_ty)).collect(),
            *span,
        ),
        other => other.clone(),
    }
}
