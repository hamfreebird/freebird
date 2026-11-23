//! Freebird 标准 I/O 库
//!
//! 提供基本的输入输出功能：print、read_int、read_char、文件操作等。
//! 使用 MIR I/O 指令（NumOut、CharOut、NumIn、CharIn）实现。

use crate::vm::mir::{MirInst, TypeId};
use crate::vm::value::Value;

/// 内建函数元数据（HIR 解析与 MIR 生成共用）
#[derive(Debug, Clone, Copy)]
pub struct BuiltinDef {
    pub name: &'static str,
    pub params: &'static [&'static str],
    pub returns: &'static [&'static str],
}

/// 当前可直接调用的内建函数。
///
/// `print` 是历史兼容入口，等价于 `print_int`。
pub const BUILTINS: &[BuiltinDef] = &[
    BuiltinDef { name: "print", params: &["i64"], returns: &[] },
    BuiltinDef { name: "print_int", params: &["i64"], returns: &[] },
    BuiltinDef { name: "print_char", params: &["char"], returns: &[] },
    BuiltinDef { name: "print_str", params: &["String"], returns: &[] },
    BuiltinDef { name: "println_str", params: &["String"], returns: &[] },
    BuiltinDef { name: "read_int", params: &[], returns: &["i64"] },
    BuiltinDef { name: "read_char", params: &[], returns: &["char"] },
    BuiltinDef { name: "read_line", params: &[], returns: &["String"] },
    BuiltinDef { name: "file_read", params: &["String"], returns: &["String"] },
    BuiltinDef {
        name: "file_write",
        params: &["String", "String"],
        returns: &[],
    },
    BuiltinDef { name: "abs", params: &["i64"], returns: &["i64"] },
    BuiltinDef { name: "int_to_str", params: &["i64"], returns: &["String"] },
    BuiltinDef { name: "str_to_int", params: &["String"], returns: &["Option<i64>"] },
    BuiltinDef { name: "min", params: &["i64", "i64"], returns: &["i64"] },
    BuiltinDef { name: "max", params: &["i64", "i64"], returns: &["i64"] },
];

/// 按名字查找内建函数
pub fn builtin_def(name: &str) -> Option<&'static BuiltinDef> {
    BUILTINS.iter().find(|b| b.name == name)
}

/// 是否为内建函数名
pub fn is_builtin(name: &str) -> bool {
    builtin_def(name).is_some()
}

/// IO 库 — 生成常用 I/O 操作的 MIR 指令
pub struct IoLib;

impl IoLib {
    /// 生成 print_int(n) 的 MIR 指令
    /// 期望栈顶有一个 i64 值
    pub fn print_int() -> Vec<MirInst> {
        vec![MirInst::NumOut]
    }

    /// 生成 print_char(c) 的 MIR 指令
    /// 期望栈顶有一个 char 值
    pub fn print_char() -> Vec<MirInst> {
        vec![MirInst::CharOut]
    }

    /// 生成 print_str(s) 的 MIR 指令（逐字符输出，不追加换行）
    /// s 必须在常量中
    pub fn print_str(s: &str) -> Vec<MirInst> {
        let mut insts = Vec::new();
        for ch in s.chars() {
            insts.push(MirInst::Push {
                value: Value::Char(ch as u32),
                ty: TypeId::Char,
            });
            insts.push(MirInst::CharOut);
        }
        insts
    }

    /// 生成 println_str(s) 的 MIR 指令（输出后追加换行）
    pub fn println_str(s: &str) -> Vec<MirInst> {
        let mut insts = Self::print_str(s);
        insts.push(MirInst::Push {
            value: Value::Char('\n' as u32),
            ty: TypeId::Char,
        });
        insts.push(MirInst::CharOut);
        insts
    }

    /// 生成 read_int() 的 MIR 指令
    /// 结果在栈顶（i64）
    pub fn read_int() -> Vec<MirInst> {
        vec![MirInst::NumIn]
    }

    /// 生成 read_char() 的 MIR 指令
    /// 结果在栈顶（char）
    pub fn read_char() -> Vec<MirInst> {
        vec![MirInst::CharIn]
    }

    /// 生成 read_line() 的 MIR 指令（读取一行，结果在栈顶）
    pub fn read_line() -> Vec<MirInst> {
        vec![MirInst::ReadLine]
    }

    /// 生成 file_read(path) 的 MIR 指令（编译期路径）
    pub fn file_read(path: &str) -> Vec<MirInst> {
        let mut insts = string_value_insts(path);
        insts.push(MirInst::FileRead);
        insts
    }

    /// 生成 file_write(path, data) 的 MIR 指令（编译期路径与数据）
    pub fn file_write(path: &str, data: &str) -> Vec<MirInst> {
        let mut insts = string_value_insts(path);
        insts.extend(string_value_insts(data));
        insts.push(MirInst::FileWrite);
        insts
    }
}

/// 生成压入字符串常量的指令（字符元组）。
fn string_value_insts(s: &str) -> Vec<MirInst> {
    vec![MirInst::Push {
        value: Value::Tuple(s.chars().map(|c| Value::Char(c as u32)).collect()),
        ty: TypeId::Unknown,
    }]
}

/// 标准库函数集 — 返回 (函数名, MIR指令序列) 的列表
pub fn std_library() -> Vec<(&'static str, Vec<MirInst>)> {
    vec![
        ("print_int", IoLib::print_int()),
        ("print_char", IoLib::print_char()),
        ("read_int", IoLib::read_int()),
        ("read_char", IoLib::read_char()),
        ("read_line", IoLib::read_line()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_str() {
        let insts = IoLib::print_str("Hello");
        // 5 个字符，每个 Push + CharOut = 10 条；不再追加换行。
        assert_eq!(insts.len(), 10);
        assert!(!insts.iter().any(|i| matches!(
            i,
            MirInst::Push { value: Value::Char(10), .. }
        )));
    }

    #[test]
    fn test_println_str_adds_newline() {
        let insts = IoLib::println_str("Hi");
        assert_eq!(insts.len(), 6); // Push+CharOut ×2 + newline Push+CharOut
        assert!(matches!(
            insts[insts.len() - 2],
            MirInst::Push { value: Value::Char(10), .. }
        ));
    }

    #[test]
    fn test_file_io_instructions() {
        let read = IoLib::file_read("/tmp/x");
        assert!(matches!(read.last(), Some(MirInst::FileRead)));

        let write = IoLib::file_write("/tmp/x", "data");
        assert!(matches!(write.last(), Some(MirInst::FileWrite)));
    }

    #[test]
    fn test_std_library() {
        let lib = std_library();
        assert_eq!(lib.len(), 5); // print_int, print_char, read_int, read_char, read_line
    }
}
