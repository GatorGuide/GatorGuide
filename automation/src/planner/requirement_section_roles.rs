use super::requirement_course_codes::{CourseCodeParser, get_parsed_course_level};
use super::requirement_shape::looks_like_standalone_requirement_title_line;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::OnceLock;

pub fn schedulable_inline_requirement_line_indices(
    owner: &Value,
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> HashSet<usize> {
    let mut current_role = "primary-requirement-section";
    let mut current_role_reason = String::new();
    let mut in_degree_route_comparison_section = false;
    let mut schedulable_indices = HashSet::new();

    for index in 0..snapshot_lines.len() {
        let line = normalize_source_section_line(&snapshot_lines[index]);
        if line.is_empty() {
            continue;
        }
        if degree_route_comparison_heading_pattern().is_match(&line) {
            in_degree_route_comparison_section = true;
        }
        let (explicit_role, explicit_reason) =
            if super::requirement_recovery_scope::is_graduate_or_applied_masters_requirement_context(
                owner,
                snapshot_lines,
                index,
            ) {
                (
                    "support-metadata",
                    "graduate requirement context is not schedulable undergraduate evidence",
                )
            } else if in_degree_route_comparison_section {
                (
                    "support-metadata",
                    "degree-route comparison section is not schedulable requirement evidence",
                )
            } else {
                classify_inline_source_section_line(
                    None,
                    &line,
                    current_role,
                    &current_role_reason,
                    course_parser,
                )
            };
        if matches!(
            explicit_role,
            "primary-requirement-section" | "admission-prep-section"
        ) {
            schedulable_indices.insert(index);
        }

        let course_codes = extract_section_role_course_codes(&line, course_parser);
        let line_looks_like_heading = course_codes.is_empty()
            && (has_prerequisite_only_cue(&line)
                || ((has_course_list_section_cue(&line)
                    || has_approved_course_list_section_cue(&line)
                    || has_elective_list_section_cue(&line)
                    || has_distribution_area_course_list_section_cue(&line))
                    && is_likely_source_section_heading_line(&line, course_parser))
                || super::requirement_rule_utils::has_option_replacement_requirement_cue(&line)
                || super::requirement_recovery_scope::has_primary_requirement_section_cue(&line)
                || has_undergraduate_requirement_reset_cue(&line)
                || has_historical_requirement_section_cue(&line)
                || has_admission_prep_heading_cue(&line)
                || has_post_admission_degree_completion_cue(&line));
        let line_looks_like_standalone_requirement_title = course_codes.is_empty()
            && looks_like_standalone_requirement_title_line(&line, |candidate| {
                course_parser.extract_from_requirement_line(candidate)
            });
        if line_looks_like_heading || line_looks_like_standalone_requirement_title {
            current_role = explicit_role;
            current_role_reason = explicit_reason.to_string();
        }
        if in_degree_route_comparison_section
            && degree_route_comparison_terminator_pattern().is_match(&line)
        {
            in_degree_route_comparison_section = false;
            current_role = "primary-requirement-section";
            current_role_reason.clear();
        }
    }
    schedulable_indices
}

pub fn build_parser_prerequisite_filter_audit_rows(
    owner_id: Option<&Value>,
    source_url: Option<&Value>,
    source_role: Option<&str>,
    headings: &[String],
    snapshot_lines: &[String],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let owner_context = json!({"ownerId": owner_id.cloned().unwrap_or(Value::Null)});
    let source_url_text = source_url
        .filter(|value| !value.is_null())
        .map(js_string_value)
        .unwrap_or_default();
    let source_context = json!({
        "sourceUrl": source_url.cloned().unwrap_or(Value::Null),
        "sourceRole": source_role.unwrap_or("ignored"),
    });
    let normalized_headings = headings
        .iter()
        .map(|heading| normalize_whitespace(heading))
        .filter(|heading| !heading.is_empty())
        .collect::<HashSet<_>>();
    let mut rows = Vec::new();
    let mut current_section_title = "source root".to_string();
    let mut current_role = "primary-requirement-section";
    let mut current_role_reason = String::new();
    let mut in_degree_route_comparison_section = false;

    for (index, source_line) in snapshot_lines.iter().enumerate() {
        let raw_line = normalize_whitespace(source_line);
        let normalized_line = normalize_source_section_line(&raw_line);
        if normalized_line.is_empty()
            || super::requirement_parse_helpers::is_noisy_line(&raw_line)
            || super::requirement_parse_helpers::is_noisy_line(&normalized_line)
        {
            continue;
        }
        if degree_route_comparison_heading_pattern().is_match(&normalized_line) {
            in_degree_route_comparison_section = true;
        }

        let is_tacoma_reset =
            is_tacoma_rn_bsn_sample_program_plan_reset_line(&source_url_text, &normalized_line);
        if is_undergraduate_catalog_section_reset_line(&normalized_line)
            || has_undergraduate_requirement_reset_cue(&normalized_line)
            || is_tacoma_reset
        {
            if !is_inherited_historical_requirement_section(
                current_role,
                &current_role_reason,
                &normalized_line,
            ) {
                current_role = "primary-requirement-section";
                current_role_reason.clear();
            }
        }

        let course_codes = course_parser.extract_from_requirement_line(&normalized_line);
        let line_is_graduate_requirement_context = !is_tacoma_reset
            && super::requirement_recovery_scope::is_graduate_or_applied_masters_requirement_context(
                &source_context,
                snapshot_lines,
                index,
            );
        let (explicit_role, explicit_reason) = if line_is_graduate_requirement_context {
            (
                "support-metadata",
                "graduate requirement context is not schedulable undergraduate evidence",
            )
        } else if in_degree_route_comparison_section {
            (
                "support-metadata",
                "degree-route comparison section is not schedulable requirement evidence",
            )
        } else if should_inherit_graduate_requirement_section(
            current_role,
            &current_role_reason,
            &normalized_line,
        ) {
            (
                "support-metadata",
                "inherits graduate requirement context from nearby heading",
            )
        } else {
            classify_inline_source_section_line(
                Some(&owner_context),
                &normalized_line,
                current_role,
                &current_role_reason,
                course_parser,
            )
        };

        let line_looks_like_heading = course_codes.is_empty()
            && (normalized_headings.contains(&normalized_line)
                || has_prerequisite_only_cue(&normalized_line)
                || ((has_course_list_section_cue(&normalized_line)
                    || has_approved_course_list_section_cue(&normalized_line)
                    || has_elective_list_section_cue(&normalized_line)
                    || has_distribution_area_course_list_section_cue(&normalized_line))
                    && is_likely_source_section_heading_line(&normalized_line, course_parser))
                || super::requirement_rule_utils::has_option_replacement_requirement_cue(
                    &normalized_line,
                )
                || super::requirement_recovery_scope::has_primary_requirement_section_cue(
                    &normalized_line,
                )
                || has_historical_requirement_section_cue(&normalized_line)
                || (is_non_schedulable_section_role(current_role)
                    && is_non_schedulable_course_list_subheading(&normalized_line))
                || has_admission_prep_heading_cue(&normalized_line)
                || has_post_admission_degree_completion_cue(&normalized_line));
        let line_looks_like_standalone_requirement_title = course_codes.is_empty()
            && looks_like_standalone_requirement_title_line(&normalized_line, |candidate| {
                course_parser.extract_from_requirement_line(candidate)
            });
        if line_looks_like_heading || line_looks_like_standalone_requirement_title {
            current_section_title = normalized_line.clone();
            current_role = explicit_role;
            current_role_reason = explicit_reason.to_string();
        }

        let schedulable = matches!(
            explicit_role,
            "primary-requirement-section" | "admission-prep-section"
        );
        let has_filter_cue = explicit_role != "primary-requirement-section"
            || has_prerequisite_only_cue(&normalized_line)
            || has_course_list_section_cue(&normalized_line)
            || has_approved_course_list_section_cue(&normalized_line)
            || has_elective_list_section_cue(&normalized_line)
            || has_distribution_area_course_list_section_cue(&normalized_line)
            || has_numbered_requirement_course_section_heading_cue(&normalized_line)
            || super::requirement_groups::has_credit_bucket_cue(&normalized_line, course_parser);
        if course_codes.is_empty() && !has_filter_cue {
            continue;
        }

        let owner_text = display_value_or(owner_id, "unknown");
        let source_text = display_value_or(source_url, "n/a");
        let section_text = if current_section_title.is_empty() {
            "source root"
        } else {
            &current_section_title
        };
        let course_codes_text = if course_codes.is_empty() {
            "none".to_string()
        } else {
            course_codes.join(", ")
        };
        let debug_text = [
            "[parser prerequisite filter audit]".to_string(),
            format!("Owner id: {owner_text}"),
            format!("Source URL: {source_text}"),
            format!("Section title: {section_text}"),
            format!(
                "Raw line: {}",
                if normalized_line.is_empty() {
                    "none"
                } else {
                    &normalized_line
                }
            ),
            format!("Course codes extracted: {course_codes_text}"),
            format!("Detected section role: {explicit_role}"),
            format!("Schedulable: {}", if schedulable { "yes" } else { "no" }),
            format!("Reason: {explicit_reason}"),
            "Issue: none".to_string(),
        ]
        .join(" ");
        rows.push(json!({
            "ownerId": owner_id.cloned().unwrap_or(Value::Null),
            "sourceUrl": source_url.cloned().unwrap_or(Value::Null),
            "sourceLineIndex": index,
            "sectionTitle": current_section_title,
            "rawLine": normalized_line,
            "courseCodesExtracted": course_codes,
            "detectedSectionRole": explicit_role,
            "schedulable": schedulable,
            "reason": explicit_reason,
            "issue": "none",
            "copyOnlyDebugText": debug_text,
        }));

        if in_degree_route_comparison_section
            && degree_route_comparison_terminator_pattern().is_match(&normalized_line)
        {
            in_degree_route_comparison_section = false;
            current_role = "primary-requirement-section";
            current_role_reason.clear();
            current_section_title = "source root".to_string();
        }
    }
    rows
}

pub fn classify_requirement_group_source_section(
    text: &str,
    course_parser: &CourseCodeParser,
) -> (String, String) {
    let normalized_text = normalize_source_section_line(text);
    let (role, reason) =
        classify_inline_source_section_line(None, &normalized_text, "", "", course_parser);
    (role.to_string(), reason.to_string())
}

fn classify_inline_source_section_line(
    source_owner: Option<&Value>,
    text: &str,
    inherited_role: &'static str,
    inherited_role_reason: &str,
    course_parser: &CourseCodeParser,
) -> (&'static str, &'static str) {
    let course_codes = extract_section_role_course_codes(text, course_parser);
    let levels = course_codes
        .iter()
        .filter_map(|course_code| get_parsed_course_level(course_code))
        .collect::<Vec<_>>();
    let upper_division_count = levels.iter().filter(|level| **level >= 300).count();
    let all_lower_division = !levels.is_empty() && upper_division_count == 0;
    let mainly_upper_division =
        !levels.is_empty() && upper_division_count >= ((levels.len() * 3).div_ceil(4)).max(1);
    let prerequisite_cue = has_prerequisite_only_cue(text);
    let approved_course_list_cue = has_approved_course_list_section_cue(text);
    let elective_list_cue = has_elective_list_section_cue(text);
    let course_list_cue = has_course_list_section_cue(text);
    let distribution_area_course_list_cue = has_distribution_area_course_list_section_cue(text);
    let heading_like = is_likely_source_section_heading_line(text, course_parser);
    let admission_cue = has_admission_prep_section_cue(text);
    let primary_cue = super::requirement_recovery_scope::has_primary_requirement_section_cue(text)
        || has_numbered_requirement_course_section_heading_cue(text);
    let option_replacement_cue =
        super::requirement_rule_utils::has_option_replacement_requirement_cue(text);
    let explicit_choice_requirement_cue =
        explicit_choice_cue_pattern().is_match(text) && primary_cue && !prerequisite_cue;
    let is_credit_bearing_course_row = !course_codes.is_empty()
        && super::requirement_hint_safety::source_line_starts_with_course_code(text)
        && credit_bearing_course_row_pattern().is_match(text);

    if has_historical_requirement_section_cue(text) {
        if source_owner
            .is_some_and(|owner| historical_requirement_section_matches_owner(owner, text))
        {
            return (
                "primary-requirement-section",
                "historical requirement section matches prior-admit curriculum owner",
            );
        }
        return (
            "support-metadata",
            "historical requirement section is not current schedulable evidence",
        );
    }
    if inherited_role == "support-metadata"
        && inherited_role_reason.contains("historical requirement section")
        && !has_current_curriculum_boundary_cue(text)
        && !is_undergraduate_catalog_section_reset_line(text)
    {
        return (
            "support-metadata",
            "inherits historical requirement section support scope",
        );
    }
    if has_historical_requirement_note_cue(text) {
        return (
            "support-metadata",
            "historical requirement note is support metadata",
        );
    }
    if course_codes.is_empty() && has_honors_only_support_cue(text) {
        return (
            "support-metadata",
            "honors-only section is support metadata",
        );
    }
    if inherited_role == "support-metadata"
        && inherited_role_reason.contains("honors-only")
        && !has_admission_prep_heading_cue(text)
        && !has_undergraduate_requirement_reset_cue(text)
        && !is_undergraduate_catalog_section_reset_line(text)
    {
        return ("support-metadata", "inherits honors-only support section");
    }
    if has_support_only_post_completion_course_cue(text) {
        return (
            "support-metadata",
            "post-completion recommendation or campus requirement note is support metadata",
        );
    }
    if has_application_only_or_restricted_registration_cue(text) {
        return (
            "support-metadata",
            "application-only or restricted-registration course row is support metadata",
        );
    }
    if super::requirement_groups::has_course_question_support_cue(text, course_parser) {
        return (
            "support-metadata",
            "course-bearing question line is support metadata",
        );
    }
    if has_academic_support_only_course_cue(text, course_parser) {
        return (
            "support-metadata",
            "optional or academic-support course row is support metadata",
        );
    }
    if super::requirement_groups::has_non_requirement_course_exclusion_cue(text, course_parser) {
        return (
            "support-metadata",
            "negative course substitution or credit note is support metadata",
        );
    }
    if !course_codes.is_empty() && suggested_course_list_pattern().is_match(text) {
        return (
            "support-metadata",
            "suggested course list is support metadata",
        );
    }
    if has_post_admission_degree_completion_cue(text) {
        return (
            "post-admission-degree-completion-section",
            "post-admission year 3/4 completion section is not transfer-prep schedulable",
        );
    }
    if has_admission_prep_heading_cue(text)
        && course_codes.is_empty()
        && admission_requirement_heading_pattern().is_match(text)
    {
        return (
            "admission-prep-section",
            "admission or preparation requirement heading",
        );
    }
    if prerequisite_cue {
        let colon_label = text
            .split_once(':')
            .map(|(label, _)| label)
            .unwrap_or_default();
        if super::requirement_hint_safety::source_line_starts_with_course_code(text)
            && labeled_course_row_pattern().is_match(text)
            && !prerequisite_table_exclusion_pattern().is_match(colon_label)
            && !prerequisite_table_exclusion_pattern().is_match(text)
        {
            return (
                "primary-requirement-section",
                "labeled requirement course row with inline prerequisite note",
            );
        }
        if is_credit_bearing_course_row
            && inherited_role == "primary-requirement-section"
            && !prerequisite_table_exclusion_pattern().is_match(text)
        {
            return (
                "primary-requirement-section",
                "credit-bearing requirement row with inline prerequisite note",
            );
        }
        if inherited_role == "primary-requirement-section"
            && super::requirement_hint_safety::source_line_starts_with_course_code(text)
            && !prerequisite_table_exclusion_pattern().is_match(text)
        {
            return (
                "primary-requirement-section",
                "requirement row with inline prerequisite note",
            );
        }
        if all_lower_division
            && lower_division_prerequisite_choice_pattern().is_match(text)
            && prerequisite_satisfaction_pattern().is_match(text)
        {
            return (
                "admission-prep-section",
                "lower-division prerequisite option satisfies admission preparation",
            );
        }
        if admission_cue
            && all_lower_division
            && !admission_prep_upper_division_reference_pattern().is_match(text)
        {
            return (
                "admission-prep-section",
                "lower-division admission or preparation prerequisite section",
            );
        }
        if mainly_upper_division
            || upper_division_count > 0
            || upper_division_reference_pattern().is_match(text)
        {
            return (
                "upper-division-prerequisite-table",
                "upper-division prerequisite-only section",
            );
        }
        return (
            "support-metadata",
            "prerequisite-only text is retained as support metadata",
        );
    }
    if option_replacement_cue {
        return (
            "primary-requirement-section",
            "option replacement requirement cue",
        );
    }
    if has_credit_limit_support_cue(text) {
        return ("support-metadata", "credit-limit note is support metadata");
    }
    if super::requirement_groups::has_credit_bucket_cue(text, course_parser) {
        return (
            "primary-requirement-section",
            "credit-bucket requirement cue",
        );
    }
    if has_credit_bucket_requirement_cue(text)
        && (course_codes.is_empty() || explicit_choice_requirement_cue)
    {
        return (
            "primary-requirement-section",
            "credit-bucket requirement cue",
        );
    }
    if is_non_schedulable_section_role(inherited_role)
        && course_codes.is_empty()
        && is_non_schedulable_course_list_subheading(text)
    {
        return (
            inherited_role,
            "inherits non-schedulable source-section role from nearby heading",
        );
    }
    if looks_like_standalone_requirement_title_line(text, |line| {
        course_parser.extract_from_line(line)
    }) && !elective_list_cue
        && !approved_course_list_cue
        && !course_list_cue
    {
        return (
            "primary-requirement-section",
            "standalone requirement title",
        );
    }
    if !explicit_choice_requirement_cue
        && elective_list_cue
        && credit_bearing_elective_pattern().is_match(text)
    {
        return (
            "primary-requirement-section",
            "credit-bearing elective requirement cue",
        );
    }
    if !explicit_choice_requirement_cue
        && course_list_cue
        && credit_bearing_capstone_pattern().is_match(text)
    {
        return (
            "primary-requirement-section",
            "credit-bearing capstone requirement cue",
        );
    }
    if !explicit_choice_requirement_cue && elective_list_cue && heading_like {
        return (
            "elective-list",
            "elective-list section is support metadata unless a primary requirement scopes it",
        );
    }
    if !explicit_choice_requirement_cue
        && (approved_course_list_cue || distribution_area_course_list_cue)
        && heading_like
    {
        return (
            "approved-course-list",
            if distribution_area_course_list_cue {
                "distribution-area course list is support metadata"
            } else {
                "approved course-list section is support metadata"
            },
        );
    }
    if !explicit_choice_requirement_cue && course_list_cue && heading_like {
        return (
            "non-schedulable-course-list",
            "course-list/navigation section is not transfer-plannable by itself",
        );
    }
    if is_non_schedulable_section_role(inherited_role)
        && course_codes.is_empty()
        && is_non_schedulable_course_list_subheading(text)
    {
        return (
            inherited_role,
            "inherits non-schedulable source-section role from nearby heading",
        );
    }
    if is_non_schedulable_section_role(inherited_role)
        && !course_codes.is_empty()
        && labeled_route_course_row_pattern().is_match(text)
    {
        return (
            "primary-requirement-section",
            "labeled route course row overrides nearby non-schedulable heading",
        );
    }
    if is_non_schedulable_section_role(inherited_role)
        && !course_codes.is_empty()
        && all_lower_division
        && credit_parenthetical_pattern().is_match(text)
        && !approved_course_list_cue
        && !elective_list_cue
        && !course_list_cue
    {
        return (
            "primary-requirement-section",
            "lower-division credit-bearing requirement row overrides nearby non-schedulable heading",
        );
    }
    if is_non_schedulable_section_role(inherited_role)
        && !course_codes.is_empty()
        && primary_cue
        && !explicit_choice_requirement_cue
    {
        return (
            inherited_role,
            "inherits non-schedulable source-section role from nearby heading",
        );
    }
    if is_non_schedulable_section_role(inherited_role)
        && !course_codes.is_empty()
        && !primary_cue
        && !explicit_choice_requirement_cue
    {
        return (
            inherited_role,
            "inherits non-schedulable source-section role from nearby heading",
        );
    }
    if mainly_upper_division && upper_division_table_cue_pattern().is_match(text) {
        return (
            "upper-division-prerequisite-table",
            "upper-division table row with prerequisite/table cues",
        );
    }
    if admission_cue && all_lower_division {
        return (
            "admission-prep-section",
            "lower-division admission/preparation section",
        );
    }
    if primary_cue || !course_codes.is_empty() {
        return (
            "primary-requirement-section",
            if primary_cue {
                "primary requirement cue"
            } else {
                "course row without prerequisite-only or course-list section cues"
            },
        );
    }
    if !inherited_role.is_empty() {
        (inherited_role, "inherits nearby source-section role")
    } else {
        (
            "support-metadata",
            "support text without schedulable requirement cue",
        )
    }
}

fn extract_section_role_course_codes(text: &str, course_parser: &CourseCodeParser) -> Vec<String> {
    course_parser
        .extract_from_requirement_line(text)
        .into_iter()
        .filter(|course_code| !is_course_code_level_reference(text, course_code))
        .collect()
}

fn is_course_code_level_reference(text: &str, course_code: &str) -> bool {
    let Some((subject, _)) = course_code.rsplit_once(' ') else {
        return false;
    };
    let Some(level) = get_parsed_course_level(course_code) else {
        return false;
    };
    let pattern = format!(
        r"(?i)\b{}\s+{}\s*[-\s]*level\b",
        regex::escape(subject).replace(' ', r"\s+"),
        level
    );
    let compatible_pattern = super::requirement_shape::js_regex_compatibility_pattern(&pattern);
    Regex::new(&compatible_pattern).is_ok_and(|pattern| pattern.is_match(text))
}

fn normalize_source_section_line(line: &str) -> String {
    let stripped = super::requirement_choice_lists::strip_choice_list_line(line);
    super::requirement_shape::normalize_js_whitespace(&stripped)
}

fn normalize_whitespace(value: &str) -> String {
    super::requirement_shape::normalize_js_whitespace(value)
}

fn display_value_or(value: Option<&Value>, fallback: &str) -> String {
    value
        .filter(|value| !value.is_null())
        .map(js_string_value)
        .unwrap_or_else(|| fallback.to_string())
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

fn has_prerequisite_only_cue(text: &str) -> bool {
    prerequisite_only_cue_pattern().is_match(text)
}

fn has_course_list_section_cue(text: &str) -> bool {
    course_list_section_cue_pattern().is_match(text)
}

fn has_distribution_area_course_list_section_cue(text: &str) -> bool {
    let normalized = normalize_whitespace(text)
        .trim_end_matches(&[':', '.'][..])
        .to_string();
    distribution_area_course_list_section_cue_pattern().is_match(&normalized)
        || academic_breadth_subject_area_pattern().is_match(&normalized)
        || subject_areas_and_courses_pattern().is_match(&normalized)
}

fn has_approved_course_list_section_cue(text: &str) -> bool {
    approved_course_list_section_cue_pattern().is_match(text)
}

fn has_elective_list_section_cue(text: &str) -> bool {
    elective_list_section_cue_pattern().is_match(text)
}

fn has_numbered_requirement_course_section_heading_cue(text: &str) -> bool {
    let normalized = normalize_whitespace(text)
        .trim_end_matches(&[':', '.'][..])
        .to_string();
    if !numbered_requirement_course_heading_pattern().is_match(&normalized)
        || !course_word_pattern().is_match(&normalized)
        || has_approved_course_list_section_cue(&normalized)
        || has_elective_list_section_cue(&normalized)
        || has_distribution_area_course_list_section_cue(&normalized)
    {
        return false;
    }
    !excluded_numbered_requirement_heading_pattern().is_match(&normalized)
}

fn is_likely_source_section_heading_line(text: &str, course_parser: &CourseCodeParser) -> bool {
    let normalized = normalize_whitespace(text);
    if normalized.is_empty()
        || likely_source_heading_exclusion_pattern().is_match(&normalized)
        || !course_parser.extract_from_line(&normalized).is_empty()
    {
        return false;
    }
    (normalized.encode_utf16().count() <= 80 && !sentence_ending_pattern().is_match(&normalized))
        || (normalized.encode_utf16().count() <= 140
            && likely_source_heading_context_pattern().is_match(&normalized)
            && !likely_source_heading_body_exclusion_pattern().is_match(&normalized))
}

fn has_admission_prep_section_cue(text: &str) -> bool {
    admission_prep_section_cue_pattern().is_match(text)
}

fn has_admission_prep_heading_cue(text: &str) -> bool {
    admission_prep_heading_cue_pattern().is_match(text)
        || recommended_preparation_heading_pattern().is_match(text)
}

fn has_post_admission_degree_completion_cue(text: &str) -> bool {
    post_admission_degree_completion_pattern().is_match(text)
        || post_admission_year_course_pattern().is_match(text)
}

fn has_undergraduate_requirement_reset_cue(text: &str) -> bool {
    undergraduate_requirement_reset_cue_pattern().is_match(text)
        && !graduate_degree_context_pattern().is_match(text)
}

fn is_undergraduate_catalog_section_reset_line(text: &str) -> bool {
    let normalized = normalize_whitespace(text);
    let without_periods = normalized.to_ascii_lowercase().replace('.', "");
    let abbreviated_bachelor_reset = ["ba in ", "bs in "]
        .iter()
        .any(|prefix| without_periods.starts_with(prefix))
        || matches!(without_periods.as_str(), "ba in" | "bs in");
    (undergraduate_catalog_reset_pattern().is_match(&normalized) || abbreviated_bachelor_reset)
        && !graduate_degree_context_pattern().is_match(&normalized)
}

fn is_non_schedulable_course_list_subheading(text: &str) -> bool {
    is_non_schedulable_course_list_subheading_pattern().is_match(&normalize_whitespace(text))
}

fn is_non_schedulable_section_role(role: &str) -> bool {
    matches!(
        role,
        "upper-division-prerequisite-table" | "non-schedulable-course-list"
    )
}

fn has_credit_bucket_requirement_cue(text: &str) -> bool {
    credit_bucket_requirement_cue_pattern_a().is_match(text)
        || credit_bucket_requirement_cue_pattern_b().is_match(text)
        || credit_bucket_requirement_cue_pattern_c().is_match(text)
        || credit_bucket_requirement_cue_pattern_d().is_match(text)
}

fn has_credit_limit_support_cue(text: &str) -> bool {
    credit_limit_support_cue_pattern_a().is_match(text)
        && credit_limit_support_cue_pattern_b().is_match(text)
}

fn has_support_only_post_completion_course_cue(text: &str) -> bool {
    support_post_completion_course_pattern_a().is_match(text)
        || (support_post_completion_course_pattern_b().is_match(text)
            && (support_post_completion_course_pattern_c().is_match(text)
                || support_post_completion_course_pattern_d().is_match(text)
                || support_post_completion_course_pattern_e().is_match(text)))
}

fn has_application_only_or_restricted_registration_cue(text: &str) -> bool {
    restricted_registration_pattern().is_match(text)
}

fn has_honors_only_support_cue(text: &str) -> bool {
    bare_honors_heading_pattern().is_match(text)
        || (honors_support_pattern().is_match(text) && !contrasting_honors_pattern().is_match(text))
}

fn has_academic_support_only_course_cue(text: &str, course_parser: &CourseCodeParser) -> bool {
    !course_parser.extract_from_line(text).is_empty()
        && (optional_course_pattern().is_match(text)
            || workshop_course_pattern().is_match(text)
            || academic_support_course_pattern().is_match(text))
}

fn has_historical_requirement_section_cue(text: &str) -> bool {
    historical_requirement_section_pattern_a().is_match(text)
        || historical_requirement_section_pattern_b().is_match(text)
}

fn has_historical_requirement_note_cue(text: &str) -> bool {
    !has_historical_requirement_section_cue(text)
        && historical_requirement_note_pattern_a().is_match(text)
        && historical_requirement_note_pattern_b().is_match(text)
}

fn has_current_curriculum_boundary_cue(text: &str) -> bool {
    current_curriculum_boundary_pattern_a().is_match(text)
        || current_curriculum_boundary_pattern_b().is_match(text)
        || current_curriculum_boundary_pattern_c().is_match(text)
}

fn is_inherited_historical_requirement_section(
    inherited_role: &str,
    inherited_role_reason: &str,
    text: &str,
) -> bool {
    inherited_role == "support-metadata"
        && inherited_role_reason.contains("historical requirement section")
        && !has_current_curriculum_boundary_cue(text)
        && !is_undergraduate_catalog_section_reset_line(text)
}

fn should_inherit_graduate_requirement_section(
    current_role: &str,
    current_role_reason: &str,
    line: &str,
) -> bool {
    current_role == "support-metadata"
        && current_role_reason.contains("graduate requirement context")
        && !super::requirement_recovery_scope::has_primary_requirement_section_cue(line)
        && !has_undergraduate_requirement_reset_cue(line)
        && !is_undergraduate_catalog_section_reset_line(line)
}

fn is_tacoma_rn_bsn_sample_program_plan_reset_line(source_url: &str, line: &str) -> bool {
    tacoma_rn_bsn_sample_program_plan_source_pattern().is_match(source_url)
        && tacoma_rn_bsn_sample_program_plan_reset_pattern().is_match(&normalize_whitespace(line))
}

fn historical_requirement_section_matches_owner(owner: &Value, text: &str) -> bool {
    let owner_context = ["ownerId", "ownerTitle", "pathwayId", "label"]
        .iter()
        .filter_map(|key| owner.get(*key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let normalized_owner_context = normalize_prior_admit_text(&owner_context);
    let normalized_section = normalize_prior_admit_text(text);
    let owner_era = prior_admit_era_pattern().captures(&normalized_owner_context);
    let section_era = prior_admit_era_pattern().captures(&normalized_section);
    match (owner_era, section_era) {
        (Some(owner_era), Some(section_era)) => {
            owner_era.get(1).map(|term| normalize_term(term.as_str()))
                == section_era.get(1).map(|term| normalize_term(term.as_str()))
                && owner_era.get(2).map(|year| year.as_str())
                    == section_era.get(2).map(|year| year.as_str())
        }
        _ => false,
    }
}

fn normalize_prior_admit_text(value: &str) -> String {
    prior_admit_separator_pattern()
        .replace_all(value, " ")
        .to_lowercase()
}

fn normalize_term(term: &str) -> String {
    match term.to_ascii_lowercase().as_str() {
        "fall" => "autumn".to_string(),
        value => value.to_string(),
    }
}

fn js_compatible_role_pattern(pattern: &str) -> Regex {
    let compatible_pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
    Regex::new(&compatible_pattern).expect("JavaScript-compatible source-role pattern")
}

fn degree_route_comparison_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(r"(?i)\bbachelors?\s+of\s+science\s+or\s+arts?\b")
    })
}

fn degree_route_comparison_terminator_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\blearn\s+more\s+about\s+b\.?\s*s\.?\s+in\b|^(?:b\.?\s*[as]\.?(?:\s+in\s+.{1,80})?\s+requirements?|(?:degree|major|completion)\s+requirements?)\b"))
}

