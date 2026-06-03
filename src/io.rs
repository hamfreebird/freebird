//! Freebird 标准 I/O 库
//!
//! 提供基本的输入输出功能：print、read_int、read_char、文件操作等。
//! 使用 MIR I/O 指令（NumOut、CharOut、NumIn、CharIn）实现。

use crate::vm::mir::{MirInst, TypeId};
use crate::vm::value::Value;

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

    /// 生成 print_str(s) 的 MIR 指令（逐字符输出）
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
        // 输出换行
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

    /// 生成 read_line() 的 MIR 指令（循环读取直到换行）
    /// 将字符串存储在远程栈中，返回 StackHandle
    pub fn read_line() -> Vec<MirInst> {
        // 分配远程栈 (cap=256)，循环读字符直到 '\n'
        // 使用 Mark/Jump 实现循环: for each char, RemotePush if not '\n'
        let mut insts = vec![
            MirInst::Push { value: Value::Usize(256), ty: TypeId::Usize },
            MirInst::AllocRemote,
        ];
        // Mark(0): loop start
        insts.push(MirInst::Mark(0));
        // Read char
        insts.push(MirInst::CharIn);
        // Dup char, Push '\n', Eq, Not → true if char != '\n'
        insts.push(MirInst::Dup { ty: TypeId::Char });
        insts.push(MirInst::Push { value: Value::Char('\n' as u32), ty: TypeId::Char });
        insts.push(MirInst::Eq);
        insts.push(MirInst::Not);
        // Jump to end (mark 1) if char == '\n'
        insts.push(MirInst::Jump(1));
        // Push char to remote (RemotePush consumes handle+value, re-pushes handle)
        insts.push(MirInst::Dup { ty: TypeId::Unknown }); // dup handle (at pos 1 after char)
        insts.push(MirInst::Swap); // handle on top, then char
        insts.push(MirInst::RemotePush);
        // Loop back
        insts.push(MirInst::UncondJump(0));
        // Mark(1): loop end — skip newline, keep handle on stack
        insts.push(MirInst::Mark(1));
        insts.push(MirInst::Pop { ty: TypeId::Char }); // discard newline
        insts.push(MirInst::Pop { ty: TypeId::Char }); // discard dup'd newline
        insts
    }

    /// 生成 file_read(path) 的 MIR 指令（简化版：仅输出注释）
    pub fn file_read(_path: &str) -> Vec<MirInst> {
        vec![
            MirInst::Comment(format!("file_read: {_path}")),
        ]
    }

    /// 生成 file_write(path, data) 的 MIR 指令（简化版）
    pub fn file_write(_path: &str, _data: &str) -> Vec<MirInst> {
        vec![
            MirInst::Comment(format!("file_write: {_path}")),
        ]
    }
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
        assert!(!insts.is_empty());
        // "Hello\n" = 6 chars, each with Push + CharOut = 12 insts
        assert_eq!(insts.len(), 12);
    }

    #[test]
    fn test_std_library() {
        let lib = std_library();
        assert_eq!(lib.len(), 5); // print_int, print_char, read_int, read_char, read_line
    }
}
