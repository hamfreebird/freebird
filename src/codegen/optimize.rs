use std::collections::HashMap;

use crate::vm::id::FnId;
use crate::vm::mir::{MirInst, TypeId};
use crate::vm::value::Value;

/// MIR 优化器 — 对 MIR 指令序列进行基本优化
pub fn optimize_mir(mir: &mut Vec<MirInst>) {
    // 多遍优化直到收敛。每遍运行所有优化（不要用 `||` 短路，否则后面的
    // 优化要等前面的优化不再变化才执行）。
    for _ in 0..3 {
        let mut changed = false;
        changed |= constant_folding(mir);
        changed |= fold_constant_jumps(mir);
        changed |= remove_noop_jumps(mir);
        changed |= peephole_optimize(mir);
        changed |= remove_dead_code(mir);
        changed |= eliminate_redundant_roll(mir);
        changed |= jump_optimization(mir);
        if !changed {
            break;
        }
    }
}

/// 收集「零参数、返回单个常量」的叶子函数：FnId → 常量值。
///
/// 判定时忽略调试标记（`Line`）、注释与作用域/标记指令，要求剩余指令恰为
/// `Push(常量)` + `Ret { argc: 1 }`。这类函数体没有副作用，可以安全内联。
pub fn constant_leaf_functions(functions: &[(FnId, Vec<MirInst>)]) -> HashMap<FnId, Value> {
    let mut constants = HashMap::new();
    for (fn_id, mir) in functions {
        let mut significant = mir.iter().filter(|inst| {
            !matches!(
                inst,
                MirInst::Line(_)
                    | MirInst::Comment(_)
                    | MirInst::EnterScope
                    | MirInst::ExitScope
                    | MirInst::Mark(_)
            )
        });
        let Some(MirInst::Push { value, .. }) = significant.next() else {
            continue;
        };
        if matches!(value, Value::CallElement { .. }) {
            continue;
        }
        let Some(MirInst::Ret { argc: 1 }) = significant.next() else {
            continue;
        };
        if significant.next().is_some() {
            continue;
        }
        constants.insert(*fn_id, value.clone());
    }
    constants
}

/// 把「零参数、返回单个常量」的叶子函数调用内联为 `Push(常量)`。
///
/// 生成期零参数调用先压 `CallElement { argc: 0 }` 再发 `Call`；命中常量叶子
/// 函数时把前者替换为常量 `Push` 并删除 `Call`，中间的 `Line`/`Comment` 保留。
pub fn inline_constant_calls(mir: &mut Vec<MirInst>, constants: &HashMap<FnId, Value>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < mir.len() {
        let is_zero_arg_call = matches!(
            &mir[i],
            MirInst::Push {
                value: Value::CallElement { argc: 0 },
                ..
            }
        );
        if !is_zero_arg_call {
            i += 1;
            continue;
        }
        let mut call_index = i + 1;
        while call_index < mir.len()
            && matches!(mir[call_index], MirInst::Line(_) | MirInst::Comment(_))
        {
            call_index += 1;
        }
        let target = match mir.get(call_index) {
            Some(MirInst::Call {
                fn_id,
                return_arity: 1,
            }) => *fn_id,
            _ => {
                i += 1;
                continue;
            }
        };
        let Some(value) = constants.get(&target).cloned() else {
            i += 1;
            continue;
        };
        mir[i] = MirInst::Push {
            value,
            ty: TypeId::Unknown,
        };
        mir.remove(call_index);
        changed = true;
        i += 1;
    }
    changed
}