fn prerequisite_only_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:prereq(?:uisites?)?|recommended preparation|course prerequisites?|minimum grade prerequisite|minimum grade required|cse\s*300[-\s]?level prerequisites?)\b"))
}

fn course_list_section_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:course lists?|list of courses|curriculum list|curricular list|tracks?|focus areas?|areas of specialization|areas of concentration|computing specializations?)\b"))
}

fn distribution_area_course_list_section_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:distribution areas?|historical depth(?: courses?)?|power and difference(?: courses?)?|genre,?\s+method,?\s+and language(?: courses?)?|pre-[0-9]{3,4}|post-[0-9]{3,4}|genre|method|language)$").expect("distribution-area section cue"))
}

fn academic_breadth_subject_area_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(
            r"(?i)\bacademic breadth\b.{0,80}\b(?:subject areas?|courses?)\b",
        )
    })
}

fn subject_areas_and_courses_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\bsubject areas?\s*&\s*courses?\b"))
}

fn approved_course_list_section_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\bapproved\b.{0,80}\b(?:courses?|course list|list)\b|\b(?:courses?|course list|list)\b.{0,80}\bapproved\b"))
}

fn elective_list_section_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:approved electives?|elective list|elective courses?|technical electives?|free electives?)\b"))
}

fn numbered_requirement_course_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^(?:[IVXLCDM]+|[0-9]+|[A-Z])\.\s+\S")
            .expect("numbered requirement course heading")
    })
}

