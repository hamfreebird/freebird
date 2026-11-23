# Freebird 任务清单

> 基线：`main @ 19971a1`（2026-06-03），`cargo test` = 126 unit + 4 integration 全绿。
> 当前：`cargo test` = 202 unit + 142 integration 全绿（2026-11-23；`tests/integration_test.rs` 33 + `tests/m0_test.rs` 28 + `tests/m1_test.rs` 21 + `tests/m2_test.rs` 58 + `tests/differential_test.rs` 2）。
> 后续开发路线见 [`ROADMAP.md`](ROADMAP.md)，能力矩阵见 [`STATUS.md`](STATUS.md)；本表保留已完成项的历史记录。
> 状态说明：`待办` / `进行中` / `已完成` / `暂缓`。完成一项后必须运行 `cargo test`，并尽量补价值断言。

## M0 — 语义收敛（已完成，2026-11-22）

| ID | 项目 | 结果 |
|---|---|---|
| M0-1 | `ret` 只接受整数立即数 | 其他表达式在解析阶段报 `InvalidRetOperand`；`ret n` 元数仍由类型检查校验 |
| M0-2 | 循环外 `break`/`continue` | 类型检查报 `LoopControlOutsideLoop` |
| M0-3 | `loop`/`break value` | M0 明确拒绝 `break value`、值位置的 `loop`；M2-6 已恢复值语义并补正向测试 |
| M0-4 | `match` 完整被匹配表达式 | 使用 `parse_expr_no_struct`，括号内恢复结构体字面量 |
| M0-5 | 模式绑定与穷尽性 | 结构体/枚举嵌套/元组/字面量/`or`/引用模式；字段简写；未知变体/字段；非穷尽与 `or` 绑定不一致均报错；`let` 结构体解构 |
| M0-6 | 空体函数与 extern | 普通空体报错；extern 调用报 `ExternCall`，不再返回残留参数 |
| M0-7 | 属性解析 | `#[key=value]`、`#[key(value)]`、`#[stardust(id = N)]`；未知/非法属性报错；`main` 上的运行时限制在 VM 生效 |
| M0-8 | `#id` / `use` | 明确报 `unsupported feature`，附规范章节 |
| M0-9 | CLI `check` | 与 `run`/`build` 共用 parse → resolve → type → borrow；`m0_test` 覆盖 CLI 借用诊断 |
| M0-10 | 编译失败测试 | 新增 `tests/m0_test.rs`，28 个测试，其中 18 个负向诊断 |
| M0-11 | 特性矩阵 | 新增 `STATUS.md`；同步 `USAGE.md`、`AGENTS.md`、`ROADMAP.md` |

## M1 — 运行时一致性（已完成：8/8，2026-11-22）

| ID | 项目 | 结果 |
|---|---|---|
| M1-1 | VM/Stardust 差分测试 | 新增 `tests/differential_test.rs`，自动遍历 `tests/programs/*.fb` 与 `examples/*.fb`；CI 新增 `differential` job |
| M1-2 | `read_line` 语义 | VM 真实 stdin 改为按字节读取 + lossy 解码；Stardust 增加 pending 字节、续字节校验、overlong/surrogate/越界码点校验；`tests/m1_test.rs` 覆盖 10 类输入 |
| M1-3 | Stardust 堆回收 | free list 复用；`VecPush` 原地追加/2 倍扩容并释放旧块，`VecPop` 原地缩减；`DropVar` 回收未被返回/别名的 Vec/Box 绑定；循环内创建/销毁 Vec 的 `HEAP_ALLOC_PTR` 稳定。数组/结构体 COW 与嵌套块保守不释放（ADR 0002） |
| M1-4 | 远程栈模型 | 采用“逻辑远程栈 + 全局堆块”，删除不可达远程栈 MIR 指令，新增 `HeapStore`/`HeapLoad`；ADR 0001 |
| M1-5 | 运行时错误 | 索引读写增加非负/上界检查，越界用除零陷阱非零退出；差分覆盖除零、越界、负索引、空数组、文件缺失 |
| M1-6 | I/O 语义 | 补 print/char/string、read_int/read_char、文件写读回环测试；USAGE 写明 UTF-8 文本边界 |
| M1-7 | 资源限制 | VM 的 step/call-depth 测试；USAGE 明确 Stardust 无等价限制 |
| M1-8 | 占位清理 | `src/codegen/stardust.rs` 中 `unsupported`/`TODO`/`placeholder` 已清零；`Reverse` 在矩阵中标红 |

> M1 完成时基线：`cargo test` = 201 unit + 84 integration。

## M2 — 语言补全（已完成：M2-1～M2-9；M2-1 为类型擦除 MVP，格式化 API 规范未定义）

