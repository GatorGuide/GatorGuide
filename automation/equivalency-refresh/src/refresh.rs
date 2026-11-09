use chrono::{DateTime, SecondsFormat, Utc};
use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::equivalency_catalog;
use crate::equivalency_derived::{
    DerivedRuleSynthesisInput, DerivedRuleSynthesisItem, synthesize_equivalency_rules,
};
use crate::equivalency_output::{EquivalencyRulesOutputInput, render_equivalency_guide_typescript};
use crate::equivalency_report::{
    EquivalencyGuideReportInput, build_equivalency_guide_report,
    render_equivalency_guide_report_markdown,
};
use crate::equivalency_rows::{EquivalencyGuideRow, parse_equivalency_guide_rows_with_titles};

const GUIDE_URL: &str =
    "https://admit.washington.edu/apply/transfer/equivalency-guide/green-river/";
const USER_AGENT: &str = "GatorGuideTransferPlannerEquivalencyIngest/1.0";
const DERIVATION_SPECS: &str = include_str!("derived-specs.json");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DerivationSpecs {
    structured: Vec<DerivedRuleSynthesisItem>,
    chains: Vec<DerivedRuleSynthesisItem>,
}

struct FetchedGuide {
    html: String,
    status: u16,
    final_url: String,
    content_type: Value,
    snapshot_path: PathBuf,
}

pub fn run() -> Result<(), String> {
    let repo_root = repo_root();
    let source_root = repo_root.join("source");
    let fetched = fetch_guide(&source_root)?;
    let course_titles = load_grc_course_titles(&source_root)?;
    let rows = parse_equivalency_guide_rows_with_titles(&fetched.html, &course_titles);
    if rows.is_empty() {
        return Err(format!(
            "The UW Green River guide returned no equivalency rows; refusing to replace generated data. Source: {}",
            fetched.final_url
        ));
    }

    let official_rules = rows_to_rules(&rows)?;
    let derived_rules = build_derived_rules(&official_rules)?;
    let refreshed_at = Utc::now();
    let snapshot_hash = format!("{:x}", Sha256::digest(fetched.html.as_bytes()));
    let snapshot_path = relative_to_source(&fetched.snapshot_path, &source_root)?;
    let report_input = EquivalencyGuideReportInput {
        generated_at: refreshed_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        source_url: GUIDE_URL.to_string(),
        status: fetched.status,
        final_url: fetched.final_url,
        content_type: fetched.content_type,
        snapshot_path,
        snapshot_sha256: snapshot_hash,
        rows: rows
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("could not serialize parsed equivalency rows: {error}"))?,
        official_rules: official_rules.clone(),
        derived_rules: derived_rules.clone(),
    };
    let report = build_equivalency_guide_report(&report_input);
    let markdown = render_equivalency_guide_report_markdown(&report);
    let output_input = EquivalencyRulesOutputInput {
        official_rules,
        derived_rules,
    };

    write_generated_outputs(&source_root, &output_input)?;
    write_reports(&source_root, &report, &markdown)?;
    let last_updated_at =
        update_refresh_date(&source_root.join("constants/locales/en.json"), refreshed_at)?;

    let summary = json!({
        "officialRules": report["officialRuleCount"],
        "derivedRules": report["derivedRuleCount"],
        "generatedRules": report["ruleCount"],
        "departments": report["departmentCount"],
        "directCourseRules": report["countsByType"]["direct-course"].as_u64().unwrap_or(0),
        "sequenceRules": report["countsByType"]["sequence"].as_u64().unwrap_or(0),
        "chainRules": report["countsByType"]["chain-rule"].as_u64().unwrap_or(0),
        "noCreditRules": report["countsByType"]["no-credit"].as_u64().unwrap_or(0),
        "limitedCreditRules": report["countsByType"]["limited-credit"].as_u64().unwrap_or(0),
        "lastUpdatedAt": last_updated_at,
        "output": "constants/transfer-planner-source/equivalency-guide.generated.ts",
        "catalogOutput": "constants/transfer-equivalency-catalog.generated.ts",
        "report": ".tmp/reports/transfer-planner-equivalency-guide-parse.json"
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&summary)
            .map_err(|error| format!("could not serialize refresh summary: {error}"))?
    );
    Ok(())
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("equivalency refresh crate must be under automation/")
        .to_path_buf()
}

