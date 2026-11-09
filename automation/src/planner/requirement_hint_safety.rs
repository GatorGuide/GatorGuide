use super::requirement_atoms::{build_source_line_hint, get_source_line_hints};
use super::requirement_course_codes::{
    CourseCodeParser, get_explicitly_excluded_course_codes_from_lines, get_parsed_course_level,
    normalize_course_code,
};
use super::requirement_recovery_scope::{
    is_graduate_career_planning_continuation_line, is_graduate_career_planning_note,
    line_belongs_to_degree_route_comparison_section, line_references_conflicting_degree_route,
    line_references_different_major_scope,
};
use super::requirement_rule_utils::filter_course_codes_by_support_only_evidence;
use super::requirement_shape::{
    PROGRAMMING_CHOICE_CONTEXT_PATTERN_SOURCE, js_regex_compatibility_pattern,
};
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub fn filter_parsed_course_codes_by_hints(
    entry: &Value,
    lines: &[String],
    course_codes: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let hint_safe_course_codes = unique_sorted(
        course_codes
            .iter()
            .filter(|course_code| {
                let hints = get_source_line_hints(lines, course_code, course_parser);
                hints.is_empty()
                    || hints.iter().any(|hint| {
                        !is_unsafe_requirement_course_hint(
                            entry,
                            course_code,
                            hint,
                            lines,
                            course_parser,
                        )
                    })
            })
            .cloned()
            .collect(),
    );
    let source_line_hints_by_course = hint_safe_course_codes
        .iter()
        .map(|course_code| {
            (
                course_code.clone(),
                get_source_line_hints(lines, course_code, course_parser),
            )
        })
        .collect::<HashMap<_, _>>();
    unique_sorted(filter_course_codes_by_support_only_evidence(
        &hint_safe_course_codes,
        &source_line_hints_by_course,
    ))
}

pub fn recover_structured_course_codes_from_source_evidence(
    entry: &Value,
    structured_course_codes: &[String],
    requirement_cue_lines: &[String],
    choose_statements: &[String],
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    if structured_course_codes.is_empty() {
        return Vec::new();
    }
    let mut seen_lines = HashSet::new();
    let evidence_lines = requirement_cue_lines
        .iter()
        .chain(choose_statements.iter())
        .chain(snapshot_lines.iter())
        .filter(|line| seen_lines.insert((*line).clone()))
        .cloned()
        .collect::<Vec<_>>();
    if evidence_lines.is_empty() {
        return Vec::new();
    }
    let source_evidence_course_codes = unique_sorted(
        evidence_lines
            .iter()
            .flat_map(|line| course_parser.extract_from_line(line))
            .collect(),
    );
    let source_evidence_course_codes = filter_parsed_course_codes_by_hints(
        entry,
        &evidence_lines,
        &source_evidence_course_codes,
        course_parser,
    )
    .into_iter()
    .map(|course_code| normalize_course_code(&course_code))
    .collect::<HashSet<_>>();
    structured_course_codes
        .iter()
        .filter(|course_code| {
            source_evidence_course_codes.contains(&normalize_course_code(course_code))
        })
        .cloned()
        .collect()
}

pub fn filter_parsed_course_codes_by_hints_with_source_recovery(
    entry: &Value,
    lines: &[String],
    course_codes: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let source_codes = collect_requirement_source_course_code_candidates(
        entry,
        lines,
        course_codes,
        course_parser,
    );
    let mut safe_codes =
        filter_parsed_course_codes_by_hints(entry, lines, &source_codes, course_parser);
    safe_codes.extend(extract_safe_direct_course_line_codes_from_lines(
        entry,
        lines,
        course_parser,
    ));
    unique_sorted(safe_codes)
}

