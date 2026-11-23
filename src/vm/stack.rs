use crate::vm::error::VmError;
use crate::vm::id::{ElementId, IdAllocator};
use crate::vm::value::Value;

/// 带标签的双栈：ID 栈 + 值栈，长度始终相等
#[derive(Debug, Clone)]
pub struct DualStack {
    pub(crate) ids: Vec<ElementId>,
    pub(crate) values: Vec<Value>,
}

impl Default for DualStack {
    fn default() -> Self {
        Self::new()
    }
}

impl DualStack {
    pub fn new() -> Self {
        Self {
            ids: Vec::new(),
            values: Vec::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            ids: Vec::with_capacity(capacity),
            values: Vec::with_capacity(capacity),
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Push — 同步压入 ID 和值，若超过 limit 返回 Err
    pub fn push_checked(&mut self, id: ElementId, value: Value, limit: usize) -> Result<(), crate::vm::error::VmError> {
        if self.len() >= limit {
            return Err(crate::vm::error::VmError::StackOverflow { limit });
        }
        self.ids.push(id);
        self.values.push(value);
        Ok(())
    }

    /// Push — 同步压入 ID 和值（不检查限制）
    pub fn push(&mut self, id: ElementId, value: Value) {
        self.ids.push(id);
        self.values.push(value);
    }

    /// Pop — 同步弹出 ID 和值
    pub fn pop(&mut self) -> Option<(ElementId, Value)> {
        match (self.ids.pop(), self.values.pop()) {
            (Some(id), Some(val)) => Some((id, val)),
            _ => None,
        }
    }

    /// Dup — 复制栈顶（生成新 ElementId）
    pub fn dup(&mut self, alloc: &mut IdAllocator) -> Result<(), VmError> {
        if self.is_empty() {
            return Err(VmError::StackUnderflow);
        }
        let new_id = alloc.alloc_element();
        let val = self.values.last().unwrap().clone();
        self.ids.push(new_id);
        self.values.push(val);
        Ok(())
    }

    /// Roll(+n): 将位置 n 的元素提取到栈顶
    /// Roll(-n): 将栈顶元素插入到位置 n
    /// Roll(0): no-op
    pub fn roll(&mut self, n: i32) -> Result<(), VmError> {
        if n == 0 {
            return Ok(());
        }
        let len = self.len();
        if len == 0 {
            return Err(VmError::StackUnderflow);
        }

        if n > 0 {
            let pos = n as usize;
            if pos >= len {
                return Err(VmError::RollOutOfBounds { n, len });
            }
            // 从栈顶向下数第 pos 个元素的索引（在 Vec 中）
            let idx = len - 1 - pos;
            let id = self.ids.remove(idx);
            let val = self.values.remove(idx);
            self.ids.push(id);
            self.values.push(val);
        } else {
            let pos = (-n) as usize;
            if pos >= len {
                return Err(VmError::RollOutOfBounds { n, len });
            }
            let id = self.ids.pop().unwrap();
            let val = self.values.pop().unwrap();
            // 原长度已减 1，所以插入索引为 len - 1 - pos
            let insert_idx = self.ids.len() - pos;
            self.ids.insert(insert_idx, id);
            self.values.insert(insert_idx, val);
        }
        Ok(())
    }

    /// Swap 栈顶两个元素
    pub fn swap(&mut self) -> Result<(), VmError> {
        if self.len() < 2 {
            return Err(VmError::SwapUnderflow);
        }
        let top = self.len() - 1;
        let under = self.len() - 2;
        self.ids.swap(top, under);
        self.values.swap(top, under);
        Ok(())
    }

    /// Reverse：反转整个数据栈（ID 栈与值栈保持同步）。
    pub fn reverse(&mut self) {
        self.ids.reverse();
        self.values.reverse();
    }

    /// Pick(n): 读取 n（栈顶），复制位置 n 的元素到栈顶（新 ElementId）
    /// n 从栈顶向下数，0 = 栈顶自己（等价于 dup）
    pub fn pick(&mut self, alloc: &mut IdAllocator) -> Result<(), VmError> {
        if self.is_empty() {
            return Err(VmError::StackUnderflow);
        }
        // 弹出栈顶的 n 值
        let (_, n_val) = self.pop().unwrap();
        let n = n_val.as_i64().ok_or(VmError::TypeMismatch {
            expected: "integer".to_string(),
            found: n_val.type_name().to_string(),
        })? as usize;

        let len = self.len();
        if n >= len {
            return Err(VmError::PickOutOfBounds { n, len });
        }
        let idx = len - 1 - n;
        let new_id = alloc.alloc_element();
        let val = self.values[idx].clone();
        self.ids.push(new_id);
        self.values.push(val);
        Ok(())
    }

    /// 在 ID 栈中线性查找 ElementId，返回距离栈顶的深度（0 = 栈顶）
    /// 未找到返回 None
    pub fn find_id(&self, id: ElementId) -> Option<usize> {
        self.ids.iter().rev().position(|&x| x == id)
    }

    /// 直接通过索引访问（0 = 栈顶，按 reverse 索引）
    pub fn get_from_top(&self, n: usize) -> Option<(&ElementId, &Value)> {
        if n >= self.len() {
            return None;
        }
        let idx = self.len() - 1 - n;
        Some((&self.ids[idx], &self.values[idx]))
    }

    /// 获取栈顶的 ElementId
    pub fn top_id(&self) -> Option<ElementId> {
        self.ids.last().copied()
    }

    /// 获取栈顶的 Value 引用
    pub fn top_value(&self) -> Option<&Value> {
        self.values.last()
    }

    /// 获取栈顶的 Value 可变引用
    pub fn top_value_mut(&mut self) -> Option<&mut Value> {
        self.values.last_mut()
    }

    /// 获取从栈顶向下第 n 个位置的 Value 引用（0 = 栈顶）
    pub fn peek(&self, n: usize) -> Option<&Value> {
        if n >= self.len() {
            return None;
        }
        Some(&self.values[self.len() - 1 - n])
    }

    /// 直接通过索引修改值（0 = 栈顶）
    pub fn set_from_top(&mut self, n: usize, value: Value) -> Result<(), VmError> {
        if n >= self.len() {
            return Err(VmError::StackUnderflow);
        }
        let idx = self.len() - 1 - n;
        self.values[idx] = value;
        Ok(())
    }

    /// 截断栈（用于 ret 时丢弃局部变量）
    pub fn truncate(&mut self, new_len: usize) {
        self.ids.truncate(new_len);
        self.values.truncate(new_len);
    }

    /// 将另一个栈的元素全部移动到当前栈顶
    pub fn extend_from_stack(&mut self, other: &mut DualStack) {
        self.ids.append(&mut other.ids);
        self.values.append(&mut other.values);
    }

    /// 迭代器：从栈底到栈顶
    pub fn iter_bottom_to_top(&self) -> impl Iterator<Item = (&ElementId, &Value)> {
        self.ids.iter().zip(self.values.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_alloc() -> IdAllocator {
        IdAllocator::new()
    }

    fn push_n(stack: &mut DualStack, alloc: &mut IdAllocator, n: usize) {
        for i in 0..n {
            stack.push(alloc.alloc_element(), Value::I64(i as i64));
        }
    }

    #[test]
    fn test_push_pop() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        let id = alloc.alloc_element();
        stack.push(id, Value::I64(42));
        assert_eq!(stack.len(), 1);
        let (popped_id, popped_val) = stack.pop().unwrap();
        assert_eq!(popped_id, id);
        assert_eq!(popped_val, Value::I64(42));
        assert!(stack.is_empty());
    }

    #[test]
    fn test_dup() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        stack.push(alloc.alloc_element(), Value::I64(10));
        stack.dup(&mut alloc).unwrap();
        assert_eq!(stack.len(), 2);
        // 值相同但 ID 不同
        assert_eq!(stack.values[0], Value::I64(10));
        assert_eq!(stack.values[1], Value::I64(10));
        assert_ne!(stack.ids[0], stack.ids[1]);
    }

    #[test]
    fn test_dup_empty() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        assert!(stack.dup(&mut alloc).is_err());
    }

    // === roll 测试 ===

    #[test]
    fn test_roll_zero_is_noop() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        push_n(&mut stack, &mut alloc, 3);
        let before = stack.values.clone();
        stack.roll(0).unwrap();
        assert_eq!(stack.values, before);
    }

