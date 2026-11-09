use super::requirement_course_codes::{
    CourseCodeParser, get_explicitly_excluded_course_codes_from_lines, normalize_course_code,
};
use super::requirement_ids::slugify_requirement_identifier;
use super::requirement_scope::RequirementSourceScope;
use super::requirement_shape::{
    SEQUENCE_ALTERNATIVE_CHOICE_LINE_PATTERN_SOURCE, SEQUENCE_CHOICE_CONTEXT_PATTERN_SOURCE,
    SEQUENCE_LABEL_ALTERNATIVE_PATTERN_SOURCE, strip_snapshot_page_prefix,
};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashSet};
use std::sync::OnceLock;

pub fn build_course_emission_kind(
    course_code: &str,
    source_scope: RequirementSourceScope,
    parsed_requirement_groups: &[Value],
    parsed_requirement_courses: &[Value],
    parsed_requirement_atom_candidates: &[Value],
) -> &'static str {
    let normalized_course_code = normalize_course_code(course_code);
    if normalized_course_code.is_empty() {
        return "hidden-support-metadata";
    }
    if source_scope.can_create_required_rows
        && (parsed_requirement_courses.iter().any(|course| {
            course.get("optionRole").and_then(Value::as_str) == Some("required")
                && normalize_course_code(&first_non_nullish(
                    course,
                    &["normalizedCourseCode", "courseCode"],
                )) == normalized_course_code
        }) || parsed_requirement_atom_candidates.iter().any(|candidate| {
            normalize_course_code(&string_value(candidate.get("uwCourseCode")))
                == normalized_course_code
        }))
    {
        return "required-row";
    }
    if source_scope.can_create_option_groups
        && parsed_requirement_groups.iter().any(|group| {
            array(group.get("options")).iter().any(|option| {
                ["uwCourses", "equivalentUwCourseCodes", "displayCourseCodes"]
                    .iter()
                    .flat_map(|key| array(option.get(key)))
                    .any(|code| {
                        normalize_course_code(&string_value(Some(code))) == normalized_course_code
                    })
            })
        })
    {
        return "option-group";
    }
    if source_scope.can_create_approved_filters {
        return "approved-list-entry";
    }
    if source_scope.can_create_elective_lists {
        return "elective-list-entry";
    }
    "hidden-support-metadata"
}

pub fn build_source_scope_audit_lines(
    owner_id: Option<&Value>,
    source_url: Option<&Value>,
    source_role: Option<&str>,
    source_scope: RequirementSourceScope,
    course_codes: &[String],
    parsed_requirement_groups: &[Value],
    parsed_requirement_courses: &[Value],
    parsed_requirement_atom_candidates: &[Value],
) -> Vec<String> {
    let course_codes = course_codes.iter().cloned().collect::<BTreeSet<_>>();
    course_codes
        .into_iter()
        .map(|course_code| {
            let emitted_as = build_course_emission_kind(
                &course_code,
                source_scope,
                parsed_requirement_groups,
                parsed_requirement_courses,
                parsed_requirement_atom_candidates,
            );
            let scheduled = source_scope.can_create_schedule_rows
                && matches!(emitted_as, "required-row" | "option-group");
            let issue = if source_scope.support_only
                && matches!(emitted_as, "required-row" | "option-group")
            {
                "support-source-created-required-row"
            } else if source_scope.non_schedulable && scheduled {
                "non-schedulable-source-scheduled"
            } else {
                "none"
            };
            [
                "[source scope audit]".to_string(),
                format!("Owner id: {}", js_value_or(owner_id, "unknown")),
                format!("Source URL: {}", js_value_or(source_url, "n/a")),
                format!("Source role: {}", source_role.unwrap_or("ignored")),
                format!("Support-only: {}", yes_no(source_scope.support_only)),
                format!(
                    "Can create required rows: {}",
                    yes_no(source_scope.can_create_required_rows)
                ),
                format!(
                    "Can create option groups: {}",
                    yes_no(source_scope.can_create_option_groups)
                ),
                format!(
                    "Can create approved filters: {}",
                    yes_no(source_scope.can_create_approved_filters)
                ),
                format!("Course code: {course_code}"),
                format!("Emitted as: {emitted_as}"),
                format!("Scheduled: {}", yes_no(scheduled)),
                format!("Issue: {issue}"),
            ]
            .join(" ")
        })
        .collect()
}

pub fn infer_approved_list_key_from_support_metadata(input: &Value) -> Option<String> {
    let context = [
        "planId",
        "ownerId",
        "ownerTitle",
        "sourceLabel",
        "sourceUrl",
        "primarySourceLabel",
        "primarySourceUrl",
    ]
    .into_iter()
    .filter_map(|field| input.get(field))
    .filter(|value| js_truthy(value))
    .map(js_value)
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase();
    let science_context = Regex::new(r"(?-u:\b)(?:natural science|science)(?-u:\b)")
        .expect("valid approved-list science pattern")
        .is_match(&context);
    match input.get("planId").and_then(Value::as_str) {
        Some("uw-seattle-computer-engineering") if science_context => {
            return Some("computer-engineering-natural-science".to_string());
        }
        Some("uw-seattle-computer-science") if science_context => {
            return Some("computer-science-approved-science".to_string());
        }
        _ => {}
    }
    let approved_elective_context = Regex::new(r"(?-u:\b)approved(?-u:\b)")
        .expect("valid approved cue pattern")
        .is_match(&context)
        && Regex::new(r"(?-u:\b)electives?(?-u:\b)")
            .expect("valid elective cue pattern")
            .is_match(&context);
    if approved_elective_context {
        let owner = ["planId", "ownerId"]
            .into_iter()
            .filter_map(|field| input.get(field))
            .find(|value| js_truthy(value))
            .map(js_value)
            .unwrap_or_else(|| "unknown-owner".to_string());
        return Some(format!(
            "{}-approved-electives",
            slugify_requirement_identifier(&owner)
        ));
    }
    None
}

pub fn build_requirement_support_lists_from_metadata(input: &Value) -> Vec<Value> {
    let approved_codes = normalize_support_course_codes(input.get("approvedFilterUwCourseCodes"));
    let elective_codes = normalize_support_course_codes(input.get("electiveListUwCourseCodes"));
    let support_only_codes = normalize_support_course_codes(input.get("supportOnlyUwCourseCodes"));
    let source_role = string_value(input.get("sourceRole"));
    let used_codes = approved_codes
        .iter()
        .chain(elective_codes.iter())
        .cloned()
        .collect::<HashSet<_>>();
    let remaining_support_only_codes =
        if source_role == "approved-course-list" && !approved_codes.is_empty() {
            Vec::new()
        } else {
            support_only_codes
                .into_iter()
                .filter(|course_code| !used_codes.contains(course_code))
                .collect()
        };
    let approved_list_key = infer_approved_list_key_from_support_metadata(input);
    let mut support_lists = Vec::new();
    if !approved_codes.is_empty()
        || (approved_list_key.is_some() && source_role == "approved-course-list")
    {
        support_lists.push(build_support_list_with_shape(
            input,
            "approved-filter-list",
            &approved_codes,
            approved_list_key.as_deref(),
        ));
    }
    if !elective_codes.is_empty() {
        support_lists.push(build_support_list_with_shape(
            input,
            "elective-list",
            &elective_codes,
            None,
        ));
    }
    if !remaining_support_only_codes.is_empty() {
        let shape = match source_role.as_str() {
            "elective-list" => Some("elective-list"),
            "approved-course-list"
            | "admission-prerequisite-source"
            | "admissions-preparation"
            | "support-source" => Some("approved-course-list"),
            _ => None,
        };
        if let Some(shape) = shape {
            let list_key = if shape == "approved-course-list" {
                approved_list_key.as_deref()
            } else {
                None
            };
            support_lists.push(build_support_list_with_shape(
                input,
                shape,
                &remaining_support_only_codes,
                list_key,
            ));
        }
    }
    dedupe_requirement_support_lists(support_lists)
}

