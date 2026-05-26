#!/usr/bin/env sh
set -eu

TARGET="${TARGET:-x86_64-unknown-linux-musl}"
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

cd "$ROOT"

if [ "${NO_SCCACHE:-0}" != "1" ] && command -v sccache >/dev/null 2>&1; then
  export RUSTC_WRAPPER=sccache
  sccache --start-server >/dev/null 2>&1 || true
fi

cargo zigbuild --target "$TARGET" --release
SIDECAR="$ROOT/target/$TARGET/release/ssh_proxy"
if [ ! -f "$SIDECAR" ]; then
  echo "Linux musl sidecar was not produced at $SIDECAR" >&2
  exit 1
fi

export SSH_PROXY_LINUX_MUSL_BIN="$SIDECAR"
cargo build --release

if [ "${RUSTC_WRAPPER:-}" = "sccache" ]; then
  sccache --show-stats || true
fi
