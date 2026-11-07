use super::requirement_shape::{strip_leading_requirement_glyphs, strip_snapshot_page_prefix};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

const SUPPLEMENTAL_UW_SUBJECTS: &[&str] = &[
    "ATMS", "B ECON", "BGIS", "BCMU", "BSE", "CSSSKL", "ENTRE", "FIN", "IBUS", "IS",
    "MKTG", "OPMGT", "THLEAD", "TIAS", "TSTAT",
];

const INVALID_SUBJECTS: &[&str] = &[
    "ABOVE", "APPLY", "AND", "ANY", "APPROVED", "ARE", "AREA", "AT", "AUTUMN", "BASIC",
    "BE", "BEFORE", "BETWEEN", "BELOW", "BEYOND", "BEGIN", "BOTH", "BOX", "BLDG",
    "BUILDING", "BUT", "BY", "CALL", "CLASSES", "COURSES", "COURSE", "CORE", "CREDITS",
    "CREDIT", "COMPLETE", "CONSIDER", "DATA", "DOES", "DIVISION", "EARN", "EARNED", "EITHER",
    "ENGLISH", "EXCEPT", "FAX", "FORTUNE", "FOR", "FOREIGN", "FORMERLY", "HALL", "FROM",
    "GRADED", "HAS", "HAVE", "HAVEA", "IF", "IN", "INCLUDE", "INCLUDES", "INTO", "IS",
    "JUST", "LANG", "LANGUAGE", "LEVEL", "LEAST", "LIKE", "MINIMUM", "MAX", "MORE", "MUST",
    "NUMBERED", "NOT", "OF", "ONE", "ON", "OFFERINGS", "OCCUPIES", "OFFICE", "OR", "OTHER",
    "PLUS", "REACH", "REQUIRE", "REQUIREMENTS", "RECOMMENDED", "REQUIRES", "REQUIRED", "ROOM",
    "SECTION", "SEPARATE", "SPRING", "RM", "BREADTH", "GROCERY", "SUITE", "SUMMER", "TAKE",
    "TAKING", "THAT", "THAN", "THE", "THEN", "THROUGH", "TO", "TOTALS", "TWO", "USE", "WANT",
    "WHILE", "WILL", "WINTER", "WITH", "YOUR",
];

const RECOVERABLE_LEADING_SUBJECT_TOKENS: &[&str] = &[
    "AND", "APPROVED", "AS", "BOTH", "CHOOSE", "COMPLETE", "EITHER", "FROM", "IN", "OF", "ONE",
    "IS", "OR", "PREREQ", "PREREQUISITE", "REQUIRED", "SELECT", "TAKE", "THE",
];
const LEADING_SUBJECT_TOKENS: &[&str] = &["AND", "AS", "OR"];
const LEADING_LIST_MARKERS: &[&str] = &["I", "II", "III", "IV"];

