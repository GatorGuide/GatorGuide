use super::requirement_course_codes::CourseCodeParser;
use super::requirement_docx::{get_linked_document_parser_type, is_linked_document_source_url};
use super::requirement_snapshots::source_url_without_hash;
use regex::Regex;
use super::requirement_plan_matching::manifest_entry_matches_specialized_plan_title;
use super::requirement_role_classifier::classify_requirement_source_role;
use super::requirement_scope::{RequirementSourceRoleStatus, get_requirement_source_role_status};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::OnceLock;

const ADMISSION_PREPARATION_SOURCE_ROLES: &[&str] = &[
    "admission-prerequisite-source",
    "admissions-preparation",
];

pub fn get_parser_recovery_candidate_parser_type(
    entry: &Value,
    candidate_url: &str,
    candidate_label: &str,
) -> String {
    if is_linked_document_source_url(candidate_url) {
        return get_linked_document_parser_type(candidate_url, candidate_label).to_string();
    }
    if candidate_curriculum_url_pattern().is_match(candidate_url) {
        return "html-curriculum-page".to_string();
    }
    if candidate_degree_requirements_url_pattern().is_match(candidate_url) {
        return "html-degree-page".to_string();
    }
    match entry.get("parserType").and_then(Value::as_str) {
        Some("html-degree-page" | "html-curriculum-page" | "html-overview-page" | "generic-html") => {
            entry.get("parserType").and_then(Value::as_str).unwrap_or_default().to_string()
        }
        _ => "html-degree-page".to_string(),
    }
}

pub fn build_parser_recovery_candidate_entry(entry: &Value, candidate: &Value) -> Value {
    let candidate_url = candidate.get("url").cloned();
    let candidate_label = candidate.get("label").cloned();
    let candidate_parser_type = candidate.get("parserType").cloned();
    let preserve_role = candidate.get("strategy").and_then(Value::as_str)
        == Some("section-scoping-recovery")
        && candidate
            .get("url")
            .and_then(Value::as_str)
            .and_then(source_url_without_hash)
            == entry
                .get("url")
                .and_then(Value::as_str)
                .and_then(source_url_without_hash);
    let mut result = entry.clone();
    if let Some(object) = result.as_object_mut() {
        if !preserve_role {
            object.remove("role");
        }
        for (key, value) in [
            ("url", candidate_url),
            ("label", candidate_label),
            ("parserType", candidate_parser_type),
        ] {
            if let Some(value) = value {
                object.insert(key.to_string(), value);
            } else {
                object.remove(key);
            }
        }
    }
    result
}

pub fn select_alternate_parseable_manifest_entries<F>(
    entry: &Value,
    manifest_entries: &[Value],
    bootstrap_plan_titles: &HashMap<String, String>,
    primary_major_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
    candidate_conflicts_with_pathway: F,
) -> Vec<Value>
where
    F: Fn(&Value, &str, &str) -> bool,
{
    let mut candidates = manifest_entries
        .iter()
        .filter(|candidate| {
            candidate.get("ownerId") == entry.get("ownerId")
                && candidate.get("id") != entry.get("id")
                && candidate.get("url") != entry.get("url")
                && candidate.get("campusId") == entry.get("campusId")
        })
        .filter(|candidate| {
            manifest_entry_matches_specialized_plan_title(
                candidate,
                bootstrap_plan_titles,
                primary_major_titles,
                manifest_entries,
                course_parser,
            )
        })
        .filter(|candidate| {
            !candidate_conflicts_with_pathway(
                entry,
                &string_field(candidate, "label"),
                &string_field(candidate, "url"),
            )
        })
        .filter(|candidate| is_parseable_parser_type(&string_field(candidate, "parserType")))
        .filter(|candidate| should_allow_alternate_manifest_entry(entry, candidate))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(compare_manifest_fallback_candidates);
    candidates
}

