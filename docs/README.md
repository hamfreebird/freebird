# Freebird 文档索引

> 适用版本：实现 0.1.1 / 规范 v0.5.0
> 最后更新：2026-11-25

本目录维护面向用户的文档与维护者流程。所有用户文档都带有版本头，并由
`scripts/check-docs.py` 在 CI 中校验。

## 用户与维护者文档

| 文档 | 内容 |
|---|---|
| [`LANGUAGE.md`](LANGUAGE.md) | 完整语言文档：语法、类型、模式匹配、泛型、trait、闭包、const、模块、unsafe/FFI、运行时限制、标准库、工具链、错误码、限制与示例 |
| [`VERSIONING.md`](VERSIONING.md) | 版本号系统：实现版本/规范版本、SemVer、bump 规则、tag 与预发布通道、校验脚本 |
| [`release.md`](release.md) | 发布与兼容流程：发布检查清单、CI 门禁、打包产物、平台支持 |
| [`adr/`](adr/) | 架构决策记录（堆模型、回收策略、impl、unsafe 栈、Call arity、语言演进） |
| [`audits/`](audits/) | 审计记录（内存/UB 审计快照） |

仓库根部的相关文档：

- [`../readme.md`](../readme.md) — 语言目标规范（权威语义）
- [`../USAGE.md`](../USAGE.md) — CLI 与工具链用法
- [`../STATUS.md`](../STATUS.md) — 规范 × 后端 × 测试能力矩阵
- [`../ROADMAP.md`](../ROADMAP.md) — 里程碑、基线与后续计划
- [`../CHANGELOG.md`](../CHANGELOG.md) — 逐版本变更记录
- [`../AGENTS.md`](../AGENTS.md) — 维护者命令、架构与约定速览

## 文档维护规则

1. **版本头**：`docs/` 下的用户文档（本文件、`LANGUAGE.md`、`VERSIONING.md`、
   `release.md`）必须以以下两行开头（紧跟在标题之后）：

   ```text
   > 适用版本：实现 <Cargo 版本> / 规范 v<规范版本>
   > 最后更新：YYYY-MM-DD
   ```

   发版或规范版本变化时同步更新；`scripts/check-docs.py` 会校验实现版本与
   `Cargo.toml` 一致。

2. **同步矩阵**：语言/CLI/诊断码/版本流程的变更必须在同一提交内更新对应文档。
   完整矩阵与最低要求见 [`LANGUAGE.md` §22](LANGUAGE.md#22-文档同步规则)。

3. **索引完整**：新增用户文档时在本页登记；`scripts/check-docs.py` 校验
   `LANGUAGE.md`、`VERSIONING.md`、`release.md`、`adr/`、`audits/` 都在索引中。

4. **提交前检查**：

   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   python3 scripts/check-doc-links.py
   python3 scripts/check-fixtures.py
   python3 scripts/check-docs.py
   python3 scripts/check-version.py
   ```

   CI 的 `docs` job 会阻塞不满足要求的提交；`Release` workflow 在发版前重复执行
   这些检查。

5. **链接**：文档间使用相对路径链接；`scripts/check-doc-links.py` 检查失效链接。
