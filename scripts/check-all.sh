#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
EXTENSION_DIR="$ROOT/apps/vscode-remote-proxy"
NO_PROCESS_CLEANUP=0

for arg in "$@"; do
  case "$arg" in
    --no-process-cleanup) NO_PROCESS_CLEANUP=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

cleanup_test_binaries() {
  if [ "$NO_PROCESS_CLEANUP" = "1" ] || ! command -v pgrep >/dev/null 2>&1; then
    return
  fi
  pattern="$ROOT/target/debug/ssh_proxy"
  pids="$(pgrep -f "$pattern" 2>/dev/null || true)"
  if [ -z "$pids" ]; then
    return
  fi
  echo "$pids" | while IFS= read -r pid; do
    [ -z "$pid" ] && continue
    [ "$pid" = "$$" ] && continue
    echo "Stopping stale Rust test process $pid"
    kill "$pid" 2>/dev/null || true
  done
  sleep 1
  pids="$(pgrep -f "$pattern" 2>/dev/null || true)"
  echo "$pids" | while IFS= read -r pid; do
    [ -z "$pid" ] && continue
    [ "$pid" = "$$" ] && continue
    kill -9 "$pid" 2>/dev/null || true
  done
}

trap cleanup_test_binaries EXIT

: "${SSH_PROXY_ALLOW_MISSING_SIDECAR:=1}"
export SSH_PROXY_ALLOW_MISSING_SIDECAR

cd "$ROOT"
cleanup_test_binaries

cargo fmt -- --check
cargo check
cargo test --tests

if [ ! -d "$EXTENSION_DIR/node_modules" ]; then
  npm --prefix "$EXTENSION_DIR" ci
fi
npm --prefix "$EXTENSION_DIR" test
