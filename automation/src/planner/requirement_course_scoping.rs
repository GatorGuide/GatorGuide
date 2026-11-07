use super::requirement_course_codes::{CourseCodeParser, normalize_course_code};
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::OnceLock;

pub fn manifest_entry_can_use_graduate_course_codes(entry: &Value) -> bool {
    let owner_text = ["ownerTitle", "majorTitle", "title", "label", "sourceLabel", "url"]
        .into_iter()
        .map(|key| string_field(entry, key))
        .collect::<Vec<_>>()
        .join(" ");
    graduate_owner_pattern().is_match(&owner_text)
}

pub fn is_graduate_cross_listed_companion_for_undergraduate_source(
    entry: &Value,
    lines: &[String],
    course_code: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    if manifest_entry_can_use_graduate_course_codes(entry) {
        return false;
    }
    let normalized_course_code = normalize_course_code(course_code);
    let Some((subject, level)) = course_subject_and_level(&normalized_course_code) else {
        return false;
    };
    if level < 500 {
        return false;
    }

    source_line_hints(lines, &normalized_course_code, course_parser)
        .iter()
        .flat_map(|hint| course_parser.extract_from_line(hint))
        .any(|candidate| {
            course_subject_and_level(&candidate)
                .is_some_and(|(candidate_subject, candidate_level)| {
                    candidate_subject == subject && candidate_level < 500
                })
        })
}

pub fn filter_graduate_cross_listed_companions_for_undergraduate_source(
    entry: &Value,
    lines: &[String],
    course_codes: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    course_codes
        .iter()
        .map(|course_code| normalize_course_code(course_code))
        .filter(|course_code| !course_code.is_empty())
        .filter(|course_code| {
            !is_graduate_cross_listed_companion_for_undergraduate_source(
                entry,
                lines,
                course_code,
                course_parser,
            )
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn source_line_hints(
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
        let normalized_line = normalize_whitespace(line);
        if normalized_line.is_empty() {
            continue;
        }
        let hint = if course_code == "ENGL 131"
            && english_composition_pattern().is_match(&normalized_line)
            && english_composition_context_pattern().is_match(&normalized_line)
            && !english_composition_exclusion_pattern().is_match(&normalized_line)
            && course_parser
                .extract_from_line(&normalized_line)
                .iter()
                .any(|candidate| candidate != course_code)
            && !normalized_line.to_ascii_uppercase().contains(course_code)
        {
            english_composition_pattern()
                .replace(&normalized_line, "English composition (ENGL 131)")
                .into_owned()
        } else if !normalized_line.to_ascii_uppercase().contains(course_code) {
            format!("{normalized_line} ({course_code})")
        } else {
            normalized_line
        };
        if hint.len() <= 280 {
            hints.push(hint);
            if hints.len() == 5 {
                break;
            }
        }
    }
    hints.sort();
    hints.dedup();
    hints
}

fn course_subject_and_level(course_code: &str) -> Option<(String, u16)> {
    let captures = course_code_pattern().captures(&normalize_course_code(course_code))?;
    Some((captures.get(1)?.as_str().trim().to_string(), captures.get(2)?.as_str().parse().ok()?))
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn graduate_owner_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:graduate|master|m\.?s\.?|m\.?a\.?|ph\.?d|doctor|certificate)\b")
            .expect("graduate source owner")
    })
}

fn course_code_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"^([A-Z& ]+)\s+(\d{3})[A-Z]?$").expect("course subject and level")
    })
}

fn english_composition_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bEnglish composition\b").expect("English composition"))
}

fn english_composition_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\b(5 credits of|five credits of|min(?:imum)?(?:\s+course)? requirements?|prerequisite|admission|complete|completed|required|eligibility|preferred)\b")
            .expect("English composition context")
    })
}

fn english_composition_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\b(additional english composition|except the 5-?credit english composition requirement|writing-intensive|w course|w courses)\b")
            .expect("English composition exclusion")
    })
}