/// 无副作用跳转消除：
/// - `UncondJump A; Mark A` → 删除跳转
/// - `Jump A; Mark A` → `Pop`（条件仍须弹出）
fn remove_noop_jumps(mir: &mut Vec<MirInst>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 1 < mir.len() {
        match (&mir[i], &mir[i + 1]) {
            (MirInst::UncondJump(target), MirInst::Mark(mark)) if target == mark => {
                mir.remove(i);
                changed = true;
                continue;
            }
            (MirInst::Jump(target), MirInst::Mark(mark)) if target == mark => {
                mir[i] = MirInst::Pop {
                    ty: TypeId::Unknown,
                };
                changed = true;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    changed
}

/// 常量条件跳转：`Push(c); Jump A` → `UncondJump A`（c != 0）或 `Pop`（c == 0）。
fn fold_constant_jumps(mir: &mut Vec<MirInst>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 1 < mir.len() {
        if let (MirInst::Push { value, .. }, MirInst::Jump(target)) = (&mir[i], &mir[i + 1])
            && let Some(condition) = value.as_i64()
        {
            if condition != 0 {
                mir[i] = MirInst::UncondJump(*target);
                mir.remove(i + 1);
            } else {
                mir[i] = MirInst::Pop {
                    ty: TypeId::Unknown,
                };
                mir.remove(i + 1);
            }
            changed = true;
            continue;
        }
        i += 1;
    }
    changed
}

/// 常量折叠: Push(a) Push(b) Add → Push(a+b)
fn constant_folding(mir: &mut Vec<MirInst>) -> bool {
    use crate::vm::value::Value;
    let mut changed = false;
    let mut i = 0;
    while i + 2 < mir.len() {
        if let (MirInst::Push { value: a, .. }, MirInst::Push { value: b, .. }, op) =
            (&mir[i], &mir[i + 1], &mir[i + 2])
        {
            let (Some(va), Some(vb)) = (a.as_i64(), b.as_i64()) else {
                i += 1;
                continue;
            };
            let result = match op {
                MirInst::Add { .. } => va.checked_add(vb).map(Value::I64),
                MirInst::Sub { .. } => va.checked_sub(vb).map(Value::I64),
                MirInst::Mul { .. } => va.checked_mul(vb).map(Value::I64),
                MirInst::Div { .. } => {
                    if vb == 0 {
                        None
                    } else {
                        va.checked_div(vb).map(Value::I64)
                    }
                }
                MirInst::Mod { .. } => {
                    if vb == 0 {
                        None
                    } else {
                        va.checked_rem(vb).map(Value::I64)
                    }
                }
                MirInst::Eq => Some(Value::Bool(va == vb)),
                MirInst::Ne => Some(Value::Bool(va != vb)),
                MirInst::Lt => Some(Value::Bool(va < vb)),
                MirInst::Gt => Some(Value::Bool(va > vb)),
                MirInst::Le => Some(Value::Bool(va <= vb)),
                MirInst::Ge => Some(Value::Bool(va >= vb)),
                MirInst::And => Some(Value::Bool(va != 0 && vb != 0)),
                MirInst::Or => Some(Value::Bool(va != 0 || vb != 0)),
                MirInst::Xor => Some(Value::Bool((va != 0) ^ (vb != 0))),
                _ => None,
            };
            if let Some(val) = result {
                mir[i] = MirInst::Push {
                    value: val,
                    ty: crate::vm::mir::TypeId::Unknown,
                };
                mir.remove(i + 1);
                mir.remove(i + 1); // 原来的 op（现在是 i+1）
                changed = true;
                continue;
            }
        }
        i += 1;
    }
    changed
}

/// 窥孔优化: Dup Pop → Nop; Roll(0) → Nop; Swap Swap → Nop
fn peephole_optimize(mir: &mut Vec<MirInst>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 1 < mir.len() {
        let remove = match (&mir[i], &mir[i + 1]) {
            (MirInst::Dup { .. }, MirInst::Pop { .. }) => {
                // Dup+Pop: 复制后立即丢弃 → 等同于 Nop，但保留 Pop
                // 实际应删除 Dup 和 Pop：mir.remove(i); mir.remove(i);
                true
            }
            (MirInst::Roll { n: 0 }, _) => {
                mir.remove(i);
                true
            }
            (MirInst::Swap, MirInst::Swap) => {
                mir.remove(i);
                mir.remove(i); // remove second Swap (now at i)
                true
            }
            _ => false,
        };
        if i + 1 < mir.len()
            && matches!(
                (&mir[i], &mir[i + 1]),
                (MirInst::Dup { .. }, MirInst::Pop { .. })
            )
        {
            mir.remove(i); // remove Dup
            mir.remove(i); // remove Pop
            changed = true;
            continue;
        }
        if remove {
            changed = true;
            continue;
        }
        i += 1;
    }
    changed
}

/// 死代码消除: 移除 Pop 后永不使用的 Push
fn remove_dead_code(mir: &mut Vec<MirInst>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 1 < mir.len() {
        if let (MirInst::Push { .. }, MirInst::Pop { .. }) = (&mir[i], &mir[i + 1]) {
            // Push 后立即 Pop → 死代码
            mir.remove(i);
            mir.remove(i); // remove Pop
            changed = true;
            continue;
        }
        i += 1;
    }
    changed
}

/// 冗余 Roll 消除: roll(n); roll(-n) → Nop（无副作用时）
fn eliminate_redundant_roll(mir: &mut Vec<MirInst>) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 1 < mir.len() {
        if let (MirInst::Roll { n: a }, MirInst::Roll { n: b }) = (&mir[i], &mir[i + 1])
            && *a == -*b
        {
            // roll(n); roll(-n) → Nop
            mir.remove(i);
            mir.remove(i);
            changed = true;
            continue;
        }
        i += 1;
    }
    changed
}

/// 跳转优化: Jump A; ... Mark A; UncondJump B → Jump B
fn jump_optimization(mir: &mut [MirInst]) -> bool {
    let mut changed = false;
    // 首先收集所有 Mark 位置
    let mut mark_positions: std::collections::HashMap<usize, usize> =
        std::collections::HashMap::new();
    for (i, inst) in mir.iter().enumerate() {
        if let MirInst::Mark(n) = inst {
            mark_positions.insert(*n, i);
        }
    }
    let mut i = 0;
    while i < mir.len() {
        if let MirInst::Jump(n) = &mir[i]
            && let Some(&target_pos) = mark_positions.get(n)
        {
            // 查找跳转目标后的指令
            if target_pos + 1 < mir.len()
                && let MirInst::UncondJump(m) = &mir[target_pos + 1]
            {
                // Jump A → ... Mark A; UncondJump B → 改为 Jump B
                mir[i] = MirInst::Jump(*m);
                changed = true;
            }
        }
        i += 1;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::value::Value;

    #[test]
    fn test_constant_folding_add() {
        let mut mir = vec![
            MirInst::Push {
                value: Value::I64(3),
                ty: crate::vm::mir::TypeId::Unknown,
            },
            MirInst::Push {
                value: Value::I64(5),
                ty: crate::vm::mir::TypeId::Unknown,
            },
            MirInst::Add {
                ty: crate::vm::mir::TypeId::Unknown,
            },
        ];
        optimize_mir(&mut mir);
        assert_eq!(mir.len(), 1);
        assert!(matches!(
            &mir[0],
            MirInst::Push {
                value: Value::I64(8),
                ..
            }
        ));
    }

    #[test]
    fn test_peephole_roll_zero() {
        let mut mir = vec![
            MirInst::Roll { n: 0 },
            MirInst::Push {
                value: Value::I64(1),
                ty: crate::vm::mir::TypeId::Unknown,
            },
        ];
        optimize_mir(&mut mir);
        assert_eq!(mir.len(), 1);
    }

    #[test]
    fn test_eliminate_push_pop() {
        let mut mir = vec![
            MirInst::Push {
                value: Value::I64(42),
                ty: crate::vm::mir::TypeId::Unknown,
            },
            MirInst::Pop {
                ty: crate::vm::mir::TypeId::Unknown,
            },
        ];
        optimize_mir(&mut mir);
        assert!(mir.is_empty());
    }

    #[test]
    fn test_redundant_roll() {
        let mut mir = vec![
            MirInst::Roll { n: 3 },
            MirInst::Roll { n: -3 },
            MirInst::Push {
                value: Value::I64(1),
                ty: crate::vm::mir::TypeId::Unknown,
            },
        ];
        optimize_mir(&mut mir);
        assert_eq!(mir.len(), 1);
    }

    #[test]
    fn test_fold_constant_jump_true() {
        let mut mir = vec![
            MirInst::Push {
                value: Value::I64(1),
                ty: TypeId::Unknown,
            },
            MirInst::Jump(7),
        ];
        optimize_mir(&mut mir);
        assert_eq!(mir.len(), 1);
        assert!(matches!(&mir[0], MirInst::UncondJump(7)));
    }

    #[test]
    fn test_fold_constant_jump_false() {
        let mut mir = vec![
            MirInst::Push {
                value: Value::I64(0),
                ty: TypeId::Unknown,
            },
            MirInst::Jump(7),
        ];
        optimize_mir(&mut mir);
        assert_eq!(mir.len(), 1);
        assert!(matches!(&mir[0], MirInst::Pop { .. }));
    }

    #[test]
    fn test_remove_noop_unconditional_jump() {
        let mut mir = vec![MirInst::UncondJump(3), MirInst::Mark(3)];
        optimize_mir(&mut mir);
        assert_eq!(mir.len(), 1);
        assert!(matches!(&mir[0], MirInst::Mark(3)));
    }

    #[test]
    fn test_fold_comparison() {
        let mut mir = vec![
            MirInst::Push {
                value: Value::I64(3),
                ty: TypeId::Unknown,
            },
            MirInst::Push {
                value: Value::I64(5),
                ty: TypeId::Unknown,
            },
            MirInst::Lt,
        ];
        optimize_mir(&mut mir);
        assert_eq!(mir.len(), 1);
        assert!(matches!(
            &mir[0],
            MirInst::Push {
                value: Value::Bool(true),
                ..
            }
        ));
    }

    #[test]
    fn test_line_marker_is_preserved() {
        let mut mir = vec![
            MirInst::Line(7),
            MirInst::Push {
                value: Value::I64(1),
                ty: TypeId::Unknown,
            },
            MirInst::Push {
                value: Value::I64(2),
                ty: TypeId::Unknown,
            },
            MirInst::Add {
                ty: TypeId::Unknown,
            },
        ];
        optimize_mir(&mut mir);
        assert!(matches!(mir[0], MirInst::Line(7)));
        assert_eq!(mir.len(), 2);
    }

    #[test]
    fn inline_constant_calls_replaces_zero_arg_call() {
        let mut caller = vec![
            MirInst::Line(3),
            MirInst::Push {
                value: Value::CallElement { argc: 0 },
                ty: TypeId::Unknown,
            },
            MirInst::Call {
                fn_id: FnId(2),
                return_arity: 1,
            },
        ];
        let mut constants = HashMap::new();
        constants.insert(FnId(2), Value::I64(7));
        assert!(inline_constant_calls(&mut caller, &constants));
        assert_eq!(caller.len(), 2);
        assert!(matches!(caller[0], MirInst::Line(3)));
        assert!(matches!(
            &caller[1],
            MirInst::Push {
                value: Value::I64(7),
                ..
            }
        ));
    }

    #[test]
    fn constant_leaf_functions_only_accepts_constant_bodies() {
        let functions = vec![
            (
                FnId(1),
                vec![
                    MirInst::Line(1),
                    MirInst::EnterScope,
                    MirInst::Push {
                        value: Value::I64(5),
                        ty: TypeId::Unknown,
                    },
                    MirInst::Ret { argc: 1 },
                ],
            ),
            (
                FnId(2),
                vec![
                    MirInst::Push {
                        value: Value::I64(1),
                        ty: TypeId::Unknown,
                    },
                    MirInst::Push {
                        value: Value::I64(2),
                        ty: TypeId::Unknown,
                    },
                    MirInst::Add {
                        ty: TypeId::Unknown,
                    },
                    MirInst::Ret { argc: 1 },
                ],
            ),
            (
                FnId(3),
                vec![
                    MirInst::Push {
                        value: Value::I64(0),
                        ty: TypeId::Unknown,
                    },
                    MirInst::Ret { argc: 0 },
                ],
            ),
        ];
        let constants = constant_leaf_functions(&functions);
        assert_eq!(constants.get(&FnId(1)), Some(&Value::I64(5)));
        assert!(!constants.contains_key(&FnId(2)));
        assert!(!constants.contains_key(&FnId(3)));
    }
}
