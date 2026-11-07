use super::requirement_course_codes::CourseCodeParser;
use super::requirement_shape::{
    CHOICE_LIST_START_PATTERN_SOURCE, looks_like_standalone_requirement_label_line,
    looks_like_standalone_requirement_title_line, strip_leading_requirement_glyphs,
    strip_snapshot_page_prefix,
};
use regex::Regex;
use std::sync::OnceLock;

pub fn has_choice_requirement_context(value: &str) -> bool {
    choice_requirement_context_pattern().is_match(&normalize_whitespace(value))
}

pub fn has_strong_choice_requirement_context(value: &str) -> bool {
    strong_choice_requirement_context_pattern().is_match(&normalize_whitespace(value))
}

pub fn strip_non_option_course_evidence_from_choice_line(
    value: &str,
    course_parser: &CourseCodeParser,
) -> String {
    let mut text = normalize_whitespace(
        &trailing_if_taken_course_pattern().replace_all(
            &prerequisite_colon_pattern().replace_all(
                &not_both_parenthetical_pattern().replace_all(
                    &not_both_bracket_pattern().replace_all(
                        &prerequisite_parenthetical_pattern().replace_all(
                            &prerequisite_bracket_pattern().replace_all(value, ""),
                            "",
                        ),
                        "",
                    ),
                    "",
                ),
                "",
            ),
            "",
        ),
    );
    if let Some(index) = embedded_one_of_pattern().find(&text).map(|matched| matched.start()) {
        if index > 0 {
            let replacement = {
                let prefix = &text[..index];
                let option_tail = &text[index..];
                let prefix_course_codes = course_parser.extract_from_line(prefix);
                let prefix_looks_like_required_course_list =
                    required_course_list_prefix_pattern().is_match(prefix)
                        || prefix_ends_with_semicolon_pattern().is_match(prefix)
                        || (prefix_course_codes.len() >= 2
                            && prefix_has_comma_or_semicolon_pattern().is_match(prefix)
                            && !contains_or_pattern().is_match(prefix));
                if !prefix_course_codes.is_empty()
                    && prefix_looks_like_required_course_list
                    && course_parser.extract_from_line(option_tail).len() >= 2
                {
                    Some(normalize_whitespace(option_tail))
                } else {
                    None
                }
            };
            if let Some(replacement) = replacement {
                text = replacement;
            }
        }
    }
    text
}

pub fn find_nearby_requirement_choice_label(
    snapshot_lines: &[String],
    choice_line_index: usize,
    course_parser: &CourseCodeParser,
) -> String {
    let start_index = choice_line_index.saturating_sub(5);
    let mut credit_line = String::new();
    for index in (start_index..choice_line_index).rev() {
        let line = snapshot_lines
            .get(index)
            .map(String::as_str)
            .map(normalize_whitespace)
            .unwrap_or_default();
        let line_without_page = strip_snapshot_page_prefix(&line);
        let label_candidate = strip_leading_requirement_glyphs(&line_without_page);
        if simple_credit_label_pattern().is_match(&label_candidate) {
            credit_line = label_candidate;
            continue;
        }
        if !course_parser.extract_from_line(&label_candidate).is_empty() {
            return String::new();
        }
        if looks_like_standalone_requirement_label_line(&label_candidate, |value| {
            course_parser.extract_from_line(value)
        }) {
            return label_candidate;
        }
        if looks_like_standalone_requirement_title_line(&label_candidate, |value| {
            course_parser.extract_from_line(value)
        }) && (has_choice_requirement_context(&line_without_page)
            || programming_choice_context_pattern().is_match(&label_candidate))
        {
            return label_candidate;
        }
        if looks_like_standalone_requirement_title_line(&label_candidate, |value| {
            course_parser.extract_from_line(value)
        }) && !credit_line.is_empty()
        {
            return normalize_whitespace(&format!("{label_candidate} {credit_line}"));
        }
    }
    String::new()
}

pub fn strip_choice_list_line(line: &str) -> String {
    strip_leading_requirement_glyphs(&strip_snapshot_page_prefix(line))
}

