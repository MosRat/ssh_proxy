#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
EXTENSION_DIR="$ROOT/apps/vscode-remote-proxy"

: "${SSH_PROXY_ALLOW_MISSING_SIDECAR:=1}"
export SSH_PROXY_ALLOW_MISSING_SIDECAR

cd "$ROOT"

cargo fmt -- --check
cargo check
cargo test --tests

if [ ! -d "$EXTENSION_DIR/node_modules" ]; then
  npm --prefix "$EXTENSION_DIR" ci
fi
npm --prefix "$EXTENSION_DIR" test
