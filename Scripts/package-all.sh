#!/usr/bin/env bash
# Packages the official UltraVox build for the current native host. There is
# one product and one packager: this is a thin entry point around
# Scripts/package-release.sh. Run it once on macOS arm64, Linux x86_64, and
# Windows x86_64; each run stages the UltraVox-<platform> asset family plus
# the compatibility copies in release/.
# Operator prerequisites (per fleet release contract):
#   APPLE_SIGNING_IDENTITY + APPLE_INSTALLER_SIGNING_IDENTITY set, or the
#   shared notarization keychain profile available for `xcrun notarytool`.
set -euo pipefail

export PATH="${HOME}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:${PATH}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

printf '==> Packaging UltraVox on %s/%s\n' "$(uname -s)" "$(uname -m)"
./Scripts/package-release.sh

printf '\nAssets ready for upload:\n'
for asset in release/*; do if [[ -f "$asset" ]]; then printf '%s\n' "$asset"; fi; done

cat <<'NOTE'

Next steps (operator runbook — do not commit tokens):
  Publish the public release to the canonical repository:
    ./Scripts/publish-release.sh
NOTE