#[derive(Debug, Clone, Default)]
pub struct CourseCodeParser {
    known_subjects: HashSet<String>,
    metadata_by_code: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CourseCreditRange {
    pub credits: Option<f64>,
    pub credit_min: Option<f64>,
    pub credit_max: Option<f64>,
    pub credit_text: Option<String>,
}

pub fn get_parsed_course_level(course_code: &str) -> Option<u16> {
    course_code_numeric_level_pattern()
        .captures(course_code)
        .and_then(|captures| captures.get(1))
        .and_then(|level| level.as_str().parse::<u16>().ok())
}

pub fn get_course_level_summary(course_codes: &[String]) -> Value {
    let levels = course_codes
        .iter()
        .filter_map(|course_code| get_parsed_course_level(course_code))
        .collect::<Vec<_>>();
    let upper_division_count = levels.iter().filter(|level| **level >= 300).count();
    let lower_division_count = levels.len() - upper_division_count;
    let required_upper_division_count = ((levels.len() * 3 + 3) / 4).max(1);
    json!({
        "totalCount": levels.len(),
        "upperDivisionCount": upper_division_count,
        "lowerDivisionCount": lower_division_count,
        "mainlyUpperDivision": !levels.is_empty() && upper_division_count >= required_upper_division_count,
        "allLowerDivision": !levels.is_empty() && upper_division_count == 0,
    })
}

pub fn get_explicitly_excluded_course_codes_from_lines(
    lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    unique_sorted(
        lines
            .iter()
            .flat_map(|line| {
                course_parser
                    .extract_from_line(line)
                    .into_iter()
                    .filter(|course_code| {
                        course_parser.line_explicitly_excludes_course_code(line, course_code)
                    })
            })
            .collect(),
    )
}

impl CourseCodeParser {
    pub fn from_metadata_directory(metadata_directory: &Path) -> Result<Self, String> {
        let entries = fs::read_dir(metadata_directory).map_err(|error| {
            format!(
                "could not read generated UW course metadata at {}: {error}",
                metadata_directory.display()
            )
        })?;
        let subject_pattern = Regex::new(r"^([A-Z&]+(?: [A-Z&]+)*) \d")
            .map_err(|error| format!("invalid course metadata subject pattern: {error}"))?;
        let mut parser = Self::default();
        let mut parsed_partition_count = 0usize;

        for entry in entries {
            let entry = entry.map_err(|error| {
                format!("could not enumerate course metadata partitions: {error}")
            })?;
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path).map_err(|error| {
                format!("could not read course metadata partition {}: {error}", path.display())
            })?;
            let rows: Value = serde_json::from_slice(&bytes).map_err(|error| {
                format!("invalid course metadata partition {}: {error}", path.display())
            })?;
            let Some(rows) = rows.as_array() else {
                return Err(format!(
                    "course metadata partition {} must contain a JSON array",
                    path.display()
                ));
            };
            parsed_partition_count += 1;

            for row in rows {
                if row.get("schoolId").and_then(Value::as_str) == Some("grc") {
                    continue;
                }
                let Some(code) = row.get("code").and_then(Value::as_str) else {
                    continue;
                };
                let normalized_code = normalize_course_code(code);
                if normalized_code.is_empty() {
                    continue;
                }
                if let Some(subject) = subject_pattern
                    .captures(&normalized_code)
                    .and_then(|capture| capture.get(1))
                {
                    parser
                        .known_subjects
                        .insert(subject.as_str().to_string());
                }
                parser
                    .metadata_by_code
                    .entry(normalized_code)
                    .or_insert_with(|| row.clone());
            }
        }

        if parsed_partition_count == 0 {
            return Err(format!(
                "no course metadata JSON partitions were found at {}",
                metadata_directory.display()
            ));
        }
        parser
            .known_subjects
            .extend(SUPPLEMENTAL_UW_SUBJECTS.iter().map(|subject| (*subject).to_string()));
        Ok(parser)
    }

    pub fn with_known_subjects(subjects: impl IntoIterator<Item = String>) -> Self {
        let mut parser = Self {
            known_subjects: subjects.into_iter().collect(),
            metadata_by_code: HashMap::new(),
        };
        parser
            .known_subjects
            .extend(SUPPLEMENTAL_UW_SUBJECTS.iter().map(|subject| (*subject).to_string()));
        parser
    }

    pub fn known_subjects(&self) -> &HashSet<String> {
        &self.known_subjects
    }

    pub fn metadata_course_codes_for_subject_prefix(
        &self,
        campus_id: &str,
        subject_prefix: &str,
    ) -> Vec<String> {
        let campus_id = normalize_whitespace(campus_id);
        let Some(subject_prefix) = self.normalize_subject(subject_prefix) else {
            return Vec::new();
        };
        unique_sorted(
            self.metadata_by_code
                .iter()
                .filter(|(_, entry)| {
                    campus_id.is_empty()
                        || entry.get("schoolId").and_then(Value::as_str) == Some(campus_id.as_str())
                })
                .map(|(course_code, _)| course_code)
                .filter(|course_code| course_code.starts_with(&format!("{subject_prefix} ")))
                .cloned()
                .collect(),
        )
    }

    pub fn metadata_for(&self, course_code: &str) -> Option<&Value> {
        self.metadata_by_code
            .get(&normalize_course_code_with_subjects(course_code, &self.known_subjects))
    }

