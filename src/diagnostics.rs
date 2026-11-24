//! JSON 诊断输出 — 用于 freebird check 命令
//!
//! 生成结构化的诊断信息，包含源码位置、错误级别、修复建议。

use serde::Serialize;

use crate::borrowck::checker::BorrowError;
use crate::frontend::error::{FrontendError, FrontendErrorKind};
use crate::frontend::token::Span;
use crate::hir::resolver::ResolveError;
use crate::typeck::{TypeCheckError, TypeCheckWarning};

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
    /// 稳定错误码（P/R/T/B/W + 三位数字）。
    pub code: &'static str,
    pub message: String,
    pub span: SourceSpan,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedLocation>,
    /// 多文件项目中诊断所在文件（相对清单根；单文件为 None）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// 内部文件 ID（不序列化）。
    #[serde(skip)]
    pub file_id: u32,
}

/// 相关位置（例如重复定义第一次出现的位置）。
#[derive(Debug, Clone, Serialize)]
pub struct RelatedLocation {
    pub message: String,
    pub span: SourceSpan,
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

/// 前端/词法/语法错误码。
fn frontend_code(kind: &FrontendErrorKind) -> &'static str {
    match kind {
        FrontendErrorKind::UnterminatedString => "P001",
        FrontendErrorKind::UnterminatedChar => "P002",
        FrontendErrorKind::InvalidCharEscape(_) => "P003",
        FrontendErrorKind::InvalidEscapeSequence(_) => "P004",
        FrontendErrorKind::UnexpectedChar(_) => "P005",
        FrontendErrorKind::InvalidNumber(_) => "P006",
        FrontendErrorKind::UnexpectedToken { .. } => "P007",
        FrontendErrorKind::UnexpectedEof { .. } => "P008",
        FrontendErrorKind::DuplicateField { .. } => "P009",
        FrontendErrorKind::UnknownAttribute(_) => "P010",
        FrontendErrorKind::MissingAttributeArg(_) => "P011",
        FrontendErrorKind::InvalidAttributeValue { .. } => "P012",
        FrontendErrorKind::InvalidRetOperand { .. } => "P013",
        FrontendErrorKind::UnknownStardustInstruction(_) => "P014",
        FrontendErrorKind::MixedCallArgs(_) => "P015",
    }
}

/// 名称解析错误码。
fn resolve_code(error: &ResolveError) -> &'static str {
    match error {
        ResolveError::UndefinedVariable { .. } => "R001",
        ResolveError::ForwardReference { .. } => "R002",
        ResolveError::DuplicateDefinition { .. } => "R003",
        ResolveError::DuplicateField { .. } => "R004",
        ResolveError::ArgumentCountMismatch { .. } => "R005",
        ResolveError::CannotAssignToImmutable { .. } => "R006",
        ResolveError::InvalidAssignmentTarget { .. } => "R007",
    }
}

/// 类型检查错误码。
fn type_code(error: &TypeCheckError) -> &'static str {
    match error {
        TypeCheckError::TypeMismatch { .. } => "T001",
        TypeCheckError::UnknownType { .. } => "T002",
        TypeCheckError::ConditionNotBool { .. } => "T003",
        TypeCheckError::NotCallable { .. } => "T004",
        TypeCheckError::UnknownFunction { .. } => "T005",
        TypeCheckError::UnknownField { .. } => "T006",
        TypeCheckError::UnknownVariant { .. } => "T007",
        TypeCheckError::UnknownStruct { .. } => "T008",
        TypeCheckError::ArgumentCountMismatch { .. } => "T009",
        TypeCheckError::ReturnArityMismatch { .. } => "T010",
        TypeCheckError::LoopControlOutsideLoop { .. } => "T011",
        TypeCheckError::UnknownLoopLabel { .. } => "T012",
        TypeCheckError::BreakWithValue { .. } => "T013",
        TypeCheckError::LoopUsedAsValue { .. } => "T014",
        TypeCheckError::NonExhaustiveMatch { .. } => "T015",
        TypeCheckError::MissingFunctionBody { .. } => "T016",
        TypeCheckError::ExternCallOutsideUnsafe { .. } => "T017",
        TypeCheckError::UnsupportedFeature { .. } => "T018",
        TypeCheckError::UnknownAttribute { .. } => "T019",
        TypeCheckError::InvalidAttribute { .. } => "T020",
        TypeCheckError::RefutableLetPattern { .. } => "T021",
        TypeCheckError::OrPatternBindingMismatch { .. } => "T022",
        TypeCheckError::GenericArgMismatch { .. } => "T023",
        TypeCheckError::UnsupportedFormatArgument { .. } => "T024",
        TypeCheckError::FormatTemplateMismatch { .. } => "T025",
    }
}

