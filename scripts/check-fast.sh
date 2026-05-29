#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
EXTENSION_DIR="$ROOT/apps/vscode-remote-proxy"
OLD_ALLOW_MISSING_SIDECAR="${SSH_PROXY_ALLOW_MISSING_SIDECAR-}"
OLD_RUSTC_WRAPPER="${RUSTC_WRAPPER-}"
OLD_CARGO_INCREMENTAL="${CARGO_INCREMENTAL-}"

SKIP_RUST=0
SKIP_VSCODE=0
INSTALL_NODE_MODULES=0
NO_SCCACHE=0
NO_PROCESS_CLEANUP=0
FULL=0
TRANSPORT=0
CONTRACTS=0
CARGO_CONFIG_ARGS=""

for arg in "$@"; do
  case "$arg" in
    --skip-rust) SKIP_RUST=1 ;;
    --skip-vscode) SKIP_VSCODE=1 ;;
    --install-node-modules) INSTALL_NODE_MODULES=1 ;;
    --no-sccache) NO_SCCACHE=1 ;;
    --no-process-cleanup) NO_PROCESS_CLEANUP=1 ;;
    --full) FULL=1 ;;
    --transport) TRANSPORT=1 ;;
    --contracts) CONTRACTS=1 ;;
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

cleanup() {
  cleanup_test_binaries
  if [ -z "$OLD_ALLOW_MISSING_SIDECAR" ]; then
    unset SSH_PROXY_ALLOW_MISSING_SIDECAR
  else
    export SSH_PROXY_ALLOW_MISSING_SIDECAR="$OLD_ALLOW_MISSING_SIDECAR"
  fi
  if [ -z "$OLD_RUSTC_WRAPPER" ]; then
    unset RUSTC_WRAPPER
  else
    export RUSTC_WRAPPER="$OLD_RUSTC_WRAPPER"
  fi
  if [ -z "$OLD_CARGO_INCREMENTAL" ]; then
    unset CARGO_INCREMENTAL
  else
    export CARGO_INCREMENTAL="$OLD_CARGO_INCREMENTAL"
  fi
}
trap cleanup EXIT

cd "$ROOT"

if [ "$SKIP_RUST" != "1" ]; then
  export SSH_PROXY_ALLOW_MISSING_SIDECAR="${SSH_PROXY_ALLOW_MISSING_SIDECAR:-1}"
  cleanup_test_binaries
  if [ "$NO_SCCACHE" != "1" ] && [ -z "${RUSTC_WRAPPER-}" ] && command -v sccache >/dev/null 2>&1; then
    export RUSTC_WRAPPER=sccache
    export CARGO_INCREMENTAL=0
    CARGO_CONFIG_ARGS="--config profile.dev.incremental=false --config profile.test.incremental=false"
    sccache --start-server >/dev/null 2>&1 || true
  fi

  # shellcheck disable=SC2086
  cargo $CARGO_CONFIG_ARGS check --workspace --tests
  if [ "$FULL" = "1" ]; then
    if cargo nextest --version >/dev/null 2>&1; then
      # shellcheck disable=SC2086
      cargo $CARGO_CONFIG_ARGS nextest run --workspace --tests
    else
      # shellcheck disable=SC2086
      cargo $CARGO_CONFIG_ARGS test --workspace --tests -- --test-threads=1
    fi
  else
    # shellcheck disable=SC2086
    cargo $CARGO_CONFIG_ARGS test -p ssh_proxy --bin ssh_proxy protocol_core
    # shellcheck disable=SC2086
    cargo $CARGO_CONFIG_ARGS test -p ssh_proxy --bin ssh_proxy peer_lifecycle
    # shellcheck disable=SC2086
    cargo $CARGO_CONFIG_ARGS test -p ssh_proxy --bin ssh_proxy deploy
    # shellcheck disable=SC2086
    cargo $CARGO_CONFIG_ARGS test -p ssh_proxy --bin ssh_proxy remote_config_write
    # shellcheck disable=SC2086
    cargo $CARGO_CONFIG_ARGS test -p ssh_proxy --bin ssh_proxy remote_resolve_defaults
    # shellcheck disable=SC2086
    cargo $CARGO_CONFIG_ARGS test -p ssh_proxy --bin ssh_proxy node_daemon::handoff
    # shellcheck disable=SC2086
    cargo $CARGO_CONFIG_ARGS test -p ssh_proxy --test node_daemon node_daemon_reuses_duplicate_route_start_for_same_spec -- --test-threads=1
    if [ "$CONTRACTS" = "1" ]; then
      # shellcheck disable=SC2086
      cargo $CARGO_CONFIG_ARGS test -p ssh_proxy --test build_contract
      # shellcheck disable=SC2086
      cargo $CARGO_CONFIG_ARGS test -p ssh_proxy --test cli cli_help_exposes_only_production_daemon_commands
    fi
    if [ "$TRANSPORT" = "1" ]; then
      # shellcheck disable=SC2086
      cargo $CARGO_CONFIG_ARGS test -p ssh_proxy --test node_daemon fixed_tcp_target_can_proxy_to_specific_port -- --test-threads=1
    fi
  fi

  if [ "${RUSTC_WRAPPER-}" = "sccache" ]; then
    sccache --show-stats || true
  fi
fi

if [ "$SKIP_VSCODE" != "1" ]; then
  if [ "$INSTALL_NODE_MODULES" = "1" ] || [ ! -d "$EXTENSION_DIR/node_modules" ]; then
    npm --prefix "$EXTENSION_DIR" ci
  fi
  npm --prefix "$EXTENSION_DIR" test
fi
