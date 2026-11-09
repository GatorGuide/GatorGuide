use chrono::{SecondsFormat, Utc};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use super::publish_readiness::js_string;
use super::requirement_course_codes::{CourseCodeParser, normalize_course_code};
use super::source_backed_coverage::{
    SourceBackedAuditBuildInput, SourceBackedAuditMode, SourceBackedMappingAuditInput,
    SourceBackedOwnerInput, build_source_backed_audit_execution_artifacts_for_input,
    hydrate_source_backed_audit_snapshot_text, parse_source_backed_audit_cli_options,
    render_source_backed_audit_execution_artifacts_for_source_root,
    select_source_backed_audit_mode, write_source_backed_audit_rendered_artifacts,
};

const GRC_EQUIVALENT_COURSE_CODE_PAIRS: &[&[&str]] = &[
    &["MATH& 254", "MATH& 264"],
    &["PHYS 154", "PHYS& 154"],
    &["PHYS 155", "PHYS& 155"],
];

/// Run the source-backed coverage, generated-registry, or mapping audit.
///
/// Planner runtime facts arrive as one JSON object on stdin when `--context-stdin` is set.
/// The Node launcher builds that object from the TypeScript planner; this command scores it,
/// writes the JSON and Markdown reports under `source/.tmp/reports`, and returns the audit exit code.
pub fn run_source_backed_coverage_audit(
    repository_root: &Path,
    args: &[String],
) -> Result<i32, String> {
    if !args.iter().any(|arg| arg == "--context-stdin") {
        return Err(
            "audit-source-backed-coverage requires --context-stdin with the planner context JSON"
                .to_string(),
        );
    }

    let source_root = repository_root.join("source");
    let options = parse_source_backed_audit_cli_options(args);
    let mode =
        select_source_backed_audit_mode(options.mapping_only, options.generated_registry_only);
    let bundle = read_stdin_json()?;
    let generated_at = generated_at_from_bundle(&bundle);
    let execution = match mode {
        SourceBackedAuditMode::Mapping => {
            let mapping = mapping_input_from_value(
                bundle
                    .get("mappingInput")
                    .ok_or_else(|| "mapping audit context is missing mappingInput".to_string())?,
            )?;
            build_source_backed_audit_execution_artifacts_for_input(
                &generated_at,
                options.report_only,
                SourceBackedAuditBuildInput::Mapping(&mapping),
            )
        }
        SourceBackedAuditMode::GeneratedRegistry => {
            let context = prepared_audit_context(&bundle, &source_root, false)?;
            let course_parser = load_course_parser(&source_root)?;
            build_source_backed_audit_execution_artifacts_for_input(
                &generated_at,
                options.report_only,
                SourceBackedAuditBuildInput::GeneratedRegistry {
                    context: &context,
                    course_parser: &course_parser,
                },
            )
        }
        SourceBackedAuditMode::Coverage => {
            let context = prepared_audit_context(&bundle, &source_root, true)?;
            let course_parser = load_course_parser(&source_root)?;
            let owner_input = SourceBackedOwnerInput {
                target_plan_id: context
                    .get("targetPlanId")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                ..SourceBackedOwnerInput::default()
            };
            build_source_backed_audit_execution_artifacts_for_input(
                &generated_at,
                options.report_only,
                SourceBackedAuditBuildInput::Coverage {
                    context: &context,
                    owner_input: &owner_input,
                    course_parser: &course_parser,
                },
            )
        }
    };
    let rendered =
        render_source_backed_audit_execution_artifacts_for_source_root(&execution, &source_root)?;
    write_source_backed_audit_rendered_artifacts(&rendered)?;
    for line in &rendered.summary_lines {
        println!("{line}");
    }
    for line in &rendered.diagnostic_lines {
        eprintln!("{line}");
    }
    Ok(rendered.exit_code)
}

fn prepared_audit_context(
    bundle: &Value,
    source_root: &Path,
    hydrate_snapshots: bool,
) -> Result<Value, String> {
    let mut context = bundle
        .get("context")
        .cloned()
        .ok_or_else(|| "source-backed audit context is missing context".to_string())?;
    if !context.is_object() {
        return Err("source-backed audit context must be a JSON object".to_string());
    }
    ensure_canonical_grc_course_codes(&mut context, source_root)?;
    if hydrate_snapshots {
        context = hydrate_source_backed_audit_snapshot_text(&context, source_root)?;
    }
    Ok(context)
}

