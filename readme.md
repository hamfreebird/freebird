# Freebird Language Specification

> **v0.5.0** — 自由如鸟，一切皆栈。
>
> Freebird 是一门基于 Stardust 多栈调用帧模型的高级系统编程语言。它继承 Stardust"所有计算皆为栈操作"的根本信条，引入**带标签的双栈数据模型**（ID 栈 + 值栈）与**变量栈顶化访问语义**，提供 Rust/Kotlin 风格的类型安全语法，同时保留对底层栈操作的完全控制。
>
> 本规范是 Freebird 语言实现的权威参考文档。
>
> **实现状态**：本文描述的是目标规范，部分特性尚未实现（例如泛型约束与按实例单态化、字符串格式化 API）。LLVM 后端与 `extern "stardust"` FFI 已在 M3 落地：LLVM 目标复用 Stardust 自带的 LLVM 编译器，因此与 Stardust 后端共享值布局与堆块语义；文件 I/O 在 LLVM 目标上编译期拒绝，非 ASCII `CharOut` 受 Stardust LLVM 的 ASCII 限制。Stardust 的“远程数据”已统一为全局堆块实现（§8.1，ADR 0001）。当前可用能力与已知限制以 `USAGE.md`、`STATUS.md` 和 `AGENTS.md` 为准；行为冲突时以 `src/` 与测试为最终依据。

---

## 目录