fn course_word_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\bcourses?\b"))
}

fn excluded_numbered_requirement_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:admissions?|advising|application|career|contacts?|faculty|resources?|scholarships?|students?)\b"))
}

fn likely_source_heading_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(r"(?i)\bT\s*ELEC\s*[0-9]+\b|^UWT Elective Course$")
    })
}

fn sentence_ending_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"[.!?]$").expect("sentence ending"))
}

fn likely_source_heading_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:requirements?|curriculum|courses?|electives?|options?|concentrations?|tracks?|preparation|prereq(?:uisites?)?)\b"))
}

fn likely_source_heading_body_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(
            r"(?i)\b(?:as a|by taking|in general|make sure|cannot overlap|together|within|below)\b",
        )
    })
}

fn explicit_choice_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(
            r"(?i)\b(?:choose|select|one\s+of|one\s+course\s+from|credits?\s+from)\b",
        )
    })
}

fn credit_bearing_course_row_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\(\s*[0-9]+(?:\.[0-9]+)?(?:\s*(?:-|to)\s*[0-9]+(?:\.[0-9]+)?)?\s*(?:credits?|cr\.?)?(?:\s*;[^)]*)?\)"))
}

fn credit_bearing_elective_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b[0-9]+\s*(?:-\s*[0-9]+)?\s*credits?\b"))
}

