use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const AUDIT_ROW_COLLECTIONS: &[&str] = &[
    "requirementCoverageRows",
    "sourceScopeAuditRows",
    "generatedSourceSeedAuditRows",
    "generatedShapeAuditRows",
    "requirementShapeAuditRows",
    "electiveApprovedListShapeAuditRows",
    "creditCategoryShapeAuditRows",
    "categoryMappingAuditRows",
    "programApprovedFilterAuditRows",
    "sequencePathwayShapeAuditRows",
    "singleEquivalencyAuditRows",
    "runtimeOptionResolutionAuditRows",
    "runtimeCompoundSequenceAuditRows",
    "requiredCoverageSequenceSuppressionAuditRows",
    "runtimeCompoundSchedulingAuditRows",
    "parserOptionExtractionAuditRows",
    "parserCreditBucketAuditRows",
    "parserCategoryOptionAuditRows",
    "parserPrerequisiteFilterAuditRows",
    "parserSequenceChoiceAuditRows",
    "parserExtractionRegressionRows",
    "sourceScopeRegressionRows",
];

const REQUIRED_RELEASE_REPORT_KEYS: &[&str] = &[
    "sourceBacked",
    "repairQueue",
    "parse",
    "sourcePipeline",
    "sourceGaps",
    "generatedRegistry",
    "mapping",
    "autoRepair",
];

const REPORTS: &[(&str, &str)] = &[
    (
        "sourceBacked",
        "transfer-planner-source-backed-coverage-audit.json",
    ),
    ("repairQueue", "transfer-planner-repair-queue.json"),
    (
        "parse",
        "transfer-planner-requirement-source-parse-report.json",
    ),
    (
        "sourcePipeline",
        "transfer-planner-source-pipeline-validation.json",
    ),
    ("sourceGaps", "transfer-planner-source-gaps.json"),
    (
        "generatedRegistry",
        "transfer-planner-generated-registry-audit.json",
    ),
    ("mapping", "transfer-planner-mapping-audit.json"),
    ("autoRepair", "transfer-planner-auto-repair-plan.json"),
];

const REPORT_MODES: &[(&str, &str, &str)] = &[
    (
        "course-source-coverage",
        "transfer-planner-course-source-coverage.json",
        "Transfer Planner Course Source Coverage",
    ),
    (
        "requirement-source-fidelity",
        "transfer-planner-requirement-source-fidelity.json",
        "Transfer Planner Requirement Source Fidelity",
    ),
    (
        "student-visible-facts",
        "transfer-planner-student-visible-facts.json",
        "Transfer Planner Student-Visible Facts",
    ),
    (
        "release-readiness",
        "transfer-planner-release-readiness.json",
        "Transfer Planner Release Readiness",
    ),
];

const PARSER_SCRIPT: &str = "scripts/planner/parse-transfer-planner-requirement-sources.cjs";
const COMMAND_MAX_BUFFER_BYTES: usize = 1024 * 1024 * 20;

fn inputs_from_value(inputs: &Value) -> HashMap<String, Value> {
    REPORTS
        .iter()
        .map(|(key, _)| {
            (
                (*key).to_string(),
                inputs.get(*key).cloned().unwrap_or(Value::Null),
            )
        })
        .collect()
}

/// Builds the course-source coverage audit from the CJS-compatible input object.
pub fn build_course_source_coverage(inputs: &Value) -> Value {
    build_course_source_coverage_from_map(&inputs_from_value(inputs))
}

/// Builds the requirement-source fidelity audit from the CJS-compatible input object.
pub fn build_requirement_source_fidelity(inputs: &Value) -> Value {
    build_requirement_source_fidelity_from_map(&inputs_from_value(inputs))
}

/// Builds the student-visible facts audit from the CJS-compatible input object.
pub fn build_student_visible_facts(inputs: &Value) -> Value {
    build_student_visible_facts_from_map(&inputs_from_value(inputs))
}

/// Builds release readiness from report inputs. `repository_root` is the GatorGuide root.
pub fn build_release_readiness(
    repository_root: &Path,
    inputs: &Value,
    run_checks: bool,
) -> Result<Value, String> {
    let source_root = repository_root.join("source");
    build_release_readiness_from_map(&source_root, inputs_from_value(inputs), run_checks)
}

/// Runs the Rust port of audit-transfer-planner-publish-readiness.cjs.
/// Returns true when the selected readiness report passes.
pub fn run_publish_readiness(repository_root: &Path, args: &[String]) -> Result<bool, String> {
    let source_root = repository_root.join("source");
    if !source_root.is_dir() {
        return Err(format!(
            "source directory not found: {}",
            source_root.display()
        ));
    }
    ensure_planner_tmp_layout(&source_root)?;

    let mode = get_arg_value(args, "--mode").unwrap_or_else(|| "release-readiness".to_string());
    let (_, output_name, title) = REPORT_MODES
        .iter()
        .copied()
        .find(|(candidate, _, _)| *candidate == mode.as_str())
        .ok_or_else(|| format!("Unknown publish-readiness audit mode: {mode}"))?;
    let inputs = load_inputs(&source_root)?;
    let report = match mode.as_str() {
        "course-source-coverage" => build_course_source_coverage_from_map(&inputs),
        "requirement-source-fidelity" => build_requirement_source_fidelity_from_map(&inputs),
        "student-visible-facts" => build_student_visible_facts_from_map(&inputs),
        "release-readiness" => build_release_readiness_from_map(
            &source_root,
            inputs,
            args.iter().any(|arg| arg == "--run-checks"),
        )?,
        _ => unreachable!("mode was checked against REPORT_MODES"),
    };

    let report_path = source_root.join(".tmp/reports").join(output_name);
    let markdown_path = report_path.with_extension("md");
    let markdown = render_markdown(title, &report);
    super::planner_reporting::write_report_pair(&report_path, &markdown_path, &report, &markdown)?;
    let outcome = report
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    println!("{title} outcome: {outcome}");
    println!("JSON report: {}", report_path.display());
    println!("Markdown report: {}", markdown_path.display());
    Ok(outcome == "passed")
}

fn ensure_planner_tmp_layout(source_root: &Path) -> Result<(), String> {
    for category in [
        "reports",
        "snapshots",
        "error_logs",
        "logs",
        "exports",
        "cache",
        "downloads",
        "screenshots",
        "scratch",
        "qa",
        "builds",
    ] {
        fs::create_dir_all(source_root.join(".tmp").join(category)).map_err(|error| {
            format!(
                "could not create planner temporary directory {}: {error}",
                source_root.join(".tmp").join(category).display()
            )
        })?;
    }
    Ok(())
}

fn load_inputs(source_root: &Path) -> Result<HashMap<String, Value>, String> {
    REPORTS
        .iter()
        .map(|(key, filename)| {
            let path = source_root.join(".tmp/reports").join(filename);
            let value = if path.exists() {
                let bytes = fs::read(&path).map_err(|error| {
                    format!("could not read report {}: {error}", path.display())
                })?;
                serde_json::from_slice(&bytes).map_err(|error| {
                    format!("could not parse report {}: {error}", path.display())
                })?
            } else {
                Value::Null
            };
            Ok(((*key).to_string(), value))
        })
        .collect()
}

