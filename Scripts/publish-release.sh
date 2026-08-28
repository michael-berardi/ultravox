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
gh auth status >/dev/null
if [[ -n "$(git -C "$ROOT_DIR" status --porcelain --untracked-files=no)" ]]; then
  echo "Commit tracked changes before publishing a release." >&2
  exit 1
fi

if [[ "${PACKAGE_CURRENT:-0}" == "1" ]]; then
  OUTPUT_DIR="$OUTPUT_DIR" "${ROOT_DIR}/Scripts/package-light-release.sh"
fi

EXPECTED_ASSETS=(
  "UltraVox-Light-macos-arm64.zip"
  "UltraVox-Light-macos-arm64.zip.sha256"
  "UltraVox-Light-linux-x86_64.AppImage"
  "UltraVox-Light-linux-x86_64.AppImage.sha256"
  "UltraVox-Light-windows-x86_64-setup.exe"
  "UltraVox-Light-windows-x86_64-setup.exe.sha256"
)
ASSETS=()
for name in "${EXPECTED_ASSETS[@]}"; do
  path="${OUTPUT_DIR}/${name}"
  [[ -f "$path" ]] || {
    echo "Required release asset is missing: $path" >&2
    exit 1
  }
  ASSETS+=("$path")
done
for checksum in "${OUTPUT_DIR}"/UltraVox-Light-*.sha256; do
  (
    cd "$OUTPUT_DIR"
    shasum -a 256 --check "$(basename "$checksum")"
  )
done

HEAD_SHA="$(git -C "$ROOT_DIR" rev-parse HEAD)"
if git -C "$ROOT_DIR" rev-parse "$TAG" >/dev/null 2>&1; then
  TAG_SHA="$(git -C "$ROOT_DIR" rev-list -n 1 "$TAG")"
  [[ "$TAG_SHA" == "$HEAD_SHA" ]] || {
    echo "$TAG already points to a different commit." >&2
    exit 1
  }
else
  git -C "$ROOT_DIR" tag -a "$TAG" -m "UltraVox Light $TAG"
fi

git -C "$ROOT_DIR" push origin HEAD
git -C "$ROOT_DIR" push origin "$TAG"
if gh release view "$TAG" --repo michael-berardi/ultravox-light >/dev/null 2>&1; then
  gh release upload "$TAG" "${ASSETS[@]}" --repo michael-berardi/ultravox-light --clobber
else
  gh release create "$TAG" "${ASSETS[@]}" \
    --repo michael-berardi/ultravox-light \
    --verify-tag \
    --title "UltraVox Light $TAG" \
    --notes "Private on-device transcription for macOS, Windows, and Linux. This release adds native installers for all three supported operating systems, verified public updates, and a non-intrusive monthly invitation to support continued open-source maintenance through UltraVox Pro."
fi

printf 'Published https://github.com/michael-berardi/ultravox-light/releases/tag/%s\n' "$TAG"