fn credit_bearing_capstone_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(
            r"(?i)\b(?:capstone|choose\s+one\s+track)\b.*\b[0-9]+\s*(?:-\s*[0-9]+)?\s*credits?\b",
        )
    })
}

fn admission_prep_section_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:admission|application|apply|preparation|before applying|before admission|prior to admission|pre-major)\b"))
}

fn admission_prep_heading_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:admissions?|application|apply|prereq(?:uisites?)?|before applying|before admission|prior to admission|pre-major)\b"))
}

fn recommended_preparation_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^recommended preparation$").expect("recommended preparation heading")
    })
}

fn post_admission_degree_completion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:year\s*(?:3|4|3\s*\+\s*4|three|four)|junior\s+year|senior\s+year)\b.{0,80}\b(?:requirements?|curriculum|courses?)\b"))
}

fn post_admission_year_course_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(r"(?i)\byear\s*(?:3|4)\b.{0,80}\b[A-Z&]{2,}\s*[0-9]{3}[A-Z]?\b")
    })
}

fn undergraduate_requirement_reset_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:curriculum|degree options?|major options?|courses?\s+for\s+graduation|core courses?)\b"))
}

fn graduate_degree_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:graduate|master(?:'s|s)?|doctoral|doctorate|ph\.?\s*d\.?|m\.?\s*a\.?|m\.?\s*s\.?)\b"))
}