fn normalize_support_course_codes(value: Option<&Value>) -> Vec<String> {
    array(value)
        .iter()
        .map(|course_code| normalize_course_code(&string_value(Some(course_code))))
        .filter(|course_code| !course_code.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn build_support_list_with_shape(
    input: &Value,
    shape: &str,
    course_codes: &[String],
    approved_list_key: Option<&str>,
) -> Value {
    let mut metadata = input.as_object().cloned().unwrap_or_default();
    metadata.insert("shape".to_string(), json!(shape));
    metadata.insert("acceptedUwCourseCodes".to_string(), json!(course_codes));
    metadata.insert(
        "approvedListKey".to_string(),
        approved_list_key
            .map(|key| json!(key))
            .unwrap_or(Value::Null),
    );
    build_requirement_support_list_from_metadata(&Value::Object(metadata))
}

pub fn build_requirement_support_list_from_metadata(input: &Value) -> Value {
    let source_url = first_truthy(input, &["sourceUrl", "primarySourceUrl"]);
    let list_title = first_truthy(input, &["sourceLabel", "primarySourceLabel"])
        .map(|value| normalize_whitespace(&js_value(value)))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if input.get("shape").and_then(Value::as_str) == Some("elective-list") {
                "Elective list".to_string()
            } else {
                "Approved course list".to_string()
            }
        });
    let base_id = first_truthy(input, &["id"])
        .map(js_value)
        .unwrap_or_else(|| {
            let fallback_seed = source_url
                .map(js_value)
                .unwrap_or_else(|| list_title.clone());
            slugify_requirement_identifier(&fallback_seed)
        });
    let shape = input
        .get("shape")
        .map(js_value)
        .unwrap_or_else(|| "undefined".to_string());
    let accepted_codes = array(input.get("acceptedUwCourseCodes"))
        .iter()
        .map(|code| normalize_course_code(&string_value(Some(code))))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    json!({
        "id": format!("{base_id}:support-list:{shape}"),
        "shape": input.get("shape").cloned().unwrap_or(Value::Null),
        "sourceUrl": source_url.cloned().unwrap_or(Value::Null),
        "sourceRole": input.get("sourceRole").filter(|value| !value.is_null()).cloned().unwrap_or(Value::Null),
        "listTitle": list_title,
        "acceptedUwCourseCodes": accepted_codes,
        "approvedListKey": input.get("approvedListKey").filter(|value| !value.is_null()).cloned().unwrap_or(Value::Null),
        "supportOnly": true,
        "canCreateRequiredRow": false,
        "canCreateScheduleRow": false,
        "linkedPrimaryRequirementIds": [],
    })
}

pub fn build_support_lists_from_source_section_audit_rows(input: &Value) -> Vec<Value> {
    let source_url = input.get("sourceUrl").filter(|value| !value.is_null());
    let source_url_text = source_url.map(js_value).unwrap_or_default();
    let parser_type = input
        .get("parserType")
        .filter(|value| !value.is_null())
        .map(js_value)
        .unwrap_or_default();
    if url_looks_like_pdf(&source_url_text) || parser_type.to_ascii_lowercase().starts_with("pdf-")
    {
        return Vec::new();
    }
    let mut groups = Vec::<(String, String, String, BTreeSet<String>)>::new();
    for row in array(input.get("sourceSectionFilterAuditRows")) {
        if row.get("schedulable").is_some_and(js_truthy) {
            continue;
        }
        let Some(role) = row.get("detectedSectionRole").and_then(Value::as_str) else {
            continue;
        };
        if role != "approved-course-list" && role != "elective-list" {
            continue;
        }
        let accepted_codes = array(row.get("courseCodesExtracted"))
            .iter()
            .map(|code| normalize_course_code(&string_value(Some(code))))
            .filter(|code| !code.is_empty())
            .collect::<BTreeSet<_>>();
        if accepted_codes.is_empty() {
            continue;
        }
        let list_title = normalize_whitespace(
            &row.get("sectionTitle")
                .map(|value| string_value(Some(value)))
                .unwrap_or_default(),
        );
        let list_title = if list_title.is_empty() {
            if role == "elective-list" {
                "Elective list"
            } else {
                "Approved course list"
            }
            .to_string()
        } else {
            list_title
        };
        let key = format!("{role}:{list_title}");
        if let Some(group_index) = groups
            .iter()
            .position(|(existing_key, _, _, _)| existing_key == &key)
        {
            groups[group_index].3.extend(accepted_codes);
        } else {
            groups.push((key, role.to_string(), list_title, accepted_codes));
        }
    }
    groups
        .into_iter()
        .map(|(_, role, list_title, accepted_codes)| {
            let shape = if role == "elective-list" {
                "elective-list"
            } else {
                "approved-course-list"
            };
            let id = format!(
                "{}:source-section:{}",
                js_value_or_undefined(input.get("id")),
                slugify_requirement_identifier(&list_title)
            );
            let plan_or_owner = first_truthy(input, &["planId", "ownerId"])
                .map(js_value)
                .unwrap_or_else(|| "unknown-owner".to_string());
            let approved_list_key = if shape == "approved-course-list" {
                Value::String(format!(
                    "{}:{}",
                    slugify_requirement_identifier(&plan_or_owner),
                    slugify_requirement_identifier(&list_title)
                ))
            } else {
                Value::Null
            };
            let metadata = json!({
                "id": id,
                "shape": shape,
                "sourceLabel": list_title,
                "acceptedUwCourseCodes": accepted_codes.into_iter().collect::<Vec<_>>(),
                "approvedListKey": approved_list_key,
                "sourceUrl": input.get("sourceUrl"),
                "primarySourceUrl": input.get("primarySourceUrl"),
                "sourceRole": input.get("sourceRole"),
                "primarySourceLabel": input.get("primarySourceLabel"),
            });
            build_requirement_support_list_from_metadata(&metadata)
        })
        .collect()
}

pub fn build_support_lists_from_parsed_requirement_groups(
    input: &Value,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let mut support_lists = Vec::new();
    for group in array(input.get("parsedRequirementGroups")) {
        let approved_list_key = normalize_whitespace(&string_value(group.get("approvedListKey")));
        if approved_list_key.is_empty()
            || group.get("programSpecific").and_then(Value::as_bool) != Some(true)
        {
            continue;
        }
        let source_text = nullish(
            group.get("sourceRowText"),
            nullish(group.get("sourceHeading"), group.get("label")),
        )
        .map(js_value)
        .unwrap_or_default();
        let Some(descriptor) = super::requirement_groups::detect_subject_prefix_credit_bucket(
            input,
            &source_text,
            course_parser,
        ) else {
            continue;
        };
        let approved_codes = array(descriptor.get("approvedUwCourseCodes"));
        if approved_codes.is_empty()
            || string_value(descriptor.get("approvedListKey")) != approved_list_key
        {
            continue;
        }
        let Some(subject_prefix) = descriptor.get("subjectPrefix").and_then(Value::as_str) else {
            continue;
        };
        let mut accepted_codes = approved_codes
            .iter()
            .map(|course_code| normalize_course_code(&string_value(Some(course_code))))
            .filter(|course_code| !course_code.is_empty())
            .collect::<BTreeSet<_>>();
        for course_code in array(input.get("parsedUwCourseCodes")) {
            let normalized_course_code = normalize_course_code(&string_value(Some(course_code)));
            if normalized_course_code.starts_with(&format!("{subject_prefix} ")) {
                accepted_codes.insert(normalized_course_code);
            }
        }
        let mut metadata = input.as_object().cloned().unwrap_or_default();
        metadata.insert(
            "id".to_string(),
            group.get("id").cloned().unwrap_or(Value::Null),
        );
        metadata.insert("shape".to_string(), json!("approved-filter-list"));
        metadata.insert(
            "sourceLabel".to_string(),
            group.get("label").cloned().unwrap_or(Value::Null),
        );
        metadata.insert(
            "acceptedUwCourseCodes".to_string(),
            json!(accepted_codes.into_iter().collect::<Vec<_>>()),
        );
        metadata.insert("approvedListKey".to_string(), json!(approved_list_key));
        support_lists.push(build_requirement_support_list_from_metadata(
            &Value::Object(metadata),
        ));
    }
    dedupe_requirement_support_lists(support_lists)
}

