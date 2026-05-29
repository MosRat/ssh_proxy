pub(crate) mod artifact;
pub(crate) mod launchd;
pub(crate) mod nohup;
pub(crate) mod systemd;
pub(crate) mod util;
pub(crate) mod windows_schtasks;
pub(crate) mod windows_scm;

pub(crate) use artifact::remote_write_peer_artifact_command;
pub(crate) use launchd::remote_launchd_install_command;
pub(crate) use nohup::{
    remote_nohup_files, remote_nohup_start_command, remote_nohup_status_snippet,
    remote_nohup_stop_snippet,
};
pub(crate) use systemd::remote_systemd_install_command;
pub(crate) use util::{node_daemon_extra_args, sh_quote, token_arg};
pub(crate) use windows_schtasks::remote_schtasks_install_command;
