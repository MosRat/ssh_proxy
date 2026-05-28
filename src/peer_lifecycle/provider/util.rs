use crate::cli;

pub(crate) fn token_arg(token: Option<&str>) -> String {
    token
        .map(|token| format!(" --token {}", sh_quote(token)))
        .unwrap_or_default()
}

pub(crate) fn node_daemon_extra_args(args: &cli::InstallRemoteArgs) -> String {
    let mut out = String::new();
    if let Some(addr) = args.remote_tls_transport {
        out.push_str(&format!(" --tls-transport {addr}"));
    }
    if let Some(addr) = args.remote_quic_transport {
        out.push_str(&format!(" --quic-transport {addr}"));
    }
    if let Some(path) = &args.remote_tls_cert {
        out.push_str(&format!(" --tls-cert {}", sh_quote(path)));
    }
    if let Some(path) = &args.remote_tls_key {
        out.push_str(&format!(" --tls-key {}", sh_quote(path)));
    }
    if let Some(path) = &args.remote_tls_client_ca {
        out.push_str(&format!(" --tls-client-ca {}", sh_quote(path)));
    }
    out
}

pub(crate) fn remote_mark_service_state_command(manager: &str, state: &str, phase: &str) -> String {
    format!(
        "now=$(date +%s 2>/dev/null || printf 0); mkdir -p \"$HOME/.ssh_proxy\"; cat > \"$HOME/.ssh_proxy/install_report.json\" <<EOF\n{{\"schema\":\"ssh_proxy_remote_install.v1\",\"state\":\"{state}\",\"phase\":\"{phase}\",\"service_manager\":\"{manager}\",\"updated_at_unix\":$now}}\nEOF\ncat > \"$HOME/.ssh_proxy/health.json\" <<EOF\n{{\"schema\":\"ssh_proxy_peer_health.v1\",\"state\":\"{state}\",\"service_manager\":\"{manager}\",\"updated_at_unix\":$now}}\nEOF"
    )
}

pub(crate) fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn windows_cmd_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

pub(crate) fn windows_extra_args(args: &cli::InstallRemoteArgs) -> String {
    let mut out = String::new();
    if let Some(addr) = args.remote_tls_transport {
        out.push_str(&format!(" --tls-transport {addr}"));
    }
    if let Some(addr) = args.remote_quic_transport {
        out.push_str(&format!(" --quic-transport {addr}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quoting_handles_single_quotes() {
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
    }
}
