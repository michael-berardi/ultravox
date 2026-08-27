#!/usr/bin/env bash
# Builds the open-source UltraVox Light macOS app and stages its public
# GitHub release archive.
set -euo pipefail

export PATH="${HOME}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:${PATH}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-$(node -p "require('${ROOT_DIR}/apps/desktop/package.json').version")}"
OUTPUT_DIR="${OUTPUT_DIR:-${ROOT_DIR}/release/light}"

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "UltraVox Light release packaging currently supports macOS arm64 only" >&2
  exit 1
fi
PLATFORM="macos-arm64"

PREFIX="UltraVox-Light-${PLATFORM}"

echo "==> Building UltraVox ${VERSION} Light (${PLATFORM})"
cd "${ROOT_DIR}/apps/desktop"
pnpm install --frozen-lockfile
pnpm tauri:build
cargo build --release --no-default-features --features cli,custom-protocol --bin ultravox-control

mkdir -p "${OUTPUT_DIR}"
rm -f "${OUTPUT_DIR}/${PREFIX}".*

STAGING="$(mktemp -d "${TMPDIR:-/tmp}/ultravox-light.XXXXXX")"
trap 'rm -rf "${STAGING}"' EXIT

write_sha() {
  # Portable SHA-256 manifest in `<hash>  <name>` format.
  if command -v shasum >/dev/null 2>&1; then
    ( cd "$(dirname "$1")" && shasum -a 256 "$(basename "$1")" )
  else
    ( cd "$(dirname "$1")" && sha256sum "$(basename "$1")" )
  fi > "$1.sha256"
}

APP="${ROOT_DIR}/target/release/bundle/macos/UltraVox Light.app"
CLI="${ROOT_DIR}/target/release/ultravox-control"
PAYLOAD="${STAGING}/UltraVox-Light-${PLATFORM}"
mkdir -p "${PAYLOAD}/bin"
ditto "${APP}" "${PAYLOAD}/UltraVox Light.app"
install -m 0755 "${CLI}" "${PAYLOAD}/bin/ultravox-control"
ditto -c -k --sequesterRsrc --keepParent "${PAYLOAD}" \
      "${OUTPUT_DIR}/${PREFIX}.zip"
write_sha "${OUTPUT_DIR}/${PREFIX}.zip"

printf 'Light release assets staged in %s\n' "${OUTPUT_DIR}"
