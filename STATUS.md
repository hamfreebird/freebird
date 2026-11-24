# Freebird 能力矩阵

> 更新日期：2026-11-24。对照 `readme.md`（目标规范）与当前 `src/` 实现。
> 图例：✅ 已实现并有测试；🟡 部分实现（见备注）；❌ 未实现，使用处应在编译期报错。
> 测试基线：`cargo test` = **232 unit + 196 integration**（`tests/integration_test.rs` 33，`tests/m0_test.rs` 28，`tests/m1_test.rs` 21，`tests/m2_test.rs` 58，`tests/m3_test.rs` 35，`tests/m4_test.rs` 13，`tests/m5_test.rs` 5，`tests/snapshot_test.rs` 1，`tests/differential_test.rs` 2）。
> 后端说明：LLVM 后端复用 Stardust 自带的 LLVM 编译器（M3），因此其语言语义与 Stardust 后端一致；文件 I/O 已在 LLVM 目标上支持（M6-2.1）；Unicode `CharOut` 已支持（bundled Stardust 的 LLVM 后端读取 `#[char_mode=unicode]`）。

## 1. 语言与运行时

| 能力 | 规范章节 | VM | Stardust | LLVM | 测试 |
|---|---|---|---|---|---|
| 整数/浮点/布尔/字符/单位值 | §3、§5 | ✅ | ✅ | ✅ | 单元 + 集成 + `m3_test` |
| 变量、遮蔽、作用域、赋值 | §4 | ✅ | ✅ | ✅ | 单元 + 集成 + `m3_test` |
| `if` / `while` / `for` / `loop`（unit） | §7 | ✅ | ✅ | ✅ | 集成 + `m3_test` |
| `if let` / `while let` | §7 | ✅ | ✅ | ✅ | `m2_test` |
| `break` / `continue` | §7 | ✅ | ✅ | ✅ | 集成 + `m3_test` |
| `break value`、值位置 `loop` | §7 | ✅ | ✅ | ✅ | `m2_test` |
| 循环标签 `break 'label` / `continue 'label` | §7 | ✅ | ✅ | ✅ | `m2_test` |
| 函数指针 `fn(A) -> B`、函数值、间接调用 | §5、§6 | ✅ 零/单/多返回值 | ✅ FnId dispatch | ✅ | `m2_test` + `m3_test` |
| 用户泛型 `struct/enum/fn<T>`（类型擦除 MVP） | §5 | ✅ 字段/实参推断 | ✅ 单份代码生成 | ✅ | `m2_test` |
| 显式类型实参（turbofish） | §5 | ✅ | ✅ | ✅ | `m2_test` |
| 泛型约束、按实例单态化 | §5 | ❌ 待办（ADR 0006 已设计） | ❌ 待办 | ❌ 待办 | — |
| 函数、递归、多值返回、`return` | §6 | ✅ | ✅ | ✅ | 集成 + `m3_test` |
| `ret n`（整数立即数、元数匹配） | §6 | ✅ | ✅ | ✅ | 集成 + `m0_test` |
| 数组、`Vec`、`String`、`Box` | §8、§9 | ✅ | ✅ | ✅ | 集成 + 差分 + `m3_test`（全局堆块，ADR 0001） |
| 容器索引越界检查 | §8 | ✅ 运行期报错 | ✅ 运行期陷阱（M1-5） | ✅ 运行期陷阱（继承 Stardust） | `differential_test` |
| 堆块回收 / 复用 | §8.5 | ✅ 随值释放 | 🟡 free list + Vec/Box DropVar 回收；数组/结构体 COW 与嵌套块保守不释放 | 🟡 同 Stardust | `m1_test` |
| 结构体、字段访问/赋值 | §8 | ✅ | ✅ | ✅ | 集成 + `m3_test` |
| 枚举、变体构造、`EnumTag` | §8 | ✅ | ✅ | ✅ | 集成 + `m3_test` |
| `&` / `&mut`、解引用、写回 | §4 | ✅ | ✅ | ✅ | 集成 + `m3_test` |
| 借用/移动检查 | §4 | ✅ 编译期 | ✅ 编译期 | ✅ 编译期（同一前端） | 单元 + 集成 |
| 标量 `as` 转换 | §5 | ✅ | ✅ | ✅ | 集成 |
| `print_*` / `read_*` / `read_line` | §9 | ✅ | ✅ UTF-8/CRLF/非法字节一致；EOF 与 VM 一致报 I/O 错误 | ✅ 继承 Stardust；Unicode `CharOut` 已支持 | 集成 + `m1_test` + `m3_test` |
| `Option` / `Result` 内建枚举（泛型载荷 `Option<T>` / `Result<T, E>`） | §9 | ✅ | ✅ | ✅ | `m2_test` + `m3_test` |
| `Option` 方法（is_some/is_none/unwrap/unwrap_or） | §9 | ✅ | ✅ | ✅ | `m2_test` + `m3_test` |
| `Option` / `Result` 方法（`is_some`/`is_none`/`unwrap`/`unwrap_or`、`is_ok`/`is_err`/`unwrap`/`unwrap_or`、`ok`/`err`/`ok_or`/`map`） | §9 | ✅ | ✅ | ✅ | `m2_test` + `m3_test` |
| 数学内建 `abs` / `min` / `max` | §9 | ✅ | ✅ | ✅ | `m2_test` |
| Vec/String `is_empty` / `clear` | §9 | ✅ | ✅ | ✅ | `m2_test` |
| 字符串解析（`str_to_int`） | §9 | ✅ | ✅ | ✅ | `m2_test` |
| 字符串格式化 `format(template, args...)` | §11.5 | ✅ | ✅ | ✅ | `m6_test`（占位符/转义/变量模板；T024/T025 诊断） |
| 用户 `impl`（关联函数 + `self` 方法） | §5 | ✅ | ✅ | ✅ | `m2_test` + `m3_test` |
| `&self` 接收者（只读方法） | §5 | ✅ | ✅ | ✅ | `m2_test` + `m3_test` |
| `&mut self` 接收者（写回） | §5 | ✅ | ✅ | ✅ | `m2_test` + `m3_test` |
| `file_read` / `file_write` | §9 | ✅ | ✅ | ✅ | 集成 + `m3_test` |
| `unsafe` + `stardust!` 宏 | §11 | ✅ 全部宏指令（Call 仅 Stardust/LLVM） | ✅ 全部宏指令 | ✅ 全部宏指令 | 集成 + `m1_test` + `m3_test` |
| `stardust!` 的 `Mark`/`Jump`/`UncondJump`/`DropN`/`Pick`/`NumOut`/`NumIn`/`CharOut`/`CharIn`/`DumpStack`/`DumpState`/`Breakpoint`/`Nop`/`ShiftL`/`ShiftR`/`Rotate`/`Depth`/`Reverse`/`Call` | §10.3 | ✅ 语义与 Stardust 一致 | ✅ | ✅ | `m3_test` |
| `extern "stardust"` FFI（`#[stardust(id = N)]`） | §10.4 | ❌ 明确报错（仅 Stardust/LLVM 可执行） | ✅ 生成 `Call(id, argc)` | ✅ `--link-stardust` 追加外部定义 | `m3_test` + `m6_test` |

