use super::requirement_course_codes::CourseCodeParser;
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::OnceLock;
use url::Url;

pub fn build_parser_recovery_source_fingerprint(value: &Value) -> String {
    let serialized = serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec());
    let digest = Sha256::digest(serialized);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex.truncate(16);
    hex
}

pub fn build_parser_recovery_owner_snapshot(owner: &Value) -> Value {
    let quality_signals = owner
        .get("qualitySignals")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| super::requirement_source_quality::build_parse_quality_signals(owner));
    let parsed_codes = array_field(owner, "parsedUwCourseCodes");
    let source_only_codes = array_field(owner, "sourceOnlyUwCourseCodes");
    let structured_only_codes = array_field(owner, "structuredOnlyUwCourseCodes");
    let groups = array_field(owner, "parsedRequirementGroups");
    let atoms = array_field(owner, "parsedRequirementAtomCandidates");
    let requirement_cues = array_field(owner, "requirementCueLines");
    let choose_statements = array_field(owner, "chooseStatements");
    let quality_signal_codes = quality_signals
        .iter()
        .filter_map(|signal| signal.get("code").cloned())
        .collect::<Vec<_>>();
    let quality_warning_codes = quality_signals
        .iter()
        .filter(|signal| signal.get("severity").and_then(Value::as_str) == Some("warning"))
        .filter_map(|signal| signal.get("code").cloned())
        .collect::<Vec<_>>();
    let fingerprint_input = json!({
        "sourceUrl": owner.get("sourceUrl").cloned().unwrap_or(Value::Null),
        "sourceLabel": owner.get("sourceLabel").cloned().unwrap_or(Value::Null),
        "parsedUwCourseCodes": parsed_codes,
        "requirementCueLines": requirement_cues.iter().take(20).cloned().collect::<Vec<_>>(),
        "chooseStatements": choose_statements.iter().take(20).cloned().collect::<Vec<_>>(),
        "qualitySignals": quality_signals,
    });
    let source_fingerprint = build_parser_recovery_source_fingerprint(&fingerprint_input);
    let role = owner
        .get("sourceRole")
        .and_then(Value::as_str)
        .unwrap_or("ignored");
    let role_status = owner
        .get("sourceRoleStatus")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| {
            json!(super::requirement_scope::get_requirement_source_role_status(Some(role)).as_str())
        });
    let parse_confidence = owner
        .get("parseConfidence")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!("low"));
    json!({
        "parsedUwCourseCodeCount": parsed_codes.len(),
        "sourceOnlyUwCourseCodeCount": source_only_codes.len(),
        "structuredOnlyUwCourseCodeCount": structured_only_codes.len(),
        "parsedRequirementGroupCount": groups.len(),
        "parsedRequirementAtomCandidateCount": atoms.len(),
        "requirementCueLineCount": requirement_cues.len(),
        "chooseStatementCount": choose_statements.len(),
        "parseConfidence": parse_confidence,
        "qualitySignalCodes": quality_signal_codes,
        "qualityWarningCodes": quality_warning_codes,
        "qualityWarningCount": quality_warning_codes.len(),
        "sourceUrl": owner.get("sourceUrl").cloned().unwrap_or(Value::Null),
        "sourceRole": owner.get("sourceRole").cloned().unwrap_or(Value::Null),
        "sourceRoleStatus": role_status,
        "sourceFingerprint": source_fingerprint,
    })
}

pub fn same_source_url(left: Option<&Value>, right: Option<&Value>) -> bool {
    normalize_url(left) == normalize_url(right)
}