fn build_course_source_coverage_from_map(inputs: &HashMap<String, Value>) -> Value {
    let blockers = collect_source_backed_blockers(report(inputs, "sourceBacked"));
    let course_relevant = blockers
        .into_iter()
        .filter(|blocker| {
            matches_any(
                blocker.get("actionableIssueClass"),
                &[
                    "equivalent-grc-course-missing-or-over-selected",
                    "source-course-parsed-not-generated",
                    "generated-row-exists-but-not-visible",
                    "primary-required-course-missing-from-runtime",
                    "support-only-course-scheduled-as-required",
                ],
            )
        })
        .collect::<Vec<_>>();
    let student_visible_unsupported = course_relevant
        .iter()
        .filter(|blocker| blocker.get("studentVisible") == Some(&Value::Bool(true)))
        .cloned()
        .collect::<Vec<_>>();
    let single_equivalency_issues = rows(
        Some(report(inputs, "sourceBacked")),
        "singleEquivalencyAuditRows",
    )
    .into_iter()
    .filter(|row| has_audit_issue(row))
    .cloned()
    .collect::<Vec<_>>();
    let generated_seed_issues = rows(
        Some(report(inputs, "sourceBacked")),
        "generatedSourceSeedAuditRows",
    )
    .into_iter()
    .filter(|row| has_audit_issue(row))
    .cloned()
    .collect::<Vec<_>>();
    let outcome = if !student_visible_unsupported.is_empty()
        || !single_equivalency_issues.is_empty()
        || !generated_seed_issues.is_empty()
    {
        "blocked"
    } else {
        "passed"
    };
    json!({
        "generatedAt": now_iso(),
        "outcome": outcome,
        "summary": {
            "courseRelevantBlockerCount": course_relevant.len(),
            "studentVisibleUnsupportedCourseFactCount": student_visible_unsupported.len(),
            "singleEquivalencyIssueCount": single_equivalency_issues.len(),
            "generatedSourceSeedIssueCount": generated_seed_issues.len(),
            "countsByCampus": count_by(course_relevant.iter().map(|blocker| nullish_value(blocker.get("campusId"), Some(&json!("unknown"))).unwrap_or(Value::Null)).collect()),
            "countsByIssueType": count_by(course_relevant.iter().map(|blocker| blocker.get("issueType").cloned().unwrap_or(Value::Null)).collect()),
        },
        "studentVisibleUnsupported": student_visible_unsupported.into_iter().take(250).collect::<Vec<_>>(),
        "blockers": course_relevant,
    })
}

