use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// 元素唯一标识符 — 全局唯一，单调递增，永不回收
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct ElementId(pub u64);

/// 栈唯一标识符 — 全局数据栈注册表的键
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct StackId(pub u64);

/// 函数唯一标识符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct FnId(pub u64);

/// ID 分配器 — 线程局部单调递增
#[derive(Debug, Clone)]
pub struct IdAllocator {
    next_element_id: u64,
    next_stack_id: u64,
    next_fn_id: u64,
    /// 递归种子，每进入一次函数调用递增，用于生成 StackId
    recursion_seed: u64,
}

impl IdAllocator {
    pub fn new() -> Self {
        Self {
            next_element_id: 1,
            next_stack_id: 1,
            next_fn_id: 1,
            recursion_seed: 0,
        }
    }

    /// 分配新的 ElementId
    pub fn alloc_element(&mut self) -> ElementId {
        let id = ElementId(self.next_element_id);
        self.next_element_id += 1;
        id
    }

    /// 分配新的 StackId
    pub fn alloc_stack(&mut self) -> StackId {
        let id = StackId(self.next_stack_id);
        self.next_stack_id += 1;
        id
    }

    /// 从 call_id + recursion_seed 派生 StackId（用于函数调用帧）
    pub fn derive_stack(&mut self, call_id: ElementId) -> StackId {
        let mut hasher = DefaultHasher::new();
        call_id.hash(&mut hasher);
        self.recursion_seed.hash(&mut hasher);
        let derived = hasher.finish();
        // 确保不与单调分配的 StackId 碰撞（高位设置标志位）
        let id = derived | 0x8000_0000_0000_0000;
        StackId(id)
    }

    /// 分配新的 FnId
    pub fn alloc_fn(&mut self) -> FnId {
        let id = FnId(self.next_fn_id);
        self.next_fn_id += 1;
        id
    }

    /// 递增递归种子（每次函数调用时调用）
    pub fn bump_recursion(&mut self) {
        self.recursion_seed = self.recursion_seed.wrapping_add(1);
    }

    /// 获取当前递归种子
    pub fn recursion_seed(&self) -> u64 {
        self.recursion_seed
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}
