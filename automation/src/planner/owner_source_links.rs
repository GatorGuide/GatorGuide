use super::{
    RequirementCourseCodeParser, RequirementSourceManifestContext, extract_html_anchor_candidates,
    select_parseable_manifest_entries,
};
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

const MODEL_CONTEXT_BRIDGE: &str = r#"
require("ts-node").register({
  skipProject: true,
  transpileOnly: true,
  compilerOptions: { module: "CommonJS", moduleResolution: "node" },
});
const registry = require("./constants/transfer-planner-source/registry");
const bootstrap = require("./constants/transfer-planner-source/bootstrap.generated");
const bootstrapPlans = (bootstrap.TRANSFER_PLANNER_BOOTSTRAP_ALL_MAJOR_PLANS ?? []).map((plan) => ({
  id: plan.id,
  title: plan.title,
  pathways: plan.pathways ?? [],
}));
process.stdout.write(JSON.stringify({
  manifestEntries: registry.TRANSFER_PLANNER_MANIFEST_REGISTRY ?? [],
  degreeMapBlocks: registry.TRANSFER_PLANNER_DEGREE_MAP_BLOCK_REGISTRY ?? [],
  bootstrapPlans,
}));
"#;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceModelContextData {
    #[serde(default)]
    manifest_entries: Vec<Value>,
    #[serde(default)]
    degree_map_blocks: Vec<Value>,
    #[serde(default)]
    bootstrap_plans: Vec<Value>,
}

#[derive(Debug)]
struct ProbeInput {
    owner_id: Option<String>,
    target_plan_id: Option<String>,
    pathway_id: Option<String>,
    source_url: Option<String>,
    html_file: Option<String>,
    expect_document_url_contains: Vec<String>,
    expect_document_label_contains: Vec<String>,
    expect_min_document_candidates: Option<String>,
    expect_title_acronym_match: bool,
    timeout_ms: f64,
}

pub fn run_probe(repository_root: &Path, args: &[String]) -> Result<bool, String> {
    if has_arg(args, "--help") || has_arg(args, "-h") {
        print_usage();
        return Ok(true);
    }

    let input = parse_args(args);
    if input.target_plan_id.is_none() && input.owner_id.is_none() {
        print_usage();
        return Ok(false);
    }
    if !input.timeout_ms.is_finite() || input.timeout_ms <= 0.0 {
        return Err(format!(
            "Invalid --timeout-ms value: {}",
            js_number_to_string(input.timeout_ms)
        ));
    }

    let source_root = repository_root.join("source");
    let model = load_source_model_context(&source_root)?;
    let course_parser = RequirementCourseCodeParser::from_metadata_directory(&source_root.join(
        "constants/transfer-planner-source/course-metadata.generated/entries-by-school-subject",
    ))?;
    let context = RequirementSourceManifestContext::new(
        model.manifest_entries.clone(),
        &model.bootstrap_plans,
        model.degree_map_blocks,
    );
    let target_plan_ids = input.target_plan_id.clone().into_iter().collect::<Vec<_>>();
    let selected_entries = select_parseable_manifest_entries(
        &model.manifest_entries,
        (!target_plan_ids.is_empty()).then_some(target_plan_ids.as_slice()),
        context.bootstrap_plan_index().plan_titles_by_id(),
        context.primary_major_titles(),
        &course_parser,
    );
    let entry = select_entry(&selected_entries, &input)?;
    let source_url_value = input
        .source_url
        .clone()
        .map(Value::String)
        .or_else(|| entry.get("url").cloned());
    let source_url = source_url_value
        .as_ref()
        .map(js_string)
        .unwrap_or_else(|| "undefined".to_string());
    let html = read_html(&source_root, &input, &source_url)?;
    let links = extract_html_anchor_candidates(&html);
    let mut probe_entry = entry.clone();
    if let (Some(source_url), Some(entry)) = (source_url_value, probe_entry.as_object_mut()) {
        entry.insert("url".to_string(), source_url);
    }
    let candidates =
        context.extract_supplemental_document_link_candidates(&probe_entry, &links, &course_parser);

    let mut failures = Vec::new();
    if let Some(minimum_text) = input.expect_min_document_candidates.as_deref() {
        let minimum = js_parse_int_10(minimum_text).unwrap_or(f64::NAN);
        if !minimum.is_finite() {
            return Err(format!(
                "Invalid --expect-min-document-candidates value: {minimum_text}"
            ));
        }
        if (candidates.len() as f64) < minimum {
            failures.push(format!(
                "expected at least {} document candidate(s), found {}",
                js_number_to_string(minimum),
                candidates.len()
            ));
        }
    }

    for expected in &input.expect_document_url_contains {
        if !candidates
            .iter()
            .any(|candidate| candidate_matches_text(candidate, "url", expected))
        {
            failures.push(format!("no document candidate URL contains \"{expected}\""));
        }
    }
    for expected in &input.expect_document_label_contains {
        if !candidates
            .iter()
            .any(|candidate| candidate_matches_text(candidate, "label", expected))
        {
            failures.push(format!(
                "no document candidate label contains \"{expected}\""
            ));
        }
    }
    if input.expect_title_acronym_match
        && !candidates
            .iter()
            .any(|candidate| candidate.get("titleAcronymMatch") == Some(&Value::Bool(true)))
    {
        failures.push("no document candidate had titleAcronymMatch=true".to_string());
    }

    println!("Owner: {}", js_string_direct(entry.get("ownerId")));
    println!(
        "Title: {}",
        entry
            .get("ownerTitle")
            .filter(|value| !value.is_null())
            .map(js_string)
            .unwrap_or_else(|| "n/a".to_string())
    );
    println!("Source: {source_url}");
    println!("Document candidates: {}", candidates.len());
    for candidate in &candidates {
        println!(
            "- {} | {} | score={} | exactTitle={} | acronym={}",
            js_string_direct(candidate.get("label")),
            js_string_direct(candidate.get("url")),
            js_string_direct(candidate.get("score")),
            js_boolean(candidate.get("exactTitleMatch")),
            js_boolean(candidate.get("titleAcronymMatch")),
        );
    }

    if !failures.is_empty() {
        eprintln!("Source link probe failed:");
        for failure in failures {
            eprintln!("- {failure}");
        }
        return Ok(false);
    }

    println!("Source link probe passed.");
    Ok(true)
}

