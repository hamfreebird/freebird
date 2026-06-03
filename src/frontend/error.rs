use std::fmt;

use crate::frontend::token::{Span, Token};

/// 前端错误
#[derive(Debug, Clone)]
pub struct FrontendError {
    pub kind: FrontendErrorKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum FrontendErrorKind {
    // 词法错误
    UnterminatedString,
    UnterminatedChar,
    InvalidCharEscape(String),
    InvalidEscapeSequence(String),
    UnexpectedChar(char),
    InvalidNumber(String),

    // 语法错误
    UnexpectedToken {
        expected: String,
        found: Token,
    },
    UnexpectedEof {
        expected: String,
    },
    DuplicateField {
        name: String,
        first: Span,
    },
    UnknownAttribute(String),
    MissingAttributeArg(String),

    // 语义错误（前端检测）
    MixedCallArgs(Span),
}

impl FrontendError {
    pub fn new(kind: FrontendErrorKind, span: Span) -> Self {
        Self { kind, span }
    }
}

impl fmt::Display for FrontendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.span, self.kind)
    }
}

impl fmt::Display for FrontendErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrontendErrorKind::UnterminatedString => write!(f, "unterminated string literal"),
            FrontendErrorKind::UnterminatedChar => write!(f, "unterminated character literal"),
            FrontendErrorKind::InvalidCharEscape(s) => write!(f, "invalid character escape: {s}"),
            FrontendErrorKind::InvalidEscapeSequence(s) => {
                write!(f, "invalid escape sequence: \\{s}")
            }
            FrontendErrorKind::UnexpectedChar(c) => write!(f, "unexpected character: '{c}'"),
            FrontendErrorKind::InvalidNumber(s) => write!(f, "invalid number literal: {s}"),
            FrontendErrorKind::UnexpectedToken { expected, found } => {
                write!(f, "expected {expected}, found {found}")
            }
            FrontendErrorKind::UnexpectedEof { expected } => {
                write!(f, "expected {expected}, found end of file")
            }
            FrontendErrorKind::DuplicateField { name, first } => {
                write!(f, "duplicate field `{name}` (first defined at {first})")
            }
            FrontendErrorKind::UnknownAttribute(name) => {
                write!(f, "unknown attribute: #[{name}]")
            }
            FrontendErrorKind::MissingAttributeArg(name) => {
                write!(f, "missing argument for attribute #[{name}]")
            }
            FrontendErrorKind::MixedCallArgs(span) => {
                write!(f, "{span}: cannot mix named and positional arguments")
            }
        }
    }
}

impl std::error::Error for FrontendError {}