## 2. 模式匹配与绑定

| 能力 | VM | Stardust | LLVM | 测试 |
|---|---|---|---|---|
| 完整被匹配表达式（调用、二元、括号结构体字面量） | ✅ | ✅ | ✅ | `m0_test` |
| 字面量、`_`、变量模式 | ✅ | ✅ | ✅ | 集成 |
| 结构体模式（含字段简写、`..`） | ✅ | ✅ | ✅ | 集成 + `m0_test` |
| 枚举模式（含嵌套载荷） | ✅ | ✅ | ✅ | 集成 + `m0_test` |
| 元组模式（含嵌套、`..` 剩余） | ✅ | ✅ | ✅ | `m0_test` + `m2_test` |
| `or` 模式（要求各分支绑定同名变量） | ✅ | ✅ | ✅ | `m0_test` |
| 引用模式 `&pat` / `&mut pat` | ✅ | ✅ | ✅ | `m0_test` |
| guard 失败后继续尝试后续分支 | ✅ | ✅ | ✅ | 集成 |
| 穷尽性检查（枚举/bool/兜底） | ✅ 编译期 | ✅ 编译期 | — | `m0_test` |
| 不可达/重复分支警告 | ✅ 编译期 warning | ✅ 编译期 warning | — | `m2_test` |
| 未知变体/字段/结构体诊断 | ✅ 编译期 | ✅ 编译期 | — | `m0_test` |
| `let` 变量/元组/结构体解构 | ✅ | ✅ | ✅ | 集成 + `m0_test` |
| `let` 可失败模式（枚举变体、字面量） | ❌ 编译期拒绝 | ❌ 编译期拒绝 | — | `m0_test` |

