//! 基础类型检查器
//!
//! 在 HIR 上做保守的静态类型检查：显式类型标注、函数参数/返回值、
//! 条件表达式、二元运算、结构体/枚举构造与字段访问。
//! 未确定的类型（`Unknown`）不会报错，避免对尚未完整实现的特性产生误报。

use std::collections::HashMap;

use crate::frontend::ast::{self, IntSuffix};
use crate::frontend::token::Span;
use crate::hir::types::{
    FnSignature, HirAssignTarget, HirBlock, HirCallArg, HirDecl, HirExpr, HirFnDecl, HirProgram,
    HirStmt, HirType,
};
use crate::typeck::TypeTable;

/// 类型检查错误
#[derive(Debug, Clone)]
pub enum TypeCheckError {
    TypeMismatch {
        expected: HirType,
        found: HirType,
        span: Span,
        context: String,
    },
    UnknownType {
        name: String,
        span: Span,
    },
    ConditionNotBool {
        found: HirType,
        span: Span,
    },
    NotCallable {
        found: HirType,
        span: Span,
    },
    UnknownFunction {
        name: String,
        span: Span,
    },
    UnknownField {
        type_name: String,
        field: String,
        span: Span,
    },
    UnknownVariant {
        enum_name: String,
        variant: String,
        span: Span,
    },
    UnknownStruct {
        name: String,
        span: Span,
    },
    ArgumentCountMismatch {
        function: String,
        expected: usize,
        found: usize,
        span: Span,
    },
    ReturnArityMismatch {
        function: String,
        expected: usize,
        found: usize,
        span: Span,
    },
}

impl TypeCheckError {
    pub fn span(&self) -> Span {
        match self {
            TypeCheckError::TypeMismatch { span, .. }
            | TypeCheckError::UnknownType { span, .. }
            | TypeCheckError::ConditionNotBool { span, .. }
            | TypeCheckError::NotCallable { span, .. }
            | TypeCheckError::UnknownFunction { span, .. }
            | TypeCheckError::UnknownField { span, .. }
            | TypeCheckError::UnknownVariant { span, .. }
            | TypeCheckError::UnknownStruct { span, .. }
            | TypeCheckError::ArgumentCountMismatch { span, .. }
            | TypeCheckError::ReturnArityMismatch { span, .. } => *span,
        }
    }
}

impl std::fmt::Display for TypeCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeCheckError::TypeMismatch { expected, found, span, context } => write!(
                f,
                "{span}: type mismatch in {context}: expected {}, found {}",
                expected.name(),
                found.name()
            ),
            TypeCheckError::UnknownType { name, span } => {
                write!(f, "{span}: unknown type `{name}`")
            }
            TypeCheckError::ConditionNotBool { found, span } => {
                write!(f, "{span}: condition must be bool, found {}", found.name())
            }
            TypeCheckError::NotCallable { found, span } => {
                write!(f, "{span}: expression of type {} is not callable", found.name())
            }
            TypeCheckError::UnknownFunction { name, span } => {
                write!(f, "{span}: unknown function `{name}`")
            }
            TypeCheckError::UnknownField { type_name, field, span } => {
                write!(f, "{span}: type `{type_name}` has no field `{field}`")
            }
            TypeCheckError::UnknownVariant { enum_name, variant, span } => {
                write!(f, "{span}: enum `{enum_name}` has no variant `{variant}`")
            }
            TypeCheckError::UnknownStruct { name, span } => {
                write!(f, "{span}: unknown struct `{name}`")
            }
            TypeCheckError::ArgumentCountMismatch { function, expected, found, span } => {
                write!(
                    f,
                    "{span}: function `{function}` expects {expected} arguments, got {found}"
                )
            }
            TypeCheckError::ReturnArityMismatch { function, expected, found, span } => {
                write!(
                    f,
                    "{span}: function `{function}` returns {expected} values, but `return` provides {found}"
                )
            }
        }
    }
}

/// 函数上下文：用于检查 `return`
#[derive(Debug, Clone)]
struct FnContext {
    name: String,
    return_types: Vec<HirType>,
}

/// 基础类型检查器
pub struct TypeChecker<'a> {
    functions: HashMap<String, FnSignature>,
    type_table: &'a TypeTable,
    scopes: Vec<HashMap<String, HirType>>,
    fn_stack: Vec<FnContext>,
    errors: Vec<TypeCheckError>,
}

