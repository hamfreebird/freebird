# Freebird 后续开发路线图

> **基线**：`main @ 4568117`（2026-11-21）。`cargo test` = **204 unit + 33 integration** 全绿，`cargo build --release` 通过。
> **M0 完成后基线**（2026-11-22）：`cargo test` = **204 unit + 61 integration** 全绿（新增 `tests/m0_test.rs` 28 项）；能力矩阵见 `STATUS.md`。
> **M1 完成后基线**（2026-11-22）：`cargo test` = **201 unit + 84 integration** 全绿。
> **M2-5/M2-6 后基线**（2026-11-23）：`cargo test` = **201 unit + 94 integration** 全绿（新增 `tests/m2_test.rs` 10 项）。
> **M2-2/M2-7 后基线**（2026-11-23）：`cargo test` = **201 unit + 99 integration** 全绿（`tests/m2_test.rs` 15 项）。
> **M2-4 零返回值指针后基线**（2026-11-23）：`cargo test` = **201 unit + 125 integration** 全绿（`tests/m2_test.rs` 41 项）。
> **文档定位**：`readme.md` 是目标规范，`USAGE.md` 描述当前可用能力，`TASKS.md` 记录已完成/待办事项，`STATUS.md` 是规范 × 后端 × 测试矩阵，`IMPROVEMENTS.md` 是历史审计快照。
> **本路线图的范围**：只列出已经通过源码检查或最小程序实测确认的差距；没有验证过的猜测不进入里程碑。行为冲突时以 `src/` 和测试为准。

---

## 1. 指导原则

1. **正确性优先于特性数量**：先让已经接受的语法要么正确执行，要么给出明确诊断，再扩展新语法。
2. **拒绝静默降级**：不接受“解析成 0”“生成注释”“返回栈上残留值”“忽略声明”这类看似成功的失败。未实现的能力必须报错，或者从规范中明确移除。
3. **VM 与 Stardust 同语义**：同一段受支持的源码，两个后端应产生相同输出和同类错误；无法对齐时必须在文档中标注不支持的子集。
4. **每个特性都有验收测试**：至少包含正向输出断言、负向诊断断言；影响后端的特性再加 VM/Stardust 差分测试。
5. **占位不是完成**：MIR/Stardust 中的 `TODO`、`unsupported`、占位 `Pop`/`Push(0)` 不能出现在“受支持”路径上。
6. **文档即契约**：规范、`USAGE.md`、特性矩阵与测试同步更新；不把未实现能力写成已实现。
7. **小步提交**：每个里程碑拆成可独立合并的 PR，每个 PR 必须保持全量测试通过。

---

## 2. 现状快照

当前流水线已经打通：

`parse` → `TypeTable::from_ast_decls` → `lower`（HIR + 名称解析）→ `check_program` → `check_borrow` → `MirGen` → `Vm` / Stardust 文本。

已可用的主要能力：

- VM：数值/布尔/字符、变量与作用域、`if`/`while`/`loop`/`for`、函数与递归、多值返回、数组、结构体、枚举与 `match`、`Vec`/`String`/`Box`、元组解构、`&`/`&mut` 引用、借用检查、`print_*`/`read_*`、文件 I/O、`stardust!` 宏的大部分指令。
- Stardust 后端：数值/字符、变量、控制流、调用/递归、I/O、`ret n`、标量 `as`、数组/结构体/枚举/字符串/Box/Vec、`&mut` 堆单元写回、`for`/`continue`、`file_read`/`file_write`。
- 工具链：`run`、`check`、`build --target stardust`、`repl`；VS Code 扩展提供语法高亮。
- 测试：204 个单元测试 + 33 个集成测试；Stardust 集成测试与 VM 输出逐字节对比。

---

## 3. 已验证差距清单

### 3.1 前端与语义

