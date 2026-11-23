use std::fmt;

/// 源码位置
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub line: usize,   // 1-based
    pub column: usize, // 1-based
    pub byte_pos: usize,
}

impl Span {
    pub fn new(line: usize, column: usize, byte_pos: usize) -> Self {
        Self { line, column, byte_pos }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// Token 类型
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // === 字面量 ===
    IntLiteral(i64, IntSuffix),
    FloatLiteral(f64),
    BoolLiteral(bool),
    CharLiteral(u32),     // Unicode 标量值
    StringLiteral(String),

    // === 标识符 ===
    Ident(String),
    /// 循环标签: `'outer`
    Label(String),

    // === 关键字 ===
    Fn,
    Let,
    Mut,
    If,
    Else,
    While,
    Loop,
    For,
    In,
    Match,
    Return,
    Ret,
    Struct,
    Enum,
    Impl,
    Mod,
    Pub,
    Extern,
    Use,
    Unsafe,
    As,
    Break,
    Continue,
    True,
    False,
    Stardust, // stardust! 宏关键字
    ElementId, // #var 的 # 前缀（词法归类为关键字）

    // === 运算符与分隔符 ===
    // 单字符
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Percent,    // %
    Eq,         // =
    Bang,       // !
    Lt,         // <
    Gt,         // >
    And,        // &
    Or,         // |
    Caret,      // ^
    Dot,        // .
    Comma,      // ,
    Colon,      // :
    Semicolon,  // ;
    Hash,       // #
    At,         // @

    // 双字符
    EqEq,       // ==
    Ne,         // !=
    Le,         // <=
    Ge,         // >=
    AndAnd,     // &&
    OrOr,       // ||
    Arrow,      // ->
    FatArrow,   // =>
    ColonColon, // ::
    DotDot,     // ..
    DotDotEq,   // ..=

    // === 分隔符 ===
    LParen,    // (
    RParen,    // )
    LBrace,    // {
    RBrace,    // }
    LBracket,  // [
    RBracket,  // ]

    // === 特殊 ===
    Underscore,  // _
    Eof,
    Error(String),
}

/// 整数后缀类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntSuffix {
    None,  // 无后缀，默认 i32
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

impl IntSuffix {
    pub fn parse(s: &str) -> Self {
        match s {
            "i8" => IntSuffix::I8,
            "i16" => IntSuffix::I16,
            "i32" => IntSuffix::I32,
            "i64" => IntSuffix::I64,
            "isize" => IntSuffix::Isize,
            "u8" => IntSuffix::U8,
            "u16" => IntSuffix::U16,
            "u32" => IntSuffix::U32,
            "u64" => IntSuffix::U64,
            "usize" => IntSuffix::Usize,
            _ => IntSuffix::None,
        }
    }
}

/// Token 实例（含位置信息）
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::IntLiteral(v, suffix) => {
                let s = match suffix {
                    IntSuffix::None => "",
                    IntSuffix::I8 => "i8",
                    IntSuffix::I16 => "i16",
                    IntSuffix::I32 => "i32",
                    IntSuffix::I64 => "i64",
                    IntSuffix::Isize => "isize",
                    IntSuffix::U8 => "u8",
                    IntSuffix::U16 => "u16",
                    IntSuffix::U32 => "u32",
                    IntSuffix::U64 => "u64",
                    IntSuffix::Usize => "usize",
                };
                write!(f, "{v}{s}")
            }
            TokenKind::FloatLiteral(v) => write!(f, "{v}"),
            TokenKind::BoolLiteral(b) => write!(f, "{b}"),
            TokenKind::CharLiteral(c) => {
                if let Some(ch) = char::from_u32(*c) {
                    write!(f, "'{ch}'")
                } else {
                    write!(f, "'\\u{{{c}}}'")
                }
            }
            TokenKind::Label(name) => write!(f, "'{name}"),
            TokenKind::StringLiteral(s) => write!(f, "\"{s}\""),
            TokenKind::Ident(s) => write!(f, "{s}"),
            TokenKind::Fn => write!(f, "fn"),
            TokenKind::Let => write!(f, "let"),
            TokenKind::Mut => write!(f, "mut"),
            TokenKind::If => write!(f, "if"),
            TokenKind::Else => write!(f, "else"),
            TokenKind::While => write!(f, "while"),
            TokenKind::Loop => write!(f, "loop"),
            TokenKind::For => write!(f, "for"),
            TokenKind::In => write!(f, "in"),
            TokenKind::Match => write!(f, "match"),
            TokenKind::Return => write!(f, "return"),
            TokenKind::Ret => write!(f, "ret"),
            TokenKind::Struct => write!(f, "struct"),
            TokenKind::Enum => write!(f, "enum"),
            TokenKind::Impl => write!(f, "impl"),
            TokenKind::Mod => write!(f, "mod"),
            TokenKind::Pub => write!(f, "pub"),
            TokenKind::Extern => write!(f, "extern"),
            TokenKind::Use => write!(f, "use"),
            TokenKind::Unsafe => write!(f, "unsafe"),
            TokenKind::As => write!(f, "as"),
            TokenKind::Break => write!(f, "break"),
            TokenKind::Continue => write!(f, "continue"),
            TokenKind::True => write!(f, "true"),
            TokenKind::False => write!(f, "false"),
            TokenKind::Stardust => write!(f, "stardust"),
            TokenKind::ElementId => write!(f, "#id"),
            TokenKind::Plus => write!(f, "+"),
            TokenKind::Minus => write!(f, "-"),
            TokenKind::Star => write!(f, "*"),
            TokenKind::Slash => write!(f, "/"),
            TokenKind::Percent => write!(f, "%"),
            TokenKind::Eq => write!(f, "="),
            TokenKind::Bang => write!(f, "!"),
            TokenKind::Lt => write!(f, "<"),
            TokenKind::Gt => write!(f, ">"),
            TokenKind::And => write!(f, "&"),
            TokenKind::Or => write!(f, "|"),
            TokenKind::Caret => write!(f, "^"),
            TokenKind::Dot => write!(f, "."),
            TokenKind::Comma => write!(f, ","),
            TokenKind::Colon => write!(f, ":"),
            TokenKind::Semicolon => write!(f, ";"),
            TokenKind::Hash => write!(f, "#"),
            TokenKind::At => write!(f, "@"),
            TokenKind::EqEq => write!(f, "=="),
            TokenKind::Ne => write!(f, "!="),
            TokenKind::Le => write!(f, "<="),
            TokenKind::Ge => write!(f, ">="),
            TokenKind::AndAnd => write!(f, "&&"),
            TokenKind::OrOr => write!(f, "||"),
            TokenKind::Arrow => write!(f, "->"),
            TokenKind::FatArrow => write!(f, "=>"),
            TokenKind::ColonColon => write!(f, "::"),
            TokenKind::DotDot => write!(f, ".."),
            TokenKind::DotDotEq => write!(f, "..="),
            TokenKind::LParen => write!(f, "("),
            TokenKind::RParen => write!(f, ")"),
            TokenKind::LBrace => write!(f, "{{"),
            TokenKind::RBrace => write!(f, "}}"),
            TokenKind::LBracket => write!(f, "["),
            TokenKind::RBracket => write!(f, "]"),
            TokenKind::Underscore => write!(f, "_"),
            TokenKind::Eof => write!(f, "<EOF>"),
            TokenKind::Error(msg) => write!(f, "<Error: {msg}>"),
        }
    }
}

