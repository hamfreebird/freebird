# Freebird 能力矩阵

> 更新日期：2026-11-23。对照 `readme.md`（目标规范）与当前 `src/` 实现。
> 图例：✅ 已实现并有测试；🟡 部分实现（见备注）；❌ 未实现，使用处应在编译期报错。
> 测试基线：`cargo test` = **201 unit + 111 integration**（`tests/integration_test.rs` 33，`tests/m0_test.rs` 28，`tests/m1_test.rs` 21，`tests/m2_test.rs` 27，`tests/differential_test.rs` 2）。

## 1. 语言与运行时

| 能力 | 规范章节 | VM | Stardust | LLVM | 测试 |
|---|---|---|---|---|---|
| 整数/浮点/布尔/字符/单位值 | §3、§5 | ✅ | ✅ | ❌ | 单元 + 集成 |
| 变量、遮蔽、作用域、赋值 | §4 | ✅ | ✅ | ❌ | 单元 + 集成 |
| `if` / `while` / `for` / `loop`（unit） | §7 | ✅ | ✅ | ❌ | 集成 |
| `if let` / `while let` | §7 | ✅ | ✅ | ❌ | `m2_test` |
| `break` / `continue` | §7 | ✅ | ✅ | ❌ | 集成 |
| `break value`、值位置 `loop` | §7 | ✅ | ✅ | ❌ | `m2_test` |
| 循环标签 `break 'label` / `continue 'label` | §7 | ✅ | ✅ | ❌ | `m2_test` |
| 函数指针 `fn(A) -> B`、函数值、间接调用 | §5、§6 | ✅ 单返回值 MVP | ✅ FnId dispatch | ❌ | `m2_test` |
| 用户泛型 `struct/enum/fn<T>`（类型擦除 MVP） | §5 | ✅ 字段/实参推断 | ✅ 单份代码生成 | ❌ | `m2_test` |
| 泛型约束、显式类型实参、按实例单态化 | §5 | ❌ 待办 | ❌ 待办 | ❌ | — |
| 函数、递归、多值返回、`return` | §6 | ✅ | ✅ | ❌ | 集成 |
| `ret n`（整数立即数、元数匹配） | §6 | ✅ | ✅ | ❌ | 集成 + `m0_test` |
| 数组、`Vec`、`String`、`Box` | §8、§9 | ✅ | ✅ | ❌ | 集成 + 差分（全局堆块，ADR 0001） |
| 容器索引越界检查 | §8 | ✅ 运行期报错 | ✅ 运行期陷阱（M1-5） | ❌ | `differential_test` |
| 堆块回收 / 复用 | §8.5 | ✅ 随值释放 | 🟡 free list + Vec/Box DropVar 回收；数组/结构体 COW 与嵌套块保守不释放 | ❌ | `m1_test` |
| 结构体、字段访问/赋值 | §8 | ✅ | ✅ | ❌ | 集成 |
| 枚举、变体构造、`EnumTag` | §8 | ✅ | ✅ | ❌ | 集成 |
| `&` / `&mut`、解引用、写回 | §4 | ✅ | ✅ | ❌ | 集成 |
| 借用/移动检查 | §4 | ✅ 编译期 | ✅ 编译期 | — | 单元 + 集成 |
| 标量 `as` 转换 | §5 | ✅ | ✅ | ❌ | 集成 |
| `print_*` / `read_*` / `read_line` | §9 | ✅ | 🟡 UTF-8/CRLF/非法字节一致；EOF 报 I/O 错误 | ❌ | 集成 + `m1_test` |
| `Option` / `Result` 内建枚举（i64 载荷） | §9 | ✅ | ✅ | ❌ | `m2_test` |
| `Option` 方法（is_some/is_none/unwrap/unwrap_or） | §9 | ✅ | ✅ | ❌ | `m2_test` |
| `Result` 方法 / 泛型 Option/Result | §9 | 🟡 用户泛型枚举可用；内建 `Option`/`Result` 仍是 `i64` 载荷 | 🟡 同上 | ❌ | `m2_test` |
| 数学内建 `abs` / `min` / `max` | §9 | ✅ | ✅ | ❌ | `m2_test` |
| Vec/String `is_empty` / `clear` | §9 | ✅ | ✅ | ❌ | `m2_test` |
| 字符串解析 / 格式化输出 | §9 | ❌ 待办 | ❌ 待办 | ❌ | — |
| `file_read` / `file_write` | §9 | ✅ | ✅ | ❌ | 集成 |
| `unsafe` + `stardust!` 宏 | §11 | 🟡 大部分 | 🟡 大部分（`Reverse` 未支持，M3-4） | ❌ | 集成 + `m1_test` |

