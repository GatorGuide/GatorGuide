use chrono::{SecondsFormat, Utc};
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Number, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const ARTIFACT_FILENAMES: &[&str] = &[
    "legacy-transfer-planner-service-test.log",
    "legacy-transfer-planner-service-test.tap",
    "legacy-transfer-planner-service-test.jsonl",
];
const MEANING: &str = "Legacy transfer-planner service-test artifacts are opt-in diagnostics only. Planner accuracy pass/fail is the source-backed-runtime-coverage gate.";
const TRUSTED_GATE_LABEL: &str = "source-backed-runtime-coverage";

#[derive(Serialize)]
struct TrustedAccuracyGate {
    label: &'static str,
    #[serde(rename = "jsonReport")]
    json_report: String,
    #[serde(rename = "markdownReport")]
    markdown_report: String,
}

#[derive(Serialize)]
struct Artifact {
    path: String,
    exists: bool,
    status: &'static str,
    #[serde(rename = "staleReasons")]
    stale_reasons: Vec<String>,
    #[serde(rename = "maxReferencedLine")]
    max_referenced_line: u64,
    #[serde(rename = "currentEntryPointLineCount")]
    current_entry_point_line_count: usize,
    #[serde(rename = "tapSummary")]
    tap_summary: Option<Map<String, Value>>,
    #[serde(rename = "lastModified")]
    last_modified: Option<String>,
    #[serde(rename = "retiredPath")]
    retired_path: Option<String>,
}

#[derive(Serialize)]
struct StatusReport {
    #[serde(rename = "generatedAt")]
    generated_at: String,
    status: String,
    meaning: &'static str,
    #[serde(rename = "trustedAccuracyGate")]
    trusted_accuracy_gate: TrustedAccuracyGate,
    #[serde(rename = "legacyDiagnosticEntryPoint")]
    legacy_diagnostic_entry_point: String,
    #[serde(rename = "legacyDiagnosticEntryPointLineCount")]
    legacy_diagnostic_entry_point_line_count: usize,
    artifacts: Vec<Artifact>,
    #[serde(rename = "retiredArtifactCount")]
    retired_artifact_count: usize,
}

pub(super) struct RefreshResult {
    pub status: String,
    pub retired_artifact_count: usize,
    pub markdown_path: PathBuf,
}

pub(super) fn refresh(source_root: &Path) -> Result<RefreshResult, String> {
    refresh_with_retire(source_root, true)
}

fn refresh_with_retire(source_root: &Path, retire_stale: bool) -> Result<RefreshResult, String> {
    let report_dir = source_root.join(".tmp/reports");
    fs::create_dir_all(&report_dir).map_err(|error| {
        format!(
            "could not create legacy artifact report directory {}: {error}",
            report_dir.display()
        )
    })?;

    let json_path = report_dir.join("legacy-transfer-planner-service-test.status.json");
    let markdown_path = report_dir.join("legacy-transfer-planner-service-test.status.md");
    let entry_point_path = source_root.join("scripts/planner/transfer-planner.service.test.ts");
    let entry_point_line_count = count_file_lines(&entry_point_path)?;
    let entry_point_mtime_ms = file_mtime_ms(&entry_point_path)?;
    let artifact_paths = default_artifact_paths(source_root, &report_dir);
    let mut artifacts = artifact_paths
        .iter()
        .map(|path| build_artifact(path, entry_point_line_count, entry_point_mtime_ms))
        .collect::<Result<Vec<_>, _>>()?;

    for artifact in &mut artifacts {
        if !artifact.exists || artifact.status != "stale" {
            continue;
        }
        let artifact_path = PathBuf::from(&artifact.path);
        let retired_path = build_retired_path(&artifact_path)?;
        artifact.retired_path = Some(retired_path.display().to_string());
        artifact.status = if retire_stale {
            fs::rename(&artifact_path, &retired_path).map_err(|error| {
                format!(
                    "could not retire stale legacy service-test artifact {} to {}: {error}",
                    artifact_path.display(),
                    retired_path.display()
                )
            })?;
            artifact.exists = false;
            "retired-stale"
        } else {
            "stale"
        };
    }

    let retired_artifact_count = artifacts
        .iter()
        .filter(|artifact| artifact.status == "retired-stale")
        .count();
    let status = if retired_artifact_count > 0 {
        "stale-legacy-artifacts-retired"
    } else if artifacts.iter().any(|artifact| artifact.status == "stale") {
        "stale-legacy-artifacts-found"
    } else {
        "no-stale-legacy-artifacts"
    }
    .to_string();

    let report = StatusReport {
        generated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        status: status.clone(),
        meaning: MEANING,
        trusted_accuracy_gate: TrustedAccuracyGate {
            label: TRUSTED_GATE_LABEL,
            json_report: source_root
                .join(".tmp/reports/transfer-planner-source-backed-coverage-audit.json")
                .display()
                .to_string(),
            markdown_report: source_root
                .join(".tmp/reports/transfer-planner-source-backed-coverage-audit.md")
                .display()
                .to_string(),
        },
        legacy_diagnostic_entry_point: entry_point_path.display().to_string(),
        legacy_diagnostic_entry_point_line_count: entry_point_line_count,
        artifacts,
        retired_artifact_count,
    };

    write_json_report(&json_path, &report)?;
    write_markdown_report(&markdown_path, &render_markdown(&report))?;
    Ok(RefreshResult {
        status,
        retired_artifact_count,
        markdown_path,
    })
}

