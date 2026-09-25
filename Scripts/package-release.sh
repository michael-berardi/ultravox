#!/usr/bin/env bash
# The single UltraVox packager. Builds the official build (closed Pro module
# linked in; Pro features stay locked without a valid entitlement) and stages
# the release assets for the current platform as the UltraVox-<platform>
# family, plus compatibility copies for pre-unification updater generations.
# Run natively on each supported host:
#   macOS arm64  : ./Scripts/package-release.sh
#   linux x86_64 : ./Scripts/package-release.sh   (requires webkit2gtk)
#   Windows      : run inside Git Bash/MSYS2; NSIS comes from tauri.conf
set -euo pipefail

export PATH="${HOME}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:${PATH}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRODUCT="UltraVox"
VERSION="${VERSION:-$(node -p "require('${ROOT_DIR}/apps/desktop/package.json').version")}"
ARCH="${PACKAGE_ARCH:-$(uname -m)}"
OUTPUT_DIR="${OUTPUT_DIR:-${ROOT_DIR}/release}"

write_sha() {
  # Portable SHA-256 manifest in `<hash>  <name>` format.
  if command -v shasum >/dev/null 2>&1; then
    ( cd "$(dirname "$1")" && shasum -a 256 "$(basename "$1")" )
  else
    ( cd "$(dirname "$1")" && sha256sum "$(basename "$1")" )
  fi > "$1.sha256"
}

copy_compat() {
  # Pre-unification updater generations resolve the historical
  # UltraVox-Light-<platform> and UltraVox-Pro-<platform> asset names.
  local src="$1" base tail family
  base="${src##*/}"
  tail="${base#UltraVox}"
  for family in UltraVox-Light UltraVox-Pro; do
    cp -f "$src" "${OUTPUT_DIR}/${family}${tail}"
    write_sha "${OUTPUT_DIR}/${family}${tail}"
  done
}

PLATFORM="macos-${ARCH}"
PAYLOAD_NAME="UltraVox-${PLATFORM}"
ARCHIVE_NAME="${PAYLOAD_NAME}.zip"
APP_PATH="${APP_PATH:-${ROOT_DIR}/target/release/bundle/macos/${PRODUCT}.app}"
CLI_PATH="${CLI_PATH:-${ROOT_DIR}/target/release/ultravox-control}"
BUILD_QUEUE="${BUILD_QUEUE:-}"
ALLOW_ADHOC="${ALLOW_ADHOC:-0}"
ADAPTER_SOURCE="${ROOT_DIR}/third_party/mediaremote-adapter"
ADAPTER_BUILD="${ROOT_DIR}/target/release/mediaremote-adapter-build"
ADAPTER_STAGE="${ROOT_DIR}/target/release/mediaremote"

