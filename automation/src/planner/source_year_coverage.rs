use super::GrcPublicMaterials;
use chrono::{DateTime, Datelike, SecondsFormat, TimeZone, Utc};
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const CURRENT_ACADEMIC_START_MONTH: u32 = 7;
const FUTURE_COVERAGE_EXPECTED_MONTH: u32 = 10;
const FUTURE_COVERAGE_EXPECTED_DAY: u32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceYearCoverageReport {
    generated_at: String,
    now_utc: String,
    current_academic_start_year: i32,
    next_academic_start_year: i32,
    future_coverage_expected_after_utc: String,
    future_coverage_expected_by_now: bool,
    latest_schedule_start_year: Option<i32>,
    discovered_schedule_labels: Vec<String>,
    schedule_count: usize,
    covers_current_academic_year: bool,
    covers_future_academic_year: bool,
    stale_year_coverage: bool,
    missing_future_coverage: bool,
    current_catalog_label: Option<String>,
    current_catalog_root_url: Option<String>,
    used_snapshot_fallback: bool,
    snapshot_fallback_reason: Option<String>,
    outcome: &'static str,
    required_actions: Vec<String>,
}

pub(super) fn write_report(
    source_root: &Path,
    materials: &GrcPublicMaterials,
) -> Result<(), String> {
    let report = build_report(materials, Utc::now());
    let reports_dir = source_root.join(".tmp/reports");
    fs::create_dir_all(&reports_dir).map_err(|error| {
        format!(
            "could not create source-year coverage report directory {}: {error}",
            reports_dir.display()
        )
    })?;
    let json_path = reports_dir.join("transfer-planner-source-year-coverage.json");
    let markdown_path = reports_dir.join("transfer-planner-source-year-coverage.md");
    let mut serialized = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("could not serialize source-year coverage report: {error}"))?;
    serialized.push('\n');
    fs::write(&json_path, serialized)
        .map_err(|error| format!("could not write {}: {error}", json_path.display()))?;
    fs::write(&markdown_path, render_markdown(&report))
        .map_err(|error| format!("could not write {}: {error}", markdown_path.display()))?;

    println!("Source year coverage outcome: {}", report.outcome);
    println!(
        "Latest schedule: {}",
        report
            .latest_schedule_start_year
            .map_or_else(|| "none".to_string(), |year| year.to_string())
    );
    println!(
        "Current baseline: {}-{}",
        report.current_academic_start_year,
        report.current_academic_start_year + 1
    );
    println!(
        "Next baseline: {}-{}",
        report.next_academic_start_year,
        report.next_academic_start_year + 1
    );
    println!("JSON report: {}", json_path.display());
    println!("Markdown report: {}", markdown_path.display());
    for action in &report.required_actions {
        println!("- {action}");
    }
    Ok(())
}

