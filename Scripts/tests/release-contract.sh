#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERIFY_APP="$ROOT_DIR/Scripts/verify-app.sh"
VERIFY_PKG="$ROOT_DIR/Scripts/verify-pkg.sh"
BUILD_PKG="$ROOT_DIR/Scripts/build-pkg.sh"
PACKAGE_RELEASE="$ROOT_DIR/Scripts/package-release.sh"
PACKAGE_ALL="$ROOT_DIR/Scripts/package-all.sh"
PUBLISH_RELEASE="$ROOT_DIR/Scripts/publish-release.sh"
EXPORT_OSS="$ROOT_DIR/Scripts/export-open-source.sh"
EXPORT_MANIFEST="$ROOT_DIR/Scripts/open-source-exclude.txt"

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

for file in "$VERIFY_APP" "$VERIFY_PKG" "$BUILD_PKG" "$PACKAGE_RELEASE" "$PACKAGE_ALL" "$PUBLISH_RELEASE" "$EXPORT_OSS"; do
  test -x "$file"
done
test -f "$EXPORT_MANIFEST"
# Exactly one packager: the old standalone open-source packager is gone.
for script in "$ROOT_DIR"/Scripts/package-*.sh; do
  if [[ "$script" != "$PACKAGE_RELEASE" && "$script" != "$PACKAGE_ALL" ]]; then
    echo "unexpected packager present: $script" >&2
    exit 1
  fi
done
for file in "$VERIFY_APP" "$VERIFY_PKG" "$BUILD_PKG"; do
  assert_contains "$file" 'com.paretocybernetics.ultravox'
done
assert_contains "$ROOT_DIR/apps/desktop/src-tauri/Info.plist" 'UltraVox uses Screen &amp; System Audio Recording'
assert_contains "$ROOT_DIR/apps/desktop/src-tauri/Info.plist" '<string>UltraVox</string>'
assert_contains "$ROOT_DIR/apps/desktop/src-tauri/Info.plist" 'CFBundleDisplayName'
assert_not_contains "$ROOT_DIR/apps/desktop/src-tauri/Info.plist" 'UltraTerm'
assert_contains "$ROOT_DIR/apps/desktop/src-tauri/src/commands.rs" 'Screen Recording access is disabled for UltraVox'
assert_not_contains "$ROOT_DIR/apps/desktop/src-tauri/src/commands.rs" 'Screen Recording access is disabled for UltraTerm'
assert_contains "$ROOT_DIR/apps/desktop/src-tauri/src/commands.rs" 'Privacy_ScreenCapture'
assert_contains "$PACKAGE_RELEASE" 'verify-app.sh'
for file in "$VERIFY_APP" "$VERIFY_PKG"; do
  assert_contains "$file" 'T63VT9UAY2'
done
assert_contains "$PACKAGE_RELEASE" 'REQUIRE_SIGNED="$SIGNED_RELEASE"'
assert_contains "$BUILD_PKG" 'verify-app.sh'
assert_contains "$VERIFY_APP" 'codesign --verify --deep --strict'
assert_contains "$VERIFY_APP" 'Designated requirement does not bind'
assert_contains "$VERIFY_PKG" 'install=/Applications'
assert_contains "$VERIFY_PKG" 'Per-user installation is disabled'
assert_contains "$VERIFY_PKG" 'PackageInfo'
assert_contains "$VERIFY_PKG" 'xcrun stapler validate'
assert_contains "$BUILD_PKG" '--install-location /Applications'
assert_contains "$BUILD_PKG" '--component-plist "$COMPONENT_PLIST"'
assert_contains "$BUILD_PKG" '--root "$PAYLOAD_ROOT"'
assert_contains "$BUILD_PKG" 'ditto "$APP_PATH" "$PAYLOAD_ROOT/UltraVox.app"'
assert_not_contains "$BUILD_PKG" '--component "$APP_PATH"'
assert_contains "$BUILD_PKG" '<key>BundleIsRelocatable</key><false/>'
assert_contains "$VERIFY_PKG" 'count(/pkg-info/relocate/bundle)'
assert_contains "$BUILD_PKG" 'Refusing to produce an unsigned production package'
assert_not_contains "$BUILD_PKG" '--requirements'
assert_contains "$PACKAGE_RELEASE" 'APPLE_SIGNING_IDENTITY:?APPLE_SIGNING_IDENTITY is required'
assert_contains "$PACKAGE_RELEASE" 'APPLE_INSTALLER_SIGNING_IDENTITY:?APPLE_INSTALLER_SIGNING_IDENTITY is required'
assert_contains "$PACKAGE_RELEASE" 'NOTARYTOOL_PROFILE:?NOTARYTOOL_PROFILE is required'
assert_not_contains "$PACKAGE_RELEASE" '/Users/'
assert_contains "$PACKAGE_RELEASE" 'ARCHIVE_NAME="${PAYLOAD_NAME}.zip"'
assert_contains "$PACKAGE_RELEASE" 'verify-app.sh'
assert_contains "$ROOT_DIR/.gitmodules" 'third_party/mediaremote-adapter'
# The retired music helper remains archival source only, never a build or bundle input.
assert_not_contains "$PACKAGE_RELEASE" 'MediaRemoteAdapter'
assert_not_contains "$PACKAGE_RELEASE" 'mediaremote-adapter'
assert_not_contains "$PACKAGE_RELEASE" 'ADAPTER_'
assert_not_contains "$PACKAGE_RELEASE" 'cmake'
assert_contains "$VERIFY_APP" 'Retired music helper must not be bundled'
assert_not_contains "$VERIFY_APP" 'APP_ADAPTER_FRAMEWORK'