pub fn get_approved_filter_codes_for_support_source(
    input: &Value,
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    if let Some(codes) = extract_approved_filter_evidence_from_lines(input, course_parser)
        .and_then(|evidence| evidence.get("approvedUwCourseCodes").cloned())
        .and_then(|codes| codes.as_array().cloned())
    {
        if !codes.is_empty() {
            return codes
                .iter()
                .map(|code| normalize_course_code(&string_value(Some(code))))
                .filter(|code| !code.is_empty())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
        }
    }
    let fallback_codes = normalize_support_course_codes(input.get("fallbackCourseCodes"));
    let source_lines = array(input.get("lines"))
        .iter()
        .map(|line| strip_source_line_noise(&string_value(Some(line))))
        .collect::<Vec<_>>()
        .join(" ");
    let fallback_subjects = fallback_codes
        .iter()
        .map(|course_code| course_code_subject(course_code))
        .collect::<HashSet<_>>();
    let filter_key = input.get("filterKey").map(js_value).unwrap_or_default();
    let natural_science_filter = approved_filter_science_filter_pattern().is_match(&filter_key);
    let owner_role = input
        .get("owner")
        .and_then(|owner| owner.get("sourceRole"))
        .map(js_value)
        .unwrap_or_default();
    if owner_role == "approved-course-list"
        && approved_filter_graduation_requirements_pattern().is_match(&source_lines)
        && (!approved_filter_approved_word_pattern().is_match(&source_lines)
            || (natural_science_filter
                && fallback_subjects
                    .iter()
                    .any(|subject| matches!(subject.as_str(), "CSE" | "ENGL" | "MATH" | "SOC"))))
    {
        return Vec::new();
    }
    fallback_codes
}