1. [设计哲学](#1-设计哲学)
2. [虚拟机执行模型](#2-虚拟机执行模型)
3. [核心数据模型：双栈与 ID](#3-核心数据模型双栈与-id)
4. [变量与栈顶化访问](#4-变量与栈顶化访问)
5. [类型系统](#5-类型系统)
6. [函数与多栈调用模型](#6-函数与多栈调用模型)
7. [复合数据类型的栈布局](#7-复合数据类型的栈布局)
8. [可变长度数据](#8-可变长度数据)
9. [所有权与引用](#9-所有权与引用)
10. [Unsafe 与 Stardust 互操作](#10-unsafe-与-stardust-互操作)
11. [语法参考](#11-语法参考)
12. [完整示例](#12-完整示例)
13. [工具链](#13-工具链)
14. [附录 A：指令速查表](#14-附录-a指令速查表)
15. [附录 B：与 Stardust 的对照](#15-附录-b与-stardust-的对照)

---

## 1. 设计哲学

Freebird 从 Stardust 继承了最根本的信条：**所有计算都是栈操作，所有结构都是栈**。

在这一基础上，Freebird 建立了三层抽象：

| 层次 | 内容 | 目标 |
|------|------|------|
| **数据层** | 双栈模型（ID 栈 + 值栈），每个值拥有全局唯一标识符 | 稳定引用、可追踪的值 |
| **语义层** | 栈顶化访问语义，变量是名字到 ID 的映射 | 保持栈式计算的纯粹性，同时提供高级语言的可用性 |
| **安全层** | 静态类型系统、所有权与借用规则 | 编译期保证内存安全，同时允许 unsafe 下潜 |

Freebird 让你能够：

- 在高层次上用接近 Rust 的语法编写结构化、类型安全的代码。
- 在低层次上直接操控栈，指定精确的元素移动，或直接嵌入 Stardust 指令。
- 享受原生的多值返回——函数可以返回任意数量的值。
- 利用唯一标识符（ID）实现值的稳定引用。
- 通过 unsafe 块实现极致的布局控制。
- 与 Stardust 生态完全互通。

---

## 2. 虚拟机执行模型

Freebird 语言在**基于栈的虚拟机**上运行。虚拟机维护以下核心组件：

| 组件 | 描述 |
|------|------|
| **全局数据栈注册表** (`GlobalRegistry`) | 记录所有活跃数据栈的注册表，每个数据栈由一个唯一的 `StackId`(u64) 标识 |
| **当前数据栈** | 当前执行帧的双层栈（ID 栈 + 值栈），所有计算在此发生 |
| **变量栈** (`VarStack`) | 当前作用域的名字到 ID 的映射表，由作用域帧栈管理 |
| **指令指针** (`PC`) | 指向当前执行的指令 |
| **调用帧栈** (`CallStack`) | 管理函数调用帧，帧 0 为主程序 |

### 2.1 编译流水线

```
Freebird 源码 → 词法分析 → 语法解析 → HIR → 类型检查 → MIR → 目标代码生成
                                                              ├── Stardust 源码
                                                              └── LLVM IR
```

- **HIR**（高级中间表示）：保留类型信息、变量名和作用域结构。
- **MIR**（中级中间表示）：将所有高级语法降级为带类型标注的栈指令序列。
- **目标代码**：生成 Stardust 源码（可被 Stardust 解释器/编译器执行）或 LLVM IR（可直接编译为二进制）。

### 2.2 运行时限制

运行时配置继承自 Stardust，支持通过 `#[key=value]` 注解覆盖：

| 配置项 | 默认值 | 说明 |
|--------|--------|------|
| `max_stack` | 1,048,576 | 单帧最大栈深度 |
| `max_call_depth` | 256 | 最大调用深度 |
| `max_steps` | 10,000,000 | 最大执行步数（0 = 无限制） |
| `time_limit` | 无限制 | 执行时间限制（秒） |

---

## 3. 核心数据模型：双栈与 ID

### 3.1 双栈结构

每个数据栈由两个**长度始终相同**的栈组成：

```
数据栈 ::= {
    id_stack:   Stack<ElementId>,   -- 元素的唯一标识符
    value_stack: Stack<Value>,      -- 元素的实际值
}
```

- 每个元素由一对 `(ElementId, Value)` 组成。
- `ElementId` 是全局唯一的 64 位无符号整数，单调递增，**永不回收**。
- 两个栈层的长度始终相等：`|id_stack| == |value_stack|`。
- 栈顶是最后一个被压入的元素，所有计算在栈顶进行。

### 3.2 基本栈操作

| 操作 | 效果 | 栈深度变化 |
|------|------|-----------|
| `push(value)` | 生成新 `ElementId`，将 `(new_id, value)` 压入双栈顶 | +1 |
| `pop()` | 移除栈顶的 `(id, value)` 对，`ElementId` 随之失效 | -1 |
| `dup()` | 复制栈顶元素（生成新 `ElementId`，复制值） | +1 |

`push` 和 `dup` 总是生成**新的** `ElementId`。`pop` 销毁 `ElementId`（该 ID 永不复用）。

### 3.3 ElementId 的唯一性

- `ElementId` 是全局唯一的 `u64`，由虚拟机单调递增分配。
- **永不回收**：即使元素被 `pop` 销毁，其 ID 也不会被重新使用。
- 这保证了任何持有 `ElementId` 的引用都能唯一确定地追踪一个值（即使该值已被销毁，引用变为悬空，ID 本身仍然是唯一的）。
- `ElementId` 命名空间与 `StackId` 命名空间**完全独立**，不会碰撞。

### 3.4 Copy 语义与 Move 语义

| 类别 | 类型 | 语义 |
|------|------|------|
| **Copy 类型** | `i8`–`i64`, `u8`–`u64`, `f32`, `f64`, `bool`, `char`, `()`, `&T` | `dup()` 后原值和副本各自独立，均有效 |
| **Move 类型** | `String`, `Vec<T>`, `Box<T>`, struct（非全 Copy）, enum（非全 Copy） | 赋值或传参时所有权转移，原绑定失效 |

Copy 类型在读取时可安全复制；Move 类型在读取时执行 `dup()` 但在语义上标记原绑定为"已移动"（编译器阻止后续使用）。

---

## 4. 变量与栈顶化访问

### 4.1 变量栈（VarStack）

每个作用域伴生一个**变量栈帧**：从变量名（字符串）到 `ElementId` 的映射。

```
VarStack ::= Stack<Map<String, ElementId>>
```

- 进入新作用域时压入空映射。
- `let x = expr;` 在**当前作用域帧**中插入绑定 `"x" → id_e`。
- 退出作用域时弹出当前帧（所有局部绑定自动失效）。
- 变量名遮蔽（shadowing）通过在当前作用域插入同名绑定实现，不影响外层。

### 4.2 栈顶化访问语义

在 Freebird 中，**所有计算都发生在栈顶**。通过变量名访问一个不在栈顶的值时，虚拟机会自动执行以下协议。

#### 4.2.1 读取变量（Read Context）

当表达式需要一个变量的值时（如 `x + 1` 中的 `x`），执行以下步骤：

```
read_var(name):
  1. 通过变量栈查找 name → id
  2. 在数据栈中定位该 id 的位置 pos（距离栈顶的深度，0 = 栈顶）
  3. 执行 roll(pos)    —— 将目标元素旋转到栈顶
  4. 执行 dup()        —— 复制栈顶（生成新 ElementId），副本作为读取结果
  5. 执行 roll(-pos)   —— 将原始元素旋转回其原始位置
  6. 返回栈顶的副本值
```

**复杂度**：变量访问为 O(pos)（基于栈扫描定位 id），pos 通常为小常数。

#### 4.2.2 写入变量（Write Context）

当对变量赋值时（`x = expr`，其中 `x` 为 `mut`），**先求值右侧表达式，再执行写入协议**：

```
write_var(name, expr):
  0. 计算表达式 expr，结果（携带新 ElementId id_new）留在栈顶
  1. 通过变量栈查找 name → id_old
  2. 在数据栈中定位旧 id 的位置 pos（距离栈顶的深度，0 = 栈顶）
  3. 执行 roll(pos)    —— 将旧值旋转到栈顶（expr 结果随之移动到 pos=1）
  4. 执行 pop()        —— 丢弃旧值（旧 ElementId 失效, expr 结果回到栈顶）
  5. 执行 roll(-pos)   —— 将新值（expr 结果）旋转回旧值的原始位置
  6. 更新变量栈：name → id_new
```

**关键不变式**：写入变量不改变栈深度，不影响其他变量的位置。此协议正确处理自引用赋值（如 `x = x + 1`，右侧的 `x` 在步骤 0 中通过 read_var 读取，此时旧值尚未被 pop）。

#### 4.2.3 `roll(n)` 的精确语义

`roll` 是栈顶化访问的核心原语。Stardust VM 的 `Rotate` 指令仅能旋转栈顶 3 个元素，Freebird 将 `roll` 定义为一个新的通用原语：

```
roll(n) 其中 n ∈ ℤ（可为正、零、负）

定义：将栈视为 0 索引（0 = 栈顶，1 = 栈顶以下第 1 个，以此类推）

roll(+n), n > 0:
  设目标位置 t = n（从栈顶向下数）
  将元素 e = stack[t] 提取出来
  位置 [0, t-1] 的所有元素向下移动一个位置（索引各自 +1）
  将 e 放到栈顶（新位置 0）

roll(0):
  no-op（栈顶元素已在栈顶）

roll(-n), n > 0:
  将栈顶元素 e = stack[0] 提取出来
  位置 [1, n] 的所有元素向上移动一个位置（索引各自 -1）
  将 e 放到位置 n
```

**性质**：`roll(n)` 与 `roll(-n)` 互为逆操作。即 `roll(n); roll(-n);` 在任何栈状态下等价于 no-op。

**示例**：
```
初始栈（顶→底）:        [d, c, b, a]
roll(2):                将 b（位置 2）提取到栈顶
  → [b, d, c, a]
roll(-2):               将 b（栈顶）放到位置 2
  → [d, c, b, a]       恢复原状
```

### 4.3 元素 ID 访问（不支持）

语言**不提供**原始元素 ID 的访问。`#x`、`*(#x)` 与 `current_stack_id()`
属于实现内部的双栈细节（`ElementId`），无法在 Stardust 后端表达，因此不作为
语言特性提供：

- `#x` 与 `*(#x)` 在编译期报 `unsupported feature`（见 §10.1）；
- 需要底层栈操作时使用 `unsafe` 块中的低级栈内建（见 §10.2）。

## 5. 类型系统

### 5.1 基本类型

| 类型 | 说明 | 宽度 | 分类 |
|------|------|------|------|
| `i8`, `i16`, `i32`, `i64`, `isize` | 有符号整数 | 8/16/32/64/指针宽 | Copy |
| `u8`, `u16`, `u32`, `u64`, `usize` | 无符号整数 | 8/16/32/64/指针宽 | Copy |
| `f32`, `f64` | IEEE 754 浮点数 | 32/64 | Copy |
| `bool` | 布尔值 (true/false) | 1 (存储为 i64) | Copy |
| `char` | Unicode 标量值 | 4 (存储为 i64) | Copy |
| `()` | 单元类型，零大小 | 0 | Copy |
| `!` | Never 类型（发散表达式） | — | — |
| `ElementId` | 内部元素标识符（语言不暴露，见 §4.3） | 8 (u64) | Copy |

**整数语义**：所有窄于 64 位的整数类型在栈上存储为完整的 i64，算术运算在编译期插入截断/扩展指令以确保语义正确。溢出行为：debug 模式下 panic，release 模式下 wrapping。

### 5.2 复合类型（固定大小）

| 类型 | 示例 | 栈槽数 | 说明 |
|------|------|--------|------|
| 元组 | `(i32, bool)` | 2 | 各元素独立 ID，按序排列 |
| 结构体 | `struct Point { x: i32, y: i32 }` | 2 | 字段按声明顺序排列 |
| 固定数组 | `[i32; 5]` | 5 | 连续槽位 |
| 枚举 | `enum Option<T> { None, Some(T) }` | 1 + max | 见 5.3 |
| 引用 | `&T`, `&mut T` | 1 | 内部表示为 `ElementId` |
| 函数指针 | `fn(A, B) -> C` | 1 | 函数标识符 |

### 5.3 枚举类型（Enum）

枚举的栈布局如下：

```
enum Option<i32> {
    None,        // 变体 0：无附加数据
    Some(i32),   // 变体 1：一个附加数据
}
```

**布局规则**：
1. 第一个槽为**标签（tag）**：`i32` 类型，从 0 开始按变体声明顺序编号。
2. 后续槽为联合数据区，占据最大变体的槽数。
3. 枚举总槽数 = `1 + max(各变体数据槽数)`。

```
Option<i32>  栈布局:
  None       → [tag=0, padding=0]      占用 2 槽
  Some(42)   → [tag=1, 42]             占用 2 槽

Result<i32, bool>:
  Ok(10)     → [tag=0, 10]             占用 2 槽（max(1, 1) = 1）
  Err(true)  → [tag=1, 1]              占用 2 槽
```

**模式匹配**读取 tag 槽以确定活动变体，然后按变体类型解析数据区。

### 5.4 类型栈（Type Stack）

编译器在每个程序点维护一个**虚拟类型栈**，其深度和类型布局与运行时的值栈对应：

```
值栈:           [42, true, "hello"]
ID 栈:          [id0, id1, id2]
类型栈:         [i32, bool, String]
```

- 每条指令的操作数类型由类型栈顶部的元素类型确定。
- 栈操作指令（push, pop, dup, roll）会相应地更新类型栈。
- Safe 代码中，编译器确保操作前后类型栈的局部一致性。
- Unsafe 代码中，类型栈的维护责任交给程序员。

### 5.5 用户泛型（类型擦除 MVP）

结构体、枚举与函数可以声明类型参数：

```freebird
struct Pair<T> { first: T, second: T }
enum Maybe<T> { Nothing, Just(T) }
fn id<T>(x: T) -> T { x }
```

- 类型实参由结构体字段访问或函数调用实参推断，并在类型检查中做替换检查。
- 布局与代码按基名共用一份（**类型擦除**，不按实例单态化）。
- 显式类型实参使用 turbofish：`f::<T>(...)`、`Pair::<T>::new(...)`。暂不支持约束与按实例单态化；泛型函数体只能使用与具体类型无关的操作。
- `impl` 暂不接受类型参数（见 §5.6）。

### 5.6 用户方法与 `impl`

可以为结构体/枚举声明关联函数与方法：

```freebird
impl Rect {
    fn square(side: i64) -> Rect { Rect { w: side, h: side } }  // 关联函数
    fn area(&self) -> i64 { self.w * self.h }                    // 只读方法
    fn grow(&mut self, by: i64) { self.w = self.w + by; }        // 写方法
}
```

- 声明内的名字在编译期扁平化为 `Type::name`；关联函数用 `Type::f(...)` 调用。
- 方法调用 `x.f(...)` 按接收者类型解析：用户方法优先，未命中才回落到内建方法（`Vec`/`String`/`Option`/`Result`）。
- 接收者可为 `self`、`&self` 或 `&mut self`；`self.field` 自动按接收者解引用，`&mut self` 的字段写入会回写到调用方变量。
- `impl` 暂不接受泛型参数。决策与实现边界见 `docs/adr/0003-user-impl-methods.md`。

---

## 6. 函数与多栈调用模型

### 6.1 调用元素机制

函数调用通过一个特殊的栈元素——**调用元素（Call Element）**——实现。整个流程如下：

**步骤 1：生成调用元素**

在调用者数据栈上 `push` 一个调用元素：
- 其值为**参数数量 `n`**（`i32` 类型）。
- 其 ID 由系统生成，记为 `call_id`。
- 调用元素为内部元素，safe 代码不可直接访问。

**步骤 2：派生函数栈 ID**

被调函数的数据栈 `StackId` 由以下公式生成：
```
callee_stack_id = hash(call_id || recursion_seed)
```
其中 `recursion_seed` 为每次函数调用递增的计数器，确保每次调用（包括递归）拥有独立的栈帧。

**步骤 3：参数准备**

根据调用方式（见 6.2 节），将 `n` 个参数值依次压入调用者栈顶。压入后栈布局为：

```
调用者栈: [...之前的元素..., call_element(n, call_id), param_1, param_2, ..., param_n]
                                                                                    ↑ 栈顶
```

- 参数以**从左到右**的顺序压入：第一个参数最先压入，最后一个参数在栈顶。
- 参数传递遵循所有权规则（Move 类型转移所有权，Copy 类型复制）。

**步骤 4：帧创建**

从调用者栈顶弹出 `n` 个参数元素（ID+值对），然后弹出调用元素。将这些元素按以下顺序移入新函数数据栈：

```
被调函数栈（底→顶）:
  [call_element(n, call_id), param_1, param_2, ..., param_n]
   slot 0                    slot 1   slot 2         slot n
```

同时创建新的变量栈帧，将函数签名中的参数名依次绑定到各参数的 `ElementId`：

```
参数绑定:
  fn foo(a: i32, b: i32)  →  VarStack 插入 "a" → param_1.id, "b" → param_2.id
```

调用元素的信息（`ret_pc` = 调用者 Call 指令的下一条 PC）存入新帧的元数据中。调用元素占据 slot 0，但其 ElementId 不暴露给变量栈——safe 代码无法通过名字访问 slot 0。

**步骤 5：执行函数体**

函数体在新栈上执行。局部 `let` 绑定在当前栈上 push 新元素并在变量栈帧中记录映射。

**步骤 6：返回**

执行 `ret m`（`m` 为返回值个数）：
1. 检查栈顶 `m` 个元素的类型是否匹配函数签名的返回类型（编译期验证）。
2. 保留栈顶 `m` 个元素。
3. 丢弃栈上其余所有元素（包括局部变量、调用元素），弹出变量栈帧。
4. 将保留的 `m` 个元素按栈顶顺序移回调用者栈顶。
5. 被调函数栈帧从全局注册表中移除。
6. 调用者 PC 恢复为 `ret_pc`。

```
返回前被调函数栈: [call_element, param_1, param_2, local_a, local_b, ret_1, ret_2]
                                                                            ↑ 栈顶
                                       |---- 保留 m=2 个 ---|

返回后调用者栈:   [...之前的元素..., ret_1, ret_2]
                                          ↑ 栈顶（新）
```

### 6.2 参数传递方式

调用函数时，参数传递有三种形式：

| 方式 | 语法示例 | 语义 |
|------|---------|------|
| **命名参数** | `divrem(a: x, b: y)` | 显式指定每个参数的来源，编译器按声明顺序压栈 |
| **位置参数** | `divrem(x, y)` | 按顺序将变量 `x`, `y` 的值压栈 |
| **栈传参** | `divrem()` | 省略所有参数，直接使用当前栈顶的 `n` 个元素作为参数（`n` 由函数签名确定） |

**命名参数**和**位置参数**不能混用。栈传参要求调用前栈顶 `n` 个元素的类型与函数签名**精确匹配**，由编译期静态验证。

栈传参是 Stardust `Call(n, m)` 的直接映射，允许调用者在调用前手动编排栈顶。

**实现注记**：命名参数和位置参数先 push 调用元素再 push 参数；栈传参的参数已在栈顶，VM 直接从栈顶弹出 n 个参数并在新帧中隐式创建调用元素。两种方式在语义上等价——被调函数看到的帧结构完全一致。

### 6.3 函数签名的 `n` 和 `m`

高级语法中参数个数 `n` 和返回值个数 `m` 由函数签名推导：

```freebird
fn divrem(a: i32, b: i32) -> (i32, i32) { ... }
//     n = 2            m = 2
```

- `n` = 参数列表中参数的数量。
- `m` = 返回类型元组中的元素数量（`-> T` 等同于 `-> (T,)`，即 m=1）。
- 无返回类型标注等价于 `-> ()`，即 m=0。

### 6.4 `ret m` 指令

`ret m` 在 safe 代码中可直接使用（不需要 unsafe 块），前提是：

1. `m` 是编译期常量。
2. `m` 与函数签名中的返回值数量一致。
3. 栈顶 `m` 个元素的类型与声明的返回类型匹配。

上述条件由编译器静态验证。如果需要在运行时动态确定返回值个数，必须使用 unsafe 块。

### 6.5 函数解析顺序

Freebird 采用**单遍声明收集 + 单遍类型检查**：

1. **声明收集遍**（Declaration Pass）：扫描整个编译单元，收集所有函数签名。
2. **类型检查遍**（Type Check Pass）：在已知所有签名的基础上进行类型检查。

因此函数可以在定义之前被调用（前向引用），只要在同一编译单元内。跨模块调用需要通过 `use` 导入。

模块内声明**默认私有**：只有加了 `pub` 的 `fn`/`struct`/`enum` 才能被其它文件通过
`use 模块::名字;` 使用，否则编译期报错。`mod name;` 加载同目录 `name.fb`，模块内的
名字统一加 `模块::` 前缀（含结构体/枚举），循环依赖会报错。

---

## 7. 复合数据类型的栈布局

### 7.1 结构体（Struct）

```freebird
struct Point {
    x: i32,
    y: i32,
}
```

**布局**：`[x, y]`，占 2 个栈槽。字段按声明顺序排列，每个字段拥有独立的 `ElementId`。

结构体的模式解构：
```freebird
let Point { x, y } = p;    // x 绑定到 p.x 的 ElementId，y 绑定到 p.y 的 ElementId
```

### 7.2 元组（Tuple）

```freebird
let t: (i32, bool, char) = (42, true, 'A');
```

**布局**：`[42, true, 'A']`，占 3 个栈槽。元素按声明顺序排列。

元组解构：
```freebird
let (a, b, c) = t;    // a=42, b=true, c='A'
```

### 7.3 固定长度数组

```freebird
let arr: [i32; 4] = [1, 2, 3, 4];
```

**布局**：`[1, 2, 3, 4]`，占 4 个栈槽。

索引访问 `arr[i]` 在运行期动态确定位置，触发栈顶化访问。越界访问在 debug 模式下 panic。

### 7.4 枚举（Enum）详细布局

```
enum Color {
    Red,           // tag=0, 数据槽: 0
    Green,         // tag=1, 数据槽: 0
    Blue,          // tag=2, 数据槽: 0
    Rgb(u8,u8,u8), // tag=3, 数据槽: 3
}

栈布局:
  Red          → [tag=0, pad, pad, pad]  ← 占 max(0,3)+1 = 4 槽
  Green        → [tag=1, pad, pad, pad]
  Blue         → [tag=2, pad, pad, pad]
  Rgb(r,g,b)   → [tag=3, r, g, b]
```

枚举总槽数 = `1 + max(各变体数据槽数)`。标签槽始终存在，类型为 `i32`。数据槽中的填充（padding）值未定义。

### 7.5 引用类型（`&T` / `&mut T`）

引用在栈上占据**1 个槽**，其内部表示为 `(ElementId, type_tag)`：

- `&T`：共享引用，允许读取但不允许修改。多个 `&T` 可同时存在。
- `&mut T`：独占引用，允许读取和修改。同一时刻仅允许一个 `&mut T`。

引用的创建：
```freebird
let x = 42;
let r: &i32 = &x;       // r 存储 x 的 ElementId
let val = *r;            // 通过引用读取（触发栈顶化访问）
```

引用始终指向某个栈中的元素。借用检查器确保被引用元素的生命周期长于引用本身。

---

## 8. 可变长度数据

### 8.1 逻辑远程栈模型（全局堆块实现）

可变长度类型（`Vec<T>`, `String`, `Box<T>` 等）在**本地栈**上仅占据 **1 个槽**，保存一个**堆块句柄**（块起始地址），指向全局堆上的数据块。语言层面仍称这部分数据为“远程数据”，但实现上不要求后端提供独立的远程栈原语；详见 `docs/adr/0001-logical-remote-stack.md`。

```
本地栈:  [... , block_handle, ...]
                      |
                      └──→ 全局堆块: [length, capacity, elem_0, elem_1, ..., elem_{cap-1}]
                                        ↑ slot 0  ↑ slot 1  ↑ slot 2       ↑ slot cap+1
```

**堆块布局**（长度前缀模型）：

| Slot | 内容 | 类型 | 说明 |
|------|------|------|------|
| 0 | `length` | `usize` | 当前有效元素数量 |
| 1 | `capacity` | `usize` | 已分配的元素槽数 |
| 2 | `elem_0` | T | 第一个元素 |
| ... | ... | T | 后续元素 |
| cap+1 | `elem_{cap-1}` | T | 最后一个已分配槽 |

远程数据块总深度 = `2 + capacity`。

### 8.2 Vec\<T\>

```freebird
let mut v: Vec<i32> = vec![1, 2, 3];
```

创建 `vec![1, 2, 3]` 时：
1. 在全局堆上分配一个数据块。
2. 在块中写入 `length=3`, `capacity=3`, `1`, `2`, `3`。
3. 在本地栈上压入块起始地址作为句柄，绑定到变量 `v`。

**操作语义**：

```
v.push(4):
  1. 从本地栈读取 v → 块句柄
  2. 在数据块上：length += 1
  3. 如果 length > capacity：
     a. 分配新的数据块，容量翻倍（最小 4）
     b. 将旧块元素复制到新块
     c. 释放旧块（Stardust 后端当前只增不减，见 M1-3）
     d. 句柄更新为新块地址（本地栈上的槽值更新）
  4. 在数据块的位置 (1 + length_old) 处写入新元素

v.pop():
  1. 从本地栈读取 v → 块句柄
  2. 读取数据块位置 (1 + length - 1) 处的值
  3. length -= 1
  4. 返回读取的值（包装为 Option<T>，栈空时返回 None）

v[2]:
  1. 从本地栈读取 v → 块句柄
  2. 在数据块上定位位置 (2 + 2) = slot 4
  3. 读取该元素
  4. 将值放到本地栈顶作为结果
```

### 8.3 String

`String` 是 `Vec<u8>` 的包装，增加了 UTF-8 有效性保证：

```freebird
let s: String = "hello";
```

- 数据块中存储 UTF-8 编码的字节序列。
- `length` 和 `capacity` 以字节为单位。
- 提供 `s.len()`, `s.push(ch)`, `s.as_bytes()` 等方法。

### 8.4 Box\<T\>

`Box<T>` 将任何值分配到独立的数据块上：

```freebird
let b: Box<i32> = Box::new(42);
```

- 数据块布局简单：`[value]`（仅一个元素，不需要长度/容量前缀）。
- `Box` 用于将数据放在堆上（逻辑远程栈），在本地栈上保持小尺寸。
- 解引用 `*b` 读取数据块上的值并执行栈顶化访问。

### 8.5 数据块的生命周期

- 创建：通过 `vec![]`, `String::new()`, `Box::new()` 等分配。
- 销毁：当持有句柄的变量离开作用域时（或所有权被转移后原变量被析构时），后端应回收该数据块。VM 中的值随栈槽释放；Stardust 后端的全局堆使用 free list 复用，`VecPush`/`VecPop` 原地操作并在扩容时释放旧块，`DropVar` 回收未被返回/别名的 Vec/Box 绑定。数组/结构体等 Copy 类型的 COW 旧块与嵌套容器内层块保守不释放（见 ADR 0002）。
- 所有权转移：句柄本身遵循 Move 语义，赋值或传参后原变量失效。

---

## 9. 所有权与引用

### 9.1 所有权规则

1. Freebird 中的每个值都有**唯一的拥有者栈**（owner stack）——即值当前所在的栈。
2. 当拥有者变量离开作用域时，其值被析构（从栈上移除，远程栈被释放）。
3. 赋值（`let y = x;`）的行为取决于类型：
   - **Copy 类型**：值被复制（`dup`），`x` 和 `y` 各自独立有效。
   - **Move 类型**：所有权转移，`x` 不再可用（编译器阻止后续访问）。

```freebird
let x: i32 = 42;       // i32 是 Copy 类型
let y = x;             // x 的值被复制
let z = x + y;         // ✓ x 仍然有效

let a: String = "hi";  // String 是 Move 类型
let b = a;             // a 的所有权转移到 b
// let c = a;          // ✗ 编译错误：a 已被移动
```

### 9.2 函数传参与所有权

```
fn consume(s: String) { ... }     // 接收所有权
fn borrow(s: &String) { ... }     // 共享借用
fn borrow_mut(s: &mut String) { ... } // 独占借用
```

- 传值（`s: String`）：所有权转移到函数，调用者失去该值。
- 传共享引用（`s: &String`）：函数可读不可改，调用者保留所有权。
- 传独占引用（`s: &mut String`）：函数可读写，调用者在借用期间不可访问。

### 9.3 借用规则

1. 在任何给定时间，只能存在以下之一：
   - 任意数量的共享引用（`&T`）。
   - 恰好一个独占引用（`&mut T`）。
2. 引用必须始终有效（被引用元素的生命周期 ≥ 引用的生命周期）。
3. 在独占引用存在期间，拥有者不可通过原名访问该值。

借用检查器在编译期检查上述规则，具体算法超出本文档范围（参考 Rust 的 NLL 借用检查）。

### 9.4 析构（Drop）

值在离开作用域时按与创建相反的顺序析构：

- **Copy 类型**：简单地从栈上 pop（无副作用）。
- **Move 类型**：调用其析构逻辑。对于 `Vec<T>`/`String`/`Box<T>`，析构时释放其远程栈。
- **复合类型**：先析构所有字段/元素，再析构结构体自身。

---

## 10. Unsafe 与 Stardust 互操作

### 10.1 Safe/Unsafe 边界标准

一条操作是 **safe** 的**当且仅当**编译器可以静态验证操作前后**类型栈保持兼容**。

| 类别 | Safe 示例 | Unsafe（需要 unsafe 块） |
|------|----------|--------------------------|
| 栈操作 | `let x = 10;` (typed push) | `push(10);` (无类型标注的 raw push) |
| 栈操纵 | 编译器自动 roll/dup/roll(-pos) | 手动 `roll(3); swap();` |
| 跨类型 | 不允许 | `unsafe { let x: i32 = pop(); }` |
| 函数调用 | `divrem(10, 3)` | 动态确定 n/m 的调用 |
| 返回 | `ret 3`（m 匹配签名） | 不提供：`ret` 只接受整数字面量（M0-1） |
| 栈 ID | 不提供（编译期拒绝） | — |

### 10.2 Unsafe 块

```freebird
unsafe {
    push(1);
    push(2);
    swap();
    let sum = pop() + pop();
}
```

在 unsafe 块内：
- 提供低级栈内建 `push(x)`、`pop()`、`dup()`、`swap()`、`roll(n)`（`roll` 的精确语义见 §4.2.3）。
- 类型栈的维护由程序员负责。
- 编译器不保证类型安全，错误操作可能导致未定义行为。
- Unsafe 块可调用 safe 函数，但反过来不行。

### 10.3 `stardust!` 宏

`stardust!` 宏允许直接嵌入 Stardust **REPL 简写语法**的指令序列：

```freebird
let result = stardust! {
    Push(3)     // stack: [3]
    Push(5)     // stack: [3, 5]
    Add         // stack: [8]       (pop 5,3; push 3+5)
    Dup         // stack: [8, 8]    (copy top)
    Mul         // stack: [64]      (pop 8,8; push 8*8)
};
// result = 64
```

宏内部为 unsafe 上下文（编译器不检查类型栈）。当前支持的简写指令：

- 栈与运算：`Push(n)`、`Pop`、`Dup`、`Swap`、`Add`、`Sub`、`Mul`、`Div`、`Mod`、`Eq`、`Ne`、`Lt`、`Gt`、`Le`、`Ge`、`And`、`Or`、`Not`、`Xor`
- 栈扩展：`ShiftL`、`ShiftR`、`Depth`、`Pick`、`DropN`、`Reverse`、`Rotate`
- 堆与控制流：`Store`、`Load`、`Mark(n)`、`Jump(n)`、`UncondJump(n)`、`Call(n, m)`、`Ret(m)`
- 调试与 I/O：`NumOut`、`NumIn`、`CharOut`、`CharIn`、`DumpStack`、`DumpState`、`Breakpoint`、`Nop`

未知指令名在解析阶段报错。宏入口/出口会临时隐藏 Freebird 内部用于参数传递的调用元素
（`CallElement`，暂存在 Stardust 堆槽 13），因此 `ShiftL`/`Depth`/`Reverse` 等整栈指令
只操作用户数据栈，VM 与 Stardust 输出一致。`Call(n, m)` 从栈顶取 `m` 个参数，
返回值个数由被调 Stardust 函数决定，Freebird 栈模型按 0 处理；该指令在 VM 上不可执行。

### 10.4 `extern "stardust"` FFI

声明外部 Stardust 函数：

```freebird
extern "stardust" {
    #[stardust(id = 1)]
    fn sd_add(a: i32, b: i32) -> i32;

    #[stardust(id = 2)]
    fn sd_print_char(c: i32);           // 无返回值 → m=0
}
```

- `#[stardust(id = N)]` 指定 Stardust 函数的数字 ID。ID 会变成生成源码中的前导空格数，因此上限为 4096；重复 ID 编译失败。
- 若省略 `id`，则从函数名使用 SipHash-2-4 派生，并映射到 `1..=999` 作为默认 ID。
- 参数和返回值类型必须显式标注。
- 调用 Stardust 函数时，Freebird 编译器生成标准的 `Call(N, M)` 指令；内部 Freebird 函数名会自动避开这些 ID。
- Stardust 函数天然支持多值返回；返回 `()` 等价于 `m=0`。
- 所有调用必须在 `unsafe` 块内。VM 无法执行 Stardust 函数，`freebird run` 会在运行时报明确错误；`--target llvm` 需要外部 Stardust 函数定义参与链接。

调用示例：

```freebird
unsafe {
    let sum = sd_add(3, 5);
    sd_print_char(72);     // 输出 'H'
}
```

由于 Stardust 函数无类型安全保障，所有 FFI 调用必须在 unsafe 块中。

---

## 11. 语法参考

### 11.1 顶层声明

```freebird
// 函数声明（可带类型参数）
fn name(params) -> ReturnType {
    statements...
    expression  // 尾表达式作为隐式返回值（当无显式 ret 时）
}
fn name<T>(params) -> ReturnType { ... }

// 结构体声明
struct Name {
    field: Type,
    ...
}
struct Name<T> {
    field: T,
    ...
}

// 用户关联函数与方法
impl Type {
    fn assoc(params) -> ReturnType { ... }   // Type::assoc(...)
    fn method(&self) -> ReturnType { ... }   // x.method()
    fn method_mut(&mut self) { ... }         // x.method_mut()
}

// 枚举声明
enum Name<T> {
    Variant1,
    Variant2(Type),
    ...
}

// 外部 Stardust 函数
extern "stardust" {
    #[stardust(id = N)]
    fn name(params) -> ReturnType;
}

// 模块导入
use module::name;
```

### 11.2 语句

```freebird
let x = expr;                  // 不可变绑定
let mut x = expr;              // 可变绑定
let (a, b) = expr;             // 元组解构
let Point { x, y } = expr;    // 结构体解构

x = expr;                      // 赋值（仅 mut）

if cond { ... }
else if cond { ... }
else { ... }

while cond { ... }
loop { ... break value; }
for pattern in iterable { ... }  // 数组、Vec、String；支持 continue/break

match expr {
    pattern => expr,
    pattern if guard => expr,
    _ => expr,
}

return expr1, expr2, ...;      // 多值返回（safe 语法）
ret n;                          // 低级返回（safe 当 n 匹配签名）
expr as Type;                   // 标量类型转换（数值、char、bool）
```

### 11.3 表达式

```freebird
// 字面量
42, 0xFF, 0o77, 0b1010          // 整数
3.14, 1.5e10                    // 浮点数
true, false                     // 布尔
'A', '猫', '\n', '\u{1F600}'    // 字符
"hello", "line\nbreak"         // 字符串
#var                             // ElementId 字面量

// 运算符（优先级从低到高）
=                                // 赋值
||                               // 逻辑或
&&                               // 逻辑与
==, !=, <, >, <=, >=            // 比较
+ , -                            // 加、减
* , / , %                        // 乘、除、取模
! , - (一元)                     // 逻辑非、算术负
* (解引用)                       // 解引用
&, &mut                          // 引用
. (字段访问)                     // 字段访问
[] (索引)                        // 索引
() (调用)                        // 函数调用

// 构造
StructName { field: value, ... }
EnumName::Variant(value)
(value, value, ...)
[value; N]
[value, value, ...]

// 控制流表达式
if cond { a } else { b }
loop { break value; }
expr as Type
```

### 11.4 Unsafe 与 Stardust

```freebird
unsafe {
    push(1);
    push(2);
    swap();
    pop();
    roll(3);
    roll(-3);     // roll 逆操作
    let v = stardust! { Push(5) Push(3) Add };
}
```

### 11.5 注释

```freebird
// 单行注释
/* 多行注释 */
```

---

## 12. 完整示例

### 12.1 多值返回与栈传参

```freebird
/// 同时计算和、差、积
fn calculate(x: i32, y: i32) -> (i32, i32, i32) {
    let sum = x + y;
    let diff = x - y;
    let prod = x * y;

    // 此时栈（顶→底）: [prod, diff, sum, y, x, call_element]
    // 我们希望返回 diff, prod, sum（演示栈操纵）
    unsafe {
        roll(2);   // sum(位置2) → 栈顶: [sum, prod, diff, y, x, call_element]
    }
    unsafe {
        roll(1);   // prod(位置1) → 栈顶: [prod, sum, diff, y, x, call_element]
        roll(2);   // diff(位置2) → 栈顶: [diff, prod, sum, y, x, call_element]
    }
    ret 3;         // 返回栈顶 3 个: diff, prod, sum
}

/// 使用栈传参
fn main() {
    let (a, b, c) = calculate(10, 5);
    // a = 5 (diff), b = 50 (prod), c = 15 (sum)

    // 准备栈传参（参数从左到右压入：第一个参数在栈深处，最后一个在栈顶）
    unsafe {
        push(2);   // x = 2（第一个参数，先压入）
        push(c);   // y = 15（第二个参数，后压入，在栈顶）
    }
    // 栈: [..., 2, 15]，栈顶为 15
    let (x, y, z) = calculate();  // 栈传参: 取栈顶 n=2 个元素
    // x = -13 (2-15), y = 30 (2*15), z = 17 (2+15)
}
```

### 12.2 Vec 操作

```freebird
fn vec_demo() {
    let mut v: Vec<i32> = vec![1, 2, 3];
    v.push(4);
    v.push(5);
    // v 现在包含 [1, 2, 3, 4, 5]

    let third: i32 = v[2];     // third = 3
    let popped = v.pop();      // popped = Some(5)

    // 遍历
    for elem in &v {
        print(elem);
    }
}
```

### 12.3 枚举与模式匹配

```freebird
enum Option<T> {
    None,
    Some(T),
}

fn divide(a: i32, b: i32) -> Option<i32> {
    if b == 0 {
        Option::None
    } else {
        Option::Some(a / b)
    }
}

fn try_divide(a: i32, b: i32) {
    match divide(a, b) {
        Option::Some(val) => print(val),
        Option::None => print("division by zero"),
    }
}
```

### 12.4 Stardust FFI

```freebird
extern "stardust" {
    #[stardust(id = 1)]
    fn sd_push_and_add(x: i32, y: i32, z: i32) -> i32;
}

fn use_ffi() -> i32 {
    unsafe {
        sd_push_and_add(1, 2, 3)   // 调用 Stardust 函数 1
    }
}
```

---

## 13. 工具链

| 命令 | 说明 |
|------|------|
| `freebird run <file.fb>` | 解释执行 Freebird 程序 |
| `freebird run --debug <file.fb>` | 调试模式执行 |
| `freebird build <file.fb>` | 编译为 Stardust 源码（默认目标） |
| `freebird build --target stardust <file.fb>` | 编译为 Stardust 源码（可在 Stardust 解释器运行） |
| `freebird build --target llvm <file.fb>` | 编译为 LLVM IR（`-o` 写文件，默认打印到 stdout） |
| `freebird build --target llvm --emit exe -o <out> <file.fb>` | 生成 LLVM IR → 可执行文件 |
| `freebird check <file.fb>` | 语法和类型检查（JSON 诊断输出，含 `schema_version` 与稳定错误码） |
| `freebird fmt <file.fb>` | 源码格式化（`--check`/`--write`/`--stdin`） |
| `freebird repl` | 交互式 REPL |
| `freebird package` | 包管理 |

源文件扩展名：`.fb`

`freebird check` 输出 JSON：顶层 `schema_version`（当前 1）、`source`、`errors`、`error_count`、`warning_count`；每条诊断含 `level`、`code`、`message`、`span`、可选 `suggestion`/`related`。错误码前缀：`P` 词法/语法、`R` 名称解析、`T` 类型、`B` 借用、`W` 警告。

---

## 14. 附录 A：指令速查表

### A.1 Safe 栈指令（类型安全的 VM 指令）

这些指令由编译器从高级语法生成，也可在 unsafe 块中手动使用（无类型检查）。

| 指令 | 操作数 | 说明 |
|------|--------|------|
| `push_T(value)` | 1 个值的槽 | 压入类型为 T 的值，生成新 ElementId |
| `pop_T()` | — | 弹出栈顶 T 类型元素 |
| `dup_T()` | — | 复制栈顶（新 ElementId） |
| `roll(n)` | n: i32 | 将位置 n 的元素旋转到栈顶（n 可为负） |
| `swap()` | — | 交换栈顶两个元素 |
| `pick(n)` | n 在栈顶 | 复制栈顶以下第 n 个元素（0=栈顶），弹出 n |
| `call f, n` | f: 函数标识符 | 调用函数 f，从栈顶取 n 个参数 |
| `ret n` | n: 立即数 | 返回，保留栈顶 n 个元素 |
| `drop_n(n)` | n 在栈顶 | 弹出 n，丢弃栈顶 n 个元素 |

### A.2 Stardust 兼容指令

以下指令直接映射到 Stardust 指令，仅在 unsafe 块中可用：

| 指令 | Stardust 等价 | 说明 |
|------|--------------|------|
| `push(v)` | `Push(v)` | 无类型压入 i64 |
| `pop()` | `Pop` | 弹出并丢弃 |
| `dup()` | `Dup` | 复制栈顶 |
| `swap()` | `Swap` | 交换栈顶两个 |
| `add/sub/mul/div/mod` | `Add/Sub/Mul/Div/Mod` | 算术运算 |
| `eq/ne/lt/gt/le/ge` | `Eq/Ne/Lt/Gt/Le/Ge` | 比较 |
| `and/or/not/xor` | `And/Or/Not/Xor` | 逻辑运算 |
| `store/load` | `Store/Load` | 堆操作 |
| `shift_l/shift_r` | `ShiftL/ShiftR` | 循环移位 |
| `depth()` | `Depth` | 压入栈深度 |
| `reverse()` | `Reverse` | 反转整个栈 |

---

## 15. 附录 B：与 Stardust 的对照

| 概念 | Stardust (v0.1.5) | Freebird (v0.5.0) |
|------|-------------------|-------------------|
| 栈结构 | 单层值栈 `Vec<i64>` | 双层（ID 栈 + 值栈），每元素有唯一 ID |
| 类型 | 无类型（一切皆为 i64） | 静态类型系统，多宽度整数、浮点、复合 |
| 变量 | 无（直接操作栈） | 名字到 ID 的变量栈，自动栈顶化访问 |
| 函数标识 | 数字 ID（usize） | 命名函数，`#[stardust(id=N)]` 映射数字 ID |
| 调用模型 | 多栈调用帧，返回时合并内容 | 同模型，增加调用元素记录参数个数 |
| 多值返回 | 天然支持（帧合并） | 天然支持，语法 `-> (A, B)` + `ret n` |
| 内存安全 | 无保障 | Safe/Unsafe 分离，所有权与借用 |
| 堆 | `HashMap<i64, i64>` | 全局堆块（逻辑远程栈） |
| 可变长度 | 无 | Vec/String/Box（堆块 + 长度前缀） |
| 编译目标 | Stardust VM / LLVM IR | Stardust 源码 / LLVM IR |
| 嵌入 | — | `stardust!` 宏 + `extern "stardust"` FFI |

---

> 自由如鸟，尽在栈中。
>
> Free as a bird, everything is a stack.