## 2. 模式匹配与绑定

| 能力 | VM | Stardust | LLVM | 测试 |
|---|---|---|---|---|
| 完整被匹配表达式（调用、二元、括号结构体字面量） | ✅ | ✅ | ❌ | `m0_test` |
| 字面量、`_`、变量模式 | ✅ | ✅ | ❌ | 集成 |
| 结构体模式（含字段简写、`..`） | ✅ | ✅ | ❌ | 集成 + `m0_test` |
| 枚举模式（含嵌套载荷） | ✅ | ✅ | ❌ | 集成 + `m0_test` |
| 元组模式（含嵌套、`..` 剩余） | ✅ | ✅ | ❌ | `m0_test` + `m2_test` |
| `or` 模式（要求各分支绑定同名变量） | ✅ | ✅ | ❌ | `m0_test` |
| 引用模式 `&pat` / `&mut pat` | ✅ | ✅ | ❌ | `m0_test` |
| guard 失败后继续尝试后续分支 | ✅ | ✅ | ❌ | 集成 |
| 穷尽性检查（枚举/bool/兜底） | ✅ 编译期 | ✅ 编译期 | — | `m0_test` |
| 不可达/重复分支警告 | ✅ 编译期 warning | ✅ 编译期 warning | — | `m2_test` |
| 未知变体/字段/结构体诊断 | ✅ 编译期 | ✅ 编译期 | — | `m0_test` |
| `let` 变量/元组/结构体解构 | ✅ | ✅ | ❌ | 集成 + `m0_test` |
| `let` 可失败模式（枚举变体、字面量） | ❌ 编译期拒绝 | ❌ 编译期拒绝 | ❌ | `m0_test` |

## 3. 声明、属性与工具链

| 能力 | VM | Stardust | LLVM | 备注/测试 |
|---|---|---|---|---|
| `#[key=value]`、`#[key(value)]` 属性解析 | ✅ | ✅ | — | 属性挂到函数/结构体/枚举声明 |
| `#[stardust(id = N)]` | ✅ 解析/校验 | ✅ 解析/校验 | ❌ | `m0_test` |
| `#[max_stack]` / `#[max_call_depth]` / `#[max_steps]` / `#[time_limit]` | ✅ 读取 `main` 属性并生效 | ❌ 仅接受、不生成限制 | ❌ | `m0_test`（VM） |
| 未知/非法属性诊断 | ✅ 编译期 | ✅ 编译期 | — | `m0_test` |
| 无函数体的普通 `fn` | ❌ 编译期拒绝 | ❌ 编译期拒绝 | — | `m0_test` |
| `extern "stardust"` 声明 | ✅ 解析 | ✅ 解析 | ❌ | `m0_test` |
| `extern` 函数调用（FFI） | ❌ 编译期拒绝 | ❌ 编译期拒绝 | ❌ | `m0_test` |
| `#id` / `ElementId` 表达式 | ❌ 编译期拒绝 | ❌ 编译期拒绝 | ❌ | `m0_test` |
| `use` / 模块 | ❌ 编译期拒绝 | ❌ 编译期拒绝 | ❌ | `m0_test` |
| `freebird check`（parse → resolve → type → borrow） | ✅ | ✅ | — | `m0_test`；JSON 含 `warning_count` |
| `freebird build --target stardust` | — | ✅ | — | 集成 |
| `freebird build --target llvm` | ❌ | ❌ | ❌ | CLI 直接报未实现 |

## 4. M0 退出标准核对

