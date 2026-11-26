# 发布与兼容流程

> 适用版本：实现 0.1.1 / 规范 v0.5.0
> 最后更新：2026-11-26
> 面向维护者。版本号体系（两条版本线、SemVer、tag 与预发布通道）见
> [`VERSIONING.md`](VERSIONING.md)；本文只描述发布操作与 CI 门禁。

## 1. 版本策略

- 实现版本遵循 SemVer 2.0.0，唯一来源是 `Cargo.toml`；语言规范版本独立记录在
  `readme.md` 顶部。两者的规则与 bump 决策见 [`VERSIONING.md`](VERSIONING.md)。
- 发布 tag 必须与 `Cargo.toml` 版本一致；预发布使用 `vX.Y.Z-rc.N`。

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
python3 scripts/check-version.py
./scripts/package-release.sh
```

同时确认：

- 新增/修改的 fixture 能被 `tests/differential_test.rs` 自动发现；
- 诊断快照已更新（`UPDATE_SNAPSHOTS=1 cargo test --test snapshot_test`）且变更合理；
- `CHANGELOG.md` 有对应版本条目，`Cargo.toml` 版本与其一致；
- `AGENTS.md`、`STATUS.md`、`ROADMAP.md` 的当前测试基线一致；
- examples 在 VM 与 Stardust 上都能运行。

## 4. 发布步骤

1. 更新 `Cargo.toml` 的 `version` 与 `CHANGELOG.md`（把 `Unreleased` 内容移入新版本条目）；
   同步 `docs/LANGUAGE.md`、`docs/VERSIONING.md` 的版本头与版本对照表。
2. 运行第 3 节检查（包含 `check-version.py` 与 `package-release.sh`）。
3. 提交并打 tag：`git tag vX.Y.Z && git push origin main --tags`
   （预发布用 `vX.Y.Z-rc.N`）。
4. GitHub Actions 的 `Release` workflow（`.github/workflows/release.yml`）会：
   - **verify**：`scripts/check-version.py --tag <tag>` 校验 tag/Cargo/CHANGELOG/规范版本，
     并从 `CHANGELOG.md` 提取对应版本段落为 Release 说明；
   - **test**：与 CI 相同的 fmt/clippy/全量测试/文档与版本检查，失败则不发布；
   - **release**：运行 `scripts/package-release.sh`，创建 GitHub Release 并上传产物；
     含 `-` 的 tag 自动标记为 prerelease。
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
- `Cargo.toml` 版本在 `CHANGELOG.md` 中有对应 `## [x.y.z]` 条目，且存在 `## [Unreleased]`；
- `docs/` 下的用户文档带有 `适用版本` 与 `最后更新` 头，且 `docs/LANGUAGE.md`
  保留完整章节；`docs/README.md` 索引覆盖所有用户文档。

`scripts/check-version.py` 校验实现版本/规范版本/tag 与 CHANGELOG 的一致性。
两者都在 CI 的 `docs` job 中运行；基线或版本变化时同步更新文档。

## 7. CI 门禁策略

`.github/workflows/ci.yml` 的 job 分为阻塞与告警两类：

| job | 阻塞合并 | 说明 |
|---|---|---|
| `quality` | 是 | `cargo fmt --check`、`clippy -D warnings`、`cargo build --release --locked`、全量 `cargo test`、examples 冒烟 |
| `docs` | 是 | 文档链接、fixture、文档一致性、版本一致性、MIR 占位符 |
| `differential` | 是 | VM/Stardust 差分测试 |
| `llvm` | 是 | 安装 clang 后跑 `m3_test`（bundled Stardust 工具链） |
| `miri` | 是 | nightly + `cargo miri setup`，核心模块 `vm::`/`mir::`/`borrowck::`（跳过名字含 `llvm` 的测试，避免匹配 `codegen::llvm::`） |
| `asan` | 否（过渡期） | nightly + `rust-src`，`-Zsanitizer=address` 跑核心模块（同样跳过 `llvm` 测试） |
| `bench` | 否 | 基准受机器噪声影响，`--tolerance 2.5`，只告警 |
| `fuzz` | 否（仅定时/手动） | 每周一 03:00 UTC + `workflow_dispatch`（可传 `fuzz_scale`）；失败上传 `target/fuzz-failures` |
| `cross-platform` | 否 | macOS/Windows `cargo check --all-targets` 编译探测 |
| `package` | 是（依赖 quality/differential/llvm） | 运行 `scripts/package-release.sh`，上传 `dist/` artifact（保留 7 天）并写入 checksum 摘要 |