fn default_artifact_paths(source_root: &Path, report_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for filename in ARTIFACT_FILENAMES {
        let current_layout_path = source_root
            .join(".tmp")
            .join(if filename.ends_with(".log") {
                "logs"
            } else {
                "reports"
            })
            .join(filename);
        let legacy_report_path = report_dir.join(filename);
        if current_layout_path == legacy_report_path {
            paths.push(current_layout_path);
        } else {
            paths.push(current_layout_path);
            paths.push(legacy_report_path);
        }
    }
    paths
}

fn count_file_lines(path: &Path) -> Result<usize, String> {
    if !path.exists() {
        return Ok(0);
    }
    let text = read_lossy(path)?;
    if text.is_empty() {
        return Ok(0);
    }
    let bytes = text.as_bytes();
    let mut separators = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                separators += 1;
                if bytes.get(index + 1) == Some(&b'\n') {
                    index += 1;
                }
            }
            b'\n' => separators += 1,
            _ => {}
        }
        index += 1;
    }
    Ok(separators + 1)
}

fn build_artifact(
    path: &Path,
    entry_point_line_count: usize,
    entry_point_mtime_ms: Option<f64>,
) -> Result<Artifact, String> {
    let text = if path.exists() {
        Some(read_lossy(path)?)
    } else {
        None
    };
    let exists = text.is_some();
    let metadata = if exists {
        Some(fs::metadata(path).map_err(|error| {
            format!(
                "could not inspect legacy service-test artifact {}: {error}",
                path.display()
            )
        })?)
    } else {
        None
    };
    let modified_ms = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(system_time_ms);
    let mut max_referenced_line = 0u64;
    let mut stale_reasons = Vec::new();
    if let Some(text) = &text {
        max_referenced_line = max_referenced_line_in_text(text);
        if max_referenced_line > entry_point_line_count as u64 {
            stale_reasons.push(format!(
                "references line {max_referenced_line} in transfer-planner.service.test.ts, but the current entry point has {entry_point_line_count} line(s)"
            ));
        }
        if Regex::new(r"(?i)Cannot find package 'json' imported from")
            .expect("constant regex")
            .is_match(text)
        {
            stale_reasons.push(
                "captures a failed JSON reporter invocation, not planner test results".to_string(),
            );
        }
        if let (Some(artifact_mtime_ms), Some(entry_mtime_ms)) = (modified_ms, entry_point_mtime_ms)
        {
            if artifact_mtime_ms < entry_mtime_ms && stale_reasons.is_empty() {
                stale_reasons
                    .push("older than the current legacy diagnostic entry point".to_string());
            }
        }
    }

    let status = if !exists {
        "missing"
    } else if !stale_reasons.is_empty() {
        "stale"
    } else {
        "current-or-unclassified"
    };
    let tap_summary = text.as_deref().and_then(parse_tap_summary);
    Ok(Artifact {
        path: path.display().to_string(),
        exists,
        status,
        stale_reasons,
        max_referenced_line,
        current_entry_point_line_count: entry_point_line_count,
        tap_summary,
        last_modified: modified_ms.and_then(iso_timestamp),
        retired_path: None,
    })
}

fn max_referenced_line_in_text(text: &str) -> u64 {
    let pattern = Regex::new(r"(?i)transfer-planner\.service\.test\.ts:([0-9]+):([0-9]+)")
        .expect("constant regex");
    pattern
        .captures_iter(text)
        .filter_map(|captures| captures.get(1)?.as_str().parse::<u64>().ok())
        .max()
        .unwrap_or(0)
}