    #[test]
    fn test_roll_positive() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        // push: a=1, b=2, c=3, d=4 (栈顶是 d)
        for v in [1, 2, 3, 4] {
            stack.push(alloc.alloc_element(), Value::I64(v));
        }
        // 栈(底→顶): [1, 2, 3, 4]，栈顶=4
        // roll(2): 将位置 2 的元素(值=2)提取到栈顶
        // 期望: [1, 3, 4, 2]
        stack.roll(2).unwrap();
        assert_eq!(stack.values[0], Value::I64(1));
        assert_eq!(stack.values[1], Value::I64(3));
        assert_eq!(stack.values[2], Value::I64(4));
        assert_eq!(stack.values[3], Value::I64(2));
    }

    #[test]
    fn test_roll_negative() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        // push: a=1, b=2, c=3, d=4 (栈顶是 d)
        for v in [1, 2, 3, 4] {
            stack.push(alloc.alloc_element(), Value::I64(v));
        }
        // roll(-2): 将栈顶(d=4)提取，[1,n]=[1,2]位置上移(elements 3,2 go up),
        //            然后将4放到位置2
        // 顶→底: [4,3,2,1] → [3,2,4,1]
        // 底→顶(Vec): [1,4,2,3]
        stack.roll(-2).unwrap();
        assert_eq!(stack.values[0], Value::I64(1));
        assert_eq!(stack.values[1], Value::I64(4));
        assert_eq!(stack.values[2], Value::I64(2));
        assert_eq!(stack.values[3], Value::I64(3));
    }

    #[test]
    fn test_roll_roundtrip() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        for v in [10, 20, 30, 40, 50] {
            stack.push(alloc.alloc_element(), Value::I64(v));
        }
        let original = stack.values.clone();
        let original_ids = stack.ids.clone();

        // roll(+3) then roll(-3) 应恢复原状
        stack.roll(3).unwrap();
        stack.roll(-3).unwrap();
        assert_eq!(stack.values, original);
        assert_eq!(stack.ids, original_ids);
    }

    #[test]
    fn test_roll_out_of_bounds() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        push_n(&mut stack, &mut alloc, 3);
        assert!(stack.roll(3).is_err()); // len=3, roll(3) 超出 (位置 0,1,2)
        assert!(stack.roll(-4).is_err());
        assert!(stack.roll(1).is_ok()); // 合法
    }

    #[test]
    fn test_roll_empty() {
        let mut stack = DualStack::new();
        assert!(stack.roll(1).is_err());
        assert!(stack.roll(-1).is_err());
    }

    // === swap 测试 ===

    #[test]
    fn test_swap() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        stack.push(alloc.alloc_element(), Value::I64(1));
        stack.push(alloc.alloc_element(), Value::I64(2));
        stack.swap().unwrap();
        assert_eq!(stack.values[0], Value::I64(2));
        assert_eq!(stack.values[1], Value::I64(1));
    }

    #[test]
    fn test_swap_underflow() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        stack.push(alloc.alloc_element(), Value::I64(1));
        assert!(stack.swap().is_err());
    }

    // === pick 测试 ===

    #[test]
    fn test_pick() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        for v in [1, 2, 3, 4] {
            stack.push(alloc.alloc_element(), Value::I64(v));
        }
        // 栈顶: 4, 下面: 3, 2, 1
        // pick(2): 复制位置 2（值为 2）到栈顶
        stack.push(alloc.alloc_element(), Value::I64(2)); // push n=2
        stack.pick(&mut alloc).unwrap();
        // 栈: [1, 2, 3, 4, 2]
        assert_eq!(stack.len(), 5);
        assert_eq!(stack.values.last().unwrap(), &Value::I64(2));
    }

    #[test]
    fn test_find_id() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        let id0 = alloc.alloc_element();
        let id1 = alloc.alloc_element();
        let id2 = alloc.alloc_element();
        stack.push(id0, Value::I64(10));
        stack.push(id1, Value::I64(20));
        stack.push(id2, Value::I64(30));
        // 栈(底→顶): [id0:10, id1:20, id2:30]
        assert_eq!(stack.find_id(id2), Some(0)); // 栈顶
        assert_eq!(stack.find_id(id1), Some(1));
        assert_eq!(stack.find_id(id0), Some(2));
        assert_eq!(stack.find_id(ElementId(999)), None);
    }

    #[test]
    fn test_get_from_top() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        stack.push(alloc.alloc_element(), Value::I64(100));
        stack.push(alloc.alloc_element(), Value::I64(200));
        stack.push(alloc.alloc_element(), Value::I64(300));
        assert_eq!(stack.get_from_top(0).unwrap().1, &Value::I64(300));
        assert_eq!(stack.get_from_top(1).unwrap().1, &Value::I64(200));
        assert_eq!(stack.get_from_top(2).unwrap().1, &Value::I64(100));
        assert!(stack.get_from_top(3).is_none());
    }

    #[test]
    fn test_reverse_keeps_ids_aligned() {
        let mut alloc = make_alloc();
        let mut stack = DualStack::new();
        let id0 = alloc.alloc_element();
        let id1 = alloc.alloc_element();
        let id2 = alloc.alloc_element();
        stack.push(id0, Value::I64(10));
        stack.push(id1, Value::I64(20));
        stack.push(id2, Value::I64(30));
        stack.reverse();
        // 反转后顶→底为 10、20、30，ID 与值保持对齐。
        assert_eq!(stack.get_from_top(0).unwrap(), (&id0, &Value::I64(10)));
        assert_eq!(stack.get_from_top(1).unwrap(), (&id1, &Value::I64(20)));
        assert_eq!(stack.get_from_top(2).unwrap(), (&id2, &Value::I64(30)));
    }
}