| ID | 差距 | 当前实测（2026-11-21） | 优先级 |
|---|---|---|---|
| L-01 | `match` 的被匹配表达式只支持前缀表达式 | `match f(y) { ... }` 报 `expected {, found (`；`match a + b { ... }` 同样失败 | P0 |
| L-02 | 用户泛型不可用 | `struct Pair<T> { ... }` 报 `expected {, found <`；`fn id<T>(x: T) -> T` 报 `expected (, found <` | P1 |
| L-03 | 函数指针类型不可解析 | AST/HIR 有 `Type::FnPtr`、`Value::FnPtr`，但 `let f: fn(i64) -> i64 = inc;` 报 `expected type, found fn` | P1 |
| L-04 | `impl`/`const`/闭包等语法不存在 | `if let`/`while let` 已实现（M2-6）；`impl` 仍报 `expected declaration, found impl`，`const`/闭包仍缺失 | P2 |
| L-05 | `loop` 的值语义未实现 | 已实现（M2-6）：`let v = loop { break 42; };` 在 VM/Stardust 上输出 42；循环标签 `break 'label` 已支持 | 已修复 |
| L-06 | 循环外的 `break`/`continue` 被静默接受 | 顶层 `break;` 编译并运行，无任何错误 | P0 |
| L-07 | `ret` 非整数字面量静默变成 `Ret(0)` | `ret a;` 被解析为 `Ret(0)`，随后报“函数返回 1 个值，但 return 提供 0 个”这类误导性类型错误 | P0 |
| L-08 | 模式绑定只覆盖很小子集 | `match p { P { x: 1, y } => ... }` 报 `undefined variable y`；MIR 只绑定 `Var` 和“变量元组”；结构体/字面量/嵌套枚举/or/引用模式多被当作通配 | P0 |
| L-09 | `match` 不检查穷尽性 | 只写 `E::A` 一个分支、不写 `_` 也能编译运行；未匹配时直接落空继续执行 | P1 |
| L-10 | `#id` / `ElementId` 只解析不执行 | `let id: ElementId = #x; print(*(id));` 报 `type mismatch in dereference: expected &?, found ElementId`；MIR 对相关表达式落 `TODO` 注释，`Value` 没有 `ElementId` 变体 | P1 |
| L-11 | `use` 只解析不生效 | `use foo::bar;` 被接受，程序照常运行；没有模块解析、可见性或跨文件加载 | P1 |
| L-12 | 无函数体的函数调用返回栈上残留值 | `fn ext(a: i32) -> i32;` 后调用 `ext(7)` 输出 `7`；`extern "stardust"` 的 `sd_add(10, 31)` 输出 `31`，并没有生成 FFI 调用 | P0 |
| L-13 | 属性语法与属性生效都有缺口 | `#[max_steps=100]` 报 `expected declaration, found =`；`#[stardust(id = 1)]` 因属性值要求标识符而报错；普通声明上的属性在 `parse_decl` 中被丢弃 | P1 |
| L-14 | `unsafe` 只是普通块 | 类型检查、借用检查照常进入 `unsafe` 块；没有 `#id`、原始栈指令或类型栈逃逸的受控语义 | P2 |
| L-15 | `Option`/`Result` 与标准库很小 | `Vec`/`String`/`Box` 是类型检查器中的特例；方法仅覆盖 `Vec`/`String` 的 `len`/`push`/`pop` 和 `Vec::new`/`String::new`/`Box::new` | P1 |
| L-16 | 函数体为空的非 `extern` 函数未报错 | 与 L-12 同源：空体函数被当作可调用函数，返回栈顶残留值 | P0 |

### 3.2 运行时与后端

| ID | 差距 | 当前实测/代码证据 | 优先级 |
|---|---|---|---|
| R-01 | Stardust 远程栈仍是占位 | `AllocRemote`、`RemotePush`、`RemotePop`、`RemoteLoad`、`RemoteStore`、`RemoteLen`、`FreeRemote` 在 `src/codegen/stardust.rs` 中生成 unsupported 注释或占位值 | P1 |
| R-02 | Stardust 堆只增不减 | `HEAP_ALLOC_PTR` 单调增长；`DropVar` 在 Stardust 中是空操作，`FreeRemote` 是占位；长循环会持续占用堆 | P1 |
| R-03 | `read_line` 的 EOF 语义不一致 | VM 在 EOF 返回空字符串；Stardust 版本不特殊处理 EOF，会继续等待/读取或产生不同结果 | P1 |
| R-04 | 文件 I/O 只支持文本 | VM 使用 `std::fs::read_to_string`/`write`；非 UTF-8 读取、二进制内容和部分错误场景没有统一定义；错误会中止程序 | P2 |
| R-05 | `stardust!` 指令覆盖不完整 | `Reverse` 在 MIR 中直接变成 `Comment("not implemented")`；其余指令未逐条与 Stardust 语义对齐并测试 | P1 |
| R-06 | LLVM 后端未实现 | `build --target llvm` 直接报 “not implemented” | P2 |
| R-07 | 缺少系统性差分测试 | 目前只有部分 Stardust 集成测试逐字节对比；没有覆盖全部 fixture、错误路径、随机程序的 VM/Stardust 差分框架 | P0（基础设施） |
| R-08 | 运行时错误语义未对齐 | 越界、除零、整数溢出、非法转换、调用深度等错误在两个后端的分类、消息和退出码没有统一约定和测试 | P1 |
| R-09 | Stardust 缺资源限制 | VM 有 `max_steps`/`max_call_depth`/`time_limit`；生成到 Stardust 的代码没有等价保护，长程序只能依赖外部解释器 | P2 |