fn parse_args(args: &[String]) -> ProbeInput {
    let timeout_text =
        get_arg_value(args, &["--timeout-ms"]).unwrap_or_else(|| "30000".to_string());
    ProbeInput {
        owner_id: get_arg_value(args, &["--owner-id"]),
        target_plan_id: get_arg_value(args, &["--target-plan-id", "--plan-id", "--target"]),
        pathway_id: get_arg_value(args, &["--pathway-id"]),
        source_url: get_arg_value(args, &["--source-url"]),
        html_file: get_arg_value(args, &["--html-file"]),
        expect_document_url_contains: split_values(get_arg_values(
            args,
            &["--expect-document-url-contains"],
        )),
        expect_document_label_contains: split_values(get_arg_values(
            args,
            &["--expect-document-label-contains"],
        )),
        expect_min_document_candidates: get_arg_value(args, &["--expect-min-document-candidates"]),
        expect_title_acronym_match: has_arg(args, "--expect-title-acronym-match"),
        timeout_ms: js_parse_int_10(&timeout_text).unwrap_or(f64::NAN),
    }
}

fn select_entry(entries: &[Value], input: &ProbeInput) -> Result<Value, String> {
    let mut candidates = entries
        .iter()
        .filter(|entry| {
            input.owner_id.as_ref().is_none_or(|owner_id| {
                entry.get("ownerId").and_then(Value::as_str) == Some(owner_id)
            })
        })
        .filter(|entry| {
            input
                .pathway_id
                .as_ref()
                .is_none_or(|pathway_id| js_string_nullish(entry.get("pathwayId")) == *pathway_id)
        })
        .cloned()
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        return Err(format!(
            "No parseable source entry matched {}.",
            input
                .owner_id
                .as_deref()
                .or(input.target_plan_id.as_deref())
                .unwrap_or("(missing target)")
        ));
    }

    let direct_major = candidates
        .iter()
        .filter(|entry| {
            input.target_plan_id.as_ref().is_some_and(|plan_id| {
                entry.get("ownerId").and_then(Value::as_str) == Some(plan_id)
                    && entry.get("pathwayId").is_none_or(Value::is_null)
            })
        })
        .collect::<Vec<_>>();
    if direct_major.len() == 1 {
        return Ok(direct_major[0].clone());
    }
    if candidates.len() == 1 {
        return Ok(candidates.remove(0));
    }

    let labels = candidates
        .iter()
        .take(12)
        .map(|entry| {
            let pathway = entry
                .get("pathwayId")
                .filter(|value| js_truthy(value))
                .map(|value| format!(" ({})", js_string(value)))
                .unwrap_or_default();
            format!("- {}{pathway}", js_string_direct(entry.get("ownerId")))
        })
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!(
        "Multiple entries matched. Pass --owner-id or --pathway-id.\n{labels}"
    ))
}

fn load_source_model_context(source_root: &Path) -> Result<SourceModelContextData, String> {
    let output = Command::new("node")
        .arg("-e")
        .arg(MODEL_CONTEXT_BRIDGE)
        .current_dir(source_root)
        .output()
        .map_err(|error| {
            format!("could not start Node.js to load planner source models: {error}")
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("planner source-model loader exited with {}", output.status)
        } else {
            format!("planner source-model loader failed: {stderr}")
        });
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("planner source-model loader returned invalid JSON: {error}"))
}

