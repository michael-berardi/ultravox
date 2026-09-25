#!/usr/bin/env bash
# Exports the open-source tree of UltraVox into <dest-dir>.
#
# Usage:
#   Scripts/export-open-source.sh <dest-dir>          export, then leak gate
#   Scripts/export-open-source.sh --verify <dest-dir> leak gate only
#
# The export copies tracked files (git ls-files) and then:
#   - drops the Pro module roots (apps/desktop/src-tauri/src/pro/ and
#     apps/desktop/src/pro/), the release/ artifacts, and every path listed in
#     Scripts/open-source-exclude.txt (override with EXPORT_MANIFEST);
#   - removes the Pro feature lines (ultravox-pro, pro-dev-unlock,
#     voice-studio) from every Cargo manifest;
#   - hard-wires the @pro frontend alias to src/pro-stub.
#
# The export refuses a non-empty destination and is repeatable into an empty
# one. It finishes with a leak gate that fails the export on any excluded path
# still present, an ungated Pro module declaration, private admin endpoints or
# flags, access-key literals carrying key material, dev vars files, private key
# blocks, or (when gitleaks is installed) any gitleaks finding.
set -euo pipefail

usage() {
  echo "Usage: $0 <dest-dir>" >&2
  echo "       $0 --verify <dest-dir>" >&2
  exit 2
}

MODE="export"
if [[ "${1:-}" == "--verify" ]]; then
  MODE="verify"
  shift