pub fn build_snapshot_parser_recovery_link_pairs(lines: &[String]) -> Vec<(String, String)> {
    let href_pattern = snapshot_href_pattern();
    let mut links = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(captures) = href_pattern.captures(line) else {
            continue;
        };
        let href = captures
            .get(1)
            .or_else(|| captures.get(2))
            .or_else(|| captures.get(3))
            .map(|value| normalize_space(value.as_str()))
            .unwrap_or_default();
        if href.is_empty()
            || href.starts_with('#')
            || ["javascript:", "mailto:", "tel:"]
                .iter()
                .any(|prefix| href.to_ascii_lowercase().starts_with(prefix))
        {
            continue;
        }
        let same_line = normalize_space(&href_pattern.replace(line, " ").to_string());
        let same_line = super::requirement_html::strip_html(&same_line);
        let mut label =
            if !snapshot_link_label_is_noise(&same_line) && same_line.encode_utf16().count() >= 4 {
                same_line
            } else {
                let mut parts = Vec::new();
                for candidate_line in lines.iter().skip(index + 1).take(6) {
                    let candidate =
                        normalize_space(&super::requirement_html::strip_html(candidate_line));
                    if snapshot_link_label_is_noise(&candidate) {
                        continue;
                    }
                    parts.push(candidate.clone());
                    if candidate.encode_utf16().count() >= 8 {
                        break;
                    }
                }
                normalize_space(&parts.join(" "))
            };
        if label.is_empty() {
            label = href.clone();
        }
        links.push((href, label));
    }
    links
}

fn normalize_space(value: &str) -> String {
    super::requirement_shape::normalize_js_whitespace(value)
}

fn js_compatible_regex(pattern: &str) -> Result<Regex, regex::Error> {
    let pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
    Regex::new(&pattern)
}

fn snapshot_link_label_is_noise(value: &str) -> bool {
    let normalized = normalize_space(value);
    normalized.is_empty()
        || js_compatible_regex(r"(?i)^</?[a-z][^>]*>?$")
            .map(|pattern| pattern.is_match(&normalized))
            .unwrap_or(false)
        || js_compatible_regex(
            r"(?i)^(?:class|style|target|rel|aria-[\w-]+|data-[\w-]+|id|title)\s*=",
        )
        .map(|pattern| pattern.is_match(&normalized))
        .unwrap_or(false)
        || js_compatible_regex(r"(?i)^(?:href|src)\s*=")
            .map(|pattern| pattern.is_match(&normalized))
            .unwrap_or(false)
        || normalized
            .chars()
            .all(|character| "<>/".contains(character))
}

fn snapshot_href_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        js_compatible_regex(r#"(?i)\bhref\s*=\s*(?:"([^"]+)"|'([^']+)'|([^\s>]+))"#)
            .expect("snapshot href pattern is valid")
    })
}

pub fn build_parser_recovery_attempt_record(
    candidate: &Value,
    status: &str,
    details: &Value,
) -> Value {
    let parsed_course_codes = details
        .get("parsedUwCourseCodes")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!([]));
    let snapshot_lines = details
        .get("snapshotLines")
        .and_then(Value::as_array)
        .map(|lines| lines.iter().take(40).cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let quality_signal_codes = details
        .get("qualitySignalCodes")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!([]));
    let fingerprint_input = json!({
        "strategy": candidate.get("strategy").cloned().unwrap_or(Value::Null),
        "sourceUrl": candidate.get("url").cloned().unwrap_or(Value::Null),
        "sourceLabel": candidate.get("label").cloned().unwrap_or(Value::Null),
        "parsedCourseCodes": parsed_course_codes,
        "snapshotLines": snapshot_lines,
        "qualitySignalCodes": quality_signal_codes,
    });
    let source_role = details
        .get("sourceRole")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| candidate.get("sourceRole").cloned())
        .unwrap_or(Value::Null);
    let source_role_name = source_role.as_str().unwrap_or("ignored");
    let source_role_status = details
        .get("sourceRoleStatus")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| {
            candidate
                .get("sourceRoleStatus")
                .filter(|value| !value.is_null())
                .cloned()
        })
        .unwrap_or_else(|| {
            json!(
                super::requirement_scope::get_requirement_source_role_status(Some(
                    source_role_name
                ))
                .as_str()
            )
        });
    let reason = details
        .get("reason")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| candidate.get("reason").cloned())
        .unwrap_or(Value::Null);
    let parsed_course_code_count = parsed_course_codes.as_array().map(Vec::len).unwrap_or(0);
    json!({
        "strategy": candidate.get("strategy").cloned().unwrap_or(Value::Null),
        "status": status,
        "sourceUrl": candidate.get("url").cloned().unwrap_or(Value::Null),
        "sourceLabel": candidate.get("label").cloned().unwrap_or(Value::Null),
        "parserType": candidate.get("parserType").cloned().unwrap_or(Value::Null),
        "sourceRole": source_role,
        "sourceRoleStatus": source_role_status,
        "score": candidate.get("score").cloned().unwrap_or(Value::Null),
        "reason": reason,
        "parsedUwCourseCodeCount": parsed_course_code_count,
        "qualitySignalCodes": quality_signal_codes,
        "qualityWarningCodes": details
            .get("qualityWarningCodes")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| json!([])),
        "sourceEvidenceFingerprint": build_parser_recovery_source_fingerprint(&fingerprint_input),
        "error": details.get("error").cloned().unwrap_or(Value::Null),
    })
}

