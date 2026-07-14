#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub exit_status: u32,
    pub stdout: String,
    pub stderr: String,
}