fn build_requirement_source_fidelity_from_map(inputs: &HashMap<String, Value>) -> Value {
    let blockers = collect_source_backed_blockers(report(inputs, "sourceBacked"));
    let fidelity_blockers = blockers
        .iter()
        .filter(|blocker| {
            matches_any(
                blocker.get("actionableIssueClass"),
                &[
                    "source-role-misclassified",
                    "option-group-collapsed-to-single-course",
                    "credit-bucket-lost-or-flattened",
                    "support-only-course-scheduled-as-required",
                ],
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let parser_collections = blockers
        .iter()
        .filter(|blocker| {
            matches_any(
                blocker.get("auditCollection"),
                &[
                    "parserPrerequisiteFilterAuditRows",
                    "parserOptionExtractionAuditRows",
                    "parserCreditBucketAuditRows",
                    "parserCategoryOptionAuditRows",
                    "parserSequenceChoiceAuditRows",
                    "parserExtractionRegressionRows",
                    "sourceScopeRegressionRows",
                ],
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let parse_report = report(inputs, "parse");
    let parse_failure_rows = first_non_nullish(
        ["owners", "sources", "blocks"]
            .iter()
            .filter_map(|key| parse_report.get(*key)),
    )
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter(|row| {
        row.get("parseSuccess") == Some(&Value::Bool(false))
            || row.get("status").and_then(Value::as_str) == Some("failed")
    })
    .cloned()
    .collect::<Vec<_>>();
    let failed_count = parse_report
        .get("failedCount")
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .unwrap_or(0.0);
    let parse_failure_count = failed_count.max(parse_failure_rows.len() as f64);
    let parse_failure_count_value = if parse_failure_count.fract() == 0.0
        && parse_failure_count >= i64::MIN as f64
        && parse_failure_count <= i64::MAX as f64
    {
        json!(parse_failure_count as i64)
    } else {
        json!(parse_failure_count)
    };
    let outcome = if !fidelity_blockers.is_empty()
        || !parser_collections.is_empty()
        || parse_failure_count > 0.0
    {
        "blocked"
    } else {
        "passed"
    };
    json!({
        "generatedAt": now_iso(),
        "outcome": outcome,
        "summary": {
            "fidelityBlockerCount": fidelity_blockers.len(),
            "parserAuditBlockerCount": parser_collections.len(),
            "parseFailureCount": parse_failure_count_value,
            "countsByIssueType": count_by(fidelity_blockers.iter().map(|blocker| blocker.get("issueType").cloned().unwrap_or(Value::Null)).collect()),
            "countsByCollection": count_by(fidelity_blockers.iter().map(|blocker| blocker.get("auditCollection").cloned().unwrap_or(Value::Null)).collect()),
        },
        "fidelityBlockers": fidelity_blockers,
        "parserAuditBlockers": parser_collections.into_iter().take(500).collect::<Vec<_>>(),
        "parseFailures": parse_failure_rows.into_iter().take(50).collect::<Vec<_>>(),
    })
}

fn build_student_visible_facts_from_map(inputs: &HashMap<String, Value>) -> Value {
    let blockers = collect_source_backed_blockers(report(inputs, "sourceBacked"));
    let visible_blockers = blockers
        .into_iter()
        .filter(|blocker| blocker.get("studentVisible") == Some(&Value::Bool(true)))
        .collect::<Vec<_>>();
    let source_gap_entries = rows(Some(report(inputs, "sourceGaps")), "entries")
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let outcome = if !visible_blockers.is_empty() || !source_gap_entries.is_empty() {
        "blocked"
    } else {
        "passed"
    };
    json!({
        "generatedAt": now_iso(),
        "outcome": outcome,
        "summary": {
            "studentVisibleUnsupportedFactCount": visible_blockers.len(),
            "hiddenSourceGapOwnerCount": source_gap_entries.len(),
            "countsByActionableClass": count_by(visible_blockers.iter().map(|blocker| blocker.get("actionableIssueClass").cloned().unwrap_or(Value::Null)).collect()),
            "countsByIssueType": count_by(visible_blockers.iter().map(|blocker| blocker.get("issueType").cloned().unwrap_or(Value::Null)).collect()),
            "countsByCampus": count_by(visible_blockers.iter().map(|blocker| nullish_value(blocker.get("campusId"), Some(&json!("unknown"))).unwrap_or(Value::Null)).collect()),
        },
        "studentVisibleUnsupportedFacts": visible_blockers,
        "hiddenSourceGaps": source_gap_entries,
    })
}

fn build_release_readiness_from_map(
    source_root: &Path,
    inputs: HashMap<String, Value>,
    run_checks: bool,
) -> Result<Value, String> {
    let mut commands = Vec::new();
    if run_checks {
        commands.extend(run_release_checks(source_root)?);
        let parse_succeeded = commands.iter().all(|command| {
            command.get("status").and_then(Value::as_i64) == Some(0)
                && command.get("timedOut") != Some(&Value::Bool(true))
        });
        if parse_succeeded {
            commands.push(run_npm_command(
                source_root,
                "planner:generate-student-runtime",
                10 * 60 * 1000,
                None,
                &["run", "planner:generate-student-runtime"],
            ));
            commands.push(run_npm_command(
                source_root,
                "planner:build-source-fingerprints",
                5 * 60 * 1000,
                None,
                &["run", "planner:build-source-fingerprints"],
            ));
            commands.push(run_npm_command(
                source_root,
                "planner:build-auto-repair-plan",
                5 * 60 * 1000,
                None,
                &["run", "planner:build-auto-repair-plan"],
            ));
            commands.push(run_npm_command(
                source_root,
                "planner:audit:source-backed-coverage",
                20 * 60 * 1000,
                None,
                &["run", "planner:audit:source-backed-coverage"],
            ));
            commands.push(run_npm_command(
                source_root,
                "planner:validate-source-pipeline",
                5 * 60 * 1000,
                None,
                &["run", "planner:validate-source-pipeline"],
            ));
            commands.push(run_npm_command(
                source_root,
                "planner:repair-queue",
                2 * 60 * 1000,
                None,
                &["run", "planner:repair-queue"],
            ));
            commands.push(run_npm_command(
                source_root,
                "planner:test:parser",
                15 * 60 * 1000,
                None,
                &["run", "planner:test:parser"],
            ));
            commands.push(run_npx_command(
                source_root,
                "npx tsc --noEmit",
                5 * 60 * 1000,
                &["tsc", "--noEmit"],
            ));
        }
    }
    let current_inputs = if run_checks {
        load_inputs(source_root)?
    } else {
        inputs
    };
    let course_coverage = build_course_source_coverage_from_map(&current_inputs);
    let requirement_fidelity = build_requirement_source_fidelity_from_map(&current_inputs);
    let student_visible_facts = build_student_visible_facts_from_map(&current_inputs);
    let source_pipeline_failures =
        get_source_pipeline_failures(report(&current_inputs, "sourcePipeline"));
    let source_backed_status =
        summarize_report_status(report(&current_inputs, "sourceBacked"), "issueCount");
    let generated_registry_status =
        summarize_report_status(report(&current_inputs, "generatedRegistry"), "issueCount");
    let mapping_status = summarize_report_status(report(&current_inputs, "mapping"), "issueCount");
    let repair_queue_status =
        summarize_report_status(report(&current_inputs, "repairQueue"), "issueCount");
    let missing_required_report_keys = get_missing_required_report_keys(&current_inputs);
    let mut blocking_reasons = Vec::new();

    if !missing_required_report_keys.is_empty() {
        blocking_reasons.push(format!(
            "missing required reports: {}",
            missing_required_report_keys.join(", ")
        ));
    }
    if js_greater_than_zero(source_backed_status.get("issueCount")) {
        blocking_reasons.push(format!(
            "coverage blockers: {}",
            js_string_nullish(source_backed_status.get("issueCount"))
        ));
    }
    let student_visible_unsupported_count = student_visible_facts
        .pointer("/summary/studentVisibleUnsupportedFactCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if student_visible_unsupported_count > 0 {
        blocking_reasons.push(format!(
            "student-visible unsupported facts: {student_visible_unsupported_count}"
        ));
    }
    let hidden_source_gap_count = student_visible_facts
        .pointer("/summary/hiddenSourceGapOwnerCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if hidden_source_gap_count > 0 {
        blocking_reasons.push(format!(
            "hidden source-gap owners: {hidden_source_gap_count}"
        ));
    }
    if !source_pipeline_failures.is_empty() {
        blocking_reasons.push(format!(
            "source-pipeline validation failures: {}",
            source_pipeline_failures.len()
        ));
    }
    if course_coverage.get("outcome").and_then(Value::as_str) != Some("passed") {
        blocking_reasons.push("course source coverage audit blocked".to_string());
    }
    if requirement_fidelity.get("outcome").and_then(Value::as_str) != Some("passed") {
        blocking_reasons.push("requirement source fidelity audit blocked".to_string());
    }
    if commands.iter().any(|command| {
        command.get("status").and_then(Value::as_i64) != Some(0)
            || command.get("timedOut") == Some(&Value::Bool(true))
    }) {
        blocking_reasons.push("release command failed or timed out".to_string());
    }

    let source_reports = REPORTS
        .iter()
        .map(|(key, filename)| {
            let path = source_root.join(".tmp/reports").join(filename);
            (
                (*key).to_string(),
                json!({
                    "path": relative_path(source_root, &path),
                    "present": path.exists(),
                }),
            )
        })
        .collect::<Map<String, Value>>();
    let outcome = if blocking_reasons.is_empty() {
        "passed"
    } else {
        "blocked"
    };
    Ok(json!({
        "generatedAt": now_iso(),
        "outcome": outcome,
        "blockingReasons": blocking_reasons,
        "sourceReports": Value::Object(source_reports),
        "missingRequiredReportKeys": missing_required_report_keys,
        "commands": commands,
        "gateStatuses": {
            "sourceBackedCoverage": source_backed_status,
            "generatedRegistry": generated_registry_status,
            "mapping": mapping_status,
            "repairQueue": repair_queue_status,
            "sourcePipeline": {
                "present": js_truthy(Some(report(&current_inputs, "sourcePipeline"))),
                "outcome": if source_pipeline_failures.is_empty() { "passed" } else { "failed" },
                "failureCount": source_pipeline_failures.len(),
            },
            "courseSourceCoverage": course_coverage.get("summary").cloned().unwrap_or(Value::Null),
            "requirementSourceFidelity": requirement_fidelity.get("summary").cloned().unwrap_or(Value::Null),
            "studentVisibleFacts": student_visible_facts.get("summary").cloned().unwrap_or(Value::Null),
        },
        "sourcePipelineFailures": source_pipeline_failures,
        "topStudentVisibleUnsupportedFacts": take_array(student_visible_facts.get("studentVisibleUnsupportedFacts"), 100),
        "topRequirementFidelityBlockers": take_array(requirement_fidelity.get("fidelityBlockers"), 100),
        "topCourseCoverageBlockers": take_array(course_coverage.get("blockers"), 100),
    }))
}

fn run_release_checks(source_root: &Path) -> Result<Vec<Value>, String> {
    let parse_report = source_root
        .join(".tmp/reports")
        .join("transfer-planner-requirement-source-parse-report.json");
    if !parse_report.exists() {
        return Ok(vec![run_npm_command(
            source_root,
            "planner:parse-requirement-sources",
            180 * 60 * 1000,
            None,
            &["run", "planner:parse-requirement-sources"],
        )]);
    }

    let plan_ids = get_parseable_primary_plan_ids(source_root)?;
    let chunk_size =
        positive_integer_env("GATORGUIDE_RELEASE_READINESS_PARSE_CHUNK_PLAN_COUNT", 20);
    let chunks = plan_ids.chunks(chunk_size).collect::<Vec<_>>();
    let mut commands = Vec::with_capacity(chunks.len());
    for (index, chunk) in chunks.iter().enumerate() {
        let chunk_value = chunk.join(",");
        let label = format!(
            "planner:parse-requirement-sources chunk {}/{} ({} plan ids)",
            index + 1,
            chunks.len(),
            chunk.len()
        );
        commands.push(run_node_command(
            source_root,
            &label,
            90 * 60 * 1000,
            &[
                "--max-old-space-size=8192",
                PARSER_SCRIPT,
                "--target-plan-ids",
                chunk_value.as_str(),
            ],
        ));
    }
    Ok(commands)
}

fn get_parseable_primary_plan_ids(source_root: &Path) -> Result<Vec<String>, String> {
    let script = format!(
        "const parser = require('./{PARSER_SCRIPT}'); process.stdout.write(JSON.stringify(parser.getParseablePrimaryEntries().map((entry) => entry.planId)));"
    );
    let output = run_command_output(source_root, "node", &["-e", script.as_str()])?;
    if !output.status.success() {
        let status = output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "terminated by signal".to_string());
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "could not enumerate parser primary plan ids (status {status}): {}",
            stderr.trim()
        ));
    }
    let values: Vec<Value> = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("could not decode parser primary plan ids: {error}"))?;
    let mut seen = Vec::new();
    for value in values {
        let normalized = js_string_nullish(Some(&value))
            .trim_matches(super::requirement_shape::is_js_whitespace)
            .to_string();
        if !normalized.is_empty() && !seen.contains(&normalized) {
            seen.push(normalized);
        }
    }
    Ok(seen)
}

fn run_command_output(
    cwd: &Path,
    command: &str,
    args: &[&str],
) -> Result<std::process::Output, String> {
    let mut process = build_command(cwd, command, args);
    super::requirement_source_fetching::run_command_bounded_with_timeout(
        &mut process,
        COMMAND_MAX_BUFFER_BYTES,
        None,
    )
}

fn run_npm_command(
    cwd: &Path,
    label: &str,
    timeout_ms: u64,
    command_label: Option<&str>,
    args: &[&str],
) -> Value {
    run_reported_command(cwd, "npm", label, timeout_ms, command_label, args)
}

fn run_npx_command(cwd: &Path, label: &str, timeout_ms: u64, args: &[&str]) -> Value {
    run_reported_command(cwd, "npx", label, timeout_ms, None, args)
}

fn run_node_command(cwd: &Path, label: &str, timeout_ms: u64, args: &[&str]) -> Value {
    run_reported_command(cwd, "node", label, timeout_ms, Some(label), args)
}

fn run_reported_command(
    cwd: &Path,
    command: &str,
    _label: &str,
    timeout_ms: u64,
    command_label: Option<&str>,
    args: &[&str],
) -> Value {
    let started_at = now_iso();
    let mut process = build_command(cwd, command, args);
    let result = super::requirement_source_fetching::run_command_bounded_with_timeout_output_and_overflow_kill(
        &mut process,
        COMMAND_MAX_BUFFER_BYTES,
        Some(Duration::from_millis(timeout_ms)),
        true,
    );
    let finished_at = now_iso();
    let (status, signal, timed_out, stdout, stderr) = match result {
        Ok(result) => {
            let timed_out = result.timed_out;
            let output_exceeded = result.output_exceeded;
            let output = result.output;
            (
                if timed_out || output_exceeded {
                    Value::Null
                } else {
                    output
                        .status
                        .code()
                        .map(|code| json!(code))
                        .unwrap_or(Value::Null)
                },
                signal_name(&output.status)
                    .map(|signal| json!(signal))
                    .unwrap_or(Value::Null),
                timed_out,
                decode_bounded_output(&output.stdout),
                decode_bounded_output(&output.stderr),
            )
        }
        Err(error) => (
            Value::Null,
            Value::Null,
            error.starts_with("command timed out after "),
            String::new(),
            String::new(),
        ),
    };
    let command_text = command_label.map(str::to_string).unwrap_or_else(|| {
        std::iter::once(command)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ")
    });
    json!({
        "command": command_text,
        "startedAt": started_at,
        "finishedAt": finished_at,
        "status": status,
        "signal": signal,
        "timedOut": timed_out,
        "stdoutTail": compact_tail_text(&Value::String(stdout), 1200),
        "stderrTail": compact_tail_text(&Value::String(stderr), 1200),
    })
}

#[cfg(unix)]
fn signal_name(status: &std::process::ExitStatus) -> Option<String> {
    let signal = status.signal()?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        return Some(
            match signal {
                1 => "SIGHUP",
                2 => "SIGINT",
                3 => "SIGQUIT",
                4 => "SIGILL",
                5 => "SIGTRAP",
                6 => "SIGABRT",
                7 => "SIGBUS",
                8 => "SIGFPE",
                9 => "SIGKILL",
                10 => "SIGUSR1",
                11 => "SIGSEGV",
                12 => "SIGUSR2",
                13 => "SIGPIPE",
                14 => "SIGALRM",
                15 => "SIGTERM",
                other => return Some(format!("SIG{other}")),
            }
            .to_string(),
        );
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        return Some(
            match signal {
                1 => "SIGHUP",
                2 => "SIGINT",
                3 => "SIGQUIT",
                4 => "SIGILL",
                5 => "SIGTRAP",
                6 => "SIGABRT",
                7 => "SIGEMT",
                8 => "SIGFPE",
                9 => "SIGKILL",
                10 => "SIGBUS",
                11 => "SIGSEGV",
                12 => "SIGSYS",
                13 => "SIGPIPE",
                14 => "SIGALRM",
                15 => "SIGTERM",
                other => return Some(format!("SIG{other}")),
            }
            .to_string(),
        );
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    Some(format!("SIG{signal}"))
}

#[cfg(not(unix))]
fn signal_name(_status: &std::process::ExitStatus) -> Option<String> {
    None
}

fn decode_bounded_output(output: &[u8]) -> String {
    String::from_utf8_lossy(&output[..output.len().min(COMMAND_MAX_BUFFER_BYTES)]).into_owned()
}

fn build_command(cwd: &Path, command: &str, args: &[&str]) -> Command {
    #[cfg(windows)]
    let mut process = if command == "npm" || command == "npx" {
        let mut process = Command::new("cmd");
        process.arg("/c").arg(format!("{command}.cmd"));
        process
    } else {
        Command::new(command)
    };
    #[cfg(not(windows))]
    let mut process = Command::new(command);
    process.args(args).current_dir(cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x0800_0000);
    }
    process
}

fn get_source_pipeline_failures(source_pipeline: &Value) -> Vec<Value> {
    rows(Some(source_pipeline), "checks")
        .into_iter()
        .filter(|check| {
            check.get("status").and_then(Value::as_str) == Some("failed")
                || check.get("passed") == Some(&Value::Bool(false))
        })
        .map(|check| {
            let mut failure = Map::new();
            if let Some(id) = check.get("id") {
                failure.insert("id".to_string(), id.clone());
            }
            if let Some(label) = check.get("label") {
                failure.insert("label".to_string(), label.clone());
            }
            failure.insert(
                "details".to_string(),
                json!(
                    rows(Some(check), "details")
                        .into_iter()
                        .map(|detail| compact_text(Some(detail), 600))
                        .collect::<Vec<_>>()
                ),
            );
            Value::Object(failure)
        })
        .collect()
}

pub fn collect_source_backed_blockers(source_backed: &Value) -> Vec<Value> {
    let mut blockers = Vec::new();
    for collection_name in AUDIT_ROW_COLLECTIONS {
        for (index, row) in rows(Some(source_backed), collection_name)
            .into_iter()
            .enumerate()
        {
            if !has_audit_issue(row) {
                continue;
            }
            let (owner_id, plan_id, pathway_id, campus_id) = get_plan_context(row);
            let issue_type = nullish_value(
                row.get("issueTypeNormalized"),
                Some(&get_row_issue_type(row)),
            );
            let actionable = nullish_value(
                row.get("actionableIssueClass"),
                Some(&get_actionable_issue_class(row)),
            );
            let suspected_layer = nullish_value(row.get("suspectedLayer"), Some(&json!("unknown")));
            let source_url = first_non_nullish(
                [
                    "auditSourceUrl",
                    "sourceUrl",
                    "uwSourceUrl",
                    "primarySourceUrl",
                ]
                .iter()
                .filter_map(|key| row.get(*key)),
            )
            .cloned()
            .unwrap_or(Value::Null);
            let source_role = first_non_nullish(
                ["auditSourceRole", "detectedSourceRole", "sourceRole"]
                    .iter()
                    .filter_map(|key| row.get(*key)),
            )
            .cloned()
            .unwrap_or(Value::Null);
            let requirement = first_non_nullish(
                [
                    "uwRequirementLabel",
                    "requirementTitle",
                    "requirement",
                    "rawLine",
                    "rawText",
                    "copyOnlyDebugText",
                ]
                .iter()
                .filter_map(|key| row.get(*key)),
            );
            let evidence = first_non_nullish(
                ["parsedSourceEvidence", "copyOnlyDebugText", "reason"]
                    .iter()
                    .filter_map(|key| row.get(*key)),
            );
            let mut course_codes = Vec::new();
            for key in [
                "parsedUwCourseCodes",
                "matchedGrcEquivalents",
                "courseCodesExtracted",
            ] {
                spread_iterable(row.get(key), &mut course_codes);
            }
            course_codes.extend(
                ["courseCode", "uwCourse", "grcCourse", "uwEquivalent"]
                    .iter()
                    .filter_map(|key| row.get(*key).cloned()),
            );
            course_codes.retain(|value| js_truthy(Some(value)));
            blockers.push(json!({
                "id": format!("{collection_name}:{index}"),
                "auditCollection": collection_name,
                "issueType": issue_type,
                "actionableIssueClass": actionable,
                "suspectedLayer": suspected_layer,
                "studentVisible": is_student_visible(row),
                "ownerId": owner_id,
                "planId": plan_id,
                "pathwayId": pathway_id,
                "campusId": campus_id,
                "sourceUrl": source_url,
                "sourceRole": source_role,
                "requirement": compact_text(requirement, 260),
                "courseCodes": course_codes,
                "evidence": compact_text(evidence, 420),
            }));
        }
    }
    blockers
}

fn get_plan_context(row: &Value) -> (Value, Value, Value, Value) {
    let existing = row.get("planContext").filter(|value| !value.is_null());
    let owner_id = nullish_value(
        row.get("ownerId"),
        existing.and_then(|value| value.get("ownerId")),
    )
    .unwrap_or_else(|| json!("unknown"));
    let owner_parts = js_string(&owner_id)
        .split(":pathway:")
        .map(str::to_string)
        .collect::<Vec<_>>();
    let owner_plan_id = owner_parts.first().cloned().map(Value::String);
    let owner_pathway_id = owner_parts.get(1).cloned().map(Value::String);
    let plan_id = nullish_value(
        row.get("planId"),
        row.get("majorId")
            .filter(|value| !value.is_null())
            .or_else(|| {
                existing
                    .and_then(|value| value.get("planId"))
                    .filter(|value| !value.is_null())
            })
            .or(owner_plan_id.as_ref()),
    )
    .unwrap_or(Value::Null);
    let pathway_id = nullish_value(
        row.get("pathwayId"),
        existing
            .and_then(|value| value.get("pathwayId"))
            .filter(|value| !value.is_null())
            .or(owner_pathway_id.as_ref()),
    )
    .unwrap_or(Value::Null);
    let campus_id = nullish_value(
        row.get("campusId"),
        existing.and_then(|value| value.get("campusId")),
    )
    .unwrap_or(Value::Null);
    (owner_id, plan_id, pathway_id, campus_id)
}

pub(crate) fn get_row_issue_type(row: &Value) -> Value {
    if let Some(issue_type) = row.get("issueType").filter(|value| js_truthy(Some(value))) {
        return issue_type.clone();
    }
    if let Some(issue) = row.get("issue") {
        if js_truthy(Some(issue)) && issue != &json!("none") {
            return issue.clone();
        }
    }
    Value::Null
}

pub(crate) fn has_audit_issue(row: &Value) -> bool {
    js_truthy(Some(&get_row_issue_type(row)))
}

pub(crate) fn get_actionable_issue_class(row: &Value) -> Value {
    let issue_type = get_row_issue_type(row);
    if !js_truthy(Some(&issue_type)) {
        return Value::Null;
    }
    let issue = issue_type.as_str().unwrap_or_default();
    let class = if issue == "missing-detected-course" {
        if row.get("generatedRuntimeRow") == Some(&Value::Bool(false)) {
            "source-course-parsed-not-generated"
        } else if row.get("visibleInTransferOnlyQuarterPlan") == Some(&Value::Bool(false))
            || row.get("visibleInPlan") == Some(&Value::Bool(false))
        {
            "generated-row-exists-but-not-visible"
        } else {
            "primary-required-course-missing-from-runtime"
        }
    } else if matches_any(
        Some(&issue_type),
        &[
            "generated-row-without-primary-source",
            "hidden-row-as-required",
            "hidden-informational-row-scheduled",
        ],
    ) {
        "visible-row-lacks-source-evidence"
    } else if matches_any(
        Some(&issue_type),
        &[
            "support-source-emitted-required-row",
            "support-source-created-required-row",
            "approved-list-source-created-required-row",
            "approved-course-list-promoted-to-required",
            "elective-list-promoted-to-required",
            "support-source-generated-required-row",
            "support-metadata-became-required",
            "approved-list-generated-required-row",
            "elective-list-generated-required-row",
            "list-promoted-to-required",
            "support-list-scheduled",
            "elective-list-as-required",
            "non-schedulable-source-scheduled",
            "non-schedulable-course-list-scheduled",
            "upper-division-prerequisite-table-scheduled",
        ],
    ) {
        "support-only-course-scheduled-as-required"
    } else if matches_any(
        Some(&issue_type),
        &[
            "missing-credit-bucket",
            "required-count-used-for-credit-bucket",
            "flattened-credit-bucket",
            "credit-bucket-as-count",
        ],
    ) {
        "credit-bucket-lost-or-flattened"
    } else if matches_any(
        Some(&issue_type),
        &[
            "placeholder-atom-scheduled",
            "placeholder-promoted-to-required",
            "false-required-sibling",
            "false-required-promotion",
            "fake-category-course",
            "category-option-invented-course",
        ],
    ) {
        "placeholder-scheduled-as-false-required"
    } else if matches_any(
        Some(&issue_type),
        &[
            "missing-option-group",
            "missed-option-group",
            "option-group-disappears-after-refresh",
            "missing-category-option",
            "flattened-option-group",
            "flattened-option",
            "wrong-shape",
            "missing-list-shape",
            "missed-sequence-choice",
            "flattened-sequence-choice",
            "sequence-choice-flattened",
            "flattened-sequence-paths",
        ],
    ) {
        "option-group-collapsed-to-single-course"
    } else if matches_any(
        Some(&issue_type),
        &[
            "missing-equivalency",
            "fake-equivalency",
            "unsupported-substitution",
            "stale-equivalency",
            "missing-compound-path",
            "partial-compound-path",
            "over-expanded-compound-path",
            "duplicate-compound-count",
            "partial-compound-path-scheduled",
            "missing-compound-component",
            "duplicate-compound-component",
            "over-scheduled-alternatives",
            "unmapped-uw-only",
        ],
    ) {
        "equivalent-grc-course-missing-or-over-selected"
    } else if matches_any(
        Some(&issue_type),
        &[
            "parser-source-scope-violation",
            "source-scope-contamination",
            "unscoped-generated-seed",
            "stale-manual-seed",
            "pathway-leak",
            "path-note-unscoped",
            "prerequisite-table-emitted-requirement",
            "course-list-emitted-requirement",
        ],
    ) {
        "source-role-misclassified"
    } else if issue == "internal-error" {
        "internal-planner-error"
    } else {
        return issue_type;
    };
    Value::String(class.to_string())
}

pub fn is_student_visible(row: &Value) -> bool {
    [
        "visibleInTransferOnlyQuarterPlan",
        "visibleInTransferOnlyPlan",
        "visibleInPlan",
        "scheduled",
    ]
    .iter()
    .any(|key| row.get(*key) == Some(&Value::Bool(true)))
}

fn summarize_report_status(report: &Value, issue_key: &str) -> Value {
    if !js_truthy(Some(report)) {
        return json!({ "present": false, "outcome": "missing", "issueCount": null });
    }
    let summary = report
        .get("summary")
        .filter(|value| !value.is_null())
        .unwrap_or(report);
    let outcome = nullish_value(
        report.get("outcome"),
        summary.get("outcome").filter(|value| !value.is_null()),
    )
    .unwrap_or_else(|| json!("unknown"));
    let issue_count = first_non_nullish(
        [
            summary.get("blockingGateIssueCount"),
            summary.get(issue_key),
            report.get("issueCount"),
            report.get("errorCaseCount"),
        ]
        .into_iter()
        .flatten(),
    )
    .cloned()
    .unwrap_or(Value::Null);
    json!({ "present": true, "outcome": outcome, "issueCount": issue_count })
}

fn get_missing_required_report_keys(inputs: &HashMap<String, Value>) -> Vec<String> {
    REQUIRED_RELEASE_REPORT_KEYS
        .iter()
        .filter(|key| !js_truthy(inputs.get(**key)))
        .map(|key| (*key).to_string())
        .collect()
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn report<'a>(inputs: &'a HashMap<String, Value>, key: &str) -> &'a Value {
    static NULL_VALUE: Value = Value::Null;
    inputs.get(key).unwrap_or(&NULL_VALUE)
}

