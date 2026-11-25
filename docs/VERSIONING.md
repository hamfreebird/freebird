# Freebird 版本号与发布策略

> 适用版本：实现 0.1.1 / 规范 v0.5.0
> 最后更新：2026-11-25

本文定义 Freebird 的版本号体系。发布操作步骤见 [`release.md`](release.md)，
语言能力矩阵见 [`../STATUS.md`](../STATUS.md)。

## 1. 两条版本线

Freebird 同时维护两个独立版本：

| 版本线 | 唯一来源 | 含义 |
|---|---|---|
| **实现版本** | `Cargo.toml` 的 `[package].version` | 编译器/VM/工具链的发布版本；`freebird --version` 输出它 |
| **语言规范版本** | `readme.md` 顶部的 `> **vX.Y.Z**` | 语言语义与目标规范的版本，可先于实现演进 |

当前对照：实现 `0.1.1` ↔ 规范 `v0.5.0`。两者的差异必须在本节或
`STATUS.md` 中可查；规范变化不强制立即提升实现版本，但实现新增语言能力时
应在 `CHANGELOG.md` 记录对应的规范章节。

## 2. 版本格式

实现版本遵循 [Semantic Versioning 2.0.0](https://semver.org/lang/zh-CN/)：

```text
MAJOR.MINOR.PATCH[-alpha.N | -beta.N | -rc.N][+build]
```

- `MAJOR`：不兼容的公开 API/语义变更。
- `MINOR`：向后兼容的新增能力。
- `PATCH`：向后兼容的修复、文档、性能与内部重构。
- 预发布标识只在发布候选阶段使用，优先级：`alpha < beta < rc < 正式版`。
- 构建元数据（`+build.N`）不参与版本比较，只用于标识本地/CI 构建。

示例：`0.2.0`、`0.2.0-rc.1`、`1.0.0`、`1.1.0+build.20261125`。

## 3. bump 规则

| 变更类型 | 0.y.z 阶段 | 1.0.0 之后 |
|---|---|---|
| 修复、文档、性能、内部重构 | `PATCH` | `PATCH` |
| 新增向后兼容的语言/CLI 能力 | `MINOR` | `MINOR` |
| 破坏性语言/CLI/诊断语义变更 | `MINOR`，但必须在 `CHANGELOG.md` 的 **Changed/Removed** 中给出迁移步骤 | `MAJOR` |
| 仅规范文本修订、实现未变 | 不强制 bump 实现版本 | 同左 |
| 仅依赖/CI 变化 | `PATCH`（或随下一次发布合并） | `PATCH` |

0.y.z 阶段允许 `MINOR` 携带破坏性变更，但**不得静默**：必须同时更新
`CHANGELOG.md`、受影响的文档（`docs/LANGUAGE.md`、`USAGE.md`、`STATUS.md`）
并在迁移说明中写清替代写法。

## 4. Tag 与发布通道

- 正式版 tag：`v<version>`，例如 `v0.2.0`。
- 预发布 tag：`v<version>-<pre>.<N>`，例如 `v0.2.0-rc.1`。
- tag 必须与 `Cargo.toml` 的实现版本完全一致（含预发布后缀）。
- 预发布 tag 发布的 GitHub Release 标记为 prerelease，不覆盖正式版说明。
- **不要移动或复用已发布的 tag**；发现问题时发布新的 `PATCH`（或更高的预发布号）。

发布通道：

1. `alpha.N`：内部可用，接口可能随时变化。
2. `beta.N`：功能冻结，只修 bug 与文档。
3. `rc.N`：候选发布，只接受阻塞性修复。
4. 正式版：通过完整 CI 与发布检查清单。

## 5. 单一来源与自动化校验

- `Cargo.toml` 是实现版本的唯一来源；`src/main.rs` 的 clap 版本必须使用
  `env!("CARGO_PKG_VERSION")`，禁止硬编码。
- `CHANGELOG.md` 必须包含 `## [Unreleased]` 与当前版本的 `## [x.y.z]` 条目。
- `scripts/check-version.py` 校验：
  - SemVer 格式；
  - `main.rs` 使用 `CARGO_PKG_VERSION`；
  - `CHANGELOG.md` 的 `Unreleased` 与当前版条目；
  - `readme.md` 的规范版本格式；
  - `docs/VERSIONING.md` 存在且保留本文的关键小节；
  - 传入 `--tag vX.Y.Z` 时，tag 与 `Cargo.toml` 一致。
- CI 的 `docs` job 与 `Release` workflow 都会运行该脚本；发布前也可本地运行：

```bash
python3 scripts/check-version.py
python3 scripts/check-version.py --tag v0.1.0
```

## 6. 兼容性承诺

- **0.x**：尽量保持源码兼容；破坏性变更必须在 `CHANGELOG.md` 标注迁移方式。
  诊断错误码（`P/R/T/B/W` + 三位数字）尽量稳定，确需变更时单独说明。
- **1.0.0 起**：遵循 SemVer；CLI 子命令/选项、`freebird check` 的 JSON
  `schema_version: 1`、`freebird.toml` 的 `[package]`/`[dependencies]` 路径语义
  属于稳定接口。
- **可演进**：生成的 Stardust 源码细节（函数名分配、注释、堆槽编号）、MIR 指令序列、
  LSP/DAP 的文本级行为不承诺跨版本兼容。
- bundled Stardust 二进制与语言规范版本独立记录；升级 bundled 工具链时在
  `CHANGELOG.md` 与 `STATUS.md` 注明。

## 7. 版本对照表

| 实现版本 | 规范版本 | 说明 |
|---|---|---|
| `0.1.0` | `v0.5.0` | M0–M6 基线；M6-5 部分语言演进落地 |
| `0.1.1` | `v0.5.0` | trait 方法完整性（R010）、`const fn`/`if`/块求值、LSP 语义化 hover/completion |

新增发布时在本表追加一行，并同步 `CHANGELOG.md`。
