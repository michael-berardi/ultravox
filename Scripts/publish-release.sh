#!/usr/bin/env bash
set -euo pipefail

export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-$(node -p "require('${ROOT_DIR}/apps/desktop/package.json').version")}"
TAG="v${VERSION}"
OUTPUT_DIR="${OUTPUT_DIR:-${ROOT_DIR}/release}"

if ! command -v gh >/dev/null 2>&1; then
  echo "GitHub CLI is required: https://cli.github.com/" >&2
  exit 1
fi
if ! gh auth status >/dev/null 2>&1; then
  echo "Authenticate GitHub CLI with: gh auth login" >&2
  exit 1
fi
if [[ -n "$(git -C "$ROOT_DIR" status --porcelain --untracked-files=no)" ]]; then
  echo "Commit tracked changes before publishing a release." >&2
  exit 1
fi

if [[ "${ALLOW_ADHOC:-0}" == "1" ]]; then
  echo "Public releases cannot use ad-hoc signatures." >&2
  exit 1
fi
if [[ "${SKIP_PACKAGE:-0}" != "1" ]]; then
  OUTPUT_DIR="$OUTPUT_DIR" "${ROOT_DIR}/Scripts/package-release.sh"
fi

PKG_PATH="${OUTPUT_DIR}/UltraVox-macos-arm64.pkg"
PKG_CHECKSUM="${PKG_PATH}.sha256"
ARCHIVE_PATH="${OUTPUT_DIR}/UltraVox-macos-arm64.zip"
ARCHIVE_CHECKSUM="${ARCHIVE_PATH}.sha256"
ASSETS=("$PKG_PATH" "$PKG_CHECKSUM" "$ARCHIVE_PATH" "$ARCHIVE_CHECKSUM")
for asset in "${ASSETS[@]}"; do
  [[ -f "$asset" ]] || {
    echo "Missing required release asset: $asset" >&2
    exit 1
  }
done
(
  cd "$OUTPUT_DIR"
  shasum -a 256 --check "$(basename "$PKG_CHECKSUM")"
  shasum -a 256 --check "$(basename "$ARCHIVE_CHECKSUM")"
)
EXPECTED_VERSION="$VERSION" REQUIRE_SIGNED=1 REQUIRE_NOTARIZED=1 ALLOW_ADHOC=0 \
  "${ROOT_DIR}/Scripts/verify-pkg.sh" "$PKG_PATH"

ARCHIVE_CHECK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ultravox-publish-check.XXXXXX")"
trap 'rm -rf "$ARCHIVE_CHECK_DIR"' EXIT
ditto -x -k "$ARCHIVE_PATH" "$ARCHIVE_CHECK_DIR"
ARCHIVE_APP="${ARCHIVE_CHECK_DIR}/UltraVox-macos-arm64/UltraVox.app"
ARCHIVE_CLI="${ARCHIVE_CHECK_DIR}/UltraVox-macos-arm64/bin/ultravox-control"
EXPECTED_VERSION="$VERSION" REQUIRE_SIGNED=1 REQUIRE_NOTARIZED=1 \
  "${ROOT_DIR}/Scripts/verify-app.sh" "$ARCHIVE_APP"
codesign --verify --strict "$ARCHIVE_CLI"
codesign -dv --verbose=4 "$ARCHIVE_CLI" 2>&1 |
  grep -E '^TeamIdentifier=T63VT9UAY2$' >/dev/null

HEAD_SHA="$(git -C "$ROOT_DIR" rev-parse HEAD)"
if git -C "$ROOT_DIR" rev-parse "$TAG" >/dev/null 2>&1; then
  TAG_SHA="$(git -C "$ROOT_DIR" rev-list -n 1 "$TAG")"
  if [[ "$TAG_SHA" != "$HEAD_SHA" ]]; then
    echo "$TAG already points to a different commit." >&2
    exit 1
  fi
else
  git -C "$ROOT_DIR" tag -a "$TAG" -m "UltraVox $TAG"
fi

git -C "$ROOT_DIR" push origin HEAD
git -C "$ROOT_DIR" push origin "$TAG"
if gh release view "$TAG" --repo michael-berardi/ultravox >/dev/null 2>&1; then
  gh release upload "$TAG" "${ASSETS[@]}" --repo michael-berardi/ultravox --clobber
else
  gh release create "$TAG" "${ASSETS[@]}" --repo michael-berardi/ultravox \
    --verify-tag --generate-notes --title "UltraVox $TAG"
fi
printf 'Published https://github.com/michael-berardi/ultravox/releases/tag/%s\n' "$TAG"
