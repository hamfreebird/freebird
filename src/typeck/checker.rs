//! 基础类型检查器
//!
//! 在 HIR 上做保守的静态类型检查：显式类型标注、函数参数/返回值、
//! 条件表达式、二元运算、结构体/枚举构造与字段访问。
//! 未确定的类型（`Unknown`）不会报错，避免对尚未完整实现的特性产生误报。

use std::collections::{HashMap, HashSet};

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
    /// 循环外使用 `break` / `continue`
    LoopControlOutsideLoop {
        keyword: &'static str,
        span: Span,
    },
    /// M0 尚未实现 `break value`；必须显式拒绝，不能按 unit 静默处理。
    BreakWithValue {
        span: Span,
    },
    /// M0 尚未实现 `loop` 的值语义；在值位置使用必须报错。
    LoopUsedAsValue {
        span: Span,
    },
    /// `match` 未覆盖所有可能值。
    NonExhaustiveMatch {
        type_name: String,
        span: Span,
    },
    /// 没有函数体的普通函数声明。
    MissingFunctionBody {
        name: String,
        span: Span,
    },
    /// 调用尚未实现的 extern/FFI 函数。
    ExternCall {
        name: String,
        abi: Option<String>,
        span: Span,
    },
    /// 已解析但尚未实现的语言特性（`use`、`#id` 等）。
    UnsupportedFeature {
        feature: String,
        reference: String,
        span: Span,
    },
    /// 未知属性。
    UnknownAttribute {
        name: String,
        span: Span,
    },
    /// 属性参数格式错误。
    InvalidAttribute {
        name: String,
        reason: String,
        span: Span,
    },
    /// `let` 只接受不可失败模式；枚举变体、字面量等应改用 `match`。
    RefutableLetPattern {
        span: Span,
    },
    /// `or` 模式各分支绑定的变量集合必须一致。
    OrPatternBindingMismatch {
        expected: String,
        found: String,
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
            | TypeCheckError::ReturnArityMismatch { span, .. }
            | TypeCheckError::LoopControlOutsideLoop { span, .. }
            | TypeCheckError::BreakWithValue { span }
            | TypeCheckError::LoopUsedAsValue { span }
            | TypeCheckError::NonExhaustiveMatch { span, .. }
            | TypeCheckError::MissingFunctionBody { span, .. }
            | TypeCheckError::ExternCall { span, .. }
            | TypeCheckError::UnsupportedFeature { span, .. }
            | TypeCheckError::UnknownAttribute { span, .. }
            | TypeCheckError::InvalidAttribute { span, .. }
            | TypeCheckError::RefutableLetPattern { span }
            | TypeCheckError::OrPatternBindingMismatch { span, .. } => *span,
        }
    }
}

/// 类型检查警告（不阻止编译）。
#[derive(Debug, Clone)]
pub enum TypeCheckWarning {
    /// 前面的分支已经覆盖了该模式。
    UnreachablePattern { span: Span },
}

impl std::fmt::Display for TypeCheckWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeCheckWarning::UnreachablePattern { span } => {
                write!(f, "{span}: unreachable pattern")
            }
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
            TypeCheckError::LoopControlOutsideLoop { keyword, span } => {
                write!(f, "{span}: `{keyword}` can only be used inside a loop")
            }
            TypeCheckError::BreakWithValue { span } => write!(
                f,
                "{span}: `break value` is only supported inside `loop`; use `break` without a value"
            ),
            TypeCheckError::LoopUsedAsValue { span } => write!(
                f,
                "{span}: `loop` cannot be used as a value yet; use it as a statement or return from a function"
            ),
            TypeCheckError::NonExhaustiveMatch { type_name, span } => write!(
                f,
                "{span}: non-exhaustive `match` on `{type_name}`; add a matching arm or `_`"
            ),
            TypeCheckError::MissingFunctionBody { name, span } => write!(
                f,
                "{span}: function `{name}` has no body; only extern functions may omit it"
            ),
            TypeCheckError::ExternCall { name, abi, span } => match abi {
                Some(abi) => write!(
                    f,
                    "{span}: calling extern function `{name}` (ABI {abi:?}) is not supported yet; FFI is unimplemented"
                ),
                None => write!(
                    f,
                    "{span}: calling function `{name}` without a body is not supported"
                ),
            },
            TypeCheckError::UnsupportedFeature { feature, reference, span } => write!(
                f,
                "{span}: unsupported feature: {feature} (see {reference})"
            ),
            TypeCheckError::UnknownAttribute { name, span } => {
                write!(f, "{span}: unknown attribute `#[{name}]`")
            }
            TypeCheckError::InvalidAttribute { name, reason, span } => {
                write!(f, "{span}: invalid attribute `#[{name}]`: {reason}")
            }
            TypeCheckError::RefutableLetPattern { span } => write!(
                f,
                "{span}: refutable pattern in `let`; use `match` for enum variants and literals"
            ),
            TypeCheckError::OrPatternBindingMismatch { expected, found, span } => write!(
                f,
                "{span}: `or` pattern alternatives bind different variables (first binds {{{expected}}}, another binds {{{found}}})"
            ),
        }
    }
}