`quality`、`docs`、`differential`、`llvm`、`miri`、`package` 是合并前的阻塞门禁；
`asan` 首次接入保持告警，连续稳定后按同样方式改为阻塞；`bench` 与
`cross-platform` 只提供趋势/编译探测。所有 job 都设置了 `timeout-minutes`，
同一 ref 的新运行会取消旧运行（定时 fuzz 除外）。

`miri` job 的约定（M6-4.1，M6 收尾转阻塞）：

- 使用 `MIRIFLAGS="-Zmiri-disable-isolation"`，因为核心模块的测试会访问临时文件；
- 该 job 已从 `continue-on-error` 转为阻塞门禁；如后续暴露新的未定义行为，应先修复或
  在对应文档登记后临时恢复告警；
- 本地 stable 工具链没有 `miri` 组件，Miri 只能在 nightly 上运行；本地验证请用
  `cargo +nightly miri test --lib -- vm:: --skip llvm`（首次需
  `rustup component add miri --toolchain nightly`）。`--skip llvm` 是必需的：`vm::`
  按子串也会匹配 `codegen::llvm::`，而 LLVM 测试会调用外部 `stardust` 进程，Miri 不支持。

ASan 已接入 CI（`asan` job），但**仍为过渡期告警**：它需要 nightly + `rust-src` 与
`-Zbuild-std`，构建成本高且与 LLVM 后端的 clang 链接可能相互影响。首次接入以
`continue-on-error` 观察；连续稳定后按同样方式改为阻塞。UBSan 已移除：当前 nightly 的
`-Zsanitizer` 支持列表不再包含 `undefined`（`rustc` 直接报错），UB 检测由阻塞的 `miri` job
覆盖。本地工具链目录只读时无法安装 `miri`/`rust-src`，此时只能依赖 CI 验证。

## 8. 发布产物与打包

`scripts/package-release.sh` 是唯一的打包入口（本地与 CI/Release 共用）：

- 构建 `cargo build --release --locked`；
- 生成 `dist/freebird-<version>-x86_64-unknown-linux-gnu/`，包含：
  `freebird` 二进制、`readme.md`、`USAGE.md`、`CHANGELOG.md`、`LICENSE`、
  `docs/`（`LANGUAGE.md`、`VERSIONING.md`、`release.md`、ADR、审计）、`RELEASE-MANIFEST.txt`；
- 生成 `.tar.gz`、对应 `.sha256` 与汇总 `dist/SHA256SUMS`。

CI 的 `package` job 在每次合并前验证打包可用；`Release` workflow 在 tag 上复用同一脚本，
并把 `.tar.gz`、`.sha256`、`SHA256SUMS` 上传到 GitHub Release。

## 9. 平台支持

- **发布产物**：`freebird-<version>-x86_64-unknown-linux-gnu.tar.gz`、对应
  `.sha256` 与汇总 `SHA256SUMS`，由 `scripts/package-release.sh` 生成。`release.yml`
  只在 `ubuntu-latest` 上构建，由 `v*` tag 触发；tag 必须与 `Cargo.toml` 版本一致。
- **代码层面**：VM 后端、前端/类型/借用检查与 `fmt`/`lsp`/`dap`/`repl`/`check` 只依赖 `std`；
  `#[cfg(unix)]` 仅用于可执行位检查（`codegen/llvm.rs`）。CI 的 `cross-platform` job 在
  macOS/Windows 上运行 `cargo check --all-targets` 做编译探测（`continue-on-error`，不阻塞合并）。
- **Stardust / LLVM 后端**：需要对应平台的 `stardust` 可执行文件。仓库自带的
  `stardust/stardust-linux-x86_64` 与兄弟仓库 `esolang_stardust` 目前只有 Linux x86_64 构建；
  在 macOS/Windows 上使用 Stardust/LLVM 目标需要自行构建 stardust，并通过
  `--stardust-bin` 或 `STARDUST_BIN` 指定（LLVM 目标还需要同平台的 clang）。
- **结论（M6-4.5）**：当前明确只支持并发布 Linux x86_64。多平台发布依赖 Stardust 工具链
  在对应平台产出二进制，作为后续工作跟踪。
