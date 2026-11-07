use super::requirement_ids::slugify_requirement_identifier;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::OnceLock;

pub fn detect_option_replacement(
    owner: &Value,
    snapshot_lines: &[String],
    source_url: Option<&Value>,
) -> Option<Value> {
    let lines = snapshot_lines
        .iter()
        .map(|line| normalize_whitespace(line))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let context_text = get_owner_context_text(owner);
    let combined = lines.join(" ");
    let option_acronym = extract_option_acronym(&format!("{context_text} {combined}"))
        .or_else(|| extract_option_acronym(&combined))?;
    let owner_looks_like_option = dynamic_pattern(&format!(
        r"(?i)\b{}\b",
        regex::escape(&option_acronym)
    ))
    .is_match(&context_text);
    if !owner_looks_like_option {
        return None;
    }
    let replacement_line = lines.iter().find(|line| {
        except_requirement_pattern().is_match(line)
            && in_place_of_pattern().is_match(line)
            && complete_pattern().is_match(line)
    })?;
    let base_match = base_replacement_pattern().captures(replacement_line)?;
    let replacement_match = replacement_requirement_pattern().captures(replacement_line)?;
    let base_credits = base_match.get(1)?.as_str().parse::<f64>().ok()?;
    let replacement_credits = replacement_match.get(1)?.as_str().parse::<f64>().ok()?;
    if !base_credits.is_finite() || !replacement_credits.is_finite() {
        return None;
    }
    let explicit_owner_prefix = explicit_owner_prefix_pattern()
        .captures(replacement_line)
        .and_then(|captures| captures.get(1))
        .map(|capture| capture.as_str().to_string())
        .or_else(|| {
            dynamic_pattern(&format!(
                r"(?i)\b([A-Z]{{2,8}})\s+{}\s+requirements\b",
                regex::escape(&option_acronym)
            ))
            .captures(&combined)
            .and_then(|captures| captures.get(1))
            .map(|capture| capture.as_str().to_string())
        });
    let fallback_program_name = owner
        .get("ownerTitle")
        .filter(|value| !value.is_null())
        .or_else(|| owner.get("planId"));
    let fallback_program_name = fallback_program_name
        .map(js_string_value)
        .unwrap_or_default();
    let owner_prefix = explicit_owner_prefix
        .unwrap_or_else(|| get_program_acronym(&fallback_program_name))
        .to_lowercase();
    let base_label = base_match.get(2)?.as_str();
    let replacement_label = replacement_match.get(2)?.as_str();
    let source_heading = lines
        .iter()
        .find(|line| {
            dynamic_pattern(&format!(
                r"(?i)\b{}\b.*\brequirements?\b",
                regex::escape(&option_acronym)
            ))
            .is_match(line)
        })
        .cloned()
        .unwrap_or_else(|| format!("{option_acronym} Option requirements"));
    let normalized_base_label = normalize_whitespace(base_label);
    let normalized_replacement_label = normalize_whitespace(replacement_label);
    let replacement_reason = format!(
        "{option_acronym} Option students complete {} credits of {} instead of the standard {}-credit {} requirement.",
        js_number_string(replacement_credits),
        normalized_replacement_label,
        js_number_string(base_credits),
        normalized_base_label,
    );
    let owner_id = owner
        .get("ownerId")
        .filter(|value| js_truthy(value))
        .or_else(|| owner.get("planId").filter(|value| js_truthy(value)))
        .map(js_string_value)
        .unwrap_or_else(|| "unknown-owner".to_string());
    let source_url = source_url
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or(Value::Null);
    let evidence = unique_in_order(vec![source_heading.clone(), replacement_line.to_string()]);

    Some(json!({
        "baseRequirementId": build_requirement_group_id_from_replacement(
            &owner_id,
            &owner_prefix,
            &normalized_base_label,
            base_credits,
        ),
        "replacedByRequirementId": build_requirement_group_id_from_replacement(
            &owner_id,
            &owner_prefix,
            &normalized_replacement_label,
            replacement_credits,
        ),
        "appliesWhen": format!("selectedOption === \"{option_acronym}\""),
        "replacementReason": replacement_reason,
        "sourceUrl": source_url,
        "sourceHeading": source_heading,
        "optionAcronym": option_acronym,
        "replacementCredits": replacement_credits,
        "baseCredits": base_credits,
        "rule": {
            "ruleId": "option-replacement-group",
            "confidence": "high",
            "evidence": evidence.into_iter().take(5).collect::<Vec<_>>(),
        },
    }))
}