fn fetch_guide(source_root: &Path) -> Result<FetchedGuide, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| format!("could not create UW guide HTTP client: {error}"))?;
    let response = match client.get(GUIDE_URL).send() {
        Ok(response) => response,
        Err(error) if error.is_timeout() || error.is_connect() => {
            let first_error = error.to_string();
            std::thread::sleep(Duration::from_millis(500));
            client.get(GUIDE_URL).send().map_err(|retry_error| {
                format!(
                    "failed to fetch {GUIDE_URL} after one retry ({first_error}): {retry_error}"
                )
            })?
        }
        Err(error) => return Err(format!("failed to fetch {GUIDE_URL}: {error}")),
    };
    let status = response.status();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null);
    let body = response
        .bytes()
        .map_err(|error| format!("could not read response from {final_url}: {error}"))?;
    let html = String::from_utf8_lossy(&body).into_owned();

    if status.as_u16() != 200 {
        return Err(format!(
            "failed to fetch {GUIDE_URL}: HTTP {} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("unknown status")
        ));
    }
    if html.trim().is_empty() {
        return Err(format!(
            "failed to fetch {GUIDE_URL}: successful response was empty"
        ));
    }

    let snapshot_dir = source_root
        .join(".tmp")
        .join("snapshots")
        .join("transfer-planner-equivalency-guide-snapshots");
    fs::create_dir_all(&snapshot_dir)
        .map_err(|error| format!("could not create {}: {error}", snapshot_dir.display()))?;
    let snapshot_path = snapshot_dir.join("uw-green-river-equivalency-guide.html");
    fs::write(&snapshot_path, &html)
        .map_err(|error| format!("could not write {}: {error}", snapshot_path.display()))?;

    Ok(FetchedGuide {
        html,
        status: status.as_u16(),
        final_url,
        content_type,
        snapshot_path,
    })
}

