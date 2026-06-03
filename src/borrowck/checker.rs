use std::collections::{HashMap, HashSet};

use crate::frontend::token::Span;

/// 借用状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BorrowState {
    /// 未借用，可自由使用
    Unused,
    /// 共享借用（计数）
    Shared(usize),
    /// 独占借用（可变引用）
    Exclusive,
    /// 已移动（不可再使用）
    Moved,
}

/// 借用检查错误
#[derive(Debug, Clone)]
pub enum BorrowError {
    UseAfterMove { name: String, span: Span },
    UseWhileBorrowedMut { name: String, span: Span },
    BorrowWhileBorrowedMut { name: String, span: Span },
    MutBorrowWhileBorrowed { name: String, span: Span },
    MoveWhileBorrowed { name: String, span: Span },
    CannotAssignToImmutable { name: String, span: Span },
}

impl std::fmt::Display for BorrowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BorrowError::UseAfterMove { name, span } => {
                write!(f, "{span}: use of moved value `{name}`")
            }
            BorrowError::UseWhileBorrowedMut { name, span } => {
                write!(f, "{span}: cannot use `{name}` while it is mutably borrowed")
            }
            BorrowError::BorrowWhileBorrowedMut { name, span } => {
                write!(f, "{span}: cannot borrow `{name}` while it is mutably borrowed")
            }
            BorrowError::MutBorrowWhileBorrowed { name, span } => {
                write!(f, "{span}: cannot mutably borrow `{name}` while it is borrowed")
            }
            BorrowError::MoveWhileBorrowed { name, span } => {
                write!(f, "{span}: cannot move `{name}` while it is borrowed")
            }
            BorrowError::CannotAssignToImmutable { name, span } => {
                write!(f, "{span}: cannot assign to immutable variable `{name}`")
            }
        }
    }
}

/// 变量信息
#[derive(Debug, Clone)]
struct VarInfo {
    /// 是否可变
    mutable: bool,
    /// 是否为 Move 类型（Vec, String, Box）
    is_move_type: bool,
    /// 当前借用状态
    borrow_state: BorrowState,
    /// 声明位置所在的 scope 深度
    scope_depth: usize,
}

/// 借用检查器 — 在 HIR 层面验证所有权和借用规则
pub struct BorrowChecker {
    /// 变量名 → 变量信息
    vars: HashMap<String, VarInfo>,
    /// 当前作用域深度
    scope_depth: usize,
    /// 作用域帧栈（每帧记录该作用域声明的变量名）
    scope_vars: Vec<Vec<String>>,
    /// 错误列表
    pub errors: Vec<BorrowError>,
    /// 已知的 Move 类型名
    move_types: HashSet<String>,
}

impl BorrowChecker {
    pub fn new() -> Self {
        let mut move_types = HashSet::new();
        move_types.insert("Vec".to_string());
        move_types.insert("String".to_string());
        move_types.insert("Box".to_string());
        move_types.insert("StackHandle".to_string());

        Self {
            vars: HashMap::new(),
            scope_depth: 0,
            scope_vars: Vec::new(),
            errors: Vec::new(),
            move_types,
        }
    }

    /// 进入新作用域
    pub fn enter_scope(&mut self) {
        self.scope_depth += 1;
        self.scope_vars.push(Vec::new());
    }

    /// 退出作用域（释放该作用域内声明的所有变量）
    pub fn exit_scope(&mut self) -> Vec<String> {
        let dropped = self.scope_vars.pop().unwrap_or_default();
        // 释放已移动的变量不报错（它们已经被转移了所有权）
        for name in &dropped {
            if let Some(info) = self.vars.get(name) {
                if info.is_move_type && info.borrow_state != BorrowState::Moved {
                    // Move 类型在此处应该被析构（Drop），但目前静默处理
                }
                self.vars.remove(name);
            }
        }
        self.scope_depth = self.scope_depth.saturating_sub(1);
        dropped
    }

