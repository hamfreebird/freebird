# Freebird 实现状态 — 深度审计报告

> **审计日期**: 2026-06-03
> **审计范围**: 40 个源文件，~10,500 行 Rust 代码，逐文件逐行检查
> **发现**: 15 个严重 bug，35 个高优先级问题，50+ 个简化/占位实现，20+ 个缺失特性
> **测试**: 130 个全部通过（但多个测试仅检查 is_ok()，未验证正确性）

---

## 目录

1. [严重 Bug — 执行正确性受损](#1-严重-bug--执行正确性受损)
2. [高优先级 Bug — 数据丢失/静默错误](#2-高优先级-bug--数据丢失静默错误)
3. [MIR 生成器问题](#3-mir-生成器问题)
4. [VM 运行时问题](#4-vm-运行时问题)
5. [Stardust 代码生成器问题](#5-stardust-代码生成器问题)
6. [解析器问题](#6-解析器问题)
7. [HIR + 借用检查器问题](#7-hir--借用检查器问题)
8. [类型系统问题](#8-类型系统问题)
9. [IO / REPL / 诊断问题](#9-io--repl--诊断问题)
10. [死代码清单](#10-死代码清单)
11. [硬编码值清单](#11-硬编码值清单)
12. [测试覆盖率缺口](#12-测试覆盖率缺口)
13. [已完成的修复（本审计后）](#13-已完成的修复)
14. [项目整体评估](#14-项目整体评估)

---

## 1. 严重 Bug — 执行正确性受损

### BUG-1: if-else 控制流反向 (mir/generator.rs:443)

```rust
// 生成 Jump(else_label) — 但 VM 的 Jump 在条件为真时跳转
self.mir.push(MirInst::Jump(else_label));
```

注释写 "条件为假时跳到 else"，但 VM 的 `Jump` 指令在弹出条件**非零（真）**时跳转。结果：`if true { A } else { B }` 执行分支 B 而非 A。While 循环正确插入了 `Not` 取反，但 if-else 缺失。

**修复**: 在 `Jump(else_label)` 前插入 `MirInst::Not`。

### BUG-2: 5 个指令处理器中 roll-back 算法错误 (vm/mod.rs:313-315, 330-332, 348-350, 374-376, 841-843)

模式 `roll(pos); dup; roll(-pos)` 在所有处理器中重复出现（Deref、FieldAccess、IndexAccess、EnumTag、execute_read_var）。`dup` 后栈增长1，但 `roll(-pos)` 仍用原始 pos，导致元素恢复错误位置。

```
栈 [A,B,C,D]，find_id(D)=pos=2:
  roll(2) → [A,C,D,B]
  dup     → [A,C,D,B,B']
  roll(-2)→ [A,C,B',D,B]     ← 错误！应为 [A,B,C,D,B']
```

`test_read_var_does_not_change_depth` 仅检查长度 (4)，不检查顺序，故测试通过而 bug 存在。

**修复**: 使用直接索引访问而非 roll 序列，或调整 `roll(-pos)` 为 `roll(-(pos+1))`。

### BUG-3: execute_write_var 回滚越界 (vm/mod.rs:867-873)

`roll(pos); pop; roll(-pos)`: pop 后栈缩短，`roll(-pos)` 在 pos 为原始栈底时越界。

```
栈 [old, new], pos=1:
  roll(1) → [new, old]
  pop     → [new]          len=1
  roll(-1)→ 越界！pos=1 >= len=1
```

**修复**: pop 后检查 `pos < frame.data.len()`，仅在有效时 roll-back。

### BUG-4: Break 仅生成注释 (mir/generator.rs:496-503)

```rust
HirExpr::Break(value, _) => {
    self.mir.push(MirInst::Comment("break".to_string()));
}
```

While/Loop 的 `loop_end` label 是局部变量，Break 无法访问。执行时直接 fall-through 到下一条指令。

### BUG-5: Enum::Variant 无数据（如 Option::None）被解析器静默丢弃 (parser.rs:323-346)

ColonColon 处理器进入分支，消耗 variant 名，检查 LParen。若无 LParen，整个 if-let 链 fall-through，variant 名被消耗但未纳入任何 AST 节点。`Option::None` 变成 `Var("Option")`。

### BUG-6: resolve_field_offset 逻辑完全错误 (mir/generator.rs:682-702)

函数遍历 TypeTable 寻找字段名==变量名的结构体。这混淆了变量名与字段名。所有 `FieldAccess` 实际返回 offset 0（fallback 701行），访问第一个字段。

### BUG-7: infer_enum_name 对多枚举程序错误 (mir/generator.rs:653-667)

HashMap 迭代顺序任意，取第一个枚举。多枚举程序大概率选错。fallback 硬编码 "Option"。

### BUG-8: Match 标签值泄漏到 arm body (mir/generator.rs:610-618)

`Dup; Push(vtag); Eq; Jump` 后，Dup 引入的 tag 副本残留在栈上。arm body 获得意外的额外栈元素。

### BUG-9: Call 使用 FnId(0) 占位 (mir/generator.rs:421, 822)

未知函数名或 stardust! 宏中的 Call 使用 `FnId(0)`。`FnId(0)` 从未注册，VM 执行时必然失败。

### BUG-10: Stardust 算术在操作前 pop 所有操作数 (codegen/stardust.rs:87-95, 172-188)

```rust
"Pop Pop  // discard rhs id,value"
"Swap Pop Pop Swap  // discard lhs id,value"
"Add"  // 此时栈为空！
```

Pop 后栈空，Add 无操作数。整个 Stardust 算术生成逻辑错误。

### BUG-11: Let 无 init 时 BindVar 操作空栈 (mir/generator.rs:311-319)

`let x;`（无初始值）：init 为 None，不 push 值，但 BindVar 仍发射。VM 调用 `top_id()` 在空栈 → StackUnderflow。

### BUG-12: f32/f64 字面量后缀丢失浮点语义 (lexer.rs:503-506)

`1f32` 被词法为 `IntLiteral(1, IntSuffix::I32)`，浮点语义完全丢失。

### BUG-13: 空 16/8/2 进制字面量不推入 errors (lexer.rs:433, 449, 465)

`0x`/`0o`/`0b` 产生 Error token 但不追加到 `self.errors`。与 10 进制路径不一致。

### BUG-14: f32::NAN as i64 平台依赖 (value.rs:65-66)

`F32(f) => Some(*f as i64)`: NaN/Inf 到 i64 的转换结果因平台而异。

### BUG-15: read_line 根本性损坏 (io.rs:57-68)

分配远程栈 → 读1字符 → 不写入 → 不循环 → 不释放。完全不工作。

---

## 2. 高优先级 Bug — 数据丢失/静默错误

### BUG-H1: 属性被静默丢弃 (parser.rs:38, 96)

`let _attrs = self.parse_attributes()` 解析后丢弃。所有 AST 节点的 `attrs: vec![]` 始终为空。

### BUG-H2: 赋值仅支持简单变量 (parser.rs:256-257)

`x.y = 5`、`arr[0] = 5`、`*ptr = 5` 均被拒绝/误报。

### BUG-H3: filter_map 在 5 处静默吞没错误 (lowering.rs:325, 353, 357, 389, 433)

Call参数、元组元素、数组元素、match臂、返回值中的子表达式降级失败时，错误被 `filter_map` 丢弃。

### BUG-H4: 错误类型误用 (lowering.rs:218, 314)

不可变赋值报 `UndefinedVariable`；参数个数不匹配报 `UndefinedVariable`。语义完全错误。

### BUG-H5: IdRef 空体 (lowering.rs:276-282)

`resolve_var` 失败时，注释说需检查但代码体为空，错误被吞没。

### BUG-H6: max_stack 配置永不被检查 (config.rs:5, mod.rs 无引用)

默认值 1,048,576，但 push/frame创建从未验证。程序可无限 push 直到 OOM。

### BUG-H7: 算术运算无溢出检查 (vm/mod.rs:169-179)

Debug 模式 panic，Release 模式静默 wrap。无 checked_add 等安全操作。

### BUG-H8: MakeEnum 泄漏 ElementIds (vm/mod.rs:362-363)

`let _new_id = self.id_alloc.alloc_element()` 分配后丢弃。每次 MakeEnum 消耗一个永久 ID。

### BUG-H9: Jump 非整数条件静默强制为 false (vm/mod.rs:228)

`cond.as_i64().unwrap_or(0)` — StackHandle/Ref/FnPtr 等非整数条件静默视为假。

### BUG-H10: CallArg::Pos span 始终错误 (parser.rs:664-669)

使用 `self.current_span()`（下一 token 的位置）而非已解析表达式的 span。

### BUG-H11: 未初始化变量被认为已初始化 (resolver.rs:123-124, lowering.rs:193)

`let x: i32;`（无 init）仍被 mark_initialized，后续读取不报错。

### BUG-H12: VarStack.insert 在空帧上静默丢弃数据 (var_stack.rs:42)

若未调用 enter_scope，insert 不存储也不报错。

### BUG-H13: RemotePop 空栈返回 Unit 不报错 (vm/mod.rs:469-471)

与所有其他 StackUnderflow 约定不一致。

### BUG-H14: 返回值的 ReturnArityMismatch 为死代码 (error.rs:32)

execute_call 硬编码 `return_arity: 0`，此错误永不被构造。

### BUG-H15: REPL 使用测试配置限制用户程序 (repl.rs:26)

`RuntimeConfig::test()`（max_stack=1024, max_call_depth=64）用于生产 REPL。

---

## 3. MIR 生成器问题

| # | 位置 | 问题 |
|---|------|------|
| G1 | generator:284-291 | `collect_drop_vars` 返回空 Vec，Drop 代码永不被生成 |
| G2 | generator:309-320 | 非 Var 的解构模式（Tuple/Struct/Enum）静默丢弃 |
| G3 | generator:525-528 | Tuple 表达式无 MakeTuple 指令，仅独立 push 元素 |
| G4 | generator:530-533 | Array 表达式无 MakeArray 指令 |
| G5 | generator:555-567 | StructLit 缺失字段时静默产生错误栈布局 |
| G6 | generator:584 | data_slots 始终为1，不处理多元组数据变体 |
| G7 | generator:447 | 死代码 Mark(else_label+1000)，可能碰撞合法 label |
| G8 | generator:424-435 | stack_pass 模式参数清理跳过，tracker 残留幽灵槽 |
| G9 | generator:241-253 | StackTracker 不追踪 CallElement（VM栈有CE，tracker无） |
| G10 | generator:493 | Ret 在 tracker 中保留了错误元素（应保留返回值但丢弃了它们） |
| G11 | generator:512-516 | 复杂 Ref 表达式仅注释，不产生实际引用 |
| G12 | generator:643-645 | 未处理 HirExpr 变体产生 TODO Comment，静默忽略 |

---

## 4. VM 运行时问题

| # | 位置 | 问题 |
|---|------|------|
| V1 | mod.rs:785-793 | 主程序返回值静默丢弃，无 API 检索 |
| V2 | mod.rs:788 | `saturating_sub(argc)` 当 argc>len 时截断整个栈，不报错 |
| V3 | mod.rs:534-539 | SwitchToRemote/SwitchToLocal 为 no-op 占位 |
| V4 | config.rs:5 | `max_call_depth` 在 run() 循环检查但 max_stack 无检查 |
| V5 | stack.rs:84-101 | roll 的 O(n) remove/insert，大栈性能问题 |
| V6 | id.rs:52-60 | derive_stack 碰撞域 2^63（高位置1），无去重 |
| V7 | value.rs:120-121 | is_copy_type() 对 Ref/MutRef 返回 true（命名混淆） |
| V8 | frame.rs:113-117 | jump_to_mark() 死代码，从未被调用 |
| V9 | registry.rs:24 | alloc_remote 未使用 _call_id 参数 |

---

## 5. Stardust 代码生成器问题

| # | 位置 | 问题 |
|---|------|------|
| S1 | stardust.rs:124-132 | Mark/Jump/UncondJump/Call/Ret 全部仅注释 |
| S2 | stardust.rs:135-137 | BindVar/ReadVar/WriteVar 仅注释 |
| S3 | stardust.rs:138-166 | 所有复合类型、远程栈、I/O 操作仅注释 |
| S4 | stardust.rs:38-42 | Push 对 id 和 value 使用相同值 |
| S5 | stardust.rs:54-73 | Roll 槽乘数应为 4 而非 2 |
| S6 | stardust.rs:80-86 | Pick 处理器为死代码（生成器不发射 Pick） |
| S7 | stardust.rs:74-79 | Swap 的 ShiftL×4 + ShiftR×2 语义存疑 |

**结论**: Stardust 后端仅 ~12/45 MIR 指令生成实际代码，其余全部为注释。生成的 Stardust 源码无法独立执行。

---

## 6. 解析器问题

| # | 位置 | 问题 |
|---|------|------|
| P1 | parser:323-346 | Enum::Variant 无数据时静默丢弃 variant (BUG-5) |
| P2 | parser:38,96 | 属性被解析后丢弃 |
| P3 | parser:256-257 | 仅 Var 类型赋值有效 |
| P4 | parser:244-243 | use 不支持通配符/重命名/嵌套 |
| P5 | parser:108-109 | FnParam 不支持 mut 或 _ |
| P6 | token:28,60 | BoolLiteral 和 ElementId 为死代码（从不产生） |
| P7 | token:127-141 | IntSuffix::parse 对未知后缀静默返回 None |
| P8 | lexer:475-509 | 贪婪字母数字后缀消费，吞没后续标识符字符 |
| P9 | lexer:375 | 不支持前导点/尾随点浮点数（.5, 1.） |
| P10 | lexer:383-394 | 科学计数法仅在小数点后有效（1e5 不解析） |
| P11 | error:30-36 | DuplicateField/UnknownAttribute/MissingAttributeArg 从不被构造 |
| P12 | mod.rs:14-27 | parse() 在词法错误后仍运行解析器 |
| P13 | ast:16,52,99 | Type::FnPtr/Pattern::Or/UnOp::Deref 为死代码变体 |

---

## 7. HIR + 借用检查器问题

| # | 位置 | 问题 |
|---|------|------|
| H1 | types:30 | HirType::Array 缺少大小维度 |
| H2 | types:28 | HirType::Ref 无生命周期参数 |
| H3 | types:33 | HirType::FnPtr 无 ABI/调用约定字段 |
| H4 | types:153-157 | HirStmt::Assign 仅支持简单变量目标 |
| H5 | lowering:236-242 | 复杂赋值目标被拒绝 |
| H6 | lowering:265 | scope_depth=usize::MAX 哨兵未文档化 |
| H7 | lowering:312-313 | stack_pass 绕过参数计数验证 |
| H8 | lowering:131-133 | 函数参数全部不可变（mutable=false 硬编码） |
| H9 | lowering:455-460 | pattern_var_name 仅处理 Var 模式 |
| H10 | resolver:94-103 | declare_var 在无作用域时静默无操作 |
| H11 | resolver:152 | register_fn 允许重复函数名 |
| H12 | resolver:141-148 | is_mutable 返回 None 时调用者未处理 |
| H13 | checker:84-87 | Move 类型硬编码（Vec/String/Box/StackHandle），用户类型不可扩展 |
| H14 | checker:141-144 | 重复变量报 CannotAssignToImmutable（错误类型） |
| H15 | checker:160-163 | 未定义变量报 UseAfterMove（错误类型） |
| H16 | checker:121-129 | drop_order 返回所有作用域变量而非仅当前作用域 |
| H17 | drop_gen:14-18 | 无条件 FreeRemote，不检查是否实际需要 |
| H18 | resolver:27 | Binding 无类型字段，丢弃所有声明类型信息 |

---

## 8. 类型系统问题

| # | 位置 | 问题 |
|---|------|------|
| T1 | type_table:74,78 | slots 始终为 1（不处理嵌套/展开槽） |
| T2 | type_table:85 | all_copy 始终为 true（含非 Copy 字段亦然） |
| T3 | type_table:102 | data_slots 始终为 1（不处理多槽数据变体） |
| T4 | type_table:148 | field_offset 返回索引而非实际偏移量 |
| T5 | type_table:205 | 泛型类型参数被丢弃 |
| T6 | type_stack:124 | merge 错误 span 始终为 (0,0,0) |
| T7 | type_stack:91-112 | roll 在 n=i32::MIN 时溢出（-n 在 debug 模式 panic） |
| T8 | type_stack:84-89 | dup 在空栈上静默无操作 |
| T9 | 全局 | TypeStack 从未被集成到流水线，完全是死代码 |
| T10 | 全局 | 无类型检查 pass，类型错误仅在运行时暴露 |

---

## 9. IO / REPL / 诊断问题

| # | 位置 | 问题 |
|---|------|------|
| I1 | io:57-68 | read_line 损坏 (BUG-15) |
| I2 | io:70-82 | file_read/file_write 为注释占位 |
| I3 | io:86-93 | print_str 未包含在 std_library 公开列表中 |
| I4 | io:37-41 | print_str 无条件追加换行 |
| R1 | repl:26 | 使用 RuntimeConfig::test() 限制生产使用 |
| R2 | repl:81-85 | 声明检测脆弱（仅匹配 "fn "） |
| R3 | repl:95-99 | 每行重新解析所有声明（O(n²)） |
| R4 | repl:129-131 | 函数注册表泄漏（旧定义不清理） |
| R5 | repl:133 | VM 状态跨行不重置（状态污染） |
| R6 | repl:136 | 通过公共字段直接访问 VM 内部 |
| D1 | diagnostics:148 | JSON 序列化失败返回无效 {}（非 DiagnosticReport） |
| D2 | diagnostics:46-54 | SourceSpan 缺少 end_line/end_column |
| D3 | diagnostics:15-19 | Warning/Note/Help 级别永不被生成 |
| D4 | main.rs:88-92 | Check 命令绕过 check_source()，重复逻辑 |
| D5 | main.rs:131 | 阶段完成声明过时（"Phases 0-10"） |

---

## 10. 死代码清单

| 文件 | 行 | 项目 |
|------|-----|------|
| token.rs | 28 | `TokenKind::BoolLiteral` — lexer 从不产生 |
| token.rs | 60 | `TokenKind::ElementId` — lexer 从不产生 |
| ast.rs | 16 | `Type::FnPtr` — parser 从不构造 |
| ast.rs | 52 | `Pattern::Or` — parser 从不构造 |
| ast.rs | 99 | `UnOp::Deref` — parser 用 Expr::Deref 代替 |
| error.rs | 30-36 | DuplicateField/UnknownAttribute/MissingAttributeArg — 从不构造 |
| frame.rs | 113 | `jump_to_mark()` — 从不调用 |
| stack.rs | 168 | `top_value_mut()` — 从不调用 |
| var_stack.rs | 52 | `lookup_with_depth()` — 仅测试使用 |
| var_stack.rs | 74 | `is_bound()` — 从不调用 |
| error.rs(vm) | 32 | `ReturnArityMismatch` — 从不构造 |
| type_stack.rs | 全部 | `TypeStack` — 从未集成 |
| stardust.rs | 80-86 | `Pick` 处理器 — 生成器不发射 |
| generator.rs | 151 | `active_vars` 字段 #[allow(dead_code)] |
| lib.rs | 22 | `check_source` — CLI 不使用 |

---

## 11. 硬编码值清单

| 文件 | 行 | 值 | 说明 |
|------|-----|-----|------|
| generator | 584 | `data_slots = 1` | 始终1槽 |
| generator | 662 | `"Option"` | 枚举名fallback |
| generator | 672 | `2` | total_slots默认值 |
| generator | 701 | `0` | 字段偏移fallback |
| generator | 421 | `FnId(0)` | 未知函数 |
| generator | 822 | `FnId(0)` | stardust调用 |
| generator | 790,794 | `index: 0` | Remote Store/Load |
| generator | 447 | `else_label + 1000` | 脆弱算术 |
| optimizer | 6 | `3` | 优化遍数上限 |
| stardust | 51,93 | `1` | 新ID占位 |
| stardust | 58 | `* 2` | 应为*4 |
| type_table | 74 | `slots: 1` | 字段槽数 |
| type_table | 85 | `all_copy: true` | 复制语义 |
| type_table | 102 | `data_slots: 1` | 枚举数据槽 |
| checker | 84-87 | `Vec/String/Box` | Move类型列表 |
| lowering | 132 | `false` | 参数mutable |
| io | 64 | `256` | read_line容量 |
| repl | 26 | `RuntimeConfig::test()` | REPL配置 |
| pipeline | 47 | 索引匹配 | 脆弱假设 |

---

## 12. 测试覆盖率缺口

### 12.1 错误路径未覆盖
- 所有 BorrowError 触发条件未测试
- 所有 VmError 变体（除 StackUnderflow）未独立测试
- 解析错误恢复路径未系统测试
- Unicode/空字符串/极大数字边界条件未测试

### 12.2 值断言缺失
- `test_if_else` 仅检查 is_ok()，不验证执行了正确分支
- `test_fibonacci` 不检查返回值是否为正确的斐波那契数
- `test_while_loop` 不检查循环后变量值
- `test_struct_construction` 不检查字段布局

### 12.3 边界条件
- 空栈上所有指令
- 单元素栈 roll/swap/dup
- 最大深度栈
- 远程栈容量边界（刚好满、恰好扩容）
- 负数/零除数
- 超大/超小整数

### 12.4 缺失的测试类别
- Stardust 代码生成快照测试
- 模糊测试（随机 MIR 序列）
- 长时间运行稳定性测试
- 性能回归测试
- 跨平台确定性测试（f32::NAN → i64）

---

## 13. 已完成的修复

以下问题已在本审计之前/期间修复：

| 问题 | 修复 |
|------|------|
| EnumTag 硬编码 2 槽 | 添加 total_slots 字段 |
| Match 硬编码 "Option" | infer_enum_name 从 TypeTable 推断 |
| 跳转优化缺失 | 添加 jump_optimization 遍 |
| TypeStack 缺失 | 创建 typeck/type_stack.rs |
| 集成测试缺失 | 创建 tests/ + 4 参考程序 |
| hello.fb 缺失 | 创建 examples/hello.fb |
| TypeTable 无迭代器 | 添加 iter() 方法 |

---

## 14. 项目整体评估

### 14.1 完成度矩阵

| 组件 | 完整度 | 最严重问题 |
|------|--------|-----------|
| Lexer | 85% | 空字面量不报错，f32后缀丢失 |
| Parser | 70% | Enum::None丢失，属性丢弃，赋值受限 |
| HIR 降级 | 60% | 5处filter_map吞错误，错误类型误用 |
| 名称解析 | 75% | 未初始化变量误标已初始化 |
| 借用检查 | 40% | 仅name-based，用户类型不可扩展 |
| 类型系统 | 25% | TypeStack未集成，slots/all_copy硬编码 |
| MIR 生成 | 55% | if-else反向，Break不工作，字段偏移错误 |
| VM 核心 | 70% | roll-back算法全部错误，无溢出检查 |
| 远程栈 | 75% | RemotePop空栈不报错 |
| Stardust | 15% | 仅12/45指令有代码，Call/Ret仅注释 |
| IO 库 | 20% | read_line损坏，file I/O占位 |
| REPL | 60% | 状态污染，测试配置限制，O(n²) |
| 诊断 | 50% | Warning/Note/Help未使用，span不完整 |
| 测试 | 45% | 缺少值断言、错误路径、边界测试 |
| **整体** | **~55%** | — |

### 14.2 优先修复顺序

1. **B-1**: if-else 反向 → 1行修复 (加 Not)
2. **B-2/B-3**: roll-back 算法 → 统一改为直接索引访问
3. **B-4**: Break 不工作 → 传递 loop_end label
4. **B-5**: Enum::None 丢失 → 解析器 ColonColon 修复
5. **G1**: Drop 代码生成 → 连接 DropGen 到 gen_block
6. **T9**: TypeStack 集成 → 在 MIR 生成前调用
7. **S1-S3**: Stardust 控制流 → 生成实际 Stardust 指令
8. **B-11**: Let 无 init BindVar → 生成缺失值的错误
9. **BUG-H3**: filter_map 吞错误 → 改为 collect+检查
10. **I1**: read_line 修复 → 实现循环逻辑

---

> 自由如鸟，尽在栈中。
> Free as a bird, everything is a stack.