pub fn extract_approved_filter_evidence_from_lines(
    input: &Value,
    course_parser: &CourseCodeParser,
) -> Option<Value> {
    let lines = array(input.get("lines"))
        .iter()
        .map(|line| string_value(Some(line)))
        .collect::<Vec<_>>();
    let null = Value::Null;
    let filter_key = input.get("filterKey").map(js_value).unwrap_or_default();
    let cues = build_approved_filter_section_cues(
        &filter_key,
        input.get("owner").unwrap_or(&null),
        input.get("supportList").unwrap_or(&null),
    );
    let (start_index, end_index, heading) =
        find_approved_filter_section_range(&lines, &cues, course_parser)?;
    let mut evidence_lines = vec![heading.clone()];
    let mut evidence_headings = vec![heading];
    let mut petition_only_notes = Vec::new();
    let mut approved_course_groups = Vec::<Vec<String>>::new();
    let mut course_codes = Vec::new();
    let mut current_subject = None::<String>;
    for line in &lines[start_index..end_index] {
        let clean_line = strip_source_line_noise(line);
        if clean_line.is_empty() {
            continue;
        }
        if let Some(subject) = approved_filter_subject_from_heading(&clean_line, course_parser)
            .filter(|_| {
                course_parser.extract_from_line(&clean_line).is_empty()
                    && !approved_filter_course_number_pattern().is_match(&clean_line)
            })
        {
            current_subject = Some(subject);
            evidence_headings.push(clean_line.clone());
            evidence_lines.push(clean_line);
            continue;
        }
        let is_conditional_note = approved_filter_conditional_note_pattern().is_match(&clean_line);
        if is_conditional_note {
            petition_only_notes.push(clean_line.clone());
        }
        let line_codes = extract_approved_filter_codes_from_line(
            &clean_line,
            current_subject.as_deref(),
            course_parser,
        );
        if !line_codes.is_empty() {
            evidence_lines.push(clean_line.clone());
            course_codes.extend(line_codes.iter().cloned());
            if approved_filter_alternative_pattern().is_match(&clean_line)
                && line_codes.len() > 1
                && line_codes.len() <= 4
            {
                approved_course_groups.push(line_codes);
            }
        } else if is_conditional_note {
            evidence_lines.push(clean_line);
        }
    }
    let approved_course_codes = course_codes.into_iter().collect::<BTreeSet<_>>();
    if approved_course_codes.is_empty() {
        return None;
    }
    let mut seen_groups = HashSet::new();
    let approved_course_groups = approved_course_groups
        .into_iter()
        .map(|group| {
            group
                .into_iter()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .filter(|group| group.len() > 1)
        .filter(|group| seen_groups.insert(group.join("|")))
        .collect::<Vec<_>>();
    Some(json!({
        "approvedUwCourseCodes": approved_course_codes,
        "approvedUwCourseGroups": approved_course_groups,
        "petitionOnlyNotes": unique_in_order(petition_only_notes),
        "sourceEvidenceHeadings": unique_in_order(evidence_headings),
        "sourceEvidenceLines": unique_in_order(evidence_lines).into_iter().take(80).collect::<Vec<_>>(),
    }))
}

fn build_approved_filter_section_cues(
    filter_key: &str,
    owner: &Value,
    support_list: &Value,
) -> (Vec<String>, Vec<String>) {
    let normalized_key = normalize_approved_filter_text(filter_key);
    let context = normalize_approved_filter_text(
        &[
            Some(filter_key.to_string()),
            owner.get("ownerTitle").map(js_value),
            owner.get("sourceLabel").map(js_value),
            support_list.get("listTitle").map(js_value),
            owner.get("sourceUrl").map(js_value),
        ]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" "),
    );
    let mut phrases = Vec::new();
    if context.contains("computer science") || normalized_key.contains("computer-science") {
        phrases.push("computer science".to_string());
    }
    if context.contains("computer engineering") || normalized_key.contains("computer-engineering") {
        phrases.push("computer engineering".to_string());
    }
    let mut category_tokens = Vec::new();
    if context.contains("natural science") || normalized_key.contains("natural-science") {
        category_tokens.extend(["natural".to_string(), "science".to_string()]);
    } else if context.contains("science") {
        category_tokens.push("science".to_string());
    }
    if context.contains("math science") || context.contains("math/science") {
        category_tokens.extend(["math".to_string(), "science".to_string()]);
    }
    if approved_filter_elective_pattern().is_match(&context) {
        category_tokens.push("elective".to_string());
    }
    (phrases, unique_in_order(category_tokens))
}

fn normalize_approved_filter_text(value: &str) -> String {
    let cleaned = strip_source_line_noise(value).to_lowercase();
    approved_filter_nonword_pattern()
        .replace_all(&cleaned, " ")
        .trim()
        .to_string()
}

fn get_approved_filter_section_heading_score(
    line: &str,
    cues: &(Vec<String>, Vec<String>),
    course_parser: &CourseCodeParser,
) -> usize {
    let text = normalize_approved_filter_text(line);
    if text.is_empty() || !course_parser.extract_from_line(line).is_empty() {
        return 0;
    }
    if cues.0.is_empty()
        && cues.1.is_empty()
        && !approved_filter_requirement_heading_pattern().is_match(&text)
    {
        return 0;
    }
    if cues.0.iter().any(|phrase| !text.contains(phrase))
        || cues.1.iter().any(|token| !text.contains(token))
    {
        return 0;
    }
    let mut score = 1 + cues.0.len() * 5 + cues.1.len() * 3;
    if approved_filter_heading_cue_pattern().is_match(&text) {
        score += 2;
    }
    score
}

fn find_approved_filter_section_range(
    lines: &[String],
    cues: &(Vec<String>, Vec<String>),
    course_parser: &CourseCodeParser,
) -> Option<(usize, usize, String)> {
    let (mut best_start, mut best_score) = (None, 0);
    for (index, line) in lines.iter().enumerate() {
        let score = get_approved_filter_section_heading_score(line, cues, course_parser);
        if score > best_score {
            best_start = Some(index);
            best_score = score;
        }
    }
    let start = best_start?;
    let mut end = lines.len();
    let mut collected_course_line = false;
    for (index, line) in lines.iter().enumerate().skip(start + 1) {
        let clean_line = strip_source_line_noise(line);
        if clean_line.is_empty() {
            continue;
        }
        if !extract_approved_filter_codes_from_line(&clean_line, None, course_parser).is_empty() {
            collected_course_line = true;
        }
        if collected_course_line
            && (approved_filter_section_stop_pattern().is_match(&clean_line)
                || (index > start + 1
                    && get_approved_filter_section_heading_score(line, cues, course_parser) > 0))
        {
            end = index;
            break;
        }
    }
    Some((start, end, strip_source_line_noise(&lines[start])))
}

fn extract_approved_filter_codes_from_line(
    line: &str,
    current_subject: Option<&str>,
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let mut clean_line = strip_source_line_noise(line);
    if clean_line.is_empty() {
        return Vec::new();
    }
    if let Some(note_start) = approved_filter_note_only_pattern()
        .find(&clean_line)
        .map(|m| m.start())
    {
        let prefix = &clean_line[..note_start];
        if approved_filter_course_number_pattern().is_match(prefix) {
            clean_line = prefix.to_string();
        } else {
            return Vec::new();
        }
    }
    let explicit_codes = course_parser.extract_from_line(&clean_line);
    let line_subject = approved_filter_leading_subject(&clean_line, course_parser);
    let prefix_subject = approved_filter_subject_from_heading(&clean_line, course_parser);
    let inherited_subject = line_subject
        .as_deref()
        .or(prefix_subject.as_deref())
        .or(current_subject);
    let should_inherit = line_subject.is_some()
        || prefix_subject.is_some()
        || approved_filter_number_list_start_pattern().is_match(&clean_line)
        || explicit_codes.iter().any(|code| {
            course_code_subject(code).as_str() == inherited_subject.unwrap_or_default()
        });
    let mut codes = explicit_codes;
    if should_inherit {
        if let Some(subject) = inherited_subject {
            codes.extend(extract_approved_filter_codes_with_subject(
                &clean_line,
                subject,
                course_parser,
            ));
        }
    }
    codes
        .into_iter()
        .map(|code| normalize_course_code(&code))
        .filter(|code| !code.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn approved_filter_subject_from_heading(
    line: &str,
    course_parser: &CourseCodeParser,
) -> Option<String> {
    let normalized = strip_source_line_noise(line)
        .trim_end_matches(|character| character == ':' || character == '.')
        .trim()
        .to_ascii_uppercase();
    if let Some(subject) = approved_filter_subject_alias(&normalized) {
        return Some(subject.to_string());
    }
    if let Some(capture) = approved_filter_colon_subject_pattern().captures(&normalized) {
        return course_parser.normalize_subject(capture.get(1)?.as_str());
    }
    course_parser.normalize_subject(&normalized)
}

fn approved_filter_leading_subject(line: &str, course_parser: &CourseCodeParser) -> Option<String> {
    let clean_line = strip_source_line_noise(line);
    let uppercase = clean_line.to_ascii_uppercase();
    for &(alias, subject) in approved_filter_subject_aliases() {
        if uppercase.starts_with(alias)
            && uppercase
                .as_bytes()
                .get(alias.len())
                .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
        {
            return Some(subject.to_string());
        }
    }
    let capture = approved_filter_leading_code_subject_pattern().captures(&clean_line)?;
    course_parser.normalize_subject(capture.get(1)?.as_str())
}

fn approved_filter_subject_alias(normalized: &str) -> Option<&'static str> {
    approved_filter_subject_aliases()
        .iter()
        .find(|(alias, _)| *alias == normalized)
        .map(|(_, subject)| *subject)
}

fn approved_filter_subject_aliases() -> &'static [(&'static str, &'static str)] {
    &[
        ("ASTRONOMY", "ASTR"),
        ("ATMOSPHERIC SCIENCE", "ATMOS"),
        ("ATMOSPHERIC SCIENCES", "ATMOS"),
        ("BIOLOGY", "BIOL"),
        ("CHEMISTRY", "CHEM"),
        ("EARTH & SPACE SCIENCES", "ESS"),
        ("EARTH AND SPACE SCIENCES", "ESS"),
        ("PHYSICS", "PHYS"),
    ]
}

pub(super) fn extract_approved_filter_codes_with_subject(
    line: &str,
    subject: &str,
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let Some(normalized_subject) = course_parser.normalize_subject(subject) else {
        return Vec::new();
    };
    let text = approved_filter_credit_text_pattern()
        .replace_all(line, "")
        .to_string();
    approved_filter_course_number_pattern()
        .find_iter(&text)
        .filter_map(|matched| {
            let before = text[..matched.start()]
                .chars()
                .rev()
                .take(8)
                .collect::<String>();
            let after = text[matched.end()..].chars().take(8).collect::<String>();
            if approved_filter_level_word_before_pattern().is_match(&before)
                || approved_filter_level_word_after_pattern().is_match(&after)
            {
                return None;
            }
            course_parser.normalize_code(&normalized_subject, matched.as_str())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn course_code_subject(course_code: &str) -> String {
    approved_filter_course_subject_pattern()
        .captures(course_code)
        .and_then(|capture| capture.get(1))
        .map(|subject| subject.as_str().to_string())
        .unwrap_or_default()
}

pub(super) fn strip_source_line_noise(line: &str) -> String {
    normalize_source_section_line(line)
}

fn approved_filter_science_filter_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)(?-u:\b)(?:natural[-\s]science|approved[-\s]science|science)(?-u:\b)")
            .expect("approved science filter")
    })
}

fn approved_filter_graduation_requirements_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)(?-u:\b)(?:graduation requirements?|degree requirements?)(?-u:\b)")
            .expect("approved graduation requirements")
    })
}

fn approved_filter_approved_word_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)(?-u:\b)approved(?-u:\b)").expect("approved word"))
}

fn approved_filter_nonword_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"[^a-z0-9&/]+").expect("approved filter text"))
}

fn approved_filter_requirement_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)(?-u:\b)(?:requirement|approved|courses?|list|electives?)(?-u:\b)")
            .expect("approved heading cue")
    })
}

fn approved_filter_heading_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)(?-u:\b)(?:requirement|approved|courses?|list)(?-u:\b)")
            .expect("approved heading score cue")
    })
}

fn approved_filter_elective_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)(?-u:\b)electives?(?-u:\b)").expect("approved elective cue")
    })
}

