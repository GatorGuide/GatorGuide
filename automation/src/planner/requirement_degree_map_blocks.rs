use super::requirement_course_codes::{CourseCodeParser, normalize_course_code};
use super::requirement_ids::slugify_requirement_identifier;
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::OnceLock;

const STRUCTURED_PATHWAY_GENERIC_TOKENS: &[&str] = &[
    "ba", "bs", "option", "pathway", "track", "family",
];

pub fn structured_degree_map_block_matches_manifest_entry(
    block: &Value,
    manifest_entry: &Value,
    all_blocks: &[Value],
) -> bool {
    let plan_id = string_field(block, "planId");
    if plan_id != string_field(manifest_entry, "planId") {
        return false;
    }

    let pathway_id = nullish_string(manifest_entry.get("pathwayId"));
    let pathway_id = if pathway_id.is_empty() {
        None
    } else {
        Some(pathway_id)
    };
    let block_pathway_id = nullish_string(block.get("pathwayId"));
    let block_pathway_id = if block_pathway_id.is_empty() {
        None
    } else {
        Some(block_pathway_id)
    };

    let Some(pathway_id) = pathway_id else {
        return block_pathway_id.is_none();
    };
    if block_pathway_id.as_deref() == Some(pathway_id.as_str()) {
        return true;
    }
    if block_pathway_id.is_some() {
        return false;
    }

    let has_scoped_blocks_for_plan = all_blocks.iter().any(|candidate| {
        string_field(candidate, "planId") == plan_id
            && js_truthy(candidate.get("pathwayId"))
    });
    if !has_scoped_blocks_for_plan {
        return true;
    }

    let pathway_tokens = slugify_requirement_identifier(&pathway_id)
        .split('-')
        .filter(|token| !token.is_empty() && !STRUCTURED_PATHWAY_GENERIC_TOKENS.contains(token))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if pathway_tokens.is_empty() {
        return false;
    }
    let block_slug = slugify_requirement_identifier(&format!(
        "{} {}",
        nullish_string(block.get("id")),
        nullish_string(block.get("title"))
    ));
    pathway_tokens
        .iter()
        .all(|token| block_slug.contains(token))
}

pub fn get_structured_uw_course_codes(
    manifest_entry: &Value,
    degree_map_blocks: &[Value],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let mut course_codes = BTreeSet::new();
    for block in degree_map_blocks.iter().filter(|block| {
        structured_degree_map_block_matches_manifest_entry(block, manifest_entry, degree_map_blocks)
    }) {
        let Some(codes) = block.get("uwCourseCodes").and_then(Value::as_array) else {
            continue;
        };
        for raw_code in codes {
            let normalized = normalize_course_code(&js_string(raw_code));
            if normalized.is_empty() || !is_recognized_course_code(&normalized, course_parser) {
                continue;
            }
            if should_keep_structured_course_code(manifest_entry, &normalized) {
                course_codes.insert(normalized);
            }
        }
    }
    course_codes.into_iter().collect()
}

fn is_recognized_course_code(value: &str, course_parser: &CourseCodeParser) -> bool {
    course_parser
        .extract_from_line(value)
        .iter()
        .any(|candidate| candidate == value)
}

fn should_keep_structured_course_code(manifest_entry: &Value, course_code: &str) -> bool {
    let Some(captures) = structured_course_number_pattern().captures(course_code) else {
        return false;
    };
    let Some(course_number) = captures
        .get(1)
        .and_then(|value| value.as_str().parse::<u16>().ok())
    else {
        return false;
    };
    if course_number < 500 {
        return true;
    }
    let owner_text = ["ownerTitle", "majorTitle", "title", "label"]
        .into_iter()
        .map(|key| string_field(manifest_entry, key))
        .collect::<Vec<_>>()
        .join(" ");
    graduate_course_owner_pattern().is_match(&owner_text)
}

fn string_field(value: &Value, key: &str) -> String {
    nullish_string(value.get(key))
}

fn nullish_string(value: Option<&Value>) -> String {
    value
        .filter(|value| !value.is_null())
        .map(js_string)
        .unwrap_or_default()
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

fn structured_course_number_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"\b[A-Z&]+\s*(\d{3})\b").expect("structured course number")
    })
}

fn graduate_course_owner_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:graduate|master|m\.?s\.?|ph\.?d|certificate)\b")
            .expect("graduate course owner")
    })
}
