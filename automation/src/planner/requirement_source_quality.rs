use super::requirement_course_codes::normalize_course_code;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashSet};
use std::sync::OnceLock;

const PARSER_RECOVERY_TRIGGER_SIGNAL_CODES: &[&str] = &[
    "no-parsed-uw-course-codes",
    "low-confidence-parsed-source",
    "material-source-structured-drift",
    "large-structured-only-course-gap",
    "high-confidence-low-course-coverage",
    "snapshot-fallback-used",
    "uw-mse-expected-course-option-missing",
    "uw-mse-requirement-course-metadata-missing",
];

const UW_MSE_EXPECTED_OPTION_COURSES: &[&str] = &[
    "AA 260",
    "BIOEN 215",
    "BSE 201",
    "CHEME 355",
    "CSE 123",
    "CSE 143",
    "CSE 160",
    "CSE 164",
    "CSE 180",
    "EE 215",
    "ENGR 101",
    "ENGR 333",
    "ENGR 490",
    "INDE 250",
    "INDE 315",
    "ME 123",
    "ME 230",
    "NME 220",
    "MSE 450",
    "MSE 452",
    "MSE 462",
    "MSE 463",
    "MSE 466",
    "MSE 471",
    "MSE 473",
    "MSE 474",
    "MSE 475",
    "MSE 476",
    "MSE 477",
    "MSE 478",
    "MSE 479",
    "MSE 481",
    "MSE 482",
    "MSE 483",
    "MSE 484",
    "MSE 486",
    "MSE 487",
    "MSE 488",
    "MSE 489",
    "MSE 490",
    "MSE 498",
    "MSE 499",
    "AMATH 352",
    "AMATH 353",
    "AMATH 383",
    "AMATH 401",
    "AMATH 403",
    "BIOC 405",
    "BIOC 406",
    "CHEM 312",
    "CHEM 455",
    "CHEM 456",
    "CHEM 457",
    "CHEME 341",
    "ENGR 321",
    "ENVIR 480",
    "PHYS 321",
    "PHYS 324",
    "PHYS 325",
    "PHYS 334",
    "PHYS 335",
    "PHYS 434",
    "PHYS 441",
    "ENTRE 370",
    "ENTRE 440",
];