pub fn get_parser_rule_registry() -> Vec<Value> {
    vec![
        json!({
            "id": "option-replacement-group",
            "appliesTo": {
                "sourceRoles": ["primary-degree-requirements", "department-requirements", "pathway-degree-sheet"],
                "pageTypes": ["html-degree-page", "pdf-degree-sheet", "generic-html"],
            },
            "detector": {"cues": [
                "option requirements", "except the ... requirement", "in place of",
                "core and elective requirements below", "may be replaced by",
            ]},
            "extractor": "extractOptionReplacementAndSectionedCourseGroup",
            "confidence": "high-with-source-heading-and-course-section",
        }),
        json!({
            "id": "sectioned-course-group",
            "appliesTo": {
                "sourceRoles": ["primary-degree-requirements", "department-requirements", "pathway-degree-sheet"],
                "pageTypes": ["html-degree-page", "pdf-degree-sheet", "generic-html"],
            },
            "detector": {"cues": [
                "selected from the following list", "courses listed below",
                "minimum of ... credits", "maximum of ... credits", "one/two electives",
            ]},
            "extractor": "extractSectionedCourseRequirementGroup",
            "confidence": "high-with-section-heading-and-course-rows",
        }),
        json!({
            "id": "credit-bucket",
            "appliesTo": {
                "sourceRoles": ["primary-degree-requirements", "department-requirements", "official-catalog"],
                "pageTypes": ["html-degree-page", "pdf-degree-sheet", "catalog-page", "generic-html"],
            },
            "detector": {"cues": [
                "additional credits from", "approved natural science", "Math/Science",
                "approved electives", "credit range",
            ]},
            "extractor": "extractCreditBucketPlaceholder",
            "confidence": "high-with-credit-range-and-approved-list-cue",
        }),
        json!({
            "id": "sequence-or-either-or",
            "appliesTo": {
                "sourceRoles": ["primary-degree-requirements", "department-requirements", "official-catalog"],
                "pageTypes": ["html-degree-page", "pdf-degree-sheet", "catalog-page", "generic-html"],
            },
            "detector": {"cues": ["or", "either", "choose one sequence", "one of the following"]},
            "extractor": "extractAlternativeCourseOrSequenceGroup",
            "confidence": "high-with-explicit-or-or-sequence-cue",
        }),
        json!({
            "id": "support-source-filter",
            "appliesTo": {
                "sourceRoles": [
                    "approved-course-list", "elective-list", "sample-schedule",
                    "upper-division-prerequisite-table", "non-schedulable-course-list",
                ],
                "pageTypes": ["html-degree-page", "pdf-degree-sheet", "generic-html"],
            },
            "detector": {"cues": [
                "approved course list", "course list", "sample schedule", "policy",
                "count toward", "support-only",
            ]},
            "extractor": "filterSupportOnlyCourseEvidence",
            "confidence": "high-with-support-role-or-support-only-line-cue",
        }),
    ]
}

pub fn get_parser_rule(rule_id: &str) -> Option<Value> {
    get_parser_rule_registry()
        .into_iter()
        .find(|rule| rule.get("id").and_then(Value::as_str) == Some(rule_id))
}

pub fn build_rule_evidence(
    rule_id: &str,
    evidence_lines: &[String],
    confidence: Option<&str>,
) -> Value {
    let evidence = unique_in_order(
        evidence_lines
            .iter()
            .map(|line| normalize_whitespace(line))
            .filter(|line| !line.is_empty())
            .collect(),
    )
    .into_iter()
    .take(5)
    .collect::<Vec<_>>();
    json!({
        "ruleId": rule_id,
        "confidence": confidence.unwrap_or("medium"),
        "evidence": evidence,
    })
}

