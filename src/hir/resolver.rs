use std::collections::HashMap;

use crate::frontend::token::Span;
use crate::hir::types::FnSignature;

/// 名称解析错误
#[derive(Debug, Clone)]
pub enum ResolveError {
    UndefinedVariable { name: String, span: Span },
    ForwardReference { name: String, span: Span },
    DuplicateDefinition { name: String, first: Span, second: Span },
    DuplicateField { name: String, first: Span, second: Span },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::UndefinedVariable { name, span } => {
                write!(f, "{span}: undefined variable `{name}`")
            }
            ResolveError::ForwardReference { name, span } => {
                write!(f, "{span}: cannot use variable `{name}` before its declaration")
            }
            ResolveError::DuplicateDefinition { name, first, second } => {
                write!(f, "{second}: duplicate definition of `{name}` (first defined at {first})")
            }
            ResolveError::DuplicateField { name, first, second } => {
                write!(f, "{second}: duplicate field `{name}` (first defined at {first})")
            }
        }
    }
}

/// 作用域帧中的绑定
#[derive(Debug, Clone)]
struct Binding {
    /// 是否可变
    mutable: bool,
    /// 声明位置
    span: Span,
    /// 是否已完成初始化（用于前向引用检测）
    initialized: bool,
}

/// 名称解析器 — 管理作用域帧并解析变量引用
pub struct Resolver {
    /// 作用域帧栈（每帧存储 name → Binding）
    scopes: Vec<HashMap<String, Binding>>,
    /// 收集的函数签名
    pub fn_signatures: Vec<FnSignature>,
    /// 当前作用域中尚未初始化的变量（用于前向引用检测）
    uninitialized: Vec<Vec<String>>,
    /// 解析错误
    pub errors: Vec<ResolveError>,
}

impl Resolver {
    pub fn new() -> Self {
        Self {
            scopes: Vec::new(),
            fn_signatures: Vec::new(),
            uninitialized: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// 进入新作用域
    pub fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.uninitialized.push(Vec::new());
    }

    /// 退出当前作用域
    pub fn exit_scope(&mut self) {
        self.scopes.pop();
        self.uninitialized.pop();
    }

    /// 获取当前作用域深度（最内层=0）
    pub fn scope_depth(&self) -> usize {
        self.scopes.len()
    }

    /// 在当前作用域中声明变量（初始未初始化）
    pub fn declare_var(&mut self, name: &str, mutable: bool, span: Span) -> Result<(), ResolveError> {
        if let Some(current) = self.scopes.last()
            && let Some(existing) = current.get(name) {
                return Err(ResolveError::DuplicateDefinition {
                    name: name.to_string(),
                    first: existing.span,
                    second: span,
                });
            }
        if let Some(current) = self.scopes.last_mut() {
            current.insert(name.to_string(), Binding {
                mutable,
                span,
                initialized: false,
            });
        }
        if let Some(ui) = self.uninitialized.last_mut() {
            ui.push(name.to_string());
        }
        Ok(())
    }

    /// 标记变量已初始化（let 语句求值 RHS 后调用）
    pub fn mark_initialized(&mut self, name: &str) {
        if let Some(ui) = self.uninitialized.last_mut() {
            ui.retain(|n| n != name);
        }
        if let Some(current) = self.scopes.last_mut()
            && let Some(binding) = current.get_mut(name) {
                binding.initialized = true;
            }
    }

    /// 解析变量引用，返回作用域深度
    pub fn resolve_var(&mut self, name: &str, span: Span) -> Result<usize, ResolveError> {
        for (depth, scope) in self.scopes.iter().rev().enumerate() {
            if let Some(binding) = scope.get(name) {
                if !binding.initialized {
                    // 检查是否在当前作用域且未初始化 → 前向引用
                    if depth == 0 && self.uninitialized.last().is_some_and(|ui| ui.contains(&name.to_string())) {
                        return Err(ResolveError::ForwardReference {
                            name: name.to_string(),
                            span,
                        });
                    }
                }
                return Ok(depth);
            }
        }
        Err(ResolveError::UndefinedVariable {
            name: name.to_string(),
            span,
        })
    }

    /// 检查变量是否可变
    pub fn is_mutable(&self, name: &str) -> Option<bool> {
        for scope in self.scopes.iter().rev() {
            if let Some(binding) = scope.get(name) {
                return Some(binding.mutable);
            }
        }
        None
    }

    /// 注册函数签名
    pub fn register_fn(&mut self, sig: FnSignature) {
        self.fn_signatures.push(sig);
    }

    /// 查找函数签名
    pub fn lookup_fn(&self, name: &str) -> Option<&FnSignature> {
        self.fn_signatures.iter().find(|s| s.name == name)
    }

    /// 收集当前作用域中的绑定名（用于调试）
    pub fn current_bindings(&self) -> Vec<String> {
        self.scopes
            .last()
            .map(|s| s.keys().cloned().collect())
            .unwrap_or_default()
    }
}

impl Default for Resolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::token::Span;

    fn span() -> Span {
        Span::new(1, 1, 0)
    }

    #[test]
    fn test_declare_and_resolve() {
        let mut r = Resolver::new();
        r.enter_scope();
        r.declare_var("x", false, span()).unwrap();
        r.mark_initialized("x");
        let depth = r.resolve_var("x", span()).unwrap();
        assert_eq!(depth, 0);
    }

    #[test]
    fn test_shadowing() {
        let mut r = Resolver::new();
        r.enter_scope();
        r.declare_var("x", false, span()).unwrap();
        r.mark_initialized("x");

        r.enter_scope();
        r.declare_var("x", false, span()).unwrap();
        r.mark_initialized("x");

        let depth = r.resolve_var("x", span()).unwrap();
        assert_eq!(depth, 0); // 最内层
        r.exit_scope();

        let depth = r.resolve_var("x", span()).unwrap();
        assert_eq!(depth, 0); // 现在是第一层（唯一的层）
    }

    #[test]
    fn test_undefined_variable() {
        let mut r = Resolver::new();
        r.enter_scope();
        let err = r.resolve_var("y", span()).unwrap_err();
        assert!(matches!(err, ResolveError::UndefinedVariable { .. }));
    }

    #[test]
    fn test_forward_reference() {
        let mut r = Resolver::new();
        r.enter_scope();
        r.declare_var("x", false, span()).unwrap();
        // 未标记初始化 → 使用时应报前向引用错误
        let err = r.resolve_var("x", span()).unwrap_err();
        assert!(matches!(err, ResolveError::ForwardReference { .. }));
    }

    #[test]
    fn test_duplicate_definition() {
        let mut r = Resolver::new();
        r.enter_scope();
        r.declare_var("x", false, span()).unwrap();
        let err = r.declare_var("x", false, span()).unwrap_err();
        assert!(matches!(err, ResolveError::DuplicateDefinition { .. }));
    }
}
