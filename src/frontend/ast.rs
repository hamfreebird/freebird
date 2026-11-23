use crate::frontend::token::Span;

// ─── 类型 AST ───

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// 路径类型: i32, std::Vec 等
    Path(Vec<String>, Span),
    /// 泛型类型: Vec<i32>, Box<T>
    Generic(Box<Type>, Vec<Type>, Span),
    /// 引用类型: &T, &mut T
    Ref(Box<Type>, bool, Span),
    /// 元组类型: (T1, T2, ...)
    Tuple(Vec<Type>, Span),
    /// 固定数组类型: [T; N]
    Array(Box<Type>, Box<Expr>, Span),
    /// 函数指针类型: fn(A, B) -> C
    FnPtr(Vec<Type>, Option<Box<Type>>, Span),
    /// Never 类型: !
    Never(Span),
    /// 通配符类型: _
    Wildcard(Span),
}

// ─── 模式 AST ───

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// 变量模式: x
    Var(String, Span),
    /// 字面量模式: 42, true
    Lit(Literal, Span),
    /// 通配符: _
    Wildcard(Span),
    /// 元组解构: (a, b, c) 或 (a, b, ..)
    Tuple(Vec<Pattern>, Option<Span>, Span),
    /// 结构体解构: Point { x, y }
    Struct {
        name: Vec<String>,
        fields: Vec<(String, Option<Pattern>)>,
        rest: bool, // 是否有 ..
        span: Span,
    },
    /// 枚举变体解构: Some(x)
    Enum {
        name: Vec<String>,
        variant: String,
        inner: Option<Box<Pattern>>,
        span: Span,
    },
    /// 引用模式: &x, &mut x
    Ref(Box<Pattern>, bool, Span),
    /// or 模式: a | b
    Or(Vec<Pattern>, Span),
}

// ─── 字面量 AST ───

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64, IntSuffix),
    Float(f64),
    Bool(bool),
    Char(u32),
    String(String),
    Unit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntSuffix {
    None,
    I8,
    I16,
    I32,
    I64,
    Isize,
    U8,
    U16,
    U32,
    U64,
    Usize,
}

// ─── 表达式 AST ───

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// 字面量
    Lit(Literal, Span),
    /// 变量引用
    Var(String, Span),
    /// ElementId 引用: #var
    IdRef(String, Span),
    /// 解引用: *expr
    Deref(Box<Expr>, Span),
    /// 引用: &expr, &mut expr
    Ref(Box<Expr>, bool, Span),
    /// 二元操作: lhs op rhs
    Binary(Box<Expr>, BinOp, Box<Expr>, Span),
    /// 一元操作
    Unary(UnOp, Box<Expr>, Span),
    /// 函数调用: func(args), func(a: x, b: y), func()
    Call {
        func: Box<Expr>,
        args: Vec<CallArg>,
        stack_pass: bool,
        span: Span,
    },
    /// 字段访问: expr.field
    FieldAccess(Box<Expr>, String, Span),
    /// 索引访问: expr[index]
    Index(Box<Expr>, Box<Expr>, Span),
    /// 元组构造: (a, b, c)
    Tuple(Vec<Expr>, Span),
    /// 数组构造: [a, b, c] 或 [expr; count]
    Array(Vec<Expr>, Span),
    Repeat(Box<Expr>, Box<Expr>, Span),
    /// 结构体构造: Point { x: 1, y: 2 }
    StructLit {
        name: Vec<String>,
        fields: Vec<(String, Expr)>,
        span: Span,
    },
    /// 枚举构造: Option::Some(42)
    EnumLit {
        name: Vec<String>,
        variant: String,
        data: Option<Box<Expr>>,
        span: Span,
    },
    /// if 表达式: if cond { then } else { else_ }
    If(Box<Expr>, Block, Option<Block>, Span),
    /// match 表达式
    Match(Box<Expr>, Vec<MatchArm>, Span),
    /// loop 表达式
    Loop(Block, Span),
    /// while 表达式
    While(Box<Expr>, Block, Span),
    /// for 表达式
    For {
        pattern: Pattern,
        iterable: Box<Expr>,
        body: Block,
        span: Span,
    },
    /// 块表达式: { ... }
    Block(Block, Span),
    /// 带标签的循环: 'outer: loop { ... }
    Labelled(String, Box<Expr>, Span),
    /// break 表达式
    Break(Option<Box<Expr>>, Span),
    /// break 'label [value]
    BreakLabel(String, Option<Box<Expr>>, Span),
    /// continue 表达式
    Continue(Span),
    /// continue 'label
    ContinueLabel(String, Span),
    /// return 表达式: return expr1, expr2, ...
    Return(Vec<Expr>, Span),
    /// ret n 指令（低级）
    Ret(i32, Span),
    /// unsafe 块
    Unsafe(Block, Span),
    /// stardust! 宏
    StardustMacro(Vec<StardustInst>, Span),
    /// as 类型转换: expr as Type
    As(Box<Expr>, Box<Type>, Span),
}

