use serde::Serialize;

use crate::vm::id::FnId;
use crate::vm::value::Value;

/// 类型标识符（Phase 1 占位，Phase 5 完善）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum TypeId {
    // 基本类型
    I8,
    I16,
    I32,
    I64,
    Isize,
    U8,
    U16,
    U32,
    U64,
    Usize,
    F32,
    F64,
    Bool,
    Char,
    Unit,
    // 引用
    Ref,
    MutRef,
    // 其他
    ElementId,
    StackHandle,
    FnPtr,
    /// 未知/动态类型（仅用于 unsafe 上下文）
    Unknown,
}

impl TypeId {
    pub fn name(&self) -> &'static str {
        match self {
            TypeId::I8 => "i8",
            TypeId::I16 => "i16",
            TypeId::I32 => "i32",
            TypeId::I64 => "i64",
            TypeId::Isize => "isize",
            TypeId::U8 => "u8",
            TypeId::U16 => "u16",
            TypeId::U32 => "u32",
            TypeId::U64 => "u64",
            TypeId::Usize => "usize",
            TypeId::F32 => "f32",
            TypeId::F64 => "f64",
            TypeId::Bool => "bool",
            TypeId::Char => "char",
            TypeId::Unit => "()",
            TypeId::Ref => "&T",
            TypeId::MutRef => "&mut T",
            TypeId::ElementId => "ElementId",
            TypeId::StackHandle => "StackHandle",
            TypeId::FnPtr => "fn",
            TypeId::Unknown => "?",
        }
    }
}

/// MIR 指令 — 介于 HIR 和 Stardust 指令之间的中间表示
#[derive(Debug, Clone, Serialize)]
pub enum MirInst {
    // === 栈操作（带类型标注）===
    Push {
        value: Value,
        #[serde(skip)]
        ty: TypeId,
    },
    Pop {
        #[serde(skip)]
        ty: TypeId,
    },
    Dup {
        #[serde(skip)]
        ty: TypeId,
    },
    Roll {
        n: i32,
    }, // 正/负/零 roll
    Swap,
    Pick, // pop n, dup stack[n]

    // === 算术 ===
    Add {
        #[serde(skip)]
        ty: TypeId,
    },
    Sub {
        #[serde(skip)]
        ty: TypeId,
    },
    Mul {
        #[serde(skip)]
        ty: TypeId,
    },
    Div {
        #[serde(skip)]
        ty: TypeId,
    },
    Mod {
        #[serde(skip)]
        ty: TypeId,
    },
    Neg {
        #[serde(skip)]
        ty: TypeId,
    },

    // === 比较 ===
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,

    // === 逻辑 ===
    And,
    Or,
    Not,
    Xor,

    // === 控制流 ===
    Mark(usize),           // 跳转目标
    Jump(usize),           // 条件跳转（pop cond, if cond≠0 jump）
    UncondJump(usize),     // 无条件跳转
    Call { fn_id: FnId }, // 函数调用（argc 从 CallElement 获取）
    Ret { argc: u32 },     // 返回

    // === 变量与作用域 ===
    EnterScope,
    ExitScope,
    BindVar { name: String },  // 将栈顶值绑定到变量名
    ReadVar { name: String },  // 栈顶化读取（编译为 roll+dup+roll(-pos)）
    WriteVar { name: String }, // 栈顶化写入（预求值 + roll+pop+roll(-pos)）

    // === 引用 ===
    Ref { name: String, mutable: bool }, // &x / &mut x
    Deref,                               // *r

    // === 复合类型 ===
    FieldAccess { index: usize }, // 字段/元组元素访问
    IndexAccess,                  // arr[i] (pop i, pop arr, push arr[i])
    MakeEnum { variant: usize, data_slots: u32 },
    EnumTag, // 读取枚举标签

    // === 远程栈操作 ===
    AllocRemote,                 // 分配远程栈（pop capacity, push StackHandle）
    FreeRemote,                  // 释放远程栈（pop StackId）
    RemotePush,                  // 向远程栈 push
    RemotePop,                   // 从远程栈 pop
    RemoteLoad { index: u32 },   // 读取远程栈元素
    RemoteStore { index: u32 },  // 写入远程栈元素
    RemoteLen,                   // 读取远程栈 length
    SwitchToRemote,              // 将当前栈上下文切换到指定远程栈
    SwitchToLocal,               // 恢复本地栈上下文

    // === I/O ===
    NumOut,
    NumIn,
    CharOut,
    CharIn,

    // === 调试 ===
    DumpStack,
    DumpState,
    Breakpoint,

