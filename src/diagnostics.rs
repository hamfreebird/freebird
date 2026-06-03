//! JSON 诊断输出 — 用于 freebird check 命令
//!
//! 生成结构化的诊断信息，包含源码位置、错误级别、修复建议。

use serde::Serialize;

use crate::frontend::error::FrontendError;
use crate::hir::resolver::ResolveError;
use crate::borrowck::checker::BorrowError;
use crate::frontend::token::Span;

/// 诊断级别
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Note,
    Help,
}

/// 诊断消息
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
    pub span: SourceSpan,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// 源码位置
#[derive(Debug, Clone, Serialize)]
pub struct SourceSpan {
    pub line: usize,
    pub column: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_column: Option<usize>,
}

impl SourceSpan {
    pub fn from_span(span: Span) -> Self {
        Self {
            line: span.line,
            column: span.column,
            end_line: None,
            end_column: None,
        }
    }
}

/// 将前端错误转换为诊断消息
pub fn frontend_to_diagnostics(errors: &[FrontendError]) -> Vec<Diagnostic> {
    errors.iter().map(|e| {
        Diagnostic {
            level: DiagnosticLevel::Error,
            message: format!("{e}"),
            span: SourceSpan::from_span(e.span),
            notes: vec![],
            suggestion: None,
        }
    }).collect()
}

/// 将名称解析错误转换为诊断消息
pub fn resolve_to_diagnostics(errors: &[ResolveError]) -> Vec<Diagnostic> {
    errors.iter().map(|e| {
        let (span, suggestion) = match e {
            ResolveError::UndefinedVariable { span, .. } => {
                (*span, Some("did you forget to declare this variable?".to_string()))
            }
            ResolveError::ForwardReference { span, .. } => {
                (*span, Some("variables must be initialized before use".to_string()))
            }
            ResolveError::DuplicateDefinition { second, .. } => {
                (*second, Some("rename or remove the duplicate definition".to_string()))
            }
            ResolveError::DuplicateField { second, .. } => {
                (*second, Some("rename or remove the duplicate field".to_string()))
            }
            ResolveError::ArgumentCountMismatch { span, expected, got, .. } => {
                (*span, Some(format!("expected {expected} arguments, got {got}")))
            }
            ResolveError::CannotAssignToImmutable { span, .. } => {
                (*span, Some("declare the variable with `let mut` to make it mutable".to_string()))
            }
        };
        Diagnostic {
            level: DiagnosticLevel::Error,
            message: format!("{e}"),
            span: SourceSpan::from_span(span),
            notes: vec![],
            suggestion,
        }
    }).collect()
}

/// 将借用检查错误转换为诊断消息
pub fn borrow_to_diagnostics(errors: &[BorrowError]) -> Vec<Diagnostic> {
    errors.iter().map(|e| {
        let (span, suggestion) = match e {
            BorrowError::UseAfterMove { span, .. } => {
                (*span, Some("the value was moved and can no longer be used".to_string()))
            }
            BorrowError::UseWhileBorrowedMut { span, .. } => {
                (*span, Some("wait for the mutable borrow to end before using this variable".to_string()))
            }
            BorrowError::BorrowWhileBorrowedMut { span, .. } => {
                (*span, Some("cannot borrow while a mutable borrow is active".to_string()))
            }
            BorrowError::MutBorrowWhileBorrowed { span, .. } => {
                (*span, Some("release existing borrows before taking a mutable borrow".to_string()))
            }
            BorrowError::MoveWhileBorrowed { span, .. } => {
                (*span, Some("cannot move a borrowed value".to_string()))
            }
            BorrowError::CannotAssignToImmutable { span, .. } => {
                (*span, Some("declare the variable with `let mut` to make it mutable".to_string()))
            }
        };
        Diagnostic {
            level: DiagnosticLevel::Error,
            message: format!("{e}"),
            span: SourceSpan::from_span(span),
            notes: vec![],
            suggestion,
        }
    }).collect()
}

/// 生成完整的 JSON 诊断报告
pub fn generate_diagnostic_json(
    source: &str,
    parse_errors: &[FrontendError],
    resolve_errors: &[ResolveError],
    borrow_errors: &[BorrowError],
) -> String {
    let mut diagnostics = Vec::new();
    diagnostics.extend(frontend_to_diagnostics(parse_errors));
    diagnostics.extend(resolve_to_diagnostics(resolve_errors));
    diagnostics.extend(borrow_to_diagnostics(borrow_errors));

    let error_count = diagnostics.len();
    let report = DiagnosticReport {
        source: source.to_string(),
        errors: diagnostics,
        error_count,
    };

    serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{{}}".to_string())
}

#[derive(Serialize)]
struct DiagnosticReport {
    source: String,
    errors: Vec<Diagnostic>,
    error_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::token::Span;

    #[test]
    fn test_empty_diagnostics() {
        let json = generate_diagnostic_json("", &[], &[], &[]);
        assert!(json.contains("\"error_count\": 0"));
    }

    #[test]
    fn test_parse_error_diagnostic() {
        let errors = vec![FrontendError::new(
            crate::frontend::error::FrontendErrorKind::UnexpectedChar('@'),
            Span::new(1, 1, 0),
        )];
        let json = generate_diagnostic_json("@", &errors, &[], &[]);
        assert!(json.contains("unexpected character"));
    }
}
