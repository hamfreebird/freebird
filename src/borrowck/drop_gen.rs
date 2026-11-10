use crate::vm::mir::MirInst;

/// Drop 代码生成器 — 在作用域退出时插入析构指令
pub struct DropGen;

impl DropGen {
    /// 为一组待析构变量生成 Drop MIR 序列
    /// `vars`: 按逆序排列的变量名列表（后创建的先析构）
    /// 每个变量生成 DropVar（仅在持有远程栈句柄时释放资源）
    pub fn gen_drop(vars: &[String]) -> Vec<MirInst> {
        let mut insts = Vec::new();
        for name in vars {
            insts.push(MirInst::Comment(format!("drop {name}")));
            insts.push(MirInst::DropVar { name: name.clone() });
        }
        insts
    }

    /// 在作用域退出时生成析构指令（简化版：仅清理远程栈句柄）
    pub fn gen_scope_exit(drop_vars: &[String]) -> Vec<MirInst> {
        if drop_vars.is_empty() {
            return vec![MirInst::ExitScope];
        }
        let mut insts = Self::gen_drop(drop_vars);
        insts.push(MirInst::ExitScope);
        insts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gen_drop_empty() {
        let insts = DropGen::gen_drop(&[]);
        assert!(insts.is_empty());
    }

    #[test]
    fn test_gen_drop_single() {
        let insts = DropGen::gen_drop(&["v".to_string()]);
        // Comment + DropVar = 2
        assert_eq!(insts.len(), 2);
        assert!(matches!(insts[1], MirInst::DropVar { ref name } if name == "v"));
    }

    #[test]
    fn test_gen_scope_exit() {
        let insts = DropGen::gen_scope_exit(&["v".to_string()]);
        assert!(matches!(insts.last().unwrap(), MirInst::ExitScope));
    }
}