pub fn slugify_parser_rule_value(value: &str) -> String {
    let ampersand_expanded = normalize_whitespace(value).replace('&', " and ");
    slugify_requirement_identifier(&ampersand_expanded)
}

pub fn should_join_credit_bucket_continuation(first_line: &str) -> bool {
    let first_line = normalize_whitespace(first_line);
    !first_line.is_empty() && credit_bucket_join_pattern().is_match(&first_line)
}

pub fn is_credit_bucket_continuation_line(line: &str) -> bool {
    let normalized_line = normalize_whitespace(line);
    if normalized_line.is_empty() {
        return false;
    }
    credit_bucket_page_pattern().is_match(&normalized_line)
        || credit_bucket_lowercase_or_parenthesis_pattern().is_match(&normalized_line)
        || credit_bucket_keyword_pattern().is_match(&normalized_line)
}

pub fn filter_course_codes_by_support_only_evidence(
    course_codes: &[String],
    source_line_hints_by_course: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    unique_in_order(course_codes.to_vec())
        .into_iter()
        .filter(|course_code| {
            let Some(hints) = source_line_hints_by_course.get(course_code) else {
                return true;
            };
            hints.is_empty() || hints.iter().any(|hint| !has_support_only_line_cue(hint))
        })
        .collect()
}

pub fn has_support_only_line_cue(line: &str) -> bool {
    let normalized_line = normalize_whitespace(line);
    if normalized_line.is_empty() {
        return false;
    }
    if after_completing_may_take_pattern().is_match(&normalized_line)
        || (fulfill_institution_requirements_pattern().is_match(&normalized_line)
            && (may_also_take_pattern().is_match(&normalized_line)
                || may_take_additional_courses_pattern().is_match(&normalized_line)
                || encouraged_take_or_enroll_pattern().is_match(&normalized_line)))
    {
        return true;
    }
    if count_toward_support_pattern().is_match(&normalized_line)
        || support_requirement_count_pattern().is_match(&normalized_line)
    {
        return true;
    }
    if required_choice_cue_pattern().is_match(&normalized_line) {
        return false;
    }
    support_list_or_count_pattern().is_match(&normalized_line)
        || list_on_the_pattern().is_match(&normalized_line)
        || generic_support_count_toward_pattern().is_match(&normalized_line)
        || generic_count_toward_support_pattern().is_match(&normalized_line)
}

pub fn detect_program_approved_credit_bucket(text: &str, owner: &Value) -> Option<Value> {
    let normalized_text = normalize_whitespace(text);
    if normalized_text.is_empty() || !approved_pattern().is_match(&normalized_text) {
        return None;
    }
    let has_natural_science = natural_science_bucket_pattern().is_match(&normalized_text);
    let has_math_science = math_science_bucket_pattern().is_match(&normalized_text);
    if !has_natural_science && !has_math_science {
        return None;
    }
    let program_name = extract_program_name_from_bucket_text(&normalized_text, owner)?;
    let program_slug = slugify_parser_rule_value(&program_name);
    let program_acronym = get_program_acronym(&program_name);
    let (bucket_slug, bucket_category, bucket_label) = if has_math_science {
        ("math-science", "MATH_SCIENCE", "Math/Science")
    } else {
        ("natural-science", "NATURAL_SCIENCE", "Natural Science")
    };
    Some(json!({
        "category": format!("{program_acronym}_{bucket_category}"),
        "sourceCategoryCode": format!("{program_acronym} {bucket_label}"),
        "longLabel": format!("approved {program_name} {bucket_label}"),
        "approvedListKey": format!("{program_slug}-{bucket_slug}"),
        "programSpecific": true,
        "rule": {
            "ruleId": "credit-bucket",
            "confidence": "high",
            "evidence": [normalized_text],
        },
    }))
}