### 3.3 工具链与工程

| ID | 差距 | 当前实测/代码证据 | 优先级 |
|---|---|---|---|
| T-01 | CLI `check` 不报告借用错误 | 库函数 `check_source` 会跑 `check_borrow`，但 `main.rs` 的 `check` 命令自己组装诊断，传入的借用错误是 `&[]`；`let s = "hi"; let t = s; print_str(s);` 的 `check` 返回 `"errors": []` | P0 |
| T-02 | 诊断能力有限 | JSON 诊断已有 message/span/suggestion，但没有稳定错误码、相关位置列表；`notes` 基本为空；没有 schema/兼容性测试 | P2 |
| T-03 | 无 LSP、调试器、包管理器 | 只有 CLI + REPL；VS Code 扩展只有语法高亮，没有 diagnostics/completion/debug | P2 |
| T-04 | 格式化与 lint 不是门禁 | `cargo fmt --check` 和 `cargo clippy --all-targets` 在现有树上失败（含 `approx_constant` 等既有 lint），CI 只跑 build/test | P2 |
| T-05 | 测试夹具不会自动发现 | `tests/programs/*.fb` 不会自动注册；`enum_match.fb` 当前没有被任何测试使用 | P1 |
| T-06 | 版本与文档有漂移 | `Cargo.toml` 是 `0.1.0`，`readme.md` 标注 `v0.5.0`；规范中的泛型、模块、FFI、LLVM、`#id` 等多未实现 | P1 |
| T-07 | CI 覆盖面窄 | 只执行 release build 和 `cargo test`；没有 examples 巡检、Stardust 差分、fmt/clippy、文档链接检查 | P1 |

---

## 4. 里程碑

### M0 — 语义收敛与诊断可信（P0，已完成 2026-11-22）

**目标**：所有前端已经接受的语法，要么正确执行，要么在编译期给出带 span 的明确错误；消除静默失败和“返回残留值”。

**完成情况**：11 项工作全部完成。新增 18 个编译失败/诊断测试与 10 个正向/VM-Stardust 差分测试（`tests/m0_test.rs`）；`freebird check` 已包含借用检查；新增 `STATUS.md` 特性矩阵。