fn read_html(source_root: &Path, input: &ProbeInput, source_url: &str) -> Result<String, String> {
    if let Some(html_file) = input.html_file.as_deref() {
        let path = PathBuf::from(html_file);
        let path = if path.is_absolute() {
            path
        } else {
            source_root.join(path)
        };
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("could not read HTML file {}: {error}", path.display()))?;
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }

    let timeout_ms = if input.timeout_ms > i32::MAX as f64 {
        1
    } else {
        input.timeout_ms as u64
    };
    let timeout = Duration::from_millis(timeout_ms);
    let client = Client::builder()
        .redirect(Policy::limited(10))
        .user_agent("GatorGuide planner source probe")
        .build()
        .map_err(|error| format!("could not create source probe HTTP client: {error}"))?;
    let retry_delay = Duration::from_millis(500);
    for attempt in 0..=1 {
        let response = client.get(source_url).timeout(timeout).send();
        let response = match response {
            Ok(response) => response,
            Err(error) if attempt == 0 && is_retryable_network_error(&error) => {
                thread::sleep(retry_delay);
                continue;
            }
            Err(error) => return Err(network_error_message(source_url, error.is_timeout())),
        };
        let status = response.status();
        let body = response.text().unwrap_or_default();
        if status.is_success() {
            return Ok(body);
        }
        if attempt == 0 && is_retryable_http_status(status.as_u16()) {
            thread::sleep(retry_delay);
            continue;
        }
        let status_text = status.canonical_reason().unwrap_or("");
        let body_text = sanitize_fetch_body(&body);
        return Err(format!(
            "Probe transfer planner owner source failed for {source_url}: {} {}{}",
            status.as_u16(),
            status_text,
            if body_text.is_empty() {
                String::new()
            } else {
                format!(" {body_text}")
            }
        ));
    }
    Err(network_error_message(source_url, false))
}

fn is_retryable_http_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

fn is_retryable_network_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

fn network_error_message(source_url: &str, timed_out: bool) -> String {
    if timed_out {
        format!("Probe transfer planner owner source timed out for {source_url}.")
    } else {
        format!("Probe transfer planner owner source failed for {source_url}.")
    }
}

fn sanitize_fetch_body(body: &str) -> String {
    let raw = body.trim_matches(is_javascript_whitespace);
    if raw.is_empty() {
        return String::new();
    }
    let lower = raw.to_ascii_lowercase();
    if lower.contains("<!doctype html")
        || lower
            .find("<html")
            .and_then(|index| lower.as_bytes().get(index + 5))
            .is_some_and(|character| character.is_ascii_whitespace() || *character == b'>')
    {
        return "[html-error-body]".to_string();
    }
    let prefix = raw.encode_utf16().take(500).collect::<Vec<_>>();
    String::from_utf16_lossy(&prefix)
}

