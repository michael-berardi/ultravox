#!/usr/bin/env bash
# Build UltraVox Light on the current native host and stage one immutable
# GitHub release artifact plus its SHA-256 manifest.
set -euo pipefail

export PATH="${HOME}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:${PATH}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-$(node -p "require('${ROOT_DIR}/apps/desktop/package.json').version")}"
OUTPUT_DIR="${OUTPUT_DIR:-${ROOT_DIR}/release}"
UNAME="$(uname -s)"

case "$UNAME" in
  Darwin)
    [[ "$(uname -m)" == "arm64" ]] || {
      echo "UltraVox Light currently supports Apple silicon Macs." >&2
      exit 1
    }
    PLATFORM="macos-arm64"
    ;;
  Linux)
    [[ "$(uname -m)" == "x86_64" ]] || {
      echo "UltraVox Light currently supports x86_64 Linux." >&2
      exit 1
    }
    PLATFORM="linux-x86_64"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    [[ "$(uname -m)" == "x86_64" ]] || {
      echo "UltraVox Light currently supports x86_64 Windows." >&2
      exit 1
    }
    PLATFORM="windows-x86_64"
    ;;
  *)
    echo "Unsupported release host: $UNAME" >&2
    exit 1
    ;;
esac

node - "$ROOT_DIR" "$VERSION" <<'NODE'
const fs = require("fs");
const [root, expected] = process.argv.slice(2);
const packageVersion = JSON.parse(fs.readFileSync(`${root}/package.json`)).version;
const desktopVersion = JSON.parse(fs.readFileSync(`${root}/apps/desktop/package.json`)).version;
const tauriVersion = JSON.parse(fs.readFileSync(`${root}/apps/desktop/src-tauri/tauri.conf.json`)).version;
const readCargoVersion = path => fs.readFileSync(path, "utf8").match(/^version = "([^"]+)"/m)?.[1];
const cargoVersion = readCargoVersion(`${root}/apps/desktop/src-tauri/Cargo.toml`);
const coreVersion = readCargoVersion(`${root}/crates/ultravox-core/Cargo.toml`);
const bridgeVersion = readCargoVersion(`${root}/crates/ultravox-macos-bridge/Cargo.toml`);
for (const [source, version] of Object.entries({ packageVersion, desktopVersion, tauriVersion, cargoVersion, coreVersion, bridgeVersion })) {
  if (version !== expected) throw new Error(`${source} is ${version}; expected ${expected}`);
}
NODE

PREFIX="UltraVox-Light-${PLATFORM}"
echo "==> Building UltraVox Light ${VERSION} (${PLATFORM})"
cd "${ROOT_DIR}/apps/desktop"
pnpm install --frozen-lockfile
pnpm exec tauri build -- --no-default-features --features custom-protocol
cargo build --release --no-default-features --features cli,custom-protocol --bin ultravox-control

mkdir -p "${OUTPUT_DIR}"
rm -f "${OUTPUT_DIR}/${PREFIX}".*

STAGING="$(mktemp -d "${TMPDIR:-/tmp}/ultravox-light.XXXXXX")"
trap 'rm -rf "${STAGING}"' EXIT

write_sha() {
  if command -v shasum >/dev/null 2>&1; then
    ( cd "$(dirname "$1")" && shasum -a 256 "$(basename "$1")" )
  else
    ( cd "$(dirname "$1")" && sha256sum "$(basename "$1")" )
  fi > "$1.sha256"
}

case "$PLATFORM" in
  macos-arm64)
    : "${APPLE_SIGNING_IDENTITY:?Set APPLE_SIGNING_IDENTITY to a Developer ID Application identity.}"
    : "${NOTARY_PROFILE:?Set NOTARY_PROFILE to a notarytool keychain profile.}"
    APP="${ROOT_DIR}/target/release/bundle/macos/UltraVox.app"
    CLI="${ROOT_DIR}/target/release/ultravox-control"
    INFO="${APP}/Contents/Info.plist"
    [[ -d "$APP" && -x "$CLI" ]] || {
      echo "The macOS app or CLI build output is missing." >&2
      exit 1
    }
    [[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$INFO")" == "com.imploselabs.ultravox" ]]
    [[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$INFO")" == "$VERSION" ]]
    /usr/bin/codesign --force --options runtime --timestamp \
      --sign "$APPLE_SIGNING_IDENTITY" "$CLI"
    /usr/bin/codesign --force --deep --options runtime --timestamp \
      --entitlements "${ROOT_DIR}/apps/desktop/src-tauri/Entitlements.plist" \
      --sign "$APPLE_SIGNING_IDENTITY" "$APP"
    /usr/bin/codesign --verify --deep --strict "$APP"
    /usr/bin/codesign --verify --strict "$CLI"
    /usr/bin/ditto -c -k --sequesterRsrc --keepParent "$APP" "${STAGING}/notary.zip"
    /usr/bin/xcrun notarytool submit "${STAGING}/notary.zip" \
      --keychain-profile "$NOTARY_PROFILE" --wait
    /usr/bin/xcrun stapler staple "$APP"
    /usr/bin/xcrun stapler validate "$APP"
    /usr/sbin/spctl --assess --type execute "$APP"
    PAYLOAD="${STAGING}/${PREFIX}"
    mkdir -p "${PAYLOAD}/bin"
    /usr/bin/ditto "$APP" "${PAYLOAD}/UltraVox.app"
    /usr/bin/install -m 0755 "$CLI" "${PAYLOAD}/bin/ultravox-control"
    /usr/bin/ditto -c -k --sequesterRsrc --keepParent "$PAYLOAD" \
      "${OUTPUT_DIR}/${PREFIX}.zip"
    write_sha "${OUTPUT_DIR}/${PREFIX}.zip"
    ;;
  linux-x86_64)
    shopt -s nullglob
    APPIMAGES=("${ROOT_DIR}"/target/release/bundle/appimage/*.AppImage)
    [[ "${#APPIMAGES[@]}" == "1" ]] || {
      echo "Expected exactly one AppImage build output." >&2
      exit 1
    }
    cp -f "${APPIMAGES[0]}" "${OUTPUT_DIR}/${PREFIX}.AppImage"
    chmod 0755 "${OUTPUT_DIR}/${PREFIX}.AppImage"
    write_sha "${OUTPUT_DIR}/${PREFIX}.AppImage"
    ;;
  windows-x86_64)
    shopt -s nullglob
    INSTALLERS=("${ROOT_DIR}"/target/release/bundle/nsis/*-setup.exe)
    [[ "${#INSTALLERS[@]}" == "1" ]] || {
      echo "Expected exactly one NSIS setup executable." >&2
      exit 1
    }
    cp -f "${INSTALLERS[0]}" "${OUTPUT_DIR}/${PREFIX}-setup.exe"
    write_sha "${OUTPUT_DIR}/${PREFIX}-setup.exe"
    ;;
esac

printf 'Release asset staged in %s\n' "${OUTPUT_DIR}"
