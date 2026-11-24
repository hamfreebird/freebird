use crate::frontend::ast;
use crate::frontend::token::Span;

/// HIR 中的变量引用 — 已消解为具体的声明点
#[derive(Debug, Clone)]
pub struct HirVarRef {
    pub name: String,
    /// 从当前作用域向外的层数（0 = 当前作用域）
    pub scope_depth: usize,
    pub span: Span,
}

/// 函数签名（编译期信息）
#[derive(Debug, Clone)]
pub struct FnSignature {
    pub name: String,
    pub params: Vec<(String, HirType)>,
    pub return_types: Vec<HirType>,
    pub n: u32, // 参数个数
    pub m: u32, // 返回值个数
    pub span: Span,
}

/// HIR 类型（简化版，Phase 5 完善）
#[derive(Debug, Clone, PartialEq)]
pub enum HirType {
    Path(Vec<String>),
    Generic(Box<HirType>, Vec<HirType>),
    Ref(Box<HirType>, bool),
    Tuple(Vec<HirType>),
    Array(Box<HirType>),
    FnPtr(Vec<HirType>, Vec<HirType>),
    Never,
    Wildcard,
    Unit,
    /// 尚未确定类型（待类型检查阶段推断）
    Unknown,
}

impl HirType {
    pub fn name(&self) -> String {
        match self {
            HirType::Path(segments) => segments.join("::"),
            HirType::Generic(base, args) => {
                let args: Vec<String> = args.iter().map(HirType::name).collect();
                format!("{}<{}>", base.name(), args.join(", "))
            }
            HirType::Ref(inner, true) => format!("&mut {}", inner.name()),
            HirType::Ref(inner, false) => format!("&{}", inner.name()),
            HirType::Tuple(types) => {
                let inner: Vec<String> = types.iter().map(|t| t.name()).collect();
                format!("({})", inner.join(", "))
            }
            HirType::Array(inner) => format!("[{}]", inner.name()),
            HirType::FnPtr(params, rets) => {
                let p: Vec<String> = params.iter().map(|t| t.name()).collect();
                let r: Vec<String> = rets.iter().map(|t| t.name()).collect();
                format!("fn({}) -> ({})", p.join(", "), r.join(", "))
            }
            HirType::Never => "!".to_string(),
            HirType::Wildcard => "_".to_string(),
            HirType::Unit => "()".to_string(),
            HirType::Unknown => "?".to_string(),
        }
    }
}

// ─── HIR 表达式 ───

#[derive(Debug, Clone)]
pub enum HirExpr {
    Lit(ast::Literal, Span),
    Var(HirVarRef),
    IdRef(String, Span),
    Deref(Box<HirExpr>, Span),
    Ref(Box<HirExpr>, bool, Span),
    Binary(Box<HirExpr>, ast::BinOp, Box<HirExpr>, Span),
    Unary(ast::UnOp, Box<HirExpr>, Span),
    Call {
        func: Box<HirExpr>,
        args: Vec<HirCallArg>,
        stack_pass: bool,
        fn_sig: Option<FnSignature>,
        span: Span,
    },
    /// 方法调用: `receiver.method(args)`，目前用于 Vec/String 内建方法
    MethodCall {
        receiver: Box<HirExpr>,
        method: String,
        args: Vec<HirExpr>,
        span: Span,
    },
    /// 关联函数调用: `Vec::new()`、`String::new()`、`Box::new(x)`
    TypeMethod {
        type_name: Vec<String>,
        method: String,
        args: Vec<HirExpr>,
        span: Span,
    },
    FieldAccess(Box<HirExpr>, String, Span),
    Index(Box<HirExpr>, Box<HirExpr>, Span),
    Tuple(Vec<HirExpr>, Span),
    Array(Vec<HirExpr>, Span),
    Repeat(Box<HirExpr>, Box<HirExpr>, Span),
    StructLit {
        name: Vec<String>,
        fields: Vec<(String, HirExpr)>,
        span: Span,
    },
    EnumLit {
        name: Vec<String>,
        variant: String,
        data: Option<Box<HirExpr>>,
        span: Span,
    },
    If(Box<HirExpr>, HirBlock, Option<HirBlock>, Span),
    Match(Box<HirExpr>, Vec<HirMatchArm>, Span),
    Loop(HirBlock, Span),
    While(Box<HirExpr>, HirBlock, Span),
    For {
        pattern: ast::Pattern,
        iterable: Box<HirExpr>,
        body: HirBlock,
        span: Span,
    },
    Block(HirBlock, Span),
    /// 带标签的循环: 'outer: loop { ... }
    Labelled(String, Box<HirExpr>, Span),
    Break(Option<Box<HirExpr>>, Span),
    /// break 'label [value]
    BreakLabel(String, Option<Box<HirExpr>>, Span),
    Continue(Span),
    /// continue 'label
    ContinueLabel(String, Span),
    Return(Vec<HirExpr>, Span),
    Ret(i32, Span),
    Unsafe(HirBlock, Span),
    StardustMacro(Vec<ast::StardustInst>, Span),
    As(Box<HirExpr>, HirType, Span),
    /// 显式类型实参（turbofish）：`f::<T>(...)`
    TypeApplied {
        callee: Box<HirExpr>,
        args: Vec<HirType>,
        span: Span,
    },
}