fn build_report(materials: &GrcPublicMaterials, now: DateTime<Utc>) -> SourceYearCoverageReport {
    let mut seen_labels = HashSet::new();
    let mut schedule_labels = materials
        .annual_schedules
        .iter()
        .filter_map(|entry| {
            seen_labels
                .insert(entry.label.clone())
                .then(|| entry.label.clone())
        })
        .collect::<Vec<_>>();
    schedule_labels.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));

    let latest_schedule_start_year = schedule_labels
        .iter()
        .filter_map(|label| parse_academic_year_start(label))
        .max();
    let current_academic_start_year = if now.month() >= CURRENT_ACADEMIC_START_MONTH {
        now.year()
    } else {
        now.year() - 1
    };
    let next_academic_start_year = current_academic_start_year + 1;
    let future_coverage_expected_after = Utc
        .with_ymd_and_hms(
            current_academic_start_year,
            FUTURE_COVERAGE_EXPECTED_MONTH,
            FUTURE_COVERAGE_EXPECTED_DAY,
            0,
            0,
            0,
        )
        .single()
        .expect("valid October 1 UTC timestamp");
    let future_coverage_expected_by_now =
        now.timestamp_millis() >= future_coverage_expected_after.timestamp_millis();
    let covers_current_academic_year =
        latest_schedule_start_year.is_some_and(|year| year >= current_academic_start_year);
    let covers_future_academic_year =
        latest_schedule_start_year.is_some_and(|year| year >= next_academic_start_year);
    let stale_year_coverage = !covers_current_academic_year;
    let missing_future_coverage = future_coverage_expected_by_now && !covers_future_academic_year;
    let mut required_actions = Vec::new();

    if stale_year_coverage {
        let latest = latest_schedule_start_year
            .map_or_else(|| "unknown".to_string(), |year| year.to_string());
        required_actions.push(format!(
            "Annual schedule coverage is stale. Latest detected schedule starts in {latest}, which does not cover current academic year {}-{}.",
            current_academic_start_year,
            current_academic_start_year + 1
        ));
    }
    if missing_future_coverage {
        // CJS interpolation uses null + 1 === 1 for this missing-data case.
        let latest_start =
            latest_schedule_start_year.map_or_else(|| "null".to_string(), |year| year.to_string());
        let latest_end = latest_schedule_start_year.map_or(1, |year| year + 1);
        required_actions.push(format!(
            "Future schedule coverage is missing. Expected at least {}-{} by now, but latest detected schedule is {latest_start}-{latest_end}.",
            next_academic_start_year,
            next_academic_start_year + 1
        ));
    }

    SourceYearCoverageReport {
        generated_at: iso_now(Utc::now()),
        now_utc: iso_now(now),
        current_academic_start_year,
        next_academic_start_year,
        future_coverage_expected_after_utc: iso_now(future_coverage_expected_after),
        future_coverage_expected_by_now,
        latest_schedule_start_year,
        schedule_count: schedule_labels.len(),
        discovered_schedule_labels: schedule_labels,
        covers_current_academic_year,
        covers_future_academic_year,
        stale_year_coverage,
        missing_future_coverage,
        current_catalog_label: Some(materials.current_catalog.label.clone()),
        current_catalog_root_url: Some(materials.current_catalog.root_url.clone()),
        used_snapshot_fallback: materials.used_snapshot_fallback,
        snapshot_fallback_reason: materials.snapshot_fallback_reason.clone(),
        outcome: if required_actions.is_empty() {
            "ok"
        } else {
            "action-required"
        },
        required_actions,
    }
}

fn parse_academic_year_start(label: &str) -> Option<i32> {
    let bytes = label.as_bytes();
    if bytes.len() != 9
        || &bytes[..2] != b"20"
        || !bytes[2..4].iter().all(u8::is_ascii_digit)
        || bytes[4] != b'-'
        || &bytes[5..7] != b"20"
        || !bytes[7..9].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    std::str::from_utf8(&bytes[..4]).ok()?.parse().ok()
}