## 3. 声明、属性与工具链

| 能力 | VM | Stardust | LLVM | 备注/测试 |
|---|---|---|---|---|
| `#[key=value]`、`#[key(value)]` 属性解析 | ✅ | ✅ | — | 属性挂到函数/结构体/枚举声明 |
| `#[stardust(id = N)]` | ✅ 解析/校验 | ✅ 生成 `Call(id, argc)` | ✅ 生成 `Call(id, argc)` | `m0_test` + `m3_test` |
| `#[max_stack]` / `#[max_call_depth]` / `#[max_steps]` / `#[time_limit]` | ✅ 读取 `main` 属性并生效 | ❌ 仅接受、不生成限制 | ❌ | `m0_test`（VM） |
| 未知/非法属性诊断 | ✅ 编译期 | ✅ 编译期 | — | `m0_test` |
| 无函数体的普通 `fn` | ❌ 编译期拒绝 | ❌ 编译期拒绝 | — | `m0_test` |
| `extern "stardust"` 声明 | ✅ 解析 | ✅ 解析 | ✅ 解析 | `m0_test` |
| `extern` 函数调用（FFI） | ❌ 运行时明确报错 | ✅ 生成真实 `Call` | 🟡 需外部 Stardust 定义参与链接 | `m3_test` |
| `#id` / `ElementId` 表达式 | ❌ 已从规范移除（§4.3） | ❌ 同上 | ❌ | `m0_test` |
| `use` / 文件级模块（`mod`、`pub`） | ✅ | ✅ | ✅ 前端展开 | `m2_test` |
| `freebird check`（parse → resolve → type → borrow） | ✅ | ✅ | — | `m0_test` + `m4_test`；JSON 含 `schema_version`、稳定错误码、`related`；项目模式诊断带 `file` |
| `freebird build --target stardust` | — | ✅ | — | 集成 |
| `freebird build --target llvm` | — | — | ✅ IR / 可执行文件 | `m3_test`；支持 `--output`、`--emit ir\|exe`、`--opt-level` |
| `freebird fmt` | — | — | — | `m4_test`；缩进/空白规范化，`--check/--write/--stdin` |
| `freebird lsp`（诊断/hover/跳转/引用/重命名/符号/补全，UTF-16 位置） | ✅ | ✅ | ✅ | `m4_test` + LSP 单元测试 |
| `freebird dap`（断点/单步/栈/变量/pause/展开） | ✅ | — | — | `m4_test` + DAP 单元测试 |
| `freebird.toml` 项目（entry + 递归路径依赖 + `freebird.lock`） | ✅ | ✅ | ✅ | `m4_test` + `m6_test`；`--manifest-path` / 自动发现；跨文件诊断带 `file` |
| CI/发布（fmt/clippy/examples/文档链接/release） | ✅ | ✅ | ✅ | `.github/workflows/{ci,release}.yml` |

## 4. M0 退出标准核对

- 所有已接受语法要么执行，要么给出带位置的编译期诊断；不再有 `break value`/`loop` 值、`#id`、`use`、`extern` 调用的静默成功。
- `check` 与 `run`/`build` 共用 parse → resolve → type → borrow 流程。
- 新增 `tests/m0_test.rs`：28 个测试，覆盖 18 个负向诊断与 10 个正向/差分场景。
- VM 与 Stardust 在新增模式匹配、`let` 解构、`ret n`、unit 循环用例上输出逐字节一致。
- LLVM 仍为占位；相关能力在矩阵中统一标 ❌，调用时明确报错。

## 5. M1 运行时一致性（已完成：8/8，保守回收）