/// 借用检查错误码。
fn borrow_code(error: &BorrowError) -> &'static str {
    match error {
        BorrowError::UseAfterMove { .. } => "B001",
        BorrowError::UseWhileBorrowedMut { .. } => "B002",
        BorrowError::BorrowWhileBorrowedMut { .. } => "B003",
        BorrowError::MutBorrowWhileBorrowed { .. } => "B004",
        BorrowError::MoveWhileBorrowed { .. } => "B005",
        BorrowError::CannotAssignToImmutable { .. } => "B006",
    }
}

/// 将前端错误转换为诊断消息
pub fn frontend_to_diagnostics(errors: &[FrontendError]) -> Vec<Diagnostic> {
    errors
        .iter()
        .map(|e| {
            let related = match &e.kind {
                FrontendErrorKind::DuplicateField { name, first } => vec![RelatedLocation {
                    message: format!("`{name}` first defined here"),
                    span: SourceSpan::from_span(*first),
                }],
                _ => Vec::new(),
            };
            let suggestion = match &e.kind {
                FrontendErrorKind::InvalidCharEscape(_)
                | FrontendErrorKind::InvalidEscapeSequence(_) => Some(
                    "use a supported escape sequence (`\\n`, `\\t`, `\\\\`, `\\'`, `\\\"`)"
                        .to_string(),
                ),
                FrontendErrorKind::UnknownAttribute(_) => {
                    Some("remove the attribute or use a documented attribute".to_string())
                }
                FrontendErrorKind::MissingAttributeArg(_) => {
                    Some("add the missing attribute argument".to_string())
                }
                FrontendErrorKind::InvalidAttributeValue { .. } => {
                    Some("use a value documented for this attribute".to_string())
                }
                FrontendErrorKind::UnknownStardustInstruction(_) => {
                    Some("see readme.md §10.3 for the supported instruction list".to_string())
                }
                FrontendErrorKind::InvalidRetOperand { .. } => Some(
                    "`ret` expects an integer literal; use `return` for expressions".to_string(),
                ),
                FrontendErrorKind::MixedCallArgs(_) => {
                    Some("use either all positional or all named arguments".to_string())
                }
                _ => None,
            };
            Diagnostic {
                level: DiagnosticLevel::Error,
                code: frontend_code(&e.kind),
                file_id: e.span.file_id,
                file: None,
                message: format!("{e}"),
                span: SourceSpan::from_span(e.span),
                notes: vec![],
                suggestion,
                related,
            }
        })
        .collect()
}

/// 将名称解析错误转换为诊断消息
pub fn resolve_to_diagnostics(errors: &[ResolveError]) -> Vec<Diagnostic> {
    errors
        .iter()
        .map(|e| {
            let (span, suggestion) = match e {
                ResolveError::UndefinedVariable { span, .. } => (
                    *span,
                    Some("did you forget to declare this variable?".to_string()),
                ),
                ResolveError::ForwardReference { span, .. } => (
                    *span,
                    Some("variables must be initialized before use".to_string()),
                ),
                ResolveError::DuplicateDefinition { second, .. } => (
                    *second,
                    Some("rename or remove the duplicate definition".to_string()),
                ),
                ResolveError::DuplicateField { second, .. } => (
                    *second,
                    Some("rename or remove the duplicate field".to_string()),
                ),
                ResolveError::ArgumentCountMismatch {
                    span,
                    expected,
                    got,
                    ..
                } => (
                    *span,
                    Some(format!("expected {expected} arguments, got {got}")),
                ),
                ResolveError::CannotAssignToImmutable { span, .. } => (
                    *span,
                    Some("declare the variable with `let mut` to make it mutable".to_string()),
                ),
                ResolveError::InvalidAssignmentTarget { span } => (
                    *span,
                    Some(
                        "assignment targets must be a variable, `*ref`, field, or index"
                            .to_string(),
                    ),
                ),
            };
            let related = match e {
                ResolveError::DuplicateDefinition { name, first, .. } => vec![RelatedLocation {
                    message: format!("`{name}` first defined here"),
                    span: SourceSpan::from_span(*first),
                }],
                ResolveError::DuplicateField { name, first, .. } => vec![RelatedLocation {
                    message: format!("field `{name}` first defined here"),
                    span: SourceSpan::from_span(*first),
                }],
                _ => Vec::new(),
            };
            Diagnostic {
                level: DiagnosticLevel::Error,
                code: resolve_code(e),
                file_id: span.file_id,
                file: None,
                message: format!("{e}"),
                span: SourceSpan::from_span(span),
                notes: vec![],
                suggestion,
                related,
            }
        })
        .collect()
}

