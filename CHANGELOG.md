# Changelog

本文件记录 Freebird 实现（Cargo 包版本）的显著变更。语言规范版本见 `readme.md` 顶部；
两者独立演进（当前规范 v0.5.0、实现 0.1.0）。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本遵循语义化版本。
0.x 阶段 minor 版本允许包含破坏性变更，但必须在对应条目中写明迁移方式（见 `docs/release.md`）。

## [Unreleased]

### Added

- **M5 质量基线**：
  - `freebird-bench` 基准套件与 `bench/baseline.json`，支持 `--check`/`--update`/`--tolerance`；
    CI 以非阻塞 job 运行。
  - 固定 seed 的随机程序差分 fuzz 与坏输入防 panic 测试（`tests/m5_test.rs`），失败自动最小化并归档。
  - 内存与 UB 审计记录（`docs/audits/0001-memory-ub-audit.md`）。
  - fixture 命名/stdin 配对检查（`scripts/check-fixtures.py`）。
  - 诊断快照（`tests/snapshots/diagnostics.txt`）与 `UPDATE_SNAPSHOTS=1` 更新流程。
  - 发布流程文档与本 CHANGELOG；`scripts/check-docs.py` 检查基线与版本一致性。
- **MIR 优化**：常量条件跳转折叠、无用跳转消除、Div/Mod/比较/逻辑常量折叠；
  `build --no-opt` 可关闭优化；`Line` 调试标记保留。

### Fixed

- MIR 不再对未知表达式生成 `TODO` 注释；`#id` 统一由类型检查报 T018。
- 多文件项目 `check` 的诊断携带 `file`（相对清单根）与文件内行号。
- `stardust!` 的 `Call(n, m[, r])` 支持显式返回 arity（ADR 0005）；默认为 0。
- LLVM 后端支持 `#[char_mode=unicode]`，`CharOut` 按 UTF-8 输出（bundled Stardust 同步更新）。
- `build --link-stardust <file>` 可追加外部 Stardust 定义（FFI）；extern 函数作为值在编译期拒绝。

### Changed

- 编译流水线新增 `CompileOptions` 与 `compile_*_with_options` 入口；
  `build` 的 Stardust/LLVM 生成默认开启优化。
- `read_line` 在 EOF 时统一报 I/O 错误（此前 VM 返回空字符串），与 Stardust/LLVM 一致；
  依赖空串判断输入结束的程序需要改为处理 I/O 错误。

### Fixed

- 优化期常量折叠改用 `checked_*`，避免优化器自身在溢出时 panic。

## [0.1.0] - 2026-11-24

### Added

- **M0–M2 语言核心**：变量/作用域、控制流、模式匹配、`loop` 值语义与标签、函数/递归/多值返回、
  函数指针、泛型声明（类型擦除 MVP）、`Option`/`Result`、模块/`use`/`pub`、`impl` 方法、
  `&`/`&mut` 与借用检查、`unsafe` 值栈、`stardust!` 宏。
- **M3 后端与 FFI**：LLVM 后端（复用 Stardust 的 LLVM 编译器）、`extern "stardust"` FFI
  （SipHash-2-4 默认 ID、函数名冲突规避）、`stardust!` 指令补全与语义对齐。
- **M4 工具链**：`freebird fmt`、`freebird lsp`（诊断/hover/定义/符号/补全）、
  `freebird dap`（断点/单步/栈/变量）、REPL 增强、`freebird.toml` 路径依赖项目、
  诊断 `schema_version` 与稳定错误码。
- **工程**：CI 的 fmt/clippy、examples、文档链接、差分与 LLVM job；release workflow。

### Notes

- LLVM 目标不支持文件 I/O，非 ASCII `CharOut` 受 Stardust LLVM 的 ASCII 限制。
- 包管理仅支持路径依赖，无版本解析/registry/锁定文件。