fn undergraduate_catalog_reset_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)^(?:undergraduate\s+programs?|program\s+of\s+study:\s+major\b|bachelor\s+of\b|bachelor\b.+\bmajor\b|b\.?\s*[as]\.\s+in\b)"))
}

fn support_post_completion_course_pattern_a() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:after|once|when)\s+(?:complet(?:ing|ed)|finish(?:ing|ed))\b.{0,180}\b(?:students?\s+)?(?:may|can)\s+(?:also\s+)?take\b"))
}

fn support_post_completion_course_pattern_b() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\bfulfill(?:ing|ed)?\s+(?:the\s+)?(?:university|college|campus)\s+requirements?\b"))
}

fn support_post_completion_course_pattern_c() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(r"(?i)\b(?:students?\s+)?(?:may|can)\s+(?:also\s+)?take\b")
    })
}

fn support_post_completion_course_pattern_d() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(r"(?i)\bmay\s+have\s+to\s+take\s+additional\s+courses?\b")
    })
}

fn support_post_completion_course_pattern_e() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\bencouraged\s+to\s+(?:take|enroll)\b"))
}

fn restricted_registration_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\bapplication[-\s]?only\b|\bby\s+application\s+only\b|\bduring\s+period\s+3\s+of\s+registration\b"))
}