fn rows<'a>(value: Option<&'a Value>, key: &str) -> &'a [Value] {
    value
        .and_then(|value| value.get(key))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn nullish_value<'a>(first: Option<&'a Value>, second: Option<&'a Value>) -> Option<Value> {
    first
        .filter(|value| !value.is_null())
        .or_else(|| second.filter(|value| !value.is_null()))
        .cloned()
}

fn first_non_nullish<'a>(values: impl IntoIterator<Item = &'a Value>) -> Option<&'a Value> {
    values.into_iter().find(|value| !value.is_null())
}

fn matches_any(value: Option<&Value>, choices: &[&str]) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|value| choices.contains(&value))
}

pub(crate) fn js_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value
            .as_f64()
            .is_some_and(|number| number != 0.0 && !number.is_nan()),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => true,
    }
}

pub(crate) fn js_string_nullish(value: Option<&Value>) -> String {
    js_string_or(value, "")
}

pub(crate) fn js_string_or(value: Option<&Value>, fallback: &str) -> String {
    match value {
        Some(value) if !value.is_null() => js_string(value),
        _ => fallback.to_string(),
    }
}

pub(crate) fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => super::requirement_shape::js_number_to_string(value),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    js_string(value)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn compact_text(value: Option<&Value>, max_length: usize) -> String {
    let normalized = super::requirement_shape::normalize_js_whitespace(&js_string_nullish(value));
    let length = normalized.encode_utf16().count();
    if length <= max_length {
        return normalized;
    }
    format!(
        "{}...",
        slice_utf16(&normalized, 0, max_length.saturating_sub(3))
    )
}

