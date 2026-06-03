# Freebird 实现状态 — v3 审计 (26 Bug 修复后)

> **审计日期**: 2026-06-03 (第3轮)
> **已修复**: 26 个 bug (4 批修复)
> **测试**: 130 全部通过 (126 unit + 4 integration)

---

## 1. 修复记录

### 第 1 批 — 9 bugs
| Bug | 修复 |
|-----|------|
| if-else 反向 | Jump 前插 Not |
| roll-back 算法 (6处) | 直接索引访问 |
| Let 无 init BindVar 空栈 | 仅在有 init 时绑定 |
| f32/f64 后缀 | lex_int_suffix 返回 float 标记 |
| 错误类型误用 | 专用变体 |
| IdRef 空体 | 正确推入错误 |
| max_stack 未检查 | push_checked |
| MakeEnum 泄漏 ID | 移除 alloc |

### 第 2 批 — 9 bugs
| Bug | 修复 |
|-----|------|
| 空字面量不报错 | is_empty 检查 |
| NaN cast | is_finite 检查 |
| Jump 非整数 | TypeMismatch |
| RemotePop 空栈 | AssertionFailed |
| Call FnId(0) | Comment 标记 |
| Enum::None 丢失 | 无 LParen 构造 EnumLit |
| Match 标签泄漏 | Pop 清理 |

### 第 3 批 — 5 bugs
| Bug | 修复 |
|-----|------|
| Break 仅注释 | loop_ends 栈 |
| 字段偏移/枚举推断 | 遍历 TypeTable |
| Stardust 算术栈空 | Pick-based |
| filter_map 吞错误 | for-loop |
| read_line 损坏 | Mark/Jump 循环 |

### 第 4 批 — 3 bugs
| Bug | 修复 |
|-----|------|
| SwitchToRemote/SwitchToLocal no-op | active_remote 完整实现 |
| return_arity 硬编码 0 | Call 添加字段 + 验证 |
| 隐式返回丢弃值 | 保留 return_arity 个值传给调用者 |

---

## 2. 仍存在的问题

### 高优先级

| # | 位置 | 问题 |
|---|------|------|
| 1 | io.rs:78 | read_line Dup 复制 char 而非 handle |
| 2 | io.rs:85 | read_line 循环退出时 Pop 了 handle |
| 3 | generator:304 | collect_drop_vars 返回空 → Drop 永不生成 |
| 4 | stardust:116-158 | 25+ MIR 指令仅注释 (控制流/变量/IO/远程栈) |
| 5 | generator:661 | 6 HirExpr 变体落入 catch-all 仅注释 (IdRef/Repeat/For/Continue/Ret/As) |
| 6 | parser:323 | 多段路径 A::B::C 解析损坏 |
| 7 | vm/mod:354 | IndexAccess 越界 else 分支静默不报错 |

### 中优先级

| # | 位置 | 问题 |
|---|------|------|
| 8 | borrowck | BorrowChecker + DropGen 从未被 pipeline 调用 |
| 9 | checker:83 | Move 类型硬编码 (Vec/String/Box/StackHandle) |
| 10 | checker:110 | exit_scope 注释 "应被析构但静默处理" |
| 11 | checker:161 | 未定义变量报 UseAfterMove (错误类型不对) |
| 12 | io:91-102 | file_read/file_write 仅注释 |
| 13 | io:37 | print_str 无条件追加换行 |
| 14 | parser:249 | 赋值仅支持简单变量 |
| 15 | parser:1023 | parse_expr_imm 非整数静默返回 0 |

### 低优先级

| # | 位置 | 问题 |
|---|------|------|
| 16 | vm/mod:343 | FieldAccess 注释 "假设占 (index+1) 槽" |
| 17 | vm/mod:365 | MakeEnum 实现为 roll 往返 (无实际验证) |
| 18 | lexer:665 | 未知转义序列 \q 静默接受 |
| 19 | lowering:236 | 复杂赋值目标报错类型不对 |
| 20 | lowering:459 | 解构模式暂不处理 |

---

## 3. 死代码

| 文件 | 项目 |
|------|------|
| token.rs:28,60 | BoolLiteral, ElementId |
| ast.rs:16,52,99 | Type::FnPtr, Pattern::Or, UnOp::Deref |
| error.rs(frontend):30-36 | 3 个错误变体 |
| frame.rs:113 | jump_to_mark() |
| stack.rs:168 | top_value_mut() |
| var_stack.rs:52,74 | lookup_with_depth(), is_bound() |
| generator:304 | collect_drop_vars() 返回空 |
| type_stack.rs 全部 | 未集成到 pipeline |
| borrowck 全部 pub fns | 从未被调用 |

---

## 4. 未实现

| 类别 | 项目 |
|------|------|
| 代码生成 | LLVM 后端, Stardust 25+ 指令 |
| 类型系统 | TypeStack 集成, 编译期类型检查, 泛型 |
| 标准库 | String, Box\<T\>, vec![], 文件 I/O |
| 工具链 | 调试器, LSP, 包管理器, CI |
| 所有权 | NLL, 运行期悬空引用, Drop 集成 |

---

## 5. 测试缺口

| 优先级 | 内容 |
|--------|------|
| 高 | 枚举匹配端到端, Vec 操作集成测试, 结构体字段验证 |
| 高 | 值断言 (Fibonacci 返回值, while 循环变量) |
| 中 | Stardust 快照, BorrowChecker 错误路径, 扩容边界 |
| 低 | 模糊测试, 性能基准 |

---

## 6. 完成度评估

| 组件 | v1 | v2 | v3 |
|------|-----|-----|-----|
| VM 核心 | 70% | 85% | **90%** |
| Parser | 70% | 75% | 75% |
| HIR 降级 | 60% | 72% | 72% |
| MIR 生成 | 55% | 70% | 70% |
| Stardust | 15% | 20% | 20% |
| IO 库 | 20% | 30% | 30% |
| 借用检查 | 40% | 40% | 40% |
| 类型系统 | 25% | 25% | 25% |
| 测试 | 45% | 50% | 50% |
| **整体** | **~55%** | **~65%** | **~70%** |

### 修复进度

| 指标 | v1 | v2 | v3 |
|------|-----|-----|-----|
| 已修复 bug | 0 | 23 | 26 |
| 剩余高优 | 35 | 8 | 7 |
| 死代码项 | 15 | 12 | 10 |
| 测试通过 | 130 | 130 | 130 |

---

> 自由如鸟，尽在栈中。
> Free as a bird, everything is a stack.