pub fn merge_recovered_support_sources(owner: &Value, support_owners: &[Value]) -> Value {
    let mut existing_lists = owner
        .get("supportLists")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut target_keys = existing_lists
        .iter()
        .filter_map(approved_list_key)
        .collect::<HashSet<_>>();
    if let Some(inferred_key) =
        super::requirement_source_audits::infer_approved_list_key_from_support_metadata(owner)
    {
        target_keys.insert(inferred_key);
    }
    let mut recovered_lists = Vec::new();
    for support_owner in support_owners {
        let lists = support_owner
            .get("supportLists")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for support_list in lists {
            let mut list = support_list.as_object().cloned().unwrap_or_default();
            let source_url = nullish_value(support_list.get("sourceUrl"))
                .or_else(|| nullish_value(support_owner.get("sourceUrl")))
                .or_else(|| nullish_value(owner.get("sourceUrl")))
                .unwrap_or(Value::Null);
            let shape =
                nullish_value(support_list.get("shape")).unwrap_or_else(|| json!("support-list"));
            let shape_text = shape.as_str().unwrap_or("support-list");
            let slug_source = nullish_value(support_list.get("sourceUrl"))
                .or_else(|| nullish_value(support_list.get("listTitle")))
                .unwrap_or_else(|| shape.clone());
            let owner_id = owner
                .get("ownerId")
                .map(js_interpolate)
                .unwrap_or_else(|| "undefined".to_string());
            let pathway_id = owner.get("pathwayId").filter(|value| js_truthy(value));
            let pathway_segment = pathway_id
                .map(|value| format!(":pathway:{}", js_interpolate(value)))
                .unwrap_or_default();
            let recovered_id = format!(
                "{owner_id}{pathway_segment}:recovered-support:{}:{shape_text}",
                slugify(&js_interpolate(&slug_source))
            );
            list.insert("id".to_string(), json!(recovered_id));
            list.insert("sourceUrl".to_string(), source_url);
            let source_role = nullish_value(support_list.get("sourceRole"))
                .or_else(|| nullish_value(support_owner.get("sourceRole")))
                .unwrap_or(Value::Null);
            list.insert("sourceRole".to_string(), source_role);
            list.insert("supportOnly".to_string(), json!(true));
            list.insert("canCreateRequiredRow".to_string(), json!(false));
            list.insert("canCreateScheduleRow".to_string(), json!(false));
            let normalized = Value::Object(list);
            let key = support_list_dedupe_key(&normalized);
            if let Some(approved_key) = approved_list_key(&normalized) {
                if !target_keys.is_empty() && !target_keys.contains(&approved_key) {
                    continue;
                }
            }
            if !existing_lists
                .iter()
                .chain(recovered_lists.iter())
                .any(|existing| support_list_dedupe_key(existing) == key)
            {
                recovered_lists.push(normalized);
            }
        }
    }
    if recovered_lists.is_empty() {
        return owner.clone();
    }
    existing_lists.extend(recovered_lists);
    let mut seen = HashSet::new();
    existing_lists.retain(|list| seen.insert(support_list_dedupe_key(list)));
    let mut result = owner.as_object().cloned().unwrap_or_default();
    result.insert("supportLists".to_string(), Value::Array(existing_lists));
    Value::Object(result)
}