pub fn has_option_replacement_requirement_cue(line: &str) -> bool {
    let normalized_line = normalize_whitespace(line);
    !normalized_line.is_empty()
        && (replacement_cue_before_requirements_pattern().is_match(&normalized_line)
            || replacement_cue_after_requirements_pattern().is_match(&normalized_line)
            || core_and_elective_requirements_pattern().is_match(&normalized_line)
            || elective_credit_requirement_pattern().is_match(&normalized_line)
            || technical_elective_requirements_pattern().is_match(&normalized_line))
}

fn extract_option_acronym(text: &str) -> Option<String> {
    option_acronym_pattern()
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|capture| capture.as_str().to_string())
}

fn get_owner_context_text(owner: &Value) -> String {
    [
        "ownerTitle",
        "sourceLabel",
        "primarySourceLabel",
        "sourceUrl",
        "primarySourceUrl",
        "planId",
        "ownerId",
    ]
    .iter()
    .filter_map(|key| owner.get(*key).filter(|value| js_truthy(value)))
    .map(js_string_value)
    .collect::<Vec<_>>()
    .join(" ")
}

fn extract_program_name_from_bucket_text(text: &str, owner: &Value) -> Option<String> {
    for pattern in program_name_patterns() {
        if let Some(program_name) = pattern
            .captures(text)
            .and_then(|captures| captures.get(1))
            .map(|capture| clean_program_name(capture.as_str()))
            .filter(|program_name| {
                !program_name.is_empty()
                    && !["approved", "additional", "natural", "math", "mathematics"]
                        .iter()
                        .any(|generic| program_name.eq_ignore_ascii_case(generic))
            })
        {
            return Some(program_name);
        }
    }
    let owner_title = owner
        .get("ownerTitle")
        .filter(|value| !value.is_null())
        .map(js_string_value)
        .map(|title| clean_program_name(&title))
        .filter(|title| !title.is_empty());
    owner_title.filter(|title| {
        dynamic_pattern(&format!(r"(?i)\b{}\b", regex::escape(title)))
            .is_match(&get_owner_context_text(owner))
    })
}

fn clean_program_name(value: &str) -> String {
    let normalized = normalize_whitespace(value);
    let normalized = clean_program_name_tail_pattern().replace(&normalized, "");
    let normalized = clean_program_name_suffix_pattern().replace(&normalized, "");
    let normalized = clean_program_name_punctuation_pattern().replace_all(&normalized, "");
    normalize_whitespace(&normalized)
}

fn get_program_acronym(program_name: &str) -> String {
    let normalized = normalize_whitespace(program_name);
    if normalized.is_empty() {
        return "PROGRAM".to_string();
    }
    let normalized_tokens = normalized.replace('&', " and ");
    let tokens = normalized_tokens
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| {
            !token.is_empty()
                && !["and", "of", "the", "for", "in", "with", "science", "sciences"]
                    .iter()
                    .any(|stop| token.eq_ignore_ascii_case(stop))
        })
        .collect::<Vec<_>>();
    let acronym = tokens
        .iter()
        .filter_map(|token| token.chars().next())
        .map(|character| character.to_ascii_uppercase())
        .collect::<String>();
    if !acronym.is_empty() {
        return acronym;
    }
    let fallback = normalized
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(4)
        .map(|character| character.to_ascii_uppercase())
        .collect::<String>();
    if fallback.is_empty() {
        "PROGRAM".to_string()
    } else {
        fallback
    }
}

fn build_requirement_group_id_from_replacement(
    owner_id: &str,
    owner_prefix: &str,
    label: &str,
    credits: f64,
) -> String {
    let label_slug = normalize_requirement_slug(label);
    let credit_slug = format!("{}-credits", js_number_string(credits));
    let owner_prefix = if owner_prefix.is_empty() {
        String::new()
    } else {
        format!("{}-", slugify_parser_rule_value(owner_prefix))
    };
    format!("{owner_id}:requirement-group:{owner_prefix}{label_slug}-{credit_slug}")
}

