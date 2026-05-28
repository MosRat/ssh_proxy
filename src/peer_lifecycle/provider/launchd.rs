use crate::cli;

use super::util::{
    node_daemon_extra_args, remote_mark_service_state_command, token_arg, xml_escape,
};

pub(crate) fn remote_launchd_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    let token_arg = token_arg(args.remote_token.as_deref());
    let extra_args = node_daemon_extra_args(args);
    let plist = "$HOME/Library/LaunchAgents/com.ssh-proxy.helper.plist";
    format!(
        "set -eu; mkdir -p \"$HOME/Library/LaunchAgents\"; cat > {plist} <<'EOF'\n<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>com.ssh-proxy.helper</string>\n<key>ProgramArguments</key><array><string>{remote_path}</string><string>node</string><string>daemon</string><string>--transport</string><string>{remote_tcp}</string><string>--control</string><string>tcp://{remote_control}</string>{token_plist}{extra_plist}</array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n<key>StandardOutPath</key><string>{home}/.ssh_proxy/log/launchd.log</string>\n<key>StandardErrorPath</key><string>{home}/.ssh_proxy/log/launchd.log</string>\n</dict></plist>\nEOF\nmkdir -p \"$HOME/.ssh_proxy/log\"; launchctl bootout gui/$(id -u) {plist} >/dev/null 2>&1 || true; launchctl bootstrap gui/$(id -u) {plist}; launchctl kickstart -k gui/$(id -u)/com.ssh-proxy.helper; {mark}",
        plist = plist,
        remote_path = xml_escape(remote_path),
        remote_tcp = args.remote_tcp,
        remote_control = args.remote_control,
        token_plist = token_arg_to_plist(&token_arg),
        extra_plist = extra_args_to_plist(&extra_args),
        home = "$HOME",
        mark = remote_mark_service_state_command("launchd_user", "healthy", "start_service"),
    )
}

fn token_arg_to_plist(token_arg: &str) -> String {
    if token_arg.is_empty() {
        return String::new();
    }
    token_arg
        .split_whitespace()
        .map(|part| format!("<string>{}</string>", xml_escape(part.trim_matches('\''))))
        .collect::<Vec<_>>()
        .join("")
}

fn extra_args_to_plist(extra_args: &str) -> String {
    extra_args
        .split_whitespace()
        .map(|part| format!("<string>{}</string>", xml_escape(part.trim_matches('\''))))
        .collect::<Vec<_>>()
        .join("")
}