fn get_arg_value(args: &[String], flags: &[&str]) -> Option<String> {
    for flag in flags {
        let prefix = format!("{flag}=");
        if let Some(value) = args.iter().find_map(|arg| arg.strip_prefix(&prefix)) {
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
        if let Some(index) = args.iter().position(|arg| arg == flag) {
            let Some(value) = args.get(index + 1).filter(|value| !value.starts_with("--")) else {
                return None;
            };
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
}

fn get_arg_values(args: &[String], flags: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if let Some((flag, value)) = flags.iter().find_map(|flag| {
            arg.strip_prefix(&format!("{flag}="))
                .map(|value| (*flag, value))
        }) {
            let _ = flag;
            let value = value.trim();
            if !value.is_empty() {
                values.push(value.to_string());
            }
        } else if flags.iter().any(|flag| arg == flag) {
            if let Some(value) = args.get(index + 1).filter(|value| !value.starts_with("--")) {
                let value = value.trim();
                if !value.is_empty() {
                    values.push(value.to_string());
                }
                index += 1;
            }
        }
        index += 1;
    }
    values
}

fn has_arg(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn split_values(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn candidate_matches_text(candidate: &Value, field: &str, expected: &str) -> bool {
    js_string_nullish(candidate.get(field))
        .to_lowercase()
        .contains(&expected.to_lowercase())
}

fn js_parse_int_10(value: &str) -> Option<f64> {
    let value = value.trim_start_matches(is_javascript_whitespace);
    let (negative, unsigned) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let digit_count = unsigned.bytes().take_while(u8::is_ascii_digit).count();
    if digit_count == 0 {
        return None;
    }
    let magnitude = unsigned[..digit_count].parse::<f64>().ok()?;
    Some(if negative { -magnitude } else { magnitude })
}

fn is_javascript_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000d}'
            | '\u{0020}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

fn js_number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value == f64::INFINITY {
        return "Infinity".to_string();
    }
    if value == f64::NEG_INFINITY {
        return "-Infinity".to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    if value.fract() == 0.0 && value.abs() < 1e21 {
        return format!("{value:.0}");
    }
    value.to_string()
}

fn js_string_direct(value: Option<&Value>) -> String {
    value
        .map(js_string)
        .unwrap_or_else(|| "undefined".to_string())
}

fn js_string_nullish(value: Option<&Value>) -> String {
    value
        .filter(|value| !value.is_null())
        .map(js_string)
        .unwrap_or_default()
}

fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
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

fn js_boolean(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

fn js_truthy(value: &Value) -> bool {
    js_boolean(Some(value))
}

fn print_usage() {
    println!(
        "Usage:\n  cargo run --manifest-path automation/Cargo.toml -- probe-owner-source-links --target-plan-id <plan-id> [checks]\n\nChecks:\n  --owner-id <owner-id>                              Select a specific owner instead of a plan.\n  --pathway-id <pathway-id>                          Select a pathway owner under the target plan.\n  --source-url <url>                                 Probe an explicit source URL for the selected owner.\n  --html-file <path>                                 Use local HTML instead of fetching the source URL.\n  --expect-document-url-contains B-EE-Curriculum     Require a recovered document candidate URL.\n  --expect-document-label-contains \"EE degree\"       Require a recovered document candidate label.\n  --expect-min-document-candidates 1                 Require at least N document candidates.\n  --expect-title-acronym-match                       Require at least one candidate matched by owner acronym.\n  --timeout-ms 30000                                 Fetch timeout.\n\nExamples:\n  cargo run --manifest-path automation/Cargo.toml -- probe-owner-source-links --target-plan-id uw-bothell-electrical-engineering --expect-document-url-contains B-EE-Curriculum --expect-title-acronym-match\n  cargo run --manifest-path automation/Cargo.toml -- probe-owner-source-links --target-plan-id uw-bothell-electrical-engineering --html-file .tmp/source.html --expect-document-url-contains B-EE-Curriculum"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(owner_id: Option<&str>, target_plan_id: Option<&str>) -> ProbeInput {
        ProbeInput {
            owner_id: owner_id.map(str::to_string),
            target_plan_id: target_plan_id.map(str::to_string),
            pathway_id: None,
            source_url: None,
            html_file: None,
            expect_document_url_contains: Vec::new(),
            expect_document_label_contains: Vec::new(),
            expect_min_document_candidates: None,
            expect_title_acronym_match: false,
            timeout_ms: 30_000.0,
        }
    }

    #[test]
    fn entry_selection_prefers_a_unique_direct_major() {
        let entries = vec![
            json!({"ownerId":"plan","pathwayId":"path-a"}),
            json!({"ownerId":"plan","pathwayId":null}),
        ];
        let selected = select_entry(&entries, &input(None, Some("plan"))).unwrap();
        assert_eq!(selected["pathwayId"], Value::Null);
    }

    #[test]
    fn entry_selection_requires_disambiguation_for_multiple_pathways() {
        let entries = vec![
            json!({"ownerId":"plan:pathway:a","pathwayId":"a"}),
            json!({"ownerId":"plan:pathway:b","pathwayId":"b"}),
        ];
        let error = select_entry(&entries, &input(None, Some("plan"))).unwrap_err();
        assert!(error.contains("Pass --owner-id or --pathway-id"));
    }

    #[test]
    fn repeated_and_comma_separated_expectations_are_preserved_in_order() {
        let args = vec![
            "--expect-document-url-contains=alpha,beta".to_string(),
            "--expect-document-url-contains".to_string(),
            "gamma".to_string(),
        ];
        assert_eq!(
            split_values(get_arg_values(&args, &["--expect-document-url-contains"])),
            vec!["alpha", "beta", "gamma"]
        );
    }

    #[test]
    fn fetch_error_bodies_match_the_javascript_sanitizer_contract() {
        assert_eq!(sanitize_fetch_body("\u{feff}  \n "), "");
        assert_eq!(
            sanitize_fetch_body("<!DOCTYPE html><html>nope</html>"),
            "[html-error-body]"
        );
        assert_eq!(
            sanitize_fetch_body(&"😀".repeat(251))
                .encode_utf16()
                .count(),
            500
        );
        assert_eq!(sanitize_fetch_body(&"x".repeat(501)), "x".repeat(500));
    }

    #[test]
    fn js_integer_parsing_matches_decimal_prefix_behavior() {
        assert_eq!(js_parse_int_10("  +30ms"), Some(30.0));
        assert_eq!(js_parse_int_10("0x10"), Some(0.0));
        assert_eq!(js_parse_int_10("\u{feff}-2"), Some(-2.0));
        assert_eq!(js_parse_int_10("nope"), None);
    }
}