if [[ "$(uname -s)" != "Darwin" ]]; then
  case "$(uname -s)" in
    Linux)
      [[ "$(uname -m)" == "x86_64" ]] || {
        echo "UltraVox currently supports x86_64 Linux." >&2
        exit 1
      }
      PLATFORM="linux-x86_64"
      ;;
    MINGW*|MSYS*|CYGWIN*)
      [[ "$(uname -m)" == "x86_64" ]] || {
        echo "UltraVox currently supports x86_64 Windows." >&2
        exit 1
      }
      PLATFORM="windows-x86_64"
      ;;
    *)
      echo "Unsupported release host: $(uname -s)" >&2
      exit 1
      ;;
  esac
  PREFIX="UltraVox-${PLATFORM}"
  echo "==> Building UltraVox ${VERSION} (${PLATFORM})"
  cd "${ROOT_DIR}/apps/desktop"
  pnpm install --frozen-lockfile
  pnpm exec tauri build -- --no-default-features --features custom-protocol,ultravox-pro
  cargo build --release --no-default-features \
    --features cli,custom-protocol,ultravox-pro --bin ultravox-control
  mkdir -p "$OUTPUT_DIR"
  rm -f "${OUTPUT_DIR}/${PREFIX}".* "${OUTPUT_DIR}/UltraVox-Light-${PLATFORM}".* \
    "${OUTPUT_DIR}/UltraVox-Pro-${PLATFORM}".*
  shopt -s nullglob
  if [[ "$PLATFORM" == "linux-x86_64" ]]; then
    APPIMAGES=("${ROOT_DIR}"/target/release/bundle/appimage/*.AppImage)
    [[ "${#APPIMAGES[@]}" == "1" ]] || {
      echo "Expected exactly one AppImage build output." >&2
      exit 1
    }
    cp -f "${APPIMAGES[0]}" "${OUTPUT_DIR}/${PREFIX}.AppImage"
    chmod 0755 "${OUTPUT_DIR}/${PREFIX}.AppImage"
    ( cd "$OUTPUT_DIR" && sha256sum "${PREFIX}.AppImage" > "${PREFIX}.AppImage.sha256" )
    copy_compat "${OUTPUT_DIR}/${PREFIX}.AppImage"
  else
    INSTALLERS=("${ROOT_DIR}"/target/release/bundle/nsis/*-setup.exe)
    [[ "${#INSTALLERS[@]}" == "1" ]] || {
      echo "Expected exactly one NSIS setup executable." >&2
      exit 1
    }
    cp -f "${INSTALLERS[0]}" "${OUTPUT_DIR}/${PREFIX}-setup.exe"
    ( cd "$OUTPUT_DIR" && sha256sum "${PREFIX}-setup.exe" > "${PREFIX}-setup.exe.sha256" )
    copy_compat "${OUTPUT_DIR}/${PREFIX}-setup.exe"
  fi
  printf 'Release assets staged in %s\n' "$OUTPUT_DIR"
  exit 0
fi

# PKG_SIGNING=app-only stages the Developer-ID zip without the installer
# package, installer identity, or notarization (owner-directed local builds).
PKG_SIGNING="${PKG_SIGNING:-full}"
if [[ "$ALLOW_ADHOC" != "1" ]]; then
  : "${APPLE_SIGNING_IDENTITY:?APPLE_SIGNING_IDENTITY is required}"
  if [[ "$PKG_SIGNING" == "full" ]]; then
    : "${APPLE_INSTALLER_SIGNING_IDENTITY:?APPLE_INSTALLER_SIGNING_IDENTITY is required}"
    : "${NOTARYTOOL_PROFILE:?NOTARYTOOL_PROFILE is required}"
  fi
fi

if [[ "$(uname -s)" != "Darwin" || "$ARCH" != "arm64" ]]; then
  echo "Release packaging currently requires Apple Silicon macOS." >&2
  exit 1
fi

SIGNED_RELEASE=0
if [[ "$ALLOW_ADHOC" != "1" ]]; then
  SIGNED_RELEASE=1
fi

run_heavy() {
  if [[ -f "$BUILD_QUEUE" ]]; then
    (cd "$ROOT_DIR" && python3 "$BUILD_QUEUE" --project "$ROOT_DIR" -- "$@")
  else
    (cd "$ROOT_DIR" && "$@")
  fi
}

# The updater compares the git tag, the release metadata, and the built app's
# CFBundleShortVersionString, and rejects any mismatch. Every version source
# must agree before building or the published update fails to install.
node - "$ROOT_DIR" "$VERSION" <<'NODE'
const fs = require("fs");
const path = require("path");
const [root, expected] = process.argv.slice(2);
const read = (rel) => path.join(root, rel);
const pkgRoot = JSON.parse(fs.readFileSync(read("package.json"), "utf8")).version;
const pkgDesktop = JSON.parse(fs.readFileSync(read("apps/desktop/package.json"), "utf8")).version;
const tauri = JSON.parse(fs.readFileSync(read("apps/desktop/src-tauri/tauri.conf.json"), "utf8")).version;
const cargo = fs.readFileSync(read("apps/desktop/src-tauri/Cargo.toml"), "utf8")
  .match(/^\[package\]\nname = "ultravox"\nversion = "([^"]+)"/m)?.[1];
const versions = { "package.json": pkgRoot, "apps/desktop/package.json": pkgDesktop, "tauri.conf.json": tauri, "Cargo.toml": cargo };
const mismatched = Object.entries(versions).filter(([, v]) => v !== expected);
if (mismatched.length > 0) {
  console.error(`Version drift: expected ${expected} everywhere, but ${mismatched.map(([f, v]) => `${f}=${v}`).join(", ")}.`);
  process.exit(1);
}
NODE

if [[ "${SKIP_BUILD:-0}" != "1" ]]; then
  run_heavy env CI=false pnpm --filter ultravox-desktop tauri build --bundles app -- \
    --no-default-features --features custom-protocol,ultravox-pro
  run_heavy cargo build --release -p ultravox --no-default-features \
    --features cli,custom-protocol,ultravox-pro --bin ultravox-control
  run_heavy cmake -S "$ADAPTER_SOURCE" -B "$ADAPTER_BUILD" -DCMAKE_BUILD_TYPE=Release
  run_heavy cmake --build "$ADAPTER_BUILD" --target MediaRemoteAdapter
  rm -rf "$ADAPTER_STAGE"
  mkdir -p "$ADAPTER_STAGE"
  ditto "$ADAPTER_BUILD/MediaRemoteAdapter.framework" \
    "$ADAPTER_STAGE/MediaRemoteAdapter.framework"
  install -m 0755 "$ADAPTER_SOURCE/bin/mediaremote-adapter.pl" \
    "$ADAPTER_STAGE/mediaremote-adapter.pl"
  install -m 0644 "$ADAPTER_SOURCE/LICENSE" "$ADAPTER_STAGE/LICENSE"
fi
if [[ ! -d "$APP_PATH" || ! -x "$CLI_PATH" ]]; then
  echo "Missing UltraVox.app or ultravox-control release binary." >&2
  exit 1
fi
if [[ ! -d "$ADAPTER_STAGE/MediaRemoteAdapter.framework" \
  || ! -x "$ADAPTER_STAGE/mediaremote-adapter.pl" ]]; then
  echo "Missing bundled MediaRemote metadata adapter." >&2
  exit 1
fi

AUDIO_INPUT_ENTITLEMENT="$(
  codesign -d --entitlements - --xml "$APP_PATH" 2>/dev/null \
    | plutil -extract 'com\.apple\.security\.device\.audio-input' raw - 2>/dev/null \
    || true
)"
if [[ "$AUDIO_INPUT_ENTITLEMENT" != "true" ]]; then
  echo "UltraVox.app is missing the required audio-input entitlement." >&2
  exit 1
fi
EXPECTED_VERSION="$VERSION" REQUIRE_SIGNED=0 ALLOW_ADHOC=1 \
  "${ROOT_DIR}/Scripts/verify-app.sh" "$APP_PATH"
APP_ADAPTER_DIR="${APP_PATH}/Contents/Resources/mediaremote"
rm -rf "$APP_ADAPTER_DIR"
mkdir -p "$APP_ADAPTER_DIR"
ditto "$ADAPTER_STAGE/." "$APP_ADAPTER_DIR/"
APP_ADAPTER_FRAMEWORK="${APP_ADAPTER_DIR}/MediaRemoteAdapter.framework"


if [[ "$SIGNED_RELEASE" == "1" ]]; then
  # Ad-hoc local builds sign without the Apple timestamp service; real
  # identities always use the default secure timestamp (required for notarization).
  local_ts=""; [[ "$ALLOW_ADHOC" == "1" ]] && local_ts="--timestamp=none"
  # Sign the app once, up front. Every artifact (zip and pkg payload) must be
  # produced from this exact signed bundle; re-signing after stapling would
  # drop the notarization ticket.
  codesign --force --options runtime $local_ts \
    --sign "$APPLE_SIGNING_IDENTITY" "$APP_ADAPTER_FRAMEWORK"
  codesign --force --deep --options runtime $local_ts \
    --entitlements "${ROOT_DIR}/apps/desktop/src-tauri/Entitlements.plist" \
    --sign "$APPLE_SIGNING_IDENTITY" "$APP_PATH"
  codesign --force --options runtime $local_ts --sign "$APPLE_SIGNING_IDENTITY" "$CLI_PATH"
else
  codesign --force --sign - "$APP_ADAPTER_FRAMEWORK"
  codesign --force --deep \
    --entitlements "${ROOT_DIR}/apps/desktop/src-tauri/Entitlements.plist" \
    --sign - "$APP_PATH"
  codesign --force --sign - "$CLI_PATH"
fi
codesign --verify --strict "$CLI_PATH"
codesign --verify --strict "$APP_ADAPTER_FRAMEWORK"
if [[ "$SIGNED_RELEASE" == "1" ]]; then
  codesign -dv --verbose=4 "$APP_ADAPTER_FRAMEWORK" 2>&1 \
    | grep -E '^TeamIdentifier=T63VT9UAY2$' >/dev/null || {
      echo "MediaRemoteAdapter.framework must be signed by Developer ID team T63VT9UAY2." >&2
      exit 1
    }
fi
POST_SIGN_AUDIO_INPUT="$(
  codesign -d --entitlements - --xml "$APP_PATH" 2>/dev/null \
    | plutil -extract 'com\.apple\.security\.device\.audio-input' raw - 2>/dev/null \
    || true
)"
if [[ "$POST_SIGN_AUDIO_INPUT" != "true" ]]; then
  echo "Signed UltraVox.app lost its required audio-input entitlement." >&2
  exit 1
fi
EXPECTED_VERSION="$VERSION" REQUIRE_SIGNED="$SIGNED_RELEASE" ALLOW_ADHOC="$ALLOW_ADHOC" \
  "${ROOT_DIR}/Scripts/verify-app.sh" "$APP_PATH"

VERSIONED_PKG="${OUTPUT_DIR}/UltraVox-${VERSION}-${ARCH}.pkg"
STABLE_PKG="${OUTPUT_DIR}/UltraVox-${PLATFORM}.pkg"
ARCHIVE_PATH="${OUTPUT_DIR}/${ARCHIVE_NAME}"
mkdir -p "$OUTPUT_DIR"
rm -f "$VERSIONED_PKG" "${VERSIONED_PKG}.sha256" \
  "$STABLE_PKG" "${STABLE_PKG}.sha256" "$ARCHIVE_PATH" "${ARCHIVE_PATH}.sha256" \
  "${OUTPUT_DIR}/UltraVox-Light-${PLATFORM}.pkg" "${OUTPUT_DIR}/UltraVox-Light-${PLATFORM}.pkg.sha256" \
  "${OUTPUT_DIR}/UltraVox-Light-${PLATFORM}.zip" "${OUTPUT_DIR}/UltraVox-Light-${PLATFORM}.zip.sha256" \
  "${OUTPUT_DIR}/UltraVox-Pro-${PLATFORM}.pkg" "${OUTPUT_DIR}/UltraVox-Pro-${PLATFORM}.pkg.sha256" \
  "${OUTPUT_DIR}/UltraVox-Pro-${PLATFORM}.zip" "${OUTPUT_DIR}/UltraVox-Pro-${PLATFORM}.zip.sha256"

STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ultravox-release.XXXXXX")"
trap 'rm -rf "$STAGING_DIR"' EXIT
PAYLOAD_DIR="${STAGING_DIR}/${PAYLOAD_NAME}"
mkdir -p "${PAYLOAD_DIR}/bin"
ditto "$APP_PATH" "${PAYLOAD_DIR}/${PRODUCT}.app"
install -m 0755 "$CLI_PATH" "${PAYLOAD_DIR}/bin/ultravox-control"

ditto -c -k --sequesterRsrc --keepParent "$PAYLOAD_DIR" "$ARCHIVE_PATH"
if [[ "$SIGNED_RELEASE" == "1" && "$PKG_SIGNING" == "full" ]]; then
  # Notarize the app (via the zip submission), then staple the ticket into the
  # canonical bundle. The pkg below is built from this stapled bundle so
  # .pkg-installed apps carry the ticket and can pass the updater's
  # notarization checks — unstapled installs were permanently unable to
  # auto-update.
  xcrun notarytool submit "$ARCHIVE_PATH" --keychain-profile "$NOTARYTOOL_PROFILE" --no-s3-acceleration --wait
  xcrun stapler staple "$APP_PATH"

  rm -rf "${PAYLOAD_DIR}/${PRODUCT}.app"
  ditto "$APP_PATH" "${PAYLOAD_DIR}/${PRODUCT}.app"
  rm -f "$ARCHIVE_PATH"
  ditto -c -k --sequesterRsrc --keepParent "$PAYLOAD_DIR" "$ARCHIVE_PATH"
fi
if [[ "$SIGNED_RELEASE" == "1" && "$PKG_SIGNING" == "full" ]]; then
  SKIP_BUILD=1 SKIP_RESIGN=1 OUTPUT_DIR="$OUTPUT_DIR" APP_PATH="$APP_PATH" EXPECTED_VERSION="$VERSION" \
    REQUIRE_SIGNED=1 NOTARIZE=1 ALLOW_ADHOC=0 \
    APPLE_SIGNING_IDENTITY="$APPLE_SIGNING_IDENTITY" \
    APPLE_INSTALLER_SIGNING_IDENTITY="$APPLE_INSTALLER_SIGNING_IDENTITY" \
    NOTARYTOOL_PROFILE="$NOTARYTOOL_PROFILE" \
    "${ROOT_DIR}/Scripts/build-pkg.sh"
  EXPECTED_VERSION="$VERSION" REQUIRE_SIGNED=1 REQUIRE_NOTARIZED=1 \
    ALLOW_ADHOC=0 "${ROOT_DIR}/Scripts/verify-pkg.sh" "$VERSIONED_PKG"
  mv "$VERSIONED_PKG" "$STABLE_PKG"
elif [[ "$SIGNED_RELEASE" == "1" ]]; then
  printf 'PKG_SIGNING=app-only: skipping installer package stage\n'
else
  SKIP_BUILD=1 OUTPUT_DIR="$OUTPUT_DIR" APP_PATH="$APP_PATH" EXPECTED_VERSION="$VERSION" \
    ALLOW_ADHOC=1 "${ROOT_DIR}/Scripts/build-pkg.sh"
  EXPECTED_VERSION="$VERSION" REQUIRE_SIGNED=0 REQUIRE_NOTARIZED=0 \
    ALLOW_ADHOC=1 "${ROOT_DIR}/Scripts/verify-pkg.sh" "$VERSIONED_PKG"
  mv "$VERSIONED_PKG" "$STABLE_PKG"
fi

rm -f "${VERSIONED_PKG}.sha256"

codesign --verify --strict "$CLI_PATH"
if [[ "$SIGNED_RELEASE" == "1" ]]; then
  codesign -dv --verbose=4 "$CLI_PATH" 2>&1 | grep -E '^TeamIdentifier=T63VT9UAY2$' >/dev/null || {
    echo "Production CLI must be signed by Developer ID team T63VT9UAY2." >&2
    exit 1
  }
fi
ARCHIVE_NOTARIZED=0
if [[ "$SIGNED_RELEASE" == "1" && "$PKG_SIGNING" == "full" ]]; then
  ARCHIVE_NOTARIZED=1
fi
ARCHIVE_CHECK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ultravox-archive-check.XXXXXX")"
trap 'rm -rf "$STAGING_DIR" "$ARCHIVE_CHECK_DIR"' EXIT
ditto -x -k "$ARCHIVE_PATH" "$ARCHIVE_CHECK_DIR"
EXPECTED_VERSION="$VERSION" REQUIRE_SIGNED="$SIGNED_RELEASE" \
  REQUIRE_NOTARIZED="$ARCHIVE_NOTARIZED" ALLOW_ADHOC="$ALLOW_ADHOC" \
  "${ROOT_DIR}/Scripts/verify-app.sh" "$ARCHIVE_CHECK_DIR/${PAYLOAD_NAME}/${PRODUCT}.app"

(
  cd "$OUTPUT_DIR"
  [[ ! -f "$STABLE_PKG" ]] || shasum -a 256 "$(basename "$STABLE_PKG")" > "$(basename "$STABLE_PKG").sha256"
  shasum -a 256 "$ARCHIVE_NAME" > "${ARCHIVE_NAME}.sha256"
)

# Compatibility copies: updater generations older than the single-product
# unification resolve the historical UltraVox-Light-<platform> and
# UltraVox-Pro-<platform> asset names. The legacy UltraVox-macos-arm64 names
# are the canonical family name on Apple Silicon and need no extra copies.
copy_compat "$ARCHIVE_PATH"
[[ ! -f "$STABLE_PKG" ]] || copy_compat "$STABLE_PKG"
printf 'Release assets staged in %s\n' "$OUTPUT_DIR"
