#!/bin/zsh
set -euo pipefail

case "${1:-dev}" in
  build)
    exec pnpm desktop:build
    ;;
  dev)
    exec pnpm desktop:dev
    ;;
  *)
    echo "Usage: ./run.sh [dev|build]" >&2
    exit 2
    ;;
esac