pub fn build_parse_quality_signals(owner: &Value) -> Vec<Value> {
    let parsed_codes = string_array(owner, "parsedUwCourseCodes");
    let source_only_codes = string_array(owner, "sourceOnlyUwCourseCodes");
    let structured_only_codes = string_array(owner, "structuredOnlyUwCourseCodes");
    let requirement_cues = string_array(owner, "requirementCueLines");
    let choose_statements = string_array(owner, "chooseStatements");
    let parsed_count = parsed_codes.len();
    let source_only_count = source_only_codes.len();
    let actionable_structured_only = get_actionable_structured_only_course_codes(
        &structured_only_codes,
        &requirement_cues,
        &choose_statements,
    );
    let structured_only_count = actionable_structured_only.len();
    let structured_coverage_count =
        parsed_count.saturating_sub(source_only_count) + structured_only_count;
    let can_schedule = owner
        .get("canCreateSchedulableRows")
        .and_then(Value::as_bool)
        != Some(false);
    let mut signals = Vec::new();

    if owner.get("sourceInactiveMajor").and_then(Value::as_bool) == Some(true) {
        push_signal(
            &mut signals,
            "note",
            "inactive-major-source",
            "The official source says this major is not currently accepting students, so missing schedulable course rows are not actionable parser debt.",
            Some(format!(
                "source-role={}; requirement-cues={}",
                owner
                    .get("sourceRole")
                    .and_then(Value::as_str)
                    .unwrap_or("ignored"),
                requirement_cues.len()
            )),
        );
    }
    if owner.get("ok").is_some_and(is_truthy) && parsed_count == 0 && can_schedule {
        push_signal(
            &mut signals,
            "warning",
            "no-parsed-uw-course-codes",
            "The official source parsed successfully but did not yield usable UW course codes.",
            Some(format!(
                "source-role={}; requirement-cues={}",
                owner
                    .get("sourceRole")
                    .and_then(Value::as_str)
                    .unwrap_or("ignored"),
                requirement_cues.len()
            )),
        );
    }
    if owner.get("ok").is_some_and(is_truthy)
        && owner.get("parseConfidence").and_then(Value::as_str) == Some("low")
        && can_schedule
    {
        push_signal(
            &mut signals,
            "warning",
            "low-confidence-parsed-source",
            "The official source parsed with low confidence and should attempt deeper recovery before manual review.",
            Some(format!(
                "parsed={parsed_count}; requirement-cues={}; choose-statements={}",
                requirement_cues.len(),
                choose_statements.len()
            )),
        );
    }
    if can_schedule && structured_only_count >= 5 {
        push_signal(
            &mut signals,
            "warning",
            "material-source-structured-drift",
            "Parsed source course coverage diverges materially from the structured degree-map coverage.",
            Some(format!(
                "parsed={parsed_count}; source-only={source_only_count}; structured-only={structured_only_count}"
            )),
        );
    }
    if can_schedule && structured_only_count >= 8 {
        push_signal(
            &mut signals,
            "warning",
            "large-structured-only-course-gap",
            "Structured degree-map coverage includes many UW course codes that were not recovered from the parsed source.",
            Some(format!(
                "structured-only={structured_only_count}; parsed={parsed_count}; structured-coverage={structured_coverage_count}"
            )),
        );
    }
    if can_schedule
        && owner.get("parseConfidence").and_then(Value::as_str) == Some("high")
        && structured_coverage_count >= 8
        && (parsed_count <= 2.max(structured_coverage_count / 3)
            || structured_only_count >= 6.max((structured_coverage_count * 3 + 4) / 5))
    {
        push_signal(
            &mut signals,
            "warning",
            "high-confidence-low-course-coverage",
            "The parser reported high confidence, but the recovered UW course coverage looks suspiciously low for this owner.",
            Some(format!(
                "parsed={parsed_count}; structured-coverage={structured_coverage_count}; structured-only={structured_only_count}"
            )),
        );
    }
    if owner.get("usedSnapshotFallback").is_some_and(is_truthy) {
        push_signal_with_json_details(
            &mut signals,
            "note",
            "snapshot-fallback-used",
            "Parsing relied on a cached snapshot after a live-source failure.",
            owner
                .get("snapshotFallbackReason")
                .filter(|value| !value.is_null())
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
    if owner.get("usedSnapshotFallback").is_some_and(is_truthy)
        && can_schedule
        && owner
            .get("snapshotHasHeadingMetadata")
            .and_then(Value::as_bool)
            == Some(false)
    {
        push_signal(
            &mut signals,
            "note",
            "snapshot-fallback-heading-context-missing",
            "Cached snapshot fallback did not include source heading metadata, which limits section-scoped parser validation.",
            Some(format!(
                "snapshot={}; requirement-cues={}; choose-statements={}",
                owner
                    .get("snapshotPath")
                    .and_then(Value::as_str)
                    .unwrap_or("n/a"),
                requirement_cues.len(),
                choose_statements.len()
            )),
        );
    }
    if owner.get("resolutionStrategy").and_then(Value::as_str) == Some("alternate-official-source")
    {
        let details = if owner.get("sourceUrl") == owner.get("primarySourceUrl") {
            None
        } else {
            owner
                .get("sourceUrl")
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        push_signal(
            &mut signals,
            "note",
            "alternate-official-source-used",
            "Parsing succeeded by switching from the primary source URL to an alternate official source URL.",
            details,
        );
    }
    append_mse_quality_signals(owner, &mut signals);
    signals
}

pub fn parser_recovery_trigger_codes(owner: &Value) -> Vec<String> {
    if !owner.get("ok").is_some_and(is_truthy) {
        return Vec::new();
    }
    let signals = owner
        .get("qualitySignals")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| build_parse_quality_signals(owner));
    let mut codes = Vec::new();
    for code in signals
        .iter()
        .filter_map(|signal| signal.get("code").and_then(Value::as_str))
        .filter(|code| PARSER_RECOVERY_TRIGGER_SIGNAL_CODES.contains(code))
    {
        if !codes
            .iter()
            .any(|existing: &String| existing.as_str() == code)
        {
            codes.push(code.to_string());
        }
    }
    if owner.get("parseConfidence").and_then(Value::as_str) == Some("low")
        && owner
            .get("canCreateSchedulableRows")
            .and_then(Value::as_bool)
            != Some(false)
        && !codes
            .iter()
            .any(|code| code.as_str() == "low-confidence-parsed-source")
    {
        codes.push("low-confidence-parsed-source".to_string());
    }
    codes
}

fn append_mse_quality_signals(owner: &Value, signals: &mut Vec<Value>) {
    if owner.get("planId").and_then(Value::as_str)
        != Some("uw-seattle-materials-science-engineering")
        || owner.get("pathwayId").is_some_and(|value| is_truthy(value))
    {
        return;
    }
    let courses = owner
        .get("parsedRequirementCourses")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let actual_codes = courses
        .iter()
        .filter_map(|course| course.get("normalizedCourseCode").and_then(Value::as_str))
        .map(normalize_course_code)
        .filter(|code| !code.is_empty())
        .collect::<HashSet<_>>();
    let missing_expected = UW_MSE_EXPECTED_OPTION_COURSES
        .iter()
        .map(|code| normalize_course_code(code))
        .filter(|code| !actual_codes.contains(code))
        .collect::<Vec<_>>();
    if !missing_expected.is_empty() {
        push_signal(
            signals,
            "warning",
            "uw-mse-expected-course-option-missing",
            "UW MSE parsed requirement course coverage is missing expected official option courses.",
            Some(missing_expected.join(", ")),
        );
    }
    let missing_metadata = courses
        .iter()
        .filter(|course| {
            [
                "requirementGroupId",
                "requirementType",
                "optionRole",
                "sourceHeading",
                "category",
            ]
            .iter()
            .any(|field| !has_nonempty_value(course.get(field)))
        })
        .filter_map(|course| course.get("normalizedCourseCode").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !missing_metadata.is_empty() {
        push_signal(
            signals,
            "warning",
            "uw-mse-requirement-course-metadata-missing",
            "UW MSE parsed requirement courses are missing group/type/source metadata.",
            Some(missing_metadata.join(", ")),
        );
    }
}

fn get_actionable_structured_only_course_codes(
    structured_only_codes: &[String],
    requirement_cues: &[String],
    choose_statements: &[String],
) -> Vec<String> {
    let normalized_codes = structured_only_codes
        .iter()
        .map(|code| normalize_course_code(code))
        .filter(|code| !code.is_empty())
        .collect::<BTreeSet<_>>();
    if normalized_codes.is_empty() {
        return Vec::new();
    }
    let mut evidence_lines = Vec::new();
    for line in requirement_cues.iter().chain(choose_statements) {
        let normalized = super::requirement_shape::normalize_js_whitespace(line);
        if !normalized.is_empty() && !evidence_lines.contains(&normalized) {
            evidence_lines.push(normalized);
        }
    }
    if evidence_lines.is_empty() {
        return normalized_codes.into_iter().collect();
    }
    normalized_codes
        .into_iter()
        .filter(|code| {
            !evidence_lines.iter().any(|line| {
                non_actionable_structured_only_hint_pattern().is_match(line)
                    && source_line_mentions_course_code_or_abbreviated_sequence(line, code)
            })
        })
        .collect()
}

fn source_line_mentions_course_code_or_abbreviated_sequence(line: &str, course_code: &str) -> bool {
    let normalized_line = super::requirement_shape::normalize_js_whitespace(line).to_uppercase();
    let normalized_course_code = normalize_course_code(course_code);
    if normalized_line.is_empty() || normalized_course_code.is_empty() {
        return false;
    }
    if normalized_line.contains(&normalized_course_code) {
        return true;
    }
    let Some((subject, number)) = normalized_course_code.rsplit_once(' ') else {
        return false;
    };
    let number_bytes = number.as_bytes();
    let valid_number = match number_bytes.len() {
        3 => number_bytes.iter().all(u8::is_ascii_digit),
        4 => {
            number_bytes[..3].iter().all(u8::is_ascii_digit) && number_bytes[3].is_ascii_uppercase()
        }
        _ => false,
    };
    let valid_subject = !subject.is_empty()
        && subject.chars().all(|character| {
            character.is_ascii_uppercase() || character == '&' || character == ' '
        });
    if !valid_subject || !valid_number {
        return false;
    }
    let Some(subject_index) = normalized_line.find(subject) else {
        return false;
    };
    let number_pattern = Regex::new(&format!(r"(?-u:\b){}(?-u:\b)", regex::escape(number)))
        .expect("escaped course number pattern");
    number_pattern.is_match(&normalized_line[subject_index + subject.len()..])
}

fn string_array(owner: &Value, field: &str) -> Vec<String> {
    owner
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

pub fn build_uw_mse_course_extraction_audit_for_owner(owner: &Value) -> Value {
    let courses = owner
        .get("parsedRequirementCourses")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let normalized_codes = courses
        .iter()
        .filter_map(|course| course.get("normalizedCourseCode"))
        .filter(|value| is_truthy(value))
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string())
        })
        .collect::<Vec<_>>();
    let normalized_code_set = normalized_codes.iter().cloned().collect::<HashSet<_>>();
    let mut category_counts = serde_json::Map::new();
    let mut option_role_counts = serde_json::Map::new();
    let mut code_counts = std::collections::HashMap::<String, usize>::new();
    for code in &normalized_codes {
        *code_counts.entry(code.clone()).or_default() += 1;
    }
    for course in courses {
        let category = course
            .get("category")
            .filter(|value| is_truthy(value))
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_else(|| "unclassified".to_string());
        increment_json_count(&mut category_counts, category);
        let option_role = course
            .get("optionRole")
            .filter(|value| is_truthy(value))
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_else(|| "unclassified".to_string());
        increment_json_count(&mut option_role_counts, option_role);
    }
    let duplicate_codes = code_counts
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(code, _)| code.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let missing_expected = UW_MSE_EXPECTED_OPTION_COURSES
        .iter()
        .map(|code| normalize_course_code(code))
        .filter(|code| !normalized_code_set.contains(code))
        .collect::<Vec<_>>();
    let unclassified_codes = courses
        .iter()
        .filter(|course| {
            !course.get("category").is_some_and(is_truthy)
                || course.get("category").and_then(Value::as_str) == Some("unclassified")
        })
        .map(|course| {
            course
                .get("normalizedCourseCode")
                .cloned()
                .unwrap_or(Value::Null)
        })
        .collect::<Vec<_>>();
    let missing_metadata_codes = courses
        .iter()
        .filter(|course| {
            [
                "requirementGroupId",
                "requirementType",
                "optionRole",
                "sourceHeading",
                "category",
            ]
            .iter()
            .any(|key| !course.get(*key).is_some_and(is_truthy))
        })
        .map(|course| {
            course
                .get("normalizedCourseCode")
                .cloned()
                .unwrap_or(Value::Null)
        })
        .collect::<Vec<_>>();
    let note_entries = courses
        .iter()
        .filter(|course| js_length_is_positive(course.get("notes")))
        .map(|course| {
            let mut entry = serde_json::Map::new();
            for key in [
                "courseCode",
                "normalizedCourseCode",
                "requirementGroupId",
                "optionRole",
            ] {
                if let Some(value) = course.get(key) {
                    entry.insert(key.to_string(), value.clone());
                }
            }
            entry.insert(
                "notes".to_string(),
                course.get("notes").cloned().unwrap_or_else(|| json!([])),
            );
            Value::Object(entry)
        })
        .collect::<Vec<_>>();
    let option_count = |key: &str| {
        option_role_counts
            .get(key)
            .cloned()
            .unwrap_or_else(|| json!(0))
    };
    let mut result = serde_json::Map::new();
    result.insert(
        "ownerId".to_string(),
        owner
            .get("ownerId")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or(Value::Null),
    );
    result.insert(
        "totalParsedOfficialEntries".to_string(),
        json!(courses.len()),
    );
    result.insert(
        "groupedCountsByCategory".to_string(),
        Value::Object(category_counts),
    );
    result.insert(
        "missingExpectedCourses".to_string(),
        json!(missing_expected),
    );
    result.insert(
        "duplicateNormalizedCourseCodes".to_string(),
        json!(duplicate_codes),
    );
    result.insert(
        "unclassifiedCourseCodes".to_string(),
        json!(unclassified_codes),
    );
    result.insert(
        "coursesMissingRequiredMetadata".to_string(),
        json!(missing_metadata_codes),
    );
    result.insert("noteOrRestrictionEntries".to_string(), json!(note_entries));
    result.insert(
        "requiredVsOptionCounts".to_string(),
        json!({
            "required": option_count("required"),
            "option": option_count("option"),
            "alias": option_count("alias"),
            "noteOnly": option_count("note_only"),
        }),
    );
    Value::Object(result)
}

fn increment_json_count(counts: &mut serde_json::Map<String, Value>, key: String) {
    let next_count = counts.get(&key).and_then(Value::as_u64).unwrap_or(0) + 1;
    counts.insert(key, json!(next_count));
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn has_nonempty_value(value: Option<&Value>) -> bool {
    value.is_some_and(is_truthy)
}

pub(super) fn js_length_is_positive(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Array(values)) => !values.is_empty(),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Object(properties)) => properties
            .get("length")
            .and_then(js_value_to_number)
            .is_some_and(|length| length > 0.0),
        _ => false,
    }
}