- **已完成**：差分测试框架与 CI `differential` job；`read_line` 的 UTF-8/CRLF/非法字节解码与 VM 逐字节一致；远程栈收敛为“逻辑远程栈 + 全局堆块”（ADR 0001）；索引越界检查；I/O 与 VM 资源限制测试；Stardust 占位清理。
- **M1-3 堆回收**：free list 复用；`VecPush` 原地追加/2 倍扩容并释放旧块，`VecPop` 原地缩减；`DropVar` 对未被返回/别名的 Vec/Box 绑定释放块。循环内创建/销毁 Vec 的 `HEAP_ALLOC_PTR` 稳定（ADR 0002）。
- **保守边界**：数组/结构体等 Copy 类型的 COW 旧块、嵌套容器内层块、从未 push/pop 的 Vec/String 不回收（安全泄漏）；完整引用计数留待后续。M6-2.5 实测 COW 写入约 10 堆元素/次（1M 堆上限 ≈ 10 万次写入），压力护栏见 `tests/m6_test.rs`，细节见 ADR 0002。
- **已知后端差异**：Stardust 越界用除零陷阱退出，错误消息与 VM 不同（分类一致）。

## 6. M2 语言补全（已完成：M2-1～M2-9；M2-1 类型擦除、格式化 API 规范未定义）

- **M2-5 模式匹配**：结构体/元组 `..` 剩余、嵌套模式、`or`、守卫与非穷尽检查已覆盖；新增重复/兜底后分支的不可达警告，`check` JSON 输出 `warning_count`。更复杂的覆盖分析（如 `or` 已覆盖的变体）仍有限。
- **M2-6 循环值语义**：`if let` / `while let` 在 parser 层降级为 `match` / `loop`；值位置 `loop` 与 `break value` 通过隐藏结果变量实现，VM 与 Stardust 输出一致；`break value` 在 `while`/`for` 中被拒绝；循环标签 `'outer: loop` + `break 'outer` / `continue 'outer` 已支持（含 while/for）。
- **M2-4 函数指针**：支持 `fn(A, B) -> C` 类型、函数名作值、作为参数/返回值与间接调用；零/单/多返回值间接调用在 VM/Stardust 均可用。`fn(A) -> (B, C)` 的返回列表与函数声明语义一致；间接调用的返回元数由类型检查以 `Span` 标注交给 MIR（生成阶段没有类型信息），元组解构直接绑定多槽结果。
- **M2-1 泛型（类型擦除 MVP）**：支持 `struct Pair<T>`、`enum Maybe<T>`、`fn id<T>(x: T) -> T`；类型实参从结构体字段和函数实参推断并做替换检查；布局按基名复用，VM/Stardust 各生成一份代码。显式类型实参使用 turbofish（`f::<T>(...)`）；暂不支持约束与按实例单态化，泛型函数体只能使用与具体类型无关的操作。
- **M2-2 Option/Result（泛型枚举）**：内建 `Option<T>`/`Result<T, E>`，载荷类型从构造表达式推断并做替换检查；`Option` 支持 `is_some`/`is_none`/`unwrap`/`unwrap_or`，`Result` 支持 `is_ok`/`is_err`/`unwrap`/`unwrap_or`。类型检查把每个方法调用的成功/失败变体标签标注在 `Span → MethodVariantTags` 上交给 MIR 生成器，因此 `Option`（`Some`=1/`None`=0）与 `Result`（`Ok`=0/`Err`=1）共用同一套指令。`map`/`ok`/`err` 等组合子由 M2-7 补齐。
- **M2-7 标准库**：`abs`/`min`/`max`、`Vec`/`String` 的 `is_empty`/`clear`、转换组合子 `Result::ok`/`Result::err`/`Option::ok_or`（新增 `EnumPayloadToOption`/`OptionToResult` 指令）、`int_to_str`（双后端，堆块逐位写入）、`str_to_int`（双后端，堆块逐字符解析）与 `Option::map`/`Result::map`（MIR 用现有指令组合实现分支，双后端一致）。
- **M2-9 impl/方法（方案 C 分期）**：`impl Type { fn f(self) }` 与关联函数 `Rect::square(3)` 已可用；名字扁平化为 `Type::f`，方法调用由类型检查解析（用户方法优先、内建方法兜底）并记录 `Span` 标注供 MIR 生成。`&self` 只读方法与 `&mut self` 写方法均已支持：调用点对具名变量取引用，方法体 `self.field` 自动解引用，`self.field = v` 走 `FieldStoreRef` 写回引用目标（VM 用 `find_ref_target` 跨帧定位，Stardust 按 cell 语义写回）。
- **待实现 / 里程碑**：泛型约束与按实例单态化、闭包/`const`/trait 属于后续里程碑；三者的语法、实现方案、兼容策略与分阶段验收见 `docs/adr/0006-language-evolution.md`。M2-8 的 unsafe 值栈已按 `docs/adr/0004-unsafe-raw-stack.md` 落地：`push`/`pop`/`swap`/`dup`/`roll(n)` 双后端一致，Stardust 侧按需翻倍扩容，`roll(n)` 支持正/零/负三向语义。

