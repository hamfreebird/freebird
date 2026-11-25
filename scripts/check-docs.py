#!/usr/bin/env python3
"""检查文档测试基线与版本一致性（M5-6）。

规则：
- `AGENTS.md` 与 `STATUS.md` 顶部的 “N unit + M integration” 必须一致；
- `ROADMAP.md` 顶部最后一条 “完成后基线” 必须与它们一致；
- `Cargo.toml` 的版本必须在 `CHANGELOG.md` 中有对应 `## [x.y.z]` 条目；
- `CHANGELOG.md` 必须保留 `## [Unreleased]`。

用法：`python3 scripts/check-docs.py`
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASELINE = re.compile(r"(\d+)\s+unit\s*\+\s*(\d+)\s+integration")
ROADMAP_BASELINE = re.compile(
    r"^> \*\*[^\n]*?基线[^\n]*?(\d+)\s+unit\s*\+\s*(\d+)\s+integration",
    re.MULTILINE,
)

USER_DOCS = [
    "docs/LANGUAGE.md",
    "docs/VERSIONING.md",
    "docs/release.md",
    "docs/README.md",
]
DOC_VERSION = re.compile(r"^> 适用版本：实现 (\d+\.\d+\.\d+)", re.MULTILINE)
DOC_UPDATED = re.compile(r"^> 最后更新：(\d{4}-\d{2}-\d{2})", re.MULTILINE)
LANGUAGE_SECTIONS = [
    "快速开始",
    "词法",
    "类型系统",
    "变量",
    "表达式",
    "控制流",
    "模式匹配",
    "函数",
    "结构体",
    "泛型",
    "trait",
    "闭包",
    "const",
    "模块",
    "unsafe",
    "运行时限制",
    "标准库",
    "工具链",
    "诊断",
    "已知限制",
    "示例",
    "文档同步",
]
DOC_INDEX_LINKS = [
    "LANGUAGE.md",
    "VERSIONING.md",
    "release.md",
    "adr/",
    "audits/",
]


def first_baseline(name: str):
    match = BASELINE.search((ROOT / name).read_text(encoding="utf-8"))
    return (int(match.group(1)), int(match.group(2))) if match else None


def roadmap_baselines():
    text = (ROOT / "ROADMAP.md").read_text(encoding="utf-8")
    return [(int(a), int(b)) for a, b in ROADMAP_BASELINE.findall(text)]


def main() -> int:
    errors: list[str] = []

    agents = first_baseline("AGENTS.md")
    status = first_baseline("STATUS.md")
    roadmap = roadmap_baselines()

    if agents is None:
        errors.append("AGENTS.md: no `N unit + M integration` baseline found")
    if status is None:
        errors.append("STATUS.md: no `N unit + M integration` baseline found")
    if not roadmap:
        errors.append("ROADMAP.md: no `> **...基线**` entries found")
    if agents and status and agents != status:
        errors.append(
            f"test baselines disagree: AGENTS={agents} STATUS={status}"
        )
    if agents and roadmap and agents != roadmap[-1]:
        errors.append(
            f"ROADMAP latest baseline {roadmap[-1]} != AGENTS baseline {agents}"
        )

    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    version_match = re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.MULTILINE)
    if version_match is None:
        errors.append("Cargo.toml: missing package version")
        version = None
    else:
        version = version_match.group(1)

    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    if "## [Unreleased]" not in changelog:
        errors.append("CHANGELOG.md: missing `## [Unreleased]` section")
    if version and f"## [{version}]" not in changelog:
        errors.append(
            f"CHANGELOG.md: missing `## [{version}]` section for Cargo version {version}"
        )

    docs_checked = 0
    for name in USER_DOCS:
        path = ROOT / name
        if not path.exists():
            errors.append(f"{name}: missing user document")
            continue
        text = path.read_text(encoding="utf-8")
        doc_version = DOC_VERSION.search(text)
        if doc_version is None:
            errors.append(f"{name}: missing `> 适用版本：实现 X.Y.Z` header")
        elif version and doc_version.group(1) != version:
            errors.append(
                f"{name}: header implementation version {doc_version.group(1)} "
                f"!= Cargo.toml version {version}"
            )
        if DOC_UPDATED.search(text) is None:
            errors.append(f"{name}: missing `> 最后更新：YYYY-MM-DD` header")
        docs_checked += 1

    language_path = ROOT / "docs/LANGUAGE.md"
    if language_path.exists():
        language = language_path.read_text(encoding="utf-8")
        for section in LANGUAGE_SECTIONS:
            if section not in language:
                errors.append(
                    f"docs/LANGUAGE.md: missing section or keyword `{section}`"
                )

    index_path = ROOT / "docs/README.md"
    if index_path.exists():
        index = index_path.read_text(encoding="utf-8")
        for link in DOC_INDEX_LINKS:
            if link not in index:
                errors.append(f"docs/README.md: index does not reference `{link}`")

    if errors:
        print("\n".join(errors))
        return 1
    print(
        "docs consistency OK: baseline "
        f"{agents[0]} unit + {agents[1]} integration, version {version}, "
        f"user docs {docs_checked}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