fn bare_honors_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE
        .get_or_init(|| Regex::new(r"(?i)^(?:[A-Z& ]+\s+)?honou?rs$").expect("bare honors heading"))
}

fn honors_support_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:with\s+honors|honors\s+designation|honors\s+thesis|honors\s+requirements?|program\s+honors|to\s+qualify\s+for\b.{0,80}\bhonors|pursue\s+honors)\b"))
}

fn contrasting_honors_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:regular|standard|accelerated)\b.{0,60}\bor\b.{0,60}\bhonors?\b|\bhonors?\b.{0,60}\bor\b.{0,60}\b(?:regular|standard|accelerated|equivalent)\b"))
}

fn optional_course_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\boptional\b"))
}

fn workshop_course_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\bworkshop\s+style\s+course\b"))
}

fn academic_support_course_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:help|helps)\s+(?:you|students?)\s+(?:solve|develop|prepare|review|practice)\b"))
}

fn suggested_course_list_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(
            r"(?i)\b(?:suggested|recommended)\b.{0,80}\b(?:first|second|college|courses?)\b",
        )
    })
}

fn admission_requirement_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(r"(?i)\b(?:requirements?|prereq(?:uisites?)?|preparation)\b")
    })
}

fn labeled_course_row_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"^[^:]{2,120}:\s+\S"))
}