pub fn classify_parser_recovery_blocker(
    source_unavailable: bool,
    candidate_count: usize,
    attempts: &[Value],
) -> &'static str {
    if source_unavailable {
        return "source-unavailable";
    }
    if attempts.is_empty() || candidate_count == 0 {
        return "needs-deeper-discovery-rule";
    }
    let parsed_attempts = attempts
        .iter()
        .filter(|attempt| attempt.get("status").and_then(Value::as_str) == Some("parsed"))
        .collect::<Vec<_>>();
    if !parsed_attempts.is_empty()
        && parsed_attempts.iter().all(|attempt| {
            attempt.get("sourceRoleStatus").and_then(Value::as_str) == Some("support")
        })
    {
        return "source-support-only";
    }
    if parsed_attempts.iter().any(|attempt| {
        attempt
            .get("parsedUwCourseCodeCount")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0
    }) {
        return "source-official-but-ambiguous";
    }
    "needs-new-parser-rule"
}

pub fn score_parser_recovery_owner(owner: &Value) -> i64 {
    let snapshot = build_parser_recovery_owner_snapshot(owner);
    let count = |field: &str| snapshot.get(field).and_then(Value::as_i64).unwrap_or(0);
    let confidence_score = match owner.get("parseConfidence").and_then(Value::as_str) {
        Some("high") => 70,
        Some("medium") => 35,
        _ => 0,
    };
    let primary_source_score =
        if normalize_url(owner.get("sourceUrl")) == normalize_url(owner.get("primarySourceUrl")) {
            400
        } else {
            0
        };
    count("parsedUwCourseCodeCount") * 100
        + count("parsedRequirementGroupCount") * 35
        + count("parsedRequirementAtomCandidateCount") * 15
        + count("requirementCueLineCount") * 3
        + count("chooseStatementCount") * 4
        + primary_source_score
        + confidence_score
        - count("qualityWarningCount") * 120
}

