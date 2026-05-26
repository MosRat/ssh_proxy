# Copy this file to scripts/bench.local.ps1 and edit the values for your lab.
# scripts/bench.local.ps1 is ignored by Git.

# Comma-separated SSH target aliases from your local SSH config.
# Set-Item Env:SSH_PROXY_BENCH_TARGETS "ssh-only-peer,direct-peer"

# Upstream proxy used by reverse-egress benchmarks.
# Set-Item Env:SSH_PROXY_BENCH_UPSTREAM_PROXY "http://127.0.0.1:<local-proxy-port>"

# Optional payload and readiness URLs.
# Set-Item Env:SSH_PROXY_BENCH_URL "https://example.com/payload.bin"
# Set-Item Env:SSH_PROXY_BENCH_READINESS_URL "https://example.com/"