| ID | 工作项 | 验收标准 |
|---|---|---|
| M0-1 | `ret` 只接受整数立即数；其他表达式报错并给出正确写法 | `ret a;` 编译失败并指出“需要整数立即数”；`ret 1;` 仍通过 |
| M0-2 | 循环外的 `break`/`continue` 报编译错误 | 顶层 `break;`/`continue;` 退出码非 0，错误定位到关键字 |
| M0-3 | 明确 `loop`/`break value` 语义：M0 先拒绝带值 `break` 和把 `loop` 当值使用，M2 再实现 | `let v = loop { break 42; };` 在 M0 编译失败而不是运行期 stack underflow；已有 unit 循环不受影响 |
| M0-4 | `match` 被匹配表达式改为完整表达式解析（保留结构体字面量歧义的消解） | `match f(y) { ... }`、`match a + b { ... }`、`match (S { x: 1 }) { ... }` 均能解析 |
| M0-5 | 模式绑定补全：结构体字段、嵌套元组、字面量、`or`、引用、嵌套枚举载荷；同时补 `match` 穷尽性检查 | L-08/L-09 的探针通过；非穷尽 `match` 报错；新增结构体/枚举/嵌套模式测试 |
| M0-6 | 无函数体的非 `extern` 函数报错；`extern` 未实现前调用必须报错，不能返回残留值 | `fn ext(a: i32) -> i32;` 和 `extern "stardust"` 调用均给出明确诊断；删除返回栈顶值的路径 |
| M0-7 | 属性解析对齐文档：支持 `#[key=value]` 与 `#[stardust(id = 1)]`，属性挂到声明上；未识别的属性给出警告或错误 | 两个文档示例能解析；`#[stardust(id=1)]` 不再因整数值报错；未知属性不被静默丢弃 |
| M0-8 | `#id`、`use` 从“静默接受”改为“明确未支持”或直接实现最小闭环 | M0 至少报 `unsupported feature` 并附规范链接；若选择实现，则 VM + Stardust 均有测试 |
| M0-9 | CLI `check` 与 `run`/`build` 使用同一套检查：parse → resolve → type → borrow，并输出一致的诊断 | T-01 的 use-after-move 在 `check` 中报错；`check`/`run` 对同一坏程序退出码都为 1 |
| M0-10 | 建立编译失败测试框架（compile-fail / diagnostics 快照） | 新增 `tests/compile_fail` 或表驱动测试；每个 M0 错误都有断言，防止回归为静默成功 |
| M0-11 | 建立特性矩阵：规范章节 × VM × Stardust × LLVM × 测试 | 新增或更新 `STATUS.md`/`USAGE.md` 表格；CI 检查示例程序仍可运行 |

**M0 退出标准**

- 第 3.1 节所有 P0 探针都变成“成功执行”或“明确编译错误”，不再出现 stack underflow、静默 0、残留参数。
- `freebird check` 与 `freebird run` 对同一输入给出相同的前端错误集合。
- `cargo test` 全绿；新增至少 10 个 compile-fail/诊断测试；现有 33 个集成测试不回退。
- `readme.md`、`USAGE.md`、`TASKS.md`、`AGENTS.md` 的当前能力描述同步。

---

### M1 — 运行时一致性与内存管理（P0/P1，已完成：8/8，保守回收）

**目标**：同一段受支持源码在 VM 与 Stardust 上行为一致；堆和远程栈有明确生命周期；错误语义可预测。

| ID | 工作项 | 验收标准 | 状态 |
|---|---|---|---|
| M1-1 | 搭建 VM/Stardust 差分测试：遍历 `tests/programs/*.fb`、`examples/*.fb` 和新增错误用例，比较 stdout、退出码、错误类别 | CI 新增 `differential` job；任何一侧不一致都失败并打印 diff | ✅ `tests/differential_test.rs` + CI job |
| M1-2 | 统一 `read_line`：EOF 返回空字符串；CRLF、UTF-8、非法字节、超长行行为与 VM 一致 | 新增 EOF/CRLF/Unicode/非法字节测试；Stardust 输出与 VM 逐字节一致 | 🟡 UTF-8/CRLF/非法字节一致；EOF 受解释器 `CharIn` 限制 |
| M1-3 | Stardust 堆实现释放/复用（free list 或等价机制），`DropVar`/`FreeRemote` 真正回收；坚持所有权+Drop，不引入 GC | 长循环分配/释放后 `HEAP_ALLOC_PTR` 不无限增长；有堆压力测试和 use-after-free 负向测试 | ✅ free list + Vec 原地操作 + DropVar 保守回收；数组/结构体 COW 与嵌套块仍保守不释放（ADR 0002） |
| M1-4 | 远程栈落地：实现 `AllocRemote`/`RemotePush`/`RemotePop`/`RemoteLoad`/`RemoteStore`/`RemoteLen`/`FreeRemote` 的 Stardust 版本，或统一改为堆块模型并从规范移除“独立远程栈”描述 | 选择一种模型并记录 ADR；`Vec`/`String`/`Box` 的 VM/Stardust 差分测试通过；代码中不再有 unsupported 占位 | ✅ 逻辑远程栈 + 全局堆块，ADR 0001 |
| M1-5 | 运行时错误对齐：越界、除零、整数溢出、非法 `as`、调用深度、栈深度 | 两个后端对同一错误用例给出同类错误和非零退出；错误消息包含操作与位置 | ✅ 越界/负索引/除零/文件错误差分；消息文本受解释器限制 |
| M1-6 | I/O 语义对齐：整数/浮点/字符/字符串格式，文件读写的 UTF-8/二进制/错误策略 | 新增格式与文件错误测试；`USAGE.md` 写明文本/二进制边界 | ✅ 格式/文件回环/文件缺失测试 |
| M1-7 | 资源限制：VM 的限制语义写清楚；Stardust 生成代码尽量插入步数/调用深度保护，或明确由外部解释器负责 | 文档 + 测试；不会出现“VM 会停、Stardust 不会停”的未说明差异 | ✅ VM 限制测试 + USAGE 说明 Stardust 无限制 |
| M1-8 | Stardust 占位清理：扫掉受支持路径上的 `unsupported`/`TODO`/占位栈操作 | `grep -n "unsupported\\|TODO\\|placeholder" src/codegen/stardust.rs` 只剩真正未支持项，且每项在特性矩阵中标红 | ✅ grep 无结果；`Reverse` 在矩阵中标红 |