fn compact_tail_text(value: &Value, max_length: usize) -> String {
    let text = super::requirement_shape::normalize_js_whitespace(&js_string_nullish(Some(value)));
    let length = text.encode_utf16().count();
    if length <= max_length {
        return text;
    }
    let start = length.saturating_sub(max_length).saturating_add(3);
    format!("...{}", slice_utf16(&text, start, length))
}

fn slice_utf16(value: &str, start: usize, end: usize) -> String {
    let mut output = String::new();
    let mut cursor = 0usize;
    for character in value.chars() {
        let units = character.len_utf16();
        let next = cursor + units;
        if cursor >= start && next <= end {
            output.push(character);
        }
        cursor = next;
        if cursor >= end {
            break;
        }
    }
    output
}

fn spread_iterable(value: Option<&Value>, output: &mut Vec<Value>) {
    match value {
        Some(Value::Array(values)) => output.extend(values.iter().cloned()),
        Some(Value::String(value)) => {
            output.extend(value.chars().map(|character| json!(character.to_string())))
        }
        _ => {}
    }
}

fn count_by(values: Vec<Value>) -> Vec<Value> {
    let mut counts: Vec<(Value, usize)> = Vec::new();
    for key in values {
        let key = if key.is_null() { json!("unknown") } else { key };
        if key.is_object() || key.is_array() {
            counts.push((key, 1));
            continue;
        }
        if let Some(index) = counts
            .iter()
            .position(|(existing, _)| count_key_equal(existing, &key))
        {
            counts[index].1 += 1;
        } else {
            counts.push((key, 1));
        }
    }
    counts.sort_by(|left, right| {
        right.1.cmp(&left.1).then_with(|| {
            let left_text = js_string(&left.0);
            let right_text = js_string(&right.0);
            left_text
                .to_lowercase()
                .cmp(&right_text.to_lowercase())
                .then_with(|| right_text.cmp(&left_text))
        })
    });
    counts
        .into_iter()
        .map(|(key, count)| json!({ "key": key, "count": count }))
        .collect()
}