fi
[[ $# -eq 1 ]] || usage
DEST="${1%/}"
[[ -n "$DEST" ]] || usage
if [[ "$MODE" == "verify" && ! -d "$DEST" ]]; then
  echo "Nothing to verify: $DEST is not a directory." >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="${EXPORT_MANIFEST:-${ROOT_DIR}/Scripts/open-source-exclude.txt}"

MANIFEST_PATHS=()
if [[ -f "$MANIFEST" ]]; then
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%"${line##*[![:space:]]}"}"
    case "$line" in ""|\#*) continue ;; esac
    MANIFEST_PATHS+=("${line%/}")
  done < "$MANIFEST"
fi

is_excluded() {
  local rel="$1" entry
  case "$rel" in
    apps/desktop/src-tauri/src/pro|apps/desktop/src-tauri/src/pro/*) return 0 ;;
    apps/desktop/src/pro|apps/desktop/src/pro/*) return 0 ;;
    release|release/*) return 0 ;;
  esac
  for entry in ${MANIFEST_PATHS[@]+"${MANIFEST_PATHS[@]}"}; do
    entry="${entry%/}"
    [[ -n "$entry" ]] || continue
    if [[ "$rel" == "$entry" || "$rel" == "$entry"/* ]]; then return 0; fi
  done
  return 1
}

if [[ "$MODE" == "export" ]]; then
  if [[ -d "$DEST" && -n "$(ls -A "$DEST" 2>/dev/null)" ]]; then
    echo "Refusing to export into a non-empty destination: $DEST" >&2
    exit 1
  fi
  mkdir -p "$DEST"
  while IFS= read -r -d '' rel; do
    if is_excluded "$rel"; then continue; fi
    src="${ROOT_DIR}/${rel}"
    # Tracked but deleted in the working tree: it is not part of the export.
    [[ -e "$src" || -L "$src" ]] || continue
    dst="${DEST}/${rel}"
    mkdir -p "$(dirname "$dst")"
    if [[ -d "$src" && ! -L "$src" ]]; then
      # Submodule working tree (gitlink entry).
      cp -R "$src" "$dst"
      rm -rf "$dst/.git"
    else
      cp -p "$src" "$dst"
    fi
  done < <(git -C "$ROOT_DIR" ls-files -z)

  python3 - "$DEST" <<'PY'
import os
import re
import sys

dest = sys.argv[1]
FEATURES = ("ultravox-pro", "pro-dev-unlock", "voice-studio")
ALIAS_CONFIG = re.compile(r"^(vite\.config\..*|tsconfig[^/]*\.json)$")


def strip_feature_lines(text):
    section = ""
    out = []
    comments = []

    def flush():
        # A comment block that names a removed feature is stale in the public
        # tree; everything else is kept.
        if comments and not any(f in "".join(comments) for f in FEATURES):
            out.extend(comments)
        del comments[:]

    for line in text.splitlines(keepends=True):
        stripped = line.strip()
        if stripped.startswith("#"):
            comments.append(line)
            continue
        match = re.match(r"\[([^\]]*)\]", stripped)
        if match:
            section = match.group(1)
        drop = section == "features" and bool(
            re.match(r'"?(?:%s)"?\s*=' % "|".join(FEATURES), stripped)
        )
        if drop:
            # The feature's doc comment goes with the feature line.
            del comments[:]
            continue
        flush()
        for feature in FEATURES:
            quoted = '"%s"' % feature
            line = line.replace(quoted + ",", "")
            line = re.sub(r",\s*" + re.escape(quoted), "", line)
            line = line.replace(quoted, "")
        out.append(line)
    flush()
    return "".join(out)


def hardwire_alias(text):
    return re.sub(r"(?<![\w.-])src/pro(?![\w-])", "src/pro-stub", text)


for root, dirs, names in os.walk(dest):
    dirs[:] = [d for d in dirs if d != ".git"]
    for name in names:
        path = os.path.join(root, name)
        if name == "Cargo.toml":
            text = open(path, encoding="utf-8").read()
            rewritten = strip_feature_lines(text)
        elif ALIAS_CONFIG.match(name):
            text = open(path, encoding="utf-8").read()
            rewritten = hardwire_alias(text)
        else:
            continue
        if rewritten != text:
            open(path, "w", encoding="utf-8").write(rewritten)
PY
fi

python3 - "$DEST" "$MANIFEST" <<'PY'
import os
import re
import shutil
import subprocess
import sys

dest = sys.argv[1]
manifest = sys.argv[2]

BUILTIN_EXCLUDED = [
    "apps/desktop/src-tauri/src/pro",
    "apps/desktop/src/pro",
    "release",
]

NEEDLE_ADMIN = "software.implosecybernetics.com/api/" + "admin"
NEEDLE_ADMIN_FLAG = "DISTRIBUTION_" + "ADMIN"
NEEDLE_KEY_PREFIX = "imp_" + "live_"
NEEDLE_DEV_VARS = "." + "dev" + ".vars"
RE_MOD_PRO = re.compile(r"mod\s+" + r"pro\s*;")
RE_KEY_BLOCK = re.compile(r"-----BEGIN [A-Z ]*" + "PRIVATE KEY")
RE_ALIAS_PATH = re.compile(r"(?<![\w.-])src/pro(?![\w-])")
ALIAS_CONFIG = re.compile(r"^(vite\.config\..*|tsconfig[^/]*\.json)$")

errors = []


def manifest_paths():
    paths = []
    if manifest and os.path.isfile(manifest):
        for raw in open(manifest, encoding="utf-8"):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            paths.append(line.rstrip("/"))
    return paths


def iter_files():
    for root, dirs, names in os.walk(dest):
        dirs[:] = [d for d in dirs if d != ".git"]
        for name in names:
            yield os.path.join(root, name)


for rel in BUILTIN_EXCLUDED + manifest_paths():
    if os.path.exists(os.path.join(dest, rel)):
        errors.append("excluded path present in export: %s" % rel)

for path in iter_files():
    rel = os.path.relpath(path, dest)
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        continue
    if NEEDLE_ADMIN in text:
        errors.append("%s: private admin endpoint" % rel)
    if NEEDLE_ADMIN_FLAG in text:
        errors.append("%s: distribution admin flag" % rel)
    if NEEDLE_DEV_VARS in text:
        errors.append("%s: dev vars file reference" % rel)
    if RE_KEY_BLOCK.search(text):
        errors.append("%s: private key block" % rel)
    lines = text.splitlines()
    for index, line in enumerate(lines):
        if RE_MOD_PRO.search(line):
            window = " ".join(lines[max(0, index - 1):index + 1])
            if "cfg" not in window:
                errors.append("%s:%d: Pro module declaration without a cfg gate"
                              % (rel, index + 1))
        if NEEDLE_KEY_PREFIX in line:
            cursor = 0
            while True:
                at = line.find(NEEDLE_KEY_PREFIX, cursor)
                if at < 0:
                    break
                body = re.match(r"[A-Za-z0-9_-]*",
                                line[at + len(NEEDLE_KEY_PREFIX):]).group(0)
                # A bare prefix constant is fine; so is an obvious placeholder
                # or test fixture. Real keys carry 22 characters of key
                # material after the prefix; anything close to that is a leak.
                if body and len(body) >= 20 and len(set(body)) > 1:
                    errors.append("%s:%d: access-key literal with key material"
                                  % (rel, index + 1))
                cursor = at + 1
    if ALIAS_CONFIG.match(os.path.basename(path)):
        if RE_ALIAS_PATH.search(text):
            errors.append("%s: @pro alias is not hard-wired to src/pro-stub" % rel)

if shutil.which("gitleaks") and not errors:
    command = ["gitleaks", "detect", "--source", dest, "--no-git", "--redact"]
    config = os.path.join(dest, ".gitleaks.toml")
    if os.path.isfile(config):
        command += ["--config", config]
    if subprocess.run(command).returncode != 0:
        errors.append("gitleaks reported findings in the export")
elif not shutil.which("gitleaks"):
    print("leak gate: gitleaks not installed; source scan skipped")

if errors:
    for error in errors:
        print("leak gate: %s" % error, file=sys.stderr)
    sys.exit(1)
print("leak gate: passed")
PY

if [[ "$MODE" == "export" ]]; then
  printf 'Open-source tree exported to %s\n' "$DEST"
fi