fn approved_filter_section_stop_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)(?-u:\b)(?:back to top|senior electives?|degree requirements?|graduation requirements?|admissions?|sample schedules?|polic(?:y|ies)|curriculum|course lists?)(?-u:\b)").expect("approved filter section stop"))
}

fn approved_filter_note_only_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)(?-u:\b)(?:not included in this list|check with|adviser|advisor|if you have not|pre-?requisite|prerequisite|recommended)(?-u:\b)").expect("approved filter note only"))
}

fn approved_filter_conditional_note_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)(?-u:\b)(?:petition|conditional|approved by|approval|adviser|advisor|not included in this list|pre-?requisite|prerequisite)(?-u:\b)").expect("approved filter conditional note"))
}

fn approved_filter_alternative_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)(?-u:\b)(?:and|or)(?-u:\b)|/|\*and\*")
            .expect("approved filter alternatives")
    })
}

fn approved_filter_course_number_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?-u:\b)[0-9]{3}[A-Za-z]?(?-u:\b)").expect("approved filter course number")
    })
}

fn approved_filter_colon_subject_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"^([A-Z&]+(?:\s+[A-Z&]+)?)\s*:").expect("approved filter heading subject")
    })
}

fn approved_filter_leading_code_subject_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^([A-Z&]+(?:\s+[A-Z&]+)?)\s*[: ]\s*[0-9]{3}")
            .expect("approved filter leading subject")
    })
}

fn approved_filter_number_list_start_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^\s*(?:[0-9]{3}|[,;/]|\(|and(?-u:\b)|or(?-u:\b))")
            .expect("approved filter number list")
    })
}

fn approved_filter_credit_text_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\([^)]*credits?[^)]*\)|(?-u:\b)[0-9]+\s*(?:credits?|cr)(?-u:\b)")
            .expect("approved filter credit noise")
    })
}

fn approved_filter_level_word_before_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)(?-u:\b)level[-\s]*$").expect("approved filter level before")
    })
}

fn approved_filter_level_word_after_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^[-\s]*level(?-u:\b)").expect("approved filter level after")
    })
}

fn approved_filter_course_subject_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^(.+?)\s+[0-9]{3}").expect("approved filter course subject"))
}

pub fn get_requirement_support_list_semantic_key(support_list: &Value) -> String {
    let shape = string_value(support_list.get("shape"));
    let source_url = string_value(support_list.get("sourceUrl"));
    let approved_list_key = string_value(nullish(
        support_list.get("approvedListKey"),
        support_list.get("filterKey"),
    ));
    if !approved_list_key.is_empty()
        && matches!(
            shape.as_str(),
            "approved-filter-list" | "approved-course-list"
        )
    {
        return format!("approved:{shape}:{approved_list_key}");
    }
    if let Some(id) = support_list.get("id").filter(|value| js_truthy(value)) {
        js_value(id)
    } else {
        format!(
            "{shape}:{source_url}:{}",
            string_value(support_list.get("listTitle"))
        )
    }
}

pub fn dedupe_requirement_support_lists(support_lists: Vec<Value>) -> Vec<Value> {
    let mut seen = HashSet::new();
    support_lists
        .into_iter()
        .filter(|support_list| seen.insert(get_requirement_support_list_semantic_key(support_list)))
        .collect()
}