| ID | 项目 | 结果 |
|---|---|---|
| M2-5 | 模式匹配收尾 | `..` 结构体/元组剩余、嵌套/or/守卫/穷尽性；重复与兜底后分支警告（`check` JSON `warning_count`） |
| M2-6 | 循环值语义 | `if let`/`while let` 降级为 `match`/`loop`；`loop` 值 + `break value`；`while`/`for` 的 `break value` 拒绝；`'label` + `break`/`continue 'label` 支持 loop/while/for |
| M2-4 | 函数指针 | `fn(A,B)->C` 类型、函数名作值、参数/返回值、间接调用；Stardust FnId dispatch；零/单/多返回值间接调用均可用（返回元数由类型检查标注供给 MIR） |
| M2-2 | Option/Result | 泛型枚举 `Option<T>`/`Result<T,E>`：载荷类型推断与替换检查；`Option` 的 `is_some`/`is_none`/`unwrap`/`unwrap_or`；`Result` 的 `is_ok`/`is_err`/`unwrap`/`unwrap_or`；`map`/`ok`/`err`/`ok_or` 等组合子由 M2-7 补齐 |
| M2-7 | 标准库扩展 | `abs`/`min`/`max`；Vec/String `is_empty`/`clear`；`Option`/`Result` 转换组合子 `Result::ok`/`Result::err`/`Option::ok_or`；`int_to_str`/`str_to_int` 双后端；`Option::map`/`Result::map` 双后端 |
| M2-1 | 泛型 | `struct Pair<T>`/`enum Maybe<T>`/`fn id<T>(x: T) -> T` 声明与泛型实参类型；字段访问与函数实参推断 + 替换检查；布局/代码按基名共用一份（类型擦除 MVP，无约束/单态化；支持 turbofish） |
| M2-9 | 用户方法与 `impl` | ADR 0003 方案 C：`impl Type { fn f(self) }` 与关联函数 `Type::f(...)` 已落地（名字扁平化、用户方法优先）；`&self` 只读方法与 `&mut self` 写方法均已支持（含 `FieldStoreRef` 引用写回） |
| M2-8 | unsafe / `#id` | `#id` 与 `current_stack_id` 已从规范移除（§4.3 改为「元素 ID 访问（不支持）」、§10.1 栈 ID 行改为编译期拒绝）；unsafe 块内的 `push`/`pop`/`swap`/`dup`/`roll(n)` 已落地（ADR 0004），仅在 `unsafe` 块内可调用，Stardust 值栈按需翻倍扩容 |
| M2-3 | 模块与 `use` | `mod name;` 递归加载同目录 `<name>.fb`，模块顶层函数注册为 `name::f`；`use name::f;` 在名称解析层做别名映射；循环依赖与未解析路径有明确诊断；模块内结构体/枚举同样加 `name::` 前缀，声明默认私有、需 `pub` |

## P0 — 基础正确性与可验证性

| ID | 模块 | 未完成项 | 现状/影响 | 状态 |
|---|---|---|---|---|
| IO-1 | VM / CLI | `run` 双重输出：VM 直接写 stdout，`main` 又打印 `vm.output` | `print(4)` 输出两个 `4` | 已完成 |
| IO-2 | MIR 生成 | `print(x)` 表达式语句多生成一条 `Pop` | `print(x); print(x)` 报 `dangling reference` | 已完成 |
| IO-3 | IO / HIR / MIR | 标准库未注册：只有 `print` 是内建函数 | `print_int`、`read_int` 等报未定义变量 | 已完成 |
| TYPE-1 | 类型检查 | `typeck` / `TypeStack` 未接入 `run`/`check` | `let x: i32 = true;` 能通过检查和运行 | 已完成 |
| TYPE-2 | 类型检查 | 公开类型错误与诊断 JSON 未包含类型错误 | `freebird check` 无法报告类型问题 | 已完成 |
| PARSE-0 | Parser / MIR | 零参调用 `foo()` 被当作栈传参且不压 `CallElement` | 零参用户函数与内建函数运行时失败 | 已完成 |
| PARSE-1 | HIR / MIR | 元组解构 `let (a, b) = ...` 未绑定变量 | 解析后变量未定义 | 已完成 |
| ENUM-1 | MIR / VM | 枚举 `match` 带载荷绑定未实现 | `Option::Some(v)` 中 `v` 未定义 | 已完成 |
| ENUM-2 | MIR / VM | `Match` 生成顺序与标签泄漏有已知问题 | 多分支 match 行为不可靠 | 已完成 |
| VEC-1 | 前端 / 类型 / VM | `vec![...]` 未解析，数组是裸栈槽 | 宏直接 parse error；数组索引顺序错误 | 已完成 |
| VEC-2 | 类型 / VM | `Vec` 方法（push/pop/len）、`String`、`Box<T>` 与远程栈未落地 | `v.push(4)`、`Box::new(42)` 等不可用 | 已完成（单槽 Tuple 表示） |
| CGEN-1 | Stardust 后端 | 曾只支持数值/字符/变量/控制流/函数调用/递归，复杂值只输出注释 | 复杂值现已落为全局堆块 | 已完成（数组/结构体/枚举/字符串/Box/Vec push-pop-索引写入/ReadLine UTF-8 解码与堆块扩容/NumIn/CharIn；`&mut` 引用堆单元支持跨函数写回；FileRead/FileWrite 已实现；远程栈收敛为全局堆块（ADR 0001）） |
| CGEN-2 | LLVM 后端 | `--target llvm` 未实现 | 直接退出 “not implemented” | 暂缓 |
| BORROW-1 | 借用检查 | `BorrowChecker` 未接入流水线 | 借用/移动错误无法在编译期发现 | 已完成 |
| DROP-1 | MIR / 借用检查 | `collect_drop_vars` 恒空，`DropGen` 未调用 | 远程栈资源不会在作用域退出时释放 | 已完成 |