    /// 获取待析构变量的 Drop 顺序（逆序创建）
    pub fn drop_order(&self) -> Vec<String> {
        let mut order: Vec<String> = self.vars.iter()
            .filter(|(_, info)| info.is_move_type && info.borrow_state != BorrowState::Moved)
            .map(|(name, _)| name.clone())
            .collect();
        // 逆序：后创建的先析构
        order.reverse();
        order
    }

    /// 声明新变量
    pub fn declare_var(
        &mut self,
        name: &str,
        mutable: bool,
        is_move_type: bool,
        span: Span,
    ) -> Result<(), BorrowError> {
        if let Some(existing) = self.vars.get(name)
            && existing.scope_depth == self.scope_depth {
                return Err(BorrowError::CannotAssignToImmutable {
                    name: format!("duplicate variable `{name}`"),
                    span,
                });
            }
        self.vars.insert(name.to_string(), VarInfo {
            mutable,
            is_move_type,
            borrow_state: BorrowState::Unused,
            scope_depth: self.scope_depth,
        });
        if let Some(frame) = self.scope_vars.last_mut() {
            frame.push(name.to_string());
        }
        Ok(())
    }

    /// 检查变量读取（Copy 类型：直接读取；Move 类型：检查是否已移动）
    pub fn check_read(&mut self, name: &str, span: Span) -> Result<(), BorrowError> {
        let info = self.vars.get(name).ok_or_else(|| BorrowError::UseAfterMove {
            name: format!("undefined variable `{name}`"),
            span,
        })?;

        match &info.borrow_state {
            BorrowState::Moved => Err(BorrowError::UseAfterMove {
                name: name.to_string(),
                span,
            }),
            BorrowState::Exclusive => Err(BorrowError::UseWhileBorrowedMut {
                name: name.to_string(),
                span,
            }),
            _ => Ok(()),
        }
    }

    /// 检查变量写入（赋值）
    pub fn check_write(&mut self, name: &str, span: Span) -> Result<(), BorrowError> {
        let info = self.vars.get_mut(name).ok_or_else(|| BorrowError::UseAfterMove {
            name: format!("undefined variable `{name}`"),
            span,
        })?;

        if !info.mutable {
            return Err(BorrowError::CannotAssignToImmutable {
                name: name.to_string(),
                span,
            });
        }

        match &info.borrow_state {
            BorrowState::Moved => Err(BorrowError::UseAfterMove {
                name: name.to_string(),
                span,
            }),
            BorrowState::Exclusive => Err(BorrowError::UseWhileBorrowedMut {
                name: name.to_string(),
                span,
            }),
            BorrowState::Shared(n) if *n > 0 => Err(BorrowError::UseWhileBorrowedMut {
                name: name.to_string(),
                span,
            }),
            _ => Ok(()),
        }
    }

    /// 创建共享引用 &x
    pub fn borrow_shared(&mut self, name: &str, span: Span) -> Result<(), BorrowError> {
        let info = self.vars.get_mut(name).ok_or_else(|| BorrowError::UseAfterMove {
            name: format!("undefined variable `{name}`"),
            span,
        })?;

        match &info.borrow_state {
            BorrowState::Moved => Err(BorrowError::UseAfterMove {
                name: name.to_string(),
                span,
            }),
            BorrowState::Exclusive => Err(BorrowError::BorrowWhileBorrowedMut {
                name: name.to_string(),
                span,
            }),
            BorrowState::Shared(n) => {
                info.borrow_state = BorrowState::Shared(n + 1);
                Ok(())
            }
            BorrowState::Unused => {
                info.borrow_state = BorrowState::Shared(1);
                Ok(())
            }
        }
    }