pub fn get_globally_explicitly_excluded_course_codes(
    lines: &[String],
    audit_rows: &[Value],
    course_parser: &CourseCodeParser,
) -> Vec<String> {
    let explicitly_excluded = get_explicitly_excluded_course_codes_from_lines(lines, course_parser);
    if explicitly_excluded.is_empty() {
        return Vec::new();
    }
    let mut positive_schedulable_evidence = HashSet::new();
    for row in audit_rows {
        if row.get("schedulable").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let raw_line = normalize_whitespace(&string_value(row.get("rawLine")));
        for course_code in array(row.get("courseCodesExtracted")) {
            let normalized_course_code = normalize_course_code(&string_value(Some(course_code)));
            if !normalized_course_code.is_empty()
                && !course_parser
                    .line_explicitly_excludes_course_code(&raw_line, &normalized_course_code)
            {
                positive_schedulable_evidence.insert(normalized_course_code);
            }
        }
    }
    explicitly_excluded
        .into_iter()
        .filter(|course_code| {
            !positive_schedulable_evidence.contains(&normalize_course_code(course_code))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn filter_explicitly_excluded_course_codes(
    course_codes: &[String],
    excluded_course_codes: &[String],
) -> Vec<String> {
    let excluded = excluded_course_codes
        .iter()
        .map(|course_code| normalize_course_code(course_code))
        .collect::<HashSet<_>>();
    course_codes
        .iter()
        .map(|course_code| normalize_course_code(course_code))
        .filter(|course_code| !course_code.is_empty() && !excluded.contains(course_code))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn build_parser_sequence_choice_audit_rows(
    owner_id: Option<&Value>,
    source_url: Option<&Value>,
    parsed_requirement_groups: &[Value],
    parsed_requirement_atom_candidates: &[Value],
) -> Vec<Value> {
    let atom_course_codes = parsed_requirement_atom_candidates
        .iter()
        .map(|candidate| normalize_course_code(&string_value(candidate.get("uwCourseCode"))))
        .collect::<HashSet<_>>();
    parsed_requirement_groups
        .iter()
        .filter(|group| {
            group.get("requirementType").and_then(Value::as_str) == Some("sequence_choice")
        })
        .map(|group| {
            let paths = array(nullish(group.get("sequencePaths"), group.get("options")));
            let sequence_paths = paths
                .iter()
                .map(|path| {
                    let raw_label = nullish(path.get("label"), path.get("pathLabel"));
                    let label = normalize_whitespace(&string_value(raw_label));
                    let label = if label.is_empty() {
                        "sequence path"
                    } else {
                        &label
                    };
                    let uw_courses = unique_in_order(
                        array(path.get("uwCourses"))
                            .iter()
                            .map(|code| normalize_course_code(&string_value(Some(code))))
                            .filter(|code| !code.is_empty())
                            .collect(),
                    );
                    format!("{label}: {}", uw_courses.join(", "))
                })
                .filter(|path| !path.is_empty())
                .collect::<Vec<_>>();
            let sequence_path_course_codes = unique_in_order(
                paths
                    .iter()
                    .flat_map(|path| {
                        array(path.get("uwCourses"))
                            .into_iter()
                            .chain(array(path.get("conditionalLabCourses")))
                            .map(|code| string_value(Some(code)))
                            .collect::<Vec<_>>()
                    })
                    .collect(),
            )
            .into_iter()
            .map(|code| normalize_course_code(&code))
            .collect::<Vec<_>>();
            let emitted_independent_required_rows = sequence_path_course_codes
                .iter()
                .any(|course_code| atom_course_codes.contains(course_code));
            let issue = if emitted_independent_required_rows {
                "flattened-sequence-paths"
            } else {
                "none"
            };
            let owner_id = non_nullish_or(owner_id, "unknown");
            let source_url = non_nullish_or(source_url, "n/a");
            let raw_text = nullish(
                group.get("sourceRowText"),
                nullish(group.get("sourceHeading"), group.get("label")),
            );
            let mut row = Map::new();
            row.insert("ownerId".to_string(), owner_id.clone());
            row.insert("sourceUrl".to_string(), source_url.clone());
            if let Some(raw_text) = raw_text {
                row.insert("rawText".to_string(), raw_text.clone());
            }
            row.insert("detectedSequenceChoice".to_string(), json!(true));
            row.insert("sequencePaths".to_string(), json!(sequence_paths.clone()));
            row.insert("selectedDefaultPath".to_string(), Value::Null);
            row.insert(
                "emittedIndependentRequiredRows".to_string(),
                json!(emitted_independent_required_rows),
            );
            row.insert("issue".to_string(), json!(issue));
            let raw_text_debug = raw_text
                .map(js_value)
                .unwrap_or_else(|| "undefined".to_string());
            let debug_text = [
                "[parser sequence-choice audit]".to_string(),
                format!("Owner id: {}", js_value(&owner_id)),
                format!("Source URL: {}", js_value(&source_url)),
                format!("Raw text: {raw_text_debug}"),
                "Detected sequence choice: yes".to_string(),
                format!("Sequence paths: {}", sequence_paths.join(" | ")),
                "Selected/default path: none".to_string(),
                format!(
                    "Emitted independent required rows: {}",
                    yes_no(emitted_independent_required_rows)
                ),
                format!("Issue: {issue}"),
            ]
            .join(" ");
            row.insert("copyOnlyDebugText".to_string(), json!(debug_text));
            Value::Object(row)
        })
        .collect()
}

pub fn find_source_section_audit_row_for_text<'a>(
    text: &str,
    audit_rows: &'a [Value],
) -> Option<&'a Value> {
    let normalized_text = normalize_source_section_line(text);
    if normalized_text.is_empty() {
        return None;
    }
    audit_rows
        .iter()
        .find(|row| normalize_whitespace(&string_value(row.get("rawLine"))) == normalized_text)
        .or_else(|| {
            if !is_informative_source_section_text(&normalized_text) {
                return None;
            }
            audit_rows.iter().find(|row| {
                normalize_whitespace(&string_value(row.get("rawLine"))).contains(&normalized_text)
            })
        })
        .or_else(|| {
            audit_rows.iter().find(|row| {
                let row_text = normalize_whitespace(&string_value(row.get("rawLine")));
                is_informative_source_section_text(&row_text) && normalized_text.contains(&row_text)
            })
        })
}

pub fn source_section_decision_allows_course_option(option: &Value, audit_rows: &[Value]) -> bool {
    let explicit_codes = array(option.get("uwCourses"))
        .iter()
        .chain(array(option.get("equivalentUwCourseCodes")).iter())
        .cloned()
        .collect::<Vec<_>>();
    let raw_codes = if explicit_codes.is_empty() {
        array(option.get("displayCourseCodes")).to_vec()
    } else {
        explicit_codes
    };
    let mut sorted_raw_codes = raw_codes;
    sorted_raw_codes.sort_by_key(js_value);
    sorted_raw_codes.dedup_by(|left, right| left == right);
    let option_course_codes = sorted_raw_codes
        .into_iter()
        .map(|code| normalize_course_code(&string_value(Some(&code))))
        .filter(|code| !code.is_empty())
        .collect::<Vec<_>>();
    if option_course_codes.is_empty() {
        return true;
    }
    let source_row_text = normalize_whitespace(&string_value(option.get("sourceRowText")));
    let matching_text = if !source_row_text.is_empty() {
        source_row_text
    } else {
        first_truthy(option, &["label", "title", "sourceHeading"])
            .map(js_value)
            .unwrap_or_default()
    };
    let matched_row = find_source_section_audit_row_for_text(&matching_text, audit_rows);
    if matched_row
        .and_then(|row| row.get("schedulable"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        return true;
    }
    if matched_row
        .and_then(|row| row.get("schedulable"))
        .and_then(Value::as_bool)
        == Some(false)
    {
        return option_course_codes.iter().any(|course_code| {
            audit_rows.iter().any(|candidate_row| {
                candidate_row.get("schedulable").and_then(Value::as_bool) == Some(true)
                    && row_course_codes(candidate_row).contains(course_code)
            })
        });
    }
    let course_section_rows = option_course_codes
        .iter()
        .map(|course_code| {
            audit_rows
                .iter()
                .filter(|row| row_course_codes(row).contains(course_code))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let has_schedulable_course_row = course_section_rows.iter().any(|rows| {
        rows.iter()
            .any(|row| row.get("schedulable").and_then(Value::as_bool) == Some(true))
    });
    let has_blocked_course_row = course_section_rows.iter().any(|rows| {
        !rows.is_empty()
            && rows
                .iter()
                .all(|row| row.get("schedulable").and_then(Value::as_bool) != Some(true))
    });
    if has_blocked_course_row && !has_schedulable_course_row {
        return false;
    }
    has_schedulable_course_row || course_section_rows.iter().any(Vec::is_empty)
}

pub fn filter_parsed_requirement_option_codes_by_source_sections(
    option: &Value,
    audit_rows: &[Value],
) -> Option<Value> {
    if option.get("optionKind").and_then(Value::as_str) == Some("category-option") {
        return Some(option.clone());
    }
    let source_row_text = normalize_whitespace(&string_value(option.get("sourceRowText")));
    let matching_text = if !source_row_text.is_empty() {
        source_row_text
    } else {
        first_truthy(option, &["label", "title", "sourceHeading"])
            .map(js_value)
            .unwrap_or_default()
    };
    let matched_row = find_source_section_audit_row_for_text(&matching_text, audit_rows);
    if matched_row
        .and_then(|row| row.get("schedulable"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Some(option.clone());
    }
    let original_uw_courses = array(option.get("uwCourses"));
    let equivalent_courses = array(option.get("equivalentUwCourseCodes"));
    let should_filter_codes = matched_row
        .and_then(|row| row.get("schedulable"))
        .and_then(Value::as_bool)
        == Some(false)
        || original_uw_courses
            .iter()
            .chain(equivalent_courses.iter())
            .any(|course_code| {
                let rows = rows_for_course_code(course_code, audit_rows);
                !rows.is_empty()
                    && !rows
                        .iter()
                        .any(|row| row.get("schedulable").and_then(Value::as_bool) == Some(true))
            });
    if !should_filter_codes {
        return Some(option.clone());
    }
    let filtered_uw_courses = original_uw_courses
        .iter()
        .filter(|course_code| {
            source_section_rows_allow_course_code(course_code, audit_rows, matched_row)
        })
        .cloned()
        .collect::<Vec<_>>();
    let filtered_equivalent_courses = equivalent_courses
        .iter()
        .filter(|course_code| {
            source_section_rows_allow_course_code(course_code, audit_rows, matched_row)
        })
        .cloned()
        .collect::<Vec<_>>();
    if filtered_uw_courses.is_empty() && filtered_equivalent_courses.is_empty() {
        return None;
    }
    let display_course_codes = array(option.get("displayCourseCodes"));
    let filtered_display_course_codes = if display_course_codes.len() == original_uw_courses.len() {
        Some(json!(
            display_course_codes
                .iter()
                .enumerate()
                .filter(|(index, _)| filtered_uw_courses.contains(&original_uw_courses[*index]))
                .map(|(_, display_code)| display_code.clone())
                .collect::<Vec<_>>()
        ))
    } else {
        option.get("displayCourseCodes").cloned()
    };
    let mut filtered_option = option.as_object().cloned().unwrap_or_default();
    filtered_option.insert("uwCourses".to_string(), json!(filtered_uw_courses));
    filtered_option.insert(
        "equivalentUwCourseCodes".to_string(),
        json!(filtered_equivalent_courses),
    );
    if let Some(display_course_codes) = filtered_display_course_codes {
        filtered_option.insert("displayCourseCodes".to_string(), display_course_codes);
    } else {
        filtered_option.remove("displayCourseCodes");
    }
    Some(Value::Object(filtered_option))
}

pub fn filter_parsed_requirement_group_options_by_source_sections(
    group: &Value,
    audit_rows: &[Value],
) -> Value {
    let options = array(group.get("options"));
    if options.is_empty() {
        return group.clone();
    }
    let filtered_options = options
        .iter()
        .filter(|option| source_section_decision_allows_course_option(option, audit_rows))
        .filter_map(|option| {
            filter_parsed_requirement_option_codes_by_source_sections(option, audit_rows)
        })
        .collect::<Vec<_>>();
    if filtered_options.len() == options.len()
        && filtered_options
            .iter()
            .zip(options.iter())
            .all(|(filtered, original)| filtered == original)
    {
        return group.clone();
    }
    let mut filtered_group = group.as_object().cloned().unwrap_or_default();
    filtered_group.insert("options".to_string(), json!(filtered_options));
    Value::Object(filtered_group)
}

pub fn get_group_source_section_decision(
    group: &Value,
    audit_rows: &[Value],
    course_parser: &CourseCodeParser,
) -> Value {
    let group_text = nullish(
        group.get("sourceRowText"),
        nullish(group.get("sourceHeading"), group.get("label")),
    )
    .map(js_value)
    .unwrap_or_default();
    if let Some(row) = find_source_section_audit_row_for_text(&group_text, audit_rows) {
        return row.clone();
    }

    let option_cue = group
        .get("detectedOptionCue")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let special_case = if option_cue == "option replacement" {
        Some((
            "primary-requirement-section",
            "option replacement parser rule",
        ))
    } else if option_cue == "standalone course-count choice list" {
        Some((
            "primary-requirement-section",
            "standalone explicit course-count choice requirement",
        ))
    } else if sectioned_requirement_option_cue_pattern().is_match(option_cue) {
        Some((
            "primary-requirement-section",
            "sectioned requirement group parsed from source heading and following course rows",
        ))
    } else if group.get("requirementType").and_then(Value::as_str) == Some("sequence_choice")
        && has_sequence_choice_context(&group_text)
    {
        Some((
            "primary-requirement-section",
            "sequence-choice requirement cue",
        ))
    } else {
        None
    };
    if let Some((role, reason)) = special_case {
        return json!({
            "detectedSectionRole": role,
            "schedulable": true,
            "reason": reason,
        });
    }

    let source_section = group
        .get("sourceSection")
        .filter(|value| !value.is_null())
        .map(js_value)
        .unwrap_or_default();
    let fallback_text = format!("{source_section} {group_text}");
    let (role, reason) =
        super::requirement_section_roles::classify_requirement_group_source_section(
            &fallback_text,
            course_parser,
        );
    json!({
        "detectedSectionRole": role,
        "schedulable": matches!(role.as_str(), "primary-requirement-section" | "admission-prep-section"),
        "reason": reason,
    })
}

pub fn filter_parsed_requirement_groups_by_source_sections(
    groups: &[Value],
    audit_rows: &[Value],
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    groups
        .iter()
        .filter(|group| {
            get_group_source_section_decision(group, audit_rows, course_parser)
                .get("schedulable")
                .and_then(Value::as_bool)
                == Some(true)
        })
        .map(|group| filter_parsed_requirement_group_options_by_source_sections(group, audit_rows))
        .filter(parsed_requirement_group_has_requirement_surface)
        .collect()
}

pub fn parsed_requirement_group_has_requirement_surface(group: &Value) -> bool {
    if !array(group.get("options")).is_empty() || !array(group.get("sequencePaths")).is_empty() {
        return true;
    }
    if group.get("categoryOption").is_some_and(js_truthy)
        || group.get("approvedListKey").is_some_and(js_truthy)
        || group.get("programSpecific").and_then(Value::as_bool) == Some(true)
    {
        return true;
    }
    group.get("requirementType").and_then(Value::as_str) == Some("choose_credits")
        && group.get("canCreatePlaceholder").and_then(Value::as_bool) != Some(false)
}

pub fn filter_parsed_requirement_groups_by_course_code_predicate<F>(
    groups: &[Value],
    should_keep_course_code: F,
) -> Vec<Value>
where
    F: Fn(&Value) -> bool,
{
    groups
        .iter()
        .map(|group| {
            let sequence_paths = array(group.get("sequencePaths"))
                .iter()
                .filter_map(|path| {
                    let original_uw_courses = array(path.get("uwCourses"));
                    let kept_uw_courses = original_uw_courses
                        .iter()
                        .filter(|course_code| should_keep_course_code(course_code))
                        .cloned()
                        .collect::<Vec<_>>();
                    let kept_conditional_lab_courses = array(path.get("conditionalLabCourses"))
                        .iter()
                        .filter(|course_code| should_keep_course_code(course_code))
                        .cloned()
                        .collect::<Vec<_>>();
                    if kept_uw_courses.is_empty() && kept_conditional_lab_courses.is_empty() {
                        return None;
                    }
                    let mut filtered_path = path.as_object().cloned().unwrap_or_default();
                    filtered_path.insert("uwCourses".to_string(), json!(kept_uw_courses.clone()));
                    filtered_path.insert(
                        "conditionalLabCourses".to_string(),
                        json!(kept_conditional_lab_courses),
                    );
                    if let Some(display_codes) = filter_aligned_display_course_codes(
                        path.get("displayCourseCodes"),
                        original_uw_courses,
                        &kept_uw_courses,
                    ) {
                        filtered_path.insert("displayCourseCodes".to_string(), display_codes);
                    }
                    Some(Value::Object(filtered_path))
                })
                .collect::<Vec<_>>();
            let options = array(group.get("options"))
                .iter()
                .filter_map(|option| {
                    if option.get("optionKind").and_then(Value::as_str) == Some("category-option") {
                        return Some(option.clone());
                    }
                    let original_uw_courses = array(option.get("uwCourses"));
                    let kept_uw_courses = original_uw_courses
                        .iter()
                        .filter(|course_code| should_keep_course_code(course_code))
                        .cloned()
                        .collect::<Vec<_>>();
                    let kept_equivalent_courses = array(option.get("equivalentUwCourseCodes"))
                        .iter()
                        .filter(|course_code| should_keep_course_code(course_code))
                        .cloned()
                        .collect::<Vec<_>>();
                    let kept_conditional_lab_courses = array(option.get("conditionalLabCourses"))
                        .iter()
                        .filter(|course_code| should_keep_course_code(course_code))
                        .cloned()
                        .collect::<Vec<_>>();
                    if kept_uw_courses.is_empty()
                        && kept_equivalent_courses.is_empty()
                        && kept_conditional_lab_courses.is_empty()
                    {
                        return None;
                    }
                    let mut filtered_option = option.as_object().cloned().unwrap_or_default();
                    filtered_option.insert("uwCourses".to_string(), json!(kept_uw_courses.clone()));
                    filtered_option.insert(
                        "equivalentUwCourseCodes".to_string(),
                        json!(kept_equivalent_courses),
                    );
                    filtered_option.insert(
                        "conditionalLabCourses".to_string(),
                        json!(kept_conditional_lab_courses),
                    );
                    if let Some(display_codes) = filter_aligned_display_course_codes(
                        option.get("displayCourseCodes"),
                        original_uw_courses,
                        &kept_uw_courses,
                    ) {
                        filtered_option.insert("displayCourseCodes".to_string(), display_codes);
                    }
                    Some(Value::Object(filtered_option))
                })
                .collect::<Vec<_>>();
            let mut filtered_group = group.as_object().cloned().unwrap_or_default();
            filtered_group.insert("sequencePaths".to_string(), json!(sequence_paths));
            filtered_group.insert("options".to_string(), json!(options));
            Value::Object(filtered_group)
        })
        .filter(parsed_requirement_group_has_requirement_surface)
        .collect()
}

fn filter_aligned_display_course_codes(
    display_course_codes: Option<&Value>,
    original_course_codes: &[Value],
    kept_course_codes: &[Value],
) -> Option<Value> {
    let display_course_codes = display_course_codes?;
    let Some(display_course_codes_array) = display_course_codes.as_array() else {
        return Some(display_course_codes.clone());
    };
    if display_course_codes_array.len() != original_course_codes.len() {
        return Some(display_course_codes.clone());
    }
    let kept_course_code_set = kept_course_codes
        .iter()
        .map(|course_code| normalize_course_code(&string_value(Some(course_code))))
        .collect::<HashSet<_>>();
    Some(json!(
        display_course_codes_array
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                kept_course_code_set.contains(&normalize_course_code(&string_value(Some(
                    &original_course_codes[*index],
                ))))
            })
            .map(|(_, display_code)| display_code.clone())
            .collect::<Vec<_>>()
    ))
}

pub fn get_source_section_decision_for_course<'a>(
    course_code: &str,
    audit_rows: &'a [Value],
) -> Option<&'a Value> {
    let normalized_course_code = normalize_course_code(course_code);
    let rows = audit_rows
        .iter()
        .filter(|row| row_course_codes(row).contains(&normalized_course_code))
        .collect::<Vec<_>>();
    let post_admission_rows = rows
        .iter()
        .copied()
        .filter(|row| {
            row.get("detectedSectionRole").and_then(Value::as_str)
                == Some("post-admission-degree-completion-section")
        })
        .collect::<Vec<_>>();
    if let Some(first_post_admission_row) = post_admission_rows.first().copied() {
        let has_strong_schedulable_row = rows.iter().any(|row| {
            row.get("schedulable").is_some_and(js_truthy)
                && source_section_strong_credit_row_pattern()
                    .is_match(&normalize_whitespace(&string_value(row.get("rawLine"))))
        });
        if !has_strong_schedulable_row {
            return Some(first_post_admission_row);
        }
    }
    rows.iter()
        .copied()
        .find(|row| row.get("schedulable").is_some_and(js_truthy))
        .or_else(|| rows.first().copied())
}

fn source_section_strong_credit_row_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\b\d+(?:\.\d+)?\s*cr\b",
        ))
        .expect("source-section strong credit row pattern")
    })
}