pub fn trim_choice_list_continuation_noise(line: &str) -> String {
    normalize_whitespace(
        &continuation_section_noise_pattern().replace_all(
            &continuation_course_annotation_pattern().replace_all(line, ""),
            "",
        ),
    )
}

pub fn normalize_sequence_source_line(line: &str) -> String {
    strip_leading_lettered_option_marker(&strip_leading_requirement_glyphs(
        &strip_snapshot_page_prefix(line),
    ))
}

pub fn strip_leading_lettered_option_marker(line: &str) -> String {
    normalize_whitespace(&leading_lettered_option_marker_pattern().replace(line, ""))
}

pub fn is_lettered_option_line(line: &str) -> bool {
    lettered_option_line_pattern().is_match(&normalize_whitespace(&strip_choice_list_line(line)))
}

pub fn is_numbered_choose_one_option_heading(line: &str) -> bool {
    numbered_choose_one_option_heading_pattern()
        .is_match(&normalize_whitespace(&strip_choice_list_line(line)))
}

pub fn find_nearest_lettered_choose_one_heading(
    snapshot_lines: &[String],
    index: usize,
) -> Option<(usize, String)> {
    let start_index = index.saturating_sub(10);
    for candidate_index in (start_index..index).rev() {
        let candidate_line = normalize_whitespace(&strip_choice_list_line(
            snapshot_lines.get(candidate_index).map(String::as_str).unwrap_or_default(),
        ));
        if candidate_line.is_empty() {
            continue;
        }
        if is_numbered_choose_one_option_heading(&candidate_line) {
            return Some((candidate_index, candidate_line));
        }
        if numbered_option_heading_pattern().is_match(&candidate_line) {
            break;
        }
    }
    None
}

pub fn is_lettered_option_line_under_choose_one_heading(
    snapshot_lines: &[String],
    index: usize,
) -> bool {
    snapshot_lines
        .get(index)
        .is_some_and(|line| is_lettered_option_line(line))
        && find_nearest_lettered_choose_one_heading(snapshot_lines, index).is_some()
}

pub fn is_combined_choose_one_heading_with_lettered_option(line: &str) -> bool {
    combined_choose_one_lettered_option_pattern()
        .is_match(&normalize_whitespace(&strip_choice_list_line(line)))
}

pub fn choice_list_continuation_looks_like_adjacent_requirement(line: &str) -> bool {
    adjacent_requirement_pattern().is_match(&normalize_whitespace(line))
}

pub fn can_continue_wrapped_course_number(previous_text: &str, line: &str) -> bool {
    let previous = normalize_whitespace(previous_text);
    let current = normalize_whitespace(line);
    if !wrapped_course_number_pattern().is_match(&current) {
        return false;
    }
    wrapped_course_subject_pattern().is_match(&previous)
}

pub fn looks_like_choice_list_continuation(line: &str, previous_text: &str, course_parser: &CourseCodeParser) -> bool {
    let normalized_line = trim_choice_list_continuation_noise(line);
    if normalized_line.is_empty()
        || choice_list_continuation_looks_like_adjacent_requirement(&normalized_line)
    {
        return false;
    }
    let course_codes = course_parser.extract_from_line(&normalized_line);
    if course_codes.len() >= 2 && continuation_delimiter_pattern().is_match(&normalized_line) {
        return true;
    }
    if continuation_and_or_subject_pattern().is_match(&normalized_line) {
        return true;
    }
    if continuation_credit_parenthetical_pattern().is_match(&normalized_line) {
        return true;
    }
    if continuation_course_code_start_pattern().is_match(&normalized_line) {
        return continuation_course_code_terminator_pattern().is_match(&normalized_line)
            || !course_codes.is_empty();
    }
    can_continue_wrapped_course_number(previous_text, &normalized_line)
}

pub fn is_standalone_credit_amount_line(line: &str) -> bool {
    simple_credit_label_pattern().is_match(&strip_choice_list_line(line))
}

pub fn line_starts_parenthetical_choice_list(
    line: &str,
    course_parser: &CourseCodeParser,
) -> bool {
    let normalized_line = normalize_whitespace(&strip_choice_list_line(line));
    if !normalized_line.starts_with('(') {
        return false;
    }
    choice_list_start_pattern().is_match(&normalized_line)
        || (contains_or_pattern().is_match(&normalized_line)
            && course_parser.extract_from_line(&normalized_line).len() >= 2)
}

