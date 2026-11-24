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
| `bench` | 否 | 基准受机器噪声影响，`--tolerance 2.5`，只告警 |
| `miri` | 否（过渡期） | nightly + `cargo miri setup`，仅跑核心模块 `vm::`/`mir::`/`borrowck::` |

`miri` job 的过渡期约定（M6-4.1）：

- 使用 `MIRIFLAGS="-Zmiri-disable-isolation"`，因为核心模块的测试会访问临时文件；
- 首次接入若暴露未定义行为/未初始化读取，先在 issue 与其对应文档中登记（基线化），
  修复完成后再把该 job 从 `continue-on-error` 改为阻塞门禁；
- 本地 stable 工具链没有 `miri` 组件，Miri 只能在 nightly 上运行；本地验证请用
  `cargo +nightly miri test --lib vm::`（首次需 `rustup component add miri --toolchain nightly`）。

ASan/UBSan 目前**未接入**：ASan 需要 `-Zbuild-std` 与 `rust-src`，构建成本高且与 LLVM 后端
的 clang 链接相互影响。需要时在本地按需运行，接入前先在 `STATUS.md` 记录结论。
