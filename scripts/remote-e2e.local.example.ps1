# Copy this file to scripts/remote-e2e.local.ps1 and edit the values for your lab.
# scripts/remote-e2e.local.ps1 is ignored by Git.

# Required opt-in gate.
# Set-Item Env:SSH_PROXY_REMOTE_E2E "1"

# Run level for the ignored Rust test harness: probe, smoke, or full.
# Set-Item Env:SSH_PROXY_REMOTE_LEVEL "probe"

# Comma-separated SSH target aliases from your local SSH config.
# Set-Item Env:SSH_PROXY_REMOTE_TARGETS "proxyjump-alias,direct-alias"

# Optional topology labels used only for redacted reporting.
# Set-Item Env:SSH_PROXY_REMOTE_JUMP_TARGET "proxyjump-alias"
# Set-Item Env:SSH_PROXY_REMOTE_DIRECT_TARGET "direct-alias"

# Optional local upstream proxy used by tests that exercise proxy setup.
# Set-Item Env:SSH_PROXY_REMOTE_UPSTREAM_PROXY "http://127.0.0.1:<local-proxy-port>"

# Allow learning unknown host keys during opt-in runs.
# Set-Item Env:SSH_PROXY_REMOTE_ACCEPT_NEW "0"

# Keep remote temporary files after a failed or exploratory run.
# Set-Item Env:SSH_PROXY_REMOTE_KEEP "0"

# Optional override for the release sidecar built by cargo zigbuild.
# Set-Item Env:SSH_PROXY_REMOTE_SIDECAR "target/x86_64-unknown-linux-musl/release/ssh_proxy"
