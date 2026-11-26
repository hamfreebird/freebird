# 内存与 UB 审计（M5-4）

> 审计日期：2026-11-24
> 范围：`src/`（VM、MIR、Stardust 代码生成、前端、类型/借用检查、LSP/DAP）
> 结论：**未发现未解释的未定义行为（UB）**；Rust 代码不使用 `unsafe`。剩余风险是内部不变量触发的 panic 与 Stardust 堆的保守泄漏，均记录在案。

---

## 1. 方法与工具状态

| 检查 | 结果 |
|---|---|
| `cargo test`（debug） | 232 unit + 192 integration，全绿（审计时快照；当前基线见 `STATUS.md`） |
| `cargo test --release` | 全绿；未发现 debug/release 行为差异（溢出路径已用 `checked_*` 处理优化期常量折叠） |
| `cargo clippy --all-targets -- -D warnings` | 通过 |
| Rust `unsafe` / `transmute` / `static mut` / `from_raw` / `as_ptr` | **无**（Freebird 语言中的 `unsafe` 关键字只是语法，VM 实现为安全的 Rust 指令） |
| `RefCell` / `Cell` / 内部可变性 | **无** |
| Miri | **CI 已接入并阻塞**（M6-4.1）：nightly job 跑核心模块 `vm::`/`mir::`/`borrowck::`（`--skip llvm` 避免匹配 `codegen::llvm::`）；本地 stable 无 `miri` 组件，未在本地运行 |
| AddressSanitizer | 已接入 CI `asan` 告警 job（需要 `-Zbuild-std` 与 rust-src）；UBSan 因 nightly 移除 `-Zsanitizer=undefined` 不再可用 |
| 坏输入 fuzz | `tests/m5_test.rs` 固定 seed 生成 60 个随机 token 输入，`check_source` 经 `catch_unwind` 断言不 panic |

Miri/sanitizer 的缺失是工具链环境限制，不是审计结论的替代；`unsafe` 为零意味着 Rust 层面没有直接 UB 入口。

---

## 2. Rust 安全性与别名

- 全部核心数据结构（`DualStack`、`VarStack`、`GlobalRegistry`、`Value`、`MirInst`）都是普通 `Vec`/`HashMap`/`Rc`/枚举，没有裸指针或手工内存管理。
- `Rc<Vec<MirInst>>` 只读共享函数体；单线程解释器，无并发别名问题。
- 前端/类型/借用检查只操作拥有的 AST/HIR 数据；`Rc` 不跨线程。
- 字符串按 Rust `String`/`char`/UTF-8 lossy 处理，没有 `unsafe` 的字节切片转换。

因此：**没有可由 Rust 类型系统之外的操作触发的 UB**。

---

## 3. panic 风险审计（unwrap/expect/索引）

`unwrap`/`expect` 数量较多的文件：`src/frontend/parser.rs`（77，多为 token 前瞻/内部不变量）、`src/codegen/stardust.rs`（39，代码生成内部不变量）、`src/pipeline.rs`（36，绝大多数在 `#[cfg(test)]` 测试中）、`src/vm/mod.rs`（32）、`src/vm/stack.rs`（22）。

分类：

1. **测试代码**：`src/pipeline.rs` 的测试模块、`tests/*`。不影响运行时。
2. **内部不变量**：
   - `DualStack` 的 `ids`/`values` 长度始终同步；索引访问依赖该不变量。
   - `pop_frame()` 后的 `call_stack.push(...)`、`frame.data.pop().ok_or(...)?` 等均先检查/使用 `?`。
   - `Rc<Vec<MirInst>>` 查找失败返回 `VmError`/跳过，不 panic。
3. **用户输入路径**：
   - parser 对非法 token 记录 `FrontendError`；M5-3 的随机坏输入未触发 panic。
   - VM 的栈/索引/调用深度/除法/I/O 错误都映射到 `VmError`；`run_source` 返回 `Err` 而不是 panic。
   - LSP/DAP 的生产代码用 `Option`/`json!`，`unwrap` 主要在测试与固定 JSON 结构上。

剩余风险：深层嵌套输入可能耗尽栈（Rust 递归 parser），M5-3 的输入长度受限未覆盖；建议后续把 fuzz 语料扩展到更深/更长的输入，并在 CI 运行。

---

## 4. 索引与边界

- VM 的 `DualStack` 直接索引 `ids[i]`/`values[i]`，依赖“双栈等长”不变量；所有公开操作（`push`/`pop`/`roll`/`pick`/`swap`/`reverse`/`shift_*`/`rotate_top3`/`drop_n`）都先检查长度并返回 `VmError`。
- 堆：VM 的 `heap: HashMap<i64, i64>` 使用 `get`/`insert`，不存在越界 panic；Stardust 生成代码的堆块访问有非负/上界检查（越界用除零陷阱退出，见 STATUS M1-5）。
- 数组/Vec/String 索引在 VM 返回 `IndexOutOfBounds`；Stardust 生成相同的检查。
- `read_line`/UTF-8 解码使用 lossy 处理，非法字节变为 U+FFFD，不 panic。

---

## 5. 堆与远程栈生命周期

- **VM**：`heap`（`HashMap`）、unsafe 值栈 `raw_stack: Vec<Value>`、ID 分配器、调用帧栈；随 VM 生命周期释放。`ElementId` 单调递增、不回收；悬空引用由借用检查 + 运行时查找返回 `DanglingReference`。
- **Stardust 后端**：逻辑远程栈用全局堆块 `[len, cap, elems...]`（ADR 0001），free list 复用；`VecPush`/`VecPop` 原地操作，`DropVar` 回收未逃逸的 Vec/Box；Copy 类型 COW 旧块与嵌套容器块保守不释放（ADR 0002）。这是**安全泄漏**，不是 UB。
- **`&mut` 引用**：VM 通过 `ElementId` 跨帧定位目标，Stardust 通过堆单元写回；编译期借用检查约束别名。

---

## 6. 发现与建议

| 级别 | 发现 | 建议 |
|---|---|---|
| 信息 | Rust `unsafe` 为零 | 保持；新增代码继续避免 `unsafe` |
| 中 | Miri/sanitizer 未接入 | M6-4.1 已接入 CI `miri` job（核心模块，现为阻塞）；ASan 已接入告警 job；UBSan 受工具链限制撤销 |
| 中 | 深层递归输入可能栈溢出（panic/abort，`catch_unwind` 捕获不到栈溢出） | 扩展 fuzz 的长度/嵌套维度；parser 增加显式递归深度限制与诊断 |
| 低 | 内部 `unwrap` 依赖不变量 | 逐步替换为 `expect("invariant: ...")` 或在关键路径返回 `VmError`；启用 `clippy::unwrap_used` 分级 |
| 低 | `src/mir/generator.rs` 对未知 `HirExpr` 只生成 `TODO` 注释 | 改为编译期错误（避免静默降级） |
| 信息 | Stardust 堆保守泄漏 | 已在 ADR 0002 记录；后续引用计数/所有权增强时再收紧 |

---

## 7. 结论

- 没有 Rust `unsafe`，因此不存在类型系统之外的内存 UB 入口。
- 运行时错误路径以 `VmError`/诊断为主；M5-3 的坏输入 fuzz 未发现 panic。
- 剩余工作是工具链层面的持续验证（Miri/ASan）与少数内部不变量/递归深度加固，已列入 M5 后续与 M6 候选。