impl<'a> TypeChecker<'a> {
    pub fn new(program: &HirProgram, type_table: &'a TypeTable) -> Self {
        let functions = program
            .fn_signatures
            .iter()
            .map(|sig| (sig.name.clone(), sig.clone()))
            .collect();
        Self {
            functions,
            type_table,
            scopes: Vec::new(),
            fn_stack: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// 检查整个程序，返回所有发现的类型错误
    pub fn check_program(mut self, program: &HirProgram) -> Vec<TypeCheckError> {
        for decl in &program.decls {
            match decl {
                HirDecl::Fn(f) => self.check_fn(f),
                HirDecl::Extern(ext) => {
                    for item in &ext.items {
                        let crate::hir::types::HirExternItem::Fn(f) = item;
                        if f.body.is_some() {
                            self.check_fn(f);
                        }
                    }
                }
                _ => {}
            }
        }
        self.errors
    }

    fn check_fn(&mut self, f: &HirFnDecl) {
        let Some(ref body) = f.body else {
            return;
        };

        self.scopes.push(HashMap::new());
        for (name, ty) in &f.sig.params {
            self.declare(name, ty.clone());
        }
        self.fn_stack.push(FnContext {
            name: f.name.clone(),
            return_types: f.sig.return_types.clone(),
        });

        self.check_block(body);

        self.fn_stack.pop();
        self.scopes.pop();
    }

    // ─── 作用域 ───

    fn declare(&mut self, name: &str, ty: HirType) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), ty);
        }
    }

    fn lookup(&self, name: &str) -> Option<&HirType> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty);
            }
        }
        None
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn error(&mut self, error: TypeCheckError) {
        self.errors.push(error);
    }

    // ─── 语句 ───

    fn check_block(&mut self, block: &HirBlock) -> HirType {
        self.push_scope();
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        let ty = match &block.tail {
            Some(expr) => self.check_expr(expr),
            None => HirType::Unit,
        };
        self.pop_scope();
        ty
    }

    fn check_stmt(&mut self, stmt: &HirStmt) {
        match stmt {
            HirStmt::Let { pattern, ty, init, span, .. } => {
                let init_ty = init.as_ref().map(|e| self.check_expr(e));

                let declared_ty = match ty {
                    Some(t) => {
                        self.validate_type(t, *span);
                        Some(t.clone())
                    }
                    None => None,
                };

                if let (Some(expected), Some(found)) = (&declared_ty, &init_ty)
                    && !is_assignable(expected, found)
                {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: expected.clone(),
                        found: found.clone(),
                        span: *span,
                        context: "let binding".to_string(),
                    });
                }

                let binding_ty = declared_ty
                    .or(init_ty)
                    .unwrap_or(HirType::Unknown);
                self.bind_pattern(pattern, &binding_ty);
            }
            HirStmt::Expr(expr, _) => {
                self.check_expr(expr);
            }
            HirStmt::Assign { target, value, span } => {
                let value_ty = self.check_expr(value);
                let expected = match target {
                    HirAssignTarget::Var(var) => self.lookup(&var.name).cloned(),
                    HirAssignTarget::Deref(inner) => {
                        let inner_ty = self.check_expr(inner);
                        match inner_ty {
                            HirType::Ref(target_ty, true) => Some(*target_ty),
                            HirType::Ref(_, false) => {
                                self.error(TypeCheckError::TypeMismatch {
                                    expected: HirType::Ref(Box::new(HirType::Unknown), true),
                                    found: inner_ty,
                                    span: *span,
                                    context: "assignment through immutable reference".to_string(),
                                });
                                None
                            }
                            HirType::Unknown => None,
                            other => {
                                self.error(TypeCheckError::TypeMismatch {
                                    expected: HirType::Ref(Box::new(HirType::Unknown), true),
                                    found: other,
                                    span: *span,
                                    context: "dereference assignment".to_string(),
                                });
                                None
                            }
                        }
                    }
                    HirAssignTarget::Field { object, field } => {
                        let object_ty = self.check_expr(object);
                        Some(self.field_type(&object_ty, field, *span))
                    }
                    HirAssignTarget::Index { array, index } => {
                        let array_ty = self.check_expr(array);
                        let index_ty = self.check_expr(index);
                        if !is_integer(&index_ty) && !is_unknown(&index_ty) {
                            self.error(TypeCheckError::TypeMismatch {
                                expected: HirType::Path(vec!["usize".to_string()]),
                                found: index_ty,
                                span: *span,
                                context: "array index".to_string(),
                            });
                        }
                        Some(self.index_result_type(array_ty, *span))
                    }
                };

                if let Some(expected) = expected
                    && !is_assignable(&expected, &value_ty)
                {
                    self.error(TypeCheckError::TypeMismatch {
                        expected,
                        found: value_ty,
                        span: *span,
                        context: "assignment".to_string(),
                    });
                }
            }
        }
    }

    fn bind_pattern(&mut self, pattern: &ast::Pattern, ty: &HirType) {
        match pattern {
            ast::Pattern::Var(name, _) => self.declare(name, ty.clone()),
            ast::Pattern::Tuple(elems, _, _) => {
                let elem_types = match ty {
                    HirType::Tuple(types) => types.clone(),
                    _ => vec![],
                };
                for (i, elem) in elems.iter().enumerate() {
                    let elem_ty = elem_types.get(i).cloned().unwrap_or(HirType::Unknown);
                    self.bind_pattern(elem, &elem_ty);
                }
            }
            ast::Pattern::Ref(inner, _, _) => {
                let inner_ty = match ty {
                    HirType::Ref(inner, _) => (**inner).clone(),
                    _ => HirType::Unknown,
                };
                self.bind_pattern(inner, &inner_ty);
            }
            // 结构体/枚举解构的绑定沿用 match 模式的推导逻辑。
            ast::Pattern::Struct { .. } | ast::Pattern::Enum { .. } => {
                self.bind_match_pattern(pattern, ty);
            }
            ast::Pattern::Or(patterns, _) => {
                for pat in patterns {
                    self.bind_pattern(pat, ty);
                }
            }
            ast::Pattern::Lit(_, _) | ast::Pattern::Wildcard(_) => {}
        }
    }

    // ─── 表达式 ───

    fn check_expr(&mut self, expr: &HirExpr) -> HirType {
        match expr {
            HirExpr::Lit(lit, _) => literal_type(lit),
            HirExpr::Var(var) => {
                if var.scope_depth == usize::MAX {
                    // 函数名/类型名占位，仅作为调用目标等场景使用
                    HirType::Unknown
                } else {
                    self.lookup(&var.name).cloned().unwrap_or(HirType::Unknown)
                }
            }
            HirExpr::IdRef(_, _) => HirType::Unknown,
            HirExpr::Deref(inner, span) => match self.check_expr(inner) {
                HirType::Ref(inner, _) => *inner,
                HirType::Generic(base, args) if base.name() == "Box" => {
                    args.first().cloned().unwrap_or(HirType::Unknown)
                }
                HirType::Unknown => HirType::Unknown,
                other => {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: HirType::Ref(Box::new(HirType::Unknown), false),
                        found: other,
                        span: *span,
                        context: "dereference".to_string(),
                    });
                    HirType::Unknown
                }
            },
            HirExpr::Ref(inner, mutable, _) => {
                HirType::Ref(Box::new(self.check_expr(inner)), *mutable)
            }
            HirExpr::Binary(lhs, op, rhs, span) => self.check_binary(lhs, *op, rhs, *span),
            HirExpr::Unary(op, inner, span) => {
                let inner_ty = self.check_expr(inner);
                match op {
                    ast::UnOp::Neg => {
                        if !is_numeric(&inner_ty) && !is_unknown(&inner_ty) {
                            self.error(TypeCheckError::TypeMismatch {
                                expected: HirType::Path(vec!["i32".to_string()]),
                                found: inner_ty,
                                span: *span,
                                context: "unary `-`".to_string(),
                            });
                            HirType::Unknown
                        } else {
                            inner_ty
                        }
                    }
                    ast::UnOp::Not => {
                        if !is_bool(&inner_ty) && !is_unknown(&inner_ty) {
                            self.error(TypeCheckError::TypeMismatch {
                                expected: HirType::Path(vec!["bool".to_string()]),
                                found: inner_ty,
                                span: *span,
                                context: "unary `!`".to_string(),
                            });
                            HirType::Unknown
                        } else {
                            HirType::Path(vec!["bool".to_string()])
                        }
                    }
                    ast::UnOp::Deref => match inner_ty {
                        HirType::Ref(inner, _) => *inner,
                        HirType::Unknown => HirType::Unknown,
                        other => {
                            self.error(TypeCheckError::TypeMismatch {
                                expected: HirType::Ref(Box::new(HirType::Unknown), false),
                                found: other,
                                span: *span,
                                context: "dereference".to_string(),
                            });
                            HirType::Unknown
                        }
                    },
                }
            }
            HirExpr::Call { func, args, stack_pass, fn_sig, span } => {
                self.check_call(func, args, *stack_pass, fn_sig.as_ref(), *span)
            }
            HirExpr::MethodCall { receiver, method, args, span } => {
                self.check_method_call(receiver, method, args, *span)
            }
            HirExpr::TypeMethod { type_name, method, args, span } => {
                self.check_type_method(type_name, method, args, *span)
            }
            HirExpr::FieldAccess(inner, field, span) => {
                let inner_ty = self.check_expr(inner);
                self.field_type(&inner_ty, field, *span)
            }
            HirExpr::Index(inner, index, span) => {
                let inner_ty = self.check_expr(inner);
                let index_ty = self.check_expr(index);
                if !is_integer(&index_ty) && !is_unknown(&index_ty) {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: HirType::Path(vec!["usize".to_string()]),
                        found: index_ty,
                        span: *span,
                        context: "array index".to_string(),
                    });
                }
                self.index_result_type(inner_ty, *span)
            }
            HirExpr::Tuple(elems, _) => {
                HirType::Tuple(elems.iter().map(|e| self.check_expr(e)).collect())
            }
            HirExpr::Array(elems, span) => {
                let elem_ty = elems.first().map(|e| self.check_expr(e)).unwrap_or(HirType::Unknown);
                for e in elems.iter().skip(1) {
                    let ty = self.check_expr(e);
                    if !is_assignable(&elem_ty, &ty) {
                        self.error(TypeCheckError::TypeMismatch {
                            expected: elem_ty.clone(),
                            found: ty,
                            span: *span,
                            context: "array element".to_string(),
                        });
                    }
                }
                HirType::Array(Box::new(elem_ty))
            }
            HirExpr::Repeat(elem, count, span) => {
                let elem_ty = self.check_expr(elem);
                let count_ty = self.check_expr(count);
                if !is_integer(&count_ty) && !is_unknown(&count_ty) {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: HirType::Path(vec!["usize".to_string()]),
                        found: count_ty,
                        span: *span,
                        context: "array repeat count".to_string(),
                    });
                }
                HirType::Array(Box::new(elem_ty))
            }
            HirExpr::StructLit { name, fields, span } => {
                self.check_struct_lit(name, fields, *span)
            }
            HirExpr::EnumLit { name, variant, data, span } => {
                self.check_enum_lit(name, variant, data.as_deref(), *span)
            }
            HirExpr::If(cond, then_block, else_block, span) => {
                let cond_ty = self.check_expr(cond);
                self.require_bool(&cond_ty, cond_span(cond, *span));
                let then_ty = self.check_block(then_block);
                match else_block {
                    Some(else_block) => {
                        let else_ty = self.check_block(else_block);
                        if !is_assignable(&then_ty, &else_ty)
                            && !is_assignable(&else_ty, &then_ty)
                        {
                            self.error(TypeCheckError::TypeMismatch {
                                expected: then_ty.clone(),
                                found: else_ty,
                                span: *span,
                                context: "if/else branches".to_string(),
                            });
                        }
                        then_ty
                    }
                    None => HirType::Unit,
                }
            }
            HirExpr::Match(scrutinee, arms, span) => {
                let scrutinee_ty = self.check_expr(scrutinee);
                let mut result_ty: Option<HirType> = None;
                for arm in arms {
                    self.push_scope();
                    self.bind_match_pattern(&arm.pattern, &scrutinee_ty);
                    if let Some(guard) = &arm.guard {
                        let guard_ty = self.check_expr(guard);
                        self.require_bool(&guard_ty, arm.span);
                    }
                    let body_ty = self.check_expr(&arm.body);
                    self.pop_scope();
                    match &result_ty {
                        None => result_ty = Some(body_ty),
                        Some(expected) => {
                            if !is_assignable(expected, &body_ty)
                                && !is_assignable(&body_ty, expected)
                            {
                                self.error(TypeCheckError::TypeMismatch {
                                    expected: expected.clone(),
                                    found: body_ty,
                                    span: arm.span,
                                    context: "match arms".to_string(),
                                });
                            }
                        }
                    }
                }
                result_ty.unwrap_or(HirType::Never)
            }
            HirExpr::Loop(_, _) => HirType::Unknown,
            HirExpr::While(cond, body, _) => {
                let cond_ty = self.check_expr(cond);
                self.require_bool(&cond_ty, cond_span(cond, expr_span(expr)));
                self.check_block(body);
                HirType::Unit
            }
            HirExpr::For { pattern, iterable, body, .. } => {
                let iter_ty = self.check_expr(iterable);
                let elem_ty = match &iter_ty {
                    HirType::Array(elem) => (**elem).clone(),
                    _ => HirType::Unknown,
                };
                self.push_scope();
                self.bind_pattern(pattern, &elem_ty);
                self.check_block(body);
                self.pop_scope();
                HirType::Unit
            }
            HirExpr::Block(block, _) => self.check_block(block),
            HirExpr::Break(_, _) | HirExpr::Continue(_) => HirType::Never,
            HirExpr::Return(values, span) => {
                self.check_return(values, *span);
                HirType::Never
            }
            HirExpr::Ret(_, _) => HirType::Never,
            HirExpr::Unsafe(block, _) => self.check_block(block),
            HirExpr::StardustMacro(_, _) => HirType::Unknown,
            HirExpr::As(inner, target, span) => {
                self.check_expr(inner);
                self.validate_type(target, *span);
                target.clone()
            }
        }
    }

    fn check_binary(
        &mut self,
        lhs: &HirExpr,
        op: ast::BinOp,
        rhs: &HirExpr,
        span: Span,
    ) -> HirType {
        let lhs_ty = self.check_expr(lhs);
        let rhs_ty = self.check_expr(rhs);

        match op {
            ast::BinOp::Add
            | ast::BinOp::Sub
            | ast::BinOp::Mul
            | ast::BinOp::Div
            | ast::BinOp::Mod => {
                if !is_numeric(&lhs_ty) && !is_unknown(&lhs_ty) {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: HirType::Path(vec!["i32".to_string()]),
                        found: lhs_ty.clone(),
                        span,
                        context: format!("operator `{}`", binop_name(op)),
                    });
                }
                if !is_numeric(&rhs_ty) && !is_unknown(&rhs_ty) {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: HirType::Path(vec!["i32".to_string()]),
                        found: rhs_ty.clone(),
                        span,
                        context: format!("operator `{}`", binop_name(op)),
                    });
                }
                arithmetic_result(&lhs_ty, &rhs_ty)
            }
            ast::BinOp::Eq | ast::BinOp::Ne => {
                if !is_comparable(&lhs_ty, &rhs_ty) {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: lhs_ty,
                        found: rhs_ty,
                        span,
                        context: format!("operator `{}`", binop_name(op)),
                    });
                }
                bool_type()
            }
            ast::BinOp::Lt | ast::BinOp::Gt | ast::BinOp::Le | ast::BinOp::Ge => {
                if !is_ordered(&lhs_ty, &rhs_ty) {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: lhs_ty,
                        found: rhs_ty,
                        span,
                        context: format!("operator `{}`", binop_name(op)),
                    });
                }
                bool_type()
            }
            ast::BinOp::And | ast::BinOp::Or | ast::BinOp::Xor => {
                if !is_bool(&lhs_ty) && !is_unknown(&lhs_ty) {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: bool_type(),
                        found: lhs_ty,
                        span,
                        context: format!("operator `{}`", binop_name(op)),
                    });
                }
                if !is_bool(&rhs_ty) && !is_unknown(&rhs_ty) {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: bool_type(),
                        found: rhs_ty,
                        span,
                        context: format!("operator `{}`", binop_name(op)),
                    });
                }
                bool_type()
            }
            ast::BinOp::Assign => {
                if !is_assignable(&lhs_ty, &rhs_ty) {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: lhs_ty,
                        found: rhs_ty,
                        span,
                        context: "assignment".to_string(),
                    });
                }
                HirType::Unit
            }
        }
    }

    fn check_call(
        &mut self,
        func: &HirExpr,
        args: &[HirCallArg],
        stack_pass: bool,
        fn_sig: Option<&FnSignature>,
        span: Span,
    ) -> HirType {
        // 求值调用目标；函数名占位不产生类型错误
        let func_ty = self.check_expr(func);

        let sig = match fn_sig {
            Some(sig) => Some(sig.clone()),
            None => match func {
                HirExpr::Var(var) if var.scope_depth == usize::MAX => {
                    if let Some(sig) = self.functions.get(&var.name) {
                        Some(sig.clone())
                    } else if let Some(def) = crate::io::builtin_def(&var.name) {
                        Some(builtin_signature(def))
                    } else {
                        self.error(TypeCheckError::UnknownFunction {
                            name: var.name.clone(),
                            span,
                        });
                        None
                    }
                }
                _ => {
                    if !is_unknown(&func_ty) {
                        self.error(TypeCheckError::NotCallable {
                            found: func_ty,
                            span,
                        });
                    }
                    None
                }
            },
        };

        let Some(sig) = sig else {
            for arg in args {
                match arg {
                    HirCallArg::Pos(e, _) | HirCallArg::Named(_, e, _) => {
                        self.check_expr(e);
                    }
                }
            }
            return HirType::Unknown;
        };

        // 栈传参不检查实参个数；空参数调用按普通零参调用处理。
        if !stack_pass {
            if args.len() != sig.params.len() {
                self.error(TypeCheckError::ArgumentCountMismatch {
                    function: sig.name.clone(),
                    expected: sig.params.len(),
                    found: args.len(),
                    span,
                });
            }
        }

        for (i, arg) in args.iter().enumerate() {
            let (expr, arg_span) = match arg {
                HirCallArg::Pos(e, s) => (e, *s),
                HirCallArg::Named(_, e, s) => (e, *s),
            };
            let found = self.check_expr(expr);

            let expected = match arg {
                HirCallArg::Named(name, _, _) => sig
                    .params
                    .iter()
                    .find(|(pname, _)| pname == name)
                    .map(|(_, ty)| ty.clone()),
                HirCallArg::Pos(_, _) => sig.params.get(i).map(|(_, ty)| ty.clone()),
            };

            if let Some(expected) = expected
                && !is_assignable(&expected, &found)
            {
                self.error(TypeCheckError::TypeMismatch {
                    expected,
                    found,
                    span: arg_span,
                    context: format!("argument {} of `{}`", i + 1, sig.name),
                });
            }
        }

        match sig.return_types.len() {
            0 => HirType::Unit,
            1 => sig.return_types[0].clone(),
            _ => HirType::Tuple(sig.return_types.clone()),
        }
    }

    fn check_return(&mut self, values: &[HirExpr], span: Span) {
        let value_types: Vec<HirType> = values.iter().map(|e| self.check_expr(e)).collect();

        let Some(ctx) = self.fn_stack.last().cloned() else {
            return;
        };

        if value_types.len() != ctx.return_types.len() {
            self.error(TypeCheckError::ReturnArityMismatch {
                function: ctx.name,
                expected: ctx.return_types.len(),
                found: value_types.len(),
                span,
            });
            return;
        }

        for (expected, found) in ctx.return_types.iter().zip(value_types.iter()) {
            if !is_assignable(expected, found) {
                self.error(TypeCheckError::TypeMismatch {
                    expected: expected.clone(),
                    found: found.clone(),
                    span,
                    context: format!("return value of `{}`", ctx.name),
                });
            }
        }
    }

    fn require_bool(&mut self, ty: &HirType, span: Span) {
        if !is_bool(ty) && !is_unknown(ty) {
            self.error(TypeCheckError::ConditionNotBool {
                found: ty.clone(),
                span,
            });
        }
    }

    /// 索引访问的结果类型；同时校验容器类型。
    fn index_result_type(&mut self, inner_ty: HirType, span: Span) -> HirType {
        match inner_ty {
            HirType::Array(elem) => *elem,
            HirType::Generic(base, args) => {
                let name = base.name();
                if name == "Vec" || name == "String" || name.ends_with("Vec") {
                    args.first().cloned().unwrap_or(HirType::Unknown)
                } else {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: HirType::Array(Box::new(HirType::Unknown)),
                        found: HirType::Generic(base, args),
                        span,
                        context: "index access".to_string(),
                    });
                    HirType::Unknown
                }
            }
            HirType::Path(segments) => {
                let name = segments.join("::");
                if name == "Vec" || name == "String" || name.ends_with("Vec") {
                    HirType::Unknown
                } else {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: HirType::Array(Box::new(HirType::Unknown)),
                        found: HirType::Path(segments),
                        span,
                        context: "index access".to_string(),
                    });
                    HirType::Unknown
                }
            }
            HirType::Unknown => HirType::Unknown,
            other => {
                self.error(TypeCheckError::TypeMismatch {
                    expected: HirType::Array(Box::new(HirType::Unknown)),
                    found: other,
                    span,
                    context: "index access".to_string(),
                });
                HirType::Unknown
            }
        }
    }

    /// 方法调用类型检查（Vec/String 的 len/push/pop）。
    fn check_method_call(
        &mut self,
        receiver: &HirExpr,
        method: &str,
        args: &[HirExpr],
        span: Span,
    ) -> HirType {
        let receiver_ty = self.check_expr(receiver);
        match method {
            "len" => {
                if !args.is_empty() {
                    self.error(TypeCheckError::ArgumentCountMismatch {
                        function: "len".to_string(),
                        expected: 0,
                        found: args.len(),
                        span,
                    });
                }
                self.container_element_type(&receiver_ty, span, "len");
                HirType::Path(vec!["i64".to_string()])
            }
            "push" => {
                if args.len() != 1 {
                    self.error(TypeCheckError::ArgumentCountMismatch {
                        function: "push".to_string(),
                        expected: 1,
                        found: args.len(),
                        span,
                    });
                }
                let elem_ty = self.container_element_type(&receiver_ty, span, "push");
                if let Some(arg) = args.first() {
                    let arg_ty = self.check_expr(arg);
                    if !is_assignable(&elem_ty, &arg_ty) {
                        self.error(TypeCheckError::TypeMismatch {
                            expected: elem_ty,
                            found: arg_ty,
                            span,
                            context: "push argument".to_string(),
                        });
                    }
                }
                HirType::Unit
            }
            "pop" => {
                if !args.is_empty() {
                    self.error(TypeCheckError::ArgumentCountMismatch {
                        function: "pop".to_string(),
                        expected: 0,
                        found: args.len(),
                        span,
                    });
                }
                self.container_element_type(&receiver_ty, span, "pop")
            }
            other => {
                self.error(TypeCheckError::UnknownField {
                    type_name: receiver_ty.name(),
                    field: other.to_string(),
                    span,
                });
                HirType::Unknown
            }
        }
    }

    /// 关联函数类型检查（Vec::new / String::new / Box::new）。
    fn check_type_method(
        &mut self,
        type_name: &[String],
        method: &str,
        args: &[HirExpr],
        span: Span,
    ) -> HirType {
        let name = type_name.join("::");
        match (name.as_str(), method) {
            ("Vec", "new") => {
                if !args.is_empty() {
                    self.error(TypeCheckError::ArgumentCountMismatch {
                        function: "Vec::new".to_string(),
                        expected: 0,
                        found: args.len(),
                        span,
                    });
                }
                HirType::Generic(
                    Box::new(HirType::Path(vec!["Vec".to_string()])),
                    vec![HirType::Unknown],
                )
            }
            ("String", "new") => {
                if !args.is_empty() {
                    self.error(TypeCheckError::ArgumentCountMismatch {
                        function: "String::new".to_string(),
                        expected: 0,
                        found: args.len(),
                        span,
                    });
                }
                HirType::Path(vec!["String".to_string()])
            }
            ("Box", "new") => {
                if args.len() != 1 {
                    self.error(TypeCheckError::ArgumentCountMismatch {
                        function: "Box::new".to_string(),
                        expected: 1,
                        found: args.len(),
                        span,
                    });
                }
                let inner = args
                    .first()
                    .map(|e| self.check_expr(e))
                    .unwrap_or(HirType::Unknown);
                HirType::Generic(Box::new(HirType::Path(vec!["Box".to_string()])), vec![inner])
            }
            _ => {
                self.error(TypeCheckError::UnknownFunction {
                    name: format!("{name}::{method}"),
                    span,
                });
                HirType::Unknown
            }
        }
    }

    /// 容器元素类型；非 Vec/String/数组时报错并返回 Unknown。
    fn container_element_type(&mut self, ty: &HirType, span: Span, method: &str) -> HirType {
        match ty {
            HirType::Array(elem) => (**elem).clone(),
            HirType::Generic(base, args) => match base.name().as_str() {
                "Vec" => args.first().cloned().unwrap_or(HirType::Unknown),
                "String" => HirType::Path(vec!["char".to_string()]),
                _ => {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: HirType::Array(Box::new(HirType::Unknown)),
                        found: ty.clone(),
                        span,
                        context: format!("method `{method}`"),
                    });
                    HirType::Unknown
                }
            },
            HirType::Path(segments) => match segments.join("::").as_str() {
                "Vec" => HirType::Unknown,
                "String" => HirType::Path(vec!["char".to_string()]),
                _ => {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: HirType::Array(Box::new(HirType::Unknown)),
                        found: ty.clone(),
                        span,
                        context: format!("method `{method}`"),
                    });
                    HirType::Unknown
                }
            },
            HirType::Unknown => HirType::Unknown,
            other => {
                self.error(TypeCheckError::TypeMismatch {
                    expected: HirType::Array(Box::new(HirType::Unknown)),
                    found: other.clone(),
                    span,
                    context: format!("method `{method}`"),
                });
                HirType::Unknown
            }
        }
    }

    fn field_type(&mut self, inner_ty: &HirType, field: &str, span: Span) -> HirType {        let type_name = match inner_ty {
            HirType::Path(segments) => segments.join("::"),
            HirType::Unknown => return HirType::Unknown,
            // 泛型容器的方法/字段尚未实现，避免级联误报。
            HirType::Generic(_, _) => return HirType::Unknown,
            _ => {
                self.error(TypeCheckError::UnknownField {
                    type_name: inner_ty.name(),
                    field: field.to_string(),
                    span,
                });
                return HirType::Unknown;
            }
        };

        match self.type_table.lookup_struct(&type_name) {
            Some(def) => match def.fields.iter().find(|f| f.name == field) {
                Some(field_def) => HirType::Path(
                    field_def
                        .type_name
                        .split("::")
                        .map(str::to_string)
                        .collect(),
                ),
                None => {
                    self.error(TypeCheckError::UnknownField {
                        type_name,
                        field: field.to_string(),
                        span,
                    });
                    HirType::Unknown
                }
            },
            None => {
                // 未知结构体（可能来自标准库或尚未登记的类型）不报错
                HirType::Unknown
            }
        }
    }

    fn check_struct_lit(
        &mut self,
        name: &[String],
        fields: &[(String, HirExpr)],
        span: Span,
    ) -> HirType {
        let type_name = name.join("::");
        let struct_def = self.type_table.lookup_struct(&type_name).cloned();

        match struct_def {
            Some(def) => {
                for (field_name, expr) in fields {
                    let found = self.check_expr(expr);
                    match def.fields.iter().find(|f| &f.name == field_name) {
                        Some(field_def) => {
                            let expected = HirType::Path(
                                field_def
                                    .type_name
                                    .split("::")
                                    .map(str::to_string)
                                    .collect(),
                            );
                            if !is_assignable(&expected, &found) {
                                self.error(TypeCheckError::TypeMismatch {
                                    expected,
                                    found,
                                    span,
                                    context: format!(
                                        "field `{field_name}` of `{type_name}`"
                                    ),
                                });
                            }
                        }
                        None => {
                            self.error(TypeCheckError::UnknownField {
                                type_name: type_name.clone(),
                                field: field_name.clone(),
                                span,
                            });
                        }
                    }
                }
                HirType::Path(name.to_vec())
            }
            None => {
                for (_, expr) in fields {
                    self.check_expr(expr);
                }
                // 大写开头的构造按用户类型处理；未登记类型仍返回其名字以便后续检查。
                if name.first().is_some_and(|n| n.chars().next().is_some_and(|c| c.is_uppercase())) {
                    HirType::Path(name.to_vec())
                } else {
                    self.error(TypeCheckError::UnknownStruct {
                        name: type_name,
                        span,
                    });
                    HirType::Unknown
                }
            }
        }
    }

    fn check_enum_lit(
        &mut self,
        name: &[String],
        variant: &str,
        data: Option<&HirExpr>,
        span: Span,
    ) -> HirType {
        let enum_name = name.join("::");
        let enum_ty = HirType::Path(name.to_vec());
        let enum_def = self.type_table.lookup_enum(&enum_name).cloned();
        let data_ty = data.map(|e| self.check_expr(e));

        let Some(def) = enum_def else {
            return enum_ty;
        };

        let Some(variant_def) = def.variants.iter().find(|v| v.name == variant) else {
            self.error(TypeCheckError::UnknownVariant {
                enum_name,
                variant: variant.to_string(),
                span,
            });
            return enum_ty;
        };

        match (&variant_def.data_type, &data_ty) {
            (Some(expected_name), Some(found)) => {
                let expected = HirType::Path(
                    expected_name.split("::").map(str::to_string).collect(),
                );
                if !is_assignable(&expected, found) {
                    self.error(TypeCheckError::TypeMismatch {
                        expected,
                        found: found.clone(),
                        span,
                        context: format!("payload of `{}::{variant}`", enum_ty.name()),
                    });
                }
            }
            (None, Some(_)) => {
                self.error(TypeCheckError::TypeMismatch {
                    expected: HirType::Unit,
                    found: data_ty.unwrap_or(HirType::Unknown),
                    span,
                    context: format!("payload of `{}::{variant}`", enum_ty.name()),
                });
            }
            (Some(expected_name), None) => {
                self.error(TypeCheckError::ArgumentCountMismatch {
                    function: format!("{}::{variant}", enum_ty.name()),
                    expected: 1,
                    found: 0,
                    span,
                });
                let _ = expected_name;
            }
            (None, None) => {}
        }

        enum_ty
    }

    fn bind_match_pattern(&mut self, pattern: &ast::Pattern, scrutinee_ty: &HirType) {
        match pattern {
            ast::Pattern::Var(name, _) => self.declare(name, scrutinee_ty.clone()),
            ast::Pattern::Tuple(elems, _, _) => {
                let elem_types = match scrutinee_ty {
                    HirType::Tuple(types) => types.clone(),
                    _ => vec![],
                };
                for (i, pat) in elems.iter().enumerate() {
                    let ty = elem_types.get(i).cloned().unwrap_or(HirType::Unknown);
                    self.bind_pattern(pat, &ty);
                }
            }
            ast::Pattern::Enum { name, variant, inner, .. } => {
                let enum_name = name.join("::");
                let payload_ty = self
                    .type_table
                    .lookup_enum(&enum_name)
                    .and_then(|def| def.variants.iter().find(|v| &v.name == variant))
                    .and_then(|v| v.data_type.as_ref())
                    .map(|t| {
                        HirType::Path(t.split("::").map(str::to_string).collect())
                    })
                    .unwrap_or(HirType::Unknown);
                if let Some(inner) = inner {
                    self.bind_pattern(inner, &payload_ty);
                }
            }
            ast::Pattern::Struct { name, fields, .. } => {
                let struct_name = name.join("::");
                for (field_name, field_pattern) in fields {
                    let field_ty = self
                        .type_table
                        .lookup_struct(&struct_name)
                        .and_then(|def| def.fields.iter().find(|f| &f.name == field_name))
                        .map(|f| {
                            HirType::Path(
                                f.type_name.split("::").map(str::to_string).collect(),
                            )
                        })
                        .unwrap_or(HirType::Unknown);
                    if let Some(pat) = field_pattern {
                        self.bind_pattern(pat, &field_ty);
                    }
                }
            }
            ast::Pattern::Ref(inner, _, _) => {
                let inner_ty = match scrutinee_ty {
                    HirType::Ref(inner, _) => (**inner).clone(),
                    _ => HirType::Unknown,
                };
                self.bind_pattern(inner, &inner_ty);
            }
            ast::Pattern::Or(patterns, _) => {
                for pat in patterns {
                    self.bind_pattern(pat, scrutinee_ty);
                }
            }
            ast::Pattern::Lit(_, _) | ast::Pattern::Wildcard(_) => {}
        }
    }

    fn validate_type(&mut self, ty: &HirType, span: Span) {
        match ty {
            HirType::Path(segments) => {
                let name = segments.join("::");
                if !is_primitive_type(&name)
                    && !self.type_table.contains(&name)
                    && !name.chars().next().is_some_and(|c| c.is_uppercase())
                {
                    self.error(TypeCheckError::UnknownType { name, span });
                }
            }
            HirType::Ref(inner, _)
            | HirType::Array(inner) => self.validate_type(inner, span),
            HirType::Generic(base, args) => {
                self.validate_type(base, span);
                for arg in args {
                    self.validate_type(arg, span);
                }
            }
            HirType::Tuple(types) => {
                for ty in types {
                    self.validate_type(ty, span);
                }
            }
            HirType::FnPtr(params, rets) => {
                for ty in params.iter().chain(rets.iter()) {
                    self.validate_type(ty, span);
                }
            }
            HirType::Never | HirType::Wildcard | HirType::Unit | HirType::Unknown => {}
        }
    }
}

