use super::requirement_role_classifier::classify_requirement_source_role;
use super::requirement_scope::{
    PARSEABLE_SUPPORT_ROLES, RequirementSourceRoleStatus, get_requirement_source_role_status,
};
use super::requirement_snapshots::source_url_has_hash;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use url::Url;

pub fn preselect_parseable_manifest_entries(
    entries: &[Value],
    target_plan_ids: Option<&[String]>,
) -> Vec<Value> {
    let target_ids = target_plan_ids
        .filter(|ids| !ids.is_empty())
        .map(|ids| ids.iter().cloned().collect::<HashSet<_>>());
    entries
        .iter()
        .filter(|entry| {
            let owner_type = entry.get("ownerType").and_then(Value::as_str).unwrap_or_default();
            if !matches!(owner_type, "major" | "pathway") {
                return false;
            }
            let campus = entry.get("campusId").and_then(Value::as_str).unwrap_or_default();
            if campus.is_empty() || campus == "grc" {
                return false;
            }
            if let Some(target_ids) = target_ids.as_ref() {
                let Some(plan_id) = entry.get("planId").and_then(Value::as_str) else {
                    return false;
                };
                if !target_ids.contains(plan_id) {
                    return false;
                }
            }
            let parser_type = entry.get("parserType").and_then(Value::as_str).unwrap_or_default();
            if !is_parseable_parser_type(parser_type) {
                return false;
            }
            if js_truthy(entry.get("isPrimaryDegreeRequirementsLink")) {
                return true;
            }
            let role = classify_requirement_source_role(entry);
            get_requirement_source_role_status(Some(role)) == RequirementSourceRoleStatus::Primary
                || PARSEABLE_SUPPORT_ROLES.contains(&role)
        })
        .cloned()
        .collect()
}

pub fn normalize_manifest_url_for_dedupe(value: &str) -> String {
    normalize_manifest_url(value, true).unwrap_or_else(|| normalize_whitespace(value))
}

pub fn normalize_manifest_url_without_hash_for_dedupe(value: &str) -> String {
    normalize_manifest_url(value, false).unwrap_or_else(|| {
        normalize_whitespace(value)
            .split_once('#')
            .map_or_else(|| normalize_whitespace(value), |(base, _)| normalize_whitespace(base))
    })
}

pub fn exclude_unhashed_primary_manifest_duplicates(entries: &[Value]) -> Vec<Value> {
    let anchored_primary_base_keys = entries
        .iter()
        .filter(|entry| {
            js_truthy(entry.get("isPrimaryDegreeRequirementsLink"))
                && source_url_has_hash(&string_field(entry, "url"))
        })
        .map(|entry| {
            format!(
                "{}\0{}",
                js_interpolate(entry.get("ownerId")),
                normalize_manifest_url_without_hash_for_dedupe(&string_field(entry, "url"))
            )
        })
        .collect::<HashSet<_>>();

    entries
        .iter()
        .filter(|entry| {
            if js_truthy(entry.get("isPrimaryDegreeRequirementsLink"))
                || source_url_has_hash(&string_field(entry, "url"))
            {
                return true;
            }
            let key = format!(
                "{}\0{}",
                js_interpolate(entry.get("ownerId")),
                normalize_manifest_url_without_hash_for_dedupe(&string_field(entry, "url"))
            );
            !anchored_primary_base_keys.contains(&key)
        })
        .cloned()
        .collect()
}

pub fn dedupe_parseable_primary_entries(entries: &[Value]) -> Vec<Value> {
    let mut by_key = HashMap::<String, Value>::new();
    for entry in entries {
        let key = parseable_manifest_entry_dedupe_key(entry);
        match by_key.get(&key) {
            Some(existing) if !candidate_is_preferred(existing, entry) => {}
            _ => {
                by_key.insert(key, entry.clone());
            }
        }
    }
    let mut output = by_key.into_values().collect::<Vec<_>>();
    output.sort_by(|left, right| {
        locale_like_compare(&string_field(left, "ownerTitle"), &string_field(right, "ownerTitle"))
            .then_with(|| locale_like_compare(&string_field(left, "ownerId"), &string_field(right, "ownerId")))
            .then_with(|| locale_like_compare(&string_field(left, "url"), &string_field(right, "url")))
    });
    output
}

pub fn parseable_manifest_entry_dedupe_key(entry: &Value) -> String {
    let owner_id = entry
        .get("ownerId")
        .filter(|value| !value.is_null())
        .map(js_string)
        .unwrap_or_else(|| {
            format!(
                "{}:{}",
                nullish_string(entry.get("planId")),
                nullish_string(entry.get("pathwayId"))
            )
        });
    format!(
        "{}\0{}\0{}\0{}",
        owner_id,
        normalize_manifest_url_for_dedupe(&string_field(entry, "url")),
        nullish_string(entry.get("parserType")),
        classify_requirement_source_role(entry)
    )
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

fn candidate_is_preferred(existing: &Value, candidate: &Value) -> bool {
    let existing_primary = js_truthy(existing.get("isPrimaryDegreeRequirementsLink"));
    let candidate_primary = js_truthy(candidate.get("isPrimaryDegreeRequirementsLink"));
    if existing_primary != candidate_primary {
        return candidate_primary;
    }
    let existing_role = classify_requirement_source_role(existing);
    let candidate_role = classify_requirement_source_role(candidate);
    let existing_status = get_requirement_source_role_status(Some(existing_role));
    let candidate_status = get_requirement_source_role_status(Some(candidate_role));
    let status_score = |status| match status {
        RequirementSourceRoleStatus::Primary => 3,
        RequirementSourceRoleStatus::Support => 2,
        RequirementSourceRoleStatus::NonSchedulable => 1,
        RequirementSourceRoleStatus::Ignored => 0,
    };
    let existing_score = status_score(existing_status);
    let candidate_score = status_score(candidate_status);
    if existing_score != candidate_score {
        return candidate_score > existing_score;
    }
    locale_like_compare(
        &nullish_string(existing.get("id")),
        &nullish_string(candidate.get("id")),
    ) == std::cmp::Ordering::Greater
}

fn normalize_manifest_url(value: &str, include_hash: bool) -> Option<String> {
    let parsed = Url::parse(value).ok()?;
    let path = parsed.path().trim_end_matches('/');
    let normalized_path = if path.is_empty() { "/" } else { path };
    let origin = parsed.origin().ascii_serialization();
    let query = parsed
        .query()
        .filter(|value| !value.is_empty())
        .map(|value| format!("?{value}"))
        .unwrap_or_default();
    let hash = if include_hash {
        parsed
            .fragment()
            .filter(|value| !value.is_empty())
            .map(|value| format!("#{value}"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    Some(format!("{origin}{normalized_path}{query}{hash}"))
}

fn locale_like_compare(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn nullish_string(value: Option<&Value>) -> String {
    value.filter(|value| !value.is_null()).map(js_string).unwrap_or_default()
}

fn string_field(value: &Value, key: &str) -> String {
    nullish_string(value.get(key))
}

fn js_interpolate(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(value) => js_string(value),
    }
}

fn js_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => true,
    }
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
