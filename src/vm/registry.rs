use std::collections::HashMap;

use crate::vm::id::{ElementId, IdAllocator, StackId};
use crate::vm::stack::DualStack;

/// 全局数据栈注册表 — 管理远程栈（Vec/String/Box 的数据栈）
#[derive(Debug, Clone)]
pub struct GlobalRegistry {
    /// StackId → 数据栈（仅存储远程栈；函数调用帧在 CallStack 中管理）
    remote_stacks: HashMap<StackId, DualStack>,
}

impl GlobalRegistry {
    pub fn new() -> Self {
        Self {
            remote_stacks: HashMap::new(),
        }
    }

    /// 分配新的远程栈，返回 StackId
    pub fn alloc_remote(
        &mut self,
        alloc: &mut IdAllocator,
        _call_id: ElementId,
    ) -> StackId {
        let sid = alloc.alloc_stack();
        self.remote_stacks.insert(sid, DualStack::new());
        sid
    }

    /// 获取远程栈的只读引用
    pub fn get(&self, sid: StackId) -> Option<&DualStack> {
        self.remote_stacks.get(&sid)
    }

    /// 获取远程栈的可变引用
    pub fn get_mut(&mut self, sid: StackId) -> Option<&mut DualStack> {
        self.remote_stacks.get_mut(&sid)
    }

    /// 释放远程栈，返回其数据栈
    pub fn free(&mut self, sid: StackId) -> Option<DualStack> {
        self.remote_stacks.remove(&sid)
    }

    /// 检查远程栈是否存在
    pub fn contains(&self, sid: StackId) -> bool {
        self.remote_stacks.contains_key(&sid)
    }

    /// 获取远程栈数量
    pub fn len(&self) -> usize {
        self.remote_stacks.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.remote_stacks.is_empty()
    }
}

impl Default for GlobalRegistry {
    fn default() -> Self {
        Self::new()
    }
}