fn load_grc_course_titles(source_root: &Path) -> Result<HashMap<String, String>, String> {
    let metadata_dir = source_root.join(
        "constants/transfer-planner-source/course-metadata.generated/entries-by-school-subject",
    );
    if !metadata_dir.is_dir() {
        return Ok(HashMap::new());
    }

    let mut files = fs::read_dir(&metadata_dir)
        .map_err(|error| format!("could not read {}: {error}", metadata_dir.display()))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| format!("could not enumerate {}: {error}", metadata_dir.display()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    files.sort();

    let mut titles = HashMap::new();
    for file in files
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
    {
        let contents = fs::read_to_string(&file)
            .map_err(|error| format!("could not read {}: {error}", file.display()))?;
        let entries: Value = serde_json::from_str(&contents)
            .map_err(|error| format!("could not parse {}: {error}", file.display()))?;
        for entry in entries.as_array().into_iter().flatten() {
            if entry.get("schoolId").and_then(Value::as_str) != Some("grc") {
                continue;
            }
            let Some(code) = entry.get("code").and_then(Value::as_str) else {
                continue;
            };
            let Some(title) = entry.get("title").and_then(Value::as_str) else {
                continue;
            };
            let code = normalize_course_code(code);
            let title = title.trim();
            if !code.is_empty() && !title.is_empty() {
                titles.insert(code, title.to_string());
            }
        }
    }
    Ok(titles)
}

fn normalize_course_code(value: &str) -> String {
    let normalized = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();
    if normalized == "MATH& 254" {
        "MATH& 264".to_string()
    } else {
        normalized
    }
}

fn rows_to_rules(rows: &[EquivalencyGuideRow]) -> Result<Vec<Value>, String> {
    let mut rules = rows
        .iter()
        .map(|row| {
            row.generated_rule
                .as_ref()
                .ok_or_else(|| {
                    format!(
                        "Rust parser omitted generated rule for guide row {}",
                        row.row_number
                    )
                })
                .and_then(|rule| {
                    serde_json::to_value(rule).map_err(|error| {
                        format!(
                            "could not serialize guide row {} rule: {error}",
                            row.row_number
                        )
                    })
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    rules.sort_by(|left, right| rule_id(left).cmp(rule_id(right)));
    Ok(rules)
}

fn build_derived_rules(official_rules: &[Value]) -> Result<Vec<Value>, String> {
    let specs: DerivationSpecs = serde_json::from_str(DERIVATION_SPECS)
        .map_err(|error| format!("could not load Rust equivalency derivation specs: {error}"))?;
    let structured_rules = synthesize_equivalency_rules(&DerivedRuleSynthesisInput {
        official_rules: official_rules.to_vec(),
        structured_rules: Vec::new(),
        items: specs.structured,
    })?;
    let chain_rules = synthesize_equivalency_rules(&DerivedRuleSynthesisInput {
        official_rules: official_rules.to_vec(),
        structured_rules: structured_rules.clone(),
        items: specs.chains,
    })?;
    let mut rules = structured_rules;
    rules.extend(chain_rules);
    rules.sort_by(|left, right| rule_id(left).cmp(rule_id(right)));
    Ok(rules)
}

fn rule_id(rule: &Value) -> &str {
    rule.get("id").and_then(Value::as_str).unwrap_or_default()
}

fn write_generated_outputs(
    source_root: &Path,
    input: &EquivalencyRulesOutputInput,
) -> Result<(), String> {
    let guide_output = render_equivalency_guide_typescript(input)?;
    let catalog_output =
        equivalency_catalog::render_transfer_equivalency_catalog_typescript(input)?;
    let guide_path =
        source_root.join("constants/transfer-planner-source/equivalency-guide.generated.ts");
    let catalog_path = source_root.join("constants/transfer-equivalency-catalog.generated.ts");
    fs::write(&guide_path, guide_output)
        .map_err(|error| format!("could not write {}: {error}", guide_path.display()))?;
    fs::write(&catalog_path, catalog_output)
        .map_err(|error| format!("could not write {}: {error}", catalog_path.display()))?;
    Ok(())
}

fn write_reports(source_root: &Path, report: &Value, markdown: &str) -> Result<(), String> {
    let report_dir = source_root.join(".tmp/reports");
    fs::create_dir_all(&report_dir)
        .map_err(|error| format!("could not create {}: {error}", report_dir.display()))?;
    let json_path = report_dir.join("transfer-planner-equivalency-guide-parse.json");
    let markdown_path = report_dir.join("transfer-planner-equivalency-guide-parse.md");
    let json_output = serde_json::to_string_pretty(report)
        .map_err(|error| format!("could not serialize equivalency report: {error}"))?;
    fs::write(&json_path, format!("{json_output}\n"))
        .map_err(|error| format!("could not write {}: {error}", json_path.display()))?;
    fs::write(&markdown_path, markdown)
        .map_err(|error| format!("could not write {}: {error}", markdown_path.display()))?;
    Ok(())
}

fn relative_to_source(path: &Path, source_root: &Path) -> Result<String, String> {
    path.strip_prefix(source_root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .map_err(|error| {
            format!(
                "{} is not under {}: {error}",
                path.display(),
                source_root.display()
            )
        })
}

fn format_refresh_date(date: DateTime<Utc>) -> String {
    date.format("%B %-d, %Y").to_string()
}

fn replace_refresh_date(description: &str, date: DateTime<Utc>) -> Result<String, String> {
    let marker = Regex::new(r"Last updated at (?:____|[A-Z][a-z]+ [0-9]{1,2}, [0-9]{4})\.")
        .map_err(|error| format!("could not compile refresh-date pattern: {error}"))?;
    let matches = marker.find_iter(description).collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "expected exactly one Equivalency Guide last-updated marker, found {}",
            matches.len()
        ));
    }
    let matched = matches[0];
    Ok(format!(
        "{}Last updated at {}.{}",
        &description[..matched.start()],
        format_refresh_date(date),
        &description[matched.end()..]
    ))
}

fn update_refresh_date(locale_path: &Path, date: DateTime<Utc>) -> Result<String, String> {
    let source = fs::read_to_string(locale_path)
        .map_err(|error| format!("could not read {}: {error}", locale_path.display()))?;
    let locale: Value = serde_json::from_str(&source)
        .map_err(|error| format!("could not parse {}: {error}", locale_path.display()))?;
    let description = locale
        .pointer("/resources/transferCategoryEquivalenciesDescription")
        .and_then(Value::as_str)
        .ok_or_else(|| "English transfer-equivalency description is missing.".to_string())?;
    let updated_description = replace_refresh_date(description, date)?;
    let old_value = serde_json::to_string(description)
        .map_err(|error| format!("could not encode existing locale description: {error}"))?;
    let new_value = serde_json::to_string(&updated_description)
        .map_err(|error| format!("could not encode updated locale description: {error}"))?;
    let key = "\"transferCategoryEquivalenciesDescription\": ";
    let old_field = format!("{key}{old_value}");
    if source.matches(&old_field).count() != 1 {
        return Err(
            "could not uniquely locate the English transfer-equivalency description.".into(),
        );
    }
    let new_field = format!("{key}{new_value}");
    if old_field != new_field {
        fs::write(locale_path, source.replacen(&old_field, &new_field, 1))
            .map_err(|error| format!("could not update {}: {error}", locale_path.display()))?;
    }
    Ok(format_refresh_date(date))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn test_date() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 9, 6, 47, 4).unwrap()
    }

    #[test]
    fn refresh_date_replaces_blank_and_existing_dates_in_utc() {
        assert_eq!(
            replace_refresh_date(
                "Double check using Equivalency Guide. Last updated at ____.",
                test_date()
            )
            .unwrap(),
            "Double check using Equivalency Guide. Last updated at November 9, 2026."
        );
        assert_eq!(
            replace_refresh_date(
                "Double check using Equivalency Guide. Last updated at July 15, 2026.",
                test_date()
            )
            .unwrap(),
            "Double check using Equivalency Guide. Last updated at November 9, 2026."
        );
    }

    #[test]
    fn refresh_date_requires_exactly_one_marker() {
        assert!(replace_refresh_date("No marker.", test_date()).is_err());
        assert!(
            replace_refresh_date("Last updated at ____. Last updated at ____.", test_date())
                .is_err()
        );
    }

    #[test]
    fn structured_and_chain_derivation_matches_the_tracked_rule_artifact() {
        let source_root = repo_root().join("source");
        let generated =
            source_root.join("constants/transfer-planner-source/equivalency-guide.generated.ts");
        let source = fs::read_to_string(&generated).unwrap();
        let official_rules =
            read_typescript_rule_chunks(&source, "TRANSFER_PLANNER_UW_GRC_EQUIVALENCY_GUIDE_RULE")
                .unwrap();
        let expected = read_typescript_rule_chunks(
            &source,
            "TRANSFER_PLANNER_UW_GRC_DERIVED_EQUIVALENCY_RULE",
        )
        .unwrap();
        let actual = build_derived_rules(&official_rules).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn monthly_action_invokes_the_rust_refresh_and_no_deleted_node_entrypoint() {
        let repo_root = repo_root();
        let workflow = fs::read_to_string(
            repo_root.join(".github/workflows/transfer-category-equivalencies-monthly.yml"),
        )
        .unwrap();
        assert!(workflow.contains("cron: \"0 12 1 * *\""));
        assert!(workflow.contains("workflow_dispatch:"));
        assert!(workflow.contains("automation/equivalency-refresh/Cargo.toml"));
        assert!(workflow.contains("refresh-equivalencies"));
        assert!(!workflow.contains("setup-node"));
        assert!(!workflow.contains("npm ci"));
        assert!(!workflow.contains("node --test"));
        assert!(!workflow.contains("parse-transfer-planner-equivalency-guide.cjs"));
        assert!(
            !repo_root
                .join("source/scripts/planner/parse-transfer-planner-equivalency-guide.cjs")
                .exists()
        );
    }

    fn read_typescript_rule_chunks(source: &str, prefix: &str) -> Result<Vec<Value>, String> {
        let declaration = format!("const {prefix}_CHUNK_");
        let mut cursor = 0;
        let mut rules = Vec::new();
        while let Some(relative_start) = source[cursor..].find(&declaration) {
            let declaration_start = cursor + relative_start;
            let assignment = source[declaration_start..]
                .find("= [")
                .ok_or_else(|| format!("missing array initializer for {prefix}"))?;
            let array_start = declaration_start + assignment + 2;
            let array_end = find_json_array_end(source, array_start)
                .ok_or_else(|| format!("unterminated rule chunk for {prefix}"))?;
            let chunk: Vec<Value> = serde_json::from_str(&source[array_start..=array_end])
                .map_err(|error| format!("invalid generated rule chunk for {prefix}: {error}"))?;
            rules.extend(chunk);
            cursor = array_end + 1;
        }
        if rules.is_empty() {
            return Err(format!("no generated rule chunks found for {prefix}"));
        }
        Ok(rules)
    }

    fn find_json_array_end(source: &str, start: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        for (offset, character) in source[start..].char_indices() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == '"' {
                    in_string = false;
                }
                continue;
            }
            match character {
                '"' => in_string = true,
                '[' => depth += 1,
                ']' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(start + offset);
                    }
                }
                _ => {}
            }
        }
        None
    }
}
