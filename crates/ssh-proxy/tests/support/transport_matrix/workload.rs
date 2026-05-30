use super::config::MatrixLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MatrixWorkload {
    Control,
    LargeDownload,
    LargeUpload,
    LongConnection,
    HighConcurrency,
}

impl MatrixWorkload {
    pub(super) fn parse_list(value: Option<&str>, level: MatrixLevel) -> Vec<Self> {
        let Some(value) = value else {
            return default_workloads(level);
        };
        let mut workloads = Vec::new();
        for item in value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            match item.to_ascii_lowercase().as_str() {
                "all" => {
                    push_unique(&mut workloads, Self::Control);
                    push_unique(&mut workloads, Self::LargeDownload);
                    push_unique(&mut workloads, Self::LargeUpload);
                    push_unique(&mut workloads, Self::LongConnection);
                    push_unique(&mut workloads, Self::HighConcurrency);
                }
                "control" | "status" => push_unique(&mut workloads, Self::Control),
                "large" | "large-download" | "download" => {
                    push_unique(&mut workloads, Self::LargeDownload)
                }
                "large-upload" | "upload" => push_unique(&mut workloads, Self::LargeUpload),
                "long" | "long-connection" | "long-stream" => {
                    push_unique(&mut workloads, Self::LongConnection)
                }
                "concurrent" | "high-concurrency" | "concurrency" => {
                    push_unique(&mut workloads, Self::HighConcurrency)
                }
                _ => {}
            }
        }
        if workloads.is_empty() {
            default_workloads(level)
        } else {
            workloads
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::LargeDownload => "large-download",
            Self::LargeUpload => "large-upload",
            Self::LongConnection => "long-connection",
            Self::HighConcurrency => "high-concurrency",
        }
    }

    pub(super) fn requires_bench_server(self) -> bool {
        !matches!(self, Self::Control)
    }

    pub(super) fn measurement_scope(self, backend: &'static str) -> &'static str {
        match (self, backend) {
            (Self::Control, "openssh") => "control-status-through-openssh-forward",
            (Self::Control, _) => "control-status-through-proxy",
            (Self::LargeDownload, "openssh") => "large-download-through-openssh-forward",
            (Self::LargeDownload, _) => "large-download-through-proxy",
            (Self::LargeUpload, "openssh") => "large-upload-through-openssh-forward",
            (Self::LargeUpload, _) => "large-upload-through-proxy",
            (Self::LongConnection, "openssh") => "long-connection-through-openssh-forward",
            (Self::LongConnection, _) => "long-connection-through-proxy",
            (Self::HighConcurrency, "openssh") => "high-concurrency-through-openssh-forward",
            (Self::HighConcurrency, _) => "high-concurrency-through-proxy",
        }
    }
}

fn default_workloads(level: MatrixLevel) -> Vec<MatrixWorkload> {
    match level {
        MatrixLevel::Probe => Vec::new(),
        MatrixLevel::Smoke => vec![MatrixWorkload::Control],
        MatrixLevel::PerfSmoke => vec![
            MatrixWorkload::Control,
            MatrixWorkload::LargeDownload,
            MatrixWorkload::LargeUpload,
            MatrixWorkload::HighConcurrency,
        ],
        MatrixLevel::Stability => vec![MatrixWorkload::Control, MatrixWorkload::LongConnection],
    }
}

fn push_unique(workloads: &mut Vec<MatrixWorkload>, workload: MatrixWorkload) {
    if !workloads.contains(&workload) {
        workloads.push(workload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_list_expands_all_and_deduplicates_aliases() {
        let workloads = MatrixWorkload::parse_list(
            Some("all,download,large-download,upload,long,concurrent"),
            MatrixLevel::PerfSmoke,
        );

        assert_eq!(
            workloads,
            vec![
                MatrixWorkload::Control,
                MatrixWorkload::LargeDownload,
                MatrixWorkload::LargeUpload,
                MatrixWorkload::LongConnection,
                MatrixWorkload::HighConcurrency,
            ]
        );
    }

    #[test]
    fn parse_list_falls_back_to_level_defaults() {
        assert_eq!(
            MatrixWorkload::parse_list(Some("unknown"), MatrixLevel::Smoke),
            vec![MatrixWorkload::Control]
        );
        assert_eq!(
            MatrixWorkload::parse_list(None, MatrixLevel::Stability),
            vec![MatrixWorkload::Control, MatrixWorkload::LongConnection]
        );
    }
}
