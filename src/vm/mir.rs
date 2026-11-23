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
    /// 类型转换（`expr as T`）
    Cast {
        target: TypeId,
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
    Call { fn_id: FnId, return_arity: u32 }, // 函数调用（argc从CallElement获取, return_arity从签名获取）
    CallIndirect { argc: u32, return_arity: u32 }, // 间接调用：栈布局 [args..., fnptr]
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
    StoreDeref,                   // *p = v (pop v, pop ref)
    FieldStore { name: String, index: usize }, // obj.field = v
    FieldStoreRef { name: String, index: usize }, // `&mut self` 方法内 self.field = v
    IndexStore { name: String },  // arr[i] = v (pop v, pop i)
    MakeArray { count: u32 },     // 将栈顶 count 个值合并为单槽数组
    MakeArrayRepeat,              // pop count, pop elem，生成重复数组
    MakeBox,                      // 将栈顶值包装为 Box
    ContainerLen,                 // pop 容器（数组/Vec/String），push 长度
    VecPush { name: String },     // v.push(x): pop x，就地追加到变量
    VecPop { name: String },      // v.pop(): 就地移除末元素，push 结果
    DropVar { name: String },     // 作用域退出时释放变量持有的远程栈（若有）
    MakeEnum { variant: usize, data_slots: u32 },
    EnumTag { total_slots: u32 }, // 读取枚举标签（total_slots = 1 + max_data_slots）

    // === unsafe 值栈（ADR 0004）===
    RawPush, // 把求值栈顶移入值栈
    RawPop,  // 把值栈顶移回求值栈
    RawSwap, // 交换值栈顶两个元素
    RawDup,  // 复制值栈顶
    RawRoll, // pop n；把值栈顶元素移动 n 个位置

    // === 堆操作（stardust! 的 store/load）===
    HeapStore,                   // 堆写：pop addr, pop value, heap[addr] = value
    HeapLoad,                    // 堆读：pop addr, push heap[addr]

    // === 标准库 ===
    Abs,                         // pop i64, push |x|
    Min,                         // pop b, pop a, push min(a, b)
    Max,                         // pop b, pop a, push max(a, b)
    IntToStr,                    // pop i64，push 十进制字符串
    StrToInt { some_tag: i64, none_tag: i64 }, // pop 字符串，push Option<i64>
    Unwrap { success_tag: i64 }, // pop enum, push payload; 失败时运行时错误
    UnwrapOr { success_tag: i64 }, // pop default, pop enum; 成功 payload，否则 default
    /// `Result::ok()`/`Result::err()`：pop 枚举，push Option
    /// （源 tag == `match_tag` 时取 payload 组成 `[some_tag, payload]`，否则 `[none_tag]`）。
    EnumPayloadToOption {
        match_tag: i64,
        some_tag: i64,
        none_tag: i64,
    },
    /// `Option::ok_or(default)`：pop default、pop Option，push Result
    /// （源 tag == `some_tag` 时组成 `[ok_tag, payload]`，否则 `[err_tag, default]`）。
    OptionToResult {
        some_tag: i64,
        ok_tag: i64,
        err_tag: i64,
    },

    // === I/O ===
    NumOut,
    NumIn,
    CharOut,
    CharIn,
    StrOut { newline: bool }, // 输出字符串（字符元组），可选换行
    ReadLine,                 // 读取一行并压入字符串（字符元组）
    FileRead,                 // pop path(String)，push 文件内容(String)
    FileWrite,                // pop data(String), path(String)，写入文件

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
            MirInst::Call { fn_id, return_arity } => write!(f, "Call({fn_id:?}, ret={return_arity})"),
            MirInst::CallIndirect { argc, return_arity } => {
                write!(f, "CallIndirect(argc={argc}, ret={return_arity})")
            }
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
            MirInst::StoreDeref => write!(f, "StoreDeref"),
            MirInst::FieldStore { name, index } => {
                write!(f, "FieldStore({name}.{index})")
            }
            MirInst::FieldStoreRef { name, index } => {
                write!(f, "FieldStoreRef({name}.{index})")
            }
            MirInst::IndexStore { name } => write!(f, "IndexStore({name})"),
            MirInst::MakeArray { count } => write!(f, "MakeArray(count={count})"),
            MirInst::MakeArrayRepeat => write!(f, "MakeArrayRepeat"),
            MirInst::MakeBox => write!(f, "MakeBox"),
            MirInst::ContainerLen => write!(f, "ContainerLen"),
            MirInst::VecPush { name } => write!(f, "VecPush({name})"),
            MirInst::VecPop { name } => write!(f, "VecPop({name})"),
            MirInst::DropVar { name } => write!(f, "DropVar({name})"),
            MirInst::MakeEnum {
                variant,
                data_slots,
            } => write!(f, "MakeEnum(variant={variant}, data_slots={data_slots})"),
            MirInst::EnumTag { total_slots } => write!(f, "EnumTag(total_slots={total_slots})"),
            MirInst::RawPush => write!(f, "RawPush"),
            MirInst::RawPop => write!(f, "RawPop"),
            MirInst::RawSwap => write!(f, "RawSwap"),
            MirInst::RawDup => write!(f, "RawDup"),
            MirInst::RawRoll => write!(f, "RawRoll"),
            MirInst::HeapStore => write!(f, "HeapStore"),
            MirInst::HeapLoad => write!(f, "HeapLoad"),
            MirInst::Abs => write!(f, "Abs"),
            MirInst::Min => write!(f, "Min"),
            MirInst::Max => write!(f, "Max"),
            MirInst::IntToStr => write!(f, "IntToStr"),
            MirInst::StrToInt {
                some_tag,
                none_tag,
            } => write!(f, "StrToInt(some_tag={some_tag}, none_tag={none_tag})"),
            MirInst::Unwrap { success_tag } => write!(f, "Unwrap({success_tag})"),
            MirInst::UnwrapOr { success_tag } => write!(f, "UnwrapOr({success_tag})"),
            MirInst::EnumPayloadToOption {
                match_tag,
                some_tag,
                none_tag,
            } => write!(
                f,
                "EnumPayloadToOption(match_tag={match_tag}, some_tag={some_tag}, none_tag={none_tag})"
            ),
            MirInst::OptionToResult {
                some_tag,
                ok_tag,
                err_tag,
            } => write!(
                f,
                "OptionToResult(some_tag={some_tag}, ok_tag={ok_tag}, err_tag={err_tag})"
            ),
            MirInst::NumOut => write!(f, "NumOut"),
            MirInst::NumIn => write!(f, "NumIn"),
            MirInst::CharOut => write!(f, "CharOut"),
            MirInst::CharIn => write!(f, "CharIn"),
            MirInst::StrOut { newline } => {
                write!(f, "StrOut(newline={newline})")
            }
            MirInst::ReadLine => write!(f, "ReadLine"),
            MirInst::Cast { target } => write!(f, "Cast({})", target.name()),
            MirInst::FileRead => write!(f, "FileRead"),
            MirInst::FileWrite => write!(f, "FileWrite"),
            MirInst::DumpStack => write!(f, "DumpStack"),
            MirInst::DumpState => write!(f, "DumpState"),
            MirInst::Breakpoint => write!(f, "Breakpoint"),
            MirInst::Nop => write!(f, "Nop"),
            MirInst::Comment(s) => write!(f, "// {s}"),
        }
    }
}