fn builtin_signature(def: &crate::io::BuiltinDef) -> FnSignature {
    let params = def
        .params
        .iter()
        .enumerate()
        .map(|(i, ty)| (format!("arg{i}"), builtin_type(ty)))
        .collect::<Vec<_>>();
    let return_types = def.returns.iter().map(|ty| builtin_type(ty)).collect::<Vec<_>>();
    FnSignature {
        name: def.name.to_string(),
        n: params.len() as u32,
        m: return_types.len() as u32,
        params,
        return_types,
        span: Span::new(0, 0, 0),
    }
}

fn builtin_type(name: &str) -> HirType {
    match name {
        "()" => HirType::Unit,
        "!" => HirType::Never,
        "_" => HirType::Wildcard,
        other => HirType::Path(other.split("::").map(str::to_string).collect()),
    }
}

// ─── 类型工具 ───

fn literal_type(lit: &ast::Literal) -> HirType {
    match lit {
        ast::Literal::Int(_, suffix) => HirType::Path(vec![int_suffix_name(*suffix).to_string()]),
        ast::Literal::Float(_) => HirType::Path(vec!["f64".to_string()]),
        ast::Literal::Bool(_) => bool_type(),
        ast::Literal::Char(_) => HirType::Path(vec!["char".to_string()]),
        ast::Literal::String(_) => HirType::Path(vec!["String".to_string()]),
        ast::Literal::Unit => HirType::Unit,
    }
}