    pub fn credit_range_for(&self, course_code: &str) -> CourseCreditRange {
        let Some(metadata) = self.metadata_for(course_code) else {
            return CourseCreditRange {
                credits: None,
                credit_min: None,
                credit_max: None,
                credit_text: None,
            };
        };
        let credit_value = metadata.get("creditValue").and_then(Value::as_f64);
        if let Some(value) = credit_value.filter(|value| value.is_finite() && *value > 0.0) {
            return CourseCreditRange {
                credits: Some(value),
                credit_min: Some(value),
                credit_max: Some(value),
                credit_text: metadata
                    .get("creditLabel")
                    .and_then(Value::as_str)
                    .map(normalize_whitespace)
                    .filter(|value| !value.is_empty())
                    .or_else(|| Some(format_number(value))),
            };
        }

        let credit_text = metadata
            .get("creditLabel")
            .and_then(Value::as_str)
            .map(normalize_whitespace)
            .filter(|value| !value.is_empty());
        if let Some(label) = credit_text.as_deref() {
            if let Some(captures) = credit_range_regex().captures(label) {
                let minimum = captures.get(1).and_then(|value| value.as_str().parse::<f64>().ok());
                let maximum = captures.get(2).and_then(|value| value.as_str().parse::<f64>().ok());
                if let (Some(minimum), Some(maximum)) = (minimum, maximum) {
                    if minimum.is_finite() && maximum.is_finite() && maximum >= minimum {
                        return CourseCreditRange {
                            credits: Some(minimum),
                            credit_min: Some(minimum),
                            credit_max: Some(maximum),
                            credit_text,
                        };
                    }
                }
            }
        }
        CourseCreditRange {
            credits: None,
            credit_min: None,
            credit_max: None,
            credit_text: None,
        }
    }

    pub fn normalize_subject(&self, raw_subject: &str) -> Option<String> {
        normalize_subject(raw_subject, &self.known_subjects)
    }

    pub fn normalize_code(&self, raw_subject: &str, raw_number: &str) -> Option<String> {
        normalize_code(raw_subject, raw_number, &self.known_subjects)
    }

    pub fn extract_from_line(&self, line: &str) -> Vec<String> {
        let normalized = normalize_whitespace(&normalize_known_spaced_subjects(
            &normalize_course_connector_spacing(line),
            &self.known_subjects,
        ));
        let mut codes = self.extract_explicit_codes(&normalized);

        for captures in shared_number_regex().captures_iter(&normalized) {
            if let (Some(left), Some(right), Some(number)) =
                (captures.get(1), captures.get(2), captures.get(3))
            {
                codes.extend(self.normalize_code(left.as_str(), number.as_str()));
                codes.extend(self.normalize_code(right.as_str(), number.as_str()));
            }
        }
        for captures in repeated_subject_alternative_regex().captures_iter(&normalized) {
            if let (Some(subject), Some(number)) = (captures.get(1), captures.get(2)) {
                codes.extend(self.normalize_code(subject.as_str(), number.as_str()));
            }
        }

        if english_composition_regex().is_match(&normalized)
            && !english_composition_exclusion_regex().is_match(&normalized)
            && (normalized.len() <= 140 || english_composition_context_regex().is_match(&normalized))
        {
            codes.push("ENGL 131".to_string());
        }
        unique_sorted(codes)
    }

    pub fn extract_explicit_from_line(&self, line: &str) -> Vec<String> {
        self.extract_explicit_codes(line)
    }

    pub fn extract_from_requirement_line(&self, line: &str) -> Vec<String> {
        let normalized_line = normalize_whitespace(line);
        let historical_aliases = self.historical_alias_course_codes(&normalized_line);
        unique_sorted(
            self.extract_from_line(&normalized_line)
                .into_iter()
                .filter(|course_code| {
                    !is_course_code_level_reference(&normalized_line, course_code)
                        && !self.line_explicitly_excludes_course_code(&normalized_line, course_code)
                        && !historical_aliases.contains(&normalize_course_code(course_code))
                })
                .collect(),
        )
    }

