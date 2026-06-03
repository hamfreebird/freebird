# Freebird 实现状态

> **状态**: 核心实现完成 (v0.1.0)，部分高级特性简化或未实现。
> **总代码量**: ~9,700 行 Rust (37 个源文件)，122 个测试全部通过。
> **实现语言**: Rust 2024 Edition (MSRV 1.85)

---

## 目录

1. [总体架构](#1-总体架构)
2. [核心数据结构](#2-核心数据结构)
3. [模块设计与实现状态](#3-模块设计与实现状态)
4. [关键算法](#4-关键算法)
5. [已完成功能](#5-已完成功能)
6. [部分实现 — 待完善](#6-部分实现--待完善)
7. [未实现 — 待开发](#7-未实现--待开发)
8. [技术风险与缓解](#8-技术风险与缓解)
9. [测试覆盖现状](#9-测试覆盖现状)
10. [项目实际结构](#10-项目实际结构)

---

## 1. 总体架构

### 1.1 编译与执行流水线

```
Freebird 源码 (.fb)
    │
    ▼
┌──────────────┐
│   Lexer      │ ✅  → Token 流 (760行，完整词法)
└──────────────┘
    │
    ▼
┌──────────────┐
│   Parser     │ ✅  → AST (递归下降+Pratt，1,110行)
└──────────────┘
    │
    ▼
┌──────────────┐
│ Name Resolver│ ✅  → HIR (作用域帧栈+前向引用检测，540行)
└──────────────┘
    │
    ▼
┌──────────────┐
│ Type Table   │ ⚠️  → 仅类型注册表（结构体/枚举布局），缺虚拟类型栈
└──────────────┘
    │
    ▼
┌──────────────┐
│ Borrow CK    │ ⚠️  → name-based 借用检查，缺 ElementId 维度
└──────────────┘
    │
    ▼
┌──────────────┐
│ MIR Gen      │ ✅  → MIR 指令序列 (620行生成器 + 优化器)
└──────────────┘
    │
    ├──────────────────────┐
    ▼                      ▼
┌──────────────┐   ┌──────────────┐
│ Stardust Gen │   │ LLVM IR Gen  │
│ ⚠️ 基本交错  │   │ ❌ 未实现    │
└──────────────┘   └──────────────┘
    │
    ▼
┌──────────────┐
│ Freebird VM  │ ✅  → 双栈解释器 (1,087行，远程栈操作完整)
└──────────────┘
```

### 1.2 执行模式实现状态

| 模式 | 命令 | 状态 |
|------|------|------|
| **解释执行** | `freebird run <file.fb>` | ✅ 完整 |
| **Stardust 编译** | `freebird build --target stardust` | ⚠️ 基本实现 (Call/Ret 为注释) |
| **LLVM 编译** | `freebird build --target llvm` | ❌ 未实现 |
| **REPL** | `freebird repl` | ✅ 完整 (含 :stack/:vars/:info) |
| **检查** | `freebird check <file.fb>` | ✅ JSON 诊断输出 |

---

## 2. 核心数据结构

### 2.1 Value 类型 — ✅ 完成

所有计划类型已实现：I8-I64, Isize, U8-U64, Usize, F32, F64, Bool, Char, Unit, Tuple, Ref, MutRef, StackHandle, FnPtr, CallElement。

### 2.2 ElementId 与 StackId — ✅ 完成

IdAllocator 完整实现，ElementId/StackId/FnId 独立分配器，StackId 使用 SipHash 派生（避免碰撞）。

### 2.3 双栈 (DualStack) — ✅ 完成

完整实现：push/pop/dup/roll(正/零/负)/swap/pick/find_id/get_from_top/truncate。
Roll 往返测试通过 (roll(+n); roll(-n) 等价于 no-op)。

### 2.4 变量栈 (VarStack) — ✅ 完成

作用域帧管理、遮蔽、update。

### 2.5 调用帧 (CallFrame) — ✅ 完成

marks 预扫描、指令共享 (Rc)、调试信息。

### 2.6 全局注册表 (GlobalRegistry) — ✅ 完成

远程栈管理，HashMap<StackId, DualStack>。

### 2.7 MIR 指令集 — ✅ 完成

50+ 变体全部定义：栈操作、算术、比较、逻辑、控制流、变量/作用域、引用、复合类型、远程栈、I/O、调试、元数据。

---

## 3. 模块设计与实现状态

### 3.1 模块依赖图（含实现状态）

```
                    ┌──────────┐
                    │   main   │ ✅ (CLI: run/check/repl/build)
                    └────┬─────┘
                         │
                    ┌────▼─────┐
                    │  lib.rs  │ ✅ (公共 API)
                    └────┬─────┘
                         │
          ┌──────────────┼──────────────────┐
          │              │                  │
    ┌─────▼──────┐ ┌─────▼──────┐ ┌────────▼───────┐
    │  frontend  │ ✅  │    hir     │ ✅  │      vm         │ ✅
    │  lexer     │     │  lowering  │     │  stack, frame   │
    │  parser    │     │  resolver  │     │  registry, id   │
    │  ast,token │     │  hir types │     │  value, remote  │
    └─────┬──────┘ └─────┬──────┘ └────────┬───────┘
          │              │                  │
          │         ┌────▼──────┐           │
          │         │  typeck   │ ⚠️        │
          │         │ type_table│ (缺type   │
          │         │           │ _stack)   │
          │         └────┬──────┘           │
          │              │                  │
          │         ┌────▼──────┐           │
          │         │ borrowck  │ ⚠️        │
          │         │ (name-    │           │
          │         │  based)   │           │
          │         └────┬──────┘           │
          │              │                  │
          │         ┌────▼──────┐    ┌──────▼──────┐
          │         │    mir    │ ✅ │   codegen    │ ⚠️
          │         │ generator │    │  stardust(⚠) │
          │         │           │    │  llvm (❌)   │
          │         └───────────┘    │  optimize(✅)│
          │                          └──────────────┘
          │
     ┌────▼──────┐
     │   repl    │ ✅ (交互式 REPL + 状态检查)
     │   io.rs   │ ⚠️ (file ops 占位)
     │diagnostics│ ✅ (JSON 诊断)
     └───────────┘
```

### 3.2 模块职责与实现状态

| 模块 | 路径 | 状态 | 说明 |
|------|------|------|------|
| `vm` | `src/vm/` | ✅ 完整 | 双栈、帧管理、全局注册表、远程栈、ID分配、MIR解释器、错误类型、运行时配置 |
| `frontend` | `src/frontend/` | ✅ 完整 | 词法分析 (760行)、语法解析 (1,110行)、AST定义、Token类型 |
| `hir` | `src/hir/` | ✅ 完整 | HIR数据结构 (228行)、AST→HIR降级 (540行)、名称解析 (239行) |
| `typeck` | `src/typeck/` | ⚠️ 部分 | 类型注册表 TypeTable (260行)；**缺: type_stack.rs (虚拟类型栈)** |
| `borrowck` | `src/borrowck/` | ⚠️ 部分 | name-based 借用检查 (385行)、Drop代码生成 (56行)；**缺: ElementId-based 运行期检查** |
| `mir` | `src/mir/` | ⚠️ 部分 | MIR生成器 (850行)；**注意: MirInst 定义在 vm/mir.rs 而非 mir/** |
| `codegen` | `src/codegen/` | ⚠️ 部分 | Stardust代码生成 (239行)、MIR优化器 (184行)；**缺: llvm.rs, intrinsics.rs** |
| `repl` | `src/repl.rs` | ✅ 完整 | 交互式 REPL 主循环、状态可视化 (215行) |
| `io` | `src/io.rs` | ⚠️ 部分 | print/read 操作 (113行)；**file_read/write 为占位注释** |
| `diagnostics` | `src/diagnostics.rs` | ✅ 完整 | JSON 诊断输出 (178行) |
| `pipeline` | `src/pipeline.rs` | ✅ 完整 | 端到端编译流水线 (302行) |

### 3.3 各模块详细分析

#### 3.3.1 AST 节点类型 — ✅ 完成

所有计划节点已实现：
- Expr: Lit, Var, IdRef, Deref, Ref, Binary, Unary, Call, FieldAccess, Index, Tuple, Array, Repeat, StructLit, EnumLit, If, Match, Loop, While, For, Block, Break, Return, Ret, Unsafe, StardustMacro, Continue, As
- Stmt: Let (含pattern/ty/mutable), Expr, Assign
- Decl: Fn, Struct, Enum, Extern, Use
- 额外实现: FnPtr类型, Never类型, Wildcard类型

**与计划的差异**: `pattern.rs` 未独立创建——模式解析集成在 `parser.rs` 中（`parse_pattern()` 方法）。

#### 3.3.2 名称解析 — ✅ 完成

作用域构建、变量绑定、名称消解、前向引用检测。
函数前向引用合法（两遍处理：先收集签名，再解析体）。

#### 3.3.3 类型检查器 — ⚠️ 仅 TypeTable

| 组件 | 状态 |
|------|------|
| TypeTable (结构体/枚举布局) | ✅ 完成 |
| 虚拟类型栈 (TypeStack) | ❌ 未实现 |
| 类型推断 | ❌ 未实现 |
| 类型合并 (控制流) | ❌ 未实现 |
| 类型错误类型 | ❌ 未实现 (error.rs 不存在) |

**影响**: 无编译期类型检查，类型错误仅在 VM 运行时以 `TypeMismatch` 形式暴露。

#### 3.3.4 MIR 生成 — ✅ 完成

MirGen 完整实现：
- ✅ 表达式 → MIR 指令序列
- ✅ ReadVar → roll+dup+roll(-pos)
- ✅ WriteVar → roll+pop+roll(-pos)
- ✅ 函数调用 → PushCallElement + Call
- ✅ if/while/loop → Mark/Jump/UncondJump
- ✅ return/break/continue
- ✅ 结构体/枚举字面量
- ⚠️ 字段访问 → 基本 roll 实现（不处理多槽偏移）
- ⚠️ Match/枚举匹配 → 简化实现（硬编码 "Option" 名）
- ⚠️ 类型名 Vars → 跳过 ReadVar 生成（scope_depth == MAX）

**MIR 优化** (已移至 `codegen/optimize.rs`):

| 优化遍 | 状态 |
|--------|------|
| 常量折叠 | ✅ Push(a) Push(b) Add → Push(a+b) |
| 死代码消除 | ✅ Push后立即Pop |
| 窥孔优化 | ✅ Roll(0)→Nop, Swap+Swap→Nop, Dup+Pop→Nop |
| 跳转优化 | ❌ 未实现 (Jump A; ... Mark A; UncondJump B → Jump B) |
| 冗余Roll消除 | ✅ roll(n); roll(-n) → Nop |

#### 3.3.5 借用检查器 — ⚠️ name-based 简化

| 计划 | 实际 |
|------|------|
| ElementId-based 借用状态追踪 | name-based (HashMap<String, VarInfo>) |
| 编译期 + 运行期双重检查 | 仅编译期 |
| 作用域帧栈同步 | 简单 scope_depth 计数 |
| Drop 语义 | 简化（所有 Move 类型统一 FreeRemote） |

**限制**: 不能区分同一变量的多个 ElementId 实例的不同借用状态。

#### 3.3.6 Stardust 代码生成 — ⚠️ 基本实现

| 计划 | 实际 |
|------|------|
| 双栈→单栈交错映射 | ✅ Push/Pop/Dup/Roll 翻译 |
| Roll 指令展开 | ✅ n×2 ShiftL/ShiftR |
| 多栈调用翻译 | ⚠️ Call/Ret 仅注释，无实际指令 |
| 远程栈→Stardust堆映射 | ⚠️ 注释占位 |
| 算术/比较/逻辑 | ✅ 含 ID 清理序列 |

**限制**: 生成的 Stardust 代码不完整——函数调用和返回仅为注释标记，不能直接在 Stardust 解释器中运行。

#### 3.3.7 LLVM 代码生成 — ❌ 未实现

整个 `codegen/llvm.rs` 和 `codegen/intrinsics.rs` 未创建。

---

## 4. 关键算法

### 4.1 变量访问的编译 (read_var / write_var 展开) — ✅ 完整

VM 执行期动态查找 ElementId + roll/dup/roll(-pos) 序列。

**优化方案（未实现）**: 编译时推断静态偏移量、绕过运行时 ID 查找。

### 4.2 类型栈的控制流合并 — ❌ 未实现

无虚拟类型栈，因此无法在控制流合并点检查分支类型一致性。

### 4.3 远程栈的增长 (Vec push) — ✅ 完整

扩容逻辑：分配新栈（容量翻倍，最小4）→ 复制旧元素 → 更新 GlobalRegistry → 释放旧栈。

### 4.4 函数调用的完整 MIR 序列 — ✅ 完成

命名/位置参数 + 栈传参 + CallElement 机制。

### 4.5 ID→位置的运行时查找优化 — ❌ 未实现

朴素线性扫描 O(d)，无 HashMap 缓存。
对典型函数 (d < 100)，线性扫描开销可忽略。

---

## 5. 已完成功能

以下所有功能的实现计划已完成并可归档：

### Phase 0: 项目搭建
Cargo 项目、CLI 框架、Value/ElementId/StackId/FnId、IdAllocator、DualStack 基础。

### Phase 1: 双栈 VM 核心
DualStack::roll/swap/pick、VarStack、CallFrame、GlobalRegistry、MirInst 指令集、VM 解释器主循环、算术/比较/逻辑执行、ReadVar/WriteVar、控制流。

### Phase 2: 前端
Lexer（完整词法+转义+注释）、Parser（递归下降+Pratt 表达式）、AST（含 Span）、错误恢复、stardust! 宏解析。

### Phase 3: HIR + 名称解析
HIR 类型、作用域构建、变量消解、前向引用检测、函数声明收集。

### Phase 4: MIR 生成 + 端到端
MirGen、ReadVar/WriteVar 序列、函数调用、if/while/loop 跳转、CLI 集成。Fibonacci/阶乘可运行。

### Phase 5: 复合类型
TypeTable、结构体/枚举布局、字段访问、构造/解构、元组、固定数组。

### Phase 6: 可变长度类型
远程栈模型、AllocRemote/FreeRemote/RemotePush/RemotePop/RemoteLoad/RemoteStore/RemoteLen、扩容逻辑。

### Phase 7: 所有权与借用检查
BorrowState 状态机、借用规则、Move 语义、Drop 生成、6 种错误类型。

### Phase 8: Unsafe + FFI + stardust!
Unsafe 块、stardust! 宏指令翻译（27种）、extern FFI。

### Phase 9: 代码生成
MIR 优化器（4 种优化遍）、Stardust 代码生成器（双栈→单栈交错映射）。

### Phase 10: 工具链
IO 标准库、REPL（:stack/:vars/:info）、JSON 诊断输出。

### Phase 11: VSCode + 文档
VSCode 扩展（.fb 语法高亮）、USAGE.md 使用指南。

---

## 6. 部分实现 — 待完善

以下功能已实现基本骨架，但在细节、健壮性或完整度上有不足，需要继续完善。

### 6.1 类型系统 (高优先级)

| 问题 | 位置 | 影响 |
|------|------|------|
| **虚拟类型栈 (TypeStack) 缺失** | `typeck/type_stack.rs` 不存在 | 无法在编译期验证栈深度和类型一致性 |
| **类型错误类型缺失** | `typeck/error.rs` 不存在 | 类型错误没有结构化诊断 |
| **TypeTable 字段类型不完整** | `type_table.rs:34` | `slots` 固定为 1（不处理嵌套结构体/元组展开） |
| **字段偏移量计算简化** | `vm/mod.rs:327` | 假设结构体占 (index+1) 槽，未使用 TypeTable 信息 |

**建议**: 实现完整的 TypeStack，在 MIR 生成前遍历 HIR 并维护类型栈，在控制流合并点检查一致性。

### 6.2 借用检查器 (高优先级)

| 问题 | 位置 | 影响 |
|------|------|------|
| **Name-based 而非 ElementId-based** | `borrowck/checker.rs:68` | 无法区分同一变量的不同 ElementId 实例 |
| **无运行期悬空引用检测** | 整个 `borrowck/` | debug 模式下不检测 ElementId 是否仍有效 |
| **Drop 生成过于简化** | `borrowck/drop_gen.rs:13` | 对所有 Move 类型统一 FreeRemote，无类型感知 |
| **无 NLL (Non-Lexical Lifetime)** | `borrowck/checker.rs` | 基于词法作用域，可能拒绝合法程序 |

**建议**: 
1. 增加 ElementId 维度的借用追踪
2. VM debug 模式在 Deref 前检查 `find_id(id).is_some()`
3. 根据 TypeTable 的 all_copy 标记决定是否生成 Drop 代码

### 6.3 MIR 生成器

| 问题 | 位置 | 影响 |
|------|------|------|
| **字段访问偏移量未使用 TypeTable** | `mir/generator.rs:630` | `resolve_field_offset()` 始终返回 0 |
| **Match/枚举匹配硬编码** | `mir/generator.rs:610` | 枚举名硬编码为 "Option" |
| **Match 分支生成不完整** | `mir/generator.rs:594-620` | 只处理第一个 Enum variant 模式，无通配符 fallthrough |
| **Stardust 宏内 Call 使用 FnId(0)** | `mir/generator.rs:783` | 占位 FnId，调用不会成功 |
| **参数绑定 roll 序列追踪** | `mir/generator.rs:240-253` | StackTracker 与 MIR 同步可能偏差 |

### 6.4 VM 运行时

| 问题 | 位置 | 影响 |
|------|------|------|
| **FieldAccess 结构体大小假定** | `vm/mod.rs:327` | 假设字段偏移即 roll 距离 |
| **EnumTag 硬编码 2 槽** | `vm/mod.rs:373` | `enum_slots = 2` 固定，不能处理 4 槽枚举 (如 Color) |
| **MakeEnum 无实际布局操作** | `vm/mod.rs:353-366` | 仅分配 ElementId，不确保 tag 在 data 下方 |
| **ID 查找线性扫描** | `stack.rs:find_id()` | O(d) 无缓存 |
| **Deref 后 roll(-pos) 越界** | `vm/mod.rs:305-307` | 当目标在栈底时 roll(-pos) 可能越界 |
| **WriteVar roll(-pos) 边界** | `vm/mod.rs:657` | 当旧值在栈底时需特殊处理（已有 fix） |

### 6.5 Stardust 代码生成

| 问题 | 位置 | 影响 |
|------|------|------|
| **Call/Ret 仅注释** | `codegen/stardust.rs:139-141` | 生成的 Stardust 代码不可独立执行 |
| **算术/比较操作序列复杂** | `stardust.rs:emit_binary_op` | Pop/Pop/Swap/Pop/Pop/Swap 序列可能不精确 |
| **无远程栈→堆映射** | `stardust.rs` | AllocRemote 等仅注释 |
| **无跳转标签生成** | `stardust.rs` | Mark/Jump 仅注释 |

### 6.6 IO 库

| 问题 | 位置 | 影响 |
|------|------|------|
| **file_read/write 为注释占位** | `io.rs:71-80` | 无实际文件 I/O |
| **read_line 简化为单字符** | `io.rs:65` | 不能读取完整行 |
| **无 String 类型支持** | 全局 | 字符串操作需在 unsafe 块通过 stardust! 实现 |

### 6.7 复合类型

| 问题 | 位置 | 影响 |
|------|------|------|
| **无元组字段访问偏移计算** | `mir/generator.rs` | 元组字段访问未经测试 |
| **枚举零数据变体执行问题** | `pipeline.rs:250` | 已知 bug，待修复 |
| **结构体解构未经端到端测试** | `let Point {x, y} = p` | 模式解构实现但未验证 |

---

## 7. 未实现 — 待开发

### 7.1 LLVM 后端 (Phase 9 剩余)

- ❌ `codegen/llvm.rs` — LLVM IR 代码生成器
- ❌ `codegen/intrinsics.rs` — 内建函数
- ❌ 栈槽→SSA 寄存器映射
- ❌ 控制流边界栈 flush/reload

### 7.2 类型系统增强

- ❌ `typeck/type_stack.rs` — 虚拟类型栈
- ❌ `typeck/error.rs` — 类型错误类型
- ❌ 类型推断（简单情况）
- ❌ 控制流合并点的类型栈验证

### 7.3 标准库类型

- ❌ **String** — Vec<u8> 包装 + UTF-8 有效性验证
- ❌ **Box\<T\>** — 单值远程栈分配
- ❌ **Vec\<T\>** 语法糖 — `vec![1, 2, 3]` 宏/语法
- ❌ **Option\<T\>** / **Result\<T, E\>** 泛型支持
- ❌ 泛型系统 (类型参数)

### 7.4 测试与 CI

- ❌ `tests/integration_test.rs` — 集成测试
- ❌ `tests/programs/*.fb` — 参考程序集
- ❌ `tests/mir_snapshots/` — MIR 快照测试
- ❌ `examples/hello.fb` — Hello World 示例文件
- ❌ CI 配置 (GitHub Actions)
- ❌ 模糊测试 (proptest)

### 7.5 工具链

- ❌ **调试器** — 单步执行、交互式断点
- ❌ **包管理器** (`freebird package`)
- ❌ **LSP 服务器** (Language Server Protocol)
- ❌ **保存时检查** (VSCode 扩展调用 `freebird check`)
- ❌ 悬停提示 (类型信息)

### 7.6 Stardust 代码生成完善

- ❌ 多栈调用 → Stardust 调用序列翻译
- ❌ 远程栈 → Stardust 堆映射
- ❌ Roll 在 Stardust 后端的 O(n) 展开优化
- ❌ 生成的 Stardust 代码可直接执行

### 7.7 所有权系统增强

- ❌ NLL (Non-Lexical Lifetime) 借用检查
- ❌ 运行期悬空引用检测 (debug 模式)
- ❌ 析构顺序验证（按创建逆序）

### 7.8 性能优化

- ❌ HashMap<ElementId, usize> ID 位置缓存
- ❌ 编译时静态偏移量推断（绕过运行时 find_id）
- ❌ Stardust 后端性能路径 (LLVM 替代)

### 7.9 结构性问题

- ❌ MirInst 定义在 `vm/mir.rs` — 应移至 `mir/inst.rs` 或 `mir/mod.rs`
- ❌ `optimize.rs` 在 `codegen/` — 应移至 `mir/optimize.rs`
- ❌ `pattern.rs` 未独立 — 模式解析逻辑应提取为独立模块
- ❌ `remote.rs` 未创建 — 远程栈逻辑内联在 `vm/mod.rs`
- ❌ `repl/` 应恢复为目录模块 (mod.rs + executor.rs + display.rs)

---

## 8. 技术风险与缓解

| 风险 | 影响 | 概率 | 当前状态 | 缓解措施 |
|------|------|------|---------|----------|
| **`roll(n)` 在 Stardust 后端的 O(n) 展开** | 中 | 高 | ⚠️ 基本实现 | MIR 优化消除冗余 roll；LLVM 后端作为性能路径 |
| **ID 查找的线性扫描在大栈上成为瓶颈** | 中 | 低 | ⚠️ 未优化 | HashMap 缓存待实现；不优化更简单 |
| **借用检查器实现复杂度超预期** | 高 | 中 | ⚠️ name-based | 完整 ElementId-based 检查推迟 |
| **Stardust `Call` 模型与 Freebird 调用元素不兼容** | 高 | 中 | ⚠️ Call/Ret 仅注释 | 调用元素在 Stardust 后端需完整翻译 |
| **LLVM 后端的 SSA 化与任意 roll 操作冲突** | 高 | 中 | ❌ 未开始 | roll 在 LLVM 后端编译为 extractvalue/insertvalue |
| **远程栈重分配时更新本地句柄的竞争条件** | 中 | 低 | ✅ 已实现 | 单线程 VM 无竞争 |
| **类型检查缺失导致运行时错误** | 中 | 高 | ⚠️ 无虚拟类型栈 | VM 运行时 TypeMismatch 捕获部分错误 |

---

## 9. 测试覆盖现状

### 9.1 测试统计

| 模块 | 测试数 | 覆盖内容 |
|------|--------|---------|
| `vm::stack` | 14 | push/pop, dup, roll(正/负/零/往返/越界/空栈), swap, pick, find_id |
| `vm::var_stack` | 5 | 作用域进出, 遮蔽, update, 未定义, 内层遮蔽更新 |
| `vm::tests` | 8 | 算术, 变量读写, if/else, Call/Ret, 远程栈(alloc/free/push/pop/load/len) |
| `frontend::lexer` | 11 | 关键字, 标识符, 整数/浮点/字符/字符串, 运算符, 分隔符, 注释, stardust!, # |
| `frontend::parser` | 35 | 每种表达式, 语句, 声明, 模式, 控制流 |
| `hir::resolver` | 5 | 声明+解析, 遮蔽, 未定义, 前向引用, 重复定义 |
| `hir::lowering` | 7 | 简单函数, 变量解析, 未定义变量, 前向引用, 遮蔽, 函数前向引用, 参数不匹配 |
| `typeck::type_table` | 3 | 结构体注册, 枚举注册, AST构建 |
| `borrowck::checker` | 5 | 共享借用, 独占借用, Move语义, 释放引用, 不可变赋值 |
| `borrowck::drop_gen` | 3 | 空Drop, 单Drop, scope exit |
| `codegen::optimize` | 4 | 常量折叠, Roll(0), Push+Pop, 冗余Roll |
| `codegen::stardust` | 2 | 简单算术生成, Roll翻译 |
| `io` | 2 | print_str, std_library |
| `diagnostics` | 2 | 空诊断, 解析错误诊断 |
| `pipeline` | 10 | 算术, 变量, if/else, while, Fibonacci, 嵌套调用, stardust!, unsafe, extern, 结构体构造 |
| **总计** | **122** | |

### 9.2 测试缺口

| 优先级 | 测试类别 | 建议 |
|--------|---------|------|
| 高 | 结构体字段访问端到端 | `Point { x, y }; .x` 读写 |
| 高 | 元组构造与解构 | `let (a, b) = (1, 2)` |
| 高 | 枚举匹配端到端 | `match opt { None => ..., Some(v) => ... }` |
| 高 | Vec 操作端到端 | push/pop/index/len 完整流程 |
| 中 | 借用检查器错误路径 | 各种 BorrowError 触发条件 |
| 中 | Stardust 代码生成快照 | 与预期 Stardust 输出比对 |
| 中 | 远程栈扩容 | 超过初始 capacity 的 push |
| 低 | 模糊测试 | 随机 MIR 序列 → VM 无 crash |
| 低 | 性能基准 | Fibonacci 大参数、深度递归 |

---

## 10. 项目实际结构

```
freebird/
├── Cargo.toml
├── Cargo.lock
├── readme.md                         # Freebird 语言规范 v0.5.0
├── IMPROVEMENTS.md                   # 本文件：实现状态
├── USAGE.md                          # 使用指南
├── LICENSE
├── .gitignore
│
├── src/
│   ├── main.rs                       # CLI 入口 (run/check/repl/build)
│   ├── lib.rs                        # 库入口 (公共 API)
│   ├── pipeline.rs                   # 端到端编译流水线
│   ├── io.rs                         # IO 标准库 (⚠️ file ops 占位)
│   ├── repl.rs                       # REPL (✅ 完整)
│   ├── diagnostics.rs                # JSON 诊断 (✅ 完整)
│   │
│   ├── frontend/                     # ✅ 完整
│   │   ├── mod.rs, lexer.rs, parser.rs
│   │   ├── ast.rs, token.rs, error.rs
│   │   └── parser_tests.rs
│   │
│   ├── hir/                          # ✅ 完整
│   │   ├── mod.rs, types.rs
│   │   ├── lowering.rs, resolver.rs
│   │
│   ├── typeck/                       # ⚠️ 仅 TypeTable
│   │   ├── mod.rs, type_table.rs
│   │   # ❌ type_stack.rs, error.rs
│   │
│   ├── borrowck/                     # ⚠️ name-based
│   │   ├── mod.rs, checker.rs, drop_gen.rs
│   │
│   ├── mir/                          # ⚠️ MirInst 在 vm/
│   │   ├── mod.rs, generator.rs
│   │
│   ├── vm/                           # ✅ 完整
│   │   ├── mod.rs, stack.rs, frame.rs
│   │   ├── registry.rs, value.rs, id.rs
│   │   ├── mir.rs (⚠️ 应为 mir/inst.rs)
│   │   ├── error.rs, config.rs, var_stack.rs
│   │   # ❌ remote.rs
│   │
│   └── codegen/                      # ⚠️ 缺 LLVM
│       ├── mod.rs, stardust.rs, optimize.rs
│       # ❌ llvm.rs, intrinsics.rs
│
├── vscode/                           # ✅ VSCode 扩展
│   ├── package.json
│   ├── language-configuration.json
│   ├── .vscodeignore, README.md
│   ├── icons/ (dark + light SVG)
│   └── syntaxes/ (freebird.tmLanguage.json)
│
├── tests/                            # ❌ 未创建
│   # 缺: integration_test.rs, programs/*.fb, mir_snapshots/
│
└── examples/                         # ❌ 未创建
    # 缺: hello.fb
```

---

## 实现完成度总结

| 类别 | 完成度 | 说明 |
|------|--------|------|
| 编译流水线 | 90% | 源码→AST→HIR→MIR→VM 完整 |
| 虚拟机核心 | 85% | 双栈/远程栈完整；ID缓存/静态偏移量待优化 |
| 类型系统 | 40% | TypeTable 完成；缺虚拟类型栈 |
| 借用检查 | 55% | name-based 完成；缺 ElementId 维度 |
| 代码生成 | 35% | Stardust 基本完成；LLVM 未开始；Call/Ret 仅注释 |
| 标准库 | 30% | print/read 可用；String/Box/Vec 语法糖未实现 |
| 工具链 | 65% | REPL/CLI/诊断完成；调试器/LSP/包管理未实现 |
| 测试 | 55% | 122 个单元测试；缺集成测试/快照测试/参考程序 |
| 文档 | 60% | README/USAGE/IMPROVEMENTS；缺标准库文档 |

**总体完成度: ~65%**

---

> 自由如鸟，尽在栈中。
> Free as a bird, everything is a stack.