fn int_suffix_name(suffix: IntSuffix) -> &'static str {
    match suffix {
        IntSuffix::None | IntSuffix::I32 => "i32",
        IntSuffix::I8 => "i8",
        IntSuffix::I16 => "i16",
        IntSuffix::I64 => "i64",
        IntSuffix::Isize => "isize",
        IntSuffix::U8 => "u8",
        IntSuffix::U16 => "u16",
        IntSuffix::U32 => "u32",
        IntSuffix::U64 => "u64",
        IntSuffix::Usize => "usize",
    }
}

fn bool_type() -> HirType {
    HirType::Path(vec!["bool".to_string()])
}

fn is_unknown(ty: &HirType) -> bool {
    matches!(ty, HirType::Unknown | HirType::Wildcard | HirType::Never)
}

fn is_bool(ty: &HirType) -> bool {
    matches!(ty, HirType::Path(segments) if segments.len() == 1 && segments[0] == "bool")
}

fn is_integer(ty: &HirType) -> bool {
    matches!(
        ty,
        HirType::Path(segments)
            if segments.len() == 1
                && matches!(
                    segments[0].as_str(),
                    "i8" | "i16" | "i32" | "i64" | "isize"
                        | "u8" | "u16" | "u32" | "u64" | "usize"
                )
    )
}