**M1 退出标准**

- 全部 fixture 与 examples 在 VM/Stardust 上输出一致（或明确标记不支持并跳过）。✅
- 堆压力测试通过；`DropVar` 在 Stardust 中不再是空操作。✅（保守回收，见 ADR 0002）
- 远程栈模型有单一结论，文档与代码一致。✅ ADR 0001。
- 差分测试进入 CI。✅

---

### M2 — 语言补全（P1，进行中：M2-1/M2-2/M2-5/M2-6 完成，M2-4/M2-7 部分完成）

**目标**：把 `readme.md` 中已经承诺、但当前缺失的核心语言能力补齐，并保持双后端一致。

| ID | 工作项 | 验收标准 |
|---|---|---|
| M2-1 | 用户泛型 MVP：`struct`/`enum`/`fn` 的泛型参数，单态化生成 MIR；先支持类型参数，不做高阶类型 | `Pair<T>`、`Option<T>`、`fn id<T>(x: T) -> T` 可解析、检查、运行；单态化测试覆盖多实例 | 🟡 类型擦除 MVP：泛型参数声明、字段访问/函数实参推断 + 替换检查；布局与代码按基名共用一份，未做单态化/约束/turbofish |
| M2-2 | `Option`/`Result` 作为泛型枚举进入标准库，替代硬编码特例 | `Option::Some`/`None`、`Result::Ok`/`Err` 及基本 `unwrap`/`map` 可用；VM/Stardust 测试 | ✅ 泛型枚举：`Option<T>`/`Result<T, E>` 载荷推断与替换检查；`Option` 的 `is_some`/`is_none`/`unwrap`/`unwrap_or` 与 `Result` 的 `is_ok`/`is_err`/`unwrap`/`unwrap_or`；`map`/组合子待办 |
| M2-3 | 模块与 `use`：至少支持文件级模块、`use path::item`、可见性检查；不引入完整包生态 | 多文件示例可编译运行；未解析路径报错；循环依赖有明确诊断 |
| M2-4 | 函数指针：解析 `fn(A, B) -> C`，端到端支持 `Value::FnPtr`、间接调用、作为参数/返回值 | `let f: fn(i64) -> i64 = inc; f(1)` 在 VM/Stardust 上工作；错误签名报错 | ✅ 零/单/多返回值间接调用均可用（Stardust FnId dispatch）；`fn(A) -> (B, C)` 的返回列表与函数声明一致，元数由类型检查标注供给 MIR |
| M2-5 | 完整模式匹配：嵌套模式、`or` 模式、`..` 剩余、守卫、穷尽性/不可达分支检查 | 模式矩阵测试；不可达分支警告；非穷尽 `match` 编译失败 | ✅ 基础完成：`..`/嵌套/`or`/守卫/穷尽性 + 重复与兜底后分支警告 |
| M2-6 | 循环值语义：`loop` 产出 `break value`，`if let`/`while let`，可选标签 `break 'label` | 相关程序输出正确；M0 的临时拒绝被移除并替换为正向测试 | ✅ `loop`/`break value`/`if let`/`while let`/`'label` + `break`/`continue 'label`（loop/while/for） |
| M2-7 | 标准库扩展：`Vec`/`String` 常用方法、字符/字符串转换、数学函数、格式化输出；不再依赖类型检查器特例 | 标准库 API 文档 + 集成测试；方法调用在 VM/Stardust 一致 | 🟡 `abs`/`min`/`max` + Vec/String `is_empty`/`clear` + 转换组合子（`ok`/`err`/`ok_or`）+ `int_to_str`/`str_to_int`（双后端）与 `Option::map`/`Result::map`（双后端） |
| M2-8 | `unsafe` 与 `#id`：实现受控的原始栈/元素 ID 访问，明确安全边界；或从规范移除并改为显式不支持 | `#x`、`*(#x)` 至少在一个后端可用；越界/悬空 ID 有定义行为或明确报错 |
| M2-9 | 用户定义方法与 `impl`（若决定纳入语言） | 单独 ADR；若纳入，则方法解析、借用检查、双后端测试齐全 | 🟡 ADR 0003 方案 C：`impl` + 关联函数 + 值接收者方法 + `&self` 只读方法已实现并在 VM/Stardust 通过；`&mut self` 待办 |

