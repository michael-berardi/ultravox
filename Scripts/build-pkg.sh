#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRODUCT_NAME="UltraVox"
BUNDLE_ID="com.imploselabs.ultravox"
VERSION="${VERSION:-$(node -p "require('${ROOT_DIR}/apps/desktop/package.json').version")}"
ARCH="${PACKAGE_ARCH:-$(uname -m)}"
OUTPUT_DIR="${OUTPUT_DIR:-${ROOT_DIR}/release}"

PKG_PRODUCT="UltraVox"
PKG_PATH="${OUTPUT_DIR}/${PKG_PRODUCT}-${VERSION}-${ARCH}.pkg"
ENTITLEMENTS_PATH="${ROOT_DIR}/apps/desktop/src-tauri/Entitlements.plist"

if [[ "${GITHUB_REF_TYPE:-}" == "tag" ]]; then
  TAG_VERSION="${GITHUB_REF_NAME#v}"
  if [[ "$TAG_VERSION" != "$VERSION" ]]; then
    echo "Tag version ${TAG_VERSION} does not match package version ${VERSION}." >&2
    exit 1
  fi
fi

if [[ "${SKIP_BUILD:-0}" != "1" ]]; then
  BUILD_ARGS=(build --bundles app)
  if [[ -n "${TAURI_TARGET:-}" ]]; then
    BUILD_ARGS+=(--target "$TAURI_TARGET")
  fi
  (cd "$ROOT_DIR" && CI=false pnpm --filter ultravox-desktop tauri "${BUILD_ARGS[@]}")
fi

TARGET_SEGMENT="${TAURI_TARGET:+${TAURI_TARGET}/}release"
APP_CANDIDATES=(
  "${ROOT_DIR}/target/${TARGET_SEGMENT}/bundle/macos/${PRODUCT_NAME}.app"
  "${ROOT_DIR}/apps/desktop/src-tauri/target/${TARGET_SEGMENT}/bundle/macos/${PRODUCT_NAME}.app"
)
APP_PATH="${APP_PATH:-}"
if [[ -z "$APP_PATH" ]]; then
  for candidate in "${APP_CANDIDATES[@]}"; do
    if [[ -d "$candidate" ]]; then
      APP_PATH="$candidate"
      break
    fi
  done
fi
if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  printf 'Could not locate %s.app. Checked:\n' "$PRODUCT_NAME" >&2
  printf '  %s\n' "${APP_CANDIDATES[@]}" >&2
  exit 1
fi

if [[ ! -f "$ENTITLEMENTS_PATH" ]]; then
  echo "Missing UltraVox entitlements: $ENTITLEMENTS_PATH" >&2
  exit 1
fi

if [[ "${REQUIRE_SIGNED:-0}" == "1" && -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  echo "APPLE_SIGNING_IDENTITY is required for a distributable package." >&2
  exit 1
fi
if [[ "${SKIP_RESIGN:-0}" == "1" ]]; then
  # The caller already signed (and, for notarized releases, stapled) the app.
  # Re-signing here would rewrite the code signature and drop the stapled
  # notarization ticket, leaving .pkg-installed apps unable to auto-update.
  echo "Using existing app signature (SKIP_RESIGN=1)."
elif [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  codesign --force --deep --options runtime --timestamp \
    --entitlements "$ENTITLEMENTS_PATH" \
    --sign "$APPLE_SIGNING_IDENTITY" "$APP_PATH"
else
  if [[ "${REQUIRE_SIGNED:-0}" == "1" ]]; then
    echo "Refusing to produce an unsigned production package." >&2
    exit 1
  fi
  codesign --force --deep --entitlements "$ENTITLEMENTS_PATH" --sign - "$APP_PATH"
fi
EXPECTED_VERSION="$VERSION" REQUIRE_SIGNED="${REQUIRE_SIGNED:-0}" \
  ALLOW_ADHOC="${ALLOW_ADHOC:-0}" "${ROOT_DIR}/Scripts/verify-app.sh" "$APP_PATH"

PACKAGE_WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ultravox-pkg.XXXXXX")"
trap 'rm -rf "$PACKAGE_WORK_DIR"' EXIT
PACKAGE_SCRIPTS_DIR="${PACKAGE_WORK_DIR}/scripts"
mkdir -p "$PACKAGE_SCRIPTS_DIR"
cat > "${PACKAGE_SCRIPTS_DIR}/preinstall" <<'SCRIPT'
#!/bin/zsh
set -eu
for APP in "/Applications/UltraVox Light.app" "/Applications/UltraVox Pro.app"; do
  if [[ -e "$APP" || -L "$APP" ]]; then /bin/rm -rf -- "$APP"; fi
done
CONSOLE_USER="$(/usr/bin/stat -f '%Su' /dev/console)"
if [[ -n "$CONSOLE_USER" && "$CONSOLE_USER" != "root" && "$CONSOLE_USER" != "loginwindow" ]]; then
  USER_HOME="$(/usr/bin/dscl . -read "/Users/$CONSOLE_USER" NFSHomeDirectory | /usr/bin/sed 's/^NFSHomeDirectory: //')"
  for APP in "$USER_HOME/Applications/UltraVox.app" "$USER_HOME/Applications/UltraVox Light.app" "$USER_HOME/Applications/UltraVox Pro.app"; do
    if [[ -e "$APP" || -L "$APP" ]]; then /bin/rm -rf -- "$APP"; fi
  done
fi
SCRIPT
chmod 700 "${PACKAGE_SCRIPTS_DIR}/preinstall"


mkdir -p "$OUTPUT_DIR"
rm -f "$PKG_PATH"
PKGBUILD_ARGS=(
  --component "$APP_PATH"
  --install-location /Applications
  --identifier "$BUNDLE_ID"
  --version "$VERSION"
  --scripts "$PACKAGE_SCRIPTS_DIR"
)
if [[ "${REQUIRE_SIGNED:-0}" == "1" && -z "${APPLE_INSTALLER_SIGNING_IDENTITY:-}" ]]; then
  echo "APPLE_INSTALLER_SIGNING_IDENTITY is required for a distributable package." >&2
  exit 1
fi
if [[ -n "${APPLE_INSTALLER_SIGNING_IDENTITY:-}" ]]; then
  PKGBUILD_ARGS+=(--sign "$APPLE_INSTALLER_SIGNING_IDENTITY")
fi
pkgbuild "${PKGBUILD_ARGS[@]}" "$PKG_PATH"

if [[ "${NOTARIZE:-0}" == "1" ]]; then
  if [[ -n "${NOTARYTOOL_PROFILE:-}" ]]; then
    xcrun notarytool submit "$PKG_PATH" --keychain-profile "$NOTARYTOOL_PROFILE" --no-s3-acceleration --wait
  elif [[ -n "${APPLE_ID:-}" && -n "${APPLE_PASSWORD:-}" && -n "${APPLE_TEAM_ID:-}" ]]; then
    xcrun notarytool submit "$PKG_PATH" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
  else
    echo "Notarization requires NOTARYTOOL_PROFILE or APPLE_ID, APPLE_PASSWORD, and APPLE_TEAM_ID." >&2
    exit 1
  fi
  xcrun stapler staple "$PKG_PATH"
fi

REQUIRE_SIGNED="${REQUIRE_SIGNED:-0}" REQUIRE_NOTARIZED="${NOTARIZE:-0}" \
  "${ROOT_DIR}/Scripts/verify-pkg.sh" "$PKG_PATH"
shasum -a 256 "$PKG_PATH" > "${PKG_PATH}.sha256"
printf '%s\n' "$PKG_PATH"