pub fn collect_requirement_source_course_code_candidates(
    entry: &Value,
    lines: &[String],
    candidate_course_codes: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let mut source_codes = candidate_course_codes.to_vec();
    source_codes.extend(extract_safe_known_subject_course_cluster_codes_from_lines(
        entry,
        lines,
        course_parser,
    ));
    source_codes.extend(extract_safe_direct_course_line_codes_from_lines(
        entry,
        lines,
        course_parser,
    ));
    source_codes.extend(extract_subject_scoped_bare_checklist_codes_from_lines(
        entry,
        lines,
        course_parser,
    ));
    let explicitly_excluded = get_explicitly_excluded_course_codes_from_lines(lines, course_parser)
        .into_iter()
        .map(|course_code| normalize_course_code(&course_code))
        .collect::<HashSet<_>>();
    unique_sorted(
        source_codes
            .into_iter()
            .filter(|course_code| {
                !explicitly_excluded.contains(&normalize_course_code(course_code))
            })
            .collect(),
    )
}

pub fn extract_safe_known_subject_course_cluster_codes_from_lines(
    entry: &Value,
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    if entry.is_null() {
        return Vec::new();
    }
    let mut recovered_codes = Vec::new();
    for (index, raw_line) in lines.iter().enumerate() {
        let line = normalize_whitespace(raw_line);
        if line.is_empty()
            || noisy_line_pattern().is_match(&line)
            || transfer_credit_noise_pattern().is_match(&line)
        {
            continue;
        }
        for course_code in course_parser.extract_from_requirement_line(&line) {
            if is_safe_known_subject_course_cluster_line(
                entry,
                lines,
                index,
                &course_code,
                course_parser,
            ) {
                recovered_codes.push(course_code);
            }
        }
    }
    unique_sorted(recovered_codes)
}

pub fn extract_subject_scoped_bare_checklist_codes_from_lines(
    entry: &Value,
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    if entry.is_null() || !is_direct_course_line_recovery_source(entry) {
        return Vec::new();
    }
    let mut subject_contexts = Vec::<(String, usize)>::new();
    let mut recovered_codes = Vec::new();
    for (index, raw_line) in lines.iter().enumerate() {
        let raw_line = normalize_whitespace(raw_line);
        let line = super::requirement_choice_lists::strip_choice_list_line(&raw_line);
        if line.is_empty()
            || noisy_line_pattern().is_match(&line)
            || transfer_credit_noise_pattern().is_match(&line)
        {
            continue;
        }
        for captures in numbered_subject_heading_pattern().captures_iter(&line) {
            let Some(subject) = captures.get(1).and_then(|capture| {
                normalize_extracted_course_subject(capture.as_str(), course_parser)
            }) else {
                continue;
            };
            subject_contexts.push((subject, index));
        }
        let explicit_course_codes = course_parser.extract_from_line(&line);
        let explicit_course_subjects = explicit_course_codes
            .iter()
            .filter_map(|course_code| {
                course_code
                    .rsplit_once(' ')
                    .map(|(subject, _)| subject.to_string())
            })
            .collect::<HashSet<_>>();
        let recent_subject_contexts = subject_contexts
            .iter()
            .rev()
            .filter(|context| index.saturating_sub(context.1) <= 30)
            .take(6)
            .map(|context| context.0.clone())
            .collect::<Vec<_>>();
        if recent_subject_contexts.is_empty() {
            continue;
        }
        for captures in bare_checklist_number_pattern().captures_iter(&line) {
            let Some(number) = captures.get(1).map(|capture| capture.as_str()) else {
                continue;
            };
            let candidate_codes = recent_subject_contexts
                .iter()
                .filter_map(|subject| {
                    normalize_extracted_course_code(subject, number, course_parser)
                        .filter(|course_code| !explicit_course_codes.contains(course_code))
                        .map(|course_code| (subject, course_code))
                })
                .collect::<Vec<_>>();
            let selected = candidate_codes
                .iter()
                .find(|(subject, _)| {
                    explicit_course_subjects.is_empty()
                        || explicit_course_subjects.contains(subject.as_str())
                })
                .or_else(|| candidate_codes.first());
            if let Some((_, course_code)) = selected {
                recovered_codes.push(course_code.to_string());
            }
        }
    }
    unique_sorted(recovered_codes)
}