fn is_float(ty: &HirType) -> bool {
    matches!(ty, HirType::Path(segments) if segments.len() == 1 && matches!(segments[0].as_str(), "f32" | "f64"))
}

fn is_numeric(ty: &HirType) -> bool {
    is_integer(ty) || is_float(ty)
}

fn is_primitive_type(name: &str) -> bool {
    matches!(
        name,
        "i8" | "i16" | "i32" | "i64" | "isize"
            | "u8" | "u16" | "u32" | "u64" | "usize"
            | "f32" | "f64" | "bool" | "char" | "str" | "String"
            | "()" | "!" | "_" | "Vec"
    )
}

fn is_comparable(a: &HirType, b: &HirType) -> bool {
    if is_unknown(a) || is_unknown(b) {
        return true;
    }
    if a == b {
        return true;
    }
    if is_numeric(a) && is_numeric(b) {
        return true;
    }
    match (a, b) {
        (HirType::Ref(a_inner, _), HirType::Ref(b_inner, _)) => is_comparable(a_inner, b_inner),
        _ => false,
    }
}

fn is_ordered(a: &HirType, b: &HirType) -> bool {
    if is_unknown(a) || is_unknown(b) {
        return true;
    }
    (is_numeric(a) && is_numeric(b))
        || (is_char(a) && is_char(b))
}

