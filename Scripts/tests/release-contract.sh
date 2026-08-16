#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERIFY_APP="$ROOT_DIR/Scripts/verify-app.sh"
VERIFY_PKG="$ROOT_DIR/Scripts/verify-pkg.sh"
BUILD_PKG="$ROOT_DIR/Scripts/build-pkg.sh"
PACKAGE_RELEASE="$ROOT_DIR/Scripts/package-release.sh"

assert_contains() {
  local file="$1" needle="$2"
  grep -Fq -- "$needle" "$file" || {
    echo "release contract missing '$needle' in $file" >&2
    exit 1
  }
}

assert_not_contains() {
  local file="$1" needle="$2"
  if grep -Fq -- "$needle" "$file"; then
    echo "release contract must not contain '$needle' in $file" >&2
    exit 1
  fi
}

for file in "$VERIFY_APP" "$VERIFY_PKG" "$BUILD_PKG" "$PACKAGE_RELEASE"; do
  test -x "$file"
done
for file in "$VERIFY_APP" "$VERIFY_PKG" "$BUILD_PKG"; do
  assert_contains "$file" 'com.imploselabs.ultravox'
done
assert_contains "$PACKAGE_RELEASE" 'verify-app.sh'
for file in "$VERIFY_APP" "$VERIFY_PKG" "$PACKAGE_RELEASE"; do
  assert_contains "$file" 'T63VT9UAY2'
done
assert_contains "$BUILD_PKG" 'verify-app.sh'
assert_contains "$VERIFY_APP" 'codesign --verify --deep --strict'
assert_contains "$VERIFY_APP" 'Designated requirement does not bind'
assert_contains "$VERIFY_PKG" 'install=/Applications'
assert_contains "$VERIFY_PKG" 'Per-user installation is disabled'
assert_contains "$VERIFY_PKG" 'PackageInfo'
assert_contains "$VERIFY_PKG" 'xcrun stapler validate'
assert_contains "$BUILD_PKG" '--install-location /Applications'
assert_contains "$BUILD_PKG" 'Refusing to produce an unsigned production package'
assert_not_contains "$BUILD_PKG" '--requirements'
assert_contains "$PACKAGE_RELEASE" 'ALLOW_ADHOC=1 only for local installer testing'
assert_contains "$PACKAGE_RELEASE" 'ARCHIVE_NAME="${PAYLOAD_NAME}.zip"'
assert_contains "$PACKAGE_RELEASE" 'verify-app.sh'
assert_contains "$ROOT_DIR/apps/desktop/src-tauri/src/update.rs" '.UltraVox.previous'
assert_contains "$ROOT_DIR/apps/desktop/src-tauri/src/update.rs" 'Update designated requirement does not match UltraVox'
assert_contains "$ROOT_DIR/apps/desktop/src-tauri/src/update.rs" "verify the update's notarization"

# The public metadata must retain one product identity and release version.
node - <<'NODE' "$ROOT_DIR"
const fs = require('node:fs');
const path = require('node:path');
const root = process.argv[2];
const packageJson = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'));
const desktopJson = JSON.parse(fs.readFileSync(path.join(root, 'apps/desktop/package.json'), 'utf8'));
const tauri = JSON.parse(fs.readFileSync(path.join(root, 'apps/desktop/src-tauri/tauri.conf.json'), 'utf8'));
if (packageJson.version !== '0.2.2' || desktopJson.version !== '0.2.2') throw new Error('version metadata is not 0.2.2');
if (tauri.productName !== 'UltraVox' || tauri.identifier !== 'com.imploselabs.ultravox') throw new Error('Tauri identity drift');
NODE

echo 'release contract passed'