**M2 退出标准**

- `readme.md` 第 5、8、10、11 节中承诺的核心语法有对应实现或明确的“未支持”标记。
- 泛型、模块、函数指针、完整模式匹配均有正向 + 负向 + 差分测试。
- 标准库 API 有文档和示例，不再靠硬编码类型名分支。

---

### M3 — 后端与 FFI（P1/P2）

**目标**：让 Stardust 后端成为完整目标，启动 LLVM 后端，并打通 `stardust!`/`extern "stardust"` 互操作。

| ID | 工作项 | 验收标准 |
|---|---|---|
| M3-1 | LLVM MVP：整数/布尔/字符、算术、比较、`if`/`while`、函数调用、`print`/`read` | `freebird build --target llvm` 生成可编译 IR；一小组程序可执行且输出与 VM 一致 |
| M3-2 | LLVM 复合类型：数组、结构体、枚举、引用；字符串/Vec/Box 采用与 VM 等价的运行时表示 | 分阶段启用；每阶段有 VM/LLVM 差分测试 |
| M3-3 | Stardust FFI：`#[stardust(id = N)]`、`extern "stardust"` 调用、参数/返回值 ABI | 文档 10.4 的示例能生成真实 FFI 调用；调用不存在的 ID 有明确错误 |
| M3-4 | `stardust!` 指令补全：`Reverse`、`Rotate`、`Store`/`Load` 等逐条对齐 Stardust 语义 | 每条指令至少一个单元/集成测试；不再有“注释替代指令” |
| M3-5 | 后端特性矩阵与差分：VM / Stardust / LLVM 三列，明确支持/不支持 | CI 按矩阵跑测试；不支持项在编译期报错，不生成错误代码 |
| M3-6 | 目标选择与产物：`build` 默认目标、输出路径、优化级别、错误处理 | CLI 行为写入 `USAGE.md`；`--target llvm` 不再是 “not implemented” |

**M3 退出标准**

- LLVM MVP 可用，且与 VM 在标量/控制流/函数子集上输出一致。
- Stardust FFI 与 `stardust!` 不再有占位指令。
- 三后端特性矩阵进入 CI。

---

### M4 — 工具链与开发者体验（P2）

**目标**：从“能跑”提升到“可日常开发”。

| ID | 工作项 | 验收标准 |
|---|---|---|
| M4-1 | LSP：诊断、hover、补全、跳转定义、引用、文档符号 | VS Code 扩展接入 LSP；打开坏文件能显示行内诊断 |
| M4-2 | 调试器：DAP 断点、单步、调用栈、变量查看；先覆盖 VM | `freebird run --debug` 或独立 `freebird dap` 可用；有端到端调试测试 |
| M4-3 | REPL 增强：多行输入、历史、`:load`、`:type`、错误恢复 | 交互测试/脚本测试；REPL 与文件执行语义一致 |
| M4-4 | 格式化器与 lint 门禁：`freebird fmt`；清理既有 fmt/clippy 问题后加入 CI | `cargo fmt --check`、`cargo clippy --all-targets` 通过并成为 CI 门禁 |
| M4-5 | 包管理器/项目清单：最小 `freebird.toml`、路径依赖、构建入口；暂不做中心化 registry | 本地多包示例可构建；锁定文件/版本解析行为有测试 |
| M4-6 | 诊断增强：稳定错误码、修复建议、相关位置、JSON schema 版本 | 诊断快照测试；`check --json` 有 schema 文档 |
| M4-7 | CI/发布：examples 巡检、差分测试、文档链接检查、release 产物 | GitHub Actions 全绿；tag 发布包含二进制和校验和 |