## P1 — 语言与运行时缺陷

| ID | 模块 | 未完成项 | 现状/影响 | 状态 |
|---|---|---|---|---|
| VM-1 | VM | `IndexAccess` 越界走 `saturating_sub`，静默不报错 | 越界访问可能读到错误元素 | 已完成 |
| VM-2 | VM | `FieldAccess` 假设结构体只占 `index+1` 槽且不消费结构体 | 多字段结构体字段访问错误 | 已完成 |
| VM-3 | VM / MIR | `MakeEnum` / `EnumTag` 用 roll 往返占位 | 枚举布局未被真实验证 | 已完成 |
| VM-4 | IO | `read_line` 的 Dup/Pop/跳转逻辑错误 | 读行功能不可用 | 已完成 |
| PARSE-2 | Parser | 多段路径 `A::B::C` 解析损坏 | 复杂枚举/模块路径不可用 | 已完成 |
| PARSE-3 | Parser | `parse_expr_imm` 非整数静默返回 0 | `stardust!` 参数错误被吞掉 | 已完成 |
| PARSE-4 | Parser / HIR | 赋值只支持简单变量 | `*p = ...`、`obj.field = ...`、`arr[i] = ...` 不支持 | 已完成 |
| PARSE-5 | Lexer | 未知转义 `\q` 静默接受 | 源码错误不报 | 已完成 |
| PARSE-6 | Parser | 块状表达式后省略分号时，下一条以 `*`/`-`/`&` 开头的语句被当作二元表达式 | `{ ... }\n*p = 4;` 报 `invalid assignment target` | 已完成（块状语句单独解析） |
| HIR-1 | HIR | 复杂赋值目标错误类型不对 | 诊断信息误导 | 已完成 |
| BORROW-2 | 借用检查 | 同名变量遮蔽时 `exit_scope` 删除外层绑定，引用绑定也不区分作用域 | 遮蔽后外层变量报 `use of moved value` 或借用误判 | 已完成（`vars` / `ref_targets` 按作用域分层） |
| IO-4 | IO | `print_str` 无条件追加换行；`file_read`/`file_write` 只是注释 | 字符串/文件 IO 不可用 | 已完成 |
| DEBUG-1 | CLI | `run --debug` 是 no-op | `RuntimeConfig.debug` 从未读取 | 已完成 |
| CONFIG-1 | VM | `RuntimeConfig.time_limit` 未执行 | 时间限制配置无效 | 已完成 |

## P2 — 工具链、测试与文档

| ID | 模块 | 未完成项 | 现状/影响 | 状态 |
|---|---|---|---|---|
| TEST-1 | 测试 | 管线/集成测试多为 `is_ok()`，缺少值断言 | 回归可能漏检 | 已完成（Stardust 集成测试与 VM 输出逐字节对比；fib/while/结构体/枚举/stardust! 等管线测试改为输出断言；仅剩 `check_source` 的合法程序断言） |
| TEST-2 | 测试 | 缺枚举匹配、Vec、借用检查、Stardust 快照测试 | 关键功能无保护 | 已完成（新增结构体/枚举/Vec/Box/字符串/输入/COW 端到端测试与堆序列单元测试） |
| CI-1 | 工程 | 无 CI 工作流 | 无自动回归门禁 | 已完成（`.github/workflows/ci.yml` 执行 release 构建与 `cargo test`） |
| FMT-1 | 工程 | `cargo fmt --check` 失败；`cargo clippy` 有既有错误 | 不能直接作为门禁 | 待办（需单独的全树格式化/修复提交） |
| DOC-1 | 文档 | `readme.md` / `USAGE.md` 描述的 LLVM、标准库、类型检查等多未实现 | 使用者会误判能力 | 已完成（`readme.md` 顶部注明目标规范与实现状态；`AGENTS.md` / `USAGE.md` 已同步当前能力与限制） |
| TOOL-1 | 工具链 | 无包管理器、LSP、调试器 | 仅 CLI + REPL | 暂缓 |

## 执行顺序

1. P0 运行时正确性：IO-1、IO-2。
2. P0 可用性：IO-3、TYPE-1、TYPE-2。
3. P1 小缺陷：VM-1、PARSE-2、PARSE-3、PARSE-5。
4. P0 语言特性：PARSE-1、ENUM-1/2、VEC-1。
5. 所有权与后端：BORROW-1、DROP-1、CGEN-1。
6. 测试与工程化：TEST-1/2、CI-1、FMT-1、DOC-1。
