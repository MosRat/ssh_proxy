mod local;
mod model;
mod ssh;

#[cfg(test)]
mod fake;

#[cfg(test)]
pub(crate) use fake::FakeExecutor;
pub(crate) use local::LocalExecutor;
pub(crate) use model::{BoxExecutorFuture, PeerExecutor, ServiceControlAction};
pub(crate) use ssh::SshExecutor;