**M4 退出标准**

- 编辑器内可完成“写代码 → 看诊断 → 跳转 → 调试”的闭环。
- fmt/clippy 成为门禁；CI 覆盖 examples 与差分。
- 包管理最小闭环可用，或有明确的“暂不做”结论。

---

### M5 — 性能、测试与稳定性（持续）

**目标**：建立长期可维护的质量基线，避免语言扩展拖垮运行时和后端。

| ID | 工作项 | 验收标准 |
|---|---|---|
| M5-1 | 基准套件：VM 启动/算术/调用/集合；Stardust 生成代码体积与运行时间；LLVM 对比 | 基准数据入库；回归超过阈值时 CI 警告 |
| M5-2 | MIR 优化：常量折叠、死代码消除、分支简化、内联（谨慎）；保持可关闭 | 优化前后输出一致；有优化开关和测试 |
| M5-3 | 模糊测试/属性测试：parser、typeck、MIR、VM；差分 fuzz | 持续运行 corpus；崩溃/差异自动最小化并归档 |
| M5-4 | 内存与 UB 审计：Rust `unsafe` 代码、Miri、sanitizer（可用时）；堆/远程栈生命周期 | 审计记录；关键路径无未解释 UB |
| M5-5 | 测试基础设施：`tests/programs` 自动发现、fixture 命名规范、快照更新流程 | 新增 `.fb` 自动进入测试；不再有未使用 fixture |
| M5-6 | 发布与兼容：语义化版本、CHANGELOG、破坏性变更流程、文档一致性检查 | 每次发布有迁移说明；文档与测试基线一致 |

---

## 5. 优先级与依赖

```
M0 语义收敛 ──→ M1 运行时一致性 ──→ M2 语言补全 ──→ M3 后端/FFI
       │                  │                  │
       └──────────────→ M4 工具链 ←──────────┘
                          │
                          └──→ M5 性能/测试（贯穿始终）
```

- **M0 必须先做**：否则后续所有语言特性都会继承“静默成功”的坏模式。
- **M1 与 M2 可部分并行**：M1 的差分框架能直接保护 M2 的新语法；但 M2 的泛型/模块会改变 MIR，需在 M1 差分稳定后合入。
- **M3 依赖 M1 的堆/远程栈模型**：否则 LLVM 会再实现一套不一致的运行时。
- **M4 依赖 M2 的模块/标准库**：LSP 和包管理器需要稳定的名称解析与项目模型。
- **M5 从 M0 开始持续投入**：测试框架、基准、fuzz 不是最后阶段才补的。

建议节奏（非承诺）：

1. **近期**：M0 全部 + M1-1/M1-2/M1-8。
2. **中期**：M1 剩余 + M2-1/M2-3/M2-4/M2-5。
3. **中后期**：M2 剩余 + M3-1/M3-3/M3-4。
4. **长期**：M3 剩余 + M4 + M5。

---

## 6. 完成定义（DoD）

一个特性只有同时满足以下条件才算完成：

1. **前端**：能解析目标语法，错误输入有明确诊断。
2. **语义**：HIR 降级、名称解析、类型检查、借用/移动检查都覆盖；不绕过流水线。
3. **VM**：有实际执行路径，输出/错误有断言。
4. **Stardust**：有真实代码生成，或明确标记“不支持”并在编译期拒绝；不能只生成注释。
5. **测试**：至少一个正向集成测试 + 一个负向测试 + （如适用）VM/Stardust 差分测试。
6. **文档**：`readme.md`（目标）、`USAGE.md`（当前）、特性矩阵同步；示例可运行。
7. **工程**：`cargo test`、`cargo build --release`、examples 巡检、CI 全绿。

**测试分层**

| 层级 | 用途 | 要求 |
|---|---|---|
| 单元测试 | lexer/parser/typeck/borrowck/MIR/VM/codegen 细节 | 每个 bug 修复尽量配回归测试 |
| 集成测试 | 端到端源码 → 输出 | 必须断言输出字节，不只 `is_ok()` |
| 编译失败测试 | 诊断与错误路径 | 断言错误类别/关键消息/span |
| 差分测试 | VM vs Stardust（未来 LLVM） | fixture 全覆盖；错误用例也比 |
| 属性/模糊测试 | 边界与崩溃 | 持续 corpus；差异自动最小化 |

