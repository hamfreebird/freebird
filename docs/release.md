# 发布与兼容流程

> 面向维护者。版本号指 Cargo 包版本（`Cargo.toml`），与语言规范版本（`readme.md` 顶部）独立。

## 1. 版本策略

- 遵循语义化版本 `MAJOR.MINOR.PATCH`。
- **0.x 阶段**：`MINOR` 可包含破坏性变更；`PATCH` 只做兼容修复。
- **1.0 之后**：破坏性变更只能进入 `MAJOR`。
- 语言规范版本与实现版本分开记录；规范变化不强制立即同步实现版本。

## 2. 破坏性变更流程

1. 先写 ADR（`docs/adr/`）说明动机、边界与迁移方式；涉及生成代码/ABI 时标注兼容性影响。
2. 在 `CHANGELOG.md` 的对应版本条目下用 **Changed/Removed** 明确列出，并给出迁移步骤。
3. 如果影响诊断：错误码（`P/R/T/B/W`）尽量保持稳定；确需变更时在 CHANGELOG 说明。
4. 如果影响 CLI/`freebird.toml`：同步 `USAGE.md`、`AGENTS.md` 与 `STATUS.md`。
5. 破坏性变更需在下一版本发布前保持至少一个 `Unreleased` 周期。

## 3. 发布前检查清单

在仓库根运行：

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --release
cargo run --release --bin freebird-bench -- --check --tolerance 2.5
python3 scripts/check-doc-links.py
python3 scripts/check-fixtures.py
python3 scripts/check-docs.py
```

同时确认：

- 新增/修改的 fixture 能被 `tests/differential_test.rs` 自动发现；
- 诊断快照已更新（`UPDATE_SNAPSHOTS=1 cargo test --test snapshot_test`）且变更合理；
- `CHANGELOG.md` 有对应版本条目，`Cargo.toml` 版本与其一致；
- `AGENTS.md`、`STATUS.md`、`ROADMAP.md` 的当前测试基线一致；
- examples 在 VM 与 Stardust 上都能运行。

## 4. 发布步骤

1. 更新 `Cargo.toml` 的 `version` 与 `CHANGELOG.md`（把 `Unreleased` 内容移入新版本条目）。
2. 运行第 3 节检查。
3. 提交并打 tag：`git tag vX.Y.Z && git push origin main --tags`。
4. GitHub Actions 的 `Release` workflow（`.github/workflows/release.yml`）会：
   - 校验 tag 与 `Cargo.toml` 版本一致；
   - `cargo build --release`；
   - 发布 `freebird-linux-x86_64` 与 `.sha256`。
5. 发布说明引用 `CHANGELOG.md` 的对应条目；破坏性变更附迁移步骤。

## 5. 兼容性边界

- **稳定**：CLI 子命令/选项、`freebird check` 的 JSON `schema_version`、诊断错误码、
  `stardust!` 指令名、`freebird.toml` 的 `[package]`/`[dependencies]` 路径语义。
- **可演进**：生成的 Stardust 源码细节（函数名分配、注释、堆槽编号）、LSP/DAP 的
  文本级行为、MIR 指令序列。
- **已知限制**：包管理无版本解析/锁定文件；
  DAP 单线程；LSP 为文本级 MVP。这些在 `STATUS.md`/`ROADMAP.md` 维护。

## 6. 文档一致性

`scripts/check-docs.py` 检查：

- `AGENTS.md` 与 `STATUS.md` 顶部测试基线一致；
- `ROADMAP.md` 最后一条“完成后基线”与它们一致；
- `Cargo.toml` 版本在 `CHANGELOG.md` 中有对应 `## [x.y.z]` 条目，且存在 `## [Unreleased]`。

该脚本在 CI 的 `test` job 中运行；基线变化时同步更新三份文档。

## 7. CI 门禁策略

`.github/workflows/ci.yml` 的 job 分为阻塞与告警两类：

| job | 阻塞合并 | 说明 |
|---|---|---|
| `test` | 是 | `cargo fmt --check`、`clippy -D warnings`、`cargo test`、文档链接/一致性、fixture、MIR 占位符、examples 冒烟 |
| `differential` | 是 | VM/Stardust 差分测试 |
| `llvm` | 是 | 安装 clang 后跑 `m3_test`（bundled Stardust 工具链） |
| `bench` | 否 | 基准受机器噪声影响，`--tolerance 2.5`，只告警；噪声基线与复现方法见 [`bench/README.md`](../bench/README.md) |
| `miri` | 是 | nightly + `cargo miri setup`，核心模块 `vm::`/`mir::`/`borrowck::` |
| `asan` | 否（过渡期） | nightly + `rust-src`，`-Zsanitizer=address` 跑核心模块；首次接入保持告警，稳定后转阻塞 |
| `ubsan` | 否（过渡期） | 同上，`-Zsanitizer=undefined`（与 ASan 分开运行） |
| `fuzz` | 否（仅定时/手动） | 每周一 03:00 UTC + `workflow_dispatch`；`FREEBIRD_FUZZ_SCALE=10` 跑 `tests/m5_test.rs`（固定 corpus + 随机用例），失败上传 `target/fuzz-failures` |

`miri` job 的约定（M6-4.1，M6 收尾转阻塞）：

- 使用 `MIRIFLAGS="-Zmiri-disable-isolation"`，因为核心模块的测试会访问临时文件；
- 该 job 已从 `continue-on-error` 转为阻塞门禁；如后续暴露新的未定义行为，应先修复或
  在对应文档登记后临时恢复告警；
- 本地 stable 工具链没有 `miri` 组件，Miri 只能在 nightly 上运行；本地验证请用
  `cargo +nightly miri test --lib vm::`（首次需 `rustup component add miri --toolchain nightly`）。

ASan/UBSan 已接入 CI（`asan`/`ubsan` job），但**仍为过渡期告警**：它们需要 nightly +
`rust-src` 与 `-Zbuild-std`，构建成本高且与 LLVM 后端的 clang 链接可能相互影响。首次接入
以 `continue-on-error` 观察；连续稳定后按同样方式改为阻塞。本地工具链目录只读时无法安装
`miri`/`rust-src`，此时只能依赖 CI 验证（M6 收尾的实测结论）。

## 8. 平台支持

- **发布产物**：`freebird-linux-x86_64`（+ `.sha256`）。`release.yml` 只在 `ubuntu-latest`
  上构建，由 `v*` tag 触发；tag 必须与 `Cargo.toml` 版本一致。
- **代码层面**：VM 后端、前端/类型/借用检查与 `fmt`/`lsp`/`dap`/`repl`/`check` 只依赖 `std`；
  `#[cfg(unix)]` 仅用于可执行位检查（`codegen/llvm.rs`）。CI 的 `cross-platform` job 在
  macOS/Windows 上运行 `cargo check --all-targets` 做编译探测（`continue-on-error`，不阻塞合并）。
- **Stardust / LLVM 后端**：需要对应平台的 `stardust` 可执行文件。仓库自带的
  `stardust/stardust-linux-x86_64` 与兄弟仓库 `esolang_stardust` 目前只有 Linux x86_64 构建；
  在 macOS/Windows 上使用 Stardust/LLVM 目标需要自行构建 stardust，并通过
  `--stardust-bin` 或 `STARDUST_BIN` 指定（LLVM 目标还需要同平台的 clang）。
- **结论（M6-4.5）**：当前明确只支持并发布 Linux x86_64。多平台发布依赖 Stardust 工具链
  在对应平台产出二进制，作为后续工作跟踪。