/// 将类型检查错误转换为诊断消息
pub fn type_to_diagnostics(errors: &[TypeCheckError]) -> Vec<Diagnostic> {
    errors
        .iter()
        .map(|e| {
            let suggestion = match e {
                TypeCheckError::TypeMismatch { .. } => {
                    Some("change the value or the expected type annotation".to_string())
                }
                TypeCheckError::ConditionNotBool { .. } => Some(
                    "conditions must be `bool`; use a comparison or a bool variable".to_string(),
                ),
                TypeCheckError::UnknownType { name, .. } => {
                    Some(format!("declare `{name}` or fix the spelling"))
                }
                TypeCheckError::UnknownFunction { name, .. } => {
                    Some(format!("declare function `{name}` or fix the spelling"))
                }
                TypeCheckError::UnknownLoopLabel { label, .. } => Some(format!(
                    "declare the loop label `'{label}` or fix the spelling"
                )),
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
                TypeCheckError::BreakWithValue { .. } => Some(
                    "use `break` without a value until loop-value semantics are implemented"
                        .to_string(),
                ),
                TypeCheckError::LoopUsedAsValue { .. } => Some(
                    "use the loop as a statement, or use `return` to leave the function"
                        .to_string(),
                ),
                TypeCheckError::NonExhaustiveMatch { .. } => {
                    Some("add the missing variants or a `_` arm".to_string())
                }
                TypeCheckError::MissingFunctionBody { .. } => Some(
                    "add `{ ... }`, or declare the function inside an `extern` block".to_string(),
                ),
                TypeCheckError::ExternCallOutsideUnsafe { .. } => {
                    Some("wrap the extern call in an `unsafe` block".to_string())
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
                TypeCheckError::GenericArgMismatch { .. } => {
                    Some("pass the number of type arguments declared by the function".to_string())
                }
                TypeCheckError::UnsupportedFormatArgument { .. } => Some(
                    "`format` accepts integers, `bool`, `char` and `String` arguments".to_string(),
                ),
                TypeCheckError::FormatTemplateMismatch { .. } => Some(
                    "make the number of `{}` placeholders match the arguments, and escape literal braces as `{{`/`}}`"
                        .to_string(),
                ),
            };
            Diagnostic {
                level: DiagnosticLevel::Error,
                code: type_code(e),
                file_id: e.span().file_id,
                file: None,
                message: format!("{e}"),
                span: SourceSpan::from_span(e.span()),
                notes: vec![],
                suggestion,
                related: Vec::new(),
            }
        })
        .collect()
}

/// 将借用检查错误转换为诊断消息
pub fn borrow_to_diagnostics(errors: &[BorrowError]) -> Vec<Diagnostic> {
    errors
        .iter()
        .map(|e| {
            let (span, suggestion) = match e {
                BorrowError::UseAfterMove { span, .. } => (
                    *span,
                    Some("the value was moved and can no longer be used".to_string()),
                ),
                BorrowError::UseWhileBorrowedMut { span, .. } => (
                    *span,
                    Some(
                        "wait for the mutable borrow to end before using this variable".to_string(),
                    ),
                ),
                BorrowError::BorrowWhileBorrowedMut { span, .. } => (
                    *span,
                    Some("cannot borrow while a mutable borrow is active".to_string()),
                ),
                BorrowError::MutBorrowWhileBorrowed { span, .. } => (
                    *span,
                    Some("release existing borrows before taking a mutable borrow".to_string()),
                ),
                BorrowError::MoveWhileBorrowed { span, .. } => {
                    (*span, Some("cannot move a borrowed value".to_string()))
                }
                BorrowError::CannotAssignToImmutable { span, .. } => (
                    *span,
                    Some("declare the variable with `let mut` to make it mutable".to_string()),
                ),
            };
            Diagnostic {
                level: DiagnosticLevel::Error,
                code: borrow_code(e),
                file_id: span.file_id,
                file: None,
                message: format!("{e}"),
                span: SourceSpan::from_span(span),
                notes: vec![],
                suggestion,
                related: Vec::new(),
            }
        })
        .collect()
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
        schema_version: 1,
        source: source.to_string(),
        errors: diagnostics,
        error_count,
        warning_count,
    };

    serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{{}}".to_string())
}

pub fn type_warnings_to_diagnostics(warnings: &[TypeCheckWarning]) -> Vec<Diagnostic> {
    warnings
        .iter()
        .map(|warning| {
            let span = match warning {
                TypeCheckWarning::UnreachablePattern { span } => *span,
            };
            Diagnostic {
                level: DiagnosticLevel::Warning,
                code: "W001",
                file_id: span.file_id,
                file: None,
                message: warning.to_string(),
                span: SourceSpan::from_span(span),
                notes: Vec::new(),
                suggestion: None,
                related: Vec::new(),
            }
        })
        .collect()
}

#[derive(Serialize)]
struct DiagnosticReport {
    /// 诊断 JSON 的 schema 版本；字段只增不改，破坏性变更时递增。
    schema_version: u32,
    source: String,
    errors: Vec<Diagnostic>,
    error_count: usize,
    warning_count: usize,
}