## 7. M3 后端与 FFI（已完成，2026-11-24）

- **LLVM 后端**：`build --target llvm` 复用 Stardust 的 LLVM 编译器。流水线为
  `Freebird → MIR → Stardust 源码 → stardust --compile → LLVM IR → clang → 可执行文件`。
  支持 `--output/-o`、`--emit ir|exe`、`--opt-level 0-3`、`--stardust-bin`；
  工具链按 `STARDUST_BIN` → `PATH` → 仓库自带 `stardust/stardust-linux-x86_64` → 兄弟仓库查找。
- **LLVM 覆盖范围**：标量、控制流、函数/递归/多值返回、函数指针、结构体、数组、枚举/`match`、
  引用与 `&mut` 写回、`Vec`/`String`/`Box`、`Option`/`Result`、`impl` 方法、`stardust!` 指令、
  `read_int`/`read_char`/`read_line`。`m3_test` 有 16 个 VM/LLVM 输出对比测试。
- **LLVM 限制**：文件 I/O（`file_read`/`file_write`）已在 M6-2.1 支持（运行时按 UTF-8 编解码，
  失败路径以非零状态退出）；Unicode `CharOut` 已支持（bundled Stardust 更新）；
  `read_line` 在 EOF 报 I/O 错误（与 VM 一致）；Freebird 的运行时限制属性只影响 VM。
- **Stardust FFI**：`extern "stardust"` + `#[stardust(id = N)]` 生成真实 `Call(id, argc)`；
  省略 `id` 时按函数名做 SipHash-2-4 并映射到 `1..=999`（ID 即 Stardust 前导空格数）。
  调用必须在 `unsafe` 块内；重复/过大 ID 编译期报错；内部函数名自动避开 extern ID。
  VM 上执行 FFI 会在运行时报明确错误；LLVM 目标需要外部 Stardust 定义参与链接。
- **`stardust!` 指令**：`Mark`/`Jump`/`UncondJump`/`DropN`/`Pick`/`NumOut`/`NumIn`/`CharOut`/
  `CharIn`/`DumpStack`/`DumpState`/`Breakpoint`/`Nop` 以及 `ShiftL`/`ShiftR`/`Rotate`/`Depth`/
  `Reverse`/`Call` 全部落地；未知指令改为编译错误。宏入口/出口隐藏并恢复 Freebird 的
  `CallElement`（堆槽 13），因此宏内整栈指令只操作用户数据栈，VM 与 Stardust 输出一致。
- **M3 完成时测试基线**：206 unit + 177 integration；`tests/m3_test.rs` 35 个测试。

## 8. M4 工具链与开发者体验（已完成，2026-11-24）