fn normalize_requirement_slug(label: &str) -> String {
    let normalized = normalize_whitespace(label);
    let normalized = requirement_slug_credits_pattern().replace_all(&normalized, "");
    let normalized = requirement_slug_requirements_pattern().replace_all(&normalized, "");
    let normalized = requirement_slug_the_pattern().replace_all(&normalized, "");
    let normalized = requirement_slug_and_pattern().replace_all(&normalized, " ");
    let normalized = normalize_whitespace(&normalized);
    let normalized = technical_elective_singular_pattern().replace(&normalized, "technical electives");
    let normalized = core_elective_singular_pattern().replace(&normalized, "core elective");
    slugify_parser_rule_value(normalized.trim())
}

fn unique_in_order(values: Vec<String>) -> Vec<String> {
    let mut unique = Vec::new();
    for value in values.into_iter().filter(|value| !value.is_empty()) {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn js_string_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value
            .as_f64()
            .map(js_number_string)
            .unwrap_or_else(|| value.to_string()),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(|value| if value.is_null() { String::new() } else { js_string_value(value) })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn js_number_string(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn dynamic_pattern(pattern: &str) -> Regex {
    Regex::new(pattern).expect("parser-rule evidence pattern")
}

fn credit_bucket_join_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:list of approved|approved|math/science|math\s*&\s*science|natural science courses for)\b").expect("credit-bucket continuation join pattern"))
}
fn credit_bucket_page_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^\[Page\s+\d+\]").expect("credit-bucket page continuation pattern"))
}
fn credit_bucket_lowercase_or_parenthesis_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^[a-z(]").expect("credit-bucket lowercase continuation pattern"))
}
fn credit_bucket_keyword_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:approved|natural science|Math/Science|Math\s*&\s*Science|Computer Engineering|CSE website|website|STAT|MATH|AMATH)\b").expect("credit-bucket keyword continuation pattern"))
}

fn approved_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bapproved\b").expect("approved bucket pattern"))
}
fn natural_science_bucket_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bnatural sciences?\b|\bN\s*Sc\b|\bNSC\b").expect("natural science bucket pattern"))
}
fn math_science_bucket_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bmath(?:ematics)?/science\b|\bmath(?:ematics)?\s*&\s*science\b").expect("math science bucket pattern"))
}
fn program_name_patterns() -> &'static [Regex] {
    static VALUE: OnceLock<Vec<Regex>> = OnceLock::new();
    VALUE.get_or_init(|| {
        [
            r"(?i)\bnatural science courses for\s+([A-Z][A-Za-z&/\s-]{2,80}?)(?:\s+on\b|\s+from\b|\s+website\b|[.;,)]|$)",
            r"(?i)\bapproved\s+([A-Z][A-Za-z&/\s-]{2,80}?)\s+(?:natural science|math/science|math\s*&\s*science)\b",
            r"(?i)\b([A-Z][A-Za-z&/\s-]{2,80}?)\s+(?:natural science|math/science|math\s*&\s*science)\s+(?:requirement|courses?|list)\b",
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("approved program name pattern"))
        .collect()
    })
}
fn clean_program_name_tail_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:on|from|at|via|website|course|courses|list|requirement|requirements)\b.*$").expect("program name tail pattern"))
}
fn clean_program_name_suffix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:degree|major|program)\b$").expect("program name suffix pattern"))
}
fn clean_program_name_punctuation_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"[.;:,]+$").expect("program name punctuation pattern"))
}