pub fn build_forward_labeled_choice_requirement_lines(
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let mut choice_lines = Vec::new();
    for (label_index, raw_label_line) in snapshot_lines.iter().enumerate() {
        let label = normalize_whitespace(&strip_leading_requirement_glyphs(
            &strip_choice_list_line(raw_label_line),
        ));
        if !looks_like_standalone_requirement_title_line(&label, |value| {
            course_parser.extract_from_line(value)
        }) {
            continue;
        }
        if !has_choice_requirement_context(&label)
            && !programming_choice_context_pattern().is_match(&label)
        {
            continue;
        }
        let credit_line = snapshot_lines
            .get(label_index + 1)
            .filter(|line| is_standalone_credit_amount_line(line))
            .map(|line| normalize_whitespace(&strip_choice_list_line(line)))
            .unwrap_or_default();
        let requirement_label = normalize_whitespace(&format!("{label} {credit_line}"));
        let scan_limit = snapshot_lines.len().min(label_index.saturating_add(12));
        for choice_index in label_index + 1..scan_limit {
            let candidate = normalize_whitespace(&strip_choice_list_line(&snapshot_lines[choice_index]));
            if candidate.is_empty() {
                continue;
            }
            if !choice_list_start_pattern().is_match(&candidate)
                && !line_starts_parenthetical_choice_list(
                    &candidate,
                    course_parser,
                )
            {
                continue;
            }
            choice_lines.push(build_choice_list_source_line(
                &requirement_label,
                &snapshot_lines[choice_index],
                snapshot_lines,
                choice_index,
                course_parser,
            ));
            break;
        }
    }
    choice_lines
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineLabeledRequirementParts {
    pub label: String,
    pub body: String,
    pub source_line: String,
}

pub fn strip_inline_labeled_requirement_marker(line: &str) -> String {
    normalize_whitespace(
        &inline_requirement_letter_marker_pattern().replace_all(
            &inline_requirement_number_marker_pattern().replace_all(&strip_choice_list_line(line), ""),
            "",
        ),
    )
}

pub fn get_inline_labeled_requirement_parts(
    raw_line: &str,
    course_parser: &CourseCodeParser,
) -> Option<InlineLabeledRequirementParts> {
    let normalized_line = strip_inline_labeled_requirement_marker(raw_line);
    let colon_index = normalized_line.find(':')?;
    if colon_index == 0 {
        return None;
    }
    let label = normalize_whitespace(
        &inline_requirement_label_terminal_punctuation_pattern()
            .replace_all(&normalized_line[..colon_index], ""),
    );
    let body = normalize_whitespace(&normalized_line[colon_index + 1..]);
    if label.is_empty()
        || body.is_empty()
        || label.encode_utf16().count() > 140
        || !course_parser.extract_from_line(&label).is_empty()
        || course_parser.extract_from_line(&body).is_empty()
    {
        return None;
    }
    Some(InlineLabeledRequirementParts {
        label,
        body,
        source_line: normalized_line,
    })
}

pub fn build_multi_line_choice_requirement_lines(
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let mut choice_lines =
        build_forward_labeled_choice_requirement_lines(snapshot_lines, course_parser);
    for (index, raw_line) in snapshot_lines.iter().enumerate() {
        let current_line = normalize_whitespace(raw_line);
        let requirement_label =
            find_nearby_requirement_choice_label(snapshot_lines, index, course_parser);
        if requirement_label.is_empty() {
            continue;
        }
        let current_without_page = strip_choice_list_line(&current_line);
        if get_inline_labeled_requirement_parts(&current_without_page, course_parser).is_some() {
            continue;
        }
        let current_course_codes = course_parser.extract_from_line(&current_without_page);
        let has_choice_list_start = choice_list_start_pattern().is_match(&current_without_page);
        let has_choice_heading_context =
            has_strong_choice_requirement_context(&requirement_label);
        let starts_heading_backed_comma_list = has_choice_heading_context
            && current_course_codes.len() >= 2
            && continuation_delimiter_pattern().is_match(&current_without_page)
            && !choice_list_continuation_looks_like_adjacent_requirement(&current_without_page);
        let starts_explicit_parenthetical_choice = current_without_page.starts_with('(')
            && contains_or_pattern().is_match(&current_without_page);
        if !has_choice_list_start
            && !starts_heading_backed_comma_list
            && !starts_explicit_parenthetical_choice
        {
            continue;
        }
        choice_lines.push(build_choice_list_source_line(
            &requirement_label,
            &current_line,
            snapshot_lines,
            index,
            course_parser,
        ));
    }
    choice_lines
}

pub fn choice_list_looks_closed(text: &str) -> bool {
    let normalized_text = normalize_whitespace(text);
    let open_count = normalized_text.chars().filter(|character| *character == '(').count();
    let close_count = normalized_text.chars().filter(|character| *character == ')').count();
    open_count > 0 && close_count >= open_count
}

pub fn build_choice_list_source_line(
    requirement_label: &str,
    start_line: &str,
    snapshot_lines: &[String],
    start_index: usize,
    course_parser: &CourseCodeParser,
) -> String {
    let first_part = trim_choice_list_continuation_noise(&strip_choice_list_line(start_line));
    let mut parts = if first_part.is_empty() {
        Vec::new()
    } else {
        vec![first_part]
    };
    let mut saw_closing_parenthesis = choice_list_looks_closed(&parts.join(" "));
    let end_index = snapshot_lines.len().min(start_index.saturating_add(10));
    for index in start_index.saturating_add(1)..end_index {
        if saw_closing_parenthesis {
            break;
        }
        let continuation = trim_choice_list_continuation_noise(&strip_choice_list_line(&snapshot_lines[index]));
        if !looks_like_choice_list_continuation(&continuation, &parts.join(" "), course_parser) {
            continue;
        }
        parts.push(continuation);
        saw_closing_parenthesis = choice_list_looks_closed(&parts.join(" "));
    }
    normalize_whitespace(&format!("{requirement_label}: {}", parts.join(" ")))
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn choice_list_start_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&format!("(?i){CHOICE_LIST_START_PATTERN_SOURCE}")).expect("choice list start pattern")
    })
}