fn is_char(ty: &HirType) -> bool {
    matches!(ty, HirType::Path(segments) if segments.len() == 1 && segments[0] == "char")
}

/// 判断 `found` 是否可赋值给 `expected`
fn is_assignable(expected: &HirType, found: &HirType) -> bool {
    if is_unknown(expected) || is_unknown(found) {
        return true;
    }
    if expected == found {
        return true;
    }
    if is_numeric(expected) && is_numeric(found) {
        return true;
    }
    match (expected, found) {
        (HirType::Ref(e, e_mut), HirType::Ref(f, f_mut)) => {
            e_mut == f_mut && is_assignable(e, f)
        }
        (HirType::Array(e), HirType::Array(f)) => is_assignable(e, f),
        // `vec![...]` 目前以数组值实现，因此允许数组初始化 Vec<T> 标注。
        (HirType::Generic(base, args), HirType::Array(elem)) => {
            base.name() == "Vec"
                && args.first().is_none_or(|arg| is_assignable(arg, elem))
        }
        (HirType::Generic(e_base, e_args), HirType::Generic(f_base, f_args)) => {
            e_base == f_base
                && e_args.len() == f_args.len()
                && e_args
                    .iter()
                    .zip(f_args.iter())
                    .all(|(a, b)| is_assignable(a, b))
        }
        (HirType::Tuple(e), HirType::Tuple(f)) => {
            e.len() == f.len() && e.iter().zip(f.iter()).all(|(a, b)| is_assignable(a, b))
        }
        _ => false,
    }
}