fn iso_now(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn render_markdown(report: &SourceYearCoverageReport) -> String {
    let mut lines = vec![
        "# Transfer Planner Source Year Coverage".to_string(),
        String::new(),
        format!("Generated: {}", report.generated_at),
        format!("Now (UTC): {}", report.now_utc),
        String::new(),
        format!(
            "- Current academic year baseline: {}-{}",
            report.current_academic_start_year,
            report.current_academic_start_year + 1
        ),
        format!(
            "- Next academic year baseline: {}-{}",
            report.next_academic_start_year,
            report.next_academic_start_year + 1
        ),
        format!(
            "- Future coverage expected after: {}",
            report.future_coverage_expected_after_utc
        ),
        format!(
            "- Future coverage expected by now: {}",
            if report.future_coverage_expected_by_now {
                "yes"
            } else {
                "no"
            }
        ),
        format!(
            "- Latest detected schedule start year: {}",
            report
                .latest_schedule_start_year
                .map_or_else(|| "none".to_string(), |year| year.to_string())
        ),
        format!(
            "- Covers current academic year: {}",
            if report.covers_current_academic_year {
                "yes"
            } else {
                "no"
            }
        ),
        format!(
            "- Covers next academic year: {}",
            if report.covers_future_academic_year {
                "yes"
            } else {
                "no"
            }
        ),
        format!(
            "- Used snapshot fallback: {}",
            if report.used_snapshot_fallback {
                "yes"
            } else {
                "no"
            }
        ),
        format!("- Outcome: {}", report.outcome),
        String::new(),
        "## Discovered Schedule Labels".to_string(),
        String::new(),
    ];
    if report.discovered_schedule_labels.is_empty() {
        lines.push("- None".to_string());
    } else {
        lines.extend(
            report
                .discovered_schedule_labels
                .iter()
                .map(|label| format!("- {label}")),
        );
    }
    lines.extend([
        String::new(),
        "## Required Actions".to_string(),
        String::new(),
    ]);
    if report.required_actions.is_empty() {
        lines.push("- None. Year coverage is up to date.".to_string());
    } else {
        lines.extend(
            report
                .required_actions
                .iter()
                .map(|action| format!("- {action}")),
        );
    }
    lines.push(String::new());
    format!("{}\n", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::grc_public_materials::GrcDiscoveryPages;
    use crate::planner::{GrcCatalogEntry, GrcCurrentCatalog, GrcSchedule};
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_OUTPUT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn temp_source_root() -> std::path::PathBuf {
        loop {
            let sequence = NEXT_OUTPUT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "gator-guide-source-year-coverage-{}-{timestamp}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&root) {
                Ok(()) => return root,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("could not create source-year test directory: {error}"),
            }
        }
    }

    fn materials(labels: &[&str]) -> GrcPublicMaterials {
        GrcPublicMaterials {
            generated_at: "2026-01-01T00:00:00.000Z".to_string(),
            used_snapshot_fallback: false,
            snapshot_fallback_reason: None,
            discovery_pages: GrcDiscoveryPages {
                class_schedules_url: "https://example.edu/schedules".to_string(),
                catalog_archive_url: "https://example.edu/catalogs".to_string(),
            },
            annual_schedules: labels
                .iter()
                .map(|label| GrcSchedule {
                    label: (*label).to_string(),
                    url: format!("https://example.edu/{label}.pdf"),
                    output_path: String::new(),
                    owner_id: String::new(),
                    source_page_url: "https://example.edu/schedules".to_string(),
                })
                .collect(),
            discovered_annual_schedule_count: labels.len(),
            catalog_entries: vec![GrcCatalogEntry {
                label: "2026-2027".to_string(),
                url: "https://example.edu/catalog.zip".to_string(),
                source_page_url: "https://example.edu/catalogs".to_string(),
            }],
            current_catalog: GrcCurrentCatalog {
                label: "2026-2027".to_string(),
                root_url: "https://example.edu/current".to_string(),
                source_page_url: String::new(),
                course_descriptions_url: String::new(),
                course_descriptions_expanded_url: String::new(),
                catoid: None,
                navoid: None,
            },
        }
    }

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn reports_latest_distinct_schedule_year_and_october_future_deadline() {
        let input = materials(&["2024-2025", "2025-2026", "bad", "2025-2026"]);
        let report = build_report(&input, at(2025, 10, 1));
        assert_eq!(report.current_academic_start_year, 2025);
        assert_eq!(report.latest_schedule_start_year, Some(2025));
        assert_eq!(report.schedule_count, 3);
        assert!(report.future_coverage_expected_by_now);
        assert!(report.covers_current_academic_year);
        assert!(!report.covers_future_academic_year);
        assert!(report.missing_future_coverage);
        assert_eq!(report.outcome, "action-required");
        assert_eq!(
            report.required_actions,
            [
                "Future schedule coverage is missing. Expected at least 2026-2027 by now, but latest detected schedule is 2025-2026."
            ]
        );
        assert_eq!(
            report.discovered_schedule_labels,
            ["2024-2025", "2025-2026", "bad"]
        );
    }

    #[test]
    fn writes_json_and_markdown_reports_under_the_planner_report_directory() {
        let root = temp_source_root();
        write_report(&root, &materials(&["bad"])).unwrap();
        let report_path = root.join(".tmp/reports/transfer-planner-source-year-coverage.json");
        let markdown_path = root.join(".tmp/reports/transfer-planner-source-year-coverage.md");
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(report_path).unwrap()).unwrap();
        let markdown = fs::read_to_string(markdown_path).unwrap();
        assert_eq!(report["discoveredScheduleLabels"][0], "bad");
        assert_eq!(report["scheduleCount"], 1);
        assert_eq!(report["outcome"], "action-required");
        assert!(markdown.starts_with("# Transfer Planner Source Year Coverage\n"));
        assert!(markdown.ends_with("\n\n"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn uses_july_academic_boundary_and_keeps_cjs_null_year_interpolation() {
        let input = materials(&[]);
        let before_july = build_report(&input, at(2025, 6, 30));
        assert_eq!(before_july.current_academic_start_year, 2024);
        assert!(before_july.future_coverage_expected_by_now);
        let at_july_boundary = build_report(&input, at(2025, 7, 1));
        assert_eq!(at_july_boundary.current_academic_start_year, 2025);
        assert!(!at_july_boundary.future_coverage_expected_by_now);
        assert_eq!(at_july_boundary.required_actions.len(), 1);
        let after_deadline = build_report(&input, at(2025, 10, 1));
        assert_eq!(after_deadline.current_academic_start_year, 2025);
        assert_eq!(after_deadline.required_actions.len(), 2);
        assert_eq!(
            after_deadline.required_actions[1],
            "Future schedule coverage is missing. Expected at least 2026-2027 by now, but latest detected schedule is null-1."
        );
    }
}