#[derive(Debug, Clone)]
pub enum HirCallArg {
    Pos(HirExpr, Span),
    Named(String, HirExpr, Span),
}

#[derive(Debug, Clone)]
pub struct HirBlock {
    pub stmts: Vec<HirStmt>,
    pub tail: Option<Box<HirExpr>>,
    pub span: Span,
    /// 本块引入的局部变量数（用于作用域管理）
    pub scope_size: usize,
}

#[derive(Debug, Clone)]
pub struct HirMatchArm {
    pub pattern: ast::Pattern,
    pub guard: Option<Box<HirExpr>>,
    pub body: Box<HirExpr>,
    pub span: Span,
}

// ─── HIR 语句 ───

#[derive(Debug, Clone)]
pub enum HirStmt {
    Let {
        pattern: ast::Pattern,
        ty: Option<HirType>,
        init: Option<HirExpr>,
        mutable: bool,
        span: Span,
    },
    Expr(HirExpr, Span),
    Assign {
        target: HirAssignTarget,
        value: HirExpr,
        span: Span,
    },
}

/// HIR 赋值目标（左值）
#[derive(Debug, Clone)]
pub enum HirAssignTarget {
    /// 简单变量
    Var(HirVarRef),
    /// 解引用赋值: *p = v
    Deref(HirExpr),
    /// 字段赋值: obj.field = v
    Field { object: Box<HirExpr>, field: String },
    /// 索引赋值: arr[i] = v
    Index {
        array: Box<HirExpr>,
        index: Box<HirExpr>,
    },
}

// ─── HIR 声明 ───

#[derive(Debug, Clone)]
pub enum HirDecl {
    Fn(HirFnDecl),
    Struct(ast::StructDecl),
    Enum(ast::EnumDecl),
    Extern(HirExternBlock),
    Use(Vec<String>, Span),
}

#[derive(Debug, Clone)]
pub struct HirFnDecl {
    pub name: String,
    /// 泛型参数（MVP：内建 trait 约束，如 `<T: Ord>`）。
    pub generics: Vec<ast::GenericParam>,
    pub sig: FnSignature,
    pub body: Option<HirBlock>,
    pub attrs: Vec<ast::Attribute>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct HirExternBlock {
    pub abi: String,
    pub items: Vec<HirExternItem>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum HirExternItem {
    Fn(HirFnDecl),
}

// ─── HIR 程序 ───

#[derive(Debug, Clone)]
pub struct HirProgram {
    pub decls: Vec<HirDecl>,
    /// 所有声明的函数签名（用于前向引用）
    pub fn_signatures: Vec<FnSignature>,
}

// ─── AST 类型到 HIR 类型的转换 ───

impl HirType {
    pub fn from_ast_type(ast_ty: &ast::Type) -> Self {
        match ast_ty {
            ast::Type::Path(segments, _) => HirType::Path(segments.clone()),
            ast::Type::Generic(base, args, _) => HirType::Generic(
                Box::new(HirType::from_ast_type(base)),
                args.iter().map(HirType::from_ast_type).collect(),
            ),
            ast::Type::Ref(inner, mutable, _) => {
                HirType::Ref(Box::new(HirType::from_ast_type(inner)), *mutable)
            }
            ast::Type::Tuple(types, _) => {
                HirType::Tuple(types.iter().map(HirType::from_ast_type).collect())
            }
            ast::Type::Array(elem, _, _) => HirType::Array(Box::new(HirType::from_ast_type(elem))),
            ast::Type::FnPtr(params, ret, _) => {
                let p: Vec<HirType> = params.iter().map(HirType::from_ast_type).collect();
                // 返回类型列表与函数声明保持一致：`-> (A, B)` 表示两个返回值。
                let r = match ret {
                    Some(rt) => match rt.as_ref() {
                        ast::Type::Tuple(types, _) => {
                            types.iter().map(HirType::from_ast_type).collect()
                        }
                        other => vec![HirType::from_ast_type(other)],
                    },
                    None => vec![],
                };
                HirType::FnPtr(p, r)
            }
            ast::Type::Never(_) => HirType::Never,
            ast::Type::Wildcard(_) => HirType::Wildcard,
        }
    }
}
