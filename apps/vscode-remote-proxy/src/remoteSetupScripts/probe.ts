import { shellQuote } from './shell';

export function buildVerifyForwardScript(host: string, port: number): string {
  return `
set -eu
host=${shellQuote(host)}
port=${shellQuote(String(port))}
if command -v nc >/dev/null 2>&1; then
  nc -z "$host" "$port"
  exit $?
fi
if command -v python3 >/dev/null 2>&1; then
  python3 - "$host" "$port" <<'PY'
import socket
import sys
host = sys.argv[1]
port = int(sys.argv[2])
sock = socket.create_connection((host, port), timeout=2)
sock.close()
PY
  exit $?
fi
if command -v bash >/dev/null 2>&1; then
  bash -c ":</dev/tcp/$host/$port"
  exit $?
fi
echo "remote-proxy: no nc/python3/bash available to verify forwarded port" >&2
exit 2
`;
}

export function buildRemotePortFreeScript(host: string, port: number): string {
  return `
set -eu
host=${shellQuote(host)}
port=${shellQuote(String(port))}
if command -v python3 >/dev/null 2>&1; then
  python3 - "$host" "$port" <<'PY'
import socket
import sys
host = sys.argv[1]
port = int(sys.argv[2])
sock = socket.socket()
sock.settimeout(1)
try:
    sock.connect((host, port))
except OSError:
    sys.exit(0)
else:
    sys.exit(1)
finally:
    sock.close()
PY
  exit $?
fi
if command -v nc >/dev/null 2>&1; then
  if nc -z "$host" "$port" >/dev/null 2>&1; then
    exit 1
  fi
  exit 0
fi
if command -v bash >/dev/null 2>&1; then
  if bash -c ":</dev/tcp/$host/$port" >/dev/null 2>&1; then
    exit 1
  fi
  exit 0
fi
exit 0
`;
}
