#!/usr/bin/env bash
# Exercises Scripts/export-open-source.sh on a temporary destination: expected
# exclusions, Cargo feature stripping, @pro alias hard-wiring, non-empty
# destination refusal, manifest handling, and every leak-gate failure class.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXPORT="${ROOT_DIR}/Scripts/export-open-source.sh"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/ultravox-export-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

fail() {
  echo "export test: $1" >&2
  exit 1
}

DEST="${TMP}/oss"
bash "$EXPORT" "$DEST" >/dev/null

# The export carries the open-source build inputs...
for rel in README.md LICENSE package.json Cargo.toml \
  apps/desktop/src-tauri/src/lib.rs Scripts/build-pkg.sh Scripts/verify-app.sh; do
  [[ -e "$DEST/$rel" ]] || fail "missing from export: $rel"
done

# ...and nothing private.
for rel in apps/desktop/src-tauri/src/pro apps/desktop/src/pro release \
  Scripts/build-organization-bundle Scripts/mirror-control Scripts/publish-release.sh; do
  [[ ! -e "$DEST/$rel" ]] || fail "excluded path leaked into export: $rel"
done

# Pro feature lines must be gone from the Cargo manifests.
if grep -RIl --include='Cargo.toml' -e 'ultravox-pro' -e 'pro-dev-unlock' "$DEST" | grep -q .; then
  fail "Pro feature lines remain in exported Cargo manifests"
fi

# The @pro alias must be hard-wired to the stub.
if grep -RIn --include='vite.config.*' --include='tsconfig*.json' 'src/pro' "$DEST/apps/desktop" 2>/dev/null \
    | grep -v 'src/pro-stub' | grep -q .; then
  fail "@pro alias is not hard-wired to src/pro-stub"
fi

# A non-empty destination is refused so the export stays repeatable.
if bash "$EXPORT" "$DEST" >/dev/null 2>&1; then
  fail "export must refuse a non-empty destination"
fi

# The exclusion manifest is honored (EXPORT_MANIFEST override proves it).
MANIFEST="${TMP}/exclude.txt"
printf '%s\n' 'README.md' > "$MANIFEST"
DEST2="${TMP}/oss-manifest"
EXPORT_MANIFEST="$MANIFEST" bash "$EXPORT" "$DEST2" >/dev/null
[[ ! -e "$DEST2/README.md" ]] || fail "manifest entry was not excluded"

# The leak gate must fail every leak class.
leak() {
  local name="$1" needle="$2" dir="${TMP}/leak-${1}"
  cp -R "$DEST" "$dir"
  printf '%s\n' "$needle" > "${dir}/LEAK.txt"
  if bash "$EXPORT" --verify "$dir" >/dev/null 2>&1; then
    fail "leak gate did not catch: ${name}"
  fi
}
leak admin-endpoint 'software.implosecybernetics.com/api/'"admin"
leak admin-flag 'DISTRIBUTION_''ADMIN'
leak access-key "imp_""live_51AbCdEfGhIjKlMnOpQrSt"
leak dev-vars '.'"dev.vars"
leak private-key '-----BEGIN ''PRIVATE KEY'
leak pro-module "pub mod pro"";"

cp -R "$DEST" "${TMP}/leak-excluded"
mkdir -p "${TMP}/leak-excluded/apps/desktop/src/pro"
if bash "$EXPORT" --verify "${TMP}/leak-excluded" >/dev/null 2>&1; then
  fail "leak gate did not catch an excluded path"
fi

# Assertions over the Pro restructure: the export copies tracked files
# (git ls-files), so each check runs only once its Pro-side counterpart is
# tracked; the exclusion checks always hold.
[[ ! -e "$DEST/apps/desktop/src-tauri/src/pro" ]] || fail "src-tauri pro/ leaked"
[[ ! -e "$DEST/apps/desktop/src/pro" ]] || fail "frontend pro/ leaked"
pro_checked=0
if git -C "$ROOT_DIR" ls-files apps/desktop/src-tauri/src/pro_stub.rs | grep -q .; then
  [[ -e "$DEST/apps/desktop/src-tauri/src/pro_stub.rs" ]] || fail "pro_stub.rs missing from export"
  pro_checked=1
fi
if git -C "$ROOT_DIR" ls-files apps/desktop/src/pro-stub | grep -q .; then
  [[ -d "$DEST/apps/desktop/src/pro-stub" ]] || fail "pro-stub/ missing from export"
  pro_checked=1
fi
if [[ "$pro_checked" == "1" ]]; then
  echo "export test: pro restructure assertions passed"
else
  echo "export test: SKIP pro restructure stub assertions (stubs not tracked yet)"
fi

echo "export test: passed"
