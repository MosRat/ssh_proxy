use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

use super::config::{MatrixConfig, MatrixLevel};

#[derive(Debug, Clone, Copy)]
pub(super) struct MeasurementSample {
    pub(super) bytes: u64,
    pub(super) duration_ms: u128,
    pub(super) first_byte_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct MatrixCaseReport {
    pub level: String,
    pub target: Option<String>,
    pub topology: Option<String>,
    pub case: String,
    pub workload: Option<String>,
    pub selected_transport: Option<String>,
    pub selection_source: Option<String>,
    pub selection_reason: Option<String>,
    pub fallback_classification: Option<String>,
    pub measurement_scope: Option<String>,
    pub sample_count: Option<u64>,
    pub request_count: Option<u64>,
    pub concurrency: Option<u64>,
    pub run_window_ms: Option<u128>,
    pub payload_bytes: Option<u64>,
    pub bytes: Option<u64>,
    pub duration_ms: Option<u128>,
    pub mibps: Option<f64>,
    pub median_mibps: Option<f64>,
    pub p95_mibps: Option<f64>,
    pub first_byte_ms: Option<u128>,
    pub median_first_byte_ms: Option<u128>,
    pub p95_first_byte_ms: Option<u128>,
    pub median_duration_ms: Option<u128>,
    pub p95_duration_ms: Option<u128>,
    pub lost_requests: Option<u64>,
    pub reconnect_count: Option<u64>,
    pub cleanup_status: Option<String>,
    pub artifact_path: Option<String>,
    pub status: String,
    pub error_kind: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
struct MatrixReportJson {
    level: String,
    requested_level: String,
    targets: Vec<String>,
    artifact_dir: String,
    run_level: String,
    mode: &'static str,
    cases: Vec<MatrixCaseReport>,
}

pub(super) struct MatrixReport {
    level: MatrixLevel,
    run_level: MatrixLevel,
    targets: Vec<String>,
    artifact_dir: PathBuf,
    cases: Vec<MatrixCaseReport>,
}

impl MatrixCaseReport {
    pub(super) fn new(
        level: &str,
        target: Option<&str>,
        topology: Option<&str>,
        case: &str,
    ) -> Self {
        Self {
            level: level.to_string(),
            target: target.map(ToOwned::to_owned),
            topology: topology.map(ToOwned::to_owned),
            case: case.to_string(),
            workload: None,
            selected_transport: None,
            selection_source: None,
            selection_reason: None,
            fallback_classification: None,
            measurement_scope: None,
            sample_count: None,
            request_count: None,
            concurrency: None,
            run_window_ms: None,
            payload_bytes: None,
            bytes: None,
            duration_ms: None,
            mibps: None,
            median_mibps: None,
            p95_mibps: None,
            first_byte_ms: None,
            median_first_byte_ms: None,
            p95_first_byte_ms: None,
            median_duration_ms: None,
            p95_duration_ms: None,
            lost_requests: None,
            reconnect_count: None,
            cleanup_status: None,
            artifact_path: None,
            status: "passed".to_string(),
            error_kind: None,
            error: None,
        }
    }

    pub(super) fn fail(&mut self, kind: impl Into<String>, error: impl Into<String>) {
        self.status = "failed".to_string();
        self.error_kind = Some(kind.into());
        self.error = Some(error.into());
    }

    pub(super) fn skip(&mut self, kind: impl Into<String>, reason: impl Into<String>) {
        self.status = "skipped".to_string();
        self.error_kind = Some(kind.into());
        self.error = Some(reason.into());
    }

    pub(super) fn with_transport(mut self, transport: &str, source: &str, reason: &str) -> Self {
        self.selected_transport = Some(transport.to_string());
        self.selection_source = Some(source.to_string());
        self.selection_reason = Some(reason.to_string());
        self
    }

    pub(super) fn with_workload(mut self, workload: &str) -> Self {
        self.workload = Some(workload.to_string());
        self
    }

    pub(super) fn with_measurement(&mut self, bytes: u64, duration_ms: u128, first_byte_ms: u128) {
        self.bytes = Some(bytes);
        self.duration_ms = Some(duration_ms);
        self.first_byte_ms = Some(first_byte_ms);
        if duration_ms > 0 {
            self.mibps = Some(((bytes as f64) / 1024.0 / 1024.0) / (duration_ms as f64 / 1000.0));
        }
    }

    pub(super) fn with_measurement_samples(&mut self, samples: &[MeasurementSample]) {
        if samples.is_empty() {
            return;
        }
        let bytes = samples.iter().map(|sample| sample.bytes).sum();
        let duration_ms = samples
            .iter()
            .map(|sample| sample.duration_ms.max(1))
            .sum::<u128>();
        let first_byte_ms = samples
            .iter()
            .map(|sample| sample.first_byte_ms)
            .min()
            .unwrap_or(0);
        self.with_measurement(bytes, duration_ms.max(1), first_byte_ms);

        let mibps_values: Vec<_> = samples
            .iter()
            .filter_map(|sample| mibps(sample.bytes, sample.duration_ms))
            .collect();
        self.median_mibps = median_f64(&mibps_values);
        self.p95_mibps = p95_f64(&mibps_values);

        let durations: Vec<_> = samples
            .iter()
            .map(|sample| sample.duration_ms.max(1))
            .collect();
        self.median_duration_ms = median_u128(&durations);
        self.p95_duration_ms = p95_u128(&durations);

        let first_bytes: Vec<_> = samples.iter().map(|sample| sample.first_byte_ms).collect();
        self.median_first_byte_ms = median_u128(&first_bytes);
        self.p95_first_byte_ms = p95_u128(&first_bytes);
    }

    pub(super) fn with_measurement_context(
        &mut self,
        scope: &str,
        sample_count: u64,
        request_count: u64,
        concurrency: u64,
        run_window_ms: u128,
    ) {
        self.measurement_scope = Some(scope.to_string());
        self.sample_count = Some(sample_count);
        self.request_count = Some(request_count);
        self.concurrency = Some(concurrency);
        self.run_window_ms = Some(run_window_ms);
    }
}

impl MatrixReport {
    pub(super) fn new(level: MatrixLevel, config: &MatrixConfig) -> Self {
        Self {
            level,
            run_level: config.run_level,
            targets: config.targets.clone(),
            artifact_dir: config.artifact_dir.clone(),
            cases: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, row: MatrixCaseReport) {
        self.cases.push(row);
    }

    pub(super) fn write(&mut self) -> PathBuf {
        fs::create_dir_all(&self.artifact_dir).unwrap_or_else(|err| {
            panic!(
                "failed to create transport matrix artifact dir {}: {err}",
                self.artifact_dir.display()
            )
        });
        let json_path = self.artifact_dir.join("transport-matrix.json");
        let csv_path = self.artifact_dir.join("transport-matrix.csv");
        let summary_path = self.artifact_dir.join("transport-matrix-summary.md");
        let json_path_text = json_path.display().to_string();
        for row in &mut self.cases {
            row.artifact_path = Some(json_path_text.clone());
        }

        let json = MatrixReportJson {
            level: self.level.as_str().to_string(),
            requested_level: self.level.as_str().to_string(),
            targets: self.targets.clone(),
            artifact_dir: self.artifact_dir.display().to_string(),
            run_level: self.run_level.as_str().to_string(),
            mode: "report-first",
            cases: self.cases.clone(),
        };
        fs::write(
            &json_path,
            serde_json::to_string_pretty(&json).expect("serialize matrix report"),
        )
        .unwrap_or_else(|err| panic!("failed to write {}: {err}", json_path.display()));
        fs::write(&csv_path, csv_rows(&self.cases))
            .unwrap_or_else(|err| panic!("failed to write {}: {err}", csv_path.display()));
        fs::write(&summary_path, summary_table(&self.cases))
            .unwrap_or_else(|err| panic!("failed to write {}: {err}", summary_path.display()));
        json_path
    }

    pub(super) fn summary_table(&self) -> String {
        summary_table(&self.cases)
    }

    pub(super) fn assert_no_hard_failures(&self) {
        let failures: Vec<_> = self
            .cases
            .iter()
            .filter(|row| row.status == "failed")
            .map(|row| {
                format!(
                    "{} target={} kind={} error={}",
                    row.case,
                    row.target.as_deref().unwrap_or("local"),
                    row.error_kind.as_deref().unwrap_or("unknown"),
                    row.error.as_deref().unwrap_or("")
                )
            })
            .collect();
        assert!(
            failures.is_empty(),
            "transport matrix hard failures:\n{}",
            failures.join("\n")
        );
    }
}

fn csv_rows(rows: &[MatrixCaseReport]) -> String {
    let mut output = String::from(
        "level,target,topology,case,workload,selected_transport,selection_source,selection_reason,fallback_classification,measurement_scope,sample_count,request_count,concurrency,run_window_ms,payload_bytes,bytes,duration_ms,mibps,median_mibps,p95_mibps,first_byte_ms,median_first_byte_ms,p95_first_byte_ms,median_duration_ms,p95_duration_ms,lost_requests,reconnect_count,cleanup_status,artifact_path,status,error_kind,error\n",
    );
    for row in rows {
        let fields = [
            row.level.as_str(),
            row.target.as_deref().unwrap_or(""),
            row.topology.as_deref().unwrap_or(""),
            row.case.as_str(),
            row.workload.as_deref().unwrap_or(""),
            row.selected_transport.as_deref().unwrap_or(""),
            row.selection_source.as_deref().unwrap_or(""),
            row.selection_reason.as_deref().unwrap_or(""),
            row.fallback_classification.as_deref().unwrap_or(""),
            row.measurement_scope.as_deref().unwrap_or(""),
            &row.sample_count
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.request_count
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.concurrency
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.run_window_ms
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.payload_bytes
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.bytes.map(|value| value.to_string()).unwrap_or_default(),
            &row.duration_ms
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.mibps
                .map(|value| format!("{value:.6}"))
                .unwrap_or_default(),
            &row.median_mibps
                .map(|value| format!("{value:.6}"))
                .unwrap_or_default(),
            &row.p95_mibps
                .map(|value| format!("{value:.6}"))
                .unwrap_or_default(),
            &row.first_byte_ms
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.median_first_byte_ms
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.p95_first_byte_ms
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.median_duration_ms
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.p95_duration_ms
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.lost_requests
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.reconnect_count
                .map(|value| value.to_string())
                .unwrap_or_default(),
            row.cleanup_status.as_deref().unwrap_or(""),
            row.artifact_path.as_deref().unwrap_or(""),
            row.status.as_str(),
            row.error_kind.as_deref().unwrap_or(""),
            row.error.as_deref().unwrap_or(""),
        ];
        output.push_str(
            &fields
                .iter()
                .map(|field| csv_escape(field))
                .collect::<Vec<_>>()
                .join(","),
        );
        output.push('\n');
    }
    output
}

fn summary_table(rows: &[MatrixCaseReport]) -> String {
    let mut output = String::from(
        "| target | topology | workload | case | transport | status | MiB/s med/p95 | first byte med/p95 ms | duration med/p95 ms | bytes | req/lost | cleanup |\n",
    );
    output.push_str(
        "| --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n",
    );
    let mut selected: Vec<_> = rows
        .iter()
        .filter(|row| {
            row.workload.is_some() || row.cleanup_status.is_some() || row.status != "passed"
        })
        .collect();
    if selected.is_empty() {
        selected = rows.iter().collect();
    }
    for row in selected {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {}/{} | {} |\n",
            md_cell(row.target.as_deref().unwrap_or("local")),
            md_cell(row.topology.as_deref().unwrap_or("")),
            md_cell(row.workload.as_deref().unwrap_or("")),
            md_cell(&row.case),
            md_cell(row.selected_transport.as_deref().unwrap_or("")),
            md_cell(&row.status),
            throughput_cell(row),
            ms_pair(
                row.median_first_byte_ms,
                row.p95_first_byte_ms,
                row.first_byte_ms
            ),
            ms_pair(row.median_duration_ms, row.p95_duration_ms, row.duration_ms),
            row.bytes
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            row.request_count.unwrap_or(0),
            row.lost_requests.unwrap_or(0),
            md_cell(row.cleanup_status.as_deref().unwrap_or("")),
        ));
    }
    output
}

fn throughput_cell(row: &MatrixCaseReport) -> String {
    if row.workload.as_deref() == Some("control") {
        return "n/a".to_string();
    }
    match (row.median_mibps, row.p95_mibps, row.mibps) {
        (Some(median), Some(p95), _) => format!("{median:.3}/{p95:.3}"),
        (_, _, Some(value)) => format!("{value:.3}"),
        _ => "-".to_string(),
    }
}

fn ms_pair(median: Option<u128>, p95: Option<u128>, fallback: Option<u128>) -> String {
    match (median, p95, fallback) {
        (Some(median), Some(p95), _) => format!("{median}/{p95}"),
        (_, _, Some(value)) => value.to_string(),
        _ => "-".to_string(),
    }
}

fn md_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[allow(dead_code)]
pub(super) fn path_text(path: &Path) -> String {
    path.display().to_string()
}

fn mibps(bytes: u64, duration_ms: u128) -> Option<f64> {
    (duration_ms > 0).then(|| ((bytes as f64) / 1024.0 / 1024.0) / (duration_ms as f64 / 1000.0))
}

fn median_f64(values: &[f64]) -> Option<f64> {
    let mut values: Vec<_> = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    values.sort_by(f64::total_cmp);
    match values.len() {
        0 => None,
        len if len % 2 == 1 => Some(values[len / 2]),
        len => Some((values[(len / 2) - 1] + values[len / 2]) / 2.0),
    }
}

fn p95_f64(values: &[f64]) -> Option<f64> {
    let mut values: Vec<_> = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    values.sort_by(f64::total_cmp);
    nearest_rank_index(values.len(), 0.95).map(|index| values[index])
}

fn median_u128(values: &[u128]) -> Option<u128> {
    let mut values = values.to_vec();
    values.sort_unstable();
    match values.len() {
        0 => None,
        len if len % 2 == 1 => Some(values[len / 2]),
        len => Some(values[(len / 2) - 1].saturating_add(values[len / 2]) / 2),
    }
}

fn p95_u128(values: &[u128]) -> Option<u128> {
    let mut values = values.to_vec();
    values.sort_unstable();
    nearest_rank_index(values.len(), 0.95).map(|index| values[index])
}

fn nearest_rank_index(len: usize, percentile: f64) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let rank = ((len as f64) * percentile).ceil().max(1.0) as usize;
    Some(rank.saturating_sub(1).min(len - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurement_samples_record_median_and_p95() {
        let mut row = MatrixCaseReport::new("perf-smoke", Some("direct"), Some("direct"), "spx")
            .with_workload("large-download");
        row.with_measurement_samples(&[
            MeasurementSample {
                bytes: 1024 * 1024,
                duration_ms: 1000,
                first_byte_ms: 20,
            },
            MeasurementSample {
                bytes: 1024 * 1024,
                duration_ms: 500,
                first_byte_ms: 10,
            },
            MeasurementSample {
                bytes: 1024 * 1024,
                duration_ms: 250,
                first_byte_ms: 30,
            },
        ]);

        assert_eq!(row.bytes, Some(3 * 1024 * 1024));
        assert_eq!(row.duration_ms, Some(1750));
        assert_eq!(row.first_byte_ms, Some(10));
        assert_eq!(row.median_mibps, Some(2.0));
        assert_eq!(row.p95_mibps, Some(4.0));
        assert_eq!(row.median_first_byte_ms, Some(20));
        assert_eq!(row.p95_first_byte_ms, Some(30));
    }

    #[test]
    fn summary_table_uses_percentiles_and_hides_control_throughput() {
        let mut control =
            MatrixCaseReport::new("perf-smoke", Some("target"), Some("direct"), "control")
                .with_workload("control");
        control.with_measurement_samples(&[MeasurementSample {
            bytes: 128,
            duration_ms: 10,
            first_byte_ms: 8,
        }]);

        let mut download =
            MatrixCaseReport::new("perf-smoke", Some("target"), Some("direct"), "download")
                .with_workload("large-download");
        download.with_measurement_samples(&[
            MeasurementSample {
                bytes: 1024 * 1024,
                duration_ms: 1000,
                first_byte_ms: 11,
            },
            MeasurementSample {
                bytes: 1024 * 1024,
                duration_ms: 500,
                first_byte_ms: 9,
            },
        ]);

        let table = summary_table(&[control, download]);
        assert!(table.contains("MiB/s med/p95"));
        assert!(table.contains("| control | control |"));
        assert!(table.contains("| n/a | 8/8 |"));
        assert!(table.contains("| large-download | download |"));
        assert!(table.contains("| 1.500/2.000 | 10/11 |"));
    }

    #[test]
    fn csv_includes_percentile_columns() {
        let row = MatrixCaseReport::new("perf-smoke", None, None, "case");
        let csv = csv_rows(&[row]);
        assert!(csv.starts_with("level,target,topology"));
        assert!(csv.contains("median_mibps,p95_mibps"));
        assert!(csv.contains("median_first_byte_ms,p95_first_byte_ms"));
        assert!(csv.contains("median_duration_ms,p95_duration_ms"));
    }
}