pub fn source_section_rows_allow_course_code(
    course_code: &Value,
    audit_rows: &[Value],
    matched_row: Option<&Value>,
) -> bool {
    let rows = rows_for_course_code(course_code, audit_rows);
    if rows.is_empty() {
        return matched_row
            .and_then(|row| row.get("schedulable"))
            .and_then(Value::as_bool)
            != Some(false);
    }
    rows.iter()
        .any(|row| row.get("schedulable").and_then(Value::as_bool) == Some(true))
}

fn rows_for_course_code<'a>(course_code: &Value, audit_rows: &'a [Value]) -> Vec<&'a Value> {
    let normalized_course_code = normalize_course_code(&string_value(Some(course_code)));
    if normalized_course_code.is_empty() {
        return Vec::new();
    }
    audit_rows
        .iter()
        .filter(|row| row_course_codes(row).contains(&normalized_course_code))
        .collect()
}

fn row_course_codes(row: &Value) -> Vec<String> {
    array(row.get("courseCodesExtracted"))
        .iter()
        .map(|course_code| normalize_course_code(&string_value(Some(course_code))))
        .collect()
}

fn normalize_source_section_line(value: &str) -> String {
    let without_page_prefix = strip_snapshot_page_prefix(value);
    let without_noise =
        without_page_prefix.trim_start_matches(is_source_line_noise_prefix_character);
    normalize_whitespace(without_noise)
}