fn choice_requirement_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:choose(?:\s+(?:from|one|two|three|four|five|six|seven|eight|nine|ten|\d+))?|select(?:ed)?(?:\s+(?:one|from|one|two|three|four|five|six|seven|eight|nine|ten|\d+))?|electives?|options?|approved\s+(?:courses?|electives?|list)|elective\s+list|course\s+list|one\s+(?:course\s+)?from|one\s+of(?:\s+the\s+following)?|either)\b").expect("choice requirement context pattern"))
}
fn strong_choice_requirement_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:choose(?:\s+(?:from|one|two|three|four|five|six|seven|eight|nine|ten|\d+))?|select(?:ed)?(?:\s+(?:one|from|one|two|three|four|five|six|seven|eight|nine|ten|\d+))?|approved\s+(?:courses?|electives?|list)|elective\s+list|course\s+list|one\s+(?:course\s+)?from|one\s+of(?:\s+the\s+following)?|either|electives?)\b").expect("strong choice requirement context pattern"))
}
fn prerequisite_bracket_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\[[^\]]*\b(?:pr|pre-?req|prereq(?:uisite)?)[^\]]*\]").expect("prerequisite bracket pattern"))
}
fn prerequisite_parenthetical_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\([^)]*\b(?:pr|pre-?req|prereq(?:uisite)?)[^)]*\)").expect("prerequisite parenthetical pattern"))
}
fn not_both_bracket_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\[[^\]]*\b(?:may\s+be\s+used|may\s+satisfy|may\s+count|cannot\s+count|but\s+not\s+both|not\s+both)\b[^\]]*\]").expect("non-option bracket pattern"))
}
fn not_both_parenthetical_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\([^)]*\b(?:may\s+be\s+used|may\s+satisfy|may\s+count|cannot\s+count|but\s+not\s+both|not\s+both)\b[^)]*\)").expect("non-option parenthetical pattern"))
}
fn prerequisite_colon_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:pr|pre-?req|prereq(?:uisite)?):\s*[^.;)\]]+").expect("prerequisite colon pattern"))
}
fn trailing_if_taken_course_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\s+\d{3}\s+or\s+[A-Z&]{2,8}\s+\d{3}[A-Z]?\s+if\s+taken\)?\.?\s*$").expect("trailing if-taken course annotation pattern"))
}
fn embedded_one_of_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:one\s+(?:course\s+)?(?:from|of)(?:\s+the\s+following)?|either)\b").expect("embedded one-of choice pattern"))
}
fn required_course_list_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:including|include|following|courses?|requirements?|complete|minimum|gpa|grade)\b").expect("required course list prefix pattern"))
}
fn prefix_ends_with_semicolon_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r";\s*$").expect("semicolon-ended prefix pattern"))
}
fn prefix_has_comma_or_semicolon_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"[,;]").expect("comma or semicolon prefix pattern"))
}
fn contains_or_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bor\b").expect("or connector pattern"))
}
fn inline_requirement_number_marker_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^\d+\s*[\).]\s*").expect("inline requirement numeric marker pattern"))
}
fn inline_requirement_letter_marker_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^[A-Z]\s*[\).]\s*").expect("inline requirement letter marker pattern"))
}
fn inline_requirement_label_terminal_punctuation_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"[:.]+$").expect("inline requirement label punctuation pattern"))
}
fn simple_credit_label_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\d+(?:-\d+)?\s*cr$").expect("simple credit label pattern"))
}
fn programming_choice_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:\[[^\]]+\]\s*)?(?:computer\s+)?(?:programming|computing)\b").expect("programming choice context pattern"))
}
fn continuation_course_annotation_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i),\s*[A-Z][A-Za-z&/ -]{2,80}\s+\([A-Z& ]+\s+\d{3}(?:\.\d+)?[A-Z]?[^)]*\)\s*$").expect("choice continuation course annotation pattern"))
}
fn continuation_section_noise_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\s+\b(?:Professional Practice|Capstone and Professional Practice|Engineering Fundamentals|Engineering & Science Electives|Technical Electives|Written Communication|Academic Planning Notes)\b.*$").expect("choice continuation section noise pattern"))
}
fn leading_lettered_option_marker_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^[a-z]\)\s*").expect("leading lettered option marker pattern"))
}
fn lettered_option_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^[a-z]\)\s+\S").expect("lettered option line pattern"))
}
fn numbered_choose_one_option_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\d+\)\s+\S.+\bchoose\s+one\s+option\b").expect("numbered choose-one heading pattern"))
}
fn numbered_option_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\d+\)\s+\S").expect("numbered option heading pattern"))
}
fn combined_choose_one_lettered_option_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\d+\)\s+\S.+\bchoose\s+one\s+option\b.*:\s*[a-z]\)\s+\S").expect("combined choose-one lettered option pattern"))
}
fn adjacent_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:taken\s+[A-Z]{3}|taken\s+in|qtr|senior year|junior year|capstone design course|professional practice)\b").expect("adjacent requirement pattern"))
}
fn wrapped_course_number_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^\d{3}(?:\.\d+)?[A-Z]?\b").expect("wrapped course number pattern"))
}
fn wrapped_course_subject_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b[A-Z]{2,8}(?:\s+[A-Z]{2,8})?$").expect("wrapped course subject pattern"))
}
fn continuation_delimiter_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"[,;]").expect("choice continuation delimiter pattern"))
}
fn continuation_and_or_subject_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:or|and)\s+[A-Z]{2,8}").expect("choice continuation and-or pattern"))
}
fn continuation_credit_parenthetical_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\(\s*\d+(?:\.\d+)?\s*cr\b").expect("choice continuation credit pattern"))
}
fn continuation_course_code_start_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^[A-Z]{2,8}(?:\s+[A-Z]{2,8})?\s+\d{3}(?:\.\d+)?[A-Z]?\b").expect("choice continuation course-code start pattern"))
}
fn continuation_course_code_terminator_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"[,;)]").expect("choice continuation course-code terminator pattern"))
}