pub fn should_allow_alternate_manifest_entry(entry: &Value, alternate_entry: &Value) -> bool {
    let source_role = classify_requirement_source_role(entry);
    let alternate_role = classify_requirement_source_role(alternate_entry);
    let source_status = get_requirement_source_role_status(Some(source_role));
    let alternate_status = get_requirement_source_role_status(Some(alternate_role));

    if ADMISSION_PREPARATION_SOURCE_ROLES.contains(&source_role) {
        return ADMISSION_PREPARATION_SOURCE_ROLES.contains(&alternate_role);
    }
    if source_status != RequirementSourceRoleStatus::Primary
        && alternate_status == RequirementSourceRoleStatus::Primary
    {
        return false;
    }
    if source_status == RequirementSourceRoleStatus::Primary
        && alternate_status == RequirementSourceRoleStatus::NonSchedulable
    {
        return false;
    }
    if source_role == "approved-course-list" {
        return alternate_role == "approved-course-list";
    }
    if source_role == "elective-list" {
        return alternate_role == "elective-list";
    }
    if source_status == RequirementSourceRoleStatus::NonSchedulable {
        return alternate_status == RequirementSourceRoleStatus::NonSchedulable;
    }
    true
}

pub fn compare_manifest_fallback_candidates(left: &Value, right: &Value) -> Ordering {
    let confidence_delta = confidence_score(&string_field(left, "confidence"))
        .cmp(&confidence_score(&string_field(right, "confidence")));
    if confidence_delta != Ordering::Equal {
        return confidence_delta;
    }
    let role_delta = source_role_score(right).cmp(&source_role_score(left));
    if role_delta != Ordering::Equal {
        return role_delta;
    }
    let parser_type_delta = parser_type_score(&string_field(right, "parserType"))
        .cmp(&parser_type_score(&string_field(left, "parserType")));
    if parser_type_delta != Ordering::Equal {
        return parser_type_delta;
    }
    locale_like_compare(&string_field(left, "label"), &string_field(right, "label"))
}

fn candidate_curriculum_url_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)/curriculum(?:[/?#]|$)|\bcurriculum\b")
            .expect("parser recovery curriculum URL")
    })
}

fn candidate_degree_requirements_url_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)/(?:degree|major)-requirements?(?:[/?#]|$)|\brequirements?\b")
            .expect("parser recovery degree requirements URL")
    })
}

fn confidence_score(value: &str) -> i8 {
    match value {
        "high" => 0,
        "medium" => 1,
        "low" => 2,
        _ => -1,
    }
}

fn source_role_score(entry: &Value) -> i8 {
    match classify_requirement_source_role(entry) {
        "official-catalog" | "primary-degree-requirements" => 6,
        "pathway-degree-sheet" => 5,
        "department-requirements" | "curriculum-map" => 4,
        "approved-course-list"
        | "elective-list"
        | "support-source"
        | "admission-prerequisite-source"
        | "admissions-preparation"
        | "sample-schedule"
        | "overview" => 2,
        "upper-division-prerequisite-table" | "non-schedulable-course-list" => 1,
        "transfer-equivalency" | "matched-grc-track" | "old-archival" => 0,
        _ => match string_field(entry, "role").as_str() {
            "degree-requirements" | "catalog" => 6,
            "pathway-degree-sheet" | "curriculum" => 5,
            "worksheet" => 4,
            "overview" => 2,
            _ => 1,
        },
    }
}

fn parser_type_score(parser_type: &str) -> i8 {
    match parser_type {
        "html-degree-page" => 8,
        "html-curriculum-page" => 7,
        "catalog-page" => 6,
        "html-overview-page" => 5,
        "pdf-degree-sheet" => 4,
        "pdf-worksheet" => 3,
        "generic-html" => 2,
        "generic-pdf" => 1,
        _ => 0,
    }
}

fn is_parseable_parser_type(parser_type: &str) -> bool {
    matches!(
        parser_type,
        "html-degree-page"
            | "html-curriculum-page"
            | "html-admissions-page"
            | "html-overview-page"
            | "catalog-page"
            | "generic-html"
            | "pdf-degree-sheet"
            | "pdf-worksheet"
            | "generic-pdf"
    )
}

fn locale_like_compare(left: &str, right: &str) -> Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
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
            .map(|value| if value.is_null() { String::new() } else { js_string(value) })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}