    // === 元数据 ===
    Nop,
    Comment(String), // 调试用注释，执行时忽略
}

impl MirInst {
    /// 是否对控制流有影响（用于优化遍判断基本块边界）
    pub fn is_control_flow(&self) -> bool {
        matches!(
            self,
            MirInst::Jump(_)
                | MirInst::UncondJump(_)
                | MirInst::Call { .. }
                | MirInst::Ret { .. }
                | MirInst::Mark(_)
        )
    }

    /// 是否为跳转目标
    pub fn is_mark(&self) -> bool {
        matches!(self, MirInst::Mark(_))
    }
}

impl std::fmt::Display for MirInst {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MirInst::Push { value, ty: _ } => write!(f, "Push({value})"),
            MirInst::Pop { .. } => write!(f, "Pop"),
            MirInst::Dup { .. } => write!(f, "Dup"),
            MirInst::Roll { n } => write!(f, "Roll({n})"),
            MirInst::Swap => write!(f, "Swap"),
            MirInst::Pick => write!(f, "Pick"),
            MirInst::Add { .. } => write!(f, "Add"),
            MirInst::Sub { .. } => write!(f, "Sub"),
            MirInst::Mul { .. } => write!(f, "Mul"),
            MirInst::Div { .. } => write!(f, "Div"),
            MirInst::Mod { .. } => write!(f, "Mod"),
            MirInst::Neg { .. } => write!(f, "Neg"),
            MirInst::Eq => write!(f, "Eq"),
            MirInst::Ne => write!(f, "Ne"),
            MirInst::Lt => write!(f, "Lt"),
            MirInst::Gt => write!(f, "Gt"),
            MirInst::Le => write!(f, "Le"),
            MirInst::Ge => write!(f, "Ge"),
            MirInst::And => write!(f, "And"),
            MirInst::Or => write!(f, "Or"),
            MirInst::Not => write!(f, "Not"),
            MirInst::Xor => write!(f, "Xor"),
            MirInst::Mark(n) => write!(f, "Mark({n})"),
            MirInst::Jump(n) => write!(f, "Jump({n})"),
            MirInst::UncondJump(n) => write!(f, "UncondJump({n})"),
            MirInst::Call { fn_id } => write!(f, "Call({fn_id:?})"),
            MirInst::Ret { argc } => write!(f, "Ret({argc})"),
            MirInst::EnterScope => write!(f, "EnterScope"),
            MirInst::ExitScope => write!(f, "ExitScope"),
            MirInst::BindVar { name } => write!(f, "BindVar({name})"),
            MirInst::ReadVar { name } => write!(f, "ReadVar({name})"),
            MirInst::WriteVar { name } => write!(f, "WriteVar({name})"),
            MirInst::Ref { name, mutable } => {
                let m = if *mutable { "mut " } else { "" };
                write!(f, "Ref(&{m}{name})")
            }
            MirInst::Deref => write!(f, "Deref"),
            MirInst::FieldAccess { index } => write!(f, "FieldAccess({index})"),
            MirInst::IndexAccess => write!(f, "IndexAccess"),
            MirInst::MakeEnum {
                variant,
                data_slots,
            } => write!(f, "MakeEnum(variant={variant}, data_slots={data_slots})"),
            MirInst::EnumTag => write!(f, "EnumTag"),
            MirInst::AllocRemote => write!(f, "AllocRemote"),
            MirInst::FreeRemote => write!(f, "FreeRemote"),
            MirInst::RemotePush => write!(f, "RemotePush"),
            MirInst::RemotePop => write!(f, "RemotePop"),
            MirInst::RemoteLoad { index } => write!(f, "RemoteLoad({index})"),
            MirInst::RemoteStore { index } => write!(f, "RemoteStore({index})"),
            MirInst::RemoteLen => write!(f, "RemoteLen"),
            MirInst::SwitchToRemote => write!(f, "SwitchToRemote"),
            MirInst::SwitchToLocal => write!(f, "SwitchToLocal"),
            MirInst::NumOut => write!(f, "NumOut"),
            MirInst::NumIn => write!(f, "NumIn"),
            MirInst::CharOut => write!(f, "CharOut"),
            MirInst::CharIn => write!(f, "CharIn"),
            MirInst::DumpStack => write!(f, "DumpStack"),
            MirInst::DumpState => write!(f, "DumpState"),
            MirInst::Breakpoint => write!(f, "Breakpoint"),
            MirInst::Nop => write!(f, "Nop"),
            MirInst::Comment(s) => write!(f, "// {s}"),
        }
    }
}