    pub fn line_explicitly_excludes_course_code(&self, line: &str, course_code: &str) -> bool {
        let normalized_line = normalize_whitespace(&strip_leading_requirement_glyphs(
            &strip_snapshot_page_prefix(line),
        ));
        let Some(course_pattern) = course_code_mention_pattern(course_code) else {
            return false;
        };
        if normalized_line.is_empty() {
            return false;
        }
        let no_except_means_allowed = dynamic_pattern(&format!(
            r"(?i)\bno\b[^.;:]{{0,80}}\bexcept\b[^.;:]{{0,100}}{course_pattern}"
        ))
        .is_match(&normalized_line);
        if !no_except_means_allowed
            && dynamic_pattern(&format!(
                r"(?i)\b(?:except|excluding|with\s+the\s+exception\s+of)\b[^.;:]{{0,100}}{course_pattern}"
            ))
            .is_match(&normalized_line)
        {
            return true;
        }
        dynamic_pattern(&format!(
            r"(?i){course_pattern}[^.;:]{{0,120}}\b(?:excluded|excludes|not\s+(?:accepted|allowed|counted?|substitut(?:e|es|ed|ing)|satisf(?:y|ies)|fulfill(?:s|ed)?|meet)|cannot\s+(?:count|receive\s+credit|be\s+(?:used|applied)|substitut(?:e|es?)|satisfy|fulfill|meet)|may\s+not\s+(?:count|receive\s+credit|be\s+(?:used|applied)|substitut(?:e|es?)|satisfy|fulfill|meet)|will\s+not\s+(?:count|receive\s+credit|be\s+(?:used|applied)|substitut(?:e|es?)|satisfy|fulfill|meet)|does\s+not\s+(?:count|satisfy|fulfill|meet)|do\s+not\s+count)\b"
        ))
        .is_match(&normalized_line)
    }

    fn historical_alias_course_codes(&self, line: &str) -> HashSet<String> {
        let normalized_line = normalize_whitespace(line);
        let mut historical_clauses = Vec::new();
        for captures in historical_parenthetical_pattern().captures_iter(&normalized_line) {
            if let Some(clause) = captures.get(0) {
                historical_clauses.push(clause.as_str().to_string());
            }
        }
        for captures in historical_formerly_pattern().captures_iter(&normalized_line) {
            if let Some(clause) = captures.get(0) {
                historical_clauses.push(clause.as_str().to_string());
            }
        }
        for captures in renumbered_from_pattern().captures_iter(&normalized_line) {
            if let Some(clause) = captures.get(0) {
                historical_clauses.push(clause.as_str().to_string());
            }
        }
        if let Some(renumbered_to_index) = renumbered_to_pattern()
            .find(&normalized_line)
            .map(|matched| matched.start())
            .filter(|index| *index > 0)
        {
            if let Some(prior_course_code) = self
                .extract_from_line(&normalized_line[..renumbered_to_index])
                .last()
            {
                historical_clauses.push(prior_course_code.clone());
            }
        }
        historical_clauses
            .iter()
            .flat_map(|clause| self.extract_from_line(clause))
            .map(|course_code| normalize_course_code(&course_code))
            .filter(|course_code| !course_code.is_empty())
            .collect()
    }

    fn extract_explicit_codes(&self, line: &str) -> Vec<String> {
        let mut normalized = normalize_whitespace(line);
        if noisy_line_regex().is_match(&normalized) {
            return Vec::new();
        }
        if let Some(noise_match) = transfer_credit_noise_regex().find(&normalized) {
            let before_noise = trim_transfer_noise_punctuation(&normalized[..noise_match.start()]);
            if !explicit_course_code_regex().is_match(&before_noise) {
                return Vec::new();
            }
            normalized = before_noise;
        }

        let explicit_matches = explicit_course_code_regex()
            .captures_iter(&normalized)
            .filter_map(|captures| {
                let whole = captures.get(0)?;
                let subject = captures.get(1)?.as_str();
                let number = captures.get(2)?.as_str();
                if immediately_followed_by_level(&normalized, whole.end()) {
                    return None;
                }
                let normalized_subject = self.normalize_subject(subject)?;
                let explicit_code = self.normalize_code(subject, number)?;
                Some((whole.start(), whole.end(), number.to_string(), normalized_subject, explicit_code))
            })
            .collect::<Vec<_>>();

        let mut codes = Vec::new();
        for (index, (start, end, number, subject, explicit_code)) in
            explicit_matches.iter().enumerate()
        {
            let _ = start;
            codes.push(explicit_code.clone());
            let next_start = explicit_matches
                .get(index + 1)
                .map(|value| value.0)
                .unwrap_or(normalized.len());
            let trailing = &normalized[*end..next_start];
            codes.extend(self.hyphenated_continuations(subject, number, trailing));
            for continuation in continuation_regex().captures_iter(trailing) {
                let Some(whole) = continuation.get(0) else {
                    continue;
                };
                let Some(number) = continuation.get(1) else {
                    continue;
                };
                if !is_ascii_boundary_after(trailing, whole.end())
                    || standalone_parenthetical_continuation(trailing, whole.start(), whole.end())
                    || immediately_followed_by_level(trailing, whole.end())
                {
                    continue;
                }
                codes.extend(self.normalize_code(subject, number.as_str()));
            }
        }

        for alias in subject_aliases() {
            if !alias.contains(' ') {
                continue;
            }
            let escaped = regex::escape(alias).replace(' ', r"\s+");
            let pattern = Regex::new(&format!(r"(?i)\b{escaped}\s+(\d{{3}}[A-Za-z]?)\b"))
                .expect("course subject alias pattern");
            for captures in pattern.captures_iter(&normalized) {
                let Some(whole) = captures.get(0) else {
                    continue;
                };
                let Some(number) = captures.get(1) else {
                    continue;
                };
                if !immediately_followed_by_level(&normalized, whole.end()) {
                    codes.extend(self.normalize_code(alias, number.as_str()));
                }
            }
        }
        unique_sorted(codes)
    }

