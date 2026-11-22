//! JSON 诊断输出 — 用于 freebird check 命令
//!
//! 生成结构化的诊断信息，包含源码位置、错误级别、修复建议。

use serde::Serialize;

use crate::frontend::error::FrontendError;
use crate::hir::resolver::ResolveError;
use crate::borrowck::checker::BorrowError;
use crate::typeck::{TypeCheckError, TypeCheckWarning};
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
            ResolveError::InvalidAssignmentTarget { span } => {
                (*span, Some("assignment targets must be a variable, `*ref`, field, or index".to_string()))
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

/// 将类型检查错误转换为诊断消息
pub fn type_to_diagnostics(errors: &[TypeCheckError]) -> Vec<Diagnostic> {
    errors.iter().map(|e| {
        let suggestion = match e {
            TypeCheckError::TypeMismatch { .. } => {
                Some("change the value or the expected type annotation".to_string())
            }
            TypeCheckError::ConditionNotBool { .. } => {
                Some("conditions must be `bool`; use a comparison or a bool variable".to_string())
            }
            TypeCheckError::UnknownType { name, .. } => {
                Some(format!("declare `{name}` or fix the spelling"))
            }
            TypeCheckError::UnknownFunction { name, .. } => {
                Some(format!("declare function `{name}` or fix the spelling"))
            }
            TypeCheckError::UnknownField { .. } => {
                Some("check the field name and the struct type".to_string())
            }
            TypeCheckError::UnknownVariant { .. } => {
                Some("check the variant name and the enum type".to_string())
            }
            TypeCheckError::UnknownStruct { name, .. } => {
                Some(format!("declare struct `{name}` or fix the spelling"))
            }
            TypeCheckError::NotCallable { .. } => {
                Some("only functions can be called".to_string())
            }
            TypeCheckError::ArgumentCountMismatch { .. } => {
                Some("pass the number of arguments declared by the function".to_string())
            }
            TypeCheckError::ReturnArityMismatch { .. } => {
                Some("return the same number of values as declared".to_string())
            }
            TypeCheckError::LoopControlOutsideLoop { keyword, .. } => {
                Some(format!("move `{keyword}` inside a loop body"))
            }
            TypeCheckError::BreakWithValue { .. } => {
                Some("use `break` without a value until loop-value semantics are implemented"
                    .to_string())
            }
            TypeCheckError::LoopUsedAsValue { .. } => {
                Some("use the loop as a statement, or use `return` to leave the function"
                    .to_string())
            }
            TypeCheckError::NonExhaustiveMatch { .. } => {
                Some("add the missing variants or a `_` arm".to_string())
            }
            TypeCheckError::MissingFunctionBody { .. } => {
                Some("add `{ ... }`, or declare the function inside an `extern` block".to_string())
            }
            TypeCheckError::ExternCall { .. } => {
                Some("FFI calls are not implemented yet; provide a Freebird implementation"
                    .to_string())
            }
            TypeCheckError::UnsupportedFeature { .. } => {
                Some("remove the construct; it is reserved for a later milestone".to_string())
            }
            TypeCheckError::UnknownAttribute { .. } => {
                Some("remove the attribute or use a documented attribute".to_string())
            }
            TypeCheckError::InvalidAttribute { .. } => {
                Some("use the attribute syntax documented in readme.md".to_string())
            }
            TypeCheckError::RefutableLetPattern { .. } => {
                Some("replace the `let` with a `match` that covers all cases".to_string())
            }
            TypeCheckError::OrPatternBindingMismatch { .. } => {
                Some("bind the same variable names in every `or` alternative".to_string())
            }
        };
        Diagnostic {
            level: DiagnosticLevel::Error,
            message: format!("{e}"),
            span: SourceSpan::from_span(e.span()),
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
    type_errors: &[TypeCheckError],
    borrow_errors: &[BorrowError],
) -> String {
    generate_diagnostic_json_with_warnings(
        source,
        parse_errors,
        resolve_errors,
        type_errors,
        borrow_errors,
        &[],
    )
}

/// 生成包含类型检查警告的 JSON 诊断。
pub fn generate_diagnostic_json_with_warnings(
    source: &str,
    parse_errors: &[FrontendError],
    resolve_errors: &[ResolveError],
    type_errors: &[TypeCheckError],
    borrow_errors: &[BorrowError],
    type_warnings: &[TypeCheckWarning],
) -> String {
    let mut diagnostics = Vec::new();
    diagnostics.extend(frontend_to_diagnostics(parse_errors));
    diagnostics.extend(resolve_to_diagnostics(resolve_errors));
    diagnostics.extend(type_to_diagnostics(type_errors));
    diagnostics.extend(borrow_to_diagnostics(borrow_errors));
    diagnostics.extend(type_warnings_to_diagnostics(type_warnings));

    let error_count = diagnostics
        .iter()
        .filter(|diagnostic| matches!(diagnostic.level, DiagnosticLevel::Error))
        .count();
    let warning_count = diagnostics
        .iter()
        .filter(|diagnostic| matches!(diagnostic.level, DiagnosticLevel::Warning))
        .count();
    let report = DiagnosticReport {
        source: source.to_string(),
        errors: diagnostics,
        error_count,
        warning_count,
    };

    serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{{}}".to_string())
}

fn type_warnings_to_diagnostics(warnings: &[TypeCheckWarning]) -> Vec<Diagnostic> {
    warnings
        .iter()
        .map(|warning| {
            let span = match warning {
                TypeCheckWarning::UnreachablePattern { span } => *span,
            };
            Diagnostic {
                level: DiagnosticLevel::Warning,
                message: warning.to_string(),
                span: SourceSpan::from_span(span),
                notes: Vec::new(),
                suggestion: None,
            }
        })
        .collect()
}

#[derive(Serialize)]
struct DiagnosticReport {
    source: String,
    errors: Vec<Diagnostic>,
    error_count: usize,
    warning_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::token::Span;

    #[test]
    fn test_empty_diagnostics() {
        let json = generate_diagnostic_json("", &[], &[], &[], &[]);
        assert!(json.contains("\"error_count\": 0"));
    }

    #[test]
    fn test_parse_error_diagnostic() {
        let errors = vec![FrontendError::new(
            crate::frontend::error::FrontendErrorKind::UnexpectedChar('@'),
            Span::new(1, 1, 0),
        )];
        let json = generate_diagnostic_json("@", &errors, &[], &[], &[]);
        assert!(json.contains("unexpected character"));
    }

    #[test]
    fn test_type_error_diagnostic() {
        let errors = vec![TypeCheckError::ConditionNotBool {
            found: crate::hir::types::HirType::Path(vec!["i32".to_string()]),
            span: Span::new(2, 5, 0),
        }];
        let json = generate_diagnostic_json("", &[], &[], &errors, &[]);
        assert!(json.contains("condition must be bool"));
        assert!(json.contains("\"error_count\": 1"));
    }
}
