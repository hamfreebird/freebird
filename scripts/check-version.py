#!/usr/bin/env python3
"""校验 Freebird 版本号系统的单一来源与一致性。

规则：
- `Cargo.toml` 的 `[package].version` 必须是合法 SemVer 2.0.0；
- `src/main.rs` 的 CLI 版本必须使用 `env!("CARGO_PKG_VERSION")`，禁止硬编码；
- `CHANGELOG.md` 必须包含 `## [Unreleased]` 与当前版本的 `## [x.y.z]`；
- `readme.md` 顶部必须有规范版本 `> **vX.Y.Z**`；
- `docs/VERSIONING.md` 必须存在并保留关键小节；
- 传入 `--tag vX.Y.Z[-pre.N]` 时，tag 必须与 Cargo 版本完全一致。

用法：
    python3 scripts/check-version.py
    python3 scripts/check-version.py --tag v0.1.0
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

SEMVER = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-(?:alpha|beta|rc)\.(?:0|[1-9]\d*))?"
    r"(?:\+[0-9A-Za-z.-]+)?$"
)
SPEC_VERSION = re.compile(r"^> \*\*v(\d+\.\d+\.\d+)\*\*", re.MULTILINE)
CARGO_VERSION = re.compile(r'^version\s*=\s*"([^"]+)"', re.MULTILINE)
REQUIRED_VERSIONING_SECTIONS = [
    "两条版本线",
    "版本格式",
    "bump 规则",
    "Tag 与发布通道",
    "单一来源与自动化校验",
    "兼容性承诺",
]


def read(name: str) -> str:
    return (ROOT / name).read_text(encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--tag",
        help="发布 tag（可带前导 v），例如 v0.2.0 或 v0.2.0-rc.1",
    )
    args = parser.parse_args()

    errors: list[str] = []

    cargo_match = CARGO_VERSION.search(read("Cargo.toml"))
    if cargo_match is None:
        print("Cargo.toml: missing package version")
        return 1
    version = cargo_match.group(1)
    if not SEMVER.match(version):
        errors.append(
            f"Cargo.toml: `{version}` is not SemVer 2.0.0 "
            "(expected MAJOR.MINOR.PATCH[-alpha.N|-beta.N|-rc.N][+build])"
        )

    if "CARGO_PKG_VERSION" not in read("src/main.rs"):
        errors.append(
            "src/main.rs: CLI version must use env!(\"CARGO_PKG_VERSION\") "
            "instead of a hard-coded string"
        )

    changelog = read("CHANGELOG.md")
    if "## [Unreleased]" not in changelog:
        errors.append("CHANGELOG.md: missing `## [Unreleased]` section")
    if f"## [{version}]" not in changelog:
        errors.append(
            f"CHANGELOG.md: missing `## [{version}]` section for Cargo version {version}"
        )

    spec = SPEC_VERSION.search(read("readme.md"))
    if spec is None:
        errors.append(
            "readme.md: missing specification version header `> **vX.Y.Z**`"
        )

    versioning = ROOT / "docs" / "VERSIONING.md"
    if not versioning.exists():
        errors.append("docs/VERSIONING.md: missing versioning policy document")
    else:
        text = versioning.read_text(encoding="utf-8")
        for section in REQUIRED_VERSIONING_SECTIONS:
            if section not in text:
                errors.append(
                    f"docs/VERSIONING.md: missing section `{section}`"
                )

    if args.tag:
        tag = args.tag[1:] if args.tag.startswith("v") else args.tag
        if tag != version:
            errors.append(
                f"release tag `{args.tag}` does not match Cargo version `{version}`"
            )

    if errors:
        print("\n".join(errors))
        return 1

    spec_text = spec.group(1) if spec else "?"
    tag_text = f", tag v{args.tag.lstrip('v')}" if args.tag else ""
    print(
        f"version consistency OK: implementation {version}, "
        f"specification v{spec_text}{tag_text}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