    fn hyphenated_continuations(
        &self,
        subject: &str,
        first_number: &str,
        trailing: &str,
    ) -> Vec<String> {
        let mut numbers = vec![first_number.to_ascii_uppercase()];
        let mut remainder = trailing;
        loop {
            let Some(captures) = hyphenated_number_regex().captures(remainder) else {
                break;
            };
            let Some(whole) = captures.get(0) else {
                break;
            };
            let Some(number) = captures.get(1) else {
                break;
            };
            numbers.push(number.as_str().to_ascii_uppercase());
            remainder = &remainder[whole.end()..];
        }
        if numbers.len() < 2 {
            return Vec::new();
        }

        let mut continuations = Vec::new();
        if numbers.len() >= 3 {
            continuations.extend(numbers.into_iter().skip(1));
        } else {
            let start = &numbers[0];
            let end = &numbers[1];
            let start_numeric = start.chars().take_while(|character| character.is_ascii_digit()).collect::<String>().parse::<i32>().ok();
            let end_numeric = end.chars().take_while(|character| character.is_ascii_digit()).collect::<String>().parse::<i32>().ok();
            let has_suffix = start.chars().last().is_some_and(|character| character.is_ascii_alphabetic())
                || end.chars().last().is_some_and(|character| character.is_ascii_alphabetic());
            if let (Some(start_numeric), Some(end_numeric)) = (start_numeric, end_numeric) {
                let distance = (end_numeric - start_numeric).abs();
                if has_suffix && start_numeric == end_numeric {
                    continuations.push(end.clone());
                } else if !has_suffix && distance > 0 && distance <= 2 {
                    let direction = if end_numeric > start_numeric { 1 } else { -1 };
                    let mut number = start_numeric + direction;
                    while number != end_numeric + direction {
                        continuations.push(format!("{number:03}"));
                        number += direction;
                    }
                }
            }
        }
        continuations
            .iter()
            .filter_map(|number| self.normalize_code(subject, number))
            .collect()
    }
}