fn prerequisite_table_exclusion_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:prerequisite table|course #|course name|required prior|recommended preparation)\b"))
}

fn lower_division_prerequisite_choice_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(r"(?i)\b(?:can take|take either|choose|select|or)\b")
    })
}

fn prerequisite_satisfaction_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(r"(?i)\bsatisf(?:y|ies)\b.{0,80}\bprereq(?:uisite)?\b")
    })
}

fn upper_division_reference_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:upper[-\s]?division|[34]00[-\s]?level|cse\s*300[-\s]?level|prerequisite table)\b"))
}

fn admission_prep_upper_division_reference_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(
            r"(?i)\b(?:upper[-\s]?division|[34]00[-\s]?level|cse\s*300[-\s]?level)\b",
        )
    })
}

fn is_non_schedulable_course_list_subheading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)^(?:physical science|life science|earth/space science|earth\s+and\s+space science|english language arts(?::\s*literature)?|social studies|mathematics|science)$"))
}

fn credit_bucket_requirement_cue_pattern_a() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:minimum(?:\s+of)?|at\s+least|min\.?)?\s*[0-9]+(?:\s*(?:-|to)\s*[0-9]+)?\s+(?:additional\s+)?credits?(?:\s*\([^)]*\))?\s+(?:from|of)\b"))
}

