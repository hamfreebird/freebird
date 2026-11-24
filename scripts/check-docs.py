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

    if errors:
        print("\n".join(errors))
        return 1
    print(
        "docs consistency OK: baseline "
        f"{agents[0]} unit + {agents[1]} integration, version {version}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