fn parse_tap_summary(text: &str) -> Option<Map<String, Value>> {
    let mut summary = Map::new();
    for key in ["tests", "pass", "fail", "cancelled", "skipped", "todo"] {
        let pattern = Regex::new(&format!(r"(?i)# {key} ([0-9]+)")).expect("constant regex");
        if let Some(captures) = pattern.captures(text) {
            if let Some(count) = captures
                .get(1)
                .and_then(|matched| matched.as_str().parse::<u64>().ok())
            {
                summary.insert(key.to_string(), Value::Number(Number::from(count)));
            }
        }
    }
    (!summary.is_empty()).then_some(summary)
}

fn build_retired_path(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("artifact path has no parent: {}", path.display()))?;
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("artifact path has no filename: {}", path.display()))?;
    let extension = path.extension().and_then(|value| value.to_str());
    let make_candidate = |suffix: &str| {
        let filename = match extension {
            Some(extension) => format!("{stem}.stale{suffix}.{extension}"),
            None => format!("{stem}.stale{suffix}"),
        };
        parent.join(filename)
    };
    let first = make_candidate("");
    if !first.exists() {
        return Ok(first);
    }
    for index in 2u64.. {
        let candidate = make_candidate(&format!("-{index}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    unreachable!("the stale artifact filename sequence is unbounded")
}

fn file_mtime_ms(path: &Path) -> Result<Option<f64>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let metadata = fs::metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    Ok(metadata.modified().ok().and_then(system_time_ms))
}

fn system_time_ms(time: SystemTime) -> Option<f64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs_f64() * 1_000.0)
}

