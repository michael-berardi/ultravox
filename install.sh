#!/usr/bin/env bash
set -euo pipefail

PRODUCT="UltraVox"
REPOSITORY="michael-berardi/ultravox"
ARCHIVE="UltraVox-macos-arm64.zip"
INSTALL_SCOPE="user"
LAUNCH=1
INSTALL_DIR="${INSTALL_DIR:-}"
BIN_DIR="${BIN_DIR:-}"
DOWNLOAD_BASE="${DOWNLOAD_BASE:-https://github.com/${REPOSITORY}/releases/latest/download}"
ALLOW_UNNOTARIZED="${ALLOW_UNNOTARIZED:-0}"

usage() {
  cat <<'USAGE'
Install the latest prebuilt UltraVox app and ultravox-control CLI.

Usage: install.sh [--user|--system] [--no-launch]

  --user       Install to ~/Applications and ~/.local/bin (default; no sudo)
  --system     Install to /Applications and /usr/local/bin (uses sudo)
  --no-launch  Do not open UltraVox after installation

Environment overrides: DOWNLOAD_BASE, INSTALL_DIR, BIN_DIR, ALLOW_UNNOTARIZED=1.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --user) INSTALL_SCOPE="user" ;;
    --system) INSTALL_SCOPE="system" ;;
    --no-launch) LAUNCH=0 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "UltraVox supports macOS only." >&2
  exit 1
fi
if [[ "$(uname -m)" != "arm64" ]]; then
  echo "UltraVox currently supports Apple Silicon (arm64) only." >&2
  exit 1
fi

if [[ "$INSTALL_SCOPE" == "system" ]]; then
  INSTALL_DIR="${INSTALL_DIR:-/Applications}"
  BIN_DIR="${BIN_DIR:-/usr/local/bin}"
else
  INSTALL_DIR="${INSTALL_DIR:-${HOME}/Applications}"
  BIN_DIR="${BIN_DIR:-${HOME}/.local/bin}"
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ultravox-install.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
ARCHIVE_PATH="${TMP_DIR}/${ARCHIVE}"
CHECKSUM_PATH="${ARCHIVE_PATH}.sha256"

curl --fail --location --silent --show-error "${DOWNLOAD_BASE}/${ARCHIVE}" --output "$ARCHIVE_PATH"
curl --fail --location --silent --show-error "${DOWNLOAD_BASE}/${ARCHIVE}.sha256" --output "$CHECKSUM_PATH"
(
  cd "$TMP_DIR"
  shasum -a 256 -c "${ARCHIVE}.sha256"
)

ditto -x -k "$ARCHIVE_PATH" "$TMP_DIR/unpacked"
PAYLOAD_DIR="${TMP_DIR}/unpacked/UltraVox-macos-arm64"
APP_SOURCE="${PAYLOAD_DIR}/${PRODUCT}.app"
CLI_SOURCE="${PAYLOAD_DIR}/bin/ultravox-control"

if [[ ! -d "$APP_SOURCE" || ! -x "$CLI_SOURCE" ]]; then
  echo "Release archive is missing UltraVox.app or ultravox-control." >&2
  exit 1
fi
codesign --verify --deep --strict "$APP_SOURCE"
codesign --verify --strict "$CLI_SOURCE"
if ! spctl --assess --type execute "$APP_SOURCE" >/dev/null 2>&1 \
  && [[ "$ALLOW_UNNOTARIZED" != "1" ]]; then
  echo "UltraVox is not accepted by Gatekeeper; refusing installation." >&2
  exit 1
fi
"$CLI_SOURCE" health >/dev/null

APP_TARGET="${INSTALL_DIR}/${PRODUCT}.app"
APP_STAGE="${INSTALL_DIR}/.${PRODUCT}.app.install.$$"
APP_BACKUP="${INSTALL_DIR}/.${PRODUCT}.app.previous.$$"
CLI_TARGET="${BIN_DIR}/ultravox-control"
if [[ "$INSTALL_SCOPE" == "system" ]]; then
  sudo mkdir -p "$INSTALL_DIR" "$BIN_DIR"
  sudo rm -rf "$APP_STAGE" "$APP_BACKUP"
  sudo ditto "$APP_SOURCE" "$APP_STAGE"
  codesign --verify --deep --strict "$APP_STAGE"
  if [[ -d "$APP_TARGET" ]]; then
    sudo mv "$APP_TARGET" "$APP_BACKUP"
  fi
  if ! sudo mv "$APP_STAGE" "$APP_TARGET"; then
    [[ ! -d "$APP_BACKUP" ]] || sudo mv "$APP_BACKUP" "$APP_TARGET"
    exit 1
  fi
  sudo rm -rf "$APP_BACKUP"
  sudo install -m 0755 "$CLI_SOURCE" "$CLI_TARGET"
else
  mkdir -p "$INSTALL_DIR" "$BIN_DIR"
  rm -rf "$APP_STAGE" "$APP_BACKUP"
  ditto "$APP_SOURCE" "$APP_STAGE"
  codesign --verify --deep --strict "$APP_STAGE"
  if [[ -d "$APP_TARGET" ]]; then
    mv "$APP_TARGET" "$APP_BACKUP"
  fi
  if ! mv "$APP_STAGE" "$APP_TARGET"; then
    [[ ! -d "$APP_BACKUP" ]] || mv "$APP_BACKUP" "$APP_TARGET"
    exit 1
  fi
  rm -rf "$APP_BACKUP"
  install -m 0755 "$CLI_SOURCE" "$CLI_TARGET"
fi

codesign --verify --deep --strict "$APP_TARGET"
"$CLI_TARGET" health >/dev/null

echo "Installed UltraVox to $APP_TARGET"
echo "Installed ultravox-control to $CLI_TARGET"
case ":${PATH}:" in
  *":${BIN_DIR}:"*) ;;
  *) echo "Add ${BIN_DIR} to PATH to call ultravox-control directly." ;;
esac
if [[ "$LAUNCH" == "1" ]]; then
  open "$APP_TARGET"
fi
