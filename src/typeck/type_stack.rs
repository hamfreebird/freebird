//! 虚拟类型栈 — 编译器维护的每个程序点的类型信息
//!
//! 用于编译期验证栈操作的局部类型一致性。

use crate::frontend::token::Span;
use crate::hir::types::HirType;

/// 类型栈错误
#[derive(Debug, Clone)]
pub enum TypeError {
    StackDepthMismatch {
        left: usize,
        right: usize,
        span: Span,
    },
    TypeMismatch {
        expected: HirType,
        found: HirType,
        span: Span,
    },
    UnexpectedEmptyStack {
        span: Span,
    },
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeError::StackDepthMismatch { left, right, span } => {
                write!(
                    f,
                    "{span}: stack depth mismatch: {left} vs {right} at control flow merge"
                )
            }
            TypeError::TypeMismatch {
                expected,
                found,
                span,
            } => {
                write!(
                    f,
                    "{span}: type mismatch: expected {}, found {}",
                    expected.name(),
                    found.name()
                )
            }
            TypeError::UnexpectedEmptyStack { span } => {
                write!(f, "{span}: unexpected empty stack")
            }
        }
    }
}

/// 虚拟类型栈
#[derive(Debug, Clone)]
pub struct TypeStack {
    types: Vec<HirType>,
}

impl TypeStack {
    pub fn new() -> Self {
        Self { types: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// 压入类型
    pub fn push(&mut self, ty: HirType) {
        self.types.push(ty);
    }

    /// 弹出类型
    pub fn pop(&mut self) -> Option<HirType> {
        self.types.pop()
    }

    /// 查看栈顶类型
    pub fn peek(&self) -> Option<&HirType> {
        self.types.last()
    }

    /// 查看栈顶向下第 n 个类型（0=栈顶）
    pub fn peek_n(&self, n: usize) -> Option<&HirType> {
        if n >= self.types.len() {
            return None;
        }
        Some(&self.types[self.types.len() - 1 - n])
    }

    /// 复制栈顶类型
    pub fn dup(&mut self) {
        if let Some(top) = self.types.last().cloned() {
            self.types.push(top);
        }
    }

    /// roll(n) 更新类型栈
    pub fn roll(&mut self, n: i32) {
        if n == 0 {
            return;
        }
        let len = self.types.len();
        if n > 0 {
            let pos = n as usize;
            if pos < len {
                let idx = len - 1 - pos;
                let item = self.types.remove(idx);
                self.types.push(item);
            }
        } else {
            let pos = (-n) as usize;
            if pos < len {
                let item = self.types.pop().unwrap();
                let insert_idx = self.types.len() - pos;
                self.types.insert(insert_idx, item);
            }
        }
    }

    /// 交换栈顶两个类型
    pub fn swap(&mut self) {
        let len = self.types.len();
        if len >= 2 {
            self.types.swap(len - 1, len - 2);
        }
    }

    /// 检查两条类型栈在控制流合并点是否兼容
    ///
    /// `TypeError` 携带完整 span 与两侧类型，Box 化会改变所有调用方签名；
    /// 这里是内部错误路径，保持既有 API 并显式豁免 clippy 的体积告警。
    #[allow(clippy::result_large_err)]
    pub fn merge(&self, other: &TypeStack) -> Result<TypeStack, TypeError> {
        let default_span = Span::new(0, 0, 0);
        if self.len() != other.len() {
            return Err(TypeError::StackDepthMismatch {
                left: self.len(),
                right: other.len(),
                span: default_span,
            });
        }
        for (t1, t2) in self.types.iter().zip(other.types.iter()) {
            if t1 != t2 {
                return Err(TypeError::TypeMismatch {
                    expected: t1.clone(),
                    found: t2.clone(),
                    span: default_span,
                });
            }
        }
        Ok(self.clone())
    }
}

impl Default for TypeStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_pop() {
        let mut ts = TypeStack::new();
        ts.push(HirType::Path(vec!["i32".to_string()]));
        assert_eq!(ts.len(), 1);
        assert_eq!(ts.pop().unwrap().name(), "i32");
        assert!(ts.is_empty());
    }

    #[test]
    fn test_merge_same() {
        let mut a = TypeStack::new();
        a.push(HirType::Path(vec!["i32".to_string()]));
        let b = a.clone();
        assert!(a.merge(&b).is_ok());
    }

    #[test]
    fn test_merge_depth_mismatch() {
        let mut a = TypeStack::new();
        a.push(HirType::Path(vec!["i32".to_string()]));
        let b = TypeStack::new();
        assert!(a.merge(&b).is_err());
    }

    #[test]
    fn test_roll() {
        let mut ts = TypeStack::new();
        ts.push(HirType::Path(vec!["i32".to_string()]));
        ts.push(HirType::Path(vec!["bool".to_string()]));
        ts.push(HirType::Path(vec!["i64".to_string()]));
        // Stack (bottom→top): [i32, bool, i64]
        ts.roll(2); // bring i32 to top
        assert_eq!(ts.peek().unwrap().name(), "i32");
        ts.roll(-2); // put i32 back
        assert_eq!(ts.peek().unwrap().name(), "i64");
    }
}
