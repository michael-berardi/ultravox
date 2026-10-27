#!/usr/bin/env bash
set -euo pipefail
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:${PATH}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PRODUCT_NAME="UltraVox"
BUNDLE_ID="com.paretocybernetics.ultravox"
TEAM_ID="T63VT9UAY2"
PKG_PATH="${1:-}"
INSTALL_MODE="${2:-}"
VERIFY_APP="${ROOT_DIR}/Scripts/verify-app.sh"

if [[ -z "$PKG_PATH" || ! -f "$PKG_PATH" ]]; then
  echo "Usage: $0 /path/to/UltraVox.pkg [--install]" >&2
  exit 1
fi
if [[ "$INSTALL_MODE" == "--install-user" ]]; then
  echo "Per-user installation is disabled; UltraVox must be installed in /Applications." >&2
  exit 2
fi
if [[ -n "$INSTALL_MODE" && "$INSTALL_MODE" != "--install" ]]; then
  echo "Unknown install mode: $INSTALL_MODE" >&2
  exit 2
fi

EXPECTED_VERSION="${EXPECTED_VERSION:-}"
if [[ -f "${PKG_PATH}.sha256" ]]; then
  EXPECTED_HASH="$(awk '{print $1}' "${PKG_PATH}.sha256")"
  ACTUAL_HASH="$(shasum -a 256 "$PKG_PATH" | awk '{print $1}')"
  [[ "$EXPECTED_HASH" == "$ACTUAL_HASH" ]] || {
    echo "Checksum mismatch for ${PKG_PATH}." >&2
    exit 1
  }
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ultravox-pkg-verify.XXXXXX")"
PAYLOAD_LIST="${TMP_DIR}/payload-files.txt"
trap 'rm -rf "$TMP_DIR"' EXIT
pkgutil --expand-full "$PKG_PATH" "$TMP_DIR/expanded"
PACKAGE_INFO="$TMP_DIR/expanded/PackageInfo"
[[ -f "$PACKAGE_INFO" ]] || { echo "Expanded package is missing PackageInfo." >&2; exit 1; }

PKG_IDENTIFIER="$(/usr/bin/xmllint --xpath 'string(/pkg-info/@identifier)' "$PACKAGE_INFO")"
PKG_VERSION="$(/usr/bin/xmllint --xpath 'string(/pkg-info/@version)' "$PACKAGE_INFO")"
INSTALL_LOCATION="$(/usr/bin/xmllint --xpath 'string(/pkg-info/@install-location)' "$PACKAGE_INFO")"
[[ "$PKG_IDENTIFIER" == "$BUNDLE_ID" ]] || {
  echo "Package identifier mismatch: ${PKG_IDENTIFIER:-<missing>}" >&2
  exit 1
}
[[ "$INSTALL_LOCATION" == "/Applications" ]] || {
  echo "Package install location mismatch: ${INSTALL_LOCATION:-<missing>}" >&2
  exit 1
}
# Top-level relocatable=false alone is insufficient: a <relocate><bundle>
# entry still allows Installer to find/update a same-ID stray application.
RELOCATE_COUNT="$(/usr/bin/xmllint --xpath 'count(/pkg-info/relocate/bundle)' "$PACKAGE_INFO")"
RELOCATABLE="$(/usr/bin/xmllint --xpath 'string(/pkg-info/@relocatable)' "$PACKAGE_INFO")"
[[ "$RELOCATE_COUNT" == "0" && "$RELOCATABLE" == "false" ]] || {
  echo 'Package permits bundle relocation; canonical /Applications install is required.' >&2
  exit 1
}
if [[ -n "$EXPECTED_VERSION" && "$PKG_VERSION" != "$EXPECTED_VERSION" ]]; then
  echo "Package version mismatch: $PKG_VERSION (expected $EXPECTED_VERSION)." >&2
  exit 1
fi

pkgutil --payload-files "$PKG_PATH" > "$PAYLOAD_LIST"
# ._ payload entries are how a pkg carries extended attributes (com.apple.provenance cannot be
# removed); Installer restores them as attributes, not files. Modes are what users need.
BOM_FILE="$(find "$TMP_DIR/expanded" -maxdepth 2 -name Bom | head -1)"
if [[ -z "$BOM_FILE" ]]; then
  echo "Package has no Bom to check file modes." >&2
  exit 1
fi
UNREADABLE="$(lsbom -p Mf "$BOM_FILE" | awk '$2 ~ /^\.\/UltraVox\.app/ && ($1 ~ /^d/ && substr($1,8,3) != "r-x" || $1 ~ /^-/ && substr($1,8,1) != "r")' | head -5)"
if [[ -n "$UNREADABLE" ]]; then
  echo "Package payload is not readable by every user (folders must be 0755, files 0644/0755):" >&2
  echo "$UNREADABLE" >&2
  exit 1
fi
grep -Eq '^\./UltraVox\.app/Contents/MacOS/ultravox$' "$PAYLOAD_LIST" || {
  echo "Package payload does not contain /Applications/UltraVox.app." >&2
  exit 1
}

if [[ "${REQUIRE_SIGNED:-0}" == "1" ]]; then
  pkgutil --check-signature "$PKG_PATH" | grep -Eq "Developer ID Installer: .*\\(${TEAM_ID}\\)" || {
    echo "Production package must be signed by Developer ID Installer team ${TEAM_ID}." >&2
    exit 1
  }
  spctl --assess --type install --verbose=2 "$PKG_PATH"
fi
if [[ "${REQUIRE_NOTARIZED:-0}" == "1" ]]; then
  xcrun stapler validate "$PKG_PATH"
fi

EXPANDED_APP="$TMP_DIR/expanded/Payload/${PRODUCT_NAME}.app"
[[ -d "$EXPANDED_APP" ]] || {
  echo "Expanded package is missing the UltraVox.app payload." >&2
  exit 1
}
EXPECTED_VERSION="$EXPECTED_VERSION" REQUIRE_SIGNED="${REQUIRE_SIGNED:-0}" \
  REQUIRE_NOTARIZED=0 ALLOW_ADHOC="${ALLOW_ADHOC:-0}" "$VERIFY_APP" "$EXPANDED_APP"

if [[ "$INSTALL_MODE" == "--install" ]]; then
  sudo installer -pkg "$PKG_PATH" -target /
  INSTALLED_APP="/Applications/${PRODUCT_NAME}.app"
  # The signed installer ticket is the package trust root. pkgbuild may not
  # preserve an app's stapled ticket in the installed payload; VERIFY_APP still
  # requires Gatekeeper acceptance and the stable Developer ID identity.
  EXPECTED_VERSION="$EXPECTED_VERSION" REQUIRE_SIGNED="${REQUIRE_SIGNED:-0}" \
    REQUIRE_NOTARIZED=0 ALLOW_ADHOC="${ALLOW_ADHOC:-0}" \
    "$VERIFY_APP" "$INSTALLED_APP"
fi

echo "Verified ${PKG_PATH} (identifier=${BUNDLE_ID}, install=/Applications)"