/// 关键字查找
pub fn lookup_keyword(name: &str) -> Option<TokenKind> {
    match name {
        "fn" => Some(TokenKind::Fn),
        "let" => Some(TokenKind::Let),
        "mut" => Some(TokenKind::Mut),
        "if" => Some(TokenKind::If),
        "else" => Some(TokenKind::Else),
        "while" => Some(TokenKind::While),
        "loop" => Some(TokenKind::Loop),
        "for" => Some(TokenKind::For),
        "in" => Some(TokenKind::In),
        "match" => Some(TokenKind::Match),
        "return" => Some(TokenKind::Return),
        "ret" => Some(TokenKind::Ret),
        "struct" => Some(TokenKind::Struct),
        "enum" => Some(TokenKind::Enum),
        "impl" => Some(TokenKind::Impl),
        "mod" => Some(TokenKind::Mod),
        "pub" => Some(TokenKind::Pub),
        "extern" => Some(TokenKind::Extern),
        "use" => Some(TokenKind::Use),
        "unsafe" => Some(TokenKind::Unsafe),
        "as" => Some(TokenKind::As),
        "break" => Some(TokenKind::Break),
        "continue" => Some(TokenKind::Continue),
        "true" => Some(TokenKind::True),
        "false" => Some(TokenKind::False),
        "stardust" => Some(TokenKind::Stardust),
        _ => None,
    }
}
