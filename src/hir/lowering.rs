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
    /// 编译期求值后的 `const` 值。
    consts: HashMap<String, ast::Literal>,
    /// 闭包提升产生的顶层函数。
    hoisted_fns: Vec<HirFnDecl>,
    /// 下一个闭包函数编号。
    next_closure_id: usize,
    /// 当前闭包的基作用域深度（用于判断是否捕获外部变量）。
    closure_bases: Vec<usize>,
}

impl Lowering {
    pub fn new() -> Self {
        Self {
            resolver: Resolver::new(),
            errors: Vec::new(),
            aliases: HashMap::new(),
            consts: HashMap::new(),
            hoisted_fns: Vec::new(),
            next_closure_id: 0,
            closure_bases: Vec::new(),
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

        // 收集 `const fn` 声明，供 const 表达式调用（与声明顺序无关）。
        let const_fns: HashMap<String, &ast::FnDecl> = ast_program
            .decls
            .iter()
            .filter_map(|decl| match decl {
                ast::Decl::ConstFn(f) => Some((f.name.clone(), f)),
                _ => None,
            })
            .collect();

        // 收集并求值 `const` 声明（按声明顺序，可引用此前的 const）。
        for decl in &ast_program.decls {
            if let ast::Decl::Const(c) = decl {
                match eval_const_expr(&c.value, &self.consts, &const_fns) {
                    Ok(lit) => {
                        self.consts.insert(c.name.clone(), lit);
                    }
                    Err(reason) => self.errors.push(ResolveError::InvalidConst {
                        name: c.name.clone(),
                        reason,
                        span: c.span,
                    }),
                }
            }
        }

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

        // 闭包已提升为顶层函数：注册签名并追加声明，供类型检查与 MIR 使用。
        for f in std::mem::take(&mut self.hoisted_fns) {
            self.resolver.register_fn(f.sig.clone());
            decls.push(HirDecl::Fn(f));
        }

        let mut traits = Vec::new();
        let mut trait_methods: HashMap<String, Vec<String>> = HashMap::new();
        for decl in &ast_program.decls {
            if let ast::Decl::Trait(t) = decl {
                traits.push(t.name.clone());
                trait_methods.insert(
                    t.name.clone(),
                    t.methods.iter().map(|m| m.name.clone()).collect(),
                );
            }
        }
        let mut trait_impls: HashMap<String, Vec<String>> = HashMap::new();
        for decl in &ast_program.decls {
            if let ast::Decl::Impl(i) = decl
                && let Some(trait_name) = &i.trait_name
            {
                trait_impls
                    .entry(trait_name.clone())
                    .or_default()
                    .push(i.type_name.clone());
                // trait 声明的方法必须在 impl 中全部实现（无默认方法）。
                if let Some(required) = trait_methods.get(trait_name) {
                    for method in required {
                        if !i.fns.iter().any(|f| &f.name == method) {
                            self.errors.push(ResolveError::MissingTraitMethod {
                                trait_name: trait_name.clone(),
                                method: method.clone(),
                                ty: i.type_name.clone(),
                                span: i.span,
                            });
                        }
                    }
                }
            }
        }

        HirProgram {
            decls,
            fn_signatures: self.resolver.fn_signatures.clone(),
            traits,
            trait_impls,
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
                ast::Decl::ConstFn(f) => {
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
            Some(ast::Type::Tuple(types, _)) => types.iter().map(HirType::from_ast_type).collect(),
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
            ast::Decl::ConstFn(f) => self.lower_fn(f).map(HirDecl::Fn),
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
            // `const` 已在第一遍求值，使用处被替换为字面量。
            ast::Decl::Const(_) => None,
            // trait 声明只作为方法集合记录；方法实现由 `impl Trait for Type` 展开。
            ast::Decl::Trait(_) => None,
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

        let tail = block
            .tail
            .as_ref()
            .and_then(|e| self.lower_expr(e).map(Box::new));
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
            ast::Stmt::Let {
                pattern,
                ty,
                init,
                mutable,
                span,
            } => {
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
            ast::Stmt::Expr(expr, span) => self.lower_expr(expr).map(|e| HirStmt::Expr(e, *span)),
            ast::Stmt::Assign {
                target,
                value,
                span,
            } => {
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
    fn lower_assign_target(
        &mut self,
        target: &ast::Expr,
        fallback_span: Span,
    ) -> Option<HirAssignTarget> {
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
                self.errors.push(ResolveError::InvalidAssignmentTarget {
                    span: fallback_span,
                });
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
            // 无捕获闭包提升为顶层函数，表达式位置用函数指针代替。
            ast::Expr::Closure { params, body, span } => {
                let name = format!("__closure_{}", self.next_closure_id);
                self.next_closure_id += 1;
                let base = self.resolver.scope_depth();
                self.resolver.enter_scope();
                self.closure_bases.push(base);
                for param in params {
                    let _ = self.resolver.declare_var(&param.name, false, param.span);
                    self.resolver.mark_initialized(&param.name);
                }
                let lowered_body = self.lower_expr(body);
                self.closure_bases.pop();
                self.resolver.exit_scope();
                let lowered_body = lowered_body?;
                let sig_params: Vec<(String, HirType)> = params
                    .iter()
                    .map(|p| (p.name.clone(), HirType::from_ast_type(&p.ty)))
                    .collect();
                let sig = FnSignature {
                    name: name.clone(),
                    n: sig_params.len() as u32,
                    m: 1,
                    params: sig_params,
                    return_types: vec![HirType::Unknown],
                    span: *span,
                };
                self.hoisted_fns.push(HirFnDecl {
                    name: name.clone(),
                    generics: Vec::new(),
                    sig,
                    body: Some(HirBlock {
                        stmts: Vec::new(),
                        tail: Some(Box::new(lowered_body)),
                        span: *span,
                        scope_size: 0,
                    }),
                    attrs: Vec::new(),
                    span: *span,
                });
                Some(HirExpr::Var(HirVarRef {
                    name,
                    scope_depth: usize::MAX,
                    span: *span,
                }))
            }
            ast::Expr::Var(name, span) => {
                match self.resolver.resolve_var(name, *span) {
                    Ok(depth) => {
                        // 闭包只支持无捕获形式：跨出闭包参数作用域的变量即捕获。
                        if let Some(base) = self.closure_bases.last().copied() {
                            let absolute = self.resolver.scope_depth().saturating_sub(depth);
                            if absolute <= base {
                                self.errors.push(ResolveError::ClosureCaptureUnsupported {
                                    name: name.clone(),
                                    span: *span,
                                });
                            }
                        }
                        Some(HirExpr::Var(HirVarRef {
                            name: name.clone(),
                            scope_depth: depth,
                            span: *span,
                        }))
                    }
                    Err(e) => {
                        // `const` 名在编译期替换为求值后的字面量。
                        if let Some(lit) = self.consts.get(name).cloned() {
                            return Some(HirExpr::Lit(lit, *span));
                        }
                        // 函数名、类型名（首字母大写）或内建函数名不作为变量错误；
                        // `use` 别名在查函数表前先解析成限定名。
                        let resolved_fn = self
                            .aliases
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| name.clone());
                        let is_type_name = name.chars().next().is_some_and(|c| c.is_uppercase());
                        if self.resolver.lookup_fn(&resolved_fn).is_some()
                            || is_type_name
                            || is_builtin_fn(name)
                        {
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
                // `#id` 已从规范移除（§4.3）；保留 IdRef 交给类型检查统一报
                // `UnsupportedFeature`，不要先报成“未定义变量”。
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
                Some(HirExpr::Binary(
                    Box::new(hir_lhs),
                    *op,
                    Box::new(hir_rhs),
                    *span,
                ))
            }
            ast::Expr::Unary(op, inner, span) => {
                let hir_inner = self.lower_expr(inner)?;
                Some(HirExpr::Unary(*op, Box::new(hir_inner), *span))
            }
            ast::Expr::Call {
                func,
                args,
                stack_pass,
                span,
            } => {
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

                // 查找函数签名（包括内建函数）。turbofish（`f::<T>(...)`）的
                // 类型实参包裹在 `TypeArgs` 里，需要递归取内层 callee 名。
                let fn_sig = if let Some(name) = callee_name(func.as_ref()) {
                    // `use` 别名：按限定名查签名，避免调用点拿不到返回元数。
                    let lookup_name = self
                        .aliases
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.to_string());
                    self.resolver
                        .lookup_fn(&lookup_name)
                        .cloned()
                        .or_else(|| builtin_fn_signature(name))
                } else {
                    None
                };

                // 验证参数个数（`format` 是变参内建，由类型检查单独校验）。
                if let Some(ref sig) = fn_sig
                    && !stack_pass
                    && sig.name != "format"
                    && args.len() as u32 != sig.n
                {
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
                        ast::CallArg::Pos(e, s) => {
                            self.lower_expr(e).map(|he| HirCallArg::Pos(he, *s))
                        }
                        ast::CallArg::Named(n, e, s) => self
                            .lower_expr(e)
                            .map(|he| HirCallArg::Named(n.clone(), he, *s)),
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
                Some(HirExpr::FieldAccess(
                    Box::new(hir_inner),
                    field.clone(),
                    *span,
                ))
            }
            ast::Expr::Index(inner, index, span) => {
                let hir_inner = self.lower_expr(inner)?;
                let hir_index = self.lower_expr(index)?;
                Some(HirExpr::Index(
                    Box::new(hir_inner),
                    Box::new(hir_index),
                    *span,
                ))
            }
            ast::Expr::Tuple(elems, span) => {
                let mut hir_elems = Vec::new();
                for e in elems {
                    if let Some(he) = self.lower_expr(e) {
                        hir_elems.push(he);
                    }
                }
                Some(HirExpr::Tuple(hir_elems, *span))
            }
            ast::Expr::Array(elems, span) => {
                let mut hir_elems = Vec::new();
                for e in elems {
                    if let Some(he) = self.lower_expr(e) {
                        hir_elems.push(he);
                    }
                }
                Some(HirExpr::Array(hir_elems, *span))
            }
            ast::Expr::Repeat(elem, count, span) => {
                let hir_elem = self.lower_expr(elem)?;
                let hir_count = self.lower_expr(count)?;
                Some(HirExpr::Repeat(
                    Box::new(hir_elem),
                    Box::new(hir_count),
                    *span,
                ))
            }
            ast::Expr::StructLit { name, fields, span } => {
                let hir_fields: Vec<(String, HirExpr)> = fields
                    .iter()
                    .map(|(n, e)| self.lower_expr(e).map(|he| (n.clone(), he)))
                    .collect::<Option<_>>()?;
                Some(HirExpr::StructLit {
                    name: name.clone(),
                    fields: hir_fields,
                    span: *span,
                })
            }
            ast::Expr::EnumLit {
                name,
                variant,
                data,
                span,
            } => {
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

                    let guard = arm
                        .guard
                        .as_ref()
                        .and_then(|g| self.lower_expr(g).map(Box::new));
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
            ast::Expr::For {
                pattern,
                iterable,
                body,
                span,
            } => {
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
                let hir_value = value
                    .as_ref()
                    .and_then(|e| self.lower_expr(e).map(Box::new));
                Some(HirExpr::BreakLabel(label.clone(), hir_value, *span))
            }
            ast::Expr::ContinueLabel(label, span) => {
                Some(HirExpr::ContinueLabel(label.clone(), *span))
            }
            ast::Expr::Break(value, span) => {
                let hir_value = value
                    .as_ref()
                    .and_then(|e| self.lower_expr(e).map(Box::new));
                Some(HirExpr::Break(hir_value, *span))
            }
            ast::Expr::Continue(span) => Some(HirExpr::Continue(*span)),
            ast::Expr::Return(values, span) => {
                let mut hir_values = Vec::new();
                for e in values {
                    if let Some(he) = self.lower_expr(e) {
                        hir_values.push(he);
                    }
                }
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
            ast::Expr::TypeArgs(inner, args, span) => {
                let callee = self.lower_expr(inner)?;
                let args = args.iter().map(HirType::from_ast_type).collect();
                Some(HirExpr::TypeApplied {
                    callee: Box::new(callee),
                    args,
                    span: *span,
                })
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

/// 编译期求值 `const` 表达式：字面量、此前的 const、一元/二元运算、
/// `if`/块表达式，以及 `const fn` 调用（含递归）。
fn eval_const_expr(
    expr: &ast::Expr,
    consts: &HashMap<String, ast::Literal>,
    const_fns: &HashMap<String, &ast::FnDecl>,
) -> Result<ast::Literal, String> {
    let mut locals = HashMap::new();
    eval_const_expr_with_env(expr, consts, const_fns, &mut locals, 0)
}

/// 递归求值深度上限，防止 `const fn` 无限递归拖垮编译器。
const CONST_EVAL_MAX_DEPTH: usize = 64;

fn eval_const_expr_with_env(
    expr: &ast::Expr,
    consts: &HashMap<String, ast::Literal>,
    const_fns: &HashMap<String, &ast::FnDecl>,
    locals: &mut HashMap<String, ast::Literal>,
    depth: usize,
) -> Result<ast::Literal, String> {
    if depth > CONST_EVAL_MAX_DEPTH {
        return Err("const evaluation recursion limit exceeded".to_string());
    }
    match expr {
        ast::Expr::Lit(lit, _) => Ok(lit.clone()),
        ast::Expr::Var(name, _) => locals
            .get(name)
            .or_else(|| consts.get(name))
            .cloned()
            .ok_or_else(|| format!("`{name}` is not a previously defined const or local")),
        ast::Expr::Unary(op, inner, _) => {
            let value = eval_const_expr_with_env(inner, consts, const_fns, locals, depth)?;
            match (op, value) {
                (ast::UnOp::Neg, ast::Literal::Int(n, suffix)) => n
                    .checked_neg()
                    .map(|v| ast::Literal::Int(v, suffix))
                    .ok_or_else(|| "integer overflow".to_string()),
                (ast::UnOp::Not, ast::Literal::Bool(b)) => Ok(ast::Literal::Bool(!b)),
                _ => Err("unsupported unary operator in const".to_string()),
            }
        }
        ast::Expr::Binary(left, op, right, _) => {
            let a = eval_const_expr_with_env(left, consts, const_fns, locals, depth)?;
            let b = eval_const_expr_with_env(right, consts, const_fns, locals, depth)?;
            eval_const_binary(*op, a, b)
        }
        ast::Expr::If(cond, then_block, else_block, _) => {
            let value = eval_const_expr_with_env(cond, consts, const_fns, locals, depth)?;
            match value {
                ast::Literal::Bool(true) => {
                    eval_const_block(then_block, consts, const_fns, depth + 1)
                }
                ast::Literal::Bool(false) => match else_block {
                    Some(block) => eval_const_block(block, consts, const_fns, depth + 1),
                    None => Ok(ast::Literal::Unit),
                },
                _ => Err("const `if` condition must be a bool".to_string()),
            }
        }
        ast::Expr::Block(block, _) => eval_const_block(block, consts, const_fns, depth + 1),
        ast::Expr::Call {
            func, args, span, ..
        } => {
            let ast::Expr::Var(name, _) = func.as_ref() else {
                return Err("const calls must name a `const fn`".to_string());
            };
            let Some(function) = const_fns.get(name) else {
                return Err(format!(
                    "`{name}` is not a `const fn`; only const functions can be called in const expressions"
                ));
            };
            if args.len() != function.params.len() {
                return Err(format!(
                    "`{name}` expects {} arguments, got {}",
                    function.params.len(),
                    args.len()
                ));
            }
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                let value = match arg {
                    ast::CallArg::Pos(expr, _) => {
                        eval_const_expr_with_env(expr, consts, const_fns, locals, depth)?
                    }
                    ast::CallArg::Named(_, _, _) => {
                        return Err("named arguments are not supported in const calls".to_string());
                    }
                };
                values.push(value);
            }
            let Some(body) = &function.body else {
                return Err(format!("`{name}` has no body"));
            };
            let mut callee_locals: HashMap<String, ast::Literal> = function
                .params
                .iter()
                .zip(values)
                .map(|(param, value)| (param.name.clone(), value))
                .collect();
            eval_const_block_with_locals(body, consts, const_fns, &mut callee_locals, depth + 1)
                .map_err(|reason| format!("in call to `{name}` at {span}: {reason}"))
        }
        _ => Err(
            "unsupported expression in const (use literals, const references, const fn calls, if/blocks, and unary/binary operators)"
                .to_string(),
        ),
    }
}

fn eval_const_block(
    block: &ast::Block,
    consts: &HashMap<String, ast::Literal>,
    const_fns: &HashMap<String, &ast::FnDecl>,
    depth: usize,
) -> Result<ast::Literal, String> {
    let mut locals = HashMap::new();
    eval_const_block_with_locals(block, consts, const_fns, &mut locals, depth)
}

fn eval_const_block_with_locals(
    block: &ast::Block,
    consts: &HashMap<String, ast::Literal>,
    const_fns: &HashMap<String, &ast::FnDecl>,
    locals: &mut HashMap<String, ast::Literal>,
    depth: usize,
) -> Result<ast::Literal, String> {
    for stmt in &block.stmts {
        match stmt {
            ast::Stmt::Let {
                pattern: ast::Pattern::Var(name, _),
                init: Some(init),
                ..
            } => {
                let value = eval_const_expr_with_env(init, consts, const_fns, locals, depth)?;
                locals.insert(name.clone(), value);
            }
            ast::Stmt::Let { init: None, .. } => {
                return Err("const `let` requires an initializer".to_string());
            }
            ast::Stmt::Let { .. } => {
                return Err(
                    "only simple `let name = ...` bindings are supported in const".to_string(),
                );
            }
            ast::Stmt::Expr(expr, _) => {
                eval_const_expr_with_env(expr, consts, const_fns, locals, depth)?;
            }
            ast::Stmt::Assign { .. } => {
                return Err("assignment is not supported in const expressions".to_string());
            }
        }
    }
    match &block.tail {
        Some(expr) => eval_const_expr_with_env(expr, consts, const_fns, locals, depth),
        None => Ok(ast::Literal::Unit),
    }
}

fn eval_const_binary(
    op: ast::BinOp,
    a: ast::Literal,
    b: ast::Literal,
) -> Result<ast::Literal, String> {
    match (a, b) {
        (ast::Literal::Int(x, suffix), ast::Literal::Int(y, _)) => {
            let value = match op {
                ast::BinOp::Add => x.checked_add(y),
                ast::BinOp::Sub => x.checked_sub(y),
                ast::BinOp::Mul => x.checked_mul(y),
                ast::BinOp::Div => {
                    if y == 0 {
                        return Err("division by zero".to_string());
                    }
                    x.checked_div(y)
                }
                ast::BinOp::Mod => {
                    if y == 0 {
                        return Err("division by zero".to_string());
                    }
                    x.checked_rem(y)
                }
                ast::BinOp::Eq => return Ok(ast::Literal::Bool(x == y)),
                ast::BinOp::Ne => return Ok(ast::Literal::Bool(x != y)),
                ast::BinOp::Lt => return Ok(ast::Literal::Bool(x < y)),
                ast::BinOp::Gt => return Ok(ast::Literal::Bool(x > y)),
                ast::BinOp::Le => return Ok(ast::Literal::Bool(x <= y)),
                ast::BinOp::Ge => return Ok(ast::Literal::Bool(x >= y)),
                _ => return Err("unsupported operator for integers in const".to_string()),
            };
            value
                .map(|v| ast::Literal::Int(v, suffix))
                .ok_or_else(|| "integer overflow".to_string())
        }
        (ast::Literal::Bool(x), ast::Literal::Bool(y)) => match op {
            ast::BinOp::And => Ok(ast::Literal::Bool(x && y)),
            ast::BinOp::Or => Ok(ast::Literal::Bool(x || y)),
            ast::BinOp::Xor => Ok(ast::Literal::Bool(x ^ y)),
            ast::BinOp::Eq => Ok(ast::Literal::Bool(x == y)),
            ast::BinOp::Ne => Ok(ast::Literal::Bool(x != y)),
            _ => Err("unsupported operator for bools in const".to_string()),
        },
        _ => Err("const operands have incompatible types".to_string()),
    }
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
                    args.split(',')
                        .map(|arg| builtin_type(arg.trim()))
                        .collect(),
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
                .map(|g| ast::Type::Path(vec![g.name.clone()], decl.span))
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
                if !generics.iter().any(|existing| existing.name == g.name) {
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

/// 取调用的 callee 名字，穿透 turbofish 的 `TypeArgs` 包装。
fn callee_name(expr: &ast::Expr) -> Option<&str> {
    match expr {
        ast::Expr::Var(name, _) => Some(name.as_str()),
        ast::Expr::TypeArgs(inner, _, _) => callee_name(inner),
        _ => None,
    }
}

/// 把类型中的 `Self` 替换为 impl 目标类型（泛型 impl 为 `Type<T...>`）。
fn replace_self_type(ty: &ast::Type, self_ty: &ast::Type) -> ast::Type {
    match ty {
        ast::Type::Path(segments, _) if segments.len() == 1 && segments[0] == "Self" => {
            self_ty.clone()
        }
        ast::Type::Ref(inner, mutable, span) => {
            ast::Type::Ref(Box::new(replace_self_type(inner, self_ty)), *mutable, *span)
        }
        ast::Type::Generic(base, args, span) => ast::Type::Generic(
            Box::new(replace_self_type(base, self_ty)),
            args.iter().map(|a| replace_self_type(a, self_ty)).collect(),
            *span,
        ),
        ast::Type::Tuple(types, span) => ast::Type::Tuple(
            types
                .iter()
                .map(|t| replace_self_type(t, self_ty))
                .collect(),
            *span,
        ),
        other => other.clone(),
    }
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
        if let HirDecl::Fn(ref f) = hir.decls[0]
            && let Some(ref body) = f.body
        {
            // x 的绑定应该是第一个 let
            assert_eq!(body.stmts.len(), 2);
        }
    }

    #[test]
    fn test_undefined_variable() {
        let src = "fn main() { let x = y; }";
        let (_, errors) = lower_source(src);
        assert!(!errors.is_empty());
        let has_undefined = errors
            .iter()
            .any(|e| matches!(e, ResolveError::UndefinedVariable { name, .. } if name == "y"));
        assert!(
            has_undefined,
            "expected undefined variable error, got: {errors:?}"
        );
    }

    #[test]
    fn test_forward_reference() {
        let src = "fn main() { let x = x + 1; }";
        let (_, errors) = lower_source(src);
        let has_forward = errors
            .iter()
            .any(|e| matches!(e, ResolveError::ForwardReference { name, .. } if name == "x"));
        assert!(
            has_forward,
            "expected forward reference error, got: {errors:?}"
        );
    }

    #[test]
    fn test_shadowing() {
        let src = "fn main() { let x = 1; { let x = 2; } let y = x; }";
        let (hir, errors) = lower_source(src);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        // y 引用的是外层 x (值为1)
        if let HirDecl::Fn(ref f) = hir.decls[0]
            && let Some(ref body) = f.body
        {
            // 最后一个 let 应该能正确解析 x -> 外层
            assert!(body.stmts.len() >= 3);
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
