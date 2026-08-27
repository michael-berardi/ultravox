#!/usr/bin/env bash
set -euo pipefail

export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:${PATH}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-$(node -p "require('${ROOT_DIR}/apps/desktop/package.json').version")}"
TAG="v${VERSION}"
OUTPUT_DIR="${OUTPUT_DIR:-${ROOT_DIR}/release}"
LIGHT_OUTPUT_DIR="${OUTPUT_DIR}/lite"

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
  OUTPUT_DIR="$LIGHT_OUTPUT_DIR" "${ROOT_DIR}/Scripts/package-light-release.sh"
fi

if ! compgen -G "${LIGHT_OUTPUT_DIR}/UltraVox-Light-*" >/dev/null; then
  echo "No UltraVox Light release assets were staged in $LIGHT_OUTPUT_DIR." >&2
  exit 1
fi
if ! compgen -G "${LIGHT_OUTPUT_DIR}/UltraVox-Light-*.sha256" >/dev/null; then
  echo "No UltraVox Light checksum files were staged in $LIGHT_OUTPUT_DIR." >&2
  exit 1
fi
ASSETS=("${LIGHT_OUTPUT_DIR}"/UltraVox-Light-*)
for checksum in "${LIGHT_OUTPUT_DIR}"/UltraVox-Light-*.sha256; do
  (
    cd "$LIGHT_OUTPUT_DIR"
    shasum -a 256 --check "$(basename "$checksum")"
  )
done

HEAD_SHA="$(git -C "$ROOT_DIR" rev-parse HEAD)"
if git -C "$ROOT_DIR" rev-parse "$TAG" >/dev/null 2>&1; then
  TAG_SHA="$(git -C "$ROOT_DIR" rev-list -n 1 "$TAG")"
  if [[ "$TAG_SHA" != "$HEAD_SHA" ]]; then
    echo "$TAG already points to a different commit." >&2
    exit 1
  fi
else
  git -C "$ROOT_DIR" tag -a "$TAG" -m "UltraVox Light $TAG"
fi

git -C "$ROOT_DIR" push origin HEAD
git -C "$ROOT_DIR" push origin "$TAG"
if gh release view "$TAG" --repo michael-berardi/ultravox-light >/dev/null 2>&1; then
  gh release upload "$TAG" "${ASSETS[@]}" --repo michael-berardi/ultravox-light --clobber
else
  gh release create "$TAG" "${ASSETS[@]}" --repo michael-berardi/ultravox-light \
    --verify-tag --generate-notes --title "UltraVox Light $TAG"
fi

printf 'Published https://github.com/michael-berardi/ultravox-light/releases/tag/%s\n' "$TAG"
