#!/bin/bash
set -euo pipefail

APP_NAME="Dictator"
APP_PATH="./target/release/bundle/macos/Dictator.app"
ZIP_PATH="./target/release/bundle/macos/Dictator.zip"
DMG_PATH="./Dictator.dmg"
KEYCHAIN_PROFILE="${NOTARYTOOL_PROFILE:-}"
CODE_SIGN_IDENTITY="${1:-}"

if [[ -z "${CODE_SIGN_IDENTITY}" ]]; then
  echo "Usage: ./notarize_app.sh 'Developer ID Application: …'" >&2
  exit 2
fi

if [[ -z "${KEYCHAIN_PROFILE}" ]]; then
  echo "Error: NOTARYTOOL_PROFILE environment variable is not set." >&2
  echo "Create a notarytool keychain profile first, e.g.:" >&2
  echo "  xcrun notarytool store-credentials <profile-name> --apple-id … --team-id …" >&2
  echo "  NOTARYTOOL_PROFILE=<profile-name> ./notarize_app.sh 'Developer ID Application: …'" >&2
  exit 2
fi

echo "Building canonical Tauri app..."
CI=false pnpm --filter dictator-desktop tauri build --bundles app

if [[ ! -d "${APP_PATH}" ]]; then
  echo "Tauri bundle not found: ${APP_PATH}" >&2
  exit 1
fi

echo "Signing ${APP_PATH}..."
codesign \
  --force \
  --deep \
  --options runtime \
  --timestamp \
  --sign "${CODE_SIGN_IDENTITY}" \
  "${APP_PATH}"

rm -f "${ZIP_PATH}" "${DMG_PATH}"
ditto -c -k --keepParent "${APP_PATH}" "${ZIP_PATH}"
xcrun notarytool submit "${ZIP_PATH}" --wait --keychain-profile "${KEYCHAIN_PROFILE}"
xcrun stapler staple "${APP_PATH}"

swifty-dmg --skipcodesign "${APP_PATH}" --output "${DMG_PATH}" --verbose
codesign --sign "${CODE_SIGN_IDENTITY}" "${DMG_PATH}"
xcrun notarytool submit "${DMG_PATH}" --wait --keychain-profile "${KEYCHAIN_PROFILE}"
xcrun stapler staple "${DMG_PATH}"

echo "Successfully notarized ${APP_NAME}"
