use super::requirement_course_codes::{CourseCodeParser, normalize_course_code};
use super::requirement_ids::slugify_requirement_identifier;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::OnceLock;

const STRUCTURED_PATHWAY_GENERIC_TOKENS: &[&str] =
    &["ba", "bs", "option", "pathway", "track", "family"];

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
        string_field(candidate, "planId") == plan_id && js_truthy(candidate.get("pathwayId"))
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

pub fn build_parsed_degree_map_block_candidates(
    owner: &Value,
    parsed_course_codes: &[String],
    requirement_cue_lines: &[String],
    choose_statements: &[String],
    pathway_labels: &[String],
    headings: &[String],
    structured_blocks: &[Value],
) -> Vec<Value> {
    if !parsed_course_codes.is_empty() {
        let owner_id = js_interpolate(owner.get("ownerId"));
        let owner_title = js_interpolate(owner.get("ownerTitle"));
        let adapter_id = nullish_string(owner.get("adapterId"));
        return vec![json!({
            "id": format!("{owner_id}:source-degree-map:{}", slugify_requirement_identifier(&adapter_id)),
            "title": format!("{owner_title} parsed official source requirements"),
            "uwCourseCodes": parsed_course_codes,
            "sourceLineHints": requirement_cue_lines.iter().take(10).collect::<Vec<_>>(),
        })];
    }

    let source_hints = unique_sorted_lines(
        headings
            .iter()
            .chain(pathway_labels.iter())
            .chain(requirement_cue_lines.iter())
            .chain(choose_statements.iter())
            .cloned()
            .collect(),
    )
    .into_iter()
    .take(40)
    .map(|line| (line.clone(), normalize_degree_map_matcher_text(&line)))
    .collect::<Vec<_>>();
    let mut structured_candidates = Vec::new();
    for block in structured_blocks.iter().filter(|block| {
        block.get("planId") == owner.get("planId")
            && (nullish_string(block.get("pathwayId")) == nullish_string(owner.get("pathwayId"))
                || (!nullish_string(owner.get("pathwayId")).is_empty()
                    && nullish_string(block.get("pathwayId")).is_empty())
                || (nullish_string(owner.get("pathwayId")).is_empty()
                    && nullish_string(block.get("pathwayId")).is_empty()))
    }) {
        let block_title = nullish_string(block.get("title"));
        let title_tokens = degree_map_block_title_tokens(&block_title);
        let matching_lines = source_hints
            .iter()
            .filter(|(_, normalized)| {
                let token_overlap_count = title_tokens
                    .iter()
                    .filter(|token| normalized.contains(token.as_str()))
                    .count();
                if token_overlap_count >= 2 {
                    return true;
                }
                if degree_map_admission_cue_pattern().is_match(normalized)
                    && degree_map_admission_cue_pattern().is_match(&block_title)
                {
                    return true;
                }
                if degree_map_pathway_cue_pattern().is_match(normalized)
                    && degree_map_pathway_cue_pattern().is_match(&block_title)
                    && token_overlap_count >= 1
                {
                    return true;
                }
                degree_map_structure_cue_pattern().is_match(normalized)
                    && degree_map_structure_cue_pattern().is_match(&block_title)
            })
            .map(|(line, _)| line.clone())
            .take(6)
            .collect::<Vec<_>>();
        if matching_lines.is_empty() {
            continue;
        }
        let owner_id = js_interpolate(owner.get("ownerId"));
        let block_id = nullish_string(block.get("id"));
        structured_candidates.push(json!({
            "id": format!("{owner_id}:source-degree-map:structured:{}", slugify_requirement_identifier(&block_id)),
            "title": block.get("title").cloned().unwrap_or(Value::Null),
            "uwCourseCodes": [],
            "sourceLineHints": matching_lines,
        }));
    }
    if !structured_candidates.is_empty() {
        return structured_candidates;
    }

    let source_line_hints = unique_sorted_lines(
        headings
            .iter()
            .chain(pathway_labels.iter())
            .chain(requirement_cue_lines.iter())
            .chain(choose_statements.iter())
            .cloned()
            .collect(),
    )
    .into_iter()
    .take(10)
    .collect::<Vec<_>>();
    if source_line_hints.is_empty() {
        return Vec::new();
    }
    let owner_id = js_interpolate(owner.get("ownerId"));
    let owner_title = js_interpolate(owner.get("ownerTitle"));
    vec![json!({
        "id": format!("{owner_id}:source-degree-map:notes"),
        "title": format!("{owner_title} parsed official source structure"),
        "uwCourseCodes": [],
        "sourceLineHints": source_line_hints,
    })]
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

fn unique_sorted_lines(mut lines: Vec<String>) -> Vec<String> {
    lines.retain(|line| !line.is_empty());
    lines.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| right.cmp(left))
    });
    lines.dedup();
    lines
}

fn normalize_degree_map_matcher_text(value: &str) -> String {
    degree_map_nonword_pattern()
        .replace_all(
            &super::requirement_shape::normalize_js_whitespace(value).to_lowercase(),
            " ",
        )
        .trim()
        .to_string()
}

fn degree_map_block_title_tokens(title: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "and",
        "arts",
        "baseline",
        "culture",
        "degree",
        "finish",
        "major",
        "media",
        "overall",
        "requirements",
        "requirement",
        "shared",
        "studies",
        "study",
        "the",
    ];
    normalize_degree_map_matcher_text(title)
        .split_whitespace()
        .filter(|token| token.len() >= 4 && !STOPWORDS.contains(token))
        .map(str::to_string)
        .collect()
}

fn degree_map_nonword_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"[^a-z0-9]+").expect("degree map matcher text"))
}

fn degree_map_admission_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_degree_map_pattern(r"(?i)\badmission\b"))
}

fn degree_map_pathway_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_degree_map_pattern(r"(?i)\b(track|option|pathway|route|specialization)\b")
    })
}

fn degree_map_structure_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_degree_map_pattern(r"(?i)\b(core|foundation|structure|requirements?)\b")
    })
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

fn js_interpolate(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(value) => js_string(value),
    }
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

fn structured_course_number_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_degree_map_pattern(r"\b[A-Z&]+\s*(\d{3})\b"))
}

fn graduate_course_owner_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_degree_map_pattern(r"(?i)\b(?:graduate|master|m\.?s\.?|ph\.?d|certificate)\b")
    })
}

fn js_compatible_degree_map_pattern(pattern: &str) -> Regex {
    let compatible_pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
    Regex::new(&compatible_pattern).expect("JavaScript-compatible degree-map pattern")
}