fn count_key_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => left.as_f64() == right.as_f64(),
        (Value::Array(_), _)
        | (Value::Object(_), _)
        | (_, Value::Array(_))
        | (_, Value::Object(_)) => false,
        _ => left == right,
    }
}

fn js_greater_than_zero(value: Option<&Value>) -> bool {
    js_to_number(value).is_some_and(|number| number > 0.0)
}

fn js_to_number(value: Option<&Value>) -> Option<f64> {
    match value {
        None | Some(Value::Null) => Some(0.0),
        Some(Value::Bool(false)) => Some(0.0),
        Some(Value::Bool(true)) => Some(1.0),
        Some(Value::Number(value)) => value.as_f64(),
        Some(Value::String(value)) => {
            let trimmed = value.trim_matches(super::requirement_shape::is_js_whitespace);
            if trimmed.is_empty() {
                return Some(0.0);
            }
            if let Some(hex) = trimmed
                .strip_prefix("0x")
                .or_else(|| trimmed.strip_prefix("0X"))
            {
                return u64::from_str_radix(hex, 16)
                    .ok()
                    .map(|number| number as f64);
            }
            if let Some(binary) = trimmed
                .strip_prefix("0b")
                .or_else(|| trimmed.strip_prefix("0B"))
            {
                return u64::from_str_radix(binary, 2)
                    .ok()
                    .map(|number| number as f64);
            }
            if let Some(octal) = trimmed
                .strip_prefix("0o")
                .or_else(|| trimmed.strip_prefix("0O"))
            {
                return u64::from_str_radix(octal, 8)
                    .ok()
                    .map(|number| number as f64);
            }
            trimmed.parse::<f64>().ok()
        }
        Some(Value::Array(values)) => {
            let joined = values
                .iter()
                .map(|value| {
                    if value.is_null() {
                        String::new()
                    } else {
                        js_string(value)
                    }
                })
                .collect::<Vec<_>>()
                .join(",");
            js_to_number(Some(&Value::String(joined)))
        }
        Some(Value::Object(_)) => None,
    }
}