fn is_source_line_noise_prefix_character(character: char) -> bool {
    matches!(
        character,
        '\u{2610}'
            | '\u{2611}'
            | '\u{2612}'
            | '\u{2713}'
            | '\u{2714}'
            | '\u{f071}'
            | '\u{25a1}'
            | '\u{25a0}'
            | '\u{25aa}'
            | '\u{25cf}'
            | '\u{2022}'
            | '*'
            | '-'
    ) || super::requirement_shape::is_js_whitespace(character)
}

fn is_informative_source_section_text(value: &str) -> bool {
    value.encode_utf16().count() >= 18
        && !source_section_numeric_range_pattern().is_match(value)
        && !source_section_header_only_pattern().is_match(value)
}

fn source_section_numeric_range_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"^[0-9]+(?:\s*(?:-|to)\s*[0-9]+)?$",
        ))
        .expect("source section numeric range")
    })
}

fn source_section_header_only_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:credits?|course #|course name|total)$",
        ))
        .expect("source section header text")
    })
}

fn first_truthy<'a>(input: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| input.get(*key))
        .find(|value| js_truthy(value))
}

fn url_looks_like_pdf(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    lowered.match_indices(".pdf").any(|(index, _)| {
        lowered
            .as_bytes()
            .get(index + 4)
            .map_or(true, |byte| *byte == b'?' || *byte == b'#')
    })
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

fn js_value_or_undefined(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(value) => js_value(value),
    }
}

fn array(value: Option<&Value>) -> &[Value] {
    value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn nullish<'a>(left: Option<&'a Value>, right: Option<&'a Value>) -> Option<&'a Value> {
    left.filter(|value| !value.is_null())
        .or_else(|| right.filter(|value| !value.is_null()))
}

fn first_non_nullish(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).filter(|field| !field.is_null()))
        .map(|field| string_value(Some(field)))
        .unwrap_or_default()
}

fn string_value(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => super::requirement_shape::js_number_to_string(value),
        Some(Value::Array(_)) => value.map(js_value).unwrap_or_default(),
        Some(Value::Object(_)) => "[object Object]".to_string(),
    }
}

fn non_nullish_or(value: Option<&Value>, fallback: &str) -> Value {
    value
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!(fallback))
}

fn js_value_or(value: Option<&Value>, fallback: &str) -> String {
    value
        .filter(|value| !value.is_null())
        .map(js_value)
        .unwrap_or_else(|| fallback.to_string())
}

fn js_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => super::requirement_shape::js_number_to_string(value),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    js_value(value)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn normalize_whitespace(value: &str) -> String {
    super::requirement_shape::normalize_js_whitespace(value)
}

fn unique_in_order(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn has_sequence_choice_context(value: &str) -> bool {
    let text = normalize_whitespace(value);
    sequence_choice_context_pattern().is_match(&text)
        || sequence_label_alternative_pattern().is_match(&text)
        || (sequence_alternative_choice_line_pattern().is_match(&text)
            && sequence_or_pattern().is_match(&text))
}

fn sectioned_requirement_option_cue_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)^(?:sectioned|pathway-scoped)\b",
        ))
        .expect("sectioned requirement option cue")
    })
}

fn sequence_choice_context_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            SEQUENCE_CHOICE_CONTEXT_PATTERN_SOURCE,
        ))
        .expect("sequence choice context")
    })
}

fn sequence_label_alternative_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            SEQUENCE_LABEL_ALTERNATIVE_PATTERN_SOURCE,
        ))
        .expect("sequence label alternatives")
    })
}

fn sequence_alternative_choice_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            SEQUENCE_ALTERNATIVE_CHOICE_LINE_PATTERN_SOURCE,
        ))
        .expect("sequence alternative choice line")
    })
}

fn sequence_or_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&super::requirement_shape::js_regex_compatibility_pattern(
            r"(?i)\bor\b",
        ))
        .expect("sequence or cue")
    })
}
