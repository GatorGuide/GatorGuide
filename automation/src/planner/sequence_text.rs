use regex::Regex;
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceInlineSegmentsInput {
    pub texts: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceCourseCodeInput {
    pub rows: Vec<SequenceCourseCodeRow>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceCourseCodeRow {
    pub text: String,
    #[serde(default)]
    pub fallback_subject: Option<String>,
    #[serde(default)]
    pub explicit_course_codes: Vec<String>,
}

pub fn split_inline_sequence_segments(texts: &[String]) -> Vec<Vec<String>> {
    texts
        .iter()
        .map(|text| split_inline_sequence_text(text))
        .collect()
}

fn split_inline_sequence_text(text: &str) -> Vec<String> {
    static PARENTHETICAL_OR: OnceLock<Regex> = OnceLock::new();
    static DASHED_OR: OnceLock<Regex> = OnceLock::new();
    static PLUS: OnceLock<Regex> = OnceLock::new();
    static ALTERNATIVE: OnceLock<Regex> = OnceLock::new();

    let parenthetical_or = PARENTHETICAL_OR
        .get_or_init(|| Regex::new(r"(?i)\(\s*or\s+").expect("parenthetical or pattern"));
    let dashed_or =
        DASHED_OR.get_or_init(|| Regex::new(r"(?i)\s+-OR-\s+").expect("dashed or pattern"));
    let plus = PLUS.get_or_init(|| Regex::new(r"\s+\+\s+").expect("plus pattern"));
    let alternative = ALTERNATIVE
        .get_or_init(|| Regex::new(r"(?i)\s*;\s*or\s+|\s+or\s+").expect("alternative pattern"));

    let normalized = parenthetical_or.replace_all(text, " or ").replace(')', " ");
    let normalized = dashed_or.replace_all(&normalized, " or ");
    let normalized = plus.replace_all(&normalized, ", ");
    alternative
        .split(&normalized)
        .map(normalize_whitespace)
        .filter(|segment| !segment.is_empty())
        .collect()
}

pub fn expand_sequence_course_codes(rows: &[SequenceCourseCodeRow]) -> Vec<Vec<String>> {
    rows.iter().map(expand_sequence_course_codes_row).collect()
}

fn expand_sequence_course_codes_row(row: &SequenceCourseCodeRow) -> Vec<String> {
    static SEQUENCE_LABEL: OnceLock<Regex> = OnceLock::new();
    static CHOOSE_SEQUENCE_PREFIX: OnceLock<Regex> = OnceLock::new();
    static COURSE_NUMBER: OnceLock<Regex> = OnceLock::new();
    static COURSE_SUBJECT: OnceLock<Regex> = OnceLock::new();
    static VALID_SUBJECT: OnceLock<Regex> = OnceLock::new();

    let sequence_label = SEQUENCE_LABEL.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:standard|regular|honors?|accelerated|calculus-based|algebra-based)\s+sequence\s+([A-Z&]{2,8}\s+\d{3})",
        )
        .expect("sequence label pattern")
    });
    let choose_sequence_prefix = CHOOSE_SEQUENCE_PREFIX.get_or_init(|| {
        Regex::new(r"(?i)^\s*(?:choose|select)\s+one\s+sequence\s*:\s*")
            .expect("choose sequence prefix pattern")
    });
    let course_number = COURSE_NUMBER
        .get_or_init(|| Regex::new(r"\d{3}[A-Za-z]?").expect("sequence course-number pattern"));
    let course_subject = COURSE_SUBJECT.get_or_init(|| {
        Regex::new(r"^(.+?)\s+\d{3}[A-Z]?$").expect("sequence explicit-course subject pattern")
    });
    let valid_subject = VALID_SUBJECT
        .get_or_init(|| Regex::new(r"^[A-Z&]+(?: [A-Z&]+)?$").expect("normalized subject pattern"));

    let text = normalize_whitespace(&sequence_label.replace_all(&row.text, "$1"));
    let text = normalize_whitespace(&choose_sequence_prefix.replace(&text, ""));
    let explicit_course_codes = unique_in_order(&row.explicit_course_codes);
    let Some(fallback_subject) = row
        .fallback_subject
        .as_deref()
        .map(|subject| normalize_whitespace(subject).to_ascii_uppercase())
        .filter(|subject| valid_subject.is_match(subject))
    else {
        return explicit_course_codes;
    };

    let has_different_explicit_subject = explicit_course_codes.iter().any(|course_code| {
        course_subject
            .captures(course_code)
            .and_then(|capture| capture.get(1))
            .is_some_and(|subject| subject.as_str().to_ascii_uppercase() != fallback_subject)
    });
    if !explicit_course_codes.is_empty() && has_different_explicit_subject {
        return explicit_course_codes;
    }

    let mut result = explicit_course_codes;
    for number_match in course_number.find_iter(&text) {
        let previous_character = text[..number_match.start()].chars().next_back();
        let next_character = text[number_match.end()..].chars().next();
        if previous_character.is_some_and(|character| character.is_ascii_alphanumeric())
            || next_character.is_some_and(|character| character.is_ascii_alphanumeric())
        {
            continue;
        }
        let number = number_match.as_str().to_ascii_uppercase();
        if number.starts_with("000") {
            continue;
        }
        let course_code = format!("{fallback_subject} {number}");
        if !result.contains(&course_code) {
            result.push(course_code);
        }
    }
    result
}