pub fn extract_safe_direct_course_line_codes_from_lines(
    entry: &Value,
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    if entry.is_null() || !is_direct_course_line_recovery_source(entry) {
        return Vec::new();
    }
    let mut recovered_codes = Vec::new();
    for (index, raw_line) in lines.iter().enumerate() {
        let line = normalize_whitespace(raw_line);
        if line.is_empty()
            || noisy_line_pattern().is_match(&line)
            || transfer_credit_noise_pattern().is_match(&line)
            || !source_line_starts_with_course_code(&line)
            || !has_requirement_context_near_line(lines, index, 80)
        {
            continue;
        }
        recovered_codes.extend(course_parser.extract_from_requirement_line(&line));
    }
    unique_sorted(recovered_codes)
}

pub fn is_direct_course_line_recovery_source(entry: &Value) -> bool {
    if entry.is_null() {
        return false;
    }
    let source_text = ["parserType", "label", "sourceLabel", "url"]
        .iter()
        .filter_map(|key| entry.get(*key).filter(|value| js_truthy(value)))
        .map(js_string_value)
        .collect::<Vec<_>>()
        .join(" ");
    let is_generic_primary_degree_html = generic_html_source_pattern().is_match(&source_text)
        && entry.get("isPrimaryDegreeRequirementsLink") == Some(&Value::Bool(true))
        && generic_primary_degree_cue_pattern().is_match(&source_text);
    pdf_degree_recovery_source_pattern().is_match(&source_text)
        || is_generic_primary_degree_html
        || (html_admissions_recovery_source_pattern().is_match(&source_text)
            && admissions_requirement_cue_pattern().is_match(&source_text))
        || (html_degree_recovery_source_pattern().is_match(&source_text)
            && html_degree_requirement_cue_pattern().is_match(&source_text))
        || (document_recovery_source_pattern().is_match(&source_text)
            && document_requirement_cue_pattern().is_match(&source_text))
}

pub fn is_unsafe_requirement_course_hint(
    entry: &Value,
    course_code: &str,
    hint: &str,
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_hint = normalize_whitespace(hint);
    if normalized_hint.is_empty() {
        return false;
    }
    if is_bare_course_code_source_hint(&normalized_hint, course_code) {
        return !is_requirement_supported_bare_course_code_hint(
            entry,
            course_code,
            lines,
            course_parser,
        );
    }
    if line_references_different_major_scope(entry, &normalized_hint)
        || non_student_facing_hint_pattern().is_match(&normalized_hint)
    {
        return true;
    }
    let extracted_course_codes = course_parser.extract_from_line(&normalized_hint);
    if !extracted_course_codes.is_empty()
        && competency_pattern().is_match(&normalized_hint)
        && !explicit_requirement_cue_pattern().is_match(&normalized_hint)
        && !source_line_starts_with_course_code(&normalized_hint)
    {
        return true;
    }
    let hint_line_index = lines.iter().position(|line| {
        normalize_whitespace(line) == normalized_hint
            || build_source_line_hint(course_code, line, course_parser).as_deref()
                == Some(normalized_hint.as_str())
    });
    if is_graduate_career_planning_note(&normalized_hint)
        || hint_line_index.map_or(false, |index| {
            is_graduate_career_planning_continuation_line(lines, index, course_parser)
        })
    {
        return true;
    }
    let hint_line_index_for_scope = hint_line_index.map(|index| index as isize).unwrap_or(-1);
    if line_references_conflicting_degree_route(entry, lines, hint_line_index_for_scope)
        || line_belongs_to_degree_route_comparison_section(lines, hint_line_index_for_scope)
    {
        return true;
    }
    let has_nearby_requirement_context = hint_line_index.map_or(false, |index| {
        has_requirement_context_near_line(lines, index, 4)
            || lines[index.saturating_sub(4)..=index].iter().any(|line| {
                programming_choice_context_pattern().is_match(
                    &super::requirement_choice_lists::strip_choice_list_line(line),
                )
            })
    });
    if extracted_course_codes.len() >= 6
        && !requirement_friendly_hint_pattern().is_match(&normalized_hint)
        && !direct_requirement_course_sequence_pattern().is_match(&normalized_hint)
        && !has_nearby_requirement_context
    {
        return true;
    }
    false
}

