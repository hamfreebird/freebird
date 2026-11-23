# Freebird 能力矩阵

> 更新日期：2026-11-23。对照 `readme.md`（目标规范）与当前 `src/` 实现。
> 图例：✅ 已实现并有测试；🟡 部分实现（见备注）；❌ 未实现，使用处应在编译期报错。
> 测试基线：`cargo test` = **202 unit + 142 integration**（`tests/integration_test.rs` 33，`tests/m0_test.rs` 28，`tests/m1_test.rs` 21，`tests/m2_test.rs` 58，`tests/differential_test.rs` 2）。

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
| 函数指针 `fn(A) -> B`、函数值、间接调用 | §5、§6 | ✅ 零/单/多返回值 | ✅ FnId dispatch | ❌ | `m2_test` |
| 用户泛型 `struct/enum/fn<T>`（类型擦除 MVP） | §5 | ✅ 字段/实参推断 | ✅ 单份代码生成 | ❌ | `m2_test` |
| 显式类型实参（turbofish） | §5 | ✅ | ✅ | ❌ | `m2_test` |
| 泛型约束、按实例单态化 | §5 | ❌ 待办 | ❌ 待办 | ❌ | — |
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
| `Option` / `Result` 内建枚举（泛型载荷 `Option<T>` / `Result<T, E>`） | §9 | ✅ | ✅ | ❌ | `m2_test` |
| `Option` 方法（is_some/is_none/unwrap/unwrap_or） | §9 | ✅ | ✅ | ❌ | `m2_test` |
| `Option` / `Result` 方法（`is_some`/`is_none`/`unwrap`/`unwrap_or`、`is_ok`/`is_err`/`unwrap`/`unwrap_or`、`ok`/`err`/`ok_or`/`map`） | §9 | ✅ | ✅ | ❌ | `m2_test` |
| 数学内建 `abs` / `min` / `max` | §9 | ✅ | ✅ | ❌ | `m2_test` |
| Vec/String `is_empty` / `clear` | §9 | ✅ | ✅ | ❌ | `m2_test` |
| 字符串解析（`str_to_int`） | §9 | ✅ | ✅ | ❌ | `m2_test` |
| 字符串格式化输出 | §9 | ❌ 待办 | ❌ 待办 | ❌ | — |
| 用户 `impl`（关联函数 + `self` 方法） | §5 | ✅ | ✅ | ❌ | `m2_test` |
| `&self` 接收者（只读方法） | §5 | ✅ | ✅ | ❌ | `m2_test` |
| `&mut self` 接收者（写回） | §5 | ✅ | ✅ | ❌ | `m2_test` |
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
| `#id` / `ElementId` 表达式 | ❌ 已从规范移除（§4.3） | ❌ 同上 | ❌ | `m0_test` |
| `use` / 文件级模块（`mod`、`pub`） | ✅ | ✅ | ❌ | `m2_test` |
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

## 6. M2 语言补全（已完成：M2-1～M2-9；M2-1 类型擦除、M2-7 除格式化输出）

- **M2-5 模式匹配**：结构体/元组 `..` 剩余、嵌套模式、`or`、守卫与非穷尽检查已覆盖；新增重复/兜底后分支的不可达警告，`check` JSON 输出 `warning_count`。更复杂的覆盖分析（如 `or` 已覆盖的变体）仍有限。
- **M2-6 循环值语义**：`if let` / `while let` 在 parser 层降级为 `match` / `loop`；值位置 `loop` 与 `break value` 通过隐藏结果变量实现，VM 与 Stardust 输出一致；`break value` 在 `while`/`for` 中被拒绝；循环标签 `'outer: loop` + `break 'outer` / `continue 'outer` 已支持（含 while/for）。
- **M2-4 函数指针**：支持 `fn(A, B) -> C` 类型、函数名作值、作为参数/返回值与间接调用；零/单/多返回值间接调用在 VM/Stardust 均可用。`fn(A) -> (B, C)` 的返回列表与函数声明语义一致；间接调用的返回元数由类型检查以 `Span` 标注交给 MIR（生成阶段没有类型信息），元组解构直接绑定多槽结果。
- **M2-1 泛型（类型擦除 MVP）**：支持 `struct Pair<T>`、`enum Maybe<T>`、`fn id<T>(x: T) -> T`；类型实参从结构体字段和函数实参推断并做替换检查；布局按基名复用，VM/Stardust 各生成一份代码。显式类型实参使用 turbofish（`f::<T>(...)`）；暂不支持约束与按实例单态化，泛型函数体只能使用与具体类型无关的操作。
- **M2-2 Option/Result（泛型枚举）**：内建 `Option<T>`/`Result<T, E>`，载荷类型从构造表达式推断并做替换检查；`Option` 支持 `is_some`/`is_none`/`unwrap`/`unwrap_or`，`Result` 支持 `is_ok`/`is_err`/`unwrap`/`unwrap_or`。类型检查把每个方法调用的成功/失败变体标签标注在 `Span → MethodVariantTags` 上交给 MIR 生成器，因此 `Option`（`Some`=1/`None`=0）与 `Result`（`Ok`=0/`Err`=1）共用同一套指令。`map`/`ok`/`err` 等组合子由 M2-7 补齐。
- **M2-7 标准库（除格式化输出外）**：`abs`/`min`/`max`、`Vec`/`String` 的 `is_empty`/`clear`、转换组合子 `Result::ok`/`Result::err`/`Option::ok_or`（新增 `EnumPayloadToOption`/`OptionToResult` 指令）、`int_to_str`（双后端，堆块逐位写入）、`str_to_int`（双后端，堆块逐字符解析）与 `Option::map`/`Result::map`（MIR 用现有指令组合实现分支，双后端一致）。
- **M2-9 impl/方法（方案 C 分期）**：`impl Type { fn f(self) }` 与关联函数 `Rect::square(3)` 已可用；名字扁平化为 `Type::f`，方法调用由类型检查解析（用户方法优先、内建方法兜底）并记录 `Span` 标注供 MIR 生成。`&self` 只读方法与 `&mut self` 写方法均已支持：调用点对具名变量取引用，方法体 `self.field` 自动解引用，`self.field = v` 走 `FieldStoreRef` 写回引用目标（VM 用 `find_ref_target` 跨帧定位，Stardust 按 cell 语义写回）。
- **待实现**：M2-7 的字符串格式化输出等剩余小项。M2-8 的 unsafe 值栈已按 `docs/adr/0004-unsafe-raw-stack.md` 落地：`push`/`pop`/`swap`/`dup`/`roll(n)` 双后端一致，Stardust 侧按需翻倍扩容，`roll(n)` 支持正/零/负三向语义。