---

## 7. 风险与需要决策的问题

| 风险/决策 | 说明 | 建议 |
|---|---|---|
| 泛型复杂度 | 单态化、类型推断、递归泛型容易失控 | 先做单态化 MVP；不支持高阶类型/关联类型；用 ADR 记录边界 |
| 模块解析 | 规范只写了 `use`，没有完整模块语法 | 先做文件级模块 + `use path::item`；不引入 `mod` 树，除非明确需要 |
| 远程栈模型 | 规范描述独立远程栈，Stardust 目前用堆块模拟 | 二选一：实现真实远程栈，或把规范改为“逻辑远程栈，后端可用堆块实现”；不要长期两套说法 |
| `#id` 语义 | 直接暴露 ElementId 与安全边界冲突 | 要么在 `unsafe` 中实现并定义失效行为，要么从规范移除；不能保持“解析但 TODO” |
| 循环值 | `break value` 需要类型合并和栈布局 | M2-6 已用隐藏结果变量实现；循环标签已支持 |
| `unsafe` 边界 | 当前只是普通块 | 明确“检查仍然执行”还是“跳过部分检查”；写入规范并测试 |
| LLVM 范围 | 完整后端工作量远大于 MVP | 先标量/控制流/函数，再复合类型；每阶段设退出标准 |
| Stardust VM 限制 | 外部解释器没有远程栈/资源限制原语 | 用堆块模拟并在文档标注差异；或在生成代码中插入检查 |
| 包管理器 | 中心化 registry 成本高 | 先做本地路径依赖和 `freebird.toml`；registry 暂缓 |
| GC vs 所有权 | 规范主打所有权/借用/Drop | 不引入 GC；Stardust 堆用 free list + Drop 回收 |

**明确暂缓（非当前承诺）**

- JIT、增量编译、完整 trait/associated type 系统。
- 中心化包 registry、GUI IDE、云端 playground。
- LLVM 全优化级别和所有目标三元组。
- 跨平台二进制分发（先保证 Linux x86_64 CI）。

---

## 8. 立即可执行的 10 项

按顺序拆成独立 PR，每项都带测试：

- [ ] **1. 编译失败测试框架**：建立 `tests/compile_fail` 表驱动断言，先把 L-06/L-07/L-12 固定下来。
- [ ] **2. `ret` 诊断**：非整数字面量报错，补 `ret 1;` 正向测试。
- [ ] **3. 循环外 `break`/`continue` 报错**：覆盖顶层、函数体、`match` 分支中的错误位置。
- [ ] **4. `match` 被匹配表达式**：改用完整表达式解析，补 `match f(x)`、`match a + b`、结构体字面量消歧测试。
- [ ] **5. 模式绑定补全**：结构体字段、嵌套元组、字面量、嵌套枚举；同步修 L-08。
- [ ] **6. 空体/`extern` 函数**：非 `extern` 空体报错；`extern` 未实现前调用报错，删除残留返回值路径。
- [ ] **7. CLI `check` 接入借用检查**：复用 `check_source` 或统一流水线，补 use-after-move 的 `check` 测试。
- [ ] **8. 属性解析对齐**：支持 `#[key=value]` 与 `#[stardust(id = 1)]`，属性挂到声明；未知属性不静默丢弃。
- [ ] **9. `#id`/`use` 显式化**：先报 `unsupported feature`，或实现最小闭环；不允许“解析后忽略”。
- [ ] **10. 差分测试骨架**：遍历 `tests/programs/*.fb` 与 `examples/*.fb`，比较 VM/Stardust 输出；接入 CI。

完成这 10 项后，再进入 M1 的堆/远程栈/EOF 和 M2 的泛型/模块。

---

## 9. 参考

- 目标规范：`readme.md`
- 当前能力：`USAGE.md`
- 任务清单：`TASKS.md`
- 历史审计：`IMPROVEMENTS.md`
- 工程约束与测试基线：`AGENTS.md`
- 流水线入口：`src/pipeline.rs`
- MIR 生成：`src/mir/generator.rs`
- Stardust 后端：`src/codegen/stardust.rs`
- VM：`src/vm/mod.rs`
- 前端：`src/frontend/`
- 类型/借用检查：`src/typeck/checker.rs`、`src/borrowck/`
