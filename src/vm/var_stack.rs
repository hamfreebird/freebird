use std::collections::HashMap;

use crate::vm::error::VmError;
use crate::vm::id::ElementId;

/// 变量栈 — 栈式作用域管理
/// 进入作用域 push 空帧，退出 pop 帧
#[derive(Debug, Clone)]
pub struct VarStack {
    frames: Vec<HashMap<String, ElementId>>,
}

impl Default for VarStack {
    fn default() -> Self {
        Self::new()
    }
}

impl VarStack {
    pub fn new() -> Self {
        Self { frames: Vec::new() }
    }

    /// 压入新的空作用域帧
    pub fn enter_scope(&mut self) {
        self.frames.push(HashMap::new());
    }

    /// 弹出当前作用域帧，返回帧中所有绑定（用于 Drop 处理）
    pub fn exit_scope(&mut self) -> HashMap<String, ElementId> {
        self.frames.pop().unwrap_or_default()
    }

    /// 获取当前作用域深度
    pub fn scope_depth(&self) -> usize {
        self.frames.len()
    }

    /// 在当前（最内层）作用域中插入绑定
    pub fn insert(&mut self, name: String, id: ElementId) {
        if let Some(frame) = self.frames.last_mut() {
            frame.insert(name, id);
        }
    }

    /// 从内向外查找变量名，返回 ElementId
    pub fn lookup(&self, name: &str) -> Option<ElementId> {
        self.frames.iter().rev().find_map(|f| f.get(name)).copied()
    }

    /// 查找变量名并返回距当前作用域的深度（0=当前，1=外层...）
    pub fn lookup_with_depth(&self, name: &str) -> Option<(ElementId, usize)> {
        let _depth = self.frames.len();
        for (i, frame) in self.frames.iter().rev().enumerate() {
            if let Some(&id) = frame.get(name) {
                return Some((id, i));
            }
        }
        None
    }

    /// 更新已有绑定（赋值时用），从内向外查找
    pub fn update(&mut self, name: &str, new_id: ElementId) -> Result<(), VmError> {
        for frame in self.frames.iter_mut().rev() {
            if let Some(old) = frame.get_mut(name) {
                *old = new_id;
                return Ok(());
            }
        }
        Err(VmError::UndefinedVariable(name.to_string()))
    }

    /// 检查变量是否可变（在当前实现中，所有已绑定的变量都可更新）
    pub fn is_bound(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    /// 获取当前作用域中的所有绑定（用于调试）
    pub fn current_frame_bindings(&self) -> Vec<(String, ElementId)> {
        self.frames
            .last()
            .map(|f| f.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enter_exit_scope() {
        let mut vs = VarStack::new();
        vs.enter_scope();
        vs.insert("x".to_string(), ElementId(1));
        assert_eq!(vs.lookup("x"), Some(ElementId(1)));
        vs.exit_scope();
        assert_eq!(vs.lookup("x"), None);
    }

    #[test]
    fn test_shadowing() {
        let mut vs = VarStack::new();
        vs.enter_scope();
        vs.insert("x".to_string(), ElementId(1));
        vs.enter_scope();
        vs.insert("x".to_string(), ElementId(2));
        // 内层遮蔽外层
        assert_eq!(vs.lookup("x"), Some(ElementId(2)));
        let (id, depth) = vs.lookup_with_depth("x").unwrap();
        assert_eq!(id, ElementId(2));
        assert_eq!(depth, 0);
        vs.exit_scope();
        // 恢复外层
        assert_eq!(vs.lookup("x"), Some(ElementId(1)));
    }

    #[test]
    fn test_update() {
        let mut vs = VarStack::new();
        vs.enter_scope();
        vs.insert("x".to_string(), ElementId(1));
        vs.update("x", ElementId(10)).unwrap();
        assert_eq!(vs.lookup("x"), Some(ElementId(10)));
    }

    #[test]
    fn test_update_undefined() {
        let mut vs = VarStack::new();
        vs.enter_scope();
        assert!(vs.update("y", ElementId(1)).is_err());
    }

    #[test]
    fn test_update_inner_shadow() {
        let mut vs = VarStack::new();
        vs.enter_scope();
        vs.insert("x".to_string(), ElementId(1));
        vs.enter_scope();
        vs.insert("x".to_string(), ElementId(2));
        vs.update("x", ElementId(20)).unwrap();
        assert_eq!(vs.lookup("x"), Some(ElementId(20)));
        vs.exit_scope();
        // 外层的 x 未被影响
        assert_eq!(vs.lookup("x"), Some(ElementId(1)));
    }
}
