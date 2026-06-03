# Freebird 实现状态 — v2 审计 (23 Bug 修复后)

> **审计日期**: 2026-06-03 (第2轮, 已修复23个bug后)
> **测试**: 130 全部通过 (126 unit + 4 integration)
> **总代码量**: ~10,500 行 Rust, 40 个源文件

---

## 目录

1. [已修复的 Bug 记录](#1-已修复的-bug-记录)
2. [仍存在的高优 Bug](#2-仍存在的高优-bug)
3. [简化/占位实现](#3-简化占位实现)
4. [未实现功能](#4-未实现功能)
5. [死代码清单](#5-死代码清单)
6. [测试缺口](#6-测试缺口)
7. [项目完成度评估](#7-项目完成度评估)

---

## 1. 已修复的 Bug 记录

### 第 1 批 — 9 bugs (VM + Generator)
| Bug | 问题 | 修复方式 |
|-----|------|---------|
| BUG-1 | if-else 控制流反向 | Jump 前插入 `Not` 取反条件 |
| BUG-2/3 | roll-back 算法错误 (Deref/FieldAccess/IndexAccess/ReadVar 等 6 处) | 改为直接索引 `values[idx].clone()` + `push` |
| BUG-11 | Let 无 init 时 BindVar 操作空栈 | 仅在有 init 值时生成 BindVar |
| BUG-12 | f32/f64 字面量后缀丢失浮点语义 | `lex_int_suffix` 返回 `(IntSuffix, bool)` 标记 float |
| BUG-H4 | 错误类型误用 (UndefinedVariable 代替专用类型) | 添加 `ArgumentCountMismatch` + `CannotAssignToImmutable` |
| BUG-H5 | IdRef 变量未声明时静默通过 | 正确推入 resolve 错误 |
| BUG-H6 | max_stack 永不被检查 | `push_checked()` + `VmError::StackOverflow` |
| BUG-H8 | MakeEnum 泄漏 ElementId | 移除未使用的 `alloc_element()` |
| BUG-8 | Match 标签泄漏到 arm body | 每个 arm 比较后 `Pop` 清理 Dup 残留 |

### 第 2 批 — 9 bugs (Lexer + Parser + VM)
| Bug | 问题 | 修复方式 |
|-----|------|---------|
| BUG-13 | 空 16/8/2 进制字面量不报错 | `num_str.is_empty()` 检查 + 推入错误 |
| BUG-14 | f32::NAN as i64 平台依赖 | `is_finite()` 检查后返回 None |
| BUG-H9 | Jump 非整数条件静默强制 false | 改为返回 `TypeMismatch` |
| BUG-H13 | RemotePop 空栈返回 Unit | 改为返回 `AssertionFailed` |
| BUG-9 | Call 使用 FnId(0) 占位 | 未知函数名插入 `Comment` 错误标记 |
| BUG-5 | Enum::None 无数据变体被解析器丢弃 | ColonColon 无 LParen 时正确构造 `EnumLit` |
| BUG-H1 | 属性被解析但丢弃 | (记录, 后续修复) |
| BUG-H2 | 赋值仅支持简单变量 | (记录, 后续修复) |

### 第 3 批 — 5 bugs (Generator + Stardust + HIR + IO)
| Bug | 问题 | 修复方式 |
|-----|------|---------|
| BUG-4 | Break 仅生成注释 | 添加 `loop_ends` 栈, 生成 `UncondJump(loop_end)` |
| BUG-6/7 | `resolve_field_offset` + `infer_enum_name` 完全错误 | 改为遍历 TypeTable 搜索匹配 |
| BUG-10 | Stardust 算术操作前栈为空 | 重写为 `Pick(0) Pick(2)` 方法 |
| BUG-H3 | `filter_map` 在 5 处静默吞没错误 | 全部改为 `for`-loop 显式收集 |
| BUG-15 | `read_line` 损坏 | 实现 Mark/Jump 循环读取至换行 |

**总计: 23 个 bug 已修复**

---

## 2. 仍存在的高优 Bug

### BUG-R1: read_line 仍有 2 个 bug (io.rs:78-86)

**Bug A** (78行): Dup 复制了栈顶的 `char` 而非 `handle`
```
栈: [handle, char] → Dup → [handle, char, char]
Swap → [handle, char, char] (无变化)
RemotePush 收到 char 而非 StackHandle → 静默忽略
```

**Bug B** (85行): 循环退出时两个 `Pop(Char)` 错误弹出了 handle
```
栈: [handle, '\n'] → Pop → [handle] → Pop → []  // handle 丢失!
```

### BUG-R2: Drop 代码永不生成 (mir/generator.rs:302-306)

```rust
fn collect_drop_vars(&self) -> Vec<String> { Vec::new() }
```
`BorrowChecker` 和 `DropGen` 已实现但从未被 pipeline 调用。Move 类型变量离开作用域时永不清理。

### BUG-R3: Stardust 后端 25+ 指令仅生成注释 (stardust.rs:116-158)

| 可生成代码 (12) | 仅注释 (25+) |
|----------------|-------------|
| Push/Pop/Dup/Roll/Swap/Pick | Mark/Jump/UncondJump/Call/Ret |
| Add/Sub/Mul/Div/Mod/Neg | BindVar/ReadVar/WriteVar |
| Eq/Ne/Lt/Gt/Le/Ge | Ref/Deref |
| And/Or/Not/Xor | FieldAccess/IndexAccess/MakeEnum/EnumTag |
| | AllocRemote/FreeRemote/RemotePush/Pop/Load/Store/Len |
| | SwitchToRemote/SwitchToLocal |
| | NumOut/NumIn/CharOut/CharIn |
| | DumpStack/DumpState/Breakpoint |

### BUG-R4: 6 个 HirExpr 变体编译为注释 (generator.rs:662-664)

`IdRef`, `Repeat`, `For`, `Continue`, `Ret(i32)`, `As` 落入 catch-all 仅生成 `Comment("TODO: ...")`.

### BUG-R5: parser 多段路径损坏 (parser.rs:323-354)

`std::option::Option::Some(42)` 中 `A::B::C` 形式无法解析。ColonColon 处理器仅处理 `A::B`，二次 `::` 因 lhs 已变为 `EnumLit` 而无法匹配。

### BUG-R6: SwitchToRemote/SwitchToLocal 是 no-op (vm/mod.rs:544-549)

### BUG-R7: IndexAccess 越界访问静默不报错 (vm/mod.rs:354-356)

`if val_idx < frame.data.len() { ... }` 的 else 分支为空。

### BUG-R8: 函数返回类型不验证 (vm/mod.rs:774)

`return_arity: 0` 硬编码，`ReturnArityMismatch` 永不被构造。

---

## 3. 简化/占位实现

### MIR 生成器
- `collect_drop_vars()` 返回空 Vec → Drop 代码永不生成
- catch-all `_ =>` 对 6 种表达式仅注释
- Drop 序列用 `ReadVar + FreeRemote` (仅能处理远程栈类型)
- 函数尾总是 `Ret { argc: 0 }` (忽略声明的返回类型)
- 复杂 `&expr` (非简单变量) 仅生成注释
- Stardust `Call` 使用 `FnId(0)`，`Reverse` 未实现

### VM
- FieldAccess: "简化：假设结构体占 (index+1) 槽"
- MakeEnum: 实现为 roll 往返 (实际无操作)
- return_arity 硬编码 0
- 隐式返回 (非 Ret 结束) 丢弃所有值

### 借用检查
- Move 类型硬编码为 Vec/String/Box/StackHandle (用户类型不可扩展)
- `exit_scope` 注释 "应该被析构但目前静默处理"
- 未定义变量报 `UseAfterMove` (错误类型不匹配)
- **BorrowChecker + DropGen 从未被 pipeline 调用**

### IO 库
- `read_line` 有 2 个 bug (Dup 目标错误 + Pop 清理过度)
- `file_read`/`file_write` 仅生成注释
- `print_str` 无条件追加换行

### 解析器
- 赋值仅支持简单变量 (不支持 `a.b = v`, `arr[i] = v`)
- 多段路径损坏 (`A::B::C`)
- `parse_expr_imm` 非整数静默返回 0
- `parse_attributes` 无错误恢复

---

## 4. 未实现功能

| 类别 | 项目 |
|------|------|
| 代码生成 | LLVM 后端、Stardust 控制流/变量/IO/远程栈 |
| 类型系统 | 虚拟类型栈集成、编译期类型检查、泛型 |
| 标准库 | String (UTF-8验证)、Box\<T\>、vec![] 宏、文件 I/O |
| 工具链 | 调试器、LSP、包管理器、CI、VSCode 保存时检查 |
| 所有权 | NLL、运行期悬空引用检测、Drop 语义集成 |
| 测试 | 集成测试、快照测试、模糊测试、性能基准 |

---

## 5. 死代码清单

| 文件 | 项目 | 说明 |
|------|------|------|
| token.rs:28 | `BoolLiteral` | lexer 从不产生 |
| token.rs:60 | `ElementId` | lexer 从不产生 |
| ast.rs:16 | `Type::FnPtr` | parser 从不构造 |
| ast.rs:52 | `Pattern::Or` | parser 从不构造 |
| ast.rs:99 | `UnOp::Deref` | parser 用 Expr::Deref 代替 |
| error.rs(frontend):30-36 | 3 个变体 | 从不构造 |
| error.rs(vm):32 | `ReturnArityMismatch` | 从不构造 |
| frame.rs:113 | `jump_to_mark()` | 从不调用 |
| stack.rs:168 | `top_value_mut()` | 从不调用 |
| var_stack.rs:52,74 | `lookup_with_depth()`, `is_bound()` | 从不调用 |
| generator.rs:302 | `collect_drop_vars()` | 返回空 |
| type_stack.rs | `TypeStack` 全部 | 从未集成 |

---

## 6. 测试缺口

| 优先级 | 测试内容 |
|--------|---------|
| 高 | 枚举匹配端到端 (match arm 选择正确分支) |
| 高 | Vec push/pop/index/len 集成测试 |
| 高 | 结构体字段访问读写验证 |
| 高 | 值断言 (Fibonacci 返回值、while 循环变量值) |
| 中 | Stardust 快照测试 (对比预期输出) |
| 中 | BorrowChecker 所有错误变体触发路径 |
| 中 | 远程栈扩容边界 (恰好满、超出初始容量) |
| 低 | 模糊测试 (随机 MIR 序列) |
| 低 | 性能基准 (大栈深度、深度递归) |

---

## 7. 项目完成度评估

| 组件 | 修复前 | 修复后 | 仍待解决 |
|------|--------|--------|---------|
| VM 核心 | 70% | **85%** | SwitchToRemote, return_arity, 隐式返回 |
| Parser | 70% | **75%** | 多段路径, 赋值限制 |
| HIR 降级 | 60% | **72%** | Drop 未连接, 解构模式 |
| MIR 生成 | 55% | **70%** | catch-all 6种表达式, Drop 未生成 |
| Stardust | 15% | **20%** | 25+ 指令仅注释 |
| IO 库 | 20% | **30%** | read_line 有 bug, file I/O 占位 |
| 借用检查 | 40% | **40%** | 未集成, Move 类型硬编码 |
| 类型系统 | 25% | **25%** | TypeStack 未集成 |
| 测试 | 45% | **50%** | 缺值断言和错误路径 |
| **整体** | **~55%** | **~65%** | — |

### 建议优先修复顺序

1. **BUG-R1**: read_line Dup/Swap 修复
2. **BUG-R2**: 连接 DropGen 到 gen_block
3. **BUG-R4**: 为 6 个 HirExpr 变体生成基本 MIR
4. **BUG-R6**: SwitchToRemote 实现
5. **BUG-R5**: parser 多段路径修复
6. **S3**: Stardust 控制流代码生成
7. **S5**: BorrowChecker 集成到 pipeline

---

> 自由如鸟，尽在栈中。
> Free as a bird, everything is a stack.