- 所有已接受语法要么执行，要么给出带位置的编译期诊断；不再有 `break value`/`loop` 值、`#id`、`use`、`extern` 调用的静默成功。
- `check` 与 `run`/`build` 共用 parse → resolve → type → borrow 流程。
- 新增 `tests/m0_test.rs`：28 个测试，覆盖 18 个负向诊断与 10 个正向/差分场景。
- VM 与 Stardust 在新增模式匹配、`let` 解构、`ret n`、unit 循环用例上输出逐字节一致。
- LLVM 仍为占位；相关能力在矩阵中统一标 ❌，调用时明确报错。

## 5. M1 运行时一致性（已完成：8/8，保守回收）

- **已完成**：差分测试框架与 CI `differential` job；`read_line` 的 UTF-8/CRLF/非法字节解码与 VM 逐字节一致；远程栈收敛为“逻辑远程栈 + 全局堆块”（ADR 0001）；索引越界检查；I/O 与 VM 资源限制测试；Stardust 占位清理。
- **M1-3 堆回收**：free list 复用；`VecPush` 原地追加/2 倍扩容并释放旧块，`VecPop` 原地缩减；`DropVar` 对未被返回/别名的 Vec/Box 绑定释放块。循环内创建/销毁 Vec 的 `HEAP_ALLOC_PTR` 稳定（ADR 0002）。
- **保守边界**：数组/结构体等 Copy 类型的 COW 旧块、嵌套容器内层块、从未 push/pop 的 Vec/String 不回收（安全泄漏）；完整引用计数留待后续。
- **已知后端差异**：`read_line` 在 EOF 时 Stardust 解释器的 `CharIn` 直接报 I/O 错误（VM 返回空字符串）；Stardust 越界用除零陷阱退出，错误消息与 VM 不同；`stardust!` 的 `Reverse` 仍不支持（M3-4）。

## 6. M2 语言补全（进行中：M2-1/M2-5/M2-6 完成，M2-2/M2-4/M2-7 部分完成）

- **M2-5 模式匹配**：结构体/元组 `..` 剩余、嵌套模式、`or`、守卫与非穷尽检查已覆盖；新增重复/兜底后分支的不可达警告，`check` JSON 输出 `warning_count`。更复杂的覆盖分析（如 `or` 已覆盖的变体）仍有限。
- **M2-6 循环值语义**：`if let` / `while let` 在 parser 层降级为 `match` / `loop`；值位置 `loop` 与 `break value` 通过隐藏结果变量实现，VM 与 Stardust 输出一致；`break value` 在 `while`/`for` 中被拒绝；循环标签 `'outer: loop` + `break 'outer` / `continue 'outer` 已支持（含 while/for）。
- **M2-4 函数指针（MVP）**：支持 `fn(A, B) -> C` 类型、函数名作值、作为参数/返回值与间接调用；Stardust 通过 FnId dispatch 模拟。间接调用目前要求恰好一个返回值。
- **M2-1 泛型（类型擦除 MVP）**：支持 `struct Pair<T>`、`enum Maybe<T>`、`fn id<T>(x: T) -> T`；类型实参从结构体字段和函数实参推断并做替换检查；布局按基名复用，VM/Stardust 各生成一份代码。暂不支持约束、显式类型实参（turbofish）与按实例单态化，泛型函数体只能使用与具体类型无关的操作。
- **M2-2 Option/Result（非泛型 MVP）**：内建 i64 载荷枚举 `Option`/`Result`，构造与 `match` 可用；`Option` 支持 `is_some`/`is_none`/`unwrap`/`unwrap_or`；`Result` 方法在类型检查阶段明确报未实现，泛型化依赖 M2-1。
- **M2-7 标准库（部分）**：新增 `abs`/`min`/`max` 数学内建，以及 `Vec`/`String` 的 `is_empty`/`clear`；`len`/`push`/`pop` 继续可用。字符串解析与格式化输出待办。
- **待实现**：M2-2 泛型化/Result 方法、M2-3 模块/`use`、M2-4 多返回值函数指针、M2-7 其余标准库、M2-8 `unsafe`/`#id`、M2-9 `impl`。