fn normalize_subject(raw_subject: &str, known_subjects: &HashSet<String>) -> Option<String> {
    let raw_subject = normalize_whitespace(raw_subject).to_ascii_uppercase();
    let normalized_subject = subject_alias(&raw_subject)
        .map(str::to_string)
        .unwrap_or(raw_subject);
    let mut tokens = normalized_subject
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    while tokens.len() > 1 && RECOVERABLE_LEADING_SUBJECT_TOKENS.contains(&tokens[0].as_str()) {
        tokens.remove(0);
    }
    if tokens.len() > 1 && LEADING_SUBJECT_TOKENS.contains(&tokens[0].as_str()) {
        tokens.remove(0);
    }
    if tokens.len() > 1 && LEADING_LIST_MARKERS.contains(&tokens[0].as_str()) {
        tokens.remove(0);
    }

    let subject = tokens.join(" ");
    let collapsed = tokens.join("");
    let dangling_ampersand = tokens
        .iter()
        .any(|token| token == "&" || token.ends_with('&'));
    let subject_known = known_subjects.contains(&subject);
    let collapsed_known = known_subjects.contains(&collapsed);
    if tokens.len() > 1 && collapsed_known && !subject_known {
        return Some(collapsed);
    }
    if tokens.is_empty()
        || tokens.len() > 2
        || (tokens.len() == 1 && tokens[0].len() < 2)
        || dangling_ampersand
        || tokens
            .iter()
            .any(|token| token.len() > 8 || !token.chars().all(|character| character.is_ascii_uppercase() || character == '&'))
        || ((INVALID_SUBJECTS.contains(&subject.as_str())
            || tokens.iter().any(|token| INVALID_SUBJECTS.contains(&token.as_str())))
            && !subject_known
            && !collapsed_known)
        || (!subject_known && !collapsed_known)
        || ((subject.ends_with("SKL") || collapsed.ends_with("SKL"))
            && !subject_known
            && !collapsed_known)
    {
        return None;
    }
    Some(subject)
}

fn normalize_code(
    raw_subject: &str,
    raw_number: &str,
    known_subjects: &HashSet<String>,
) -> Option<String> {
    let subject = normalize_subject(raw_subject, known_subjects)?;
    let number = normalize_whitespace(raw_number).to_ascii_uppercase();
    if !course_number_regex().is_match(&number) || number.starts_with("000") {
        return None;
    }
    let code = normalize_course_code_with_subjects(&format!("{subject} {number}"), known_subjects);
    (!code.is_empty()).then_some(code)
}

pub fn normalize_course_code(raw: &str) -> String {
    normalize_course_code_with_subjects(raw, &HashSet::new())
}

fn normalize_course_code_with_subjects(raw: &str, known_subjects: &HashSet<String>) -> String {
    let normalized = normalize_whitespace(raw).to_ascii_uppercase();
    if normalized == "MATH& 254" {
        return "MATH& 264".to_string();
    }
    let Some(captures) = canonical_course_code_regex().captures(&normalized) else {
        return normalized;
    };
    let Some(subject_match) = captures.get(1) else {
        return normalized;
    };
    let Some(number_match) = captures.get(2) else {
        return normalized;
    };
    let mut tokens = subject_match
        .as_str()
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    while tokens.len() > 1
        && RECOVERABLE_LEADING_SUBJECT_TOKENS.contains(&tokens[0].as_str())
    {
        let candidates = tokens[1..].join(" ");
        let collapsed = tokens[1..].join("");
        if known_subjects.contains(&candidates) || known_subjects.contains(&collapsed) {
            tokens.remove(0);
        } else {
            break;
        }
    }
    let subject = if tokens.iter().all(|token| token.len() == 1) {
        tokens.join("")
    } else {
        tokens.join(" ")
    };
    let subject = subject_alias(&subject)
        .map(str::to_string)
        .unwrap_or(subject);
    let code = format!("{subject} {}", number_match.as_str());
    if code == "MATH& 254" { "MATH& 264".to_string() } else { code }
}

fn normalize_known_spaced_subjects(line: &str, known_subjects: &HashSet<String>) -> String {
    let mut output = String::with_capacity(line.len());
    let mut cursor = 0;
    for captures in single_letter_spaced_subject_regex().captures_iter(line) {
        let (Some(whole), Some(leading), Some(trailing), Some(number)) = (
            captures.get(0),
            captures.get(1),
            captures.get(2),
            captures.get(3),
        ) else {
            continue;
        };
        output.push_str(&line[cursor..whole.start()]);
        let spaced = format!("{} {}", leading.as_str(), trailing.as_str()).to_ascii_uppercase();
        let collapsed = format!("{}{}", leading.as_str(), trailing.as_str()).to_ascii_uppercase();
        if !known_subjects.contains(&spaced) && known_subjects.contains(&collapsed) {
            output.push_str(&collapsed);
            output.push(' ');
            output.push_str(number.as_str());
        } else {
            output.push_str(whole.as_str());
        }
        cursor = whole.end();
    }
    output.push_str(&line[cursor..]);
    output
}

