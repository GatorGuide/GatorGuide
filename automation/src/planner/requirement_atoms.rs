use super::requirement_course_codes::CourseCodeParser;
use super::requirement_ids::{build_unique_derived_id, slugify_requirement_identifier};
use super::requirement_source_audits::get_source_section_decision_for_course;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRequirementPhase {
    pub phase: Option<&'static str>,
    pub display_phase: Option<&'static str>,
    pub phase_confidence: Option<&'static str>,
}

pub fn build_source_line_hint(
    course_code: &str,
    line: &str,
    course_parser: &CourseCodeParser,
) -> Option<String> {
    let normalized_line = normalize_requirement_text(line);
    if normalized_line.is_empty() {
        return None;
    }
    let explicit_course_codes = course_parser.extract_explicit_from_line(&normalized_line);
    if course_code == "ENGL 131"
        && !explicit_course_codes.is_empty()
        && !explicit_course_codes.iter().any(|code| code == course_code)
        && english_composition_requirement_pattern().is_match(&normalized_line)
        && !english_composition_exclusion_pattern().is_match(&normalized_line)
    {
        return Some(
            english_composition_requirement_pattern()
                .replace(&normalized_line, "English composition (ENGL 131)")
                .into_owned(),
        );
    }
    if !course_code.is_empty()
        && course_parser
            .extract_from_line(&normalized_line)
            .iter()
            .any(|code| code == course_code)
        && !normalized_line
            .to_uppercase()
            .contains(&course_code.to_uppercase())
    {
        return Some(format!("{normalized_line} ({course_code})"));
    }
    Some(normalized_line)
}

pub fn get_source_line_hints(
    lines: &[String],
    course_code: &str,
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let mut hints = Vec::new();
    for line in lines {
        if !course_parser
            .extract_from_line(line)
            .iter()
            .any(|candidate| candidate == course_code)
        {
            continue;
        }
        if let Some(hint) = build_source_line_hint(course_code, line, course_parser) {
            if hint.encode_utf16().count() <= 280 {
                hints.push(hint);
            }
        }
        if hints.len() >= 5 {
            break;
        }
    }
    unique_sorted(hints)
}

pub fn infer_parsed_requirement_phase_from_hints(
    source_line_hints: &[String],
) -> ParsedRequirementPhase {
    let combined_hints = source_line_hints
        .iter()
        .map(|line| normalize_requirement_text(line).to_lowercase())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if combined_hints.is_empty() {
        return no_requirement_phase();
    }
    if before_application_phase_pattern().is_match(&combined_hints) {
        return ParsedRequirementPhase {
            phase: Some("before-application"),
            display_phase: Some("before-application"),
            phase_confidence: Some("high"),
        };
    }
    if before_enrollment_phase_pattern().is_match(&combined_hints) {
        return ParsedRequirementPhase {
            phase: Some("before-enrollment"),
            display_phase: Some("before-enrollment"),
            phase_confidence: Some("high"),
        };
    }
    if stay_at_grc_phase_pattern().is_match(&combined_hints) {
        return ParsedRequirementPhase {
            phase: Some("stay-at-grc"),
            display_phase: Some("stay-at-grc"),
            phase_confidence: Some("medium"),
        };
    }
    if required_course_phase_pattern().is_match(&combined_hints) {
        return ParsedRequirementPhase {
            phase: Some("before-enrollment"),
            display_phase: Some("before-enrollment"),
            phase_confidence: Some("low"),
        };
    }
    no_requirement_phase()
}

pub fn build_parsed_requirement_atom_candidates(
    owner: &Value,
    parsed_course_codes: &[String],
    snapshot_lines: &[String],
    source_section_audit_rows: &[Value],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let owner_id = owner
        .get("ownerId")
        .map(js_string_value)
        .unwrap_or_else(|| "undefined".to_string());
    let mut seen_atom_ids = HashMap::new();
    parsed_course_codes
        .iter()
        .map(|course_code| {
            let base_id = format!(
                "{owner_id}:source-atom:{}",
                slugify_requirement_identifier(course_code)
            );
            let source_line_hints =
                get_source_line_hints(snapshot_lines, course_code, course_parser);
            let phase = infer_parsed_requirement_phase_from_hints(&source_line_hints);
            let section_decision = get_source_section_decision_for_course(
                course_code,
                source_section_audit_rows,
            );
            let mut candidate = Map::new();
            candidate.insert(
                "id".into(),
                json!(build_unique_derived_id(&base_id, &mut seen_atom_ids)),
            );
            candidate.insert("title".into(), json!(course_code));
            candidate.insert("uwCourseCode".into(), json!(course_code));
            candidate.insert("phase".into(), option_string(phase.phase));
            candidate.insert(
                "displayPhase".into(),
                option_string(phase.display_phase),
            );
            candidate.insert(
                "phaseConfidence".into(),
                option_string(phase.phase_confidence),
            );
            candidate.insert("sourceLineHints".into(), json!(source_line_hints));
            candidate.insert(
                "sourceSectionRole".into(),
                section_decision
                    .and_then(|decision| decision.get("detectedSectionRole"))
                    .filter(|value| !value.is_null())
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            candidate.insert(
                "sourceSectionSchedulable".into(),
                section_decision
                    .and_then(|decision| decision.get("schedulable"))
                    .and_then(Value::as_bool)
                    .map(Value::from)
                    .unwrap_or(Value::Null),
            );
            candidate.insert(
                "sourceSectionReason".into(),
                section_decision
                    .and_then(|decision| decision.get("reason"))
                    .filter(|value| !value.is_null())
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            Value::Object(candidate)
        })
        .collect()
}

fn no_requirement_phase() -> ParsedRequirementPhase {
    ParsedRequirementPhase {
        phase: None,
        display_phase: None,
        phase_confidence: None,
    }
}

fn option_string(value: Option<&str>) -> Value {
    value.map(|value| json!(value)).unwrap_or(Value::Null)
}

fn normalize_requirement_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn unique_sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
    values.dedup();
    values
}

fn js_string_value(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(|value| if value.is_null() { String::new() } else { js_string_value(value) })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

fn english_composition_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\benglish composition\b").expect("English composition requirement pattern"))
}
fn english_composition_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(additional english composition|except the 5-?credit english composition requirement|writing-intensive|w course|w courses)\b").expect("English composition exclusion pattern"))
}
fn before_application_phase_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b(admission|apply|application|prereq|prerequisite|eligibility|before applying)\b").expect("before-application phase pattern"))
}
fn before_enrollment_phase_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b(before enrollment|before matriculation|matriculation|before transfer|prior to transfer|pre-professional)\b").expect("before-enrollment phase pattern"))
}
fn stay_at_grc_phase_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b(recommended|encouraged|suggested|optional|elective)\b").expect("stay-at-GRC phase pattern"))
}
fn required_course_phase_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b(requirements?|required|core courses?|degree requirements?)\b").expect("required-course phase pattern"))
}
