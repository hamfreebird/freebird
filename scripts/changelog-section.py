#!/usr/bin/env python3
"""提取 CHANGELOG.md 中指定版本的段落，用于 GitHub Release 说明。

用法：
    python3 scripts/changelog-section.py 0.1.0
    python3 scripts/changelog-section.py 0.2.0-rc.1

找不到对应版本时返回 1，并把可用版本列表写到 stderr。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: changelog-section.py <version>", file=sys.stderr)
        return 2
    version = sys.argv[1]
    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    pattern = re.compile(
        rf"^## \[{re.escape(version)}\][^\n]*\n(.*?)(?=^## \[|\Z)",
        re.MULTILINE | re.DOTALL,
    )
    match = pattern.search(changelog)
    if match is None:
        available = re.findall(r"^## \[([^\]]+)\]", changelog, re.MULTILINE)
        print(
            f"CHANGELOG.md: no section for `{version}`; "
            f"available: {', '.join(available)}",
            file=sys.stderr,
        )
        return 1
    print(match.group(1).strip())
    return 0


if __name__ == "__main__":
    sys.exit(main())
