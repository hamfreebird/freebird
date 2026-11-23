# ADR 0004：unsafe 值栈（M2-8 后续）

- 状态：已实施（2026-11-23）
- 相关：`readme.md` §4.2.3、§10.2、`src/io.rs`、`src/vm/mir.rs`、`src/vm/mod.rs`、`src/codegen/stardust.rs`

## 背景

`readme.md` §10.2 的 unsafe 块示例使用了 `push(1); push(2); swap(); let sum = pop() + pop();`。
这些操作的对象是一个**用户可见的值栈**，而不是编译器管理的求值栈。

第一版尝试把 `push`/`pop` 直接映射到 MIR 求值栈（`push` 不生成指令、`pop` 返回栈顶），
结果是语义错的：`swap()` 在 VM 报「栈上不足 2 个元素」、Stardust 输出错误值。
根因是求值栈的深度由编译器按表达式结构管理，用户无法在其上"压/弹"。

## 决策

为用户提供**独立的值栈**（raw stack），由新 MIR 指令驱动：

| 指令 | 语义 |
|---|---|
| `RawPush` | 把求值栈顶的值移入值栈 |
| `RawPop` | 把值栈顶移回求值栈 |
| `RawSwap` | 交换值栈顶两个元素（不足两个则运行时错误） |
| `RawDup` | 复制值栈顶（空栈则运行时错误） |
| `RawRoll` | 从求值栈取 `n`，按 `readme.md` §4.2.3 旋转值栈 |

内建函数 `push(x)`、`pop()`、`swap()`、`dup()`、`roll(n)` 分别生成上述指令（`push` 的实参先在
求值栈上，再 `RawPush` 移入值栈；`pop()` 的 `RawPop` 结果即函数返回值）。

`roll(n)` 遵循 `readme.md` §4.2.3：`n > 0` 把从栈顶数位置 `n` 的元素提到栈顶，`n == 0`
为 no-op，`n < 0` 把栈顶元素放到位置 `|n|`；`|n| >= 栈深` 是运行时错误（VM 报
`AssertionFailed`，Stardust 用除零陷阱）。

值栈**跨函数调用保留**，属于运行时的全局状态：

- VM：`Vm` 持有 `raw_stack: Vec<Value>`。
- Stardust：用一个堆块模拟 `[len, cap, elems...]`，栈顶下标存在固定的 scratch 槽，
  以便在无"全局变量"的解释器里跨越调用。初始容量 64，`RawPush`/`RawDup` 发现
  `len == cap` 时按 2 倍分配新块并拷贝现有元素（旧块不回收，见 ADR 0002 的保守策略）。

## 安全边界

- 这组内建属于 unsafe 语义：值栈上的值不参与类型检查，也不参与借用检查。
- 这组内建**只能在 `unsafe` 块内调用**：类型检查维护 `unsafe_depth`，在 `unsafe`
  之外调用 `push`/`pop`/`swap`/`dup`/`roll` 会报 `unsupported feature`（指向 `readme.md` §10.2）。
- `pop()` 在空值栈上、`swap()`/`dup()` 在元素不足时、`roll(n)` 在 `|n| >= 栈深` 时
  给出明确的运行时错误。

## 后果

- unsafe 块获得可用的底层栈操作，且 VM 与 Stardust 行为一致（由测试保证）。
- 值栈是运行时全局状态：递归或多函数混用时会共享同一个栈，这与 `readme.md`
  §10.2 "类型栈的维护由程序员负责"的表述一致。
- 值栈容量不再固定：Stardust 侧按需翻倍，因此长时间 `push` 不会撞上 64 的上限。
- `roll(n)` 已提供完整三向语义（正/零/负），见 `readme.md` §4.2.3。