# One product, one asset family: UltraVox-<platform> with compatibility copies.
assert_contains "$PACKAGE_RELEASE" 'PAYLOAD_NAME="UltraVox-${PLATFORM}"'
assert_contains "$PACKAGE_RELEASE" 'STABLE_PKG="${OUTPUT_DIR}/UltraVox-${PLATFORM}.pkg"'
assert_contains "$PACKAGE_RELEASE" 'for family in UltraVox-Light UltraVox-Pro; do'
assert_contains "$PACKAGE_RELEASE" 'custom-protocol,ultravox-pro'
assert_not_contains "$PACKAGE_RELEASE" 'UltraVox-Pro-macos'
assert_not_contains "$PACKAGE_RELEASE" 'UltraVox-Light-macos'
assert_contains "$BUILD_PKG" 'PKG_PRODUCT="UltraVox"'
assert_not_contains "$BUILD_PKG" 'UltraVox-Pro'
assert_contains "$BUILD_PKG" 'UltraVox Light.app'
assert_contains "$BUILD_PKG" 'UltraVox Pro.app'
assert_contains "$PUBLISH_RELEASE" 'PUBLIC_REPO:-michael-berardi/ultravox'
assert_contains "$PUBLISH_RELEASE" 'gh release upload'
assert_contains "$PUBLISH_RELEASE" 'MIKE_APPROVED_PUBLIC_RELEASE'
assert_contains "$PUBLISH_RELEASE" '--verify-tag --draft --latest=false'
assert_contains "$PUBLISH_RELEASE" 'REVIEWED_SOURCE_COMMIT'
assert_contains "$PUBLISH_RELEASE" 'verify_latest_bridge'
assert_contains "$PUBLISH_RELEASE" 'Only v0.13.3 bridge activation may be latest=true'
assert_contains "$ROOT_DIR/apps/desktop/src-tauri/src/lib.rs" 'bridge_guard::intercept'
assert_contains "$ROOT_DIR/apps/desktop/src-tauri/src/bridge_guard.rs" 'Install final'
assert_not_contains "$ROOT_DIR/README.md" '/releases/latest'
assert_contains "$ROOT_DIR/README.md" '/releases/tag/v0.13.4'
assert_contains "$PUBLISH_RELEASE" 'macos-arm64-recovery'
assert_contains "$PUBLISH_RELEASE" 'Keep that endpoint pinned to v0.13.3'
assert_contains "$PUBLISH_RELEASE" 'shasum -a 256 -c'
assert_contains "$ROOT_DIR/apps/desktop/src-tauri/src/update.rs" '/releases?per_page=100'
assert_contains "$ROOT_DIR/apps/desktop/src-tauri/src/update.rs" '/releases/download/{tag}'
assert_not_contains "$ROOT_DIR/apps/desktop/src-tauri/src/update.rs" '/releases/latest/download'
assert_not_contains "$ROOT_DIR/apps/desktop/src-tauri/src/update.rs" 'format!("{feed}/releases/latest")'
assert_contains "$PUBLISH_RELEASE" 'refusing to overwrite immutable assets'
assert_not_contains "$PUBLISH_RELEASE" '--clobber'
assert_not_contains "$PUBLISH_RELEASE" 'ultravox-''light'
assert_contains "$PACKAGE_ALL" 'Scripts/package-release.sh'
assert_not_contains "$PACKAGE_ALL" 'ultravox-''light'
assert_contains "$EXPORT_OSS" 'open-source-exclude.txt'
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
const tauriDev = JSON.parse(fs.readFileSync(path.join(root, 'apps/desktop/src-tauri/tauri.dev.conf.json'), 'utf8'));
const cargo = fs.readFileSync(path.join(root, 'apps/desktop/src-tauri/Cargo.toml'), 'utf8');
const cargoVersion = cargo.match(/^version = "([^"]+)"$/m)?.[1];
const version = packageJson.version;
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`invalid release version ${version}`);
if (desktopJson.version !== version || tauri.version !== version || cargoVersion !== version) {
  throw new Error(`version metadata drift: root=${version} desktop=${desktopJson.version} tauri=${tauri.version} cargo=${cargoVersion}`);
}
for (const crate of ['crates/ultravox-core/Cargo.toml', 'crates/ultravox-macos-bridge/Cargo.toml']) {
  const v = fs.readFileSync(path.join(root, crate), 'utf8').match(/^version = "([^"]+)"$/m)?.[1];
  if (v !== version) throw new Error(`workspace version drift: ${crate}=${v}`);
}
const lock = fs.readFileSync(path.join(root, 'Cargo.lock'), 'utf8');
for (const crate of ['ultravox', 'ultravox-core', 'ultravox-macos-bridge']) {
  if (!lock.includes(`name = "${crate}"\nversion = "${version}"`)) throw new Error(`lock version drift: ${crate}`);
}
if (tauri.productName !== 'UltraVox' || tauri.identifier !== 'com.paretocybernetics.ultravox') throw new Error('Tauri identity drift');
if (!tauri.bundle?.copyright?.includes('Pareto Cybernetics')) throw new Error('public product branding must use Pareto Cybernetics');
if (tauriDev.identifier !== 'com.imploselabs.ultravox.dev') throw new Error('Tauri development identity must stay isolated from production TCC grants');
if (!packageJson.scripts['desktop:dev'].includes('tauri:dev') || !desktopJson.scripts['tauri:dev'].includes('tauri.dev.conf.json')) {
  throw new Error('development scripts must use the isolated Tauri identity');
}
NODE

echo 'release contract passed'