/// 函数调用参数
#[derive(Debug, Clone, PartialEq)]
pub enum CallArg {
    /// 位置参数: expr
    Pos(Expr, Span),
    /// 命名参数: name: expr
    Named(String, Expr, Span),
}

/// 二元操作符
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    Xor,
    Assign,
}

/// 一元操作符
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Deref,
}

/// 块 = 一系列语句 + 可选尾表达式
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

/// match 分支
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Box<Expr>>,
    pub body: Box<Expr>,
    pub span: Span,
}

/// Stardust 低级指令
#[derive(Debug, Clone, PartialEq)]
pub enum StardustInst {
    Push(i64),
    Pop,
    Dup,
    Swap,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    Not,
    Xor,
    Store,
    Load,
    ShiftL,
    ShiftR,
    Depth,
    Reverse,
    Rotate,
    Call { n: u32, m: u32 },
    Ret { m: u32 },
    Ident(String), // 自定义 Stardust 指令
}

// ─── 语句 AST ───

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// let 语句: let x = expr; 或 let mut x: Type = expr;
    Let {
        pattern: Pattern,
        ty: Option<Type>,
        init: Option<Expr>,
        mutable: bool,
        span: Span,
    },
    /// 表达式语句: expr;
    Expr(Expr, Span),
    /// 赋值语句: x = expr;
    Assign {
        target: Expr,
        value: Expr,
        span: Span,
    },
}

// ─── 顶层声明 AST ───

#[derive(Debug, Clone, PartialEq)]
pub enum Decl {
    /// 函数声明
    Fn(FnDecl),
    /// 结构体声明
    Struct(StructDecl),
    /// 枚举声明
    Enum(EnumDecl),
    /// 外部块: extern "stardust" { ... }
    Extern(ExternBlock),
    /// 使用声明: use path::to::name;
    Use(Vec<String>, Span),
}

/// 函数声明
#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<FnParam>,
    pub return_type: Option<Type>,
    pub body: Option<Block>,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

/// 函数参数
#[derive(Debug, Clone, PartialEq)]
pub struct FnParam {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

/// 结构体声明
#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    pub name: String,
    pub fields: Vec<StructField>,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructField {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

/// 枚举声明
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    pub name: String,
    pub variants: Vec<EnumVariant>,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    pub data: Option<Type>,
    pub span: Span,
}

/// extern 块
#[derive(Debug, Clone, PartialEq)]
pub struct ExternBlock {
    pub abi: String,
    pub items: Vec<ExternItem>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExternItem {
    Fn(FnDecl),
}

/// 属性: #[attr(key = value)] 或 #[attr]
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub args: Vec<AttributeArg>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttributeArg {
    KeyValue(String, String),
    Value(String),
}

// ─── 程序根节点 ───

#[derive(Debug, Clone)]
pub struct Program {
    pub decls: Vec<Decl>,
}

// ─── 运算符优先级 ───

impl BinOp {
    /// 返回运算符的优先级（数值越大优先级越高）
    pub fn precedence(&self) -> u8 {
        match self {
            BinOp::Assign => 1,
            BinOp::Or => 2,
            BinOp::And => 3,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => 4,
            BinOp::Add | BinOp::Sub => 5,
            BinOp::Mul | BinOp::Div | BinOp::Mod => 6,
            BinOp::Xor => 3, // 与 And 同级
        }
    }

    /// 是否右结合
    pub fn is_right_associative(&self) -> bool {
        matches!(self, BinOp::Assign)
    }

    /// 从 TokenKind 转换
    pub fn from_token_kind(kind: &crate::frontend::token::TokenKind) -> Option<Self> {
        match kind {
            crate::frontend::token::TokenKind::Plus => Some(BinOp::Add),
            crate::frontend::token::TokenKind::Minus => Some(BinOp::Sub),
            crate::frontend::token::TokenKind::Star => Some(BinOp::Mul),
            crate::frontend::token::TokenKind::Slash => Some(BinOp::Div),
            crate::frontend::token::TokenKind::Percent => Some(BinOp::Mod),
            crate::frontend::token::TokenKind::EqEq => Some(BinOp::Eq),
            crate::frontend::token::TokenKind::Ne => Some(BinOp::Ne),
            crate::frontend::token::TokenKind::Lt => Some(BinOp::Lt),
            crate::frontend::token::TokenKind::Gt => Some(BinOp::Gt),
            crate::frontend::token::TokenKind::Le => Some(BinOp::Le),
            crate::frontend::token::TokenKind::Ge => Some(BinOp::Ge),
            crate::frontend::token::TokenKind::AndAnd => Some(BinOp::And),
            crate::frontend::token::TokenKind::OrOr => Some(BinOp::Or),
            crate::frontend::token::TokenKind::Caret => Some(BinOp::Xor),
            _ => None,
        }
    }
}
