#!/usr/bin/env bash
set -euo pipefail

PRODUCT_NAME="UltraVox"
BUNDLE_ID="com.imploselabs.ultravox"
TEAM_ID="T63VT9UAY2"
EXPECTED_VERSION="${EXPECTED_VERSION:-}"
APP_PATH="${1:-}"

if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  echo "Usage: $0 /path/to/UltraVox.app" >&2
  exit 1
fi
if [[ "$(basename "$APP_PATH")" != "${PRODUCT_NAME}.app" ]]; then
  echo "Unexpected app name: $APP_PATH" >&2
  exit 1
fi

INFO_PLIST="$APP_PATH/Contents/Info.plist"
EXECUTABLE="$APP_PATH/Contents/MacOS/ultravox"
[[ -f "$INFO_PLIST" ]] || { echo "Missing Info.plist" >&2; exit 1; }
[[ -x "$EXECUTABLE" ]] || { echo "Missing executable: $EXECUTABLE" >&2; exit 1; }

IDENTIFIER="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$INFO_PLIST" 2>/dev/null || true)"
[[ "$IDENTIFIER" == "$BUNDLE_ID" ]] || { echo "Bundle identifier mismatch: $IDENTIFIER" >&2; exit 1; }
if [[ -n "$EXPECTED_VERSION" ]]; then
  VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$INFO_PLIST" 2>/dev/null || true)"
  [[ "$VERSION" == "$EXPECTED_VERSION" ]] || { echo "Version mismatch: $VERSION (expected $EXPECTED_VERSION)" >&2; exit 1; }
fi

CODESIGN_DETAILS="$(codesign -dv --verbose=4 "$APP_PATH" 2>&1)"
SIGNED_IDENTIFIER="$(printf '%s\n' "$CODESIGN_DETAILS" | sed -n 's/^Identifier=//p')"
[[ "$SIGNED_IDENTIFIER" == "$BUNDLE_ID" ]] || { echo "Signed identifier mismatch: $SIGNED_IDENTIFIER" >&2; exit 1; }
SIGNED_TEAM="$(printf '%s\n' "$CODESIGN_DETAILS" | sed -n 's/^TeamIdentifier=//p')"
if [[ "${REQUIRE_SIGNED:-0}" == "1" ]]; then
  [[ "$SIGNED_TEAM" == "$TEAM_ID" ]] || {
    echo "Signing team mismatch: ${SIGNED_TEAM:-<ad-hoc>} (expected $TEAM_ID)" >&2
    exit 1
  }
elif [[ "$SIGNED_TEAM" != "" && "$SIGNED_TEAM" != "$TEAM_ID" ]]; then
  echo "Unexpected signing team: $SIGNED_TEAM (expected $TEAM_ID or ad-hoc)." >&2
  exit 1
fi

# Deep strict verification is the sealed-code check. Production additionally
# requires a Developer ID Application chain and the hardened runtime.
codesign --verify --deep --strict --verbose=2 "$APP_PATH"
REQUIRE_SIGNED="${REQUIRE_SIGNED:-0}"
if [[ "$REQUIRE_SIGNED" == "1" ]]; then
  printf '%s\n' "$CODESIGN_DETAILS" | grep -Eq '^Authority=Developer ID Application: .* \(' || {
    echo "Production app must be Developer ID Application signed." >&2
    exit 1
  }
  printf '%s\n' "$CODESIGN_DETAILS" | grep -Eq '^Flags=.*runtime' || {
    echo "Production app must use the hardened runtime." >&2
    exit 1
  }
  DESIGNATED_REQUIREMENT="$(codesign -d -r- "$APP_PATH" 2>&1 || true)"
  printf '%s\n' "$DESIGNATED_REQUIREMENT" | grep -Fq "identifier \"$BUNDLE_ID\"" || {
    echo "Designated requirement does not bind to $BUNDLE_ID." >&2
    exit 1
  }
  printf '%s\n' "$DESIGNATED_REQUIREMENT" | grep -Fq "anchor apple generic" || {
    echo "Designated requirement is missing the Apple trust anchor." >&2
    exit 1
  }
  printf '%s\n' "$DESIGNATED_REQUIREMENT" | grep -Eq "certificate.*OU.*\"${TEAM_ID}\"" || {
    echo "Designated requirement is missing Developer Team $TEAM_ID." >&2
    exit 1
  }
fi

if [[ "${REQUIRE_NOTARIZED:-0}" == "1" ]]; then
  spctl --assess --type execute --verbose=2 "$APP_PATH"
  xcrun stapler validate "$APP_PATH"
fi

echo "Verified ${APP_PATH} (bundle=${BUNDLE_ID}, team=${SIGNED_TEAM:-ad-hoc})"