/// 函数上下文：用于检查 `return`
#[derive(Debug, Clone)]
struct FnContext {
    name: String,
    return_types: Vec<HirType>,
}

/// 类型检查期的循环上下文。
struct LoopCheckContext {
    /// 是否为值 `loop`（只有 `loop` 允许 `break value`）。
    is_loop: bool,
    /// 所有 `break` 值合并出的结果类型。
    result_ty: Option<HirType>,
}

/// 基础类型检查器
pub struct TypeChecker<'a> {
    functions: HashMap<String, FnSignature>,
    /// extern 函数名 → ABI（用于在调用点给出明确诊断）。
    extern_functions: HashMap<String, String>,
    type_table: &'a TypeTable,
    scopes: Vec<HashMap<String, HirType>>,
    fn_stack: Vec<FnContext>,
    /// 当前循环嵌套深度；`break`/`continue` 只在大于 0 时合法。
    loop_depth: usize,
    /// 循环上下文栈：记录最近循环是否为 `loop` 以及 `break` 值类型。
    loop_contexts: Vec<LoopCheckContext>,
    errors: Vec<TypeCheckError>,
    warnings: Vec<TypeCheckWarning>,
}

impl<'a> TypeChecker<'a> {
    pub fn new(program: &HirProgram, type_table: &'a TypeTable) -> Self {
        let functions = program
            .fn_signatures
            .iter()
            .map(|sig| (sig.name.clone(), sig.clone()))
            .collect();
        let mut extern_functions = HashMap::new();
        for decl in &program.decls {
            if let HirDecl::Extern(ext) = decl {
                for item in &ext.items {
                    let crate::hir::types::HirExternItem::Fn(f) = item;
                    extern_functions.insert(f.name.clone(), ext.abi.clone());
                }
            }
        }
        Self {
            functions,
            extern_functions,
            type_table,
            scopes: Vec::new(),
            fn_stack: Vec::new(),
            loop_depth: 0,
            loop_contexts: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// 检查整个程序，返回所有发现的类型错误（丢弃警告，兼容旧调用）。
    pub fn check_program(self, program: &HirProgram) -> Vec<TypeCheckError> {
        self.check_program_with_warnings(program).0
    }

    /// 检查整个程序，返回类型错误与警告。
    pub fn check_program_with_warnings(
        mut self,
        program: &HirProgram,
    ) -> (Vec<TypeCheckError>, Vec<TypeCheckWarning>) {
        for decl in &program.decls {
            match decl {
                HirDecl::Fn(f) => {
                    self.validate_attributes(&f.attrs);
                    if f.body.is_none() {
                        self.error(TypeCheckError::MissingFunctionBody {
                            name: f.name.clone(),
                            span: f.span,
                        });
                    }
                    self.check_fn(f);
                }
                HirDecl::Extern(ext) => {
                    for item in &ext.items {
                        let crate::hir::types::HirExternItem::Fn(f) = item;
                        self.validate_attributes(&f.attrs);
                        if f.body.is_some() {
                            self.check_fn(f);
                        }
                    }
                }
                HirDecl::Struct(s) => self.validate_attributes(&s.attrs),
                HirDecl::Enum(e) => self.validate_attributes(&e.attrs),
                HirDecl::Use(_, span) => self.error(TypeCheckError::UnsupportedFeature {
                    feature: "`use` declarations".to_string(),
                    reference: "readme.md §10 (modules are planned; no module loader yet)".to_string(),
                    span: *span,
                }),
            }
        }
        (self.errors, self.warnings)
    }

    /// 校验属性的语法与已知名称；未识别属性必须报错，不能静默丢弃。
    fn validate_attributes(&mut self, attrs: &[ast::Attribute]) {
        for attr in attrs {
            match attr.name.as_str() {
                "stardust" => {
                    if attr.args.is_empty() {
                        continue;
                    }
                    let Some(value) = attr.args.iter().find_map(|arg| match arg {
                        ast::AttributeArg::KeyValue(key, value) if key == "id" => Some(value),
                        _ => None,
                    }) else {
                        self.error(TypeCheckError::InvalidAttribute {
                            name: attr.name.clone(),
                            reason: "expected `id = <integer>`".to_string(),
                            span: attr.span,
                        });
                        continue;
                    };
                    if value.parse::<u64>().is_err() {
                        self.error(TypeCheckError::InvalidAttribute {
                            name: attr.name.clone(),
                            reason: format!("`id` must be an integer, found `{value}`"),
                            span: attr.span,
                        });
                    }
                }
                "max_stack" | "max_call_depth" | "max_steps" | "time_limit" => {
                    let value = attribute_value(attr);
                    match value.and_then(|v| v.parse::<u64>().ok()) {
                        Some(_) => {}
                        None => self.error(TypeCheckError::InvalidAttribute {
                            name: attr.name.clone(),
                            reason: "expected a non-negative integer value".to_string(),
                            span: attr.span,
                        }),
                    }
                }
                _ => self.error(TypeCheckError::UnknownAttribute {
                    name: attr.name.clone(),
                    span: attr.span,
                }),
            }
        }
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

        // 返回 () 的函数体尾部表达式会被丢弃，因此允许在尾部写 unit 循环。
        self.check_block_with_loop_tail(body, f.sig.return_types.is_empty());

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

    fn warning(&mut self, warning: TypeCheckWarning) {
        self.warnings.push(warning);
    }

    // ─── 语句 ───

    fn check_block(&mut self, block: &HirBlock) -> HirType {
        self.check_block_with_loop_tail(block, false)
    }

    /// 检查块；`_allow_loop_tail` 保留为调用兼容参数。
    fn check_block_with_loop_tail(
        &mut self,
        block: &HirBlock,
        _allow_loop_tail: bool,
    ) -> HirType {
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
                if !pattern_is_irrefutable(pattern) {
                    self.error(TypeCheckError::RefutableLetPattern { span: *span });
                }
                self.validate_pattern_bindings(pattern);
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
            HirExpr::IdRef(_, span) => {
                self.error(TypeCheckError::UnsupportedFeature {
                    feature: "`#id` / ElementId expressions".to_string(),
                    reference: "readme.md §4.3 (ElementId is not implemented in the VM value model)"
                        .to_string(),
                    span: *span,
                });
                HirType::Unknown
            }
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
                let mut seen_patterns: Vec<String> = Vec::new();
                let mut irrefutable_seen = false;
                for arm in arms {
                    let key = pattern_key(&arm.pattern);
                    let duplicate = arm.guard.is_none()
                        && seen_patterns.iter().any(|seen| seen == &key);
                    if irrefutable_seen || duplicate {
                        self.warning(TypeCheckWarning::UnreachablePattern { span: arm.span });
                    }
                    self.validate_pattern_bindings(&arm.pattern);
                    self.push_scope();
                    self.bind_match_pattern(&arm.pattern, &scrutinee_ty);
                    if let Some(guard) = &arm.guard {
                        let guard_ty = self.check_expr(guard);
                        self.require_bool(&guard_ty, arm.span);
                    }
                    let body_ty = self.check_expr(&arm.body);
                    self.pop_scope();
                    if arm.guard.is_none() {
                        if pattern_is_irrefutable(&arm.pattern) {
                            irrefutable_seen = true;
                        }
                        seen_patterns.push(key);
                    }
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
                if !self.match_is_exhaustive(&scrutinee_ty, arms) {
                    self.error(TypeCheckError::NonExhaustiveMatch {
                        type_name: scrutinee_ty.name(),
                        span: *span,
                    });
                }
                result_ty.unwrap_or(HirType::Never)
            }
            HirExpr::Loop(body, _) => {
                self.loop_depth += 1;
                self.loop_contexts.push(LoopCheckContext {
                    is_loop: true,
                    result_ty: None,
                });
                self.check_block(body);
                let ctx = self.loop_contexts.pop().expect("loop context");
                self.loop_depth -= 1;
                ctx.result_ty.unwrap_or(HirType::Never)
            }
            HirExpr::While(cond, body, _) => {
                let cond_ty = self.check_expr(cond);
                self.require_bool(&cond_ty, cond_span(cond, expr_span(expr)));
                self.loop_depth += 1;
                self.loop_contexts.push(LoopCheckContext {
                    is_loop: false,
                    result_ty: None,
                });
                self.check_block(body);
                self.loop_contexts.pop();
                self.loop_depth -= 1;
                HirType::Unit
            }
            HirExpr::For { pattern, iterable, body, span } => {
                let iter_ty = self.check_expr(iterable);
                let elem_ty = self.for_element_type(&iter_ty, *span);
                self.push_scope();
                self.bind_pattern(pattern, &elem_ty);
                self.loop_depth += 1;
                self.loop_contexts.push(LoopCheckContext {
                    is_loop: false,
                    result_ty: None,
                });
                self.check_block(body);
                self.loop_contexts.pop();
                self.loop_depth -= 1;
                self.pop_scope();
                HirType::Unit
            }
            HirExpr::Block(block, _) => self.check_block(block),
            HirExpr::Break(value, span) => {
                if self.loop_depth == 0 {
                    if let Some(value) = value {
                        self.check_expr(value);
                    }
                    self.error(TypeCheckError::LoopControlOutsideLoop {
                        keyword: "break",
                        span: *span,
                    });
                    return HirType::Never;
                }
                if let Some(value) = value {
                    let value_ty = self.check_expr(value);
                    if let Some(ctx) = self.loop_contexts.last_mut() {
                        if !ctx.is_loop {
                            self.error(TypeCheckError::BreakWithValue { span: *span });
                        } else {
                            match &ctx.result_ty {
                                None => ctx.result_ty = Some(value_ty),
                                Some(expected) => {
                                    if !is_assignable(expected, &value_ty)
                                        && !is_assignable(&value_ty, expected)
                                    {
                                        let expected = expected.clone();
                                        self.error(TypeCheckError::TypeMismatch {
                                            expected,
                                            found: value_ty,
                                            span: *span,
                                            context: "break value".to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                } else if let Some(ctx) = self.loop_contexts.last_mut() {
                    if ctx.is_loop {
                        match &ctx.result_ty {
                            None => ctx.result_ty = Some(HirType::Unit),
                            Some(expected) if *expected != HirType::Unit => {
                                let expected = expected.clone();
                                self.error(TypeCheckError::TypeMismatch {
                                    expected,
                                    found: HirType::Unit,
                                    span: *span,
                                    context: "break value".to_string(),
                                });
                            }
                            _ => {}
                        }
                    }
                }
                HirType::Never
            }
            HirExpr::Continue(span) => {
                if self.loop_depth == 0 {
                    self.error(TypeCheckError::LoopControlOutsideLoop {
                        keyword: "continue",
                        span: *span,
                    });
                }
                HirType::Never
            }
            HirExpr::Return(values, span) => {
                self.check_return(values, *span);
                HirType::Never
            }
            HirExpr::Ret(n, span) => {
                if let Some(ctx) = self.fn_stack.last() {
                    if *n < 0 || *n as usize != ctx.return_types.len() {
                        self.error(TypeCheckError::ReturnArityMismatch {
                            function: ctx.name.clone(),
                            expected: ctx.return_types.len(),
                            found: (*n).max(0) as usize,
                            span: *span,
                        });
                    }
                }
                HirType::Never
            }
            HirExpr::Unsafe(block, _) => self.check_block(block),
            HirExpr::StardustMacro(_, _) => HirType::Unknown,
            HirExpr::As(inner, target, span) => {
                let source = self.check_expr(inner);
                self.validate_type(target, *span);
                if !is_castable(&source, target) {
                    self.error(TypeCheckError::TypeMismatch {
                        expected: target.clone(),
                        found: source,
                        span: *span,
                        context: "`as` cast".to_string(),
                    });
                }
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

        if let Some(abi) = self.extern_functions.get(&sig.name).cloned() {
            // extern 函数尚未与 Stardust FFI 接通；调用必须失败，不能返回
            // 栈上残留的参数值。
            self.error(TypeCheckError::ExternCall {
                name: sig.name.clone(),
                abi: Some(abi),
                span,
            });
        }

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
        let receiver_name = match &receiver_ty {
            HirType::Path(segments) => segments.last().cloned().unwrap_or_default(),
            _ => String::new(),
        };
        if receiver_name == "Option" {
            let i64_ty = HirType::Path(vec!["i64".to_string()]);
            match method {
                "is_some" | "is_none" => {
                    if !args.is_empty() {
                        self.error(TypeCheckError::ArgumentCountMismatch {
                            function: method.to_string(),
                            expected: 0,
                            found: args.len(),
                            span,
                        });
                    }
                    return HirType::Path(vec!["bool".to_string()]);
                }
                "unwrap" => {
                    if !args.is_empty() {
                        self.error(TypeCheckError::ArgumentCountMismatch {
                            function: method.to_string(),
                            expected: 0,
                            found: args.len(),
                            span,
                        });
                    }
                    return i64_ty;
                }
                "unwrap_or" => {
                    if args.len() != 1 {
                        self.error(TypeCheckError::ArgumentCountMismatch {
                            function: method.to_string(),
                            expected: 1,
                            found: args.len(),
                            span,
                        });
                    }
                    if let Some(arg) = args.first() {
                        let arg_ty = self.check_expr(arg);
                        if !is_assignable(&i64_ty, &arg_ty) {
                            self.error(TypeCheckError::TypeMismatch {
                                expected: i64_ty,
                                found: arg_ty,
                                span,
                                context: "Option::unwrap_or argument".to_string(),
                            });
                        }
                    }
                    return HirType::Path(vec!["i64".to_string()]);
                }
                _ => {}
            }
        }
        if receiver_name == "Result"
            && matches!(method, "unwrap" | "unwrap_or" | "is_ok" | "is_err")
        {
            self.error(TypeCheckError::UnsupportedFeature {
                feature: format!("Result::{method}"),
                reference: "M2-2: Result methods are not implemented yet".to_string(),
                span,
            });
            return HirType::Unknown;
        }
        match method {
            "is_empty" => {
                if !args.is_empty() {
                    self.error(TypeCheckError::ArgumentCountMismatch {
                        function: method.to_string(),
                        expected: 0,
                        found: args.len(),
                        span,
                    });
                }
                self.container_element_type(&receiver_ty, span, "is_empty");
                HirType::Path(vec!["bool".to_string()])
            }
            "clear" => {
                if !args.is_empty() {
                    self.error(TypeCheckError::ArgumentCountMismatch {
                        function: method.to_string(),
                        expected: 0,
                        found: args.len(),
                        span,
                    });
                }
                self.container_element_type(&receiver_ty, span, "clear");
                HirType::Unit
            }
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

    /// `for` 迭代器的元素类型。
    ///
    /// 支持数组、`Vec<T>`、`String`（迭代 `char`）以及它们的引用包装；
    /// 其他类型报告一次错误，后续按 `Unknown` 处理以避免级联误报。
    fn for_element_type(&mut self, ty: &HirType, span: Span) -> HirType {
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
                        context: "`for` iterable".to_string(),
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
                        context: "`for` iterable".to_string(),
                    });
                    HirType::Unknown
                }
            },
            HirType::Ref(inner, _) => self.for_element_type(inner, span),
            HirType::Unknown => HirType::Unknown,
            other => {
                self.error(TypeCheckError::TypeMismatch {
                    expected: HirType::Array(Box::new(HirType::Unknown)),
                    found: other.clone(),
                    span,
                    context: "`for` iterable".to_string(),
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

    /// 校验 `or` 模式每个分支绑定相同的变量集合。
    ///
    /// MIR 只为整个 `or` 模式生成一次绑定路径，若分支绑定不同变量，
    /// 运行期会读到错误字段；必须在类型检查阶段拒绝。
    fn validate_pattern_bindings(&mut self, pattern: &ast::Pattern) {
        if let ast::Pattern::Or(patterns, span) = pattern {
            let mut expected: Vec<String> = Vec::new();
            if let Some(first) = patterns.first() {
                expected = pattern_binding_names(first);
            }
            for alternative in patterns.iter().skip(1) {
                let found = pattern_binding_names(alternative);
                if found != expected {
                    self.error(TypeCheckError::OrPatternBindingMismatch {
                        expected: expected.join(", "),
                        found: found.join(", "),
                        span: *span,
                    });
                }
            }
        }
        match pattern {
            ast::Pattern::Tuple(elems, _, _) | ast::Pattern::Or(elems, _) => {
                for elem in elems {
                    self.validate_pattern_bindings(elem);
                }
            }
            ast::Pattern::Struct { fields, .. } => {
                for (_, sub) in fields {
                    if let Some(sub) = sub {
                        self.validate_pattern_bindings(sub);
                    }
                }
            }
            ast::Pattern::Enum { inner, .. } => {
                if let Some(inner) = inner {
                    self.validate_pattern_bindings(inner);
                }
            }
            ast::Pattern::Ref(inner, _, _) => self.validate_pattern_bindings(inner),
            _ => {}
        }
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
            ast::Pattern::Enum { name, variant, inner, span } => {
                let enum_name = name.join("::");
                let variant_def = self
                    .type_table
                    .lookup_enum(&enum_name)
                    .and_then(|def| def.variants.iter().find(|v| &v.name == variant));
                let payload_ty = match variant_def {
                    Some(v) => v
                        .data_type
                        .as_ref()
                        .map(|t| {
                            HirType::Path(t.split("::").map(str::to_string).collect())
                        })
                        .unwrap_or(HirType::Unknown),
                    None => {
                        self.error(TypeCheckError::UnknownVariant {
                            enum_name: enum_name.clone(),
                            variant: variant.clone(),
                            span: *span,
                        });
                        HirType::Unknown
                    }
                };
                if let Some(inner) = inner {
                    self.bind_pattern(inner, &payload_ty);
                }
            }
            ast::Pattern::Struct { name, fields, span, .. } => {
                let struct_name = name.join("::");
                let struct_def = self.type_table.lookup_struct(&struct_name).cloned();
                if struct_def.is_none() {
                    self.error(TypeCheckError::UnknownStruct {
                        name: struct_name.clone(),
                        span: *span,
                    });
                }
                for (field_name, field_pattern) in fields {
                    let field = struct_def
                        .as_ref()
                        .and_then(|def| def.fields.iter().find(|f| &f.name == field_name));
                    if struct_def.is_some() && field.is_none() {
                        self.error(TypeCheckError::UnknownField {
                            type_name: struct_name.clone(),
                            field: field_name.clone(),
                            span: *span,
                        });
                    }
                    let field_ty = field
                        .map(|f| {
                            HirType::Path(
                                f.type_name.split("::").map(str::to_string).collect(),
                            )
                        })
                        .unwrap_or(HirType::Unknown);
                    match field_pattern {
                        // `Point { x, y }` 是 `Point { x: x, y: y }` 的简写。
                        Some(pat) => self.bind_pattern(pat, &field_ty),
                        None => self.declare(field_name, field_ty),
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

impl TypeChecker<'_> {
    /// 判断 `match` 是否覆盖被匹配类型的所有值。
    ///
    /// - 枚举：所有变体（含 `or` 模式）都被无 guard 的分支覆盖；
    /// - bool：`true` 与 `false` 都被覆盖；
    /// - 其他类型：必须有 `_` 或变量模式兜底。
    ///
    /// 类型未知时保守地视为穷尽，避免级联误报。
    fn match_is_exhaustive(&self, ty: &HirType, arms: &[crate::hir::types::HirMatchArm]) -> bool {
        if is_unknown(ty) || matches!(ty, HirType::Never | HirType::Wildcard) {
            return true;
        }

        if arms.iter().any(|arm| {
            arm.guard.is_none() && pattern_is_irrefutable(&arm.pattern)
        }) {
            return true;
        }

        match ty {
            HirType::Path(segments) => {
                let name = segments.join("::");
                let short = segments.last().map(String::as_str).unwrap_or(name.as_str());
                if let Some(def) = self
                    .type_table
                    .lookup_enum(&name)
                    .or_else(|| self.type_table.lookup_enum(short))
                {
                    let mut covered = HashSet::new();
                    for arm in arms {
                        if arm.guard.is_none() {
                            collect_covered_variants(&arm.pattern, &mut covered);
                        }
                    }
                    return def.variants.iter().all(|v| covered.contains(&v.name));
                }
                if name == "bool" {
                    let mut true_covered = false;
                    let mut false_covered = false;
                    for arm in arms {
                        if arm.guard.is_some() {
                            continue;
                        }
                        collect_covered_bools(
                            &arm.pattern,
                            &mut true_covered,
                            &mut false_covered,
                        );
                    }
                    return true_covered && false_covered;
                }
                false
            }
            _ => false,
        }
    }
}

/// 模式是否无条件匹配（不检查字面量/枚举标签）。
///
/// 结构体与元组解构本身不限制值，只要其子模式都不可失败，就覆盖整个类型；
/// 这正是 `match p { Point { x, y } => ... }` 能作为穷尽分支的原因。
/// 生成忽略 span 的模式键，用于检测重复分支。
fn pattern_key(pattern: &ast::Pattern) -> String {
    match pattern {
        ast::Pattern::Var(name, _) => format!("var:{name}"),
        ast::Pattern::Lit(lit, _) => format!("lit:{lit:?}"),
        ast::Pattern::Wildcard(_) => "_".to_string(),
        ast::Pattern::Tuple(elems, rest, _) => {
            let elems: Vec<String> = elems.iter().map(pattern_key).collect();
            format!("tuple:{elems:?}:{}", rest.is_some())
        }
        ast::Pattern::Struct { name, fields, rest, .. } => {
            let fields: Vec<(String, Option<String>)> = fields
                .iter()
                .map(|(field, pattern)| (field.clone(), pattern.as_ref().map(pattern_key)))
                .collect();
            format!("struct:{}:{fields:?}:{rest}", name.join("::"))
        }
        ast::Pattern::Enum { name, variant, inner, .. } => {
            format!(
                "enum:{}::{variant}:{:?}",
                name.join("::"),
                inner.as_deref().map(pattern_key)
            )
        }
        ast::Pattern::Ref(inner, mutable, _) => {
            format!("ref:{mutable}:{}", pattern_key(inner))
        }
        ast::Pattern::Or(patterns, _) => {
            let patterns: Vec<String> = patterns.iter().map(pattern_key).collect();
            format!("or:{patterns:?}")
        }
    }
}

fn pattern_is_irrefutable(pattern: &ast::Pattern) -> bool {
    match pattern {
        ast::Pattern::Var(_, _) | ast::Pattern::Wildcard(_) => true,
        ast::Pattern::Tuple(elems, _, _) => elems.iter().all(pattern_is_irrefutable),
        ast::Pattern::Struct { fields, .. } => fields
            .iter()
            .all(|(_, sub)| sub.as_ref().is_none_or(pattern_is_irrefutable)),
        ast::Pattern::Ref(inner, _, _) => pattern_is_irrefutable(inner),
        ast::Pattern::Or(patterns, _) => patterns.iter().any(pattern_is_irrefutable),
        ast::Pattern::Lit(_, _) | ast::Pattern::Enum { .. } => false,
    }
}

fn collect_covered_variants(pattern: &ast::Pattern, out: &mut HashSet<String>) {
    match pattern {
        ast::Pattern::Enum { variant, .. } => {
            out.insert(variant.clone());
        }
        ast::Pattern::Or(patterns, _) => {
            for pattern in patterns {
                collect_covered_variants(pattern, out);
            }
        }
        _ => {}
    }
}

/// 模式绑定的变量名（排序后用于比较 `or` 各分支）。
fn pattern_binding_names(pattern: &ast::Pattern) -> Vec<String> {
    fn collect(pattern: &ast::Pattern, names: &mut Vec<String>) {
        match pattern {
            ast::Pattern::Var(name, _) => names.push(name.clone()),
            ast::Pattern::Tuple(elems, _, _) | ast::Pattern::Or(elems, _) => {
                for elem in elems {
                    collect(elem, names);
                }
            }
            ast::Pattern::Struct { fields, .. } => {
                for (field, sub) in fields {
                    match sub {
                        Some(sub) => collect(sub, names),
                        None => names.push(field.clone()),
                    }
                }
            }
            ast::Pattern::Enum { inner, .. } => {
                if let Some(inner) = inner {
                    collect(inner, names);
                }
            }
            ast::Pattern::Ref(inner, _, _) => collect(inner, names),
            ast::Pattern::Lit(_, _) | ast::Pattern::Wildcard(_) => {}
        }
    }

    let mut names = Vec::new();
    collect(pattern, &mut names);
    names.sort();
    names.dedup();
    names
}

fn collect_covered_bools(pattern: &ast::Pattern, true_covered: &mut bool, false_covered: &mut bool) {
    match pattern {
        ast::Pattern::Lit(ast::Literal::Bool(value), _) => {
            if *value {
                *true_covered = true;
            } else {
                *false_covered = true;
            }
        }
        ast::Pattern::Or(patterns, _) => {
            for pattern in patterns {
                collect_covered_bools(pattern, true_covered, false_covered);
            }
        }
        _ => {}
    }
}

fn attribute_value(attr: &ast::Attribute) -> Option<&str> {
    attr.args.iter().find_map(|arg| match arg {
        ast::AttributeArg::KeyValue(key, value) if key == &attr.name => Some(value.as_str()),
        ast::AttributeArg::Value(value) => Some(value.as_str()),
        _ => None,
    })
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

/// `as` 只在标量之间转换：数值、char、bool。
fn is_castable(from: &HirType, to: &HirType) -> bool {
    if is_unknown(from) || is_unknown(to) {
        return true;
    }
    let scalar = |ty: &HirType| is_numeric(ty) || is_char(ty) || is_bool(ty);
    scalar(from) && scalar(to)
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

/// 便捷入口：检查整个 HIR 程序（丢弃警告）。
pub fn check_program(program: &HirProgram, type_table: &TypeTable) -> Vec<TypeCheckError> {
    check_program_with_warnings(program, type_table).0
}

/// 便捷入口：检查整个 HIR 程序，返回错误与警告。
pub fn check_program_with_warnings(
    program: &HirProgram,
    type_table: &TypeTable,
) -> (Vec<TypeCheckError>, Vec<TypeCheckWarning>) {
    TypeChecker::new(program, type_table).check_program_with_warnings(program)
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

    #[test]
    fn test_for_infers_element_types() {
        let errors = check(
            "fn main() { for x in [1, 2, 3] { let y: i32 = x; } for c in \"ab\" { let ch: char = c; } }",
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn test_for_infers_vec_element_type_through_ref() {
        let errors = check(
            "fn main() { let v = Vec::new(); for x in &v { let y: i32 = x; } }",
        );
        // Vec::new() 的元素类型未知，不应误报；关键是引用包装不会触发 iterable 错误。
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn test_for_rejects_non_iterable() {
        let errors = check("fn main() { for x in 1 { } }");
        assert!(
            errors.iter().any(|e| matches!(
                e,
                TypeCheckError::TypeMismatch { context, .. } if context.contains("for")
            )),
            "{errors:?}"
        );
    }

    #[test]
    fn test_ret_arity_mismatch() {
        let errors = check("fn f() -> i32 { ret 2; } fn main() { }");
        assert!(
            errors.iter().any(|e| matches!(e, TypeCheckError::ReturnArityMismatch { .. })),
            "{errors:?}"
        );
    }

    #[test]
    fn test_as_rejects_non_scalar_cast() {
        let errors = check("fn main() { let x = \"hello\" as i64; }");
        assert!(
            errors.iter().any(|e| matches!(
                e,
                TypeCheckError::TypeMismatch { context, .. } if context.contains("as")
            )),
            "{errors:?}"
        );
    }
}
