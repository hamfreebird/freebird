use serde::Serialize;

use crate::vm::id::{ElementId, FnId, StackId};

/// 虚拟机中的值
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Value {
    // === 基本类型 — 底层存储为 i64 ===
    I8(i64),
    I16(i64),
    I32(i64),
    I64(i64),
    Isize(i64),
    U8(i64),
    U16(i64),
    U32(i64),
    U64(i64),
    Usize(i64),

    // 浮点数单独存储
    F32(f32),
    F64(f64),

    // 布尔和字符
    Bool(bool),
    Char(u32), // Unicode 标量值
    Unit,      // ()

    // === 复合类型 ===
    /// 元组（仅用于常量折叠，运行时展开到栈槽）
    Tuple(Vec<Value>),

    // === 引用类型 ===
    Ref(ElementId),    // &T
    MutRef(ElementId), // &mut T

    // === 句柄类型 ===
    /// 远程栈引用
    StackHandle(StackId),

    // === 函数指针 ===
    FnPtr(FnId),

    // === 内部类型（不可由用户构造）===
    /// 调用元素 — 标记函数调用，值为参数个数
    CallElement { argc: u32 },
}

impl Value {
    /// 尝试将值解释为 i64（用于算术运算）
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::I8(v)
            | Value::I16(v)
            | Value::I32(v)
            | Value::I64(v)
            | Value::Isize(v)
            | Value::U8(v)
            | Value::U16(v)
            | Value::U32(v)
            | Value::U64(v)
            | Value::Usize(v) => Some(*v),
            Value::Bool(b) => Some(if *b { 1 } else { 0 }),
            Value::Char(c) => Some(*c as i64),
            Value::F32(f) => Some(*f as i64),
            Value::F64(f) => Some(*f as i64),
            _ => None,
        }
    }

    /// 尝试将值解释为 bool
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::I8(v)
            | Value::I16(v)
            | Value::I32(v)
            | Value::I64(v)
            | Value::Isize(v)
            | Value::U8(v)
            | Value::U16(v)
            | Value::U32(v)
            | Value::U64(v)
            | Value::Usize(v) => Some(*v != 0),
            _ => None,
        }
    }

    /// 尝试将值解释为 f64
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::F32(f) => Some(*f as f64),
            Value::F64(f) => Some(*f),
            Value::I8(v) | Value::I16(v) | Value::I32(v) | Value::I64(v) | Value::Isize(v) => {
                Some(*v as f64)
            }
            _ => None,
        }
    }

    /// 判断是否为 Copy 类型
    pub fn is_copy_type(&self) -> bool {
        matches!(
            self,
            Value::I8(_)
                | Value::I16(_)
                | Value::I32(_)
                | Value::I64(_)
                | Value::Isize(_)
                | Value::U8(_)
                | Value::U16(_)
                | Value::U32(_)
                | Value::U64(_)
                | Value::Usize(_)
                | Value::F32(_)
                | Value::F64(_)
                | Value::Bool(_)
                | Value::Char(_)
                | Value::Unit
                | Value::Ref(_)
                | Value::MutRef(_)
                | Value::FnPtr(_)
        )
    }

    /// 获取值的简短类型名称（用于错误消息）
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::I8(_) => "i8",
            Value::I16(_) => "i16",
            Value::I32(_) => "i32",
            Value::I64(_) => "i64",
            Value::Isize(_) => "isize",
            Value::U8(_) => "u8",
            Value::U16(_) => "u16",
            Value::U32(_) => "u32",
            Value::U64(_) => "u64",
            Value::Usize(_) => "usize",
            Value::F32(_) => "f32",
            Value::F64(_) => "f64",
            Value::Bool(_) => "bool",
            Value::Char(_) => "char",
            Value::Unit => "()",
            Value::Tuple(_) => "tuple",
            Value::Ref(_) => "&T",
            Value::MutRef(_) => "&mut T",
            Value::StackHandle(_) => "StackHandle",
            Value::FnPtr(_) => "fn",
            Value::CallElement { .. } => "CallElement",
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::I8(v) => write!(f, "{v}_i8"),
            Value::I16(v) => write!(f, "{v}_i16"),
            Value::I32(v) => write!(f, "{v}_i32"),
            Value::I64(v) => write!(f, "{v}_i64"),
            Value::Isize(v) => write!(f, "{v}_isize"),
            Value::U8(v) => write!(f, "{v}_u8"),
            Value::U16(v) => write!(f, "{v}_u16"),
            Value::U32(v) => write!(f, "{v}_u32"),
            Value::U64(v) => write!(f, "{v}_u64"),
            Value::Usize(v) => write!(f, "{v}_usize"),
            Value::F32(v) => write!(f, "{v}_f32"),
            Value::F64(v) => write!(f, "{v}_f64"),
            Value::Bool(v) => write!(f, "{v}"),
            Value::Char(c) => {
                if let Some(ch) = char::from_u32(*c) {
                    write!(f, "'{ch}'")
                } else {
                    write!(f, "char({c})")
                }
            }
            Value::Unit => write!(f, "()"),
            Value::Tuple(elems) => {
                write!(f, "(")?;
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{e}")?;
                }
                write!(f, ")")
            }
            Value::Ref(id) => write!(f, "&{:?}", id),
            Value::MutRef(id) => write!(f, "&mut {:?}", id),
            Value::StackHandle(sid) => write!(f, "StackHandle({:?})", sid),
            Value::FnPtr(fid) => write!(f, "FnPtr({:?})", fid),
            Value::CallElement { argc } => write!(f, "CallElement(argc={})", argc),
        }
    }
}
