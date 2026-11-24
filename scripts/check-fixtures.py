#!/usr/bin/env python3
"""检查差分测试 fixture 的命名与 stdin 配对（M5-5）。

规则：
- `tests/programs/` 与 `examples/` 下的 `.fb` 使用 snake_case 文件名；
- 每个 `.in` 必须有同名 `.fb`（stdin 输入不能孤立存在）；
- 两个目录至少各有一个 fixture；
- `tests/differential_test.rs` 仍显式遍历这两个目录（保证“新增 fixture 自动进入测试”）。

用法：`python3 scripts/check-fixtures.py`
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIRECTORIES = [ROOT / "tests" / "programs", ROOT / "examples"]
NAME = re.compile(r"^[a-z0-9_]+\.fb$")


def main() -> int:
    errors: list[str] = []
    total = 0

    for directory in DIRECTORIES:
        relative = directory.relative_to(ROOT)
        if not directory.exists():
            errors.append(f"{relative}: missing fixture directory")
            continue

        fixtures = sorted(directory.glob("*.fb"))
        if not fixtures:
            errors.append(f"{relative}: no .fb fixtures")
        for fixture in fixtures:
            total += 1
            if not NAME.match(fixture.name):
                errors.append(
                    f"{fixture.relative_to(ROOT)}: fixture names must be snake_case"
                )
            if fixture.stat().st_size == 0:
                errors.append(f"{fixture.relative_to(ROOT)}: empty fixture")

        for stdin in sorted(directory.glob("*.in")):
            paired = stdin.with_suffix(".fb")
            if not paired.exists():
                errors.append(
                    f"{stdin.relative_to(ROOT)}: .in has no matching .fb fixture"
                )

    differential = ROOT / "tests" / "differential_test.rs"
    if not differential.exists():
        errors.append("tests/differential_test.rs: missing")
    else:
        text = differential.read_text(encoding="utf-8")
        for needle in ["tests/programs", "examples"]:
            if needle not in text:
                errors.append(
                    f"tests/differential_test.rs: does not discover `{needle}` fixtures"
                )

    if errors:
        print("\n".join(errors))
        return 1
    print(f"checked {total} fixtures: names and stdin pairs OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
