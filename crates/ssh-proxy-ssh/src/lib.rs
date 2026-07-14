mod auth;
pub mod client;

pub use client::{
    Client, SshStream, Target, connect_intent, openssh_default_identity_candidates,
    resolve_intent_target, resolve_target,
};
pub use ssh_proxy_core::command::ExecOutput;