fn positive_integer_env(name: &str, fallback: usize) -> usize {
    let Some(raw) = env::var_os(name) else {
        return fallback;
    };
    let raw = raw.to_string_lossy();
    if raw.is_empty() {
        return fallback;
    }
    let mut chars = raw
        .trim_start_matches(super::requirement_shape::is_js_whitespace)
        .chars()
        .peekable();
    let mut digits = String::new();
    if chars
        .peek()
        .is_some_and(|character| *character == '+' || *character == '-')
    {
        digits.push(chars.next().unwrap_or_default());
    }
    while chars
        .peek()
        .is_some_and(|character| character.is_ascii_digit())
    {
        digits.push(chars.next().unwrap_or_default());
    }
    match digits
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
    {
        Some(value) if value >= usize::MAX as f64 => usize::MAX,
        Some(value) => (value as usize).max(1),
        None => fallback,
    }
}

fn get_arg_value(args: &[String], flag: &str) -> Option<String> {
    let direct_prefix = format!("{flag}=");
    if let Some(value) = args.iter().find_map(|arg| arg.strip_prefix(&direct_prefix)) {
        let value = value.trim_matches(super::requirement_shape::is_js_whitespace);
        return (!value.is_empty()).then(|| value.to_string());
    }
    let index = args.iter().position(|arg| arg == flag)?;
    let value = args.get(index + 1)?;
    if value.starts_with("--") {
        return None;
    }
    let value = value.trim_matches(super::requirement_shape::is_js_whitespace);
    (!value.is_empty()).then(|| value.to_string())
}

fn markdown_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut lines = Vec::with_capacity(rows.len() + 2);
    lines.push(format!(
        "| {} |",
        headers
            .iter()
            .map(|value| markdown_escape(value))
            .collect::<Vec<_>>()
            .join(" | ")
    ));
    lines.push(format!(
        "| {} |",
        headers
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join(" | ")
    ));
    if rows.is_empty() {
        lines.push(format!(
            "| {} |",
            headers.iter().map(|_| "").collect::<Vec<_>>().join(" | ")
        ));
    } else {
        lines.extend(rows.iter().map(|row| {
            format!(
                "| {} |",
                row.iter()
                    .map(|value| markdown_escape(value))
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        }));
    }
    lines.join("\n")
}

fn markdown_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace("\r\n", "<br>")
        .replace('\n', "<br>")
}

