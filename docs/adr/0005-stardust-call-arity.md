# ADR 0005：`stardust!` 的 Call 返回 arity

> 状态：已接受（M6-1.3，2026-11-24）

## 背景

Stardust 函数天然支持多值返回，返回值个数由函数体决定。Freebird 的 `stardust!`
宏内 `Call(n, m)` 只声明被调函数 ID `n` 与参数个数 `m`，MIR 栈模型无法推断返回值
个数。旧实现按 0 处理：若程序员在宏内使用返回值，后续栈模型会错位，且没有诊断。

## 决策

`stardust!` 的调用指令扩展为：

```freebird
Call(n, m)       // 兼容旧写法：Freebird 模型认为返回 0 个值
Call(n, m, r)    // 显式声明：Freebird 栈模型认为返回 r 个值
```

`r` **只影响 Freebird 的栈模型**（以及生成的注释），不改变生成的 Stardust
`Call(n, m)`；运行时实际返回值仍由被调 Stardust 函数决定。`r` 与实际返回值不一致
属于 `unsafe` 宏内程序员的责任（与 `stardust!` 的其他栈操作一致）。

## 备选方案

1. **禁止在 Call 后使用返回值**：过于严格，无法表达常见的“调用后直接输出”用法。
2. **从被调函数体自动推断**：外部 Stardust 函数可能不在当前源码中，不可行。
3. **新增独立指令**：与 Stardust 简写语法不一致，增加学习成本。

## 兼容性

- `Call(n, m)` 保持原语义（`r = 0`），现有程序不需要修改。
- VM 仍不能执行原始 Stardust 调用（`RawCallUnsupported`）；Stardust/LLVM 后端可执行。
- 新的 `Call(n, m, r)` 只影响 MIR 栈模型与注释，不改变生成代码。

## 测试

- `tests/m6_test.rs::stardust_call_arity_declares_return_values`：
  `Call(7, 2, 1)` 调用外部 `Add` 定义，输出 `8`；VM 仍报明确错误。
- `tests/m3_test.rs` 的 raw call 用例更新为显式 arity。