pub fn parser_recovery_owner_looks_too_broad(
    before_owner: &Value,
    after_owner: &Value,
    candidate: &Value,
) -> bool {
    let after_count = build_parser_recovery_owner_snapshot(after_owner)
        .get("parsedUwCourseCodeCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let before_structured_coverage_count = structured_coverage_count(before_owner);
    let signals = candidate.get("signals").unwrap_or(&Value::Null);
    let exact_title_match = signals.get("exactTitleMatch").is_some_and(js_truthy);
    let same_program_link = signals
        .get("sameProgramRequirementLink")
        .is_some_and(js_truthy);
    let document_signal = signals.get("documentSignal").is_some_and(js_truthy);
    if after_count >= 80 && !exact_title_match && !same_program_link && !document_signal {
        return true;
    }
    before_structured_coverage_count > 0
        && after_count > 60_u64.max(before_structured_coverage_count.saturating_mul(5))
        && !exact_title_match
        && !same_program_link
}

pub fn should_accept_parser_recovery_owner(
    before_owner: &Value,
    after_owner: &Value,
    candidate: &Value,
    primary_major_titles: &std::collections::HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> bool {
    if !after_owner.get("ok").is_some_and(js_truthy)
        || after_owner.get("supportOnly").is_some_and(js_truthy)
        || after_owner
            .get("canCreateSchedulableRows")
            .and_then(Value::as_bool)
            == Some(false)
        || parser_recovery_owner_looks_too_broad(before_owner, after_owner, candidate)
    {
        return false;
    }
    let before = build_parser_recovery_owner_snapshot(before_owner);
    let after = build_parser_recovery_owner_snapshot(after_owner);
    let before_course_count = snapshot_count(&before, "parsedUwCourseCodeCount");
    let after_course_count = snapshot_count(&after, "parsedUwCourseCodeCount");
    let before_group_count = snapshot_count(&before, "parsedRequirementGroupCount");
    let after_group_count = snapshot_count(&after, "parsedRequirementGroupCount");
    let before_warning_count = snapshot_count(&before, "qualityWarningCount");
    let after_warning_count = snapshot_count(&after, "qualityWarningCount");
    let signals = candidate.get("signals").unwrap_or(&Value::Null);
    let exact_title_match = signals.get("exactTitleMatch").is_some_and(js_truthy);
    let title_overlap = signals
        .get("titleTokenOverlapCount")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let before_title_token_count =
        super::requirement_recovery_links::parser_recovery_title_scope_tokens(before_owner).len();
    let before_source = before_owner
        .get("sourceUrl")
        .filter(|value| !value.is_null())
        .or_else(|| before_owner.get("primarySourceUrl"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let after_source = after_owner
        .get("sourceUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if super::requirement_recovery_scope::is_different_program_sibling_page(
        before_source,
        after_source,
    ) && before_title_token_count >= 2
        && !exact_title_match
        && title_overlap < 2.min(before_title_token_count)
    {
        return false;
    }
    if after_course_count == 0 {
        return false;
    }
    let before_primary_url = before_owner
        .get("primarySourceUrl")
        .filter(|value| !value.is_null())
        .or_else(|| before_owner.get("sourceUrl"));
    let after_primary_url = after_owner
        .get("primarySourceUrl")
        .filter(|value| !value.is_null())
        .or_else(|| after_owner.get("sourceUrl"));
    let before_uses_primary = before_owner
        .get("resolutionStrategy")
        .and_then(Value::as_str)
        != Some("alternate-official-source")
        && normalize_url(before_owner.get("sourceUrl")) == normalize_url(before_primary_url);
    let after_uses_primary = after_owner
        .get("resolutionStrategy")
        .and_then(Value::as_str)
        != Some("alternate-official-source")
        && normalize_url(after_owner.get("sourceUrl")) == normalize_url(after_primary_url);
    if before_uses_primary
        && !after_uses_primary
        && before_course_count >= 8
        && before_group_count > 0
        && after_course_count <= before_course_count + 5
        && after_group_count <= before_group_count + 2
    {
        return false;
    }
    let has_pathway = before_owner.get("pathwayId").is_some_and(js_truthy);
    if has_pathway
        && before_course_count > 0
        && snapshot_lines(before_owner).iter().any(|line| {
            super::requirement_recovery_scope::is_likely_selected_pathway_html_section_start_line(
                before_owner,
                line,
                primary_major_titles,
                course_parser,
            )
        })
        && after_course_count > before_course_count + 10
    {
        return false;
    }
    let lines = candidate
        .get("sectionLines")
        .and_then(Value::as_array)
        .map(|lines| {
            lines
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| snapshot_lines(after_owner));
    if has_pathway
        && candidate.get("strategy").and_then(Value::as_str)
            == Some("cached-owner-snapshot-recovery")
        && before_course_count > 0
        && after_course_count > before_course_count + 2
        && lines.iter().any(|line| {
            super::requirement_recovery_scope::is_pathway_html_section_sibling_start(
                before_owner,
                line,
                primary_major_titles,
                course_parser,
            )
        })
    {
        return false;
    }
    if !has_pathway
        && before_course_count > 0
        && after_course_count > before_course_count + 2
        && super::requirement_recovery_scope::remove_parallel_pathway_html_table_content_from_base_scope(
            before_owner,
            &lines,
            course_parser,
        )
        .len()
            < lines.len()
    {
        return false;
    }
    let before_is_primary =
        before_owner.get("sourceRoleStatus").and_then(Value::as_str) == Some("primary");
    if (before_owner
        .get("canCreateSchedulableRows")
        .and_then(Value::as_bool)
        == Some(false)
        || !before_is_primary)
        && after_owner
            .get("canCreateSchedulableRows")
            .and_then(Value::as_bool)
            != Some(false)
        && after_owner.get("sourceRoleStatus").and_then(Value::as_str) == Some("primary")
    {
        return true;
    }
    if before_course_count == 0 && after_course_count > 0 {
        return true;
    }
    if after_course_count >= before_course_count + 2 && after_warning_count <= before_warning_count
    {
        return true;
    }
    if after_course_count >= before_course_count && after_warning_count < before_warning_count {
        return true;
    }
    if after_group_count > before_group_count && after_course_count >= before_course_count {
        return true;
    }
    score_parser_recovery_owner(after_owner) > score_parser_recovery_owner(before_owner) + 120
}

fn structured_coverage_count(owner: &Value) -> u64 {
    let parsed = array_field(owner, "parsedUwCourseCodes").len() as u64;
    let source_only = array_field(owner, "sourceOnlyUwCourseCodes").len() as u64;
    let structured_only = array_field(owner, "structuredOnlyUwCourseCodes").len() as u64;
    parsed.saturating_sub(source_only) + structured_only
}

fn snapshot_lines(owner: &Value) -> Vec<String> {
    owner
        .get("snapshotLines")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn snapshot_count(snapshot: &Value, field: &str) -> u64 {
    snapshot.get(field).and_then(Value::as_u64).unwrap_or(0)
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

fn approved_list_key(support_list: &Value) -> Option<String> {
    let key = support_list
        .get("approvedListKey")
        .filter(|value| !value.is_null())
        .or_else(|| {
            support_list
                .get("filterKey")
                .filter(|value| !value.is_null())
        })?
        .as_str()?;
    (!key.is_empty()).then(|| key.to_string())
}

fn support_list_dedupe_key(support_list: &Value) -> String {
    let source_url = support_list
        .get("sourceUrl")
        .map(js_interpolate)
        .unwrap_or_else(|| "undefined".to_string());
    let shape = support_list
        .get("shape")
        .map(js_interpolate)
        .unwrap_or_else(|| "undefined".to_string());
    let approved_key = approved_list_key(support_list).unwrap_or_default();
    format!("{source_url}:{shape}:{approved_key}")
}

fn nullish_value(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| !value.is_null()).cloned()
}

fn js_interpolate(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => super::requirement_shape::js_number_to_string(value),
        Value::Array(values) => values
            .iter()
            .map(js_interpolate)
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn slugify(value: &str) -> String {
    let normalized = super::requirement_shape::normalize_js_whitespace(value).to_lowercase();
    let mut slug = String::with_capacity(normalized.len());
    let mut last_hyphen = false;
    for character in normalized.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_hyphen = false;
        } else if !slug.is_empty() && !last_hyphen {
            slug.push('-');
            last_hyphen = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

fn array_field<'a>(owner: &'a Value, field: &str) -> &'a [Value] {
    owner
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

pub fn get_requested_url_anchor(url: &str) -> Option<String> {
    let raw_fragment = Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.fragment().map(str::to_string))
        .or_else(|| {
            url.split_once('#')
                .map(|(_, fragment)| fragment.to_string())
        })?;
    let decoded = decode_uri_component(&raw_fragment)?;
    let anchor = decoded
        .trim_matches(super::requirement_shape::is_js_whitespace)
        .to_string();
    (!anchor.is_empty()).then_some(anchor)
}

fn decode_uri_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn normalize_url(value: Option<&Value>) -> Option<String> {
    let value = value?.as_str()?;
    let mut parsed = Url::parse(value).ok()?;
    parsed.set_fragment(None);
    let path = parsed.path().trim_end_matches('/');
    let normalized_path = if path.is_empty() { "/" } else { path };
    let origin = parsed.origin().ascii_serialization();
    Some(format!(
        "{origin}{normalized_path}{}",
        parsed
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default()
    ))
}
