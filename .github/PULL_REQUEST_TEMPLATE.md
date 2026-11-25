# Pull Request

## 变更说明

<!-- 这个 PR 做了什么、为什么；若有破坏性变更，给出迁移方式。 -->

## 检查清单

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`（新增/修改行为有测试；双后端一致时使用 `assert_same_output`）
- [ ] 文档同步：按需更新 `docs/LANGUAGE.md` / `USAGE.md` / `STATUS.md` / `CHANGELOG.md`
- [ ] 若涉及诊断码：更新 `docs/LANGUAGE.md` 第 19 节与 `tests/snapshots/diagnostics.txt`
- [ ] 若涉及版本/发布：更新 `docs/VERSIONING.md` / `docs/release.md` 与 `docs/` 版本头
- [ ] `python3 scripts/check-doc-links.py`
- [ ] `python3 scripts/check-fixtures.py`
- [ ] `python3 scripts/check-docs.py`
- [ ] `python3 scripts/check-version.py`

## 备注

<!-- 已知限制、后续工作或需要 reviewer 关注的点。 -->
