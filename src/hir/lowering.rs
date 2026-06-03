use crate::frontend::ast;
use crate::hir::resolver::{ResolveError, Resolver};
use crate::hir::types::*;

/// AST → HIR 降级器
pub struct Lowering {
    resolver: Resolver,
    errors: Vec<ResolveError>,
}

impl Lowering {
    pub fn new() -> Self {
        Self { resolver: Resolver::new(), errors: Vec::new() }
    }

    pub fn errors(&self) -> &[ResolveError] {
        &self.errors
    }

    /// 降级整个程序
    pub fn lower_program(&mut self, ast_program: &ast::Program) -> HirProgram {
        // 第一遍：收集所有函数签名
        self.collect_fn_signatures(ast_program);

        // 第二遍：降级所有声明
        let mut decls = Vec::new();
        for decl in &ast_program.decls {
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
        }
    }

    fn lower_fn(&mut self, f: &ast::FnDecl) -> Option<HirFnDecl> {
        let sig = self.resolver.lookup_fn(&f.name)?.clone();

        // 降级函数体
        let body = f.body.as_ref().map(|block| {
            self.resolver.enter_scope();

            // 先声明所有参数
            for param in &f.params {
                let _ = self.resolver.declare_var(&param.name, false, param.span);
                self.resolver.mark_initialized(&param.name);
            }

            let hir_block = self.lower_block(block);
            self.resolver.exit_scope();
            hir_block
        });

        Some(HirFnDecl {
            name: f.name.clone(),
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
                let var_name = self.pattern_var_name(pattern);
                if let Some(ref name) = var_name
                    && let Err(e) = self.resolver.declare_var(name, *mutable, *span) {
                        self.errors.push(e);
                    }

                // 求值 RHS
                let hir_init = init.as_ref().and_then(|e| self.lower_expr(e));

                // 标记已初始化
                if let Some(ref name) = var_name {
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

                // 解析赋值目标
                if let ast::Expr::Var(name, tspan) = target {
                    match self.resolver.resolve_var(name, *tspan) {
                        Ok(depth) => {
                            // 检查可变性（报错但不阻止降级）
                            if self.resolver.is_mutable(name) == Some(false) {
                                self.errors.push(ResolveError::UndefinedVariable {
                                    name: format!("cannot assign to immutable variable `{name}`"),
                                    span: *tspan,
                                });
                            }
                            Some(HirStmt::Assign {
                                target: name.clone(),
                                depth,
                                value: hir_value,
                                span: *span,
                            })
                        }
                        Err(e) => {
                            self.errors.push(e);
                            None
                        }
                    }
                } else {
                    // 非简单变量赋值（如 *ptr = expr），暂不支持
                    self.errors.push(ResolveError::UndefinedVariable {
                        name: "complex assignment targets not yet supported".to_string(),
                        span: *span,
                    });
                    None
                }
            }
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
                        // 函数名或类型名（首字母大写）不作为变量错误
                        let is_type_name = name.chars().next().is_some_and(|c| c.is_uppercase());
                        if self.resolver.lookup_fn(name).is_some() || is_type_name {
                            Some(HirExpr::Var(HirVarRef {
                                name: name.clone(),
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
                // 首先验证变量存在
                if self.resolver.resolve_var(name, *span).is_err() {
                    // #id 可以引用未初始化的变量（获取其 ID）
                    // 但我们仍需要变量已声明
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
                let hir_func = self.lower_expr(func)?;

                // 查找函数签名
                let fn_sig = if let ast::Expr::Var(name, _) = func.as_ref() {
                    self.resolver.lookup_fn(name).cloned()
                } else {
                    None
                };

                // 验证参数个数
                if let Some(ref sig) = fn_sig
                    && !stack_pass && args.len() as u32 != sig.n {
                        self.errors.push(ResolveError::UndefinedVariable {
                            name: format!(
                                "function `{}` expects {} arguments, got {}",
                                sig.name, sig.n, args.len()
                            ),
                            span: *span,
                        });
                    }

                let hir_args: Vec<HirCallArg> = args
                    .iter()
                    .filter_map(|arg| match arg {
                        ast::CallArg::Pos(e, s) => {
                            self.lower_expr(e).map(|he| HirCallArg::Pos(he, *s))
                        }
                        ast::CallArg::Named(n, e, s) => {
                            self.lower_expr(e).map(|he| HirCallArg::Named(n.clone(), he, *s))
                        }
                    })
                    .collect();

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
                let hir_elems: Vec<HirExpr> = elems.iter().filter_map(|e| self.lower_expr(e)).collect();
                Some(HirExpr::Tuple(hir_elems, *span))
            }
            ast::Expr::Array(elems, span) => {
                let hir_elems: Vec<HirExpr> = elems.iter().filter_map(|e| self.lower_expr(e)).collect();
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
                let hir_arms: Vec<HirMatchArm> = arms
                    .iter()
                    .filter_map(|arm| {
                        let guard = arm.guard.as_ref().and_then(|g| self.lower_expr(g).map(Box::new));
                        let body = self.lower_expr(&arm.body)?;
                        Some(HirMatchArm {
                            pattern: arm.pattern.clone(),
                            guard,
                            body: Box::new(body),
                            span: arm.span,
                        })
                    })
                    .collect();
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
                let hir_body = self.lower_block(body);
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
            ast::Expr::Break(value, span) => {
                let hir_value = value.as_ref().and_then(|e| self.lower_expr(e).map(Box::new));
                Some(HirExpr::Break(hir_value, *span))
            }
            ast::Expr::Continue(span) => Some(HirExpr::Continue(*span)),
            ast::Expr::Return(values, span) => {
                let hir_values: Vec<HirExpr> = values.iter().filter_map(|e| self.lower_expr(e)).collect();
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

    /// 从模式中提取变量名（仅简单变量模式）
    fn pattern_var_name(&self, pattern: &ast::Pattern) -> Option<String> {
        match pattern {
            ast::Pattern::Var(name, _) => Some(name.clone()),
            _ => None, // 解构模式暂不处理
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
