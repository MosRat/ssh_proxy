#!/usr/bin/env sh
set -eu

if ! command -v sccache >/dev/null 2>&1; then
  echo "sccache was not found on PATH. Install it with 'cargo install sccache' or your package manager, then rerun this script." >&2
  exit 127
fi

export RUSTC_WRAPPER=sccache
sccache --start-server >/dev/null 2>&1 || true

if [ "$#" -eq 0 ]; then
  set -- check
fi

cargo "$@"
status=$?
sccache --show-stats || true
exit "$status"