pub fn is_bare_course_code_source_hint(hint: &str, course_code: &str) -> bool {
    let normalized_hint = hint
        .chars()
        .map(|character| {
            if "|,;:.()[]{}".contains(character) {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    normalize_course_code(&normalize_whitespace(&normalized_hint))
        == normalize_course_code(course_code)
}

fn is_requirement_supported_bare_course_code_hint(
    entry: &Value,
    course_code: &str,
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> bool {
    lines.iter().enumerate().any(|(index, line)| {
        is_bare_course_code_source_hint(line, course_code)
            && (is_safe_known_subject_course_cluster_line(
                entry,
                lines,
                index,
                course_code,
                course_parser,
            ) || is_bare_course_code_supported_by_requirement_heading(
                entry,
                course_code,
                lines,
                index,
            ))
    })
}

fn is_bare_course_code_supported_by_requirement_heading(
    entry: &Value,
    course_code: &str,
    lines: &[String],
    index: usize,
) -> bool {
    let Some(line) = lines.get(index).map(|line| normalize_whitespace(line)) else {
        return false;
    };
    if !is_bare_course_code_source_hint(&line, course_code) {
        return false;
    }
    let context_lines = lines[index.saturating_sub(4)..(index + 2).min(lines.len())]
        .iter()
        .map(|line| normalize_whitespace(line))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let context_text = context_lines.join(" ");
    if !bare_course_requirement_heading_pattern().is_match(&context_text) {
        return false;
    }
    !context_lines
        .iter()
        .any(|context_line| line_references_different_major_scope(entry, context_line))
}

fn is_safe_known_subject_course_cluster_line(
    entry: &Value,
    lines: &[String],
    index: usize,
    course_code: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    if entry.is_null()
        || !has_known_subject_course_cluster(lines, index, course_code, course_parser)
    {
        return false;
    }
    let source_identity = ["parserType", "sourceLabel", "url"]
        .iter()
        .filter_map(|key| entry.get(*key).filter(|value| js_truthy(value)))
        .map(js_string_value)
        .collect::<Vec<_>>()
        .join(" ");
    has_requirement_context_near_line(lines, index, 80)
        || requirement_source_identity_pattern().is_match(&source_identity)
}

fn has_known_subject_course_cluster(
    lines: &[String],
    index: usize,
    course_code: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let Some((subject, _)) = course_code.rsplit_once(' ') else {
        return false;
    };
    if !is_known_extracted_subject(subject, course_parser)
        || get_parsed_course_level(course_code).map_or(true, |level| level >= 500)
    {
        return false;
    }
    let start = index.saturating_sub(8);
    let end = (index + 9).min(lines.len());
    let mut same_subject_course_count = 0usize;
    let mut has_descriptive_course_line = false;
    for line in &lines[start..end] {
        let normalized_line = normalize_whitespace(line);
        let line_course_codes = course_parser
            .extract_from_line(&normalized_line)
            .into_iter()
            .filter(|line_course_code| {
                line_course_code
                    .rsplit_once(' ')
                    .map(|(line_subject, _)| line_subject)
                    == Some(subject)
            })
            .collect::<Vec<_>>();
        same_subject_course_count += line_course_codes.len();
        if !line_course_codes.is_empty()
            && !line_course_codes.iter().all(|line_course_code| {
                is_bare_course_code_source_hint(&normalized_line, line_course_code)
            })
        {
            has_descriptive_course_line = true;
        }
    }
    same_subject_course_count >= 3
        || (same_subject_course_count >= 2 && has_descriptive_course_line)
}

fn is_known_extracted_subject(subject: &str, course_parser: &CourseCodeParser) -> bool {
    supplemental_extracted_subjects().contains(&subject)
        || course_parser.normalize_subject(subject).is_some()
}

fn normalize_extracted_course_subject(
    raw_subject: &str,
    course_parser: &CourseCodeParser,
) -> Option<String> {
    let normalized_subject = normalize_whitespace(raw_subject).to_ascii_uppercase();
    course_parser
        .normalize_subject(&normalized_subject)
        .or_else(|| {
            supplemental_extracted_subjects()
                .contains(&normalized_subject.as_str())
                .then_some(normalized_subject)
        })
}

fn normalize_extracted_course_code(
    subject: &str,
    raw_number: &str,
    course_parser: &CourseCodeParser,
) -> Option<String> {
    let number = normalize_whitespace(raw_number).to_ascii_uppercase();
    let valid_number = (number.len() == 3 || number.len() == 4)
        && number[..3]
            .chars()
            .all(|character| character.is_ascii_digit())
        && &number[..3] != "000"
        && (number.len() == 3 || number.as_bytes()[3].is_ascii_uppercase());
    if !valid_number {
        return None;
    }
    course_parser.normalize_code(subject, &number).or_else(|| {
        is_supplemental_extracted_subject(subject).then(|| format!("{subject} {number}"))
    })
}

fn is_supplemental_extracted_subject(subject: &str) -> bool {
    supplemental_extracted_subjects().contains(&subject)
}

fn supplemental_extracted_subjects() -> &'static [&'static str] {
    &[
        "ATMS", "B ECON", "BGIS", "BCMU", "BSE", "CSSSKL", "ENTRE", "FIN", "IBUS", "IS", "MKTG",
        "OPMGT", "THLEAD", "TIAS", "TSTAT",
    ]
}

fn has_requirement_context_near_line(lines: &[String], index: usize, radius: usize) -> bool {
    let start = index.saturating_sub(radius);
    let end = (index + radius + 1).min(lines.len());
    lines[start..end].iter().any(|line| {
        let line = normalize_whitespace(line);
        !line.is_empty()
            && (structural_requirement_pattern().is_match(&line)
                || requirement_cue_pattern().is_match(&line)
                || requirement_friendly_hint_pattern().is_match(&line)
                || course_cluster_requirement_context_pattern().is_match(&line))
    })
}

pub fn source_line_starts_with_course_code(line: &str) -> bool {
    let without_page_prefix = super::requirement_shape::strip_snapshot_page_prefix(line);
    direct_course_line_start_pattern().is_match(&normalize_whitespace(&without_page_prefix))
}

fn js_string_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => super::requirement_shape::js_number_to_string(value),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(js_string_value)
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().map_or(false, |number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn normalize_whitespace(value: &str) -> String {
    super::requirement_shape::normalize_js_whitespace(value)
}

fn unique_sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

fn non_student_facing_hint_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(suggested general education|not required for transferring|highly recommended courses?|other recommended courses?|approved list|study abroad|capstone courses?|course evaluations?|course lists?|policy(?:\s*&\s*|\s+and\s+)procedures|graduate school|graduate programs?)\b")).expect("non-student-facing hint pattern"))
}

fn competency_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            r"(?i)\bcompetenc(?:y|ies)\b",
        ))
        .expect("competency hint pattern")
    })
}

