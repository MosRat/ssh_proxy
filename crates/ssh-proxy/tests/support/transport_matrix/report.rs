use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

use super::config::{MatrixConfig, MatrixLevel};

#[derive(Debug, Clone, Serialize)]
pub(super) struct MatrixCaseReport {
    pub level: String,
    pub target: Option<String>,
    pub topology: Option<String>,
    pub case: String,
    pub selected_transport: Option<String>,
    pub selection_source: Option<String>,
    pub selection_reason: Option<String>,
    pub fallback_classification: Option<String>,
    pub measurement_scope: Option<String>,
    pub sample_count: Option<u64>,
    pub request_count: Option<u64>,
    pub concurrency: Option<u64>,
    pub run_window_ms: Option<u128>,
    pub bytes: Option<u64>,
    pub duration_ms: Option<u128>,
    pub mibps: Option<f64>,
    pub first_byte_ms: Option<u128>,
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
            selected_transport: None,
            selection_source: None,
            selection_reason: None,
            fallback_classification: None,
            measurement_scope: None,
            sample_count: None,
            request_count: None,
            concurrency: None,
            run_window_ms: None,
            bytes: None,
            duration_ms: None,
            mibps: None,
            first_byte_ms: None,
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

    pub(super) fn with_measurement(&mut self, bytes: u64, duration_ms: u128, first_byte_ms: u128) {
        self.bytes = Some(bytes);
        self.duration_ms = Some(duration_ms);
        self.first_byte_ms = Some(first_byte_ms);
        if duration_ms > 0 {
            self.mibps = Some(((bytes as f64) / 1024.0 / 1024.0) / (duration_ms as f64 / 1000.0));
        }
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
        json_path
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
        "level,target,topology,case,selected_transport,selection_source,selection_reason,fallback_classification,measurement_scope,sample_count,request_count,concurrency,run_window_ms,bytes,duration_ms,mibps,first_byte_ms,lost_requests,reconnect_count,cleanup_status,artifact_path,status,error_kind,error\n",
    );
    for row in rows {
        let fields = [
            row.level.as_str(),
            row.target.as_deref().unwrap_or(""),
            row.topology.as_deref().unwrap_or(""),
            row.case.as_str(),
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
            &row.bytes.map(|value| value.to_string()).unwrap_or_default(),
            &row.duration_ms
                .map(|value| value.to_string())
                .unwrap_or_default(),
            &row.mibps
                .map(|value| format!("{value:.6}"))
                .unwrap_or_default(),
            &row.first_byte_ms
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