fn credit_bucket_requirement_cue_pattern_b() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:minimum(?:\s+of)?|at\s+least|min\.?)?\s*[0-9]+(?:\s*(?:-|to)\s*[0-9]+)?\s+(?:additional\s+)?credits?\s+(?:primary|secondary)?\s*language\b"))
}

fn credit_bucket_requirement_cue_pattern_c() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:minimum(?:\s+of)?|at\s+least|min\.?)?\s*[0-9]+(?:\s*(?:-|to)\s*[0-9]+)?\s+(?:additional\s+)?credits?\b.{0,100}\bcomplete\b.{0,100}\b(?:second|third|fourth)[-\s]?year\s+level\b"))
}

fn credit_bucket_requirement_cue_pattern_d() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:minimum(?:\s+of)?|at\s+least|min\.?)?\s*[0-9]+(?:\s*(?:-|to)\s*[0-9]+)?\s+(?:additional\s+)?credits?\b.{0,80}\b(?:elective coursework|courses?)\b.{0,80}\b(?:from|below|lists?)\b"))
}

fn credit_limit_support_cue_pattern_a() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:up\s+to|maximum(?:\s+of)?|no\s+more\s+than)\s+[0-9]+(?:\.[0-9]+)?\s+credits?\b"))
}

fn credit_limit_support_cue_pattern_b() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(
            r"(?i)\b(?:may|can|could)\s+(?:be\s+)?(?:appl(?:y|ied)|count(?:ed)?|used)\b",
        )
    })
}

fn historical_requirement_section_pattern_a() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)^for\s+students?\s+admitted\b.{0,80}\b(?:before|prior to|pre[-\s]?)\b.{0,60}\b(?:spring|summer|autumn|fall|winter)\s+[0-9]{4}\b"))
}

fn historical_requirement_section_pattern_b() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)^(?:pre|before|prior to)[-\s]*(?:spring|summer|autumn|fall|winter)\s+[0-9]{4}\b.{0,80}\b(?:degree\s+)?requirements?\b"))
}

fn historical_requirement_note_pattern_a() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\bstudents?\s+admitted\b.{0,80}\b(?:before|prior to|pre[-\s]?)\b.{0,60}\b(?:spring|summer|autumn|fall|winter)\s+[0-9]{4}\b"))
}

fn historical_requirement_note_pattern_b() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(
            r"(?i)\b(?:existing|current|previous|old)\s+(?:degree\s+)?requirements?\b",
        )
    })
}

fn prior_admit_era_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(
            r"(?i)\b(?:pre|before|prior\s+to)\s+(spring|summer|autumn|fall|winter)\s+([0-9]{4})\b",
        )
    })
}

fn prior_admit_separator_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"[_:-]+"))
}

fn credit_parenthetical_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(r"(?i)\(\s*[0-9]+(?:\.[0-9]+)?\s*(?:credits?|cr)?\s*\)")
    })
}

fn labeled_route_course_row_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)^[^:]{2,90}:\s+"))
}

fn upper_division_table_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:table|prereq|prerequisite|course #|course name|required prior|recommended preparation)\b"))
}

fn current_curriculum_boundary_pattern_a() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(
            r"(?i)^(?:current|new)\s+(?:degree\s+)?(?:curriculum|requirements?)\b",
        )
    })
}

fn current_curriculum_boundary_pattern_b() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(
            r"(?i)^(?:curriculum|requirements?)\s+(?:for\s+)?(?:current|new)\s+students?\b",
        )
    })
}

fn current_curriculum_boundary_pattern_c() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:beginning|effective)\s+(?:in\s+)?(?:spring|summer|autumn|fall|winter)\s+[0-9]{4}\b"))
}

fn tacoma_rn_bsn_sample_program_plan_source_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_role_pattern(
            r"(?i)//www\.tacoma\.uw\.edu/nursing/rn-bsn-sample-program-plans(?:$|[?#/])",
        )
    })
}

fn tacoma_rn_bsn_sample_program_plan_reset_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_role_pattern(r"(?i)\b(?:rn-bsn\s+sample\s+program\s+plans?|sample\s+program\s+plans?\s+for\s+(?:summer|autumn)\s+cohort)\b"))
}