fn arithmetic_result(a: &HirType, b: &HirType) -> HirType {
    if is_unknown(a) && is_unknown(b) {
        HirType::Unknown
    } else if is_float(a) || is_float(b) {
        HirType::Path(vec!["f64".to_string()])
    } else if is_unknown(a) {
        b.clone()
    } else if is_unknown(b) {
        a.clone()
    } else if is_integer(a) && is_integer(b) {
        // 混合整型宽度时取较宽的一侧；这里保守地返回左值类型。
        a.clone()
    } else {
        HirType::Unknown
    }
}

fn binop_name(op: ast::BinOp) -> &'static str {
    match op {
        ast::BinOp::Add => "+",
        ast::BinOp::Sub => "-",
        ast::BinOp::Mul => "*",
        ast::BinOp::Div => "/",
        ast::BinOp::Mod => "%",
        ast::BinOp::Eq => "==",
        ast::BinOp::Ne => "!=",
        ast::BinOp::Lt => "<",
        ast::BinOp::Gt => ">",
        ast::BinOp::Le => "<=",
        ast::BinOp::Ge => ">=",
        ast::BinOp::And => "&&",
        ast::BinOp::Or => "||",
        ast::BinOp::Xor => "^",
        ast::BinOp::Assign => "=",
    }
}