/// 收集源码的全部诊断（parse → resolve → type → borrow），供 LSP 复用。
///
/// 与 `freebird check` 使用同一套检查顺序：解析/名称解析失败时不再做
/// 类型与借用检查，避免级联误报。
pub fn collect_diagnostics(source: &str) -> Vec<Diagnostic> {
    let (ast, parse_errors) = crate::frontend::parse(source);
    let (hir, resolve_errors) = crate::hir::lower(&ast);

    let mut diagnostics = frontend_to_diagnostics(&parse_errors);
    diagnostics.extend(resolve_to_diagnostics(&resolve_errors));

    if parse_errors.is_empty() && resolve_errors.is_empty() {
        let type_table = crate::typeck::TypeTable::from_ast_decls(&ast.decls);
        let (type_errors, warnings) = crate::typeck::check_program_with_warnings(&hir, &type_table);
        diagnostics.extend(type_to_diagnostics(&type_errors));
        diagnostics.extend(type_warnings_to_diagnostics(&warnings));
        if type_errors.is_empty() {
            diagnostics.extend(borrow_to_diagnostics(&crate::borrowck::check_borrow(&hir)));
        }
    }

    diagnostics
}

/// 对已解析的项目 AST 收集诊断（名称解析 → 类型 → 借用）。
///
/// 多文件项目的行号来自各自文件；JSON 的 `source` 使用入口文件内容，
/// 因此跨文件诊断的位置需要结合文件名理解。
pub fn collect_diagnostics_for_program(ast: &crate::frontend::ast::Program) -> Vec<Diagnostic> {
    let (hir, resolve_errors) = crate::hir::lower(ast);
    let type_table = crate::typeck::TypeTable::from_ast_decls(&ast.decls);

    let mut diagnostics = resolve_to_diagnostics(&resolve_errors);
    if resolve_errors.is_empty() {
        let (type_errors, warnings) = crate::typeck::check_program_with_warnings(&hir, &type_table);
        diagnostics.extend(type_to_diagnostics(&type_errors));
        diagnostics.extend(type_warnings_to_diagnostics(&warnings));
        if type_errors.is_empty() {
            diagnostics.extend(borrow_to_diagnostics(&crate::borrowck::check_borrow(&hir)));
        }
    }
    diagnostics
}

/// 把诊断数组序列化为 `check --json` 的完整报告。
///
/// 返回 `(json, error_count, warning_count)`。
pub fn diagnostics_json(diagnostics: Vec<Diagnostic>, source: &str) -> (String, usize, usize) {
    let error_count = diagnostics
        .iter()
        .filter(|diagnostic| matches!(diagnostic.level, DiagnosticLevel::Error))
        .count();
    let warning_count = diagnostics
        .iter()
        .filter(|diagnostic| matches!(diagnostic.level, DiagnosticLevel::Warning))
        .count();
    let report = DiagnosticReport {
        schema_version: 1,
        source: source.to_string(),
        errors: diagnostics,
        error_count,
        warning_count,
    };
    let json = serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string());
    (json, error_count, warning_count)
}

/// 把诊断的 file_id 映射为文件路径（相对清单根）。
pub fn attach_source_map(
    diagnostics: &mut [Diagnostic],
    source_map: &crate::frontend::SourceMap,
    root: &std::path::Path,
) {
    for diagnostic in diagnostics {
        if let Some(path) = source_map.path(diagnostic.file_id) {
            diagnostic.file = Some(relative_path(path, root));
        }
    }
}

fn relative_path(path: &std::path::Path, base: &std::path::Path) -> String {
    use std::path::PathBuf;
    let path_components: Vec<_> = path.components().collect();
    let base_components: Vec<_> = base.components().collect();
    let common = path_components
        .iter()
        .zip(base_components.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut result = PathBuf::new();
    for _ in common..base_components.len() {
        result.push("..");
    }
    for component in &path_components[common..] {
        result.push(component.as_os_str());
    }
    result.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::token::Span;

    #[test]
    fn test_empty_diagnostics() {
        let json = generate_diagnostic_json("", &[], &[], &[], &[]);
        assert!(json.contains("\"error_count\": 0"));
        assert!(json.contains("\"schema_version\": 1"));
    }

    #[test]
    fn test_parse_error_diagnostic() {
        let errors = vec![FrontendError::new(
            crate::frontend::error::FrontendErrorKind::UnexpectedChar('@'),
            Span::new(1, 1, 0),
        )];
        let json = generate_diagnostic_json("@", &errors, &[], &[], &[]);
        assert!(json.contains("unexpected character"));
        assert!(json.contains("\"code\": \"P005\""));
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
        assert!(json.contains("\"code\": \"T003\""));
    }
}