fn unique_in_order(values: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .iter()
        .filter(|value| !value.is_empty() && seen.insert((*value).clone()))
        .cloned()
        .collect()
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::{
        SequenceCourseCodeRow, expand_sequence_course_codes, split_inline_sequence_segments,
    };

    #[test]
    fn splits_inline_course_sequence_alternatives_in_order() {
        let inputs = vec![
            "PHYS 114, PHYS 115 or PHYS 121, PHYS 122".to_string(),
            "PHYS 121 + 122 + 123 -OR- PHYS 114 + 115 + 116".to_string(),
            "Calculus-based sequence (PHYS 121, 122, 123) or Algebra-based sequence (PHYS 114, 115, 116)".to_string(),
            "Choose one sequence: MATH 124, 125, 126; or MATH 134, 135, 136".to_string(),
        ];

        assert_eq!(
            split_inline_sequence_segments(&inputs),
            vec![
                vec![
                    "PHYS 114, PHYS 115".to_string(),
                    "PHYS 121, PHYS 122".to_string(),
                ],
                vec![
                    "PHYS 121, 122, 123".to_string(),
                    "PHYS 114, 115, 116".to_string(),
                ],
                vec![
                    "Calculus-based sequence (PHYS 121, 122, 123".to_string(),
                    "Algebra-based sequence (PHYS 114, 115, 116".to_string(),
                ],
                vec![
                    "Choose one sequence: MATH 124, 125, 126".to_string(),
                    "MATH 134, 135, 136".to_string(),
                ],
            ]
        );
    }

    #[test]
    fn expands_carried_subject_course_numbers_in_source_order() {
        let rows = vec![
            SequenceCourseCodeRow {
                text: "MATH 124, 125, 126".to_string(),
                fallback_subject: Some("MATH".to_string()),
                explicit_course_codes: vec!["MATH 124".to_string()],
            },
            SequenceCourseCodeRow {
                text: "Calculus-based sequence PHYS 121, 122, 123".to_string(),
                fallback_subject: Some("PHYS".to_string()),
                explicit_course_codes: vec!["PHYS 121".to_string()],
            },
        ];

        assert_eq!(
            expand_sequence_course_codes(&rows),
            vec![
                vec![
                    "MATH 124".to_string(),
                    "MATH 125".to_string(),
                    "MATH 126".to_string(),
                ],
                vec![
                    "PHYS 121".to_string(),
                    "PHYS 122".to_string(),
                    "PHYS 123".to_string(),
                ],
            ]
        );
    }

    #[test]
    fn does_not_carry_subject_across_mixed_explicit_codes_or_without_context() {
        let rows = vec![
            SequenceCourseCodeRow {
                text: "PHYS 122 or CSE 122".to_string(),
                fallback_subject: Some("PHYS".to_string()),
                explicit_course_codes: vec!["CSE 122".to_string(), "PHYS 122".to_string()],
            },
            SequenceCourseCodeRow {
                text: "PHYS 117, 118".to_string(),
                fallback_subject: None,
                explicit_course_codes: vec!["PHYS 117".to_string()],
            },
        ];

        assert_eq!(
            expand_sequence_course_codes(&rows),
            vec![
                vec!["CSE 122".to_string(), "PHYS 122".to_string()],
                vec!["PHYS 117".to_string()],
            ]
        );
    }

    #[test]
    fn normalizes_empty_and_non_alternative_inputs() {
        let inputs = vec![String::new(), "  PHYS 114   and PHYS 115  ".to_string()];
        assert_eq!(
            split_inline_sequence_segments(&inputs),
            vec![vec![], vec!["PHYS 114 and PHYS 115".to_string()]]
        );
    }
}