/// 表达式自身的 span；HIR 表达式没有统一 span 访问器，这里只用于少数条件检查。
fn expr_span(expr: &HirExpr) -> Span {
    match expr {
        HirExpr::Lit(_, span)
        | HirExpr::IdRef(_, span)
        | HirExpr::Deref(_, span)
        | HirExpr::Ref(_, _, span)
        | HirExpr::Binary(_, _, _, span)
        | HirExpr::Unary(_, _, span)
        | HirExpr::Call { span, .. }
        | HirExpr::MethodCall { span, .. }
        | HirExpr::TypeMethod { span, .. }
        | HirExpr::FieldAccess(_, _, span)
        | HirExpr::Index(_, _, span)
        | HirExpr::Tuple(_, span)
        | HirExpr::Array(_, span)
        | HirExpr::Repeat(_, _, span)
        | HirExpr::StructLit { span, .. }
        | HirExpr::EnumLit { span, .. }
        | HirExpr::If(_, _, _, span)
        | HirExpr::Match(_, _, span)
        | HirExpr::Loop(_, span)
        | HirExpr::While(_, _, span)
        | HirExpr::For { span, .. }
        | HirExpr::Block(_, span)
        | HirExpr::Break(_, span)
        | HirExpr::Continue(span)
        | HirExpr::Return(_, span)
        | HirExpr::Ret(_, span)
        | HirExpr::Unsafe(_, span)
        | HirExpr::StardustMacro(_, span)
        | HirExpr::As(_, _, span) => *span,
        HirExpr::Var(var) => var.span,
    }
}

fn cond_span(cond: &HirExpr, fallback: Span) -> Span {
    let span = expr_span(cond);
    if span.line == 0 { fallback } else { span }
}

/// 便捷入口：检查整个 HIR 程序
pub fn check_program(program: &HirProgram, type_table: &TypeTable) -> Vec<TypeCheckError> {
    TypeChecker::new(program, type_table).check_program(program)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::parse;
    use crate::hir::lower;

    fn check(src: &str) -> Vec<TypeCheckError> {
        let (ast, errors) = parse(src);
        assert!(errors.is_empty(), "parse errors: {errors:?}");
        let (hir, errors) = lower(&ast);
        assert!(errors.is_empty(), "resolve errors: {errors:?}");
        let table = TypeTable::from_ast_decls(&ast.decls);
        check_program(&hir, &table)
    }

    #[test]
    fn test_valid_program() {
        let errors = check(
            "fn add(a: i32, b: i32) -> i32 { return a + b; } fn main() { let x: i32 = add(1, 2); }",
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn test_let_annotation_mismatch() {
        let errors = check("fn main() { let x: i32 = true; }");
        assert!(
            errors.iter().any(|e| matches!(
                e,
                TypeCheckError::TypeMismatch { context, .. } if context == "let binding"
            )),
            "{errors:?}"
        );
    }

    #[test]
    fn test_condition_must_be_bool() {
        let errors = check("fn main() { if 1 { } }");
        assert!(
            errors.iter().any(|e| matches!(e, TypeCheckError::ConditionNotBool { .. })),
            "{errors:?}"
        );
    }

    #[test]
    fn test_return_type_mismatch() {
        let errors = check("fn f() -> i32 { return true; } fn main() { }");
        assert!(
            errors.iter().any(|e| matches!(
                e,
                TypeCheckError::TypeMismatch { context, .. } if context.contains("return value")
            )),
            "{errors:?}"
        );
    }

    #[test]
    fn test_argument_type_mismatch() {
        let errors = check(
            "fn takes_bool(b: bool) { } fn main() { takes_bool(1); }",
        );
        assert!(
            errors.iter().any(|e| matches!(
                e,
                TypeCheckError::TypeMismatch { context, .. } if context.contains("argument")
            )),
            "{errors:?}"
        );
    }

    #[test]
    fn test_arithmetic_on_bool_rejected() {
        let errors = check("fn main() { let x = true + 1; }");
        assert!(
            errors.iter().any(|e| matches!(
                e,
                TypeCheckError::TypeMismatch { context, .. } if context.contains("operator")
            )),
            "{errors:?}"
        );
    }

    #[test]
    fn test_struct_field_type() {
        let errors = check(
            "struct Point { x: i32, y: bool } fn main() { let p = Point { x: 1, y: true }; let b: bool = p.y; }",
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn test_if_else_numeric_compatible() {
        let errors = check("fn main() { let x = if true { 1 } else { 2 }; }");
        assert!(errors.is_empty(), "{errors:?}");
    }
}
