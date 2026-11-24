use crate::vm::mir::MirInst;

/// MIR 优化器 — 对 MIR 指令序列进行基本优化
pub fn optimize_mir(mir: &mut Vec<MirInst>) {
    // 多遍优化直到收敛
    for _ in 0..3 {
        let changed = constant_folding(mir)
            || peephole_optimize(mir)
            || remove_dead_code(mir)
            || eliminate_redundant_roll(mir)
            || jump_optimization(mir);
        if !changed {
            break;
        }
    }
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
            let result = match op {
                MirInst::Add { .. } => {
                    if let (Some(va), Some(vb)) = (a.as_i64(), b.as_i64()) {
                        Some(Value::I64(va + vb))
                    } else {
                        None
                    }
                }
                MirInst::Sub { .. } => {
                    if let (Some(va), Some(vb)) = (a.as_i64(), b.as_i64()) {
                        Some(Value::I64(va - vb))
                    } else {
                        None
                    }
                }
                MirInst::Mul { .. } => {
                    if let (Some(va), Some(vb)) = (a.as_i64(), b.as_i64()) {
                        Some(Value::I64(va * vb))
                    } else {
                        None
                    }
                }
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
        if let (MirInst::Roll { n: a }, MirInst::Roll { n: b }) = (&mir[i], &mir[i + 1]) {
            if *a == -*b {
                // roll(n); roll(-n) → Nop
                mir.remove(i);
                mir.remove(i);
                changed = true;
                continue;
            }
        }
        i += 1;
    }
    changed
}

/// 跳转优化: Jump A; ... Mark A; UncondJump B → Jump B
fn jump_optimization(mir: &mut Vec<MirInst>) -> bool {
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
        if let MirInst::Jump(n) = &mir[i] {
            if let Some(&target_pos) = mark_positions.get(n) {
                // 查找跳转目标后的指令
                if target_pos + 1 < mir.len() {
                    if let MirInst::UncondJump(m) = &mir[target_pos + 1] {
                        // Jump A → ... Mark A; UncondJump B → 改为 Jump B
                        mir[i] = MirInst::Jump(*m);
                        changed = true;
                    }
                }
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
}
