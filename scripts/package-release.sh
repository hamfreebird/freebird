#!/usr/bin/env bash
# 构建并打包 Freebird 发布产物（Linux x86_64）。
#
# 单一入口：本地与 CI 共用，避免打包逻辑分叉。
# 输出：
#   dist/freebird-<version>-<target>.tar.gz
#   dist/freebird-<version>-<target>.tar.gz.sha256
#   dist/SHA256SUMS
#
# 可选环境变量：
#   VERSION  覆盖从 Cargo.toml 读取的版本（默认读取唯一来源）
#   TARGET   覆盖目标三元组（默认 x86_64-unknown-linux-gnu）
#   DIST     覆盖输出目录（默认仓库下 dist/）
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
TARGET="${TARGET:-x86_64-unknown-linux-gnu}"
DIST="${DIST:-$ROOT/dist}"
NAME="freebird-${VERSION}-${TARGET}"
STAGE="$DIST/$NAME"

if [[ -z "$VERSION" ]]; then
  echo "error: could not read version from Cargo.toml" >&2
  exit 1
fi

if [[ ! -x "target/release/freebird" ]]; then
  cargo build --release --locked
fi

rm -rf "$STAGE"
mkdir -p "$STAGE/docs/adr" "$STAGE/docs/audits"
cp target/release/freebird "$STAGE/"
cp readme.md USAGE.md LICENSE CHANGELOG.md "$STAGE/"
cp docs/*.md "$STAGE/docs/"
cp docs/adr/*.md "$STAGE/docs/adr/"
cp docs/audits/*.md "$STAGE/docs/audits/"

{
  echo "Freebird ${VERSION}"
  echo "target: ${TARGET}"
  echo "built: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "commit: $(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
  echo
  echo "files:"
  (cd "$STAGE" && find . -type f | sort)
} > "$STAGE/RELEASE-MANIFEST.txt"

tar -C "$DIST" -czf "$DIST/$NAME.tar.gz" "$NAME"
(
  cd "$DIST"
  sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256"
  sha256sum ./*.tar.gz > SHA256SUMS
)

echo "package: $DIST/$NAME.tar.gz"
echo "checksums: $DIST/SHA256SUMS"
