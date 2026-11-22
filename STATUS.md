# Freebird 能力矩阵

> 更新日期：2026-11-22。对照 `readme.md`（目标规范）与当前 `src/` 实现。
> 图例：✅ 已实现并有测试；🟡 部分实现（见备注）；❌ 未实现，使用处应在编译期报错。
> 测试基线：`cargo test` = **201 unit + 79 integration**（`tests/integration_test.rs` 33，`tests/m0_test.rs` 28，`tests/m1_test.rs` 16，`tests/differential_test.rs` 2）。

## 1. 语言与运行时

| 能力 | 规范章节 | VM | Stardust | LLVM | 测试 |
|---|---|---|---|---|---|
| 整数/浮点/布尔/字符/单位值 | §3、§5 | ✅ | ✅ | ❌ | 单元 + 集成 |
| 变量、遮蔽、作用域、赋值 | §4 | ✅ | ✅ | ❌ | 单元 + 集成 |
| `if` / `while` / `for` / `loop`（unit） | §7 | ✅ | ✅ | ❌ | 集成 |
| `break` / `continue` | §7 | ✅ | ✅ | ❌ | 集成 |
| `break value`、把 `loop` 当值使用 | §7 | ❌ 编译期拒绝 | ❌ 编译期拒绝 | ❌ | `m0_test` |
| 函数、递归、多值返回、`return` | §6 | ✅ | ✅ | ❌ | 集成 |
| `ret n`（整数立即数、元数匹配） | §6 | ✅ | ✅ | ❌ | 集成 + `m0_test` |
| 数组、`Vec`、`String`、`Box` | §8、§9 | ✅ | ✅ | ❌ | 集成 + 差分（全局堆块，ADR 0001） |
| 容器索引越界检查 | §8 | ✅ 运行期报错 | ✅ 运行期陷阱（M1-5） | ❌ | `differential_test` |
| 堆块回收 / 复用 | §8.5 | ✅ 随值释放 | 🟡 只增不减（M1-3 待引用计数，ADR 0002） | ❌ | — |
| 结构体、字段访问/赋值 | §8 | ✅ | ✅ | ❌ | 集成 |
| 枚举、变体构造、`EnumTag` | §8 | ✅ | ✅ | ❌ | 集成 |
| `&` / `&mut`、解引用、写回 | §4 | ✅ | ✅ | ❌ | 集成 |
| 借用/移动检查 | §4 | ✅ 编译期 | ✅ 编译期 | — | 单元 + 集成 |
| 标量 `as` 转换 | §5 | ✅ | ✅ | ❌ | 集成 |
| `print_*` / `read_*` / `read_line` | §9 | ✅ | 🟡 UTF-8/CRLF/非法字节一致；EOF 报 I/O 错误 | ❌ | 集成 + `m1_test` |
| `file_read` / `file_write` | §9 | ✅ | ✅ | ❌ | 集成 |
| `unsafe` + `stardust!` 宏 | §11 | 🟡 大部分 | 🟡 大部分（`Reverse` 未支持，M3-4） | ❌ | 集成 + `m1_test` |

## 2. 模式匹配与绑定

| 能力 | VM | Stardust | LLVM | 测试 |
|---|---|---|---|---|
| 完整被匹配表达式（调用、二元、括号结构体字面量） | ✅ | ✅ | ❌ | `m0_test` |
| 字面量、`_`、变量模式 | ✅ | ✅ | ❌ | 集成 |
| 结构体模式（含字段简写、`..`） | ✅ | ✅ | ❌ | 集成 + `m0_test` |
| 枚举模式（含嵌套载荷） | ✅ | ✅ | ❌ | 集成 + `m0_test` |
| 元组模式（含嵌套） | ✅ | ✅ | ❌ | `m0_test` |
| `or` 模式（要求各分支绑定同名变量） | ✅ | ✅ | ❌ | `m0_test` |
| 引用模式 `&pat` / `&mut pat` | ✅ | ✅ | ❌ | `m0_test` |
| guard 失败后继续尝试后续分支 | ✅ | ✅ | ❌ | 集成 |
| 穷尽性检查（枚举/bool/兜底） | ✅ 编译期 | ✅ 编译期 | — | `m0_test` |
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
| `freebird check`（parse → resolve → type → borrow） | ✅ | ✅ | — | `m0_test` |
| `freebird build --target stardust` | — | ✅ | — | 集成 |
| `freebird build --target llvm` | ❌ | ❌ | ❌ | CLI 直接报未实现 |

## 4. M0 退出标准核对

- 所有已接受语法要么执行，要么给出带位置的编译期诊断；不再有 `break value`/`loop` 值、`#id`、`use`、`extern` 调用的静默成功。
- `check` 与 `run`/`build` 共用 parse → resolve → type → borrow 流程。
- 新增 `tests/m0_test.rs`：28 个测试，覆盖 18 个负向诊断与 10 个正向/差分场景。
- VM 与 Stardust 在新增模式匹配、`let` 解构、`ret n`、unit 循环用例上输出逐字节一致。
- LLVM 仍为占位；相关能力在矩阵中统一标 ❌，调用时明确报错。

## 5. M1 运行时一致性（进行中：7/8）

- **已完成**：差分测试框架与 CI `differential` job；`read_line` 的 UTF-8/CRLF/非法字节解码与 VM 逐字节一致；远程栈收敛为“逻辑远程栈 + 全局堆块”（ADR 0001）；索引越界检查；I/O 与 VM 资源限制测试；Stardust 占位清理。
- **遗留 M1-3**：Stardust 堆块仍只增不减。值语义 + copy-on-write 让多个变量共享块，安全回收需要引用计数或所有权跟踪；方案见 ADR 0002，本轮不实现。
- **已知后端差异**：`read_line` 在 EOF 时 Stardust 解释器的 `CharIn` 直接报 I/O 错误（VM 返回空字符串）；Stardust 越界用除零陷阱退出，错误消息与 VM 不同；`stardust!` 的 `Reverse` 仍不支持（M3-4）。