- **M4-1 LSP**：`freebird lsp`（stdio JSON-RPC）：诊断、hover、跳转定义、文档符号、补全；VS Code 扩展通过 `vscode-languageclient` 接入，`freebird.serverPath` 可配置。
- **M4-2 DAP**：`freebird dap`（stdio DAP）：launch、setBreakpoints、configurationDone、stackTrace/scopes/variables、continue/next/stepIn/stepOut、pause；基于 `MirInst::Line` 行标记。M6-3.2 起 stdin 由独立线程读取、执行按分片推进，运行中可 `pause`；`variables` 可展开数组/结构体/枚举Payload/Vec/Box，`stackTrace` 每帧带自己的行号。
- **M4-3 REPL**：多行输入（括号/字符串/块注释未闭合继续）、`:history`/`:run n`、`:load`、`:type`（复用 `typeck::infer_expression_type`）、类型/借用检查后的错误恢复。M6-3.4 起 `let` 绑定跨输入保留（整段会话重放）、历史持久化到状态目录（`FREEBIRD_REPL_HISTORY`/`XDG_STATE_HOME`），不内置行编辑。
- **M4-4 fmt/lint**：`freebird fmt [--check|--write|--stdin]`；M6-3.5 起除缩进/空行外还做 token 级行内空白整理与 `}`/`else` 合并（保留注释与字符串、幂等、`examples/` 全量可解析）；全树 `cargo fmt` 清理；clippy 清零并纳入 CI（`-D warnings`）。
- **M4-5 包管理**：`freebird.toml`（`[package] name/entry`、`[dependencies]` 路径）；`run/check/build` 支持 `--manifest-path` 与当前目录自动发现；依赖以 `<dep>::` 前缀加载。M6-3.3 起依赖递归展开（传递依赖同样注册），`freebird lock` 生成 `freebird.lock`（相对路径 + 包内 `.fb` 内容哈希），构建时校验。仅路径依赖，无版本解析/registry。
- **M4-6 诊断**：JSON `schema_version: 1`、稳定错误码（P/R/T/B/W）、`related` 位置与修复建议。
- **M4-7 CI/发布**：fmt/clippy、release 构建、全量测试、文档链接检查、examples 冒烟；`llvm` job；`v*` tag 发布 `freebird-linux-x86_64` + sha256。M6-4.5 起 CI 增加 macOS/Windows 的 `cargo check` 探测（非阻塞）；发布仍只覆盖 Linux x86_64（见 `docs/release.md` §8）。
- **M4 完成时测试基线**：227 unit + 190 integration；`tests/m4_test.rs` 13 项。
- **已知限制**：LSP 为文本级 MVP（UTF-16 列近似）；包管理仅单层路径依赖、无锁定文件；格式化器只规范化空白/缩进。

## 9. M5 质量基线（基线已完成，2026-11-24）

- **基准**：`freebird-bench`（VM fib/循环/Vec、Stardust 体积与耗时、LLVM 可选）+ `bench/baseline.json`；计时项三次取中位数；`--check` 阈值比较，CI 非阻塞 job；噪声基线、阈值策略与复现步骤见 `bench/README.md`。
- **MIR 优化**：`CompileOptions`/`--no-opt`；常量折叠（算术/Div/Mod/比较/逻辑）、常量条件跳转、无用跳转、死代码消除、冗余 roll；M6-4.3 起对「零参数、返回单个常量」的叶子函数做调用点内联；`Line` 保留；`tests/m5_test.rs` 验证优化/非优化输出一致。
- **fuzz**：`tests/fuzz/corpus/` 固定语料（深层递归/长循环/嵌套容器/枚举 match/深表达式）在 VM/Stardust 两端差分；随机程序与坏输入规模由 `FREEBIRD_FUZZ_SCALE` 控制（默认 30 + 60，主门禁保持快速）；失败最小化并归档到 `target/fuzz-failures`。CI 每周定时 job 用 `FREEBIRD_FUZZ_SCALE=10` 扩展运行，失败上传归档产物。
- **审计**：`docs/audits/0001-memory-ub-audit.md`；Rust unsafe 为零；Miri 已在 CI 的非阻塞 `miri` job 运行（本地 stable 无组件），ASan/UBSan 未接入已记录。
- **测试基础设施**：`scripts/check-fixtures.py`、`tests/snapshot_test.rs` + `UPDATE_SNAPSHOTS`。
- **发布**：`CHANGELOG.md`、`docs/release.md`、`scripts/check-docs.py`；release workflow 校验 tag/版本。
- **测试基线**：232 unit + 196 integration。
- **已知限制**：Miri 仅 CI 非阻塞运行（过渡期）、ASan/UBSan 未接入；fuzz 主门禁为默认规模（定时 job 扩展）、内联未实现；bench 仅告警。