fn normalize_course_connector_spacing(line: &str) -> String {
    normalize_course_connector_regex()
        .replace_all(line, "$1 $2")
        .into_owned()
}

fn immediately_followed_by_level(line: &str, match_end: usize) -> bool {
    level_suffix_regex().is_match(&line[match_end..])
}

fn standalone_parenthetical_continuation(segment: &str, start: usize, end: usize) -> bool {
    let matched = &segment[start..end];
    if !matched.trim_start().starts_with('(') {
        return false;
    }
    let after = &segment[end..];
    let Some(close) = parenthetical_close_regex().find(after) else {
        return false;
    };
    let parenthetical = &segment[start..end + close.end()];
    !parenthetical_logic_separator_regex().is_match(parenthetical)
}

fn trim_transfer_noise_punctuation(value: &str) -> String {
    normalize_whitespace(value.trim_end_matches(|character: char| {
        character == ';' || character == ',' || character == ':' || character == ')' || character.is_whitespace()
    }))
}

fn unique_sorted(values: Vec<String>) -> Vec<String> {
    let mut values = values.into_iter().filter(|value| !value.is_empty()).collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn is_course_code_level_reference(text: &str, course_code: &str) -> bool {
    let normalized_course_code = normalize_course_code(course_code);
    let Some(subject_match) = course_code_subject_pattern().captures(&normalized_course_code) else {
        return false;
    };
    let Some(subject) = subject_match.get(1).map(|value| value.as_str()) else {
        return false;
    };
    let Some(level) = course_code_numeric_level_pattern()
        .captures(&normalized_course_code)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str())
    else {
        return false;
    };
    let escaped_subject = regex::escape(subject).replace(" ", r"\s+");
    dynamic_pattern(&format!(r"(?i)\b{escaped_subject}\s+{level}\s*[-\s]*level\b"))
        .is_match(text)
}

fn course_code_mention_pattern(course_code: &str) -> Option<String> {
    let normalized_course_code = normalize_course_code(course_code);
    let captures = course_code_full_pattern().captures(&normalized_course_code)?;
    let subject = captures.get(1)?.as_str();
    let number = captures.get(2)?.as_str();
    let subject_pattern = regex::escape(subject).replace(" ", r"\s+");
    let number_pattern = regex::escape(number);
    Some(format!(r"\b{subject_pattern}\s+{number_pattern}\b"))
}

fn dynamic_pattern(pattern: &str) -> Regex {
    Regex::new(pattern).expect("dynamic course-code evidence pattern")
}

fn course_code_subject_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^([A-Z& ]+)\s+\d").expect("course-code subject pattern"))
}
fn course_code_numeric_level_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b(\d{3})").expect("course-code numeric level pattern"))
}
fn course_code_full_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^([A-Z&]+(?: [A-Z&]+)*)\s+(\d{3}[A-Z]?)$").expect("course-code full pattern"))
}
fn historical_parenthetical_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\([^)]*\b(?:formerly|previously|renumbered)\b[^)]*\)").expect("historical parenthetical pattern"))
}
fn historical_formerly_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:formerly|previously)(?:\s+known\s+as)?\b[^.;)]*").expect("historical formerly pattern"))
}
fn renumbered_from_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\brenumbered\s+from\b[^.;)]*").expect("historical renumbered-from pattern"))
}
fn renumbered_to_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:was\s+)?renumbered\s+(?:as|to)\b").expect("historical renumbered-to pattern"))
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn subject_alias(subject: &str) -> Option<&'static str> {
    Some(match subject {
        "A A" => "AA",
        "A MATH" => "AMATH",
        "ART H" => "ARTH",
        "B GIS" => "BGIS",
        "B HTLH" => "BHLTH",
        "ACCOUNTING" => "ACCTG",
        "ASTRONOMY" => "ASTR",
        "APPLIED MATHEMATICS" => "AMATH",
        "BIOENGINEERING" => "BIOEN",
        "BIOSTATISTICS" => "BIOST",
        "BIOLOGY" => "BIOL",
        "CHEM E" => "CHEME",
        "E E" => "EE",
        "I S" => "IS",
        "IND E" => "INDE",
        "CSSKL" => "CSSSKL",
        "M E" => "ME",
        "MATHEMATICS" => "MATH",
        "PHYSICS" => "PHYS",
        "SPANISH" => "SPAN",
        _ => return None,
    })
}

