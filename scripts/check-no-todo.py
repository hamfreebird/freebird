#!/usr/bin/env python3
"""检查 MIR 生成器没有 TODO 占位（M6-1.1）。"""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MIR = ROOT / "src" / "mir"


def main() -> int:
    hits: list[str] = []
    for path in sorted(MIR.rglob("*.rs")):
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if "TODO" in line:
                hits.append(f"{path.relative_to(ROOT)}:{number}: {line.strip()}")
    if hits:
        print("\n".join(hits))
        return 1
    print("MIR has no TODO placeholders")
    return 0


if __name__ == "__main__":
    sys.exit(main())