fn render_markdown(title: &str, report: &Value) -> String {
    let mut lines = vec![
        format!("# {title}"),
        String::new(),
        format!(
            "Generated: {}",
            js_string_nullish(report.get("generatedAt"))
        ),
        String::new(),
        format!("Outcome: {}", js_string_nullish(report.get("outcome"))),
        String::new(),
    ];
    if let Some(reasons) = report.get("blockingReasons").and_then(Value::as_array) {
        if !reasons.is_empty() {
            lines.push("## Blocking Reasons".to_string());
            lines.push(String::new());
            for reason in reasons {
                lines.push(format!("- {}", js_string_nullish(Some(reason))));
            }
            lines.push(String::new());
        }
    }
    lines.push("## Summary".to_string());
    lines.push(String::new());
    let summary = report
        .get("summary")
        .filter(|value| !value.is_null())
        .or_else(|| report.get("gateStatuses"))
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let summary_entries = object_entries(&summary);
    let summary_rows = summary_entries
        .iter()
        .map(|(key, value)| {
            vec![
                key.clone(),
                if value.is_object() || value.is_array() || value.is_null() {
                    js_json_stringify(value)
                } else {
                    js_string(value)
                },
            ]
        })
        .collect::<Vec<_>>();
    lines.push(markdown_table(&["Metric", "Value"], &summary_rows));

    let examples = first_non_nullish(
        [
            report.get("studentVisibleUnsupported"),
            report.get("studentVisibleUnsupportedFacts"),
            report.get("topStudentVisibleUnsupportedFacts"),
            report.get("fidelityBlockers"),
            report.get("blockers"),
            report.get("topCourseCoverageBlockers"),
        ]
        .into_iter()
        .flatten(),
    )
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
    if !examples.is_empty() {
        lines.push(String::new());
        lines.push("## Examples".to_string());
        lines.push(String::new());
        let example_rows = examples
            .iter()
            .take(50)
            .map(|row| {
                vec![
                    js_string_nullish(row.get("id")),
                    js_string_nullish(row.get("ownerId")),
                    js_string_nullish(row.get("issueType")),
                    js_string_nullish(row.get("actionableIssueClass")),
                    if row.get("studentVisible") == Some(&Value::Bool(true)) {
                        "yes".to_string()
                    } else {
                        "no".to_string()
                    },
                    nullish_value(row.get("sourceUrl"), Some(&json!("not reported")))
                        .map(|value| js_string(&value))
                        .unwrap_or_default(),
                    nullish_value(row.get("requirement"), Some(&json!("not reported")))
                        .map(|value| js_string(&value))
                        .unwrap_or_default(),
                    row.get("courseCodes")
                        .and_then(Value::as_array)
                        .map(|values| {
                            values
                                .iter()
                                .map(|value| {
                                    if value.is_null() {
                                        String::new()
                                    } else {
                                        js_string(value)
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| "not reported".to_string()),
                ]
            })
            .collect::<Vec<_>>();
        lines.push(markdown_table(
            &[
                "ID",
                "Owner",
                "Issue",
                "Class",
                "Visible",
                "Source URL",
                "Requirement",
                "Courses",
            ],
            &example_rows,
        ));
    }

    if let Some(failures) = report
        .get("sourcePipelineFailures")
        .and_then(Value::as_array)
    {
        if !failures.is_empty() {
            lines.push(String::new());
            lines.push("## Source Pipeline Failures".to_string());
            lines.push(String::new());
            let failure_rows = failures
                .iter()
                .map(|failure| {
                    vec![
                        js_string_nullish(failure.get("id")),
                        js_string_nullish(failure.get("label")),
                        failure
                            .get("details")
                            .and_then(Value::as_array)
                            .map(|details| {
                                details
                                    .iter()
                                    .map(js_string)
                                    .collect::<Vec<_>>()
                                    .join(" | ")
                            })
                            .unwrap_or_default(),
                    ]
                })
                .collect::<Vec<_>>();
            lines.push(markdown_table(&["ID", "Label", "Details"], &failure_rows));
        }
    }

    if let Some(commands) = report.get("commands").and_then(Value::as_array) {
        if !commands.is_empty() {
            lines.push(String::new());
            lines.push("## Commands".to_string());
            lines.push(String::new());
            let command_rows = commands
                .iter()
                .map(|command| {
                    let status = first_non_nullish(
                        [command.get("status"), command.get("signal")]
                            .into_iter()
                            .flatten(),
                    )
                    .map(js_string)
                    .unwrap_or_else(|| "unknown".to_string());
                    vec![
                        js_string_nullish(command.get("command")),
                        status,
                        if command.get("timedOut") == Some(&Value::Bool(true)) {
                            "yes".to_string()
                        } else {
                            "no".to_string()
                        },
                        js_string_nullish(command.get("stdoutTail")),
                        js_string_nullish(command.get("stderrTail")),
                    ]
                })
                .collect::<Vec<_>>();
            lines.push(markdown_table(
                &[
                    "Command",
                    "Status",
                    "Timed out",
                    "Stdout tail",
                    "Stderr tail",
                ],
                &command_rows,
            ));
        }
    }
    lines.join("\n")
}

fn object_entries(value: &Value) -> Vec<(String, Value)> {
    match value {
        Value::Object(values) => {
            let mut entries = values
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| {
                match (js_array_index(left), js_array_index(right)) {
                    (Some(left), Some(right)) => left.cmp(&right),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            });
            entries
        }
        Value::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| (index.to_string(), value.clone()))
            .collect(),
        Value::String(value) => value
            .chars()
            .enumerate()
            .map(|(index, character)| (index.to_string(), json!(character.to_string())))
            .collect(),
        _ => Vec::new(),
    }
}

fn js_array_index(key: &str) -> Option<u32> {
    let index = key.parse::<u32>().ok()?;
    (index != u32::MAX && index.to_string() == key).then_some(index)
}

fn js_json_stringify(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => super::requirement_shape::js_number_to_string(value),
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(js_json_stringify)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(_) => format!(
            "{{{}}}",
            object_entries(value)
                .iter()
                .map(|(key, value)| {
                    let key = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{key}:{}", js_json_stringify(value))
                })
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn take_array(value: Option<&Value>, limit: usize) -> Vec<Value> {
    value
        .and_then(Value::as_array)
        .map(|values| values.iter().take(limit).cloned().collect())
        .unwrap_or_default()
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_by_uses_javascript_number_equality_and_unknown_null_keys() {
        let decimal = serde_json::from_str::<Value>("1.0").expect("valid JSON number");
        let counts = count_by(vec![json!(1), decimal, Value::Null, json!("unknown")]);

        assert_eq!(counts.len(), 2);
        assert_eq!(counts[0].get("key").and_then(Value::as_f64), Some(1.0));
        assert_eq!(counts[0].get("count").and_then(Value::as_u64), Some(2));
        assert_eq!(
            counts[1].get("key").and_then(Value::as_str),
            Some("unknown")
        );
        assert_eq!(counts[1].get("count").and_then(Value::as_u64), Some(2));
    }

    #[test]
    fn count_by_does_not_merge_structurally_equal_object_keys() {
        let counts = count_by(vec![json!({"x": 1}), json!({"x": 1})]);

        assert_eq!(counts.len(), 2);
        assert!(
            counts
                .iter()
                .all(|entry| entry.get("count").and_then(Value::as_u64) == Some(1))
        );
    }

    #[test]
    fn markdown_json_stringify_matches_javascript_numeric_key_order_and_number_format() {
        let value = serde_json::from_str::<Value>(r#"{"10":1.0,"2":1,"name":null}"#)
            .expect("valid JSON object");

        assert_eq!(js_json_stringify(&value), r#"{"2":1,"10":1,"name":null}"#);
    }

    #[test]
    fn source_pipeline_failures_omit_undefined_identity_fields() {
        let failures = get_source_pipeline_failures(&json!({
            "checks": [{"status": "failed", "details": []}]
        }));

        assert_eq!(failures.len(), 1);
        assert!(failures[0].get("id").is_none());
        assert!(failures[0].get("label").is_none());
        assert_eq!(
            failures[0]
                .get("details")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn fidelity_report_preserves_fractional_failure_counts() {
        let report = build_requirement_source_fidelity(&json!({
            "parse": {"failedCount": 1.5}
        }));

        assert_eq!(
            report.get("outcome").and_then(Value::as_str),
            Some("blocked")
        );
        assert_eq!(
            report
                .pointer("/summary/parseFailureCount")
                .and_then(Value::as_f64),
            Some(1.5)
        );
    }

    #[test]
    fn blocker_collection_derives_plan_context_and_source_codes() {
        let blockers = collect_source_backed_blockers(&json!({
            "requirementCoverageRows": [{
                "issueType": "missing-equivalency",
                "ownerId": "owner:pathway:track",
                "parsedUwCourseCodes": ["CSE 142"],
                "visibleInPlan": true
            }]
        }));

        assert_eq!(blockers.len(), 1);
        assert_eq!(
            blockers[0].get("planId").and_then(Value::as_str),
            Some("owner")
        );
        assert_eq!(
            blockers[0].get("pathwayId").and_then(Value::as_str),
            Some("track")
        );
        assert_eq!(
            blockers[0]
                .get("actionableIssueClass")
                .and_then(Value::as_str),
            Some("equivalent-grc-course-missing-or-over-selected")
        );
        assert_eq!(
            blockers[0]
                .get("courseCodes")
                .and_then(Value::as_array)
                .and_then(|codes| codes.first())
                .and_then(Value::as_str),
            Some("CSE 142")
        );
    }

    #[test]
    fn student_visibility_requires_literal_true() {
        assert!(is_student_visible(&json!({"scheduled": true})));
        assert!(!is_student_visible(&json!({"scheduled": 1})));
        assert!(!is_student_visible(&json!({"visibleInPlan": "true"})));
    }

    #[test]
    fn compact_text_uses_javascript_whitespace_normalization() {
        let value = json!("\u{00a0}CSE\n 142\u{00a0}");

        assert_eq!(compact_text(Some(&value), 260), "CSE 142");
    }

    #[test]
    fn issue_count_threshold_uses_javascript_number_coercion() {
        let hexadecimal = json!("0x10");
        let single_value_array = json!([2]);
        let invalid = json!("not a number");

        assert!(js_greater_than_zero(Some(&hexadecimal)));
        assert!(js_greater_than_zero(Some(&single_value_array)));
        assert!(!js_greater_than_zero(Some(&invalid)));
    }
}