fn explicit_requirement_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(?:required|requirements?|complete|completed|select|choose|one\s+(?:course|of)|all\s+(?:of\s+)?the\s+following|must)\b")).expect("explicit requirement cue pattern"))
}

fn requirement_friendly_hint_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(required|requirements?|prereq|prerequisite|complete|completed|admission|degree requirements?|credits?|engineering fundamentals|mathematics|sciences|written\s*&\s*oral communication|english composition|areas of inquiry|choose from the following|select one sequence|prior to the start of|before the start of|continuation requirements?)\b")).expect("requirement-friendly hint pattern"))
}

fn direct_requirement_course_sequence_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\*?[A-Z&]+(?:\s+[A-Z&]+)?\s+\d{3}[A-Za-z]?\s*,\s*\d{3}[A-Za-z]?\s*,\s*\d{3}[A-Za-z]?(?:\s+(?:or|and)\s+\d{3}[A-Za-z]?\s*,\s*\d{3}[A-Za-z]?\s*,\s*\d{3}[A-Za-z]?)?\s*\(\s*\d+\s*\)")).expect("direct requirement course sequence pattern"))
}

fn structural_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(admission requirements|degree requirements|major requirements|completion requirements|required courses|elective courses|core courses|core requirements|curriculum|prerequisite courses|shared set of core courses|specialization|track|option|route|pathway|concentration|checklist|foundation|distribution requirement)\b")).expect("structural requirement context pattern"))
}

fn requirement_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(required|requirements|prereq|prerequisite|complete|credits|credit|elective|select|choose|one of the following|two of the following|option|track|route|pathway|concentration)\b")).expect("requirement cue pattern"))
}