fn js_value_to_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(value) => value.as_f64(),
        Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        Value::Null => Some(0.0),
        Value::String(value) => {
            let normalized = super::requirement_shape::normalize_js_whitespace(value);
            if normalized.is_empty() {
                Some(0.0)
            } else {
                normalized.parse::<f64>().ok()
            }
        }
        Value::Array(values) if values.is_empty() => Some(0.0),
        Value::Array(values) if values.len() == 1 => match &values[0] {
            Value::String(value) => {
                let normalized = super::requirement_shape::normalize_js_whitespace(value);
                if normalized.is_empty() {
                    Some(0.0)
                } else {
                    normalized.parse::<f64>().ok()
                }
            }
            Value::Number(value) => value.as_f64(),
            Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
            Value::Null => Some(0.0),
            _ => None,
        },
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn push_signal(
    signals: &mut Vec<Value>,
    severity: &str,
    code: &str,
    message: &str,
    details: Option<String>,
) {
    signals.push(json!({
        "severity": severity,
        "code": code,
        "message": message,
        "details": details,
    }));
}

fn push_signal_with_json_details(
    signals: &mut Vec<Value>,
    severity: &str,
    code: &str,
    message: &str,
    details: Value,
) {
    signals.push(json!({
        "severity": severity,
        "code": code,
        "message": message,
        "details": details,
    }));
}

fn non_actionable_structured_only_hint_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"(?i)(?-u:\b)(?:advis(?:e|er|or|ing)|if\s+completing|excludes?|excluded|not\s+accepted|not\s+count|cannot\s+count|may\s+not\s+count|for\s+major\s+planning\s+only)(?-u:\b)|(?-u:\b)(?:suggested|recommended)(?-u:\b).{0,80}(?-u:\b)(?:first|second|college|courses?)(?-u:\b)")
            .expect("structured-only quality evidence pattern")
    })
}