    /// 创建独占引用 &mut x
    pub fn borrow_mut(&mut self, name: &str, span: Span) -> Result<(), BorrowError> {
        let info = self.vars.get_mut(name).ok_or_else(|| BorrowError::UseAfterMove {
            name: format!("undefined variable `{name}`"),
            span,
        })?;

        match &info.borrow_state {
            BorrowState::Moved => Err(BorrowError::UseAfterMove {
                name: name.to_string(),
                span,
            }),
            BorrowState::Shared(n) if *n > 0 => Err(BorrowError::MutBorrowWhileBorrowed {
                name: name.to_string(),
                span,
            }),
            BorrowState::Exclusive => Err(BorrowError::BorrowWhileBorrowedMut {
                name: name.to_string(),
                span,
            }),
            _ => {
                if !info.mutable {
                    return Err(BorrowError::CannotAssignToImmutable {
                        name: format!("cannot mutably borrow immutable variable `{name}`"),
                        span,
                    });
                }
                info.borrow_state = BorrowState::Exclusive;
                Ok(())
            }
        }
    }

    /// 释放引用（引用离开作用域时）
    pub fn release_ref(&mut self, name: &str) {
        if let Some(info) = self.vars.get_mut(name) {
            match info.borrow_state {
                BorrowState::Shared(n) if n > 1 => {
                    info.borrow_state = BorrowState::Shared(n - 1);
                }
                BorrowState::Shared(_) | BorrowState::Exclusive => {
                    info.borrow_state = BorrowState::Unused;
                }
                _ => {}
            }
        }
    }

    /// 移动变量（赋值或传参时所有权转移）
    pub fn move_var(&mut self, name: &str, span: Span) -> Result<(), BorrowError> {
        let info = self.vars.get(name).ok_or_else(|| BorrowError::UseAfterMove {
            name: format!("undefined variable `{name}`"),
            span,
        })?;

        match &info.borrow_state {
            BorrowState::Shared(n) if *n > 0 => Err(BorrowError::MoveWhileBorrowed {
                name: name.to_string(),
                span,
            }),
            BorrowState::Exclusive => Err(BorrowError::MoveWhileBorrowed {
                name: name.to_string(),
                span,
            }),
            _ => Ok(()),
        }?;

        if let Some(info) = self.vars.get_mut(name) {
            info.borrow_state = BorrowState::Moved;
        }
        Ok(())
    }

    /// 判断变量是否为 Move 类型
    pub fn is_move_type(&self, name: &str) -> bool {
        self.vars.get(name).is_some_and(|v| v.is_move_type)
    }

    /// 查询变量是否可变
    pub fn is_mutable(&self, name: &str) -> bool {
        self.vars.get(name).is_some_and(|v| v.mutable)
    }
}

impl Default for BorrowChecker {
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
    fn test_shared_borrow() {
        let mut bc = BorrowChecker::new();
        bc.enter_scope();
        bc.declare_var("x", false, false, span()).unwrap();
        bc.borrow_shared("x", span()).unwrap();
        // 再次共享借用 OK
        bc.borrow_shared("x", span()).unwrap();
        // 不能可变借用
        assert!(bc.borrow_mut("x", span()).is_err());
    }

    #[test]
    fn test_exclusive_borrow() {
        let mut bc = BorrowChecker::new();
        bc.enter_scope();
        bc.declare_var("x", true, false, span()).unwrap();
        bc.borrow_mut("x", span()).unwrap();
        // 不能再共享借用
        assert!(bc.borrow_shared("x", span()).is_err());
    }

    #[test]
    fn test_move_semantics() {
        let mut bc = BorrowChecker::new();
        bc.enter_scope();
        bc.declare_var("v", false, true, span()).unwrap(); // Move type
        bc.move_var("v", span()).unwrap();
        // 移动后不可再使用
        assert!(bc.check_read("v", span()).is_err());
    }

    #[test]
    fn test_release_ref() {
        let mut bc = BorrowChecker::new();
        bc.enter_scope();
        bc.declare_var("x", true, false, span()).unwrap();
        bc.borrow_mut("x", span()).unwrap();
        bc.release_ref("x");
        // 释放后可重新借用
        assert!(bc.borrow_shared("x", span()).is_ok());
    }

    #[test]
    fn test_cannot_assign_immutable() {
        let mut bc = BorrowChecker::new();
        bc.enter_scope();
        bc.declare_var("x", false, false, span()).unwrap();
        assert!(bc.check_write("x", span()).is_err());
    }
}
