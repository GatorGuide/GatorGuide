use super::requirement_course_codes::normalize_course_code;
use super::requirement_ids::slugify_requirement_identifier;
use super::requirement_scope::RequirementSourceScope;
use super::requirement_shape::{strip_leading_requirement_glyphs, strip_snapshot_page_prefix};
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
                && normalize_course_code(&first_non_nullish(course, &["normalizedCourseCode", "courseCode"]))
                    == normalized_course_code
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
                    .any(|code| normalize_course_code(&string_value(Some(code))) == normalized_course_code)
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
                format!("Can create required rows: {}", yes_no(source_scope.can_create_required_rows)),
                format!("Can create option groups: {}", yes_no(source_scope.can_create_option_groups)),
                format!("Can create approved filters: {}", yes_no(source_scope.can_create_approved_filters)),
                format!("Course code: {course_code}"),
                format!("Emitted as: {emitted_as}"),
                format!("Scheduled: {}", yes_no(scheduled)),
                format!("Issue: {issue}"),
            ]
            .join(" ")
        })
        .collect()
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
    let shape = input.get("shape").map(js_value).unwrap_or_else(|| "undefined".to_string());
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
    if url_looks_like_pdf(&source_url_text) || parser_type.to_ascii_lowercase().starts_with("pdf-") {
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
        let list_title = normalize_whitespace(&row.get("sectionTitle").map(|value| string_value(Some(value))).unwrap_or_default());
        let list_title = if list_title.is_empty() {
            if role == "elective-list" { "Elective list" } else { "Approved course list" }.to_string()
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
            groups.push((
                key,
                role.to_string(),
                list_title,
                accepted_codes,
            ));
        }
    }
    groups
        .into_iter()
        .map(|(_, role, list_title, accepted_codes)| {
            let shape = if role == "elective-list" { "elective-list" } else { "approved-course-list" };
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
        .filter(|group| group.get("requirementType").and_then(Value::as_str) == Some("sequence_choice"))
        .map(|group| {
            let paths = array(nullish(group.get("sequencePaths"), group.get("options")));
            let sequence_paths = paths
                .iter()
                .map(|path| {
                    let raw_label = nullish(path.get("label"), path.get("pathLabel"));
                    let label = normalize_whitespace(&string_value(raw_label));
                    let label = if label.is_empty() { "sequence path" } else { &label };
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
            let raw_text_debug = raw_text.map(js_value).unwrap_or_else(|| "undefined".to_string());
            let debug_text = [
                "[parser sequence-choice audit]".to_string(),
                format!("Owner id: {}", js_value(&owner_id)),
                format!("Source URL: {}", js_value(&source_url)),
                format!("Raw text: {raw_text_debug}"),
                "Detected sequence choice: yes".to_string(),
                format!("Sequence paths: {}", sequence_paths.join(" | ")),
                "Selected/default path: none".to_string(),
                format!("Emitted independent required rows: {}", yes_no(emitted_independent_required_rows)),
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
                normalize_whitespace(&string_value(row.get("rawLine")))
                    .contains(&normalized_text)
            })
        })
        .or_else(|| {
            audit_rows.iter().find(|row| {
                let row_text = normalize_whitespace(&string_value(row.get("rawLine")));
                is_informative_source_section_text(&row_text)
                    && normalized_text.contains(&row_text)
            })
        })
}

pub fn source_section_decision_allows_course_option(
    option: &Value,
    audit_rows: &[Value],
) -> bool {
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
    if matched_row.and_then(|row| row.get("schedulable")).and_then(Value::as_bool) == Some(true) {
        return true;
    }
    if matched_row.and_then(|row| row.get("schedulable")).and_then(Value::as_bool) == Some(false) {
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
    let has_schedulable_course_row = course_section_rows
        .iter()
        .any(|rows| rows.iter().any(|row| row.get("schedulable").and_then(Value::as_bool) == Some(true)));
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
    if matched_row.and_then(|row| row.get("schedulable")).and_then(Value::as_bool) == Some(true) {
        return Some(option.clone());
    }
    let original_uw_courses = array(option.get("uwCourses"));
    let equivalent_courses = array(option.get("equivalentUwCourseCodes"));
    let should_filter_codes = matched_row
        .and_then(|row| row.get("schedulable"))
        .and_then(Value::as_bool)
        == Some(false)
        || original_uw_courses.iter().chain(equivalent_courses.iter()).any(|course_code| {
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
        .filter(|course_code| source_section_rows_allow_course_code(course_code, audit_rows, matched_row))
        .cloned()
        .collect::<Vec<_>>();
    let filtered_equivalent_courses = equivalent_courses
        .iter()
        .filter(|course_code| source_section_rows_allow_course_code(course_code, audit_rows, matched_row))
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
    filtered_option.insert("equivalentUwCourseCodes".to_string(), json!(filtered_equivalent_courses));
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
        .filter_map(|option| filter_parsed_requirement_option_codes_by_source_sections(option, audit_rows))
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
        Regex::new(r"(?i)\b\d+(?:\.\d+)?\s*cr\b")
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
    normalize_whitespace(&strip_leading_requirement_glyphs(&without_page_prefix))
}

fn is_informative_source_section_text(value: &str) -> bool {
    value.encode_utf16().count() >= 18
        && !source_section_numeric_range_pattern().is_match(value)
        && !source_section_header_only_pattern().is_match(value)
}

fn source_section_numeric_range_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"^\d+(?:\s*(?:-|to)\s*\d+)?$").expect("source section numeric range"))
}

fn source_section_header_only_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)^(?:credits?|course #|course name|total)$").expect("source section header text"))
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
    value.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

fn nullish<'a>(left: Option<&'a Value>, right: Option<&'a Value>) -> Option<&'a Value> {
    left.filter(|value| !value.is_null()).or_else(|| right.filter(|value| !value.is_null()))
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
        Some(Value::Number(value)) => value.to_string(),
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
    value.filter(|value| !value.is_null()).map(js_value).unwrap_or_else(|| fallback.to_string())
}

fn js_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(values) => values.iter().map(|value| if value.is_null() { String::new() } else { js_value(value) }).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn unique_in_order(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values.into_iter().filter(|value| seen.insert(value.clone())).collect()
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
