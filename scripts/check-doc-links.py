#!/usr/bin/env python3
"""检查 Markdown 文档中的相对链接目标是否存在（M4-7）。

用法：`python3 scripts/check-doc-links.py`（在仓库任意目录运行均可）。
只检查仓库内的相对路径链接；`http(s)`、`mailto:`、页内锚点会跳过。
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")


def markdown_files() -> list[Path]:
    files = [
        ROOT / "readme.md",
        ROOT / "USAGE.md",
        ROOT / "STATUS.md",
        ROOT / "ROADMAP.md",
        ROOT / "AGENTS.md",
        ROOT / "TASKS.md",
        ROOT / "IMPROVEMENTS.md",
        ROOT / "vscode" / "README.md",
    ]
    files.extend(sorted((ROOT / "docs").rglob("*.md")))
    return [path for path in files if path.exists()]


def main() -> int:
    errors: list[str] = []
    for path in markdown_files():
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(ROOT)
        for match in LINK.finditer(text):
            target = match.group(1).strip()
            if target.startswith(("http://", "https://", "mailto:", "#", "tel:")):
                continue
            target = target.split("#", 1)[0]
            if not target:
                continue
            candidate = (path.parent / target).resolve()
            if not candidate.exists():
                errors.append(f"{relative}: missing link target `{target}`")

    if errors:
        print("\n".join(errors))
        return 1
    print(f"checked {len(markdown_files())} markdown files: links OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