fn iso_timestamp(milliseconds: f64) -> Option<String> {
    let millis = milliseconds.trunc() as i64;
    chrono::DateTime::<Utc>::from_timestamp_millis(millis)
        .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn read_lossy(path: &Path) -> Result<String, String> {
    fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .map_err(|error| format!("could not read {}: {error}", path.display()))
}

fn render_markdown(report: &StatusReport) -> String {
    let mut lines = vec![
        "# Legacy Transfer Planner Service-Test Artifact Status".to_string(),
        String::new(),
        format!("Generated: {}", report.generated_at),
        String::new(),
        format!("- Status: {}", report.status),
        format!("- Meaning: {}", report.meaning),
        format!(
            "- Trusted accuracy gate: {}",
            report.trusted_accuracy_gate.label
        ),
        format!(
            "- Trusted accuracy report: {}",
            report.trusted_accuracy_gate.markdown_report
        ),
        format!(
            "- Legacy diagnostic entry point: {}",
            report.legacy_diagnostic_entry_point
        ),
        format!(
            "- Current entry point lines: {}",
            report.legacy_diagnostic_entry_point_line_count
        ),
        format!(
            "- Retired stale artifacts: {}",
            report.retired_artifact_count
        ),
        String::new(),
        "## Artifacts".to_string(),
        String::new(),
        "| Artifact | Status | Reason | Retired Path | TAP Summary |".to_string(),
        "| --- | --- | --- | --- | --- |".to_string(),
    ];
    for artifact in &report.artifacts {
        let reasons = if artifact.stale_reasons.is_empty() {
            "n/a".to_string()
        } else {
            artifact.stale_reasons.join("; ").replace('|', "\\|")
        };
        let tap_summary = artifact
            .tap_summary
            .as_ref()
            .map(|summary| {
                summary
                    .iter()
                    .map(|(key, value)| format!("{key}: {value}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "n/a".to_string());
        lines.push(format!(
            "| {} | {} | {} | {} | {} |",
            artifact.path,
            artifact.status,
            reasons,
            artifact.retired_path.as_deref().unwrap_or_default(),
            tap_summary
        ));
    }
    lines.push(String::new());
    lines.push(
        "Use the source-backed coverage audit for planner accuracy. Run this legacy diagnostic only when investigating old service-test behavior."
            .to_string(),
    );
    lines.push(String::new());
    lines.join("\n")
}

fn write_json_report(path: &Path, report: &StatusReport) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("report path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "could not create report directory {}: {error}",
            parent.display()
        )
    })?;
    let mut serialized = serde_json::to_string_pretty(report)
        .map_err(|error| format!("could not serialize legacy artifact status: {error}"))?;
    serialized.push('\n');
    fs::write(path, serialized)
        .map_err(|error| format!("could not write report {}: {error}", path.display()))
}

fn write_markdown_report(path: &Path, markdown: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("report path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "could not create report directory {}: {error}",
            parent.display()
        )
    })?;
    let output = if markdown.ends_with('\n') {
        markdown.to_string()
    } else {
        format!("{markdown}\n")
    };
    fs::write(path, output)
        .map_err(|error| format!("could not write report {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn test_source_root() -> PathBuf {
        let root = loop {
            let id = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let candidate = std::env::temp_dir().join(format!(
                "gator-guide-legacy-artifact-status-{}-{timestamp}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("could not create test directory: {error}"),
            }
        };
        fs::create_dir_all(root.join("scripts/planner")).unwrap();
        fs::write(
            root.join("scripts/planner/transfer-planner.service.test.ts"),
            "line one\nline two\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn status_report_retires_stale_artifacts_and_preserves_tap_summary() {
        let root = test_source_root();
        let artifact = root.join(".tmp/logs/legacy-transfer-planner-service-test.log");
        fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        fs::write(
            &artifact,
            "transfer-planner.service.test.ts:4:4\n# tests 4\n# pass 3\n# fail 1\n",
        )
        .unwrap();

        let result = refresh(&root).unwrap();
        assert_eq!(result.status, "stale-legacy-artifacts-retired");
        assert_eq!(result.retired_artifact_count, 1);
        assert!(!artifact.exists());
        assert!(
            root.join(".tmp/logs/legacy-transfer-planner-service-test.stale.log")
                .is_file()
        );
        let report: Value = serde_json::from_slice(
            &fs::read(root.join(".tmp/reports/legacy-transfer-planner-service-test.status.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(report["status"], "stale-legacy-artifacts-retired");
        assert_eq!(report["retiredArtifactCount"], 1);
        assert_eq!(report["artifacts"][0]["status"], "retired-stale");
        assert_eq!(report["artifacts"][0]["tapSummary"]["tests"], 4);
        assert_eq!(report["artifacts"][0]["tapSummary"]["pass"], 3);
        assert_eq!(report["artifacts"][0]["tapSummary"]["fail"], 1);
        assert!(
            fs::read_to_string(result.markdown_path)
                .unwrap()
                .contains("| tests: 4, pass: 3, fail: 1 |")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn artifact_status_retains_missing_and_unclassified_states_without_retiring() {
        let root = test_source_root();
        let entry_path = root.join("scripts/planner/transfer-planner.service.test.ts");
        let line_count = count_file_lines(&entry_path).unwrap();
        let paths = default_artifact_paths(&root, &root.join(".tmp/reports"));
        assert_eq!(paths.len(), 4);
        assert!(paths[0].ends_with(".tmp/logs/legacy-transfer-planner-service-test.log"));
        assert!(paths[1].ends_with(".tmp/reports/legacy-transfer-planner-service-test.log"));
        let artifacts = paths
            .iter()
            .map(|path| build_artifact(path, line_count, file_mtime_ms(&entry_path).unwrap()))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            artifacts
                .iter()
                .all(|artifact| artifact.status == "missing")
        );
        assert_eq!(line_count, 3);

        let output = root.join(".tmp/reports/legacy-transfer-planner-service-test.tap");
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        fs::write(&output, "# tests 2\n# pass 2\n").unwrap();
        let artifact = build_artifact(&output, line_count, None).unwrap();
        assert_eq!(artifact.status, "current-or-unclassified");
        assert_eq!(artifact.tap_summary.unwrap()["tests"], 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_artifacts_can_be_reported_without_retiring_them() {
        let root = test_source_root();
        let artifact = root.join(".tmp/logs/legacy-transfer-planner-service-test.log");
        fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        fs::write(&artifact, "transfer-planner.service.test.ts:4:1\n").unwrap();

        let result = refresh_with_retire(&root, false).unwrap();
        assert_eq!(result.status, "stale-legacy-artifacts-found");
        assert_eq!(result.retired_artifact_count, 0);
        assert!(artifact.is_file());
        assert!(
            !root
                .join(".tmp/logs/legacy-transfer-planner-service-test.stale.log")
                .exists()
        );
        let report: Value = serde_json::from_slice(
            &fs::read(root.join(".tmp/reports/legacy-transfer-planner-service-test.status.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(report["artifacts"][0]["status"], "stale");
        assert!(
            report["artifacts"][0]["retiredPath"]
                .as_str()
                .unwrap()
                .replace('\\', "/")
                .ends_with(".tmp/logs/legacy-transfer-planner-service-test.stale.log")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
