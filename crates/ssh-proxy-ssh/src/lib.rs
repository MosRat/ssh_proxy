mod auth;
pub mod client;

pub use client::{Client, Target, openssh_default_identity_candidates, resolve_target};
pub use ssh_proxy_core::command::ExecOutput;
