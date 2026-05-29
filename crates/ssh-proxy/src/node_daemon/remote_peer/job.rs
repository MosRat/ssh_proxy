use super::{JobPhase, JobRecord, JobState, ProxySessionSpec};

pub(super) fn remote_peer_job(
    alias: &str,
    job_id: &str,
    kind: &str,
    spec: Option<&ProxySessionSpec>,
    phase: JobPhase,
    progress: u8,
) -> JobRecord {
    let mut job = JobRecord::new(job_id.to_string(), kind.to_string())
        .with_target(alias.to_string())
        .transition(JobState::Running, phase, progress);
    if let Some(spec) = spec {
        job = job
            .with_workspace(spec.workspace_id.clone())
            .with_route(spec.route_id())
            .with_remote_url(Some(spec.remote_url()));
    }
    job
}

pub(super) fn sanitize_key(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}