fn generated_at_from_bundle(bundle: &Value) -> String {
    bundle
        .get("generatedAt")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn read_stdin_json() -> Result<Value, String> {
    let mut buffer = String::new();
    io::stdin()
        .read_to_string(&mut buffer)
        .map_err(|error| format!("could not read source-backed audit context stdin: {error}"))?;
    if buffer.trim().is_empty() {
        return Err("source-backed audit context stdin was empty".to_string());
    }
    serde_json::from_str(&buffer)
        .map_err(|error| format!("invalid source-backed audit context JSON: {error}"))
}

fn load_course_parser(source_root: &Path) -> Result<CourseCodeParser, String> {
    let metadata_directory = source_root.join(
        "constants/transfer-planner-source/course-metadata.generated/entries-by-school-subject",
    );
    CourseCodeParser::from_metadata_directory(&metadata_directory)
}

fn ensure_canonical_grc_course_codes(
    context: &mut Value,
    source_root: &Path,
) -> Result<(), String> {
    let has_codes = context
        .get("canonicalGrcCourseCodes")
        .and_then(Value::as_array)
        .is_some_and(|codes| !codes.is_empty());
    if has_codes {
        return Ok(());
    }
    let codes = load_canonical_grc_course_codes(source_root)?;
    context
        .as_object_mut()
        .ok_or_else(|| "source-backed audit context must be a JSON object".to_string())?
        .insert(
            "canonicalGrcCourseCodes".to_string(),
            Value::Array(codes.into_iter().map(Value::String).collect()),
        );
    Ok(())
}

pub(crate) fn load_canonical_grc_course_codes(source_root: &Path) -> Result<Vec<String>, String> {
    let path = source_root.join(
        "constants/transfer-planner-source/student-runtime.generated/compact-course-registry.generated.json",
    );
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "could not read compact course registry {}: {error}",
            path.display()
        )
    })?;
    let rows: Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "invalid compact course registry {}: {error}",
            path.display()
        )
    })?;
    let rows = rows.as_array().ok_or_else(|| {
        format!(
            "compact course registry {} must contain a JSON array",
            path.display()
        )
    })?;
    let mut codes = BTreeSet::new();
    for row in rows {
        if row.get("schoolId").and_then(Value::as_str) != Some("grc") {
            continue;
        }
        let Some(code) = row.get("code").and_then(Value::as_str) else {
            continue;
        };
        let normalized = normalize_course_code(code);
        if !normalized.is_empty() {
            codes.insert(normalized);
        }
    }
    for pair in GRC_EQUIVALENT_COURSE_CODE_PAIRS {
        for code in *pair {
            let normalized = normalize_course_code(code);
            if !normalized.is_empty() {
                codes.insert(normalized);
            }
        }
    }
    Ok(codes.into_iter().collect())
}

pub(crate) fn mapping_input_from_value(
    value: &Value,
) -> Result<SourceBackedMappingAuditInput, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "mappingInput must be a JSON object".to_string())?;
    let field = |name: &str| object.get(name);
    Ok(SourceBackedMappingAuditInput {
        known_single_equivalency_rows: json_array(field("knownSingleEquivalencyRows")),
        equivalency_rules: json_array(field("equivalencyRules")),
        runtime_plan_exists: json_bool(field("runtimePlanExists")),
        runtime_compound_audit_rows: json_array(field("runtimeCompoundAuditRows")),
        aeronautics_nsc_audit_row: json_optional(field("aeronauticsNscAuditRow")),
        computer_engineering_nsc_audit_row: json_optional(field("computerEngineeringNscAuditRow")),
        computer_science_nsc_audit_row: json_optional(field("computerScienceNscAuditRow")),
        computer_engineering_filter_rows: json_array(field("computerEngineeringFilterRows")),
        computer_engineering_source_blocks: json_array(field("computerEngineeringSourceBlocks")),
        computer_science_filter_rows: json_array(field("computerScienceFilterRows")),
        computer_science_source_blocks: json_array(field("computerScienceSourceBlocks")),
        environmental_plan_exists: json_bool(field("environmentalPlanExists")),
        environmental_cee347_coverage_row: json_optional(field("environmentalCee347CoverageRow")),
        informatics_plan_exists: json_bool(field("informaticsPlanExists")),
        informatics_suspicious_visible_labels: json_strings(field(
            "informaticsSuspiciousVisibleLabels",
        )),
    })
}

fn json_array(value: Option<&Value>) -> Vec<Value> {
    value.and_then(Value::as_array).cloned().unwrap_or_default()
}

fn json_bool(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(false)
}

fn json_optional(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| !value.is_null()).cloned()
}

fn json_strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter(|value| !value.is_null())
                .map(js_string)
                .filter(|value| !value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_context_flag_is_rejected_before_stdin() {
        let error = run_source_backed_coverage_audit(Path::new("."), &[])
            .expect_err("missing context flag");
        assert!(error.contains("--context-stdin"));
    }

    #[test]
    fn mapping_input_reads_planner_audit_fields() {
        let input = mapping_input_from_value(&json!({
            "knownSingleEquivalencyRows": [{"grcCourse": "CHEM& 161"}],
            "equivalencyRules": [],
            "runtimePlanExists": true,
            "runtimeCompoundAuditRows": [{"uwCourse": "CHEM 152"}],
            "aeronauticsNscAuditRow": null,
            "computerEngineeringNscAuditRow": {"requirement": "Natural Science"},
            "computerScienceNscAuditRow": null,
            "computerEngineeringFilterRows": [],
            "computerEngineeringSourceBlocks": [],
            "computerScienceFilterRows": [],
            "computerScienceSourceBlocks": [],
            "environmentalPlanExists": false,
            "environmentalCee347CoverageRow": null,
            "informaticsPlanExists": true,
            "informaticsSuspiciousVisibleLabels": ["INFO 340"]
        }))
        .expect("mapping input");
        assert!(input.runtime_plan_exists);
        assert!(input.aeronautics_nsc_audit_row.is_none());
        assert_eq!(input.runtime_compound_audit_rows.len(), 1);
        assert_eq!(
            input.informatics_suspicious_visible_labels,
            vec!["INFO 340".to_string()]
        );
    }

    #[test]
    fn canonical_grc_codes_include_equivalent_aliases() {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../source");
        let codes = load_canonical_grc_course_codes(&source_root).expect("canonical codes");
        assert!(codes.iter().any(|code| code == "CHEM& 161"));
        assert!(codes.iter().any(|code| code == "PHYS& 154"));
        assert!(codes.iter().any(|code| code == "PHYS 154"));
        assert!(codes.iter().any(|code| code == "MATH& 264"));
    }
}