fn credit_range_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(\d+(?:\.\d+)?)\s*(?:-|to)\s*(\d+(?:\.\d+)?)$").expect("credit range pattern"))
}
fn canonical_course_code_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^([A-Z&]+(?: [A-Z&]+)*) (\d{3}(?:\.\d+)?[A-Z]?)$").expect("course code pattern"))
}
fn course_number_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^\d{3}[A-Z]?$|^000[A-Z]?$").expect("course number pattern"))
}
fn explicit_course_code_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b([A-Z&][A-Za-z&]{0,19}(?:\s+[A-Z&][A-Za-z&]{0,19})?)\s*(\d{3}[A-Za-z]?)\b").expect("explicit course code pattern"))
}
fn continuation_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?:^|[,(;/+&]\s*|\b(?:or|and)\s+)(\d{3}[A-Za-z]?)").expect("course continuation pattern"))
}
fn shared_number_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b([A-Za-z&]+(?:\s+[A-Za-z&]+)*)\s*/\s*([A-Za-z&]+(?:\s+[A-Za-z&]+)*)\s+(\d{3}[A-Za-z]?)\b").expect("shared-number course pattern"))
}
fn repeated_subject_alternative_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b(?:or|and)\s+([A-Za-z&]+(?:\s+[A-Za-z&]+)?)\s*(\d{3}[A-Za-z]?)\b").expect("repeated-subject alternative pattern"))
}
fn hyphenated_number_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^\s*[-–—]\s*(\d{3}[A-Za-z]?)").expect("hyphenated course number pattern"))
}
fn single_letter_spaced_subject_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b([A-Za-z])\s+([A-Za-z&]{2,})\s+(\d{3}[A-Za-z]?\b)").expect("spaced course subject pattern"))
}
fn normalize_course_connector_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b([A-Za-z&]+(?:\s+[A-Za-z&]+)*\s+\d{3})\s*(or|and)\b").expect("course connector spacing pattern"))
}
fn level_suffix_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\s*(?:(?:-\s*)?level\b|\+\s*(?:level\b|courses?\b|$))").expect("course level suffix pattern"))
}
fn parenthetical_close_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^\s*\)").expect("parenthetical close pattern"))
}
fn parenthetical_logic_separator_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:or|and)\b|[,;/+&]").expect("parenthetical continuation pattern"))
}
fn noisy_line_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)(?:src=|srcset=|aria-label=|title=|\.jpg\b|\.jpeg\b|\.png\b|\.svg\b|\.gif\b|^\[page\s+\d+\]\s*%pdf-|^\s*%pdf-|^\s*(?:<!--|-->|(?:pre-bubbling\s+)?cache\s+(?:contexts|tags|max-age):?$|(?:\*\s*)?(?:route\.name\.|url\.site$|languages:|user\.permissions$|theme$))").expect("noisy source line pattern"))
}
fn transfer_credit_noise_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(transfer credits?|transfer equivalenc(?:y|ies)|articulation agreement|community colleges?|community college)\b").expect("transfer credit noise pattern"))
}
fn english_composition_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\benglish composition\b").expect("English composition pattern"))
}
fn english_composition_context_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(5 credits of|five credits of|min(?:imum)?(?:\s+course)? requirements?|prerequisite|admission|complete|completed|required|eligibility|preferred)\b").expect("English composition context pattern"))
}
fn english_composition_exclusion_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(additional english composition|except the 5-?credit english composition requirement|writing-intensive|w course|w courses)\b").expect("English composition exclusion pattern"))
}
fn subject_aliases() -> &'static [&'static str] {
    &[
        "A A", "A MATH", "ART H", "B GIS", "B HTLH", "ACCOUNTING", "ASTRONOMY",
        "APPLIED MATHEMATICS", "BIOENGINEERING", "BIOSTATISTICS", "BIOLOGY", "CHEM E", "E E",
        "I S", "IND E", "CSSKL", "M E", "MATHEMATICS", "PHYSICS", "SPANISH",
    ]
}

fn is_ascii_boundary_after(text: &str, end: usize) -> bool {
    text[end..]
        .chars()
        .next()
        .map_or(true, |character| !character.is_ascii_alphanumeric())
}
