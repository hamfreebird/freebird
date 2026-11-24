pub mod checker;
pub mod error;
pub mod type_stack;
pub mod type_table;

pub use checker::{
    MethodResolution, MethodTagMap, MethodVariantTags, TypeCheckError, TypeCheckWarning,
    TypeChecker, check_program, check_program_with_tags, check_program_with_warnings,
};
pub use type_stack::TypeStack;
pub use type_table::TypeTable;

use crate::frontend::parse;
use crate::hir::lower;
use crate::hir::types::{HirDecl, HirStmt};

/// 在给定的顶层声明上下文中推断表达式类型（REPL `:type` / LSP hover 复用）。
///
/// 表达式被包装进一个探针函数的 `let` 初始化中，因此只接受表达式，
/// 不接受语句或多条表达式。返回类型的可读名称（如 `i64`、`Vec<i64>`）。
pub fn infer_expression_type(declarations: &str, expression: &str) -> Result<String, Vec<String>> {
    let source = format!(
        "{declarations}\nfn _fb_type_probe() {{\n    let _fb_probe_value = {expression};\n}}\n"
    );
    let (ast, parse_errors) = parse(&source);
    if !parse_errors.is_empty() {
        return Err(parse_errors.iter().map(|e| e.to_string()).collect());
    }
    let (hir, resolve_errors) = lower(&ast);
    if !resolve_errors.is_empty() {
        return Err(resolve_errors.iter().map(|e| e.to_string()).collect());
    }
    let type_table = TypeTable::from_ast_decls(&ast.decls);

    let probe = hir
        .decls
        .iter()
        .find_map(|decl| match decl {
            HirDecl::Fn(f) if f.name == "_fb_type_probe" => Some(f),
            _ => None,
        })
        .ok_or_else(|| vec!["internal error: type probe function was not lowered".to_string()])?;
    let expr = probe
        .body
        .as_ref()
        .and_then(|body| body.stmts.first())
        .and_then(|stmt| match stmt {
            HirStmt::Let {
                init: Some(init), ..
            } => Some(init),
            _ => None,
        })
        .ok_or_else(|| vec!["internal error: type probe has no initializer".to_string()])?;

    let mut checker = TypeChecker::new(&hir, &type_table);
    let (ty, errors) = checker.infer_standalone_expr(expr);
    if !errors.is_empty() {
        return Err(errors.iter().map(|e| e.to_string()).collect());
    }
    Ok(ty.name())
}