fn after_completing_may_take_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:after|once|when)\s+(?:complet(?:ing|ed)|finish(?:ing|ed))\b.{0,180}\b(?:students?\s+)?(?:may|can)\s+(?:also\s+)?take\b").expect("support-only after-completion pattern"))
}
fn fulfill_institution_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bfulfill(?:ing|ed)?\s+(?:the\s+)?(?:university|college|campus)\s+requirements?\b").expect("support-only institutional requirement pattern"))
}
fn may_also_take_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:students?\s+)?(?:may|can)\s+(?:also\s+)?take\b").expect("support-only may-take pattern"))
}
fn may_take_additional_courses_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bmay\s+have\s+to\s+take\s+additional\s+courses?\b").expect("support-only additional-course pattern"))
}
fn encouraged_take_or_enroll_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bencouraged\s+to\s+(?:take|enroll)\b").expect("support-only encouraged-course pattern"))
}
fn count_toward_support_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:as well as|which can include|can include|up to)\b.{0,140}\bcount(?:s|ed)?\s+(?:toward|required(?:\s+for\s+graduation)?)\b").expect("support-only count-toward pattern"))
}
fn support_requirement_count_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bcount(?:s|ed)?\s+(?:toward|required(?:\s+for\s+graduation)?)\b.{0,140}\b(?:natural sciences?|additional coursework|elective credits?|requirements?)\b").expect("support-only requirement count pattern"))
}
fn required_choice_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:one\s+course\s+chosen\s+from|one\s+course\s+from|choose|select|required)\b").expect("required choice cue pattern"))
}
fn support_list_or_count_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:approved\s+)?(?:courses?|course|core|capstone|electives?|systems electives?)\s+list\b|\b(?:as well as|which can include|can include|up to)\b.{0,120}\bcount(?:s|ed)?\s+toward\b|\bcount(?:s|ed)?\s+toward\b.{0,120}\b(?:natural sciences?|additional coursework|elective credits?|requirements?)\b").expect("support-only list pattern"))
}
fn list_on_the_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\blist\s+on\s+the\b").expect("support-only list-on pattern"))
}
fn generic_support_count_toward_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:as well as|which can include|can include|up to)\b.{0,120}\bcount(?:s|ed)?\s+toward\b").expect("support-only generic count pattern"))
}
fn generic_count_toward_support_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bcount(?:s|ed)?\s+toward\b.{0,120}\b(?:natural sciences?|additional coursework|elective credits?|requirements?)\b").expect("support-only generic requirement pattern"))
}

fn option_acronym_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b([A-Z][A-Z0-9&]{1,8})\s+Option\b").expect("option acronym pattern"))
}
fn except_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bexcept\b.{0,120}\brequirement\b").expect("replacement exception pattern"))
}
fn in_place_of_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bin place of\b").expect("in-place-of pattern"))
}
fn complete_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bcomplete\b").expect("complete requirement pattern"))
}
fn base_replacement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bexcept\s+the\s+(\d+)\s+credits?\s+(.+?)\s+requirement\b").expect("base replacement pattern"))
}
fn replacement_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bcomplete\s+(?:a\s+|the\s+)?(\d+)\s+credits?\s+(.+?)(?:\s+below|[.;]|$)").expect("replacement requirement pattern"))
}
fn explicit_owner_prefix_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\ball\s+([A-Z]{2,8})\s+degree requirements\b").expect("explicit owner prefix pattern"))
}
fn replacement_cue_before_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b(?:in place of|instead of|may be replaced by|replace(?:s|d)?|substitut(?:e|es|ed))\b.{0,140}\brequirements?\b").expect("replacement cue pattern"))
}
fn replacement_cue_after_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\brequirements?\b.{0,140}\b(?:in place of|instead of|may be replaced by|replace(?:s|d)?|substitut(?:e|es|ed))\b").expect("replacement cue pattern"))
}
fn core_and_elective_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bcore\s+(?:and|&)\s+elective\s+requirements?\b").expect("core and elective requirement pattern"))
}
fn elective_credit_requirement_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\belectives?\s*\([^)]*\b\d+\s+credits?\s+required\b").expect("elective credit requirement pattern"))
}
fn technical_elective_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\btechnical electives?\s+requirements?\b").expect("technical elective requirement pattern"))
}
fn requirement_slug_credits_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\b\d+\s*credits?\b").expect("requirement slug credits pattern"))
}
fn requirement_slug_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\brequirements?\b").expect("requirement slug requirement pattern"))
}
fn requirement_slug_the_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bthe\b").expect("requirement slug article pattern"))
}
fn requirement_slug_and_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\band\b").expect("requirement slug conjunction pattern"))
}
fn technical_elective_singular_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\btechnical elective\b").expect("technical elective slug pattern"))
}
fn core_elective_singular_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\bcore elective\b").expect("core elective slug pattern"))
}