fn course_cluster_requirement_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(model program of study|program of study|curriculum overview|curriculum plan|core curriculum|foundation courses|practice courses|required curriculum|graduation requirements|social welfare electives|upper-division .*electives)\b")).expect("course cluster requirement context pattern"))
}

fn bare_course_requirement_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(?:required courses?|required course|core courses?|major specific requirements?|additional completion requirements?|completion requirements?|degree requirements?|major requirements?)\b")).expect("bare course requirement heading pattern"))
}

fn direct_course_line_start_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)^[-*\u{2022}\u{F071}\s]*(?:\(\s*\d+(?:\.\d+)?\s*\)\s*)?(?:[A-Z&]+(?:\s+[A-Z&]+)?(?:\s*/\s*[A-Z&]+(?:\s+[A-Z&]+)?)?\s+\d{3}[A-Za-z]?\*?|\d{3}[A-Za-z]?\b)")).expect("direct course line start pattern"))
}

fn requirement_source_identity_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            r"(?i)\b(curriculum|requirements?|checklist|worksheet)\b",
        ))
        .expect("requirement source identity pattern")
    })
}

fn programming_choice_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            PROGRAMMING_CHOICE_CONTEXT_PATTERN_SOURCE,
        ))
        .expect("programming choice context pattern")
    })
}

fn noisy_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)(?:src=|srcset=|aria-label=|title=|\.jpg\b|\.jpeg\b|\.png\b|\.svg\b|\.gif\b|^\[page\s+\d+\]\s*%pdf-|^\s*%pdf-|^\s*(?:<!--|-->|(?:pre-bubbling\s+)?cache\s+(?:contexts|tags|max-age):?$|(?:\*\s*)?(?:route\.name\.|url\.site$|languages:|user\.permissions$|theme$)))")).expect("noisy source line pattern"))
}

fn transfer_credit_noise_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(transfer credits?|transfer equivalenc(?:y|ies)|articulation agreement|community colleges?|community college)\b")).expect("transfer-credit noise pattern"))
}

fn generic_html_source_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(r"(?i)\bgeneric-html\b"))
            .expect("generic HTML parser type pattern")
    })
}

fn generic_primary_degree_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(&js_regex_compatibility_pattern(r"(?i)\b(?:b\.?\s*a\.?|b\.?\s*s\.?|bachelor|degree|major|requirements?|curriculum|core|introductory|bookend|capstone|courses?|electives?)\b")).expect("generic primary degree cue pattern"))
}

fn pdf_degree_recovery_source_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            r"(?i)\bpdf-(?:degree-sheet|worksheet)\b",
        ))
        .expect("PDF degree recovery source pattern")
    })
}

fn html_admissions_recovery_source_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            r"(?i)\bhtml-admissions-page\b",
        ))
        .expect("HTML admissions recovery source pattern")
    })
}

fn admissions_requirement_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            r"(?i)\b(?:admissions?|prereq(?:uisites?)?|requirements?|preparation)\b",
        ))
        .expect("admissions requirement cue pattern")
    })
}

fn html_degree_recovery_source_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            r"(?i)\bhtml-(?:degree|curriculum|overview)-page\b",
        ))
        .expect("HTML degree recovery source pattern")
    })
}

fn html_degree_requirement_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            r"(?i)\b(curriculum|requirements?|checklist|worksheet|degree|sample program plan)\b",
        ))
        .expect("HTML degree requirement cue pattern")
    })
}

fn document_recovery_source_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            r"(?i)\.(?:pdf|docx)(?:$|[?#])",
        ))
        .expect("document recovery source pattern")
    })
}

fn document_requirement_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            r"(?i)\b(curriculum|requirements?|checklist|worksheet|degree)\b",
        ))
        .expect("document requirement cue pattern")
    })
}

fn numbered_subject_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            r"\b\d+\)\s+[^()]{2,90}\(([A-Z& ]{2,10})\)",
        ))
        .expect("numbered subject heading pattern")
    })
}

fn bare_checklist_number_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&js_regex_compatibility_pattern(
            r"(?i)(?:^|[^\w])(?:[^\w\d]{1,10}\s*)?(\d{3}[A-Za-z]?)\s*\(\d+(?:\.\d+)?\)",
        ))
        .expect("bare checklist number pattern")
    })
